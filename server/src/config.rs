//! Process configuration, all from the environment.
//!
//! No RPC or archive credentials appear here, and none are needed: the serving
//! path is entirely offline. Corpus construction is a separate workflow that
//! runs elsewhere, with its own credentials, and never on the path of a pull
//! request.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// Uploads are bounded so a malformed or hostile request cannot exhaust memory.
const DEFAULT_MAX_CANDIDATE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_EXPECTATION_BYTES: usize = 256 * 1024;
/// Replay is CPU-bound and synchronous. A small cap keeps a pilot host
/// responsive without a queue, which is deliberately not built yet.
const DEFAULT_MAX_CONCURRENT_RUNS: usize = 2;

#[derive(Clone, Debug)]
pub struct Config {
    /// Browser origins allowed to call this API.
    ///
    /// Empty by default, which means no cross-origin browser access at all: a
    /// page served from another origin cannot read this API unless somebody
    /// deliberately names it. A CI runner is unaffected either way — `curl` does
    /// not enforce the same-origin policy — so an absent setting costs nothing
    /// and an over-broad one costs a lot.
    pub allowed_origins: Vec<String>,
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    pub max_candidate_bytes: usize,
    pub max_expectation_bytes: usize,
    pub max_concurrent_runs: usize,
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

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
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
            bind: var("EPLYX_BIND", "0.0.0.0:8080".parse::<SocketAddr>()?).context("EPLYX_BIND")?,
            max_candidate_bytes: var("EPLYX_MAX_CANDIDATE_BYTES", DEFAULT_MAX_CANDIDATE_BYTES)?,
            max_expectation_bytes: var(
                "EPLYX_MAX_EXPECTATION_BYTES",
                DEFAULT_MAX_EXPECTATION_BYTES,
            )?,
            max_concurrent_runs: var("EPLYX_MAX_CONCURRENT_RUNS", DEFAULT_MAX_CONCURRENT_RUNS)?,
        })
    }
}
