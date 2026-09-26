//! Bounded protocol proof and graph refinement. No routing, valuations or execution.
pub mod meteora_dlmm;

use super::{
    decode::{raw_account_bytes, MintConfig, TokenAccountState},
    rpc::SolanaRpc,
    EntityType, EvidenceRef, LifecycleSnapshot, RpcEvidence,
};
use anyhow::{ensure, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::replay::hash_bytes as sha256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterIdentity {
    pub name: String,
    pub version: String,
    pub decoder_revision: String,
    pub program_id: String,
}

/// Protocol rules belong here, never in mint discovery or generic graph code.
pub trait ExposureAdapter {
    fn identity(&self) -> AdapterIdentity;
    fn required_accounts(&self, pool: &str, raw: &Value, target_mint: &str) -> Result<Vec<String>>;
    fn verify(
        &self,
        snapshot: &LifecycleSnapshot,
        run: &AdapterRun,
        run_index: usize,
    ) -> Result<LiquidityExposure>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterRun {
    pub adapter: AdapterIdentity,
    pub candidate_pool: String,
    pub captured_at: String,
    pub rpc_origin: String,
    pub evidence: Vec<RpcEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EvidenceLayer {
    Lifecycle,
    Adapter { run: usize },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProtocolEvidence {
    pub layer: EvidenceLayer,
    pub account: String,
    pub runtime_owner: String,
    pub reference: EvidenceRef,
    pub raw_data_sha256: String,
    pub decoder: String,
}

impl ProtocolEvidence {
    pub fn lifecycle(
        snapshot: &LifecycleSnapshot,
        address: &str,
        reference: &EvidenceRef,
        decoder: &str,
    ) -> Result<Self> {
        let record = snapshot
            .evidence
            .get(reference.rpc_id)
            .context("missing lifecycle evidence record")?;
        ensure!(
            context_slot(&record.result)? == reference.slot,
            "lifecycle reference context mismatch"
        );
        let raw = record
            .result
            .pointer(&reference.pointer)
            .context("missing lifecycle evidence pointer")?;
        Self::from_raw(
            EvidenceLayer::Lifecycle,
            address,
            reference.clone(),
            raw,
            decoder,
        )
    }
    pub fn adapter(
        run: &AdapterRun,
        run_index: usize,
        index: usize,
        decoder: &str,
    ) -> Result<Self> {
        let record = run
            .evidence
            .get(2)
            .context("missing protocol verification batch")?;
        let address = record.params[0][index]
            .as_str()
            .context("missing verification address")?;
        let reference = EvidenceRef {
            rpc_id: 2,
            pointer: format!("/value/{index}"),
            slot: context_slot(&record.result)?,
        };
        let raw = record
            .result
            .pointer(&reference.pointer)
            .context("missing verification account")?;
        Self::from_raw(
            EvidenceLayer::Adapter { run: run_index },
            address,
            reference,
            raw,
            decoder,
        )
    }
    fn from_raw(
        layer: EvidenceLayer,
        address: &str,
        reference: EvidenceRef,
        raw: &Value,
        decoder: &str,
    ) -> Result<Self> {
        Ok(Self {
            layer,
            account: address.into(),
            runtime_owner: raw["owner"]
                .as_str()
                .context("evidence runtime owner missing")?
                .into(),
            reference,
            raw_data_sha256: sha256(&raw_account_bytes(raw)?),
            decoder: decoder.into(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotVaultLink {
    pub phase2_entity_id: String,
    pub original_classification: EntityType,
    pub phase2_raw_balance: String,
    pub current_raw_balance: String,
    pub phase2_vault_evidence: ProtocolEvidence,
    /// Phase 2 already retained the raw pool as the token authority account.
    pub phase2_pool_evidence: ProtocolEvidence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquidityAsset {
    pub mint: String,
    pub vault: String,
    pub mint_config: MintConfig,
    pub state: TokenAccountState,
    pub mint_evidence: ProtocolEvidence,
    pub vault_evidence: ProtocolEvidence,
    pub phase2_link: Option<SnapshotVaultLink>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquidityExposure {
    pub id: String,
    pub protocol: String,
    pub product: String,
    pub program_id: String,
    pub pool_address: String,
    pub authority: String,
    pub authority_rule: String,
    pub assets: Vec<LiquidityAsset>,
    /// LP/position discovery is intentionally not performed.
    pub position_model: Option<String>,
    pub decoded_pool: Value,
    #[serde(with = "crate::numfmt::u64_string")]
    pub discovered_at_slot: u64,
    pub adapter: AdapterIdentity,
    pub pool_evidence: ProtocolEvidence,
    pub program_evidence: ProtocolEvidence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClassificationRefinement {
    pub classification: String,
    pub protocol_exposure_id: String,
    pub program_controlled: bool,
    pub reason: String,
    pub evidence: Vec<ProtocolEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountExposure {
    pub phase2_entity_id: String,
    pub token_account: String,
    pub original_classification: EntityType,
    pub refinement: Option<ClassificationRefinement>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExposureEdge {
    pub from: String,
    pub to: String,
    pub relationship: String,
    pub reason: String,
    pub evidence: Vec<ProtocolEvidence>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExposureSummary {
    pub token_accounts: usize,
    pub wallet_compatible: usize,
    pub verified_program_controlled: usize,
    pub pools_verified: usize,
    pub target_vaults: usize,
    pub phase2_entities_refined: usize,
    pub phase2_unknown_refined: usize,
    pub phase2_unknown_remaining: usize,
    pub program_owned_authority_without_verified_integration: usize,
    /// Neither wallet compatibility nor a selected integration was proven.
    pub unresolved_entities: usize,
    pub active_delegated: usize,
    pub frozen: usize,
    pub current_target_raw_in_verified_vaults: String,
    pub phase2_target_raw_in_verified_vaults: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExposureGraph {
    pub schema_version: u32,
    pub asset_mint: String,
    pub source_snapshot_sha256: String,
    pub adapter_runs: Vec<AdapterRun>,
    pub protocol_exposures: Vec<LiquidityExposure>,
    pub account_exposures: Vec<AccountExposure>,
    pub edges: Vec<ExposureEdge>,
    pub summary: ExposureSummary,
}

impl ExposureGraph {
    pub fn validate(&self, base: &LifecycleSnapshot) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported exposure graph schema"
        );
        ensure!(
            *self == normalize_graph(base, self.adapter_runs.clone())?,
            "exposure graph disagrees with raw proof or Phase 2 state"
        );
        Ok(())
    }
    pub fn render_summary(&self, base: &LifecycleSnapshot) -> String {
        let s = &self.summary;
        let mut text=format!("{} Production Exposure\nMint: {}\n\nToken accounts: {}\nWallet-compatible (Phase 2): {}\nVerified program-controlled vaults: {}\nPhase 2 entities refined: {}\nPreviously Unknown refined: {}\nUnknown remaining: {}\nUnresolved entities: {}\nActive delegated (Phase 2): {}\nFrozen (Phase 2): {}\n\n",
            base.asset.name,self.asset_mint,s.token_accounts,s.wallet_compatible,s.verified_program_controlled,
            s.phase2_entities_refined,s.phase2_unknown_refined,s.phase2_unknown_remaining,s.unresolved_entities,s.active_delegated,s.frozen);
        for exposure in &self.protocol_exposures {
            text.push_str(&format!(
                "{} {}\n  Pool: {}\n  Program: {}\n  Pool authority: {}\n  Verified at slot: {}\n",
                exposure.protocol,
                exposure.product,
                exposure.pool_address,
                exposure.program_id,
                exposure.authority,
                exposure.discovered_at_slot
            ));
            for asset in &exposure.assets {
                text.push_str(&format!("  {} asset\n    Mint: {}\n    Vault: {}\n    Raw balance: {}\n    Decimal base units: {}\n    Token program: {}\n",if asset.mint==self.asset_mint {"Lifecycle"}else{"Paired"},asset.mint,asset.vault,asset.state.raw_balance,asset.state.ui_balance,asset.mint_config.token_program));
            }
        }
        for warning in &s.warnings {
            text.push_str(&format!("Warning: {warning}\n"));
        }
        text
    }
}

pub(super) fn context_slot(result: &Value) -> Result<u64> {
    result["context"]["slot"]
        .as_u64()
        .context("protocol RPC context slot missing")
}
fn config(min_slot: u64) -> Value {
    json!({"encoding":"base64","commitment":"finalized","minContextSlot":min_slot})
}
fn record(
    rpc: &impl SolanaRpc,
    evidence: &mut Vec<RpcEvidence>,
    method: &str,
    params: Value,
) -> Result<Value> {
    let result = rpc.call(method, params.clone())?;
    evidence.push(RpcEvidence {
        id: evidence.len(),
        method: method.into(),
        params,
        result: result.clone(),
    });
    Ok(result)
}

/// Enrich a schema-1 snapshot; the original account entities/evidence are retained.
pub fn discover(
    base: &LifecycleSnapshot,
    adapter: &dyn ExposureAdapter,
    pool: &str,
    rpc: &impl SolanaRpc,
) -> Result<LifecycleSnapshot> {
    base.validate()?;
    ensure!(base.schema_version==1 && base.exposures.is_none(),"exposure capture requires a Phase 2 schema-1 snapshot; existing graphs are not silently replaced");
    let mut evidence = Vec::new();
    let genesis = record(rpc, &mut evidence, "getGenesisHash", json!([]))?;
    ensure!(
        genesis.as_str() == Some(&base.source.genesis_hash),
        "protocol RPC is on a different chain than the snapshot"
    );
    let initial = record(
        rpc,
        &mut evidence,
        "getAccountInfo",
        json!([pool, config(base.source.max_observed_slot)]),
    )?;
    let initial_slot = context_slot(&initial)?;
    ensure!(
        initial_slot >= base.source.max_observed_slot,
        "protocol observation predates snapshot"
    );
    let accounts = adapter.required_accounts(pool, &initial["value"], &base.asset.mint)?;
    record(
        rpc,
        &mut evidence,
        "getMultipleAccounts",
        json!([accounts, config(initial_slot)]),
    )?;
    let run = AdapterRun {
        adapter: adapter.identity(),
        candidate_pool: pool.into(),
        captured_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        rpc_origin: "configured_read_only_provider".into(),
        evidence,
    };
    let graph = normalize_graph(base, vec![run])?;
    let mut enriched = base.clone();
    enriched.schema_version = 2;
    enriched.exposures = Some(graph);
    enriched.validate()?;
    Ok(enriched)
}

fn verify_run(
    base: &LifecycleSnapshot,
    run: &AdapterRun,
    index: usize,
    adapter: &dyn ExposureAdapter,
) -> Result<LiquidityExposure> {
    ensure!(
        run.adapter == adapter.identity(),
        "unsupported adapter identity/version/decoder revision"
    );
    chrono::DateTime::parse_from_rfc3339(&run.captured_at)
        .context("invalid protocol capture timestamp")?;
    ensure!(
        run.evidence.len() == 3,
        "incomplete or unexpected protocol evidence transcript"
    );
    for (i, e) in run.evidence.iter().enumerate() {
        ensure!(e.id == i, "noncanonical protocol evidence IDs");
    }
    let genesis = &run.evidence[0];
    ensure!(
        genesis.method == "getGenesisHash"
            && genesis.params == json!([])
            && genesis.result.as_str() == Some(&base.source.genesis_hash),
        "protocol genesis evidence mismatch"
    );
    let initial = &run.evidence[1];
    ensure!(
        initial.method == "getAccountInfo"
            && initial.params == json!([run.candidate_pool, config(base.source.max_observed_slot)]),
        "invalid initial protocol query"
    );
    let initial_slot = context_slot(&initial.result)?;
    ensure!(
        initial_slot >= base.source.max_observed_slot,
        "protocol context predates Phase 2 snapshot"
    );
    let accounts = adapter.required_accounts(
        &run.candidate_pool,
        &initial.result["value"],
        &base.asset.mint,
    )?;
    let final_read = &run.evidence[2];
    ensure!(
        final_read.method == "getMultipleAccounts"
            && final_read.params == json!([accounts, config(initial_slot)]),
        "invalid protocol verification batch"
    );
    let values = final_read.result["value"]
        .as_array()
        .context("protocol batch values missing")?;
    ensure!(
        values.len() == accounts.len(),
        "incomplete protocol verification batch"
    );
    ensure!(
        context_slot(&final_read.result)? >= initial_slot,
        "protocol verification slot predates candidate read"
    );
    ensure!(
        adapter.required_accounts(&run.candidate_pool, &values[0], &base.asset.mint)? == accounts,
        "protocol mint/vault relationship changed during capture"
    );
    adapter.verify(base, run, index)
}

pub fn normalize_graph(base: &LifecycleSnapshot, runs: Vec<AdapterRun>) -> Result<ExposureGraph> {
    ensure!(
        base.schema_version == 1 && base.exposures.is_none(),
        "graph normalization requires schema-1 base state"
    );
    base.validate()?;
    // Phase 3 intentionally selects one object and one adapter, not global indexing.
    ensure!(
        runs.len() == 1,
        "Phase 3 supports exactly one candidate adapter run"
    );
    let adapter = meteora_dlmm::MeteoraDlmmAdapter;
    let exposure = verify_run(base, &runs[0], 0, &adapter)?;
    let mut refinements = BTreeMap::new();
    let mut edges = Vec::new();
    let mut summary = ExposureSummary::default();
    let mut live_total = 0u128;
    let mut frozen_total = 0u128;
    let pool_id = format!("pool:{}", exposure.pool_address);
    edges.push(ExposureEdge {
        from: pool_id.clone(),
        to: format!("program:{}", exposure.program_id),
        relationship: "OwnedByProtocolProgram".into(),
        reason:
            "Pool runtime owner matches the supported protocol; the program account is executable"
                .into(),
        evidence: vec![
            exposure.pool_evidence.clone(),
            exposure.program_evidence.clone(),
        ],
    });
    edges.push(ExposureEdge {
        from: pool_id.clone(),
        to: format!("authority:{}", exposure.authority),
        relationship: "PoolPdaAuthority".into(),
        reason: exposure.authority_rule.clone(),
        evidence: vec![exposure.pool_evidence.clone()],
    });
    for asset in &exposure.assets {
        let vault_id = format!("token-account:{}", asset.vault);
        let mint_id = format!("asset:{}", asset.mint);
        let proofs = vec![
            exposure.pool_evidence.clone(),
            asset.vault_evidence.clone(),
            asset.mint_evidence.clone(),
        ];
        edges.push(ExposureEdge {from:pool_id.clone(),to:vault_id.clone(),relationship:"CanonicalLiquidityVault".into(),reason:"Decoded pool reserve and canonical [pool,mint] PDA equal this vault; its SPL owner is the pool PDA".into(),evidence:proofs.clone()});
        edges.push(ExposureEdge {
            from: vault_id.clone(),
            to: mint_id.clone(),
            relationship: "VaultMint".into(),
            reason: "Vault mint bytes match decoded pool asset and supported mint program".into(),
            evidence: proofs.clone(),
        });
        edges.push(ExposureEdge {from:mint_id,to:pool_id.clone(),relationship:"EmbeddedInPool".into(),reason:"Supported on-chain pool, canonical vault and matching mint prove the asset relationship".into(),evidence:proofs.clone()});
        if let Some(link) = &asset.phase2_link {
            live_total = live_total
                .checked_add(asset.state.raw_balance.parse::<u128>()?)
                .context("current vault sum overflow")?;
            frozen_total = frozen_total
                .checked_add(link.phase2_raw_balance.parse::<u128>()?)
                .context("Phase 2 vault sum overflow")?;
            summary.target_vaults += 1;
            let mut refinement_proof = proofs;
            refinement_proof.extend([
                link.phase2_vault_evidence.clone(),
                link.phase2_pool_evidence.clone(),
            ]);
            ensure!(refinements.insert(link.phase2_entity_id.clone(),ClassificationRefinement {
                classification:"LiquidityVault".into(),protocol_exposure_id:exposure.id.clone(),program_controlled:true,
                reason:"Pool and canonical vault relationship are proven in retained Phase 2 bytes and current protocol verification; original classification and balances remain intact".into(),evidence:refinement_proof }).is_none(),"duplicate target refinement");
            if link.phase2_raw_balance != link.current_raw_balance {
                summary.warnings.push(format!("Vault {} balance changed since Phase 2: {} -> {} raw units. Current integration and frozen holder amounts are separate observations.",asset.vault,link.phase2_raw_balance,link.current_raw_balance));
            }
        }
    }
    ensure!(
        summary.target_vaults == 1,
        "selected integration must link exactly one lifecycle-asset vault"
    );
    let mut account_exposures = Vec::new();
    let mut seen = BTreeSet::new();
    for entity in &base.entities {
        ensure!(seen.insert(&entity.id), "duplicate Phase 2 entity ID");
        let proof = ProtocolEvidence::lifecycle(
            base,
            &entity.token_account,
            &entity.token_account_evidence,
            "SPL token account / Token-2022 extensions 3.1.1",
        )?;
        let account_id = format!("token-account:{}", entity.token_account);
        edges.push(ExposureEdge {from:format!("asset:{}",base.asset.mint),to:account_id.clone(),relationship:"Phase2Holding".into(),reason:"Retained Phase 2 token account mint equals lifecycle asset; public balance is in the parent entity".into(),evidence:vec![proof.clone()]});
        if entity.state.has_active_delegate {
            edges.push(ExposureEdge {from:account_id.clone(),to:format!("authority:{}",entity.state.delegate.as_ref().context("active delegate absent")?),relationship:"ActiveDelegate".into(),reason:"Retained token account has a delegate and a positive allowance; frozen/mint-wide authority state remains separate".into(),evidence:vec![proof.clone()]});
        }
        if entity.state.is_frozen {
            edges.push(ExposureEdge {
                from: account_id,
                to: "account-state:Frozen".into(),
                relationship: "FrozenAccount".into(),
                reason: "Retained SPL account state is Frozen".into(),
                evidence: vec![proof],
            });
        }
        let refinement = refinements.remove(&entity.id);
        if refinement.is_some() {
            summary.phase2_entities_refined += 1;
            summary.verified_program_controlled += 1;
            summary.phase2_unknown_refined +=
                usize::from(entity.entity_type == EntityType::Unknown);
        } else if entity.entity_type == EntityType::WalletCompatible {
            summary.wallet_compatible += 1;
        } else {
            summary.unresolved_entities += 1;
            summary.phase2_unknown_remaining +=
                usize::from(entity.entity_type == EntityType::Unknown);
            summary.program_owned_authority_without_verified_integration +=
                usize::from(entity.entity_type == EntityType::ProgramOwnedAuthority);
        }
        account_exposures.push(AccountExposure {
            phase2_entity_id: entity.id.clone(),
            token_account: entity.token_account.clone(),
            original_classification: entity.entity_type.clone(),
            refinement,
        });
    }
    ensure!(
        refinements.is_empty(),
        "protocol refinement has no matching Phase 2 entity"
    );
    account_exposures.sort_by(|a, b| a.token_account.cmp(&b.token_account));
    edges.sort_by(|a, b| (&a.from, &a.to, &a.relationship).cmp(&(&b.from, &b.to, &b.relationship)));
    summary.token_accounts = base.entities.len();
    summary.pools_verified = 1;
    summary.active_delegated = base.summary.active_delegated;
    summary.frozen = base.summary.frozen;
    summary.current_target_raw_in_verified_vaults = live_total.to_string();
    summary.phase2_target_raw_in_verified_vaults = frozen_total.to_string();
    summary.warnings.push("Exactly one supplied pool is verified. This is not complete venue discovery; wallet compatibility is not proof of direct human ownership, and other program-owned authorities are not proven program-controlled.".into());
    summary.warnings.push("Pool, both vaults, both mints and executable program were verified in one finalized getMultipleAccounts context. Phase 2 holdings come from earlier contexts; no combined atomic or historical world state is claimed.".into());
    summary.warnings.push("Vault amounts are public decimal base units, with Token-2022 extensions preserved; no scaled display, price, withdrawable LP reserve or exitability inference is made. Mint-wide permanent delegates mean program control need not be exclusive.".into());
    Ok(ExposureGraph {
        schema_version: 1,
        asset_mint: base.asset.mint.clone(),
        source_snapshot_sha256: sha256(base.to_json()?.as_bytes()),
        adapter_runs: runs,
        protocol_exposures: vec![exposure],
        account_exposures,
        edges,
        summary,
    })
}
