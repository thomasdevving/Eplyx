//! External data discovery/normalization/cache; never executes candidate code.
pub mod accounts;
pub mod controlled;
pub mod rpc;
pub mod transactions;
use crate::{
    replay::{hash_bytes, ReplayRecord},
    types::NamedAccount,
};
use anyhow::{Context, Result};
use rpc::RpcProvider;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use transactions::HistoricalTransaction;

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    // One writer per cache directory. Interrupted writes never replace a valid entry.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .context("invalid persisted JSON")
}
pub fn cache_path(root: &Path, method: &str, params: &Value) -> PathBuf {
    let digest = hash_bytes(&serde_json::to_vec(&(1, method, params)).expect("serializable JSON"));
    root.join("rpc-v1").join(format!("{digest}.json"))
}
pub struct CachedRpc<'a> {
    pub provider: &'a dyn RpcProvider,
    pub root: PathBuf,
}
impl RpcProvider for CachedRpc<'_> {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let path = cache_path(&self.root, method, &params);
        if path.exists() {
            return read_json(&path);
        }
        let value = self.provider.call(method, params)?;
        // Unavailable transactions are not permanent cache hits.
        if !value.is_null() {
            write_json(&path, &value)?;
        }
        Ok(value)
    }
}

#[derive(Default)]
pub struct CacheMetrics {
    hits: AtomicU64,
    misses: AtomicU64,
}

impl CacheMetrics {
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
}

/// Cache adapter with explicit per-run metrics. Kept separate from `CachedRpc`
/// so Phase 4's minimal API and behavior remain unchanged.
pub struct MeasuredCachedRpc<'a> {
    pub provider: &'a dyn RpcProvider,
    pub root: PathBuf,
    pub metrics: &'a CacheMetrics,
}

impl RpcProvider for MeasuredCachedRpc<'_> {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let path = cache_path(&self.root, method, &params);
        if path.exists() {
            self.metrics.hits.fetch_add(1, Ordering::Relaxed);
            return read_json(&path);
        }
        self.metrics.misses.fetch_add(1, Ordering::Relaxed);
        let value = self.provider.call(method, params)?;
        if !value.is_null() {
            write_json(&path, &value)?;
        }
        Ok(value)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestManifest {
    pub schema_version: u32,
    pub program_id: String,
    pub genesis_hash: String,
    pub start_slot: u64,
    pub end_slot: u64,
    pub transactions: Vec<HistoricalTransaction>,
}
/// Inclusive slot window; paginate until complete. Errors never mean an empty
/// successful corpus. Activity/address association is filtered to instructions.
pub fn discover(
    rpc: &dyn RpcProvider,
    program: &str,
    start: u64,
    end: u64,
) -> Result<IngestManifest> {
    discover_bounded_with_concurrency(rpc, program, start, end, None, 1)
}

/// As [`discover`], with a deterministic cap on normalized interactions. The
/// newest matching signatures in the slot window are retained, then sorted in
/// canonical slot/signature order.
pub fn discover_bounded(
    rpc: &dyn RpcProvider,
    program: &str,
    start: u64,
    end: u64,
    max_transactions: Option<usize>,
) -> Result<IngestManifest> {
    discover_bounded_with_concurrency(rpc, program, start, end, max_transactions, 1)
}

pub fn discover_bounded_with_concurrency(
    rpc: &dyn RpcProvider,
    program: &str,
    start: u64,
    end: u64,
    max_transactions: Option<usize>,
    concurrency: usize,
) -> Result<IngestManifest> {
    program.parse::<solana_address::Address>()?;
    anyhow::ensure!(start <= end, "start_slot must be <= end_slot");
    anyhow::ensure!(
        max_transactions != Some(0),
        "transaction limit must be positive"
    );
    anyhow::ensure!(concurrency > 0, "RPC concurrency must be positive");
    let genesis_hash = rpc
        .call("getGenesisHash", json!([]))?
        .as_str()
        .context("missing genesis hash")?
        .to_string();
    let mut before: Option<String> = None;
    let mut seen = BTreeSet::new();
    let mut txs = Vec::new();
    loop {
        let mut config = json!({"limit":1000,"commitment":"confirmed"});
        if let Some(cursor) = &before {
            config["before"] = json!(cursor);
        }
        let page = rpc.call("getSignaturesForAddress", json!([program, config]))?;
        let page = page.as_array().context("invalid signature page")?;
        if page.is_empty() {
            break;
        }
        let mut reached_start = false;
        let mut candidates = Vec::new();
        for item in page {
            let slot = item["slot"].as_u64().context("signature missing slot")?;
            let signature = item["signature"]
                .as_str()
                .context("missing signature")?
                .to_string();
            anyhow::ensure!(
                seen.insert(signature.clone()),
                "RPC pagination repeated a signature"
            );
            if slot < start {
                reached_start = true;
                continue;
            }
            if slot > end {
                continue;
            }
            candidates.push((signature, slot));
        }
        let mut reached_limit = false;
        for chunk in candidates.chunks(concurrency) {
            let results: Vec<_> = std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|(signature, slot)| {
                        scope.spawn(move || {
                            let raw=rpc.call("getTransaction",json!([signature,{"encoding":"json","commitment":"confirmed","maxSupportedTransactionVersion":0}]))?;
                            let tx = transactions::normalize(&raw)
                                .with_context(|| format!("normalizing {signature}"))?;
                            anyhow::ensure!(
                                tx.signature == *signature && tx.slot == *slot,
                                "transaction identity differs from discovery"
                            );
                            Ok::<_, anyhow::Error>(tx)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("RPC worker panicked"))
                    .collect()
            });
            for result in results {
                let tx = result?;
                if tx
                    .instructions
                    .iter()
                    .chain(tx.inner_instructions.iter())
                    .any(|i| i.program == program)
                {
                    txs.push(tx);
                    if max_transactions.is_some_and(|limit| txs.len() >= limit) {
                        reached_limit = true;
                        break;
                    }
                }
            }
            if reached_limit {
                break;
            }
        }
        if reached_start || reached_limit {
            break;
        }
        before = Some(
            page.last().context("empty page")?["signature"]
                .as_str()
                .context("missing pagination signature")?
                .into(),
        );
    }
    txs.sort_by(|a, b| a.slot.cmp(&b.slot).then(a.signature.cmp(&b.signature)));
    Ok(IngestManifest {
        schema_version: 1,
        program_id: program.into(),
        genesis_hash,
        start_slot: start,
        end_slot: end,
        transactions: txs,
    })
}

pub fn fetch_accounts(
    rpc: &dyn RpcProvider,
    addresses: &[String],
) -> Result<(u64, Vec<NamedAccount>)> {
    anyhow::ensure!(
        !addresses.is_empty() && addresses.len() <= 100,
        "account snapshot must contain 1..100 addresses in one atomic RPC response"
    );
    let response = rpc.call(
        "getMultipleAccounts",
        json!([addresses,{"encoding":"base64","commitment":"confirmed"}]),
    )?;
    let slot = response["context"]["slot"]
        .as_u64()
        .context("missing account context slot")?;
    let values = response["value"].as_array().context("missing accounts")?;
    anyhow::ensure!(
        values.len() == addresses.len(),
        "account response count mismatch"
    );
    let mut result = Vec::new();
    for (address, value) in addresses.iter().zip(values) {
        result.push(NamedAccount {
            label: address.clone(),
            address: address.clone(),
            account: accounts::normalize(value)?,
        });
    }
    Ok((slot, result))
}

pub fn ingest(
    rpc: &dyn RpcProvider,
    root: &Path,
    program: &str,
    start: u64,
    end: u64,
) -> Result<IngestManifest> {
    ingest_bounded(rpc, root, program, start, end, None)
}

pub fn ingest_bounded(
    rpc: &dyn RpcProvider,
    root: &Path,
    program: &str,
    start: u64,
    end: u64,
    max_transactions: Option<usize>,
) -> Result<IngestManifest> {
    ingest_bounded_with_concurrency(rpc, root, program, start, end, max_transactions, 1)
}

pub fn ingest_bounded_with_concurrency(
    rpc: &dyn RpcProvider,
    root: &Path,
    program: &str,
    start: u64,
    end: u64,
    max_transactions: Option<usize>,
    concurrency: usize,
) -> Result<IngestManifest> {
    let manifest =
        discover_bounded_with_concurrency(rpc, program, start, end, max_transactions, concurrency)?;
    for tx in &manifest.transactions {
        write_json(
            &root
                .join("transactions")
                .join(format!("{}.json", tx.signature)),
            tx,
        )?;
        // Public RPC only supplies CURRENT state. Persist observed slot and source,
        // never promote these samples into controlled historical snapshots.
        let addresses: Vec<_> = tx.account_keys.iter().map(|k| k.address.clone()).collect();
        let account_path = root.join("accounts").join(format!("{}.json", tx.signature));
        if !account_path.exists() {
            // Closed historical accounts can be absent now; raw current responses
            // are preserved, including null, rather than invented zero accounts.
            let mut samples = Vec::new();
            for chunk in addresses.chunks(100) {
                samples.push(json!({"addresses":chunk,"response":rpc.call("getMultipleAccounts",json!([chunk,{"encoding":"base64","commitment":"confirmed"}]))?}));
            }
            write_json(
                &account_path,
                &json!({"schema_version":1,"state_source":"current_approximation","samples":samples}),
            )?;
        }
    }
    write_json(&root.join("manifest.json"), &manifest)?;
    Ok(manifest)
}

pub fn build_corpus(
    root: &Path,
    snapshots: &Path,
    out: &Path,
    limit: Option<usize>,
) -> Result<usize> {
    let manifest: IngestManifest = read_json(&root.join("manifest.json"))?;
    anyhow::ensure!(manifest.schema_version == 1, "unsupported ingest schema");
    let mut records = Vec::new();
    // Explicit selection: all captured interactions in discovery order, or first N.
    // Setup transactions have no capture and are not selected, not failed replays.
    for tx in &manifest.transactions {
        let path = snapshots.join(format!("{}.json", tx.signature));
        if !path.exists() {
            continue;
        }
        let record: ReplayRecord = read_json(&path)?;
        anyhow::ensure!(
            record.transaction == *tx
                && record.genesis_hash == manifest.genesis_hash
                && record.program_id == manifest.program_id,
            "capture does not match ingested transaction/network"
        );
        record.validate()?;
        records.push(record);
    }
    if let Some(limit) = limit {
        records.truncate(limit);
    }
    anyhow::ensure!(!records.is_empty(),"no controlled snapshots matched; current RPC state cannot supply exact historical pre-state");
    write_json(out, &records)?;
    Ok(records.len())
}
