//! Provider-neutral venue observations. RPC program scans are discovery, not execution.
use super::*;
use crate::lifecycle::exposure::meteora_dlmm as dlmm;
use anyhow::Context;
use serde_json::json;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextKind {
    MeteoraDlmm,
    Token2022Destination,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VenueContext {
    pub id: String,
    pub kind: ContextKind,
    pub address: String,
    pub output_mint: String,
    pub execution_supported: bool,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryAttempt {
    pub provider: String,
    pub method: String,
    pub params: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VenueInventory {
    pub schema_version: u32,
    pub snapshot_sha256: String,
    pub rpc_origin: Option<String>,
    pub contexts: Vec<VenueContext>,
    pub attempts: Vec<DiscoveryAttempt>,
    pub additional_venue_runs: Vec<crate::lifecycle::exposure::AdapterRun>,
    pub verification_failures: Vec<String>,
    pub second_venue_outcome: String,
}
impl VenueInventory {
    pub fn from_snapshot(snapshot: &LifecycleSnapshot, baseline: &CoverageReport) -> Result<Self> {
        let mut contexts = Vec::new();
        if let Some(g) = &snapshot.exposures {
            for p in &g.protocol_exposures {
                let output = p
                    .assets
                    .iter()
                    .find(|a| a.mint != snapshot.asset.mint)
                    .context("paired mint absent")?;
                contexts.push(VenueContext{
id:p.pool_address.clone(),
kind:ContextKind::MeteoraDlmm,
address:p.pool_address.clone(),
output_mint:output.mint.clone(),
execution_supported:p.program_id==dlmm::PROGRAM_ID && output.mint_config.token_program==crate::lifecycle::decode::LEGACY_PROGRAM,
reason:"Previously verified exact pool/mint/vault relationships; execution still requires per-source coherent capture".into()}
);
            }
        }
        // Recipient is an actual observed token account, chosen without new outcomes.
        let recipient = baseline
            .entities
            .iter()
            .filter(|e| {
                e.classification == CoverageClassification::Proven
                    && e.account_type == "WalletCompatible"
            })
            .min_by(|a, b| a.entity_id.cmp(&b.entity_id))
            .and_then(|e| snapshot.entities.iter().find(|x| x.id == e.entity_id))
            .or_else(|| {
                snapshot
                    .entities
                    .iter()
                    .filter(|e| {
                        e.entity_type == EntityType::WalletCompatible
                            && e.state.is_initialized
                            && !e.state.is_frozen
                            && e.state.raw_balance != "0"
                    })
                    .min_by(|a, b| a.id.cmp(&b.id))
            });
        if let Some(e) = recipient {
            contexts.push(VenueContext{
id:format!("token-transfer:{}",
e.token_account),
kind:ContextKind::Token2022Destination,
address:e.token_account.clone(),
output_mint:snapshot.asset.mint.clone(),
execution_supported:snapshot.mint_config.token_program==crate::lifecycle::decode::TOKEN_2022_PROGRAM,
reason:"Actual initialized observed recipient token account; transfer means token movement only, never market exit or official transition".into()}
);
        }
        contexts.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Self {
            schema_version: 1,
            snapshot_sha256: baseline.snapshot_sha256.clone(),
            rpc_origin: None,
            contexts,
            attempts: vec![],
            additional_venue_runs: vec![],
            verification_failures: vec![],
            second_venue_outcome: "NotAttempted".into(),
        })
    }
    pub fn validate(&self, s: &LifecycleSnapshot, b: &CoverageReport) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.snapshot_sha256 == b.snapshot_sha256,
            "inventory population fingerprint mismatch"
        );
        let mut expected = Self::from_snapshot(s, b)?;
        expected.rpc_origin = self.rpc_origin.clone();
        for a in &self.attempts {
            ensure!(
                a.provider == "SolanaStandardRpc/DlmmMintScan-v1",
                "unknown discovery provider"
            );
            ensure!(
                a.method == "getProgramAccounts"
                    && [88, 120]
                        .iter()
                        .any(|offset| a.params == scan_params(s, *offset)),
                "unexpected discovery scan"
            );
            ensure!(
                a.result.is_some() != a.error.is_some(),
                "discovery must contain either result or error"
            );
            if let Some(r) = &a.result {
                add_scan(&mut expected, s, r)?;
            }
        }
        ensure!(
            self.additional_venue_runs.len() <= 1,
            "second venue verification budget exceeded"
        );
        for run in &self.additional_venue_runs {
            let world = execution_world(s, self, Some(&run.candidate_pool))?;
            let c = expected
                .contexts
                .iter_mut()
                .find(|c| c.address == run.candidate_pool)
                .context("verified venue not in discovery scan")?;
            let exposure = world
                .exposures
                .as_ref()
                .context("verified graph missing")?
                .protocol_exposures
                .iter()
                .find(|p| p.pool_address == c.address)
                .context("new venue proof missing")?;
            let output = exposure
                .assets
                .iter()
                .find(|a| a.mint != s.asset.mint)
                .context("paired proof missing")?;
            c.execution_supported = output.mint_config.token_program
                == crate::lifecycle::decode::LEGACY_PROGRAM
                && output.state.raw_balance.parse::<u64>()? > 0;
            c.reason = if c.execution_supported {
"Additional exact venue relationships verified before selection from independent finalized RPC; per-source code/bin/Clock capture still required"}
 else {
"Verified additional venue has unsupported output program or no positive paired public reserve"}
.into();
        }
        expected.additional_venue_runs = self.additional_venue_runs.clone();
        expected.verification_failures = self.verification_failures.clone();
        expected.attempts = self.attempts.clone();
        expected.second_venue_outcome = outcome(&expected);
        // A snapshot-only inventory legitimately states NotAttempted.
        if self.attempts.is_empty() {
            expected.second_venue_outcome = "NotAttempted".into();
        }
        expected.contexts.sort_by(|a, b| a.id.cmp(&b.id));
        ensure!(
            *self == expected,
            "inventory relationships/status differ from raw discovery evidence"
        );
        Ok(())
    }
}
fn scan_params(s: &LifecycleSnapshot, offset: u64) -> serde_json::Value {
    json!([dlmm::PROGRAM_ID,{"commitment":"finalized","encoding":"base64","withContext":true,"minContextSlot":s.source.max_observed_slot,"filters":[{"dataSize":904},{"memcmp":{"offset":offset,"bytes":s.asset.mint}}]}])
}
fn add_scan(inv: &mut VenueInventory, s: &LifecycleSnapshot, r: &serde_json::Value) -> Result<()> {
    ensure!(
        r["context"]["slot"]
            .as_u64()
            .is_some_and(|x| x >= s.source.max_observed_slot),
        "discovery scan predates population"
    );
    let mut entries = r["value"]
        .as_array()
        .context("scan accounts missing")?
        .clone();
    entries.sort_by_key(|e| e["pubkey"].as_str().unwrap_or("").to_string());
    for e in entries {
        let address = e["pubkey"].as_str().context("scan pubkey missing")?;
        if inv.contexts.iter().any(|c| c.address == address) {
            continue;
        }
        let decoded = dlmm::decode_pool(address, &e["account"], &s.asset.mint);
        let (output,
reason)=match decoded {
Ok(p)=>(p.mints.iter().find(|m|m.to_string()!=s.asset.mint).map(ToString::to_string).unwrap_or_default(),
"Real DLMM mint-scan candidate; additional pool/vault/oracle/bin relationship verification is not part of the immutable baseline snapshot, so this context is not executable in this run".into()),
Err(e)=>(String::new(),
format!("Real discovered account has unsupported/invalid bounded DLMM layout: {e:#}"))}
;

        inv.contexts.push(VenueContext {
            id: address.into(),
            kind: ContextKind::MeteoraDlmm,
            address: address.into(),
            output_mint: output,
            execution_supported: false,
            reason,
        });
    }
    Ok(())
}
fn outcome(i: &VenueInventory) -> String {
    if i.contexts
        .iter()
        .filter(|c| c.kind == ContextKind::MeteoraDlmm && c.execution_supported)
        .count()
        > 1
    {
        "SecondVenueVerifiedExecutionPending"
    } else if i
        .contexts
        .iter()
        .filter(|c| c.kind == ContextKind::MeteoraDlmm)
        .count()
        > 1
    {
        "SecondVenueDiscoveredButUnsupported"
    } else if i.attempts.iter().any(|a| a.error.is_some()) {
        "NoSecondViableVenueFound_DiscoveryIncomplete"
    } else {
        "NoSecondViableVenueFound_InBoundedDlmmScan"
    }
    .into()
}
/// Add only independently verified venue observations to an ephemeral execution
/// world. The original population and all Phase 4–6 artifacts stay unchanged.
pub fn execution_world(
    s: &LifecycleSnapshot,
    inventory: &VenueInventory,
    context: Option<&str>,
) -> Result<LifecycleSnapshot> {
    let Some(run) = inventory
        .additional_venue_runs
        .iter()
        .find(|r| Some(r.candidate_pool.as_str()) == context)
    else {
        return Ok(s.clone());
    };
    let mut base = s.clone();
    base.schema_version = 1;
    base.exposures = None;
    let graph = crate::lifecycle::exposure::normalize_graph(&base, vec![run.clone()])?;
    let mut world = s.clone();
    world.exposures = Some(graph);
    Ok(world)
}
