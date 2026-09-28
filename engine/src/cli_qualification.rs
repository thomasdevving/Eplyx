use anyhow::Result;
use clap::Parser;
use eplyx_engine::{
    ingest::{
        rpc::{HttpRpc, OfflineRpc, RpcProvider},
        CachedRpc,
    },
    qualification::{self, Config, Providers},
};
use serde_json::Value;
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
pub struct PrepareArgs {
    /// Explicit qualification JSON (distinct from global project --config).
    #[arg(long)]
    spec: PathBuf,
    /// Must not already exist. Never refreshes or deletes an existing run.
    #[arg(long)]
    out: PathBuf,
    /// Existing RPC cache root, reusable across fresh output runs.
    #[arg(long)]
    cache: Option<PathBuf>,
    /// Disable transport; every request must already be cached.
    #[arg(long)]
    offline: bool,
    /// Explicitly accept the persisted bounded coverage, never activate a project.
    #[arg(long)]
    accept_coverage: bool,
    #[arg(long,value_enum,default_value_t=crate::Format::Text)]
    format: crate::Format,
}
// Lazy transport keeps missing support ahead of even endpoint resolution. All
// errors crossing the qualification boundary are fixed typed codes.
struct RuntimeRpc<'a> {
    url_env: &'a str,
    origin_env: Option<&'a str>,
    cache: PathBuf,
    offline: bool,
}
impl RpcProvider for RuntimeRpc<'_> {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let url = std::env::var(self.url_env)
            .map_err(|_| anyhow::anyhow!("provider environment missing"))?;
        let root = self
            .cache
            .join(eplyx_engine::replay::hash_bytes(url.as_bytes()));
        if self.offline {
            return CachedRpc {
                provider: &OfflineRpc,
                root,
            }
            .call(method, params);
        }
        let mut rpc = HttpRpc::new(url)?;
        if let Some(env) = self.origin_env {
            rpc = rpc.with_origin(
                std::env::var(env)
                    .map_err(|_| anyhow::anyhow!("provider origin environment missing"))?,
            )?;
        }
        let retry = eplyx_engine::ingest::rpc::RetryingRpc {
            provider: &rpc,
            max_retries: 2,
            base_backoff: std::time::Duration::from_millis(250),
        };
        CachedRpc {
            provider: &retry,
            root,
        }
        .call(method, params)
    }
}
pub fn run(args: PrepareArgs) -> Result<ExitCode> {
    // Do not format serde errors: an unknown field may itself contain a secret.
    let config = std::fs::read(&args.spec)
        .ok()
        .and_then(|b| serde_json::from_slice::<Config>(&b).ok());
    let Some(mut config) = config else {
        eprintln!("qualification: invalid input JSON; see docs/bundle-qualification.md");
        return Ok(ExitCode::from(23));
    };
    config.accept_coverage |= args.accept_coverage;
    // Resolve data paths relative to the spec, never persist that absolute path.
    let parent = args.spec.parent().unwrap_or(std::path::Path::new("."));
    for path in [&mut config.observed, &mut config.prepared_corpus]
        .into_iter()
        .flatten()
    {
        if path.is_relative() {
            *path = parent.join(&*path);
        }
    }
    let cache = args.cache.unwrap_or_else(|| args.out.join("cache"));
    let make = |name| RuntimeRpc {
        url_env: name,
        origin_env: config.providers.origin_env.as_deref(),
        cache: cache.clone(),
        offline: args.offline,
    };
    let tx = make(config.providers.transaction_url_env.as_str());
    let accounts = make(config.providers.account_url_env.as_str());
    let blocks = make(config.providers.block_url_env.as_str());
    let receipt = match qualification::prepare(
        &config,
        &args.out,
        Providers {
            transactions: &tx,
            accounts: &accounts,
            blocks: &blocks,
        },
    ) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("qualification: could not create or persist fresh output; existing output is never overwritten");
            return Ok(ExitCode::from(24));
        }
    };
    match args.format {
        crate::Format::Json => println!("{}", serde_json::to_string_pretty(&receipt)?),
        crate::Format::Text => {
            let name = |s| {
                serde_json::to_value(s)
                    .unwrap_or(Value::Null)
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned()
            };
            println!(
                "Qualification stages: {}",
                receipt
                    .states
                    .iter()
                    .map(name)
                    .collect::<Vec<_>>()
                    .join(" → ")
            );
            println!("Qualification state: {}", name(&receipt.state));
            if let Some(bundle) = &receipt.bundle {
                println!(
                    "Bundle: {}\nSelected: {} records",
                    bundle.bundle_sha256, bundle.record_count
                );
            }
            println!(
                "Observed semantic population: {}",
                receipt.observed_semantic_population["status"]
                    .as_str()
                    .unwrap_or("unknown")
            );
            for blocker in &receipt.blockers {
                println!(
                    "- {}",
                    serde_json::to_value(blocker)?
                        .as_str()
                        .unwrap_or("internal_failure")
                );
            }
            for limitation in &receipt.limitations {
                println!("- {}: {}", limitation.code, limitation.detail);
            }
            println!(
                "Next: {}\nReceipt: <output>/receipt.json",
                receipt.next_action
            );
        }
    }
    Ok(ExitCode::from(receipt.exit_code))
}
