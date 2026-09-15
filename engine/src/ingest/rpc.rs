//! Standard JSON-RPC transport. No vendor SDK or endpoint in persisted data.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

pub trait RpcProvider {
    fn call(&self, method: &str, params: Value) -> Result<Value>;
}

pub struct HttpRpc {
    url: String,
}
impl HttpRpc {
    pub fn new(url: String) -> Result<Self> {
        anyhow::ensure!(
            url.starts_with("http://") || url.starts_with("https://"),
            "RPC URL must use HTTP(S)"
        );
        anyhow::ensure!(!url.contains(['\n', '\r', '"', '\\']), "invalid RPC URL");
        Ok(Self { url })
    }
}
impl RpcProvider for HttpRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        // Pass URL and JSON over stdin: API tokens never appear in process args,
        // persisted cache keys or error diagnostics. curl provides system TLS.
        let body = json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}).to_string();
        let escaped = body.replace('\\', "\\\\").replace('"', "\\\"");
        let config = format!(
            "url = \"{}\"\nheader = \"Content-Type: application/json\"\ndata = \"{escaped}\"\n",
            self.url
        );
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
