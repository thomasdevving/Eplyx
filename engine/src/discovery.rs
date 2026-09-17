//! Deterministic mainnet interaction classification and representative selection.
//!
//! This module deliberately stops before stateful execution. A discovery item
//! can be valuable even when standard RPC cannot provide its historical
//! pre-state; eligibility makes that limitation machine-readable.

use crate::{
    ingest::{read_json, transactions::HistoricalTransaction, IngestManifest},
    replay::{hash_bytes, ReplayFidelity, ReplayRecord, ReplayStateSource},
    types::{AccountMetaSpec, InstructionSpec},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const DISCOVERY_SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionType {
    DirectInteraction,
    CpiInteraction,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayEligibility {
    /// Exact historical state is available for this interaction.
    ///
    /// Named for the *state*, not the fidelity outcome: whether a replay
    /// reproduces the original is decided later by `ReplayFidelity`, and an
    /// archive record that does reproduce it is `Matched`, never `Exact`.
    /// The old `exact_ready` spelling is accepted on read so records written
    /// before the rename still load.
    #[serde(alias = "exact_ready")]
    HistoricalStateReady,
    ReconstructedReady,
    ApproximateOnly,
    MissingState,
    UnsupportedTransaction,
    UnsupportedCpi,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionFingerprint {
    pub id: String,
    pub program_id: String,
    pub data_prefix_hex: String,
    pub data_length: u64,
    pub account_count: u64,
    pub signer_count: u64,
    pub writable_count: u64,
    /// `s` signer, `w` writable, `r` readonly; order is instruction order.
    pub privilege_pattern: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionProvenance {
    pub source_signature: String,
    pub source_slot: u64,
    pub genesis_hash: String,
    pub program_id: String,
    pub rpc_capture_schema: u32,
    pub state_source: Option<ReplayStateSource>,
    pub replay_fidelity: Option<ReplayFidelity>,
    /// Response context slots for current RPC account observations. These are
    /// evidence of when sampling occurred, never the historical transaction slot.
    pub account_observation_context_slots: Vec<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionMetadata {
    pub error: Option<Value>,
    pub log_messages_hash: String,
    /// Adapter-supplied identity. Generic discovery leaves this unknown.
    pub economic_entity_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramInteraction {
    pub id: String,
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub interaction_type: InteractionType,
    pub top_level_programs: Vec<String>,
    pub invoked_programs: Vec<String>,
    pub instruction_fingerprints: Vec<InstructionFingerprint>,
    pub account_keys: Vec<AccountMetaSpec>,
    pub success: bool,
    pub compute_units: Option<u64>,
    pub fee_lamports: u64,
    /// Native balance movement only; not asset value, protocol volume, or TVL.
    pub native_value_lamports: Option<u64>,
    pub transaction_version: String,
    pub cluster_id: String,
    pub replay_eligibility: ReplayEligibility,
    pub provenance: InteractionProvenance,
    pub metadata: InteractionMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionCluster {
    pub id: String,
    pub interaction_type: InteractionType,
    pub transaction_version: String,
    pub success: bool,
    pub instruction_fingerprint_ids: Vec<String>,
    pub account_count: u64,
    pub signer_count: u64,
    pub writable_count: u64,
    pub invoked_programs: Vec<String>,
    pub occurrences: u64,
    pub selected: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub rarity: u64,
    pub compute: u64,
    pub failure: u64,
    pub structural_novelty: u64,
    pub cpi: u64,
    pub native_value: u64,
    pub temporal: u64,
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedInteraction {
    pub interaction: ProgramInteraction,
    pub score: ScoreBreakdown,
    pub selection_reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionPolicy {
    pub name: String,
    pub max_records: u64,
    pub max_per_dedup_bucket: u64,
    pub rare_quota_permille: u64,
    pub high_compute_quota_permille: u64,
    pub failure_quota_permille: u64,
    pub cpi_quota_permille: u64,
    pub high_value_quota_permille: u64,
    pub temporal_quota_permille: u64,
}

impl Default for SelectionPolicy {
    fn default() -> Self {
        Self {
            name: "representative-v1".into(),
            max_records: 250,
            max_per_dedup_bucket: 2,
            rare_quota_permille: 200,
            high_compute_quota_permille: 150,
            failure_quota_permille: 100,
            cpi_quota_permille: 100,
            high_value_quota_permille: 50,
            temporal_quota_permille: 50,
        }
    }
}

impl SelectionPolicy {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.max_records > 0,
            "selection max_records must be positive"
        );
        anyhow::ensure!(
            self.max_per_dedup_bucket > 0,
            "max_per_dedup_bucket must be positive"
        );
        let quotas = [
            self.rare_quota_permille,
            self.high_compute_quota_permille,
            self.failure_quota_permille,
            self.cpi_quota_permille,
            self.high_value_quota_permille,
            self.temporal_quota_permille,
        ];
        anyhow::ensure!(
            quotas.iter().sum::<u64>() <= 1000,
            "selection quotas exceed 1000 permille"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EligibilityStatistics {
    #[serde(alias = "exact_ready")]
    pub historical_state_ready: u64,
    pub reconstructed_ready: u64,
    pub approximate_only: u64,
    pub missing_state: u64,
    pub unsupported_transaction: u64,
    pub unsupported_cpi: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryStatistics {
    pub transactions_discovered: u64,
    pub transactions_normalized: u64,
    pub interaction_clusters: u64,
    pub rare_clusters: u64,
    pub successful_transactions: u64,
    pub failed_transactions: u64,
    pub cpi_interactions: u64,
    pub selected_records: u64,
    pub replay_eligibility: EligibilityStatistics,
    pub observation_count: u64,
    pub unique_economic_entities: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterCoverage {
    pub cluster_id: String,
    pub discovered: u64,
    pub selected: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageStatistics {
    pub clusters_represented: u64,
    pub clusters_total: u64,
    pub rare_clusters_represented: u64,
    pub rare_clusters_total: u64,
    pub historical_failures_discovered: u64,
    pub historical_failures_selected: u64,
    pub compute_deciles_observed: Vec<u64>,
    pub compute_deciles_selected: Vec<u64>,
    pub by_cluster: Vec<ClusterCoverage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverySource {
    pub kind: String,
    pub genesis_hash: String,
    /// Hash of the credentialed URL, never the URL itself.
    pub endpoint_sha256: Option<String>,
    pub rpc_concurrency: u64,
    pub bounded_retries: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryCorpus {
    pub schema_version: u32,
    pub program_id: String,
    pub start_slot: u64,
    pub end_slot: u64,
    pub source: DiscoverySource,
    pub selection_policy: SelectionPolicy,
    pub clusters: Vec<InteractionCluster>,
    pub selected: Vec<SelectedInteraction>,
    pub statistics: DiscoveryStatistics,
    pub coverage: CoverageStatistics,
    pub notice: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateAcquisitionResult {
    pub state_source: Option<ReplayStateSource>,
    pub accounts_observed: u64,
    pub accounts_missing: u64,
    pub pre_state_hash: Option<String>,
    pub note: String,
}

/// Future archive/reconstruction integrations plug in here. Discovery does not
/// assume that transaction history implies historical account state.
pub trait HistoricalStateProvider {
    fn load_pre_state(&self, interaction: &ProgramInteraction) -> Result<StateAcquisitionResult>;
}

/// Reads the current-state observations already persisted by ingestion. It
/// never upgrades them to historical or exact state.
pub struct CurrentApproximationProvider {
    pub cache_root: PathBuf,
}

impl HistoricalStateProvider for CurrentApproximationProvider {
    fn load_pre_state(&self, interaction: &ProgramInteraction) -> Result<StateAcquisitionResult> {
        let path = self
            .cache_root
            .join("accounts")
            .join(format!("{}.json", interaction.signature));
        if !path.exists() {
            return Ok(StateAcquisitionResult {
                state_source: None,
                accounts_observed: 0,
                accounts_missing: interaction.account_keys.len() as u64,
                pre_state_hash: None,
                note: "no account observation is available".into(),
            });
        }
        let value: Value = read_json(&path)?;
        anyhow::ensure!(
            value["state_source"] == "current_approximation",
            "unsupported state observation source"
        );
        let mut observed = 0_u64;
        let mut missing = 0_u64;
        for sample in value["samples"]
            .as_array()
            .context("account observation missing samples")?
        {
            for account in sample["response"]["value"]
                .as_array()
                .context("account observation missing response values")?
            {
                if account.is_null() {
                    missing += 1;
                } else {
                    observed += 1;
                }
            }
        }
        Ok(StateAcquisitionResult {
            state_source: Some(ReplayStateSource::CurrentApproximation),
            accounts_observed: observed,
            accounts_missing: missing,
            pre_state_hash: None,
            note: "sampled at current RPC context slots; not historical pre-state".into(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct ClusterShape<'a> {
    interaction_type: InteractionType,
    version: &'a str,
    success: bool,
    fingerprints: Vec<&'a str>,
    account_count: usize,
    signer_count: usize,
    writable_count: usize,
    invoked_programs: Vec<&'a str>,
}

fn stable_unique(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub fn instruction_fingerprint(instruction: &InstructionSpec) -> InstructionFingerprint {
    let prefix_len = instruction.data.len().min(8);
    let data_prefix_hex = crate::hexfmt::encode(&instruction.data[..prefix_len]);
    let privilege_pattern: String = instruction
        .accounts
        .iter()
        .map(|account| {
            if account.is_signer {
                's'
            } else if account.is_writable {
                'w'
            } else {
                'r'
            }
        })
        .collect();
    let signer_count = instruction.accounts.iter().filter(|a| a.is_signer).count() as u64;
    let writable_count = instruction
        .accounts
        .iter()
        .filter(|a| a.is_writable)
        .count() as u64;
    let identity = serde_json::to_vec(&(
        "instruction-fingerprint-v1",
        &instruction.program,
        &data_prefix_hex,
        instruction.data.len(),
        instruction.accounts.len(),
        signer_count,
        writable_count,
        &privilege_pattern,
    ))
    .expect("fingerprint input is serializable");
    InstructionFingerprint {
        id: format!("shape-{}", &hash_bytes(&identity)[..16]),
        program_id: instruction.program.clone(),
        data_prefix_hex,
        data_length: instruction.data.len() as u64,
        account_count: instruction.accounts.len() as u64,
        signer_count,
        writable_count,
        privilege_pattern,
    }
}

pub fn interaction_type(transaction: &HistoricalTransaction, program: &str) -> InteractionType {
    if transaction
        .instructions
        .iter()
        .any(|instruction| instruction.program == program)
    {
        InteractionType::DirectInteraction
    } else if transaction
        .inner_instructions
        .iter()
        .any(|instruction| instruction.program == program)
    {
        InteractionType::CpiInteraction
    } else {
        InteractionType::Unknown
    }
}

pub fn replay_eligibility(
    transaction: &HistoricalTransaction,
    program: &str,
    source: Option<&ReplayStateSource>,
) -> ReplayEligibility {
    let kind = interaction_type(transaction, program);

    // Where an adapter owns this program, *it* defines the replayable contract,
    // and discovery must not answer a question it does not own. The blanket
    // rejection below is the pre-adapter contract: one top-level instruction,
    // no inner instructions. Applying it to an adapter program labelled both
    // committed stake-pool records `unsupported_cpi` while replay accepted and
    // reproduced them, so discovery and replay disagreed about the same
    // transaction.
    if let Some(adapter) = crate::protocol::adapter_for(program) {
        if adapter.accept(transaction).is_err() {
            return ReplayEligibility::UnsupportedTransaction;
        }
        if !transaction.success {
            return ReplayEligibility::UnsupportedTransaction;
        }
        if transaction.version != "legacy" && transaction.loaded_address_count != 0 {
            return ReplayEligibility::UnsupportedTransaction;
        }
        if !transaction.inner_instructions.is_empty() && !adapter.supports_cpi() {
            return ReplayEligibility::UnsupportedCpi;
        }
        return match source {
            Some(ReplayStateSource::ControlledSnapshot | ReplayStateSource::HistoricalArchive) => {
                ReplayEligibility::HistoricalStateReady
            }
            Some(ReplayStateSource::Reconstructed) => ReplayEligibility::ReconstructedReady,
            Some(ReplayStateSource::CurrentApproximation) => ReplayEligibility::ApproximateOnly,
            None => ReplayEligibility::MissingState,
        };
    }

    if kind == InteractionType::CpiInteraction || !transaction.inner_instructions.is_empty() {
        return ReplayEligibility::UnsupportedCpi;
    }
    let bounded_memo_transfer = program == crate::replay::MEMO_PROGRAM_ID
        && transaction.instructions.len() == 2
        && transaction.instructions[0].program == crate::replay::SYSTEM_PROGRAM_ID
        && transaction.instructions[1].program == program;
    let executable_message =
        transaction.version == "legacy" || transaction.loaded_address_count == 0;
    if kind == InteractionType::Unknown
        || !executable_message
        || !transaction.success
        || (transaction.instructions.len() != 1 && !bounded_memo_transfer)
    {
        return ReplayEligibility::UnsupportedTransaction;
    }
    match source {
        Some(ReplayStateSource::ControlledSnapshot | ReplayStateSource::HistoricalArchive) => {
            ReplayEligibility::HistoricalStateReady
        }
        Some(ReplayStateSource::Reconstructed) => ReplayEligibility::ReconstructedReady,
        Some(ReplayStateSource::CurrentApproximation) => ReplayEligibility::ApproximateOnly,
        None => ReplayEligibility::MissingState,
    }
}

fn normalize_interaction(
    transaction: &HistoricalTransaction,
    manifest: &IngestManifest,
    state_source: Option<ReplayStateSource>,
) -> ProgramInteraction {
    let kind = interaction_type(transaction, &manifest.program_id);
    let relevant = match kind {
        InteractionType::DirectInteraction => transaction.instructions.iter(),
        InteractionType::CpiInteraction => transaction.inner_instructions.iter(),
        InteractionType::Unknown => transaction.instructions.iter(),
    }
    .filter(|instruction| instruction.program == manifest.program_id)
    .map(instruction_fingerprint)
    .collect();
    let top_level_programs = stable_unique(
        transaction
            .instructions
            .iter()
            .map(|instruction| instruction.program.clone()),
    );
    let mut invoked_programs = stable_unique(
        transaction
            .instructions
            .iter()
            .chain(&transaction.inner_instructions)
            .map(|instruction| instruction.program.clone()),
    );
    invoked_programs.sort();
    let fidelity = match state_source {
        Some(ReplayStateSource::CurrentApproximation) => Some(ReplayFidelity::Approximate),
        _ => None,
    };
    let eligibility = replay_eligibility(transaction, &manifest.program_id, state_source.as_ref());
    let id_bytes = serde_json::to_vec(&(
        "program-interaction-v1",
        &manifest.genesis_hash,
        &manifest.program_id,
        &transaction.signature,
        transaction.slot,
    ))
    .expect("interaction identity is serializable");
    ProgramInteraction {
        id: format!("interaction-{}", &hash_bytes(&id_bytes)[..20]),
        signature: transaction.signature.clone(),
        slot: transaction.slot,
        block_time: transaction.block_time,
        interaction_type: kind,
        top_level_programs,
        invoked_programs,
        instruction_fingerprints: relevant,
        account_keys: transaction.account_keys.clone(),
        success: transaction.success,
        compute_units: transaction.compute_units,
        fee_lamports: transaction.fee,
        native_value_lamports: transaction.native_value_lamports,
        transaction_version: transaction.version.clone(),
        cluster_id: String::new(),
        replay_eligibility: eligibility,
        provenance: InteractionProvenance {
            source_signature: transaction.signature.clone(),
            source_slot: transaction.slot,
            genesis_hash: manifest.genesis_hash.clone(),
            program_id: manifest.program_id.clone(),
            rpc_capture_schema: manifest.schema_version,
            state_source,
            replay_fidelity: fidelity,
            account_observation_context_slots: Vec::new(),
        },
        metadata: InteractionMetadata {
            error: transaction.error.clone(),
            log_messages_hash: hash_bytes(
                &serde_json::to_vec(&transaction.logs).expect("logs are serializable"),
            ),
            economic_entity_id: None,
        },
    }
}

fn cluster_shape(interaction: &ProgramInteraction) -> ClusterShape<'_> {
    ClusterShape {
        interaction_type: interaction.interaction_type,
        version: &interaction.transaction_version,
        success: interaction.success,
        fingerprints: interaction
            .instruction_fingerprints
            .iter()
            .map(|fingerprint| fingerprint.id.as_str())
            .collect(),
        account_count: interaction.account_keys.len(),
        signer_count: interaction
            .account_keys
            .iter()
            .filter(|account| account.is_signer)
            .count(),
        writable_count: interaction
            .account_keys
            .iter()
            .filter(|account| account.is_writable)
            .count(),
        invoked_programs: interaction
            .invoked_programs
            .iter()
            .map(String::as_str)
            .collect(),
    }
}

fn assign_clusters(interactions: &mut [ProgramInteraction]) -> Vec<InteractionCluster> {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, interaction) in interactions.iter().enumerate() {
        let bytes = serde_json::to_vec(&cluster_shape(interaction)).expect("shape is serializable");
        let id = format!("cluster-{}", &hash_bytes(&bytes)[..16]);
        groups.entry(id).or_default().push(index);
    }
    let mut clusters = Vec::new();
    for (id, members) in groups {
        for index in &members {
            interactions[*index].cluster_id = id.clone();
        }
        let representative = &interactions[members[0]];
        let shape = cluster_shape(representative);
        clusters.push(InteractionCluster {
            id,
            interaction_type: shape.interaction_type,
            transaction_version: shape.version.into(),
            success: shape.success,
            instruction_fingerprint_ids: shape
                .fingerprints
                .into_iter()
                .map(str::to_string)
                .collect(),
            account_count: shape.account_count as u64,
            signer_count: shape.signer_count as u64,
            writable_count: shape.writable_count as u64,
            invoked_programs: shape
                .invoked_programs
                .into_iter()
                .map(str::to_string)
                .collect(),
            occurrences: members.len() as u64,
            selected: 0,
        });
    }
    clusters
}

fn percentiles(values: impl Iterator<Item = Option<u64>>) -> BTreeMap<u64, u64> {
    let mut values: Vec<u64> = values.flatten().collect();
    values.sort_unstable();
    values.dedup();
    let denominator = values.len().saturating_sub(1).max(1) as u64;
    values
        .into_iter()
        .enumerate()
        .map(|(rank, value)| (value, rank as u64 * 1000 / denominator))
        .collect()
}

fn is_rare(occurrences: u64, total: u64) -> bool {
    occurrences <= 3 || occurrences.saturating_mul(100) <= total.max(1)
}

fn score(
    interaction: &ProgramInteraction,
    occurrences: u64,
    total: u64,
    compute_percentiles: &BTreeMap<u64, u64>,
    value_percentiles: &BTreeMap<u64, u64>,
    start_slot: u64,
    end_slot: u64,
) -> (ScoreBreakdown, Vec<String>, u64, u64) {
    let compute_percentile = interaction
        .compute_units
        .and_then(|value| compute_percentiles.get(&value).copied())
        .unwrap_or(0);
    let value_percentile = interaction
        .native_value_lamports
        .and_then(|value| value_percentiles.get(&value).copied())
        .unwrap_or(0);
    let rarity = if occurrences == 1 {
        3000
    } else if occurrences <= 3 {
        2200
    } else if occurrences.saturating_mul(100) <= total.max(1) {
        1400
    } else {
        300
    };
    let compute = compute_percentile.saturating_mul(2);
    let failure = if interaction.success { 0 } else { 1800 };
    let structural_novelty = if occurrences <= 2 { 1200 } else { 0 };
    let cpi = if interaction.interaction_type == InteractionType::CpiInteraction {
        700
    } else {
        0
    };
    let native_value = value_percentile;
    let span = end_slot.saturating_sub(start_slot).max(1);
    let offset = interaction.slot.saturating_sub(start_slot).min(span);
    let temporal_decile = offset.saturating_mul(10) / span;
    let temporal = if temporal_decile == 0 || temporal_decile >= 9 {
        500
    } else {
        0
    };
    let total_score =
        rarity + compute + failure + structural_novelty + cpi + native_value + temporal;
    let mut reasons = Vec::new();
    if is_rare(occurrences, total) {
        reasons.push(format!("rare_instruction_shape_{occurrences}_occurrences"));
    }
    if compute_percentile >= 900 && interaction.compute_units.is_some() {
        reasons.push(format!(
            "high_compute_percentile_{compute_percentile}_permille"
        ));
    }
    if !interaction.success {
        reasons.push("historical_transaction_failed".into());
    }
    if structural_novelty > 0 {
        reasons.push("unusual_account_or_program_shape".into());
    }
    if cpi > 0 {
        reasons.push("cpi_interaction".into());
    }
    if value_percentile >= 900 && interaction.native_value_lamports.is_some() {
        reasons.push(format!(
            "high_native_movement_percentile_{value_percentile}_permille"
        ));
    }
    if temporal > 0 {
        reasons.push(format!("temporal_decile_{temporal_decile}"));
    }
    if reasons.is_empty() {
        reasons.push("common_cluster_representative".into());
    }
    (
        ScoreBreakdown {
            rarity,
            compute,
            failure,
            structural_novelty,
            cpi,
            native_value,
            temporal,
            total: total_score,
        },
        reasons,
        compute_percentile,
        value_percentile,
    )
}

#[derive(Clone)]
struct Candidate {
    index: usize,
    score: ScoreBreakdown,
    reasons: Vec<String>,
    compute_percentile: u64,
    value_percentile: u64,
    temporal_decile: u64,
    dedup_key: String,
}

type SelectionPredicate<'a> = Box<dyn Fn(&Candidate, &ProgramInteraction) -> bool + 'a>;

fn quota(limit: usize, permille: u64) -> usize {
    if permille == 0 {
        0
    } else {
        (limit.saturating_mul(permille as usize) / 1000).max(1)
    }
}

fn select(
    interactions: &[ProgramInteraction],
    clusters: &[InteractionCluster],
    policy: &SelectionPolicy,
    start_slot: u64,
    end_slot: u64,
) -> Vec<SelectedInteraction> {
    let limit = usize::try_from(policy.max_records)
        .unwrap_or(usize::MAX)
        .min(interactions.len());
    let counts: BTreeMap<_, _> = clusters
        .iter()
        .map(|cluster| (cluster.id.clone(), cluster.occurrences))
        .collect();
    let compute_percentiles = percentiles(interactions.iter().map(|item| item.compute_units));
    let value_percentiles = percentiles(interactions.iter().map(|item| item.native_value_lamports));
    let span = end_slot.saturating_sub(start_slot).max(1);
    let mut candidates: Vec<_> = interactions
        .iter()
        .enumerate()
        .map(|(index, interaction)| {
            let occurrences = counts[&interaction.cluster_id];
            let (score, reasons, compute_percentile, value_percentile) = score(
                interaction,
                occurrences,
                interactions.len() as u64,
                &compute_percentiles,
                &value_percentiles,
                start_slot,
                end_slot,
            );
            let temporal_decile = interaction
                .slot
                .saturating_sub(start_slot)
                .min(span)
                .saturating_mul(10)
                / span;
            let compute_band = compute_percentile / 200;
            let value_band = value_percentile / 200;
            let dedup_key = format!(
                "{}:{compute_band}:{value_band}:{temporal_decile}",
                interaction.cluster_id
            );
            Candidate {
                index,
                score,
                reasons,
                compute_percentile,
                value_percentile,
                temporal_decile,
                dedup_key,
            }
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .total
            .cmp(&a.score.total)
            .then(interactions[b.index].slot.cmp(&interactions[a.index].slot))
            .then(
                interactions[a.index]
                    .signature
                    .cmp(&interactions[b.index].signature),
            )
    });

    let mut chosen = BTreeSet::new();
    let mut dedup_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut add = |candidate: &Candidate, force_cluster_coverage: bool| {
        if chosen.len() >= limit || chosen.contains(&candidate.index) {
            return false;
        }
        let count = dedup_counts.get(&candidate.dedup_key).copied().unwrap_or(0);
        if !force_cluster_coverage && count >= policy.max_per_dedup_bucket {
            return false;
        }
        chosen.insert(candidate.index);
        dedup_counts.insert(candidate.dedup_key.clone(), count + 1);
        true
    };

    // First preserve interaction-family coverage. Rare clusters go first when
    // the corpus is smaller than the number of clusters.
    let mut cluster_order: Vec<_> = clusters.iter().collect();
    cluster_order.sort_by(|a, b| a.occurrences.cmp(&b.occurrences).then(a.id.cmp(&b.id)));
    for cluster in cluster_order {
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| interactions[candidate.index].cluster_id == cluster.id)
        {
            add(candidate, true);
        }
    }

    let passes: Vec<(usize, SelectionPredicate<'_>)> = vec![
        (
            quota(limit, policy.rare_quota_permille),
            Box::new(|_, item| counts[&item.cluster_id] <= 3),
        ),
        (
            quota(limit, policy.high_compute_quota_permille),
            Box::new(|candidate, _| candidate.compute_percentile >= 900),
        ),
        (
            quota(limit, policy.failure_quota_permille),
            Box::new(|_, item| !item.success),
        ),
        (
            quota(limit, policy.cpi_quota_permille),
            Box::new(|_, item| item.interaction_type == InteractionType::CpiInteraction),
        ),
        (
            quota(limit, policy.high_value_quota_permille),
            Box::new(|candidate, item| {
                item.native_value_lamports.is_some() && candidate.value_percentile >= 900
            }),
        ),
        (
            quota(limit, policy.temporal_quota_permille),
            Box::new(|candidate, _| {
                candidate.temporal_decile == 0 || candidate.temporal_decile >= 9
            }),
        ),
    ];
    for (target, predicate) in passes {
        let mut added = 0;
        for candidate in &candidates {
            if added >= target {
                break;
            }
            let item = &interactions[candidate.index];
            if predicate(candidate, item) && add(candidate, false) {
                added += 1;
            }
        }
    }
    for candidate in &candidates {
        add(candidate, false);
    }

    let candidate_by_index: BTreeMap<_, _> = candidates
        .into_iter()
        .map(|candidate| (candidate.index, candidate))
        .collect();
    let mut selected: Vec<_> = chosen
        .into_iter()
        .map(|index| {
            let candidate = &candidate_by_index[&index];
            SelectedInteraction {
                interaction: interactions[index].clone(),
                score: candidate.score.clone(),
                selection_reasons: candidate.reasons.clone(),
            }
        })
        .collect();
    selected.sort_by(|a, b| {
        b.score
            .total
            .cmp(&a.score.total)
            .then(a.interaction.slot.cmp(&b.interaction.slot))
            .then(a.interaction.signature.cmp(&b.interaction.signature))
    });
    selected
}

fn eligibility_statistics(selected: &[SelectedInteraction]) -> EligibilityStatistics {
    let mut result = EligibilityStatistics {
        historical_state_ready: 0,
        reconstructed_ready: 0,
        approximate_only: 0,
        missing_state: 0,
        unsupported_transaction: 0,
        unsupported_cpi: 0,
    };
    for item in selected {
        match item.interaction.replay_eligibility {
            ReplayEligibility::HistoricalStateReady => result.historical_state_ready += 1,
            ReplayEligibility::ReconstructedReady => result.reconstructed_ready += 1,
            ReplayEligibility::ApproximateOnly => result.approximate_only += 1,
            ReplayEligibility::MissingState => result.missing_state += 1,
            ReplayEligibility::UnsupportedTransaction => result.unsupported_transaction += 1,
            ReplayEligibility::UnsupportedCpi => result.unsupported_cpi += 1,
        }
    }
    result
}

pub fn economic_entity_statistics(interactions: &[ProgramInteraction]) -> (u64, Option<u64>) {
    let unique = interactions
        .iter()
        .map(|item| item.metadata.economic_entity_id.as_ref())
        .collect::<Option<BTreeSet<_>>>()
        .map(|values| values.len() as u64);
    (interactions.len() as u64, unique)
}

/// Construct a discovery corpus from provider-independent normalized input.
/// `state_source_for` can expose controlled/imported state without coupling
/// selection to any acquisition implementation.
pub fn build(
    manifest: &IngestManifest,
    policy: SelectionPolicy,
    endpoint_sha256: Option<String>,
    rpc_concurrency: u64,
    bounded_retries: u64,
    state_source_for: impl Fn(&str) -> Option<ReplayStateSource>,
) -> Result<DiscoveryCorpus> {
    anyhow::ensure!(manifest.schema_version == 1, "unsupported ingest schema");
    policy.validate()?;
    anyhow::ensure!(rpc_concurrency > 0, "RPC concurrency must be positive");
    let mut interactions: Vec<_> = manifest
        .transactions
        .iter()
        .map(|transaction| {
            normalize_interaction(
                transaction,
                manifest,
                state_source_for(&transaction.signature),
            )
        })
        .collect();
    interactions.sort_by(|a, b| a.slot.cmp(&b.slot).then(a.signature.cmp(&b.signature)));
    let mut clusters = assign_clusters(&mut interactions);
    let selected = select(
        &interactions,
        &clusters,
        &policy,
        manifest.start_slot,
        manifest.end_slot,
    );
    let selected_by_cluster: BTreeMap<String, u64> =
        selected.iter().fold(BTreeMap::new(), |mut counts, item| {
            *counts
                .entry(item.interaction.cluster_id.clone())
                .or_default() += 1;
            counts
        });
    for cluster in &mut clusters {
        cluster.selected = selected_by_cluster.get(&cluster.id).copied().unwrap_or(0);
    }
    let rare_total = clusters
        .iter()
        .filter(|cluster| is_rare(cluster.occurrences, interactions.len() as u64))
        .count() as u64;
    let rare_selected = clusters
        .iter()
        .filter(|cluster| {
            cluster.selected > 0 && is_rare(cluster.occurrences, interactions.len() as u64)
        })
        .count() as u64;
    let compute_map = percentiles(interactions.iter().map(|item| item.compute_units));
    let mut observed_deciles: Vec<_> = interactions
        .iter()
        .filter_map(|item| item.compute_units)
        .filter_map(|value| compute_map.get(&value))
        .map(|percentile| percentile / 100)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut selected_deciles: Vec<_> = selected
        .iter()
        .filter_map(|item| item.interaction.compute_units)
        .filter_map(|value| compute_map.get(&value))
        .map(|percentile| percentile / 100)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    observed_deciles.sort_unstable();
    selected_deciles.sort_unstable();
    let failures_discovered = interactions.iter().filter(|item| !item.success).count() as u64;
    let failures_selected = selected
        .iter()
        .filter(|item| !item.interaction.success)
        .count() as u64;
    let eligibility = eligibility_statistics(&selected);
    let selected_interactions: Vec<_> = selected
        .iter()
        .map(|item| item.interaction.clone())
        .collect();
    let (observation_count, unique_economic_entities) =
        economic_entity_statistics(&selected_interactions);
    let corpus = DiscoveryCorpus {
        schema_version: DISCOVERY_SCHEMA,
        program_id: manifest.program_id.clone(),
        start_slot: manifest.start_slot,
        end_slot: manifest.end_slot,
        source: DiscoverySource {
            kind: "solana_json_rpc".into(),
            genesis_hash: manifest.genesis_hash.clone(),
            endpoint_sha256,
            rpc_concurrency,
            bounded_retries,
        },
        selection_policy: policy,
        statistics: DiscoveryStatistics {
            transactions_discovered: manifest.transactions.len() as u64,
            transactions_normalized: interactions.len() as u64,
            interaction_clusters: clusters.len() as u64,
            rare_clusters: rare_total,
            successful_transactions: interactions.iter().filter(|item| item.success).count() as u64,
            failed_transactions: failures_discovered,
            cpi_interactions: interactions
                .iter()
                .filter(|item| item.interaction_type == InteractionType::CpiInteraction)
                .count() as u64,
            selected_records: selected.len() as u64,
            replay_eligibility: eligibility,
            observation_count,
            unique_economic_entities,
        },
        coverage: CoverageStatistics {
            clusters_represented: clusters
                .iter()
                .filter(|cluster| cluster.selected > 0)
                .count() as u64,
            clusters_total: clusters.len() as u64,
            rare_clusters_represented: rare_selected,
            rare_clusters_total: rare_total,
            historical_failures_discovered: failures_discovered,
            historical_failures_selected: failures_selected,
            compute_deciles_observed: observed_deciles,
            compute_deciles_selected: selected_deciles,
            by_cluster: clusters
                .iter()
                .map(|cluster| ClusterCoverage {
                    cluster_id: cluster.id.clone(),
                    discovered: cluster.occurrences,
                    selected: cluster.selected,
                })
                .collect(),
        },
        clusters,
        selected,
        notice: "Discovery corpus only. No trusted V1/V2 comparison was performed.".into(),
    };
    validate(&corpus)?;
    Ok(corpus)
}

pub fn build_from_cache(
    cache_root: &Path,
    policy: SelectionPolicy,
    endpoint_sha256: Option<String>,
    rpc_concurrency: u64,
    bounded_retries: u64,
) -> Result<DiscoveryCorpus> {
    let manifest: IngestManifest = read_json(&cache_root.join("manifest.json"))?;
    let mut corpus = build(
        &manifest,
        policy,
        endpoint_sha256,
        rpc_concurrency,
        bounded_retries,
        |signature| {
            cache_root
                .join("accounts")
                .join(format!("{signature}.json"))
                .exists()
                .then_some(ReplayStateSource::CurrentApproximation)
        },
    )?;
    attach_account_context_slots(cache_root, &mut corpus)?;
    validate(&corpus)?;
    Ok(corpus)
}

fn attach_account_context_slots(cache_root: &Path, corpus: &mut DiscoveryCorpus) -> Result<()> {
    for selected in &mut corpus.selected {
        if selected.interaction.provenance.state_source
            != Some(ReplayStateSource::CurrentApproximation)
        {
            continue;
        }
        let path = cache_root
            .join("accounts")
            .join(format!("{}.json", selected.interaction.signature));
        let value: Value = read_json(&path)?;
        let mut slots: Vec<_> = value["samples"]
            .as_array()
            .context("account observation missing samples")?
            .iter()
            .filter_map(|sample| sample["response"]["context"]["slot"].as_u64())
            .collect();
        slots.sort_unstable();
        slots.dedup();
        selected
            .interaction
            .provenance
            .account_observation_context_slots = slots;
    }
    Ok(())
}

/// Build from Phase 4 captures as well as current RPC observations. A capture
/// is promoted only after its complete ReplayRecord provenance validates.
pub fn build_from_cache_and_snapshots(
    cache_root: &Path,
    snapshots: &Path,
    policy: SelectionPolicy,
    endpoint_sha256: Option<String>,
    rpc_concurrency: u64,
    bounded_retries: u64,
) -> Result<DiscoveryCorpus> {
    let manifest: IngestManifest = read_json(&cache_root.join("manifest.json"))?;
    let mut sources = BTreeMap::new();
    for transaction in &manifest.transactions {
        let snapshot = snapshots.join(format!("{}.json", transaction.signature));
        if snapshot.exists() {
            let record: ReplayRecord = read_json(&snapshot)?;
            anyhow::ensure!(
                record.transaction == *transaction
                    && record.genesis_hash == manifest.genesis_hash
                    && record.program_id == manifest.program_id,
                "historical snapshot does not match discovery provenance"
            );
            record.validate()?;
            sources.insert(transaction.signature.clone(), record.state_source.clone());
        } else if cache_root
            .join("accounts")
            .join(format!("{}.json", transaction.signature))
            .exists()
        {
            sources.insert(
                transaction.signature.clone(),
                ReplayStateSource::CurrentApproximation,
            );
        }
    }
    let mut corpus = build(
        &manifest,
        policy,
        endpoint_sha256,
        rpc_concurrency,
        bounded_retries,
        |signature| sources.get(signature).cloned(),
    )?;
    attach_account_context_slots(cache_root, &mut corpus)?;
    validate(&corpus)?;
    Ok(corpus)
}

pub fn validate(corpus: &DiscoveryCorpus) -> Result<()> {
    anyhow::ensure!(
        corpus.schema_version == DISCOVERY_SCHEMA,
        "unsupported discovery schema"
    );
    corpus.selection_policy.validate()?;
    anyhow::ensure!(
        corpus.statistics.selected_records == corpus.selected.len() as u64,
        "selected record count mismatch"
    );
    let ids: BTreeSet<_> = corpus
        .selected
        .iter()
        .map(|item| item.interaction.id.as_str())
        .collect();
    anyhow::ensure!(
        ids.len() == corpus.selected.len(),
        "duplicate selected interaction"
    );
    let clusters: BTreeSet<_> = corpus
        .clusters
        .iter()
        .map(|cluster| cluster.id.as_str())
        .collect();
    anyhow::ensure!(
        clusters.len() == corpus.clusters.len(),
        "duplicate cluster id"
    );
    for item in &corpus.selected {
        anyhow::ensure!(
            item.interaction.provenance.source_signature == item.interaction.signature
                && item.interaction.provenance.source_slot == item.interaction.slot
                && item.interaction.provenance.program_id == corpus.program_id
                && clusters.contains(item.interaction.cluster_id.as_str()),
            "selected interaction provenance mismatch"
        );
        anyhow::ensure!(
            item.score.total
                == item.score.rarity
                    + item.score.compute
                    + item.score.failure
                    + item.score.structural_novelty
                    + item.score.cpi
                    + item.score.native_value
                    + item.score.temporal,
            "selection score does not match breakdown"
        );
    }
    Ok(())
}

pub fn render_text(corpus: &DiscoveryCorpus) -> String {
    let stats = &corpus.statistics;
    let eligibility = &stats.replay_eligibility;
    format!(
        "EPLYX MAINNET DISCOVERY\n\nProgram:\n{}\n\nWindow:\nslots {}–{}\n\nTransactions discovered:   {}\nNormalized interactions:   {}\nClusters:                  {}\nSuccessful transactions:  {}\nHistorical failures:      {}\nCPI interactions:         {}\n\nREPRESENTATIVE CORPUS\n\nSelected interactions:     {}\nCluster coverage:          {} / {}\nRare clusters represented: {} / {}\nHistorical failures kept:  {} / {}\n\nREPLAY ELIGIBILITY\n\nExact ready:               {}\nReconstructed ready:       {}\nApproximate only:          {}\nMissing state:             {}\nUnsupported transaction:  {}\nUnsupported CPI:          {}\nUnique economic entities: {}\n\nDiscovery complete.\nNo trusted V1/V2 comparison performed because sufficient historical pre-state was not supplied.\n",
        corpus.program_id,
        corpus.start_slot,
        corpus.end_slot,
        stats.transactions_discovered,
        stats.transactions_normalized,
        stats.interaction_clusters,
        stats.successful_transactions,
        stats.failed_transactions,
        stats.cpi_interactions,
        stats.selected_records,
        corpus.coverage.clusters_represented,
        corpus.coverage.clusters_total,
        corpus.coverage.rare_clusters_represented,
        corpus.coverage.rare_clusters_total,
        corpus.coverage.historical_failures_selected,
        corpus.coverage.historical_failures_discovered,
        eligibility.historical_state_ready,
        eligibility.reconstructed_ready,
        eligibility.approximate_only,
        eligibility.missing_state,
        eligibility.unsupported_transaction,
        eligibility.unsupported_cpi,
        stats
            .unique_economic_entities
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unknown".into()),
    )
}
