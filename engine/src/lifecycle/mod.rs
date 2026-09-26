//! Frozen production observations, independent of ChangeScenario and consequence execution.
pub mod artifact;
pub mod consequence;
pub mod counterfactual;
pub mod current;
pub mod frozen;
pub mod notice;
pub mod readiness;
pub mod resolution;
pub mod rollout;
pub mod spec;
pub mod support;
pub use support as decode;
pub mod exposure;
pub mod policy;
pub mod rpc {
    pub use crate::ingest::rpc::{HttpRpc as HttpSolanaRpc, RpcProvider as SolanaRpc};
}

use anyhow::{ensure, Context, Result};
use chrono::{SecondsFormat, Utc};
use decode::{decode_mint, decode_token_account, MintConfig, TokenAccountState};
use rpc::SolanaRpc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetDescriptor {
    pub name: String,
    pub mint: String,
    pub expected_token_program: Option<String>,
    pub expected_genesis_hash: Option<String>,
    /// Off-chain identity verification, never represented as on-chain evidence.
    pub verification: Vec<AssetVerification>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetVerification {
    pub url: String,
    pub observation: String,
    pub verified_at: String,
}

pub use crate::evidence::authority::{
    classify_authority, AuthorityObservation, EntityType, EvidenceRef,
};
pub use crate::ingest::observation::RpcEvidence;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LifecycleEntity {
    pub id: String,
    pub entity_type: EntityType,
    pub token_account: String,
    pub state: TokenAccountState,
    pub authority_observation: AuthorityObservation,
    pub classification_reason: String,
    pub token_account_evidence: EvidenceRef,
    pub authority_evidence: EvidenceRef,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSummary {
    pub token_accounts: usize,
    pub distinct_owner_authorities: usize,
    pub wallet_compatible: usize,
    pub program_owned_authority: usize,
    pub token_multisig: usize,
    pub unknown: usize,
    /// These counts overlap with each other and the entity types above.
    pub active_delegated: usize,
    pub delegate_present: usize,
    pub frozen: usize,
    pub uninitialized: usize,
    pub zero_balance: usize,
    pub account_extensions: BTreeMap<String, usize>,
    pub summed_public_raw_balance: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSource {
    pub rpc_origin: String,
    pub genesis_hash: String,
    pub commitment: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub enumeration_slot: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub min_observed_slot: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub max_observed_slot: u64,
    pub consistency: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LifecycleSnapshot {
    pub schema_version: u32,
    pub asset: AssetDescriptor,
    pub captured_at: String,
    /// Enumeration context, not a claim that all queries observed this slot.
    #[serde(with = "crate::numfmt::u64_string")]
    pub slot: u64,
    pub source: SnapshotSource,
    pub mint_config: MintConfig,
    pub mint_evidence: EvidenceRef,
    pub entities: Vec<LifecycleEntity>,
    pub summary: SnapshotSummary,
    pub evidence: Vec<RpcEvidence>,
    /// Schema 2 adds a graph; schema 1 files omit it and remain byte-stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposures: Option<exposure::ExposureGraph>,
}

impl LifecycleSnapshot {
    pub fn to_json(&self) -> Result<String> {
        crate::canonical::document(self)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        // create_new protects previously frozen evidence from accidental overwrite.
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| {
                format!(
                    "creating snapshot {} (output must not exist)",
                    path.display()
                )
            })?;
        file.write_all(self.to_json()?.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let snapshot: Self = serde_json::from_slice(&crate::lifecycle::artifact::read(path)?)?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Rebuild normalized facts from retained raw results without RPC access.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.schema_version, 1 | 2),
            "unsupported snapshot schema"
        );
        ensure!(
            (self.schema_version == 2) == self.exposures.is_some(),
            "snapshot schema/exposure mismatch"
        );
        let rebuilt = normalize(
            self.asset.clone(),
            self.captured_at.clone(),
            self.source.rpc_origin.clone(),
            self.evidence.clone(),
        )?;
        let mut base = self.clone();
        base.schema_version = 1;
        base.exposures = None;
        ensure!(
            base == rebuilt,
            "snapshot normalized state disagrees with its raw RPC evidence"
        );
        if let Some(graph) = &self.exposures {
            graph.validate(&base)?;
        }
        Ok(())
    }

    pub fn render_summary(&self) -> String {
        let mut text = format!("Asset: {}\nMint: {}\n\nProgram: {}\nDecimals: {}\nSupply (decimal units): {}\nMint authority: {:?}\nFreeze authority: {:?}\n\nToken accounts/entities: {}\nDistinct owner authorities: {}\nWallet-compatible: {}\nProgram-owned authority: {}\nToken multisig: {}\nUnknown: {}\nActive delegated: {} (delegate present: {})\nFrozen: {}\nZero balance: {}\n\nEnumeration slot: {}\nObserved slot range: {}..{}\nConsistency: {}\n\nMint extensions:\n",
            self.asset.name, self.asset.mint, if self.mint_config.is_token_2022 { "Token-2022" } else { "SPL Token" },
            self.mint_config.decimals, self.mint_config.decimal_supply, self.mint_config.mint_authority, self.mint_config.freeze_authority,
            self.summary.token_accounts, self.summary.distinct_owner_authorities, self.summary.wallet_compatible,
            self.summary.program_owned_authority, self.summary.token_multisig, self.summary.unknown,
            self.summary.active_delegated, self.summary.delegate_present, self.summary.frozen, self.summary.zero_balance,
            self.slot, self.source.min_observed_slot, self.source.max_observed_slot, self.source.consistency);
        for e in &self.mint_config.extensions {
            text.push_str(&format!("- {}: {}\n", e.extension_type, e.config));
        }
        for warning in &self.summary.warnings {
            text.push_str(&format!("Warning: {warning}\n"));
        }
        text
    }
}

pub trait LifecycleStateSource {
    fn capture(&self, asset: AssetDescriptor) -> Result<LifecycleSnapshot>;
}

pub struct SolanaTokenAssetSource<R: SolanaRpc> {
    pub rpc: R,
}

fn slot(result: &Value) -> Result<u64> {
    result["context"]["slot"]
        .as_u64()
        .context("RPC context slot missing")
}
fn config(min_slot: Option<u64>) -> Value {
    let mut v = json!({"encoding":"base64", "commitment":"finalized"});
    if let Some(s) = min_slot {
        v["minContextSlot"] = json!(s);
    }
    v
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

impl<R: SolanaRpc> LifecycleStateSource for SolanaTokenAssetSource<R> {
    fn capture(&self, asset: AssetDescriptor) -> Result<LifecycleSnapshot> {
        let _: Address = asset.mint.parse().context("invalid mint address")?;
        let mut evidence = Vec::new();
        let genesis = record(&self.rpc, &mut evidence, "getGenesisHash", json!([]))?;
        if let Some(expected) = &asset.expected_genesis_hash {
            ensure!(
                genesis.as_str() == Some(expected),
                "asset configuration targets a different chain"
            );
        }
        let initial = record(
            &self.rpc,
            &mut evidence,
            "getAccountInfo",
            json!([asset.mint, config(None)]),
        )?;
        let mint = decode_mint(&initial["value"])?;
        if let Some(expected) = &asset.expected_token_program {
            ensure!(
                *expected == mint.token_program,
                "mint does not match configured token program"
            );
        }
        let initial_slot = slot(&initial)?;
        let mut enumeration_config = config(Some(initial_slot));
        enumeration_config["withContext"] = json!(true);
        // No fixed dataSize: Token-2022 accounts have extension-dependent sizes.
        enumeration_config["filters"] = json!([{"memcmp":{"offset":0,"bytes":asset.mint}}]);
        let accounts = record(
            &self.rpc,
            &mut evidence,
            "getProgramAccounts",
            json!([mint.token_program, enumeration_config]),
        )?;
        let enumeration_slot = slot(&accounts)?;
        ensure!(
            enumeration_slot >= initial_slot,
            "RPC enumeration context predates mint query"
        );
        let mut owners = BTreeSet::new();
        for entry in accounts["value"]
            .as_array()
            .context("RPC enumeration not a contextual account array")?
        {
            let state = decode_token_account(
                &entry["account"],
                &mint.token_program,
                &asset.mint,
                mint.decimals,
            )
            .with_context(|| format!("decoding token account {}", entry["pubkey"]))?;
            owners.insert(state.owner);
        }
        let owners: Vec<_> = owners.into_iter().collect();
        for (index, batch) in owners.chunks(100).enumerate() {
            if index % 20 == 0 {
                eprintln!(
                    "Reading owner authority evidence: {}/{}",
                    index * 100,
                    owners.len()
                );
            }
            let r = record(
                &self.rpc,
                &mut evidence,
                "getMultipleAccounts",
                json!([batch, config(Some(enumeration_slot))]),
            )?;
            ensure!(
                slot(&r)? >= enumeration_slot,
                "RPC authority context predates enumeration"
            );
        }
        record(
            &self.rpc,
            &mut evidence,
            "getAccountInfo",
            json!([asset.mint, config(Some(enumeration_slot))]),
        )?;
        normalize(
            asset,
            Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            "configured_read_only_provider".into(),
            evidence,
        )
    }
}

/// Deterministic replay of the exact discovery transcript. RPC request shape and
/// contextual bounds are checked as well as every decoded fact.
pub fn normalize(
    asset: AssetDescriptor,
    captured_at: String,
    rpc_origin: String,
    evidence: Vec<RpcEvidence>,
) -> Result<LifecycleSnapshot> {
    let _: Address = asset.mint.parse().context("invalid asset mint")?;
    chrono::DateTime::parse_from_rfc3339(&captured_at).context("invalid capture timestamp")?;
    ensure!(evidence.len() >= 4, "incomplete discovery evidence");
    for (index, e) in evidence.iter().enumerate() {
        ensure!(e.id == index, "noncanonical evidence IDs");
    }
    ensure!(
        evidence[0].method == "getGenesisHash" && evidence[0].params == json!([]),
        "missing genesis evidence"
    );
    let genesis_hash = evidence[0]
        .result
        .as_str()
        .context("invalid genesis hash")?
        .to_string();
    if let Some(expected) = &asset.expected_genesis_hash {
        ensure!(
            *expected == genesis_hash,
            "asset configuration targets a different chain"
        );
    }
    ensure!(
        evidence[1].method == "getAccountInfo"
            && evidence[1].params == json!([asset.mint, config(None)]),
        "invalid initial mint query"
    );
    let initial = decode_mint(&evidence[1].result["value"])?;
    let initial_slot = slot(&evidence[1].result)?;
    let enumeration = &evidence[2];
    let mut expected_config = config(Some(initial_slot));
    expected_config["withContext"] = json!(true);
    expected_config["filters"] = json!([{"memcmp":{"offset":0,"bytes":asset.mint}}]);
    ensure!(
        enumeration.method == "getProgramAccounts"
            && enumeration.params == json!([initial.token_program, expected_config]),
        "invalid enumeration query or filters"
    );
    let enumeration_slot = slot(&enumeration.result)?;
    ensure!(
        enumeration_slot >= initial_slot,
        "enumeration slot predates initial mint"
    );
    let last = evidence.last().context("mint evidence missing")?;
    ensure!(
        last.method == "getAccountInfo"
            && last.params == json!([asset.mint, config(Some(enumeration_slot))]),
        "invalid final mint query"
    );
    let mint_config = decode_mint(&last.result["value"])?;
    ensure!(
        slot(&last.result)? >= enumeration_slot,
        "final mint slot predates enumeration"
    );
    ensure!(
        initial.token_program == mint_config.token_program
            && initial.decimals == mint_config.decimals,
        "mint program or decimals changed during capture"
    );
    if let Some(expected) = &asset.expected_token_program {
        ensure!(
            *expected == mint_config.token_program,
            "mint does not match configured token program"
        );
    }
    let mut owner_evidence = BTreeMap::new();
    let mut min_slot = initial_slot;
    let mut max_slot = enumeration_slot.max(slot(&last.result)?);
    for e in &evidence[3..evidence.len() - 1] {
        ensure!(
            e.method == "getMultipleAccounts" && e.params[1] == config(Some(enumeration_slot)),
            "invalid authority query"
        );
        let s = slot(&e.result)?;
        ensure!(s >= enumeration_slot, "authority slot predates enumeration");
        min_slot = min_slot.min(s);
        max_slot = max_slot.max(s);
        let addresses = e.params[0]
            .as_array()
            .context("authority addresses missing")?;
        let values = e.result["value"]
            .as_array()
            .context("authority values missing")?;
        ensure!(
            !addresses.is_empty() && addresses.len() <= 100 && addresses.len() == values.len(),
            "incomplete authority batch"
        );
        for (index, address) in addresses.iter().enumerate() {
            let address = address
                .as_str()
                .context("invalid authority query address")?
                .to_string();
            ensure!(
                owner_evidence
                    .insert(
                        address,
                        (
                            values[index].clone(),
                            EvidenceRef {
                                rpc_id: e.id,
                                pointer: format!("/value/{index}"),
                                slot: s
                            }
                        )
                    )
                    .is_none(),
                "duplicate authority evidence"
            );
        }
    }
    let mut entities = Vec::new();
    let mut ids = BTreeSet::new();
    let mut owners = BTreeSet::new();
    let mut summary = SnapshotSummary::default();
    let mut balance_sum = 0u128;
    for (index, entry) in enumeration.result["value"]
        .as_array()
        .context("enumeration value missing")?
        .iter()
        .enumerate()
    {
        let token_account = entry["pubkey"]
            .as_str()
            .context("token account address missing")?
            .to_string();
        let _: Address = token_account
            .parse()
            .context("invalid token account address")?;
        ensure!(
            ids.insert(token_account.clone()),
            "duplicate enumerated token account"
        );
        let state = decode_token_account(
            &entry["account"],
            &mint_config.token_program,
            &asset.mint,
            mint_config.decimals,
        )?;
        owners.insert(state.owner.clone());
        let (authority, authority_ref) = owner_evidence
            .get(&state.owner)
            .context("missing owner authority evidence")?;
        let (entity_type, authority_observation, reason) =
            classify_authority(&state.owner, authority)?;
        match entity_type {
            EntityType::WalletCompatible => summary.wallet_compatible += 1,
            EntityType::ProgramOwnedAuthority => summary.program_owned_authority += 1,
            EntityType::TokenMultisig => summary.token_multisig += 1,
            EntityType::Unknown => summary.unknown += 1,
        }
        summary.active_delegated += usize::from(state.has_active_delegate);
        summary.delegate_present += usize::from(state.delegate.is_some());
        summary.frozen += usize::from(state.is_frozen);
        summary.uninitialized += usize::from(!state.is_initialized);
        let balance: u64 = state.raw_balance.parse()?;
        summary.zero_balance += usize::from(balance == 0);
        balance_sum = balance_sum
            .checked_add(u128::from(balance))
            .context("balance sum overflow")?;
        for ext in &state.extensions {
            *summary
                .account_extensions
                .entry(ext.extension_type.clone())
                .or_default() += 1;
        }
        entities.push(LifecycleEntity {
            id: format!("solana-token-account:{token_account}"),
            entity_type,
            token_account,
            state,
            authority_observation,
            classification_reason: reason,
            token_account_evidence: EvidenceRef {
                rpc_id: enumeration.id,
                pointer: format!("/value/{index}/account"),
                slot: enumeration_slot,
            },
            authority_evidence: authority_ref.clone(),
        });
    }
    ensure!(
        owners.len() == owner_evidence.len(),
        "unrelated authority evidence included"
    );
    entities.sort_by(|a, b| a.token_account.cmp(&b.token_account));
    summary.token_accounts = entities.len();
    summary.distinct_owner_authorities = owners.len();
    summary.summed_public_raw_balance = balance_sum.to_string();
    summary.warnings.push("Classification concerns authority accounts, not human holders, PDA signing programs or liquidity positions. Delegated/frozen/zero counts overlap entity types.".into());
    if min_slot != max_slot {
        summary.warnings.push("Standard RPC queries observed multiple finalized contexts. This frozen observation set is not an atomic single-slot world state; minContextSlot is a lower bound only.".into());
    }
    if initial != mint_config {
        summary.warnings.push("Mint configuration changed between initial and final reads; both raw observations are retained. Final read defines mint_config.".into());
    }
    if balance_sum.to_string() != mint_config.raw_supply {
        summary.warnings.push("Summed public account balances differ from mint supply. Withheld fees, encrypted balances and context drift can explain this; no missing-account inference is made.".into());
    }
    if mint_config
        .extensions
        .iter()
        .any(|e| e.extension_type.starts_with("Confidential"))
        || summary
            .account_extensions
            .keys()
            .any(|k| k.starts_with("Confidential"))
    {
        summary.warnings.push("Confidential configurations and ciphertext are preserved. Encrypted balances cannot be recovered from public RPC; raw_balance covers only the public balance.".into());
    }
    if mint_config
        .extensions
        .iter()
        .any(|e| ["ScaledUiAmount", "InterestBearingConfig"].contains(&e.extension_type.as_str()))
    {
        summary.warnings.push("UI amounts are exact decimal base units. Scaled/interest-bearing display transforms are recorded as mint configuration and are not applied.".into());
    }
    Ok(LifecycleSnapshot {
        schema_version: 1,
        asset,
        captured_at,
        slot: enumeration_slot,
        source: SnapshotSource {
            rpc_origin,
            genesis_hash,
            commitment: "finalized".into(),
            enumeration_slot,
            min_observed_slot: min_slot,
            max_observed_slot: max_slot,
            consistency: if min_slot == max_slot {
                "SameRpcContext"
            } else {
                "MultipleRpcContexts"
            }
            .into(),
        },
        mint_config,
        mint_evidence: EvidenceRef {
            rpc_id: last.id,
            pointer: "/value".into(),
            slot: slot(&last.result)?,
        },
        entities,
        summary,
        evidence,
        exposures: None,
    })
}
