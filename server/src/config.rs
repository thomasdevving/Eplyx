//! Process configuration, all from the environment.
//!
//! The analysis path is entirely offline and needs no RPC or archive
//! credentials: corpus construction is a separate workflow that runs
//! elsewhere, never on the path of a pull request. Governance and current-state observation use separate optional read-only
//! providers in the parent service. Every analysis worker receives a cleared
//! environment and retained bytes; no run reads either provider.
//!
//! `EPLYX_RPC_URL` is the one Solana provider an operator connects: it serves
//! developers' CLIs through the password-gated hosted RPC (`/v1/rpc`, with
//! `EPLYX_ACCESS_PASSWORD`), and is the default for governance and observation
//! when their dedicated variables are unset.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, Result};

/// Uploads are bounded so a malformed or hostile request cannot exhaust memory.
const DEFAULT_MAX_CANDIDATE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_EXPECTATION_BYTES: usize = 256 * 1024;
/// A bundle is a corpus, a baseline and every dependency binary, so it is the
/// largest thing this service accepts by some distance.
const DEFAULT_MAX_BUNDLE_BYTES: usize = 192 * 1024 * 1024;
/// Replay is CPU-bound and synchronous. A small cap bounds execution of MAIN
/// durable queued runs.
const DEFAULT_MAX_CONCURRENT_RUNS: usize = 2;

#[derive(Clone, Debug)]
pub struct Config {
    /// Trusted executable, never configurable by an HTTP request.
    pub worker_binary: PathBuf,
    /// Browser origins allowed to call this API.
    ///
    /// Empty by default, which means no cross-origin browser access at all: a
    /// page served from another origin cannot read this API unless somebody
    /// deliberately names it. A CI runner is unaffected either way — `curl` does
    /// not enforce the same-origin policy — so an absent setting costs nothing
    /// and an over-broad one costs a lot.
    pub allowed_origins: Vec<String>,
    /// The hosted dashboard's credential.
    ///
    /// Configured on the server, never issued by it, and absent by default:
    /// with no operator token nothing can create a project or move a bundle
    /// pointer over HTTP, which is the right posture for a service that has not
    /// been given one. Project tokens are unaffected.
    pub operator_token: Option<String>,
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    pub max_candidate_bytes: usize,
    pub max_expectation_bytes: usize,
    pub max_bundle_bytes: usize,
    pub max_concurrent_runs: usize,
    /// JSON-RPC endpoint for governance verification (`EPLYX_GOVERNANCE_RPC_URL`,
    /// else `EPLYX_RPC_URL`). Absent by default, which turns that one endpoint
    /// off. Transport configuration only: never written to a record, report or
    /// response.
    pub governance_rpc_url: Option<String>,
    /// The hosted Solana RPC developers use instead of a provider of their
    /// own: `EPLYX_RPC_URL` behind `EPLYX_ACCESS_PASSWORD`. Present only when
    /// both are set; otherwise `/v1/rpc` is off, never open.
    pub hosted_rpc: Option<HostedRpc>,
}

#[derive(Clone)]
pub struct HostedRpc {
    /// The operator's provider, possibly carrying its API key.
    pub upstream: String,
    /// The access password developers unlock it with.
    pub password: String,
}

impl std::fmt::Debug for HostedRpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostedRpc(redacted)")
    }
}

fn var<T: std::str::FromStr>(name: &str, fallback: T) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(text) => text
            .parse()
            .map_err(|error| anyhow::anyhow!("{name}: {error}")),
        Err(_) => Ok(fallback),
    }
}

/// The address to listen on.
///
/// `EPLYX_BIND` wins when it is set, so a local run can still name its own
/// address. Otherwise a platform-injected `PORT` is honoured: a managed host
/// chooses the port and expects the process to follow, and a server that
/// ignores it is simply unreachable there. Only then the local default.
fn bind_address() -> Result<SocketAddr> {
    const DEFAULT: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8080);
    if std::env::var_os("EPLYX_BIND").is_some() {
        return var("EPLYX_BIND", DEFAULT).context("EPLYX_BIND");
    }
    match std::env::var("PORT") {
        Ok(port) => {
            let port: u16 = port
                .trim()
                .parse()
                .map_err(|error| anyhow::anyhow!("PORT: {error}"))?;
            Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port))
        }
        Err(_) => Ok(DEFAULT),
    }
}

fn nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `EPLYX_RPC_URL` and `EPLYX_ACCESS_PASSWORD`, validated without ever
/// printing either.
fn hosted_rpc() -> Result<Option<HostedRpc>> {
    let upstream = nonempty("EPLYX_RPC_URL");
    if let Some(url) = &upstream {
        eplyx_engine::ingest::rpc::HttpRpc::new(url.clone())
            .map_err(|_| anyhow::anyhow!("EPLYX_RPC_URL must be an http(s) URL"))?;
    }
    let password = nonempty("EPLYX_ACCESS_PASSWORD");
    if let Some(password) = &password {
        eplyx_engine::cloud::access::validate_password(password)
            .context("EPLYX_ACCESS_PASSWORD")?;
    }
    Ok(upstream
        .zip(password)
        .map(|(upstream, password)| HostedRpc { upstream, password }))
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            worker_binary: std::env::current_exe()?,
            operator_token: std::env::var("EPLYX_OPERATOR_TOKEN")
                .ok()
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty()),
            allowed_origins: std::env::var("EPLYX_ALLOWED_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(str::to_string)
                .collect(),
            data_dir: PathBuf::from(
                std::env::var("EPLYX_DATA_DIR").unwrap_or_else(|_| "/data".to_string()),
            ),
            bind: bind_address()?,
            max_candidate_bytes: var("EPLYX_MAX_CANDIDATE_BYTES", DEFAULT_MAX_CANDIDATE_BYTES)?,
            max_expectation_bytes: var(
                "EPLYX_MAX_EXPECTATION_BYTES",
                DEFAULT_MAX_EXPECTATION_BYTES,
            )?,
            max_bundle_bytes: var("EPLYX_MAX_BUNDLE_BYTES", DEFAULT_MAX_BUNDLE_BYTES)?,
            max_concurrent_runs: var("EPLYX_MAX_CONCURRENT_RUNS", DEFAULT_MAX_CONCURRENT_RUNS)?,
            governance_rpc_url: nonempty("EPLYX_GOVERNANCE_RPC_URL")
                .or_else(|| nonempty("EPLYX_RPC_URL")),
            hosted_rpc: hosted_rpc()?,
        })
    }
}
