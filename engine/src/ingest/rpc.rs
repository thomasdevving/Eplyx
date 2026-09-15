//! Standard JSON-RPC transport. No vendor SDK or endpoint in persisted data.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
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
}
impl HttpRpc {
    pub fn new(url: String) -> Result<Self> {
        anyhow::ensure!(
            url.starts_with("http://") || url.starts_with("https://"),
            "RPC URL must use HTTP(S)"
        );
        anyhow::ensure!(!url.contains(['\n', '\r', '"', '\\']), "invalid RPC URL");
        Ok(Self { url, origin: None })
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
}
impl RpcProvider for HttpRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
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
        config.push_str(&format!("data = \"{escaped}\"\n"));
        let mut child = Command::new("curl")
            .args(["--silent", "--fail", "--max-time", "30", "--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("starting curl RPC transport")?;
        child
            .stdin
            .take()
            .context("curl stdin")?
            .write_all(config.as_bytes())?;
        let output = child.wait_with_output()?;
        anyhow::ensure!(
            output.status.success(),
            "RPC transport failed for {method} (endpoint redacted)"
        );
        let response: Value = serde_json::from_slice(&output.stdout).context("invalid RPC JSON")?;
        if response.get("error").is_some() {
            bail!(
                "RPC {method} returned error code {}, transaction error {}, program logs {}",
                response["error"]["code"],
                response["error"]["data"]["err"],
                response["error"]["data"]["logs"]
            );
        }
        response
            .get("result")
            .cloned()
            .context("RPC response missing result")
    }
}
