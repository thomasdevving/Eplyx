//! Standard JSON-RPC transport. No vendor SDK or endpoint in persisted data.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

pub trait RpcProvider: Sync {
    fn call(&self, method: &str, params: Value) -> Result<Value>;
}

/// Deliberately unavailable transport for proving a cache-backed workflow makes
/// no network calls. A cache miss becomes an explicit error.
pub struct OfflineRpc;
impl RpcProvider for OfflineRpc {
    fn call(&self, method: &str, _: Value) -> Result<Value> {
        bail!("offline mode cache miss for RPC {method}")
    }
}

/// Provider-neutral bounded retry layer. Put the cache outside this adapter so
/// cache hits never consume retry budget or touch the transport.
pub struct RetryingRpc<'a> {
    pub provider: &'a dyn RpcProvider,
    pub max_retries: u32,
    pub base_backoff: Duration,
}

impl RpcProvider for RetryingRpc<'_> {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut attempt = 0_u32;
        loop {
            match self.provider.call(method, params.clone()) {
                Ok(value) => return Ok(value),
                Err(error) if attempt < self.max_retries => {
                    let multiplier = 1_u32.checked_shl(attempt.min(10)).unwrap_or(u32::MAX);
                    let delay = self
                        .base_backoff
                        .checked_mul(multiplier)
                        .unwrap_or(Duration::from_secs(30))
                        .min(Duration::from_secs(30));
                    if !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                    attempt += 1;
                    let _ = error;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("RPC {method} failed after {} attempt(s)", attempt + 1)
                    })
                }
            }
        }
    }
}

pub struct HttpRpc {
    url: String,
    origin: Option<String>,
    bearer: Option<String>,
    response_limit: usize,
    timeout_seconds: u64,
}

pub const DEFAULT_RESPONSE_LIMIT: usize = 64 * 1024 * 1024;
impl HttpRpc {
    pub fn new(url: String) -> Result<Self> {
        anyhow::ensure!(
            url.starts_with("http://") || url.starts_with("https://"),
            "RPC URL must use HTTP(S)"
        );
        anyhow::ensure!(!url.contains(['\n', '\r', '"', '\\']), "invalid RPC URL");
        Ok(Self {
            url,
            origin: None,
            bearer: None,
            response_limit: DEFAULT_RESPONSE_LIMIT,
            timeout_seconds: 30,
        })
    }

    /// A server-selected byte ceiling, applied while reading, before JSON
    /// allocation. Capture controllers own retries; this transport makes one
    /// attempt and never follows redirects.
    pub fn with_response_limit(mut self, bytes: usize) -> Result<Self> {
        anyhow::ensure!(
            bytes > 0 && bytes <= 512 * 1024 * 1024,
            "invalid RPC response budget"
        );
        self.response_limit = bytes;
        Ok(self)
    }

    /// Capture controllers select a finite timeout for bounded population scans.
    pub fn with_timeout(mut self, seconds: u64) -> Result<Self> {
        anyhow::ensure!((1..=1800).contains(&seconds), "invalid RPC timeout budget");
        self.timeout_seconds = seconds;
        Ok(self)
    }

    /// Attach an Origin header for providers that protect browser-facing demo
    /// endpoints with an origin allowlist. The value is transport configuration
    /// only and is never included in a cache, manifest, corpus, or report.
    pub fn with_origin(mut self, origin: String) -> Result<Self> {
        anyhow::ensure!(
            origin.starts_with("http://") || origin.starts_with("https://"),
            "RPC origin must use HTTP(S)"
        );
        anyhow::ensure!(
            !origin.contains(['\n', '\r', '"', '\\']),
            "invalid RPC origin"
        );
        self.origin = Some(origin);
        Ok(self)
    }

    /// Send `Authorization: Bearer <secret>`, as the Eplyx hosted RPC requires.
    /// Like the URL, the secret travels over curl's stdin only and never
    /// appears in process arguments, a cache key or an error.
    pub fn with_bearer(mut self, secret: &str) -> Result<Self> {
        anyhow::ensure!(
            !secret.is_empty()
                && secret.len() <= 512
                && secret
                    .bytes()
                    .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\'),
            "invalid RPC bearer credential"
        );
        self.bearer = Some(secret.to_owned());
        Ok(self)
    }

    /// One request, returning the validated JSON-RPC envelope as the endpoint
    /// sent it: `result` or `error`, uninterpreted. [`RpcProvider::call`] is
    /// this plus the interpretation; a relay needs the envelope itself.
    pub fn envelope(&self, method: &str, params: Value) -> Result<Value> {
        // Pass URL and JSON over stdin: API tokens never appear in process args,
        // persisted cache keys or error diagnostics. curl provides system TLS.
        let body = json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}).to_string();
        let escaped = body.replace('\\', "\\\\").replace('"', "\\\"");
        let mut config = format!(
            "url = \"{}\"\nheader = \"Content-Type: application/json\"\n",
            self.url
        );
        if let Some(origin) = &self.origin {
            config.push_str(&format!("header = \"Origin: {origin}\"\n"));
        }
        if let Some(secret) = &self.bearer {
            config.push_str(&format!("header = \"Authorization: Bearer {secret}\"\n"));
        }
        config.push_str(&format!("data = \"{escaped}\"\n"));
        let mut child = Command::new("curl")
            .args([
                "--disable",
                "--silent",
                "--max-time",
                &self.timeout_seconds.to_string(),
                "--write-out",
                "\n%{http_code}",
                "--config",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("starting curl RPC transport")?;
        let sent = child
            .stdin
            .take()
            .context("curl stdin")?
            .write_all(config.as_bytes());
        if sent.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            bail!("RPC request write failed (endpoint redacted)");
        }
        // The body plus the status line `--write-out` appends.
        let bytes = read_limited(
            child.stdout.take().context("curl stdout")?,
            self.response_limit + STATUS_SUFFIX,
        );
        if bytes.is_err() {
            let _ = child.kill();
        }
        let status = child.wait()?;
        let bytes = bytes?;
        anyhow::ensure!(
            status.success(),
            "RPC transport failed for {method} (endpoint redacted)"
        );
        let (body, http) = split_status(&bytes)
            .with_context(|| format!("RPC transport failed for {method} (endpoint redacted)"))?;
        anyhow::ensure!(
            body.len() <= self.response_limit,
            "RPC response exceeds observation byte budget"
        );
        if !(200..300).contains(&http) {
            if self.bearer.is_some() && http == 401 {
                bail!(
                    "the Eplyx hosted RPC rejected the access password; run `eplyx access` again"
                );
            }
            // The Eplyx hosted RPC explains a refusal; a provider's own error
            // page is never echoed.
            let detail = self
                .bearer
                .as_ref()
                .and_then(|_| serde_json::from_slice::<Value>(body).ok())
                .and_then(|value| value["error"].as_str().map(str::to_owned))
                .filter(|text| {
                    text.len() <= 300
                        && !text.contains("://")
                        && !text.chars().any(char::is_control)
                });
            return Err(HttpStatusError {
                method: method.to_owned(),
                status: http,
                detail,
            }
            .into());
        }
        let response: Value = serde_json::from_slice(body).context("invalid RPC JSON")?;
        anyhow::ensure!(
            response["jsonrpc"] == "2.0" && response["id"] == 1,
            "invalid RPC envelope"
        );
        Ok(response)
    }
}

/// An endpoint answered with a non-success HTTP status. Typed so a relay can
/// tell a provider's rate limit from an outage.
#[derive(Debug)]
pub struct HttpStatusError {
    pub method: String,
    pub status: u16,
    /// The Eplyx hosted RPC's own explanation, when it gave one.
    pub detail: Option<String>,
}

impl std::fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RPC transport failed for {} (HTTP {}, endpoint redacted)",
            self.method, self.status
        )?;
        match &self.detail {
            Some(detail) => write!(f, ": {detail}"),
            None => Ok(()),
        }
    }
}

impl std::error::Error for HttpStatusError {}

/// `\n` and at most three status digits.
const STATUS_SUFFIX: usize = 4;

/// Separate the status `--write-out` appended from the response body.
fn split_status(bytes: &[u8]) -> Option<(&[u8], u16)> {
    let at = bytes.iter().rposition(|b| *b == b'\n')?;
    let status = std::str::from_utf8(&bytes[at + 1..]).ok()?.parse().ok()?;
    Some((&bytes[..at], status))
}

impl RpcProvider for HttpRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let response = self.envelope(method, params)?;
        if response.get("error").is_some() {
            bail!(
                "RPC {method} returned error code {:?}",
                response["error"]["code"].as_i64()
            );
        }
        response
            .get("result")
            .cloned()
            .context("RPC response missing result")
    }
}

fn read_limited(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .context("RPC response read failed")?;
    anyhow::ensure!(
        bytes.len() <= limit,
        "RPC response exceeds observation byte budget"
    );
    Ok(bytes)
}

pub const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";

/// Check network identity explicitly before collecting a current world.
pub fn require_mainnet_genesis(provider: &dyn RpcProvider) -> Result<String> {
    let result = provider.call("getGenesisHash", json!([]))?;
    anyhow::ensure!(
        result.as_str() == Some(MAINNET_GENESIS),
        "capture requires mainnet genesis"
    );
    Ok(MAINNET_GENESIS.into())
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountFilter {
    DataSize(u64),
    Memcmp { offset: usize, bytes: String },
}

fn address(value: &str) -> Result<()> {
    let key: solana_address::Address = value.parse().context("invalid capture account address")?;
    anyhow::ensure!(
        key.to_string() == value,
        "noncanonical capture account address"
    );
    Ok(())
}

/// The RPC result is retained verbatim so its context and raw account bytes
/// remain available to capture transcripts. A lagging response is never used.
pub fn get_program_accounts(
    provider: &dyn RpcProvider,
    program: &str,
    filters: &[AccountFilter],
    min_context_slot: Option<u64>,
) -> Result<Value> {
    address(program)?;
    anyhow::ensure!(filters.len() <= 4, "too many program account filters");
    for filter in filters {
        if let AccountFilter::Memcmp { bytes, .. } = filter {
            let decoded = bs58::decode(bytes)
                .into_vec()
                .context("invalid memcmp bytes")?;
            anyhow::ensure!(
                decoded.len() <= 128 && bs58::encode(&decoded).into_string() == *bytes,
                "invalid memcmp byte budget or encoding"
            );
        }
    }
    let mut config = json!({"commitment":"finalized", "encoding":"base64", "withContext":true, "filters":filters});
    if let Some(slot) = min_context_slot {
        config["minContextSlot"] = json!(slot);
    }
    let result = provider.call("getProgramAccounts", json!([program, config]))?;
    context_slot(&result, min_context_slot)?;
    anyhow::ensure!(
        result["value"].is_array(),
        "program account response lacks contextual values"
    );
    Ok(result)
}

pub fn get_multiple_accounts(
    provider: &dyn RpcProvider,
    accounts: &[String],
    min_context_slot: Option<u64>,
) -> Result<Value> {
    anyhow::ensure!(
        !accounts.is_empty() && accounts.len() <= 100,
        "multiple-account request outside 1..=100 budget"
    );
    for account in accounts {
        address(account)?;
    }
    let mut config = json!({"commitment":"finalized", "encoding":"base64"});
    if let Some(slot) = min_context_slot {
        config["minContextSlot"] = json!(slot);
    }
    let result = provider.call("getMultipleAccounts", json!([accounts, config]))?;
    context_slot(&result, min_context_slot)?;
    anyhow::ensure!(
        result["value"]
            .as_array()
            .is_some_and(|values| values.len() == accounts.len()),
        "multiple-account response count differs from exact request"
    );
    Ok(result)
}

pub fn context_slot(result: &Value, minimum: Option<u64>) -> Result<u64> {
    let slot = result["context"]["slot"]
        .as_u64()
        .context("RPC result lacks context slot")?;
    anyhow::ensure!(
        minimum.is_none_or(|minimum| slot >= minimum),
        "RPC context is older than minContextSlot"
    );
    Ok(slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Mock {
        calls: Mutex<Vec<(String, Value)>>,
        result: Value,
    }
    impl RpcProvider for Mock {
        fn call(&self, method: &str, params: Value) -> Result<Value> {
            self.calls.lock().unwrap().push((method.into(), params));
            Ok(self.result.clone())
        }
    }
    fn mock(result: Value) -> Mock {
        Mock {
            calls: Mutex::new(vec![]),
            result,
        }
    }

    #[test]
    fn the_http_status_is_split_from_the_body() {
        assert_eq!(
            split_status(b"{\"result\":1}\n200"),
            Some((&b"{\"result\":1}"[..], 200))
        );
        assert_eq!(
            split_status(b"line\nline\n401"),
            Some((&b"line\nline"[..], 401))
        );
        assert_eq!(split_status(b"\n000"), Some((&b""[..], 0)));
        assert_eq!(split_status(b"no status"), None);
        assert!(HttpRpc::new("https://rpc.example".into())
            .unwrap()
            .with_bearer("has space")
            .is_err());
    }

    #[test]
    fn bounded_reader_stops_at_one_byte_over_budget() {
        let mut input = std::io::Cursor::new(vec![42; 10_000]);
        assert!(read_limited(&mut input, 100).is_err());
        assert_eq!(input.position(), 101);
        assert_eq!(read_limited(&b"exact"[..], 5).unwrap(), b"exact");
    }

    #[test]
    fn observations_are_finalized_contextual_and_keep_the_exact_account_order() {
        let provider = mock(json!({"context":{"slot":51},"value":[null,null]}));
        let accounts = vec![
            bs58::encode([2; 32]).into_string(),
            bs58::encode([1; 32]).into_string(),
        ];
        let result = get_multiple_accounts(&provider, &accounts, Some(50)).unwrap();
        assert_eq!(result, provider.result);
        assert_eq!(
            provider.calls.lock().unwrap()[0],
            (
                "getMultipleAccounts".into(),
                json!([accounts, {"commitment":"finalized","encoding":"base64","minContextSlot":50}])
            )
        );
        let filters = [AccountFilter::Memcmp {
            offset: 0,
            bytes: accounts[0].clone(),
        }];
        get_program_accounts(&provider, &accounts[1], &filters, Some(51)).unwrap();
        assert_eq!(
            provider.calls.lock().unwrap()[1].1[1],
            json!({"commitment":"finalized","encoding":"base64","minContextSlot":51,"withContext":true,"filters":[{"memcmp":{"offset":0,"bytes":accounts[0]}}]})
        );
        assert!(get_multiple_accounts(&provider, &accounts, Some(52)).is_err());
        assert!(get_multiple_accounts(&provider, &accounts[..1], None).is_err());
        assert!(get_multiple_accounts(&provider, &[], None).is_err());
    }

    #[test]
    fn missing_context_and_wrong_genesis_fail_closed() {
        assert_eq!(bs58::decode(MAINNET_GENESIS).into_vec().unwrap().len(), 32);
        assert!(context_slot(&json!({"value":[]}), None).is_err());
        assert!(require_mainnet_genesis(&mock(json!("fixture-other-genesis"))).is_err());
        let provider = mock(json!(MAINNET_GENESIS));
        assert_eq!(require_mainnet_genesis(&provider).unwrap(), MAINNET_GENESIS);
        assert_eq!(
            provider.calls.lock().unwrap()[0],
            ("getGenesisHash".into(), json!([]))
        );
    }
}
