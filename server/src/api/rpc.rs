//! The hosted Solana RPC: a password-gated, read-only JSON-RPC relay to the
//! operator's provider, so a developer running `eplyx` needs no provider of
//! their own (`eplyx access` saves the password once).
//!
//! What it forwards is deliberately narrow: one request at a time, and only
//! the read methods Eplyx's chain commands use. Nothing that sends, signs,
//! simulates or subscribes is relayed. The provider URL, and any key in it,
//! never leaves this process: a transport failure is reported generically and
//! upstream error text is passed on only when it carries no URL. The password
//! is compared by digest in constant time, and a wrong one is answered only
//! after a delay that callers share, which bounds guessing without locking a
//! real user out. Analysis workers still start with an empty environment and
//! never see either value; no run reads the chain.

use super::*;

use std::sync::LazyLock;
use std::time::Duration;

use eplyx_engine::ingest::rpc::{HttpRpc, HttpStatusError};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The read methods Eplyx's chain commands call, and a few equally harmless
/// reads a developer may probe with.
pub const METHODS: &[&str] = &[
    "getAccountInfo",
    "getBalance",
    "getBlock",
    "getBlockHeight",
    "getBlockTime",
    "getEpochInfo",
    "getEpochSchedule",
    "getFirstAvailableBlock",
    "getGenesisHash",
    "getHealth",
    "getLatestBlockhash",
    "getMinimumBalanceForRentExemption",
    "getMultipleAccounts",
    "getProgramAccounts",
    "getSignatureStatuses",
    "getSignaturesForAddress",
    "getSlot",
    "getTokenAccountBalance",
    "getTokenAccountsByOwner",
    "getTokenLargestAccounts",
    "getTokenSupply",
    "getTransaction",
    "getVersion",
];

/// A read request is a method and its parameters; nothing near this is one.
const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// The same ceiling the CLI's own transport applies. A whole block fits.
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
/// Below the CLI's hosted timeout, so the client hears why instead of timing out.
const UPSTREAM_TIMEOUT_SECONDS: u64 = 60;
/// Each relayed request is a curl process; this bounds them.
const MAX_IN_FLIGHT: usize = 16;
/// Every wrong password waits this long, one at a time across all callers.
const WRONG_PASSWORD_DELAY: Duration = Duration::from_secs(1);

static IN_FLIGHT: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(MAX_IN_FLIGHT));
static WRONG_PASSWORDS: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// `POST /v1/rpc`.
pub(super) async fn relay(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let Some(hosted) = state.config.hosted_rpc.clone() else {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "the hosted Solana RPC is not configured on this server",
        ));
    };
    if !authorized(&headers, &hosted.password) {
        let _one_at_a_time = WRONG_PASSWORDS.lock().await;
        tokio::time::sleep(WRONG_PASSWORD_DELAY).await;
        return Err(ApiError::unauthorized());
    }
    if body.len() > MAX_REQUEST_BYTES {
        return Err(ApiError::too_large("RPC request", MAX_REQUEST_BYTES));
    }
    let request = match parse_request(&body) {
        Ok(request) => request,
        Err((id, code, message)) => return Ok(rpc_error(id, code, message)),
    };
    let _permit = IN_FLIGHT
        .acquire()
        .await
        .map_err(|_| ApiError::internal("the hosted RPC is shutting down"))?;
    let unreachable = || {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "the hosted Solana RPC could not reach its provider",
        )
    };
    let upstream = HttpRpc::new(hosted.upstream.clone())
        .and_then(|rpc| rpc.with_response_limit(MAX_RESPONSE_BYTES))
        .and_then(|rpc| rpc.with_timeout(UPSTREAM_TIMEOUT_SECONDS))
        .map_err(|_| unreachable())?;
    let Request { id, method, params } = request;
    let envelope = tokio::task::spawn_blocking(move || upstream.envelope(&method, params))
        .await
        .map_err(|_| unreachable())?
        .map_err(|error| {
            // A provider's rate limit is worth retrying; say so, as 429, so the
            // CLI's own backoff applies. Anything else is an outage.
            match error.downcast_ref::<HttpStatusError>() {
                Some(HttpStatusError { status: 429, .. }) => ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "the RPC provider is rate limiting the hosted Solana RPC; retry shortly",
                ),
                _ => unreachable(),
            }
        })?;
    let mut envelope = match envelope {
        Value::Object(map) => map,
        _ => return Err(unreachable()),
    };
    let mut answer = serde_json::Map::new();
    answer.insert("jsonrpc".into(), json!("2.0"));
    answer.insert("id".into(), id);
    match envelope.remove("result") {
        Some(result) => {
            answer.insert("result".into(), result);
        }
        None => {
            let error = envelope.remove("error").unwrap_or(Value::Null);
            answer.insert("error".into(), upstream_error(&error, &hosted.upstream));
        }
    }
    Ok(Json(Value::Object(answer)).into_response())
}

struct Request {
    id: Value,
    method: String,
    params: Value,
}

/// One JSON-RPC 2.0 request for an allowed method, or the JSON-RPC error to
/// answer with.
fn parse_request(body: &[u8]) -> std::result::Result<Request, (Value, i64, String)> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| (Value::Null, -32700, "parse error".to_string()))?;
    let Value::Object(mut request) = value else {
        return Err((
            Value::Null,
            -32600,
            "the Eplyx hosted RPC takes one request at a time, not a batch".into(),
        ));
    };
    let id = request.remove("id").unwrap_or(Value::Null);
    if !matches!(id, Value::Null | Value::Number(_) | Value::String(_)) {
        return Err((Value::Null, -32600, "invalid request id".into()));
    }
    if request.get("jsonrpc") != Some(&json!("2.0")) {
        return Err((id, -32600, "not a JSON-RPC 2.0 request".into()));
    }
    let Some(Value::String(method)) = request.remove("method") else {
        return Err((id, -32600, "request has no method".into()));
    };
    if !METHODS.contains(&method.as_str()) {
        return Err((
            id,
            -32601,
            format!("{method} is not available through the read-only Eplyx hosted RPC"),
        ));
    }
    let params = request.remove("params").unwrap_or_else(|| json!([]));
    if !matches!(params, Value::Array(_) | Value::Object(_)) {
        return Err((id, -32602, "params must be an array or an object".into()));
    }
    Ok(Request { id, method, params })
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    }))
    .into_response()
}

/// The provider's error code, always: callers act on it (`-32016` means a
/// node behind the requested context slot). Its message only when it is short
/// and names no URL, so the provider address can never be echoed out.
fn upstream_error(error: &Value, upstream: &str) -> Value {
    let code = error["code"].as_i64().unwrap_or(-32603);
    let message = error["message"]
        .as_str()
        .filter(|text| {
            text.len() <= 512
                && !text.contains("://")
                && !text.contains(upstream)
                && !text.chars().any(char::is_control)
        })
        .unwrap_or("the RPC provider returned an error");
    json!({ "code": code, "message": message })
}

/// `Authorization: Bearer <password>`, compared by digest in constant time.
fn authorized(headers: &HeaderMap, password: &str) -> bool {
    let Some(offered) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };
    let offered = Sha256::digest(offered.as_bytes());
    let expected = Sha256::digest(password.as_bytes());
    offered
        .iter()
        .zip(expected.iter())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_single_requests_for_read_methods_are_relayed() {
        let ok =
            parse_request(br#"{"jsonrpc":"2.0","id":7,"method":"getAccountInfo","params":["x"]}"#)
                .unwrap_or_else(|_| panic!("accepted"));
        assert_eq!((ok.id, ok.method.as_str()), (json!(7), "getAccountInfo"));
        for (body, code) in [
            (
                &br#"{"jsonrpc":"2.0","id":1,"method":"sendTransaction","params":["AA"]}"#[..],
                -32601,
            ),
            (
                br#"{"jsonrpc":"2.0","id":1,"method":"requestAirdrop","params":[]}"#,
                -32601,
            ),
            (
                br#"{"jsonrpc":"2.0","id":1,"method":"simulateTransaction","params":[]}"#,
                -32601,
            ),
            (br#"[{"jsonrpc":"2.0","id":1,"method":"getSlot"}]"#, -32600),
            (br#"{"id":1,"method":"getSlot"}"#, -32600),
            (br#"{"jsonrpc":"2.0","id":{},"method":"getSlot"}"#, -32600),
            (
                br#"{"jsonrpc":"2.0","id":1,"method":"getSlot","params":5}"#,
                -32602,
            ),
            (b"not json", -32700),
        ] {
            let Err((_, actual, _)) = parse_request(body) else {
                panic!("relayed {}", String::from_utf8_lossy(body));
            };
            assert_eq!(actual, code, "{}", String::from_utf8_lossy(body));
        }
        assert!(!METHODS.iter().any(|m| m.starts_with("send")
            || m.starts_with("simulate")
            || m.starts_with("request")
            || m.ends_with("Subscribe")));
    }

    #[test]
    fn provider_errors_keep_their_code_and_never_echo_an_address() {
        let upstream = "https://provider.example/?api-key=SECRET";
        let kept = upstream_error(
            &json!({"code":-32016,"message":"Minimum context slot has not been reached"}),
            upstream,
        );
        assert_eq!(kept["code"], -32016);
        assert_eq!(kept["message"], "Minimum context slot has not been reached");
        let leaky = upstream_error(
            &json!({"code":-32000,"message":format!("failed calling {upstream}")}),
            upstream,
        );
        assert_eq!(leaky["code"], -32000);
        assert!(!leaky.to_string().contains("SECRET"));
        assert_eq!(upstream_error(&Value::Null, upstream)["code"], -32603);
    }

    #[test]
    fn the_password_must_match_exactly() {
        let mut headers = HeaderMap::new();
        assert!(!authorized(&headers, "judges-2026"));
        headers.insert(header::AUTHORIZATION, "Bearer judges-2026".parse().unwrap());
        assert!(authorized(&headers, "judges-2026"));
        assert!(!authorized(&headers, "judges-2027"));
        assert!(!authorized(&headers, "judges-202"));
        headers.insert(header::AUTHORIZATION, "Basic judges-2026".parse().unwrap());
        assert!(!authorized(&headers, "judges-2026"));
    }
}
