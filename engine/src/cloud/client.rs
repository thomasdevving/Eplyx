//! Synchronous cloud transport using MAIN's curl convention. Only cloud CLI
//! commands use it; credentials go over stdin, never process arguments.
use anyhow::{anyhow, ensure, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};

pub struct Client {
    server: String,
    token: Option<String>,
}
#[derive(Debug)]
pub enum CloudError {
    Unreachable(String),
    Status { status: u16, message: String },
}
impl std::fmt::Display for CloudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(s) => write!(f, "{s}"),
            Self::Status { status, message } => write!(f, "HTTP {status}: {message}"),
        }
    }
}
impl std::error::Error for CloudError {}
impl Client {
    pub fn new(server: &str, token: Option<String>) -> Result<Self> {
        let server = super::credentials::normalize_server(server)?;
        ensure!(
            token.as_ref().is_none_or(|t| !t.is_empty()
                && t.len() <= 512
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))),
            "invalid Eplyx access token"
        );
        Ok(Self { server, token })
    }
    pub fn server(&self) -> &str {
        &self.server
    }
    fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<(u16, Value), CloudError> {
        let unreachable = || {
            CloudError::Unreachable(
                "could not reach Eplyx; retry when the service is reachable".into(),
            )
        };
        if !path.starts_with("/v1/") || path.contains(['\n', '\r', '"', '\\', '#']) {
            return Err(CloudError::Status {
                status: 400,
                message: "invalid API path".into(),
            });
        }
        let mut config=format!("url = \"{}{path}\"\nrequest = \"{method}\"\nheader = \"Content-Type: application/json\"\n",self.server);
        if let Some(token) = &self.token {
            config.push_str(&format!("header = \"Authorization: Bearer {token}\"\n"));
        }
        if let Some(body) = body {
            let text = String::from_utf8(body).map_err(|_| unreachable())?;
            config.push_str(&format!(
                "data = \"{}\"\n",
                text.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
            ));
        }
        // No redirects, proxy environment or user curlrc; the chosen origin is final.
        let mut child = Command::new("curl")
            .env_clear()
            .args([
                "--disable",
                "--silent",
                "--connect-timeout",
                "10",
                "--max-time",
                "90",
                "--max-filesize",
                "16777216",
                "--write-out",
                "\n%{http_code}",
                "--config",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| unreachable())?;
        let sent = child
            .stdin
            .take()
            .ok_or_else(unreachable)?
            .write_all(config.as_bytes());
        if sent.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(unreachable());
        }
        let mut bytes = Vec::new();
        let read = child
            .stdout
            .take()
            .ok_or_else(unreachable)?
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes);
        if read.is_err() || bytes.len() > 16 * 1024 * 1024 {
            let _ = child.kill();
            let _ = child.wait();
            return Err(unreachable());
        }
        if !child.wait().map_err(|_| unreachable())?.success() {
            return Err(unreachable());
        }
        let text = String::from_utf8(bytes).map_err(|_| unreachable())?;
        let (body, status) = text.rsplit_once('\n').ok_or_else(unreachable)?;
        let status: u16 = status.parse().map_err(|_| unreachable())?;
        let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            Ok((status, value))
        } else {
            let message = value["error"]
                .as_str()
                .filter(|s| {
                    s.len() <= 1024
                        && !s.chars().any(char::is_control)
                        && super::privacy::scan("server error", s).is_ok()
                        && self.token.as_ref().is_none_or(|t| !s.contains(t))
                })
                .unwrap_or("request refused")
                .to_owned();
            Err(CloudError::Status { status, message })
        }
    }
    pub fn get(&self, path: &str) -> Result<Value, CloudError> {
        self.send("GET", path, None).map(|(_, v)| v)
    }
    pub fn post<T: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<(u16, Value), CloudError> {
        let body = serde_json::to_vec(body).map_err(|_| CloudError::Status {
            status: 400,
            message: "invalid request".into(),
        })?;
        self.send("POST", path, Some(body))
    }
    pub fn delete(&self, path: &str) -> Result<Value, CloudError> {
        self.send("DELETE", path, None).map(|(_, v)| v)
    }
}
pub fn explain(error: &CloudError) -> anyhow::Error {
    match error {
 CloudError::Unreachable(detail)=>anyhow!("{detail}. Local analysis, search, reproduce and dashboard remain available."),
 CloudError::Status{status:401,..}=>anyhow!("Eplyx rejected the access token (HTTP 401). Run `eplyx login` again or check EPLYX_TOKEN."),
 CloudError::Status{status,message}=>anyhow!("Eplyx: {message} (HTTP {status})")
}
}
