//! The hosted Solana RPC, end to end: the CLI's own transport, a live server
//! and a provider stand-in on loopback. Nothing in between is faked.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use common::test_config;
use eplyx_engine::ingest::rpc::{HttpRpc, RpcProvider, MAINNET_GENESIS};
use eplyx_server::api::{self, AppState};
use eplyx_server::config::HostedRpc;
use eplyx_server::registry::Registry;
use eplyx_server::storage::Storage;
use serde_json::{json, Value};

const PASSWORD: &str = "judges-demo-2026";

/// What the provider saw: each request's headers and JSON body.
type Seen = Arc<Mutex<Vec<(String, Value)>>>;

/// A Solana provider stand-in. The key in its URL is what must never leak.
fn provider() -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/?api-key=PROVIDER-SECRET",
        listener.local_addr().unwrap()
    );
    let seen: Seen = Arc::default();
    let record = seen.clone();
    let echo = url.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                head.push_str(&line);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            let response = match request["method"].as_str() {
                Some("getGenesisHash") => json!({"jsonrpc":"2.0","id":1,"result":MAINNET_GENESIS}),
                Some("getTransaction") => json!({"jsonrpc":"2.0","id":1,"result":null}),
                Some("getSlot") => json!({"jsonrpc":"2.0","id":1,"error":{
                    "code":-32016,"message":"Minimum context slot has not been reached"}}),
                _ => json!({"jsonrpc":"2.0","id":1,"error":{
                    "code":-32000,"message":format!("upstream {echo} failed")}}),
            }
            .to_string();
            // A provider's rate limit, with its own address in the page.
            let (status, response) = if request["method"] == "getBlockHeight" {
                (
                    "429 Too Many Requests",
                    format!("{{\"error\":\"slow down, {echo}\"}}"),
                )
            } else {
                ("200 OK", response)
            };
            record.lock().unwrap().push((head, request));
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            );
        }
    });
    (url, seen)
}

/// A live server on loopback; returns its relay URL and the volume it owns.
fn serve(
    runtime: &tokio::runtime::Runtime,
    hosted: Option<HostedRpc>,
) -> (String, tempfile::TempDir) {
    let scratch = tempfile::tempdir().unwrap();
    let mut config = test_config();
    config.hosted_rpc = hosted;
    let state = Arc::new(AppState {
        observation: None,
        identity: None,
        config,
        registry: Registry::new(Storage::open(scratch.path().join("data")).unwrap()),
        runs: tokio::sync::Semaphore::new(1),
        governance: None,
    });
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    runtime.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        axum::serve(listener, api::router(state)).await.unwrap();
    });
    (format!("http://{address}/v1/rpc"), scratch)
}

fn client(relay: &str, password: &str) -> HttpRpc {
    HttpRpc::new(relay.into())
        .unwrap()
        .with_bearer(password)
        .unwrap()
}

#[test]
fn a_developer_reads_mainnet_through_the_hosted_rpc_with_only_the_password() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (upstream, seen) = provider();
    let (relay, _volume) = serve(
        &runtime,
        Some(HostedRpc {
            upstream: upstream.clone(),
            password: PASSWORD.into(),
        }),
    );
    let rpc = client(&relay, PASSWORD);

    assert_eq!(
        rpc.call("getGenesisHash", json!([])).unwrap(),
        json!(MAINNET_GENESIS)
    );
    // A null result is an answer, not an error.
    assert_eq!(
        rpc.call("getTransaction", json!(["sig", {"encoding": "json"}]))
            .unwrap(),
        Value::Null
    );
    // The provider's error code survives the hop: governance waits out -32016.
    let lagging = rpc.call("getSlot", json!([])).unwrap_err().to_string();
    assert!(lagging.contains("-32016"), "{lagging}");
    // Its text never carries the provider's address back out.
    let leaky = rpc
        .envelope("getBalance", json!(["x"]))
        .unwrap()
        .to_string();
    assert!(leaky.contains("-32000"), "{leaky}");
    assert!(!leaky.contains("PROVIDER-SECRET"), "{leaky}");
    // A provider rate limit arrives as one, explained, and still redacted.
    let limited = rpc
        .call("getBlockHeight", json!([]))
        .unwrap_err()
        .to_string();
    assert!(
        limited.contains("HTTP 429") && limited.contains("rate limiting"),
        "{limited}"
    );
    assert!(!limited.contains("PROVIDER-SECRET"), "{limited}");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 5);
    for (head, request) in seen.iter() {
        // The password authenticates to Eplyx and goes no further.
        assert!(
            !head.to_ascii_lowercase().contains("authorization"),
            "{head}"
        );
        assert!(!head.contains(PASSWORD), "{head}");
        assert!(!request.to_string().contains(PASSWORD));
    }
}

#[test]
fn without_the_password_nothing_reaches_the_provider() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (upstream, seen) = provider();
    let (relay, _volume) = serve(
        &runtime,
        Some(HostedRpc {
            upstream,
            password: PASSWORD.into(),
        }),
    );

    let wrong = client(&relay, "judges-demo-2025")
        .call("getGenesisHash", json!([]))
        .unwrap_err()
        .to_string();
    assert!(wrong.contains("eplyx access"), "{wrong}");
    let none = HttpRpc::new(relay.clone())
        .unwrap()
        .call("getGenesisHash", json!([]))
        .unwrap_err()
        .to_string();
    assert!(none.contains("HTTP 401"), "{none}");

    // Read-only: a write is refused by the relay, never forwarded.
    let rpc = client(&relay, PASSWORD);
    for method in ["sendTransaction", "requestAirdrop", "simulateTransaction"] {
        let refused = rpc.envelope(method, json!(["AA"])).unwrap();
        assert_eq!(refused["error"]["code"], -32601, "{method}: {refused}");
    }
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn an_unconfigured_server_has_no_open_relay() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (relay, _volume) = serve(&runtime, None);
    let off = client(&relay, PASSWORD)
        .call("getGenesisHash", json!([]))
        .unwrap_err()
        .to_string();
    assert!(
        off.contains("HTTP 503") && off.contains("not configured"),
        "{off}"
    );
}
