use crate::{
    acquisition_error::AcquisitionError as AE,
    ingest::transactions::HistoricalTransaction,
    protocol::{self, ProtocolAdapter},
    semantics::EvaluableSubject,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub scope: Scope,
    pub providers: ProviderConfig,
    #[serde(default)]
    pub observed: Option<PathBuf>,
    #[serde(default)]
    pub prepared_corpus: Option<PathBuf>,
    #[serde(default)]
    pub accept_coverage: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub program: String,
    pub actions: Vec<String>,
    pub subjects: Vec<EvaluableSubject>,
    pub start_slot: u64,
    pub end_slot: u64,
    pub discovery_limit: usize,
    pub acquisition_budget: usize,
    pub target_size: usize,
    pub replay_schema: u32,
    pub runtime_profile: String,
}
/// Environment variable names, never endpoint values. Not part of the receipt.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub transaction_url_env: String,
    pub account_url_env: String,
    pub block_url_env: String,
    #[serde(default)]
    pub origin_env: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    ScopeUnqualified,
    SupportBlocked,
    AcquisitionBlocked,
    EvidenceAcquired,
    BundleCandidate,
    BundleVerified,
    CoverageReviewRequired,
    ReadyForActivation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    InvalidConfiguration,
    InvalidObservedPopulation,
    MissingAdapter,
    UnsupportedAction,
    MissingEvaluableSubject,
    MissingSelectedSubject,
    OutsideSchema1QualificationContract,
    PreparedCorpusOutsideAcquisitionContract,
    UnsupportedRuntime,
    ProviderGenesisFailed,
    DiscoveryFailed,
    UnsupportedTransaction,
    UnsupportedCpi,
    UnsupportedLookupTables,
    AdapterRejected,
    TransactionUnavailable,
    TargetExecutableUnavailable,
    DependencyResolutionFailed,
    HistoricalAccountUnavailable,
    ArchiveSlotMismatch,
    CreationClosureUnsupported,
    SameSlotConflict,
    BlockHistoryUnavailable,
    BoundaryProofFailed,
    NoExactHistoricalRecords,
    EvidenceIdentityMismatch,
    IncompatibleEvidenceCohorts,
    NoScopedRecords,
    SelectionFailed,
    BaselineFidelityOrBuildFailed,
    BundleVerificationFailed,
    InternalFailure,
}
impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "qualification: {self:?}")
    }
}
impl std::error::Error for Code {}
impl Code {
    pub fn failure_state(self) -> (State, u8) {
        match self {
            Self::InvalidConfiguration | Self::InvalidObservedPopulation => {
                (State::ScopeUnqualified, 23)
            }
            Self::MissingAdapter
            | Self::UnsupportedAction
            | Self::MissingEvaluableSubject
            | Self::OutsideSchema1QualificationContract
            | Self::PreparedCorpusOutsideAcquisitionContract
            | Self::UnsupportedRuntime => (State::SupportBlocked, 21),
            Self::InternalFailure => (State::ScopeUnqualified, 24),
            _ => (State::AcquisitionBlocked, 22),
        }
    }
    pub fn next_action(self) -> &'static str {
        match self {
            Self::OutsideSchema1QualificationContract=>"use the existing prepared schema-2 bundle build / bundle verify route; no down-conversion",
            Self::IncompatibleEvidenceCohorts=>"choose an explicit window/cohort with one V1 deployment and dependency set; inspect receipt cohorts",
            Self::BaselineFidelityOrBuildFailed=>"inspect per-record fidelity; do not remove a mismatch silently",
            Self::InvalidConfiguration|Self::InvalidObservedPopulation=>"correct the input contract and use a fresh output directory",
            Self::MissingAdapter|Self::UnsupportedAction|Self::MissingEvaluableSubject|Self::UnsupportedRuntime=>"review the missing support contract or explicitly narrow scope",
            _=>"inspect the established evidence and typed refusal ledger; correct the capability or scope and rerun into a fresh directory",
        }
    }
}

/// Qualification metadata for existing schema-1 semantic contracts. This does
/// not admit transactions: adapter.accept and evaluable_subjects remain final.
/// Unknown adapter versions fail closed until this metadata is reviewed.
pub fn subject_action(adapter: &str, action: &str) -> Option<&'static str> {
    match (adapter, action) {
        ("spl-stake-pool", "deposit_sol") => Some("deposit"),
        ("spl-stake-pool", "withdraw_sol") => Some("withdraw"),
        ("kamino-klend", "deposit_reserve_liquidity_and_obligation_collateral") => Some("deposit"),
        _ => None,
    }
}
fn subject_supported(adapter: &str, action: &str, domain: &str, subject: &str) -> bool {
    if subject_action(adapter, action).is_none() {
        return false;
    }
    if domain == "execution" {
        return subject == "transaction";
    }
    if domain != "economic" {
        return false;
    }
    match (adapter, action) {
        ("spl-stake-pool", "deposit_sol") => subject == "pool_tokens_received",
        ("spl-stake-pool", "withdraw_sol") => [
            "pool_tokens_debited",
            "pool_tokens_burned",
            "sol_received_by_user",
        ]
        .contains(&subject),
        ("kamino-klend", "deposit_reserve_liquidity_and_obligation_collateral") => [
            "liquidity_deposited",
            "reserve_liquidity_received",
            "obligation_collateral_deposited",
        ]
        .contains(&subject),
        _ => false,
    }
}
/// Invalid arbitrary strings (including paths/URLs) never get reflected into receipts.
pub fn safe_scope(s: &Scope) -> bool {
    let identifier = |v: &str| {
        !v.is_empty()
            && v.len() <= 128
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    };
    s.program.parse::<solana_address::Address>().is_ok()
        && s.actions.iter().all(|a| identifier(a))
        && s.subjects.iter().all(|v| {
            identifier(v.protocol.as_str())
                && identifier(v.action.as_str())
                && identifier(v.subject.as_str())
        })
        && identifier(&s.runtime_profile)
}

pub fn preflight(config: &Config) -> Result<&'static dyn ProtocolAdapter> {
    let s = &config.scope;
    let env_valid = |v: &str| {
        !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    };
    if !safe_scope(s)
        || ![
            config.providers.transaction_url_env.as_str(),
            config.providers.account_url_env.as_str(),
            config.providers.block_url_env.as_str(),
        ]
        .into_iter()
        .all(env_valid)
        || config
            .providers
            .origin_env
            .as_deref()
            .is_some_and(|e| !env_valid(e))
    {
        return Err(Code::InvalidConfiguration.into());
    }
    if config.schema_version != 1
        || s.start_slot == 0
        || s.start_slot > s.end_slot
        || s.discovery_limit == 0
        || s.acquisition_budget == 0
        || s.target_size == 0
        || s.actions.is_empty()
        || s.subjects.is_empty()
        || s.actions.iter().collect::<BTreeSet<_>>().len() != s.actions.len()
        || s.subjects.iter().collect::<BTreeSet<_>>().len() != s.subjects.len()
    {
        return Err(Code::InvalidConfiguration.into());
    }
    if s.replay_schema != 1 {
        return Err(Code::OutsideSchema1QualificationContract.into());
    }
    if s.runtime_profile != "schema1_litesvm_mainnet" {
        return Err(Code::UnsupportedRuntime.into());
    }
    let adapter = protocol::adapter_for(&s.program).context(Code::MissingAdapter)?;
    match (adapter.name(), adapter.adapter_version()) {
        ("spl-stake-pool", 3) | ("kamino-klend", 1) => {}
        ("orca-whirlpool", _) | ("drift", _) => {
            return Err(Code::OutsideSchema1QualificationContract.into())
        }
        _ => {
            // These adapters explicitly require universal target boundaries.
            if s.program == protocol::orca::PROGRAM_ID || s.program == protocol::drift::PROGRAM_ID {
                return Err(Code::OutsideSchema1QualificationContract.into());
            }
            return Err(Code::MissingEvaluableSubject.into());
        }
    }
    let valid_actions: &[&str] = if adapter.name() == "spl-stake-pool" {
        &["deposit", "withdraw"]
    } else {
        &["deposit"]
    };
    if s.actions
        .iter()
        .any(|a| !valid_actions.contains(&a.as_str()))
    {
        return Err(Code::UnsupportedAction.into());
    }
    for subject in &s.subjects {
        if subject.protocol.as_str() != adapter.name()
            || !subject_supported(
                adapter.name(),
                subject.action.as_str(),
                subject.domain.as_str(),
                subject.subject.as_str(),
            )
            || !s.actions.iter().any(|a| {
                Some(a.as_str()) == subject_action(adapter.name(), subject.action.as_str())
            })
        {
            return Err(Code::MissingEvaluableSubject.into());
        }
    }
    if s.actions.iter().any(|a| {
        !s.subjects
            .iter()
            .any(|s| subject_action(adapter.name(), s.action.as_str()) == Some(a.as_str()))
    }) {
        return Err(Code::MissingEvaluableSubject.into());
    }
    Ok(adapter)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasuredPopulation {
    pub schema_version: u32,
    pub program: String,
    pub adapter: String,
    pub adapter_version: u32,
    /// Stable classifier identifier, not a URL or arbitrary shell invocation.
    pub classifier: String,
    pub actions: Vec<String>,
    pub start_slot: u64,
    pub end_slot: u64,
    pub counts: BTreeMap<String, usize>,
    pub completeness: Completeness,
    pub provenance_sha256: String,
    pub unknown_interactions: usize,
    pub unsupported_interactions: usize,
    pub failed_interactions: usize,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    CompleteDeclaredWindow,
    PartialDeclaredWindow,
}
impl MeasuredPopulation {
    pub fn validate(&self, s: &Scope, a: &dyn ProtocolAdapter) -> Result<()> {
        let keys: BTreeSet<_> = self.counts.keys().collect();
        if self.schema_version != 1
            || self.program != s.program
            || self.adapter != a.name()
            || self.adapter_version != a.adapter_version()
            || self.start_slot != s.start_slot
            || self.end_slot != s.end_slot
            || keys != s.actions.iter().collect()
            || keys != self.actions.iter().collect()
            || self.actions.len() != keys.len()
            || self.classifier.is_empty()
            || self.classifier.len() > 128
            || !self
                .classifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
            || self.provenance_sha256.len() != 64
            || !self
                .provenance_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Code::InvalidObservedPopulation.into());
        }
        Ok(())
    }
}
pub fn transaction_blocker(tx: &HistoricalTransaction, a: &dyn ProtocolAdapter) -> Option<Code> {
    if tx.loaded_address_count > 0 {
        return Some(Code::UnsupportedLookupTables);
    }
    if !matches!(tx.version.as_str(), "legacy" | "v0") || !tx.success {
        return Some(Code::UnsupportedTransaction);
    }
    if !tx.inner_instructions.is_empty() && !a.supports_cpi() {
        return Some(Code::UnsupportedCpi);
    }
    if a.accept(tx).is_err() {
        if tx
            .inner_instruction_frames
            .iter()
            .any(|frame| frame.stack_height > 2)
            || (!tx.instructions.iter().any(|i| i.program == a.program_id())
                && tx
                    .inner_instructions
                    .iter()
                    .any(|i| i.program == a.program_id()))
        {
            return Some(Code::UnsupportedCpi);
        }
        return Some(Code::AdapterRejected);
    }
    None
}
pub fn acquisition_code(error: &anyhow::Error) -> Code {
    // Inspect typed contexts, preferring exact failures over their enclosing stage.
    if error
        .chain()
        .any(|e| e.downcast_ref::<AE>() == Some(&AE::ArchiveSlotMismatch))
    {
        return Code::ArchiveSlotMismatch;
    }
    match error.downcast_ref::<AE>() {
        Some(AE::UnsupportedRuntime) => Code::UnsupportedRuntime,
        Some(AE::InvalidReplayEvidence) => Code::EvidenceIdentityMismatch,
        Some(AE::TransactionUnavailable) => Code::TransactionUnavailable,
        Some(AE::AdapterRejected) => Code::AdapterRejected,
        Some(AE::TargetExecutableUnavailable) => Code::TargetExecutableUnavailable,
        Some(AE::DependencyResolutionFailed) => Code::DependencyResolutionFailed,
        Some(AE::HistoricalAccountUnavailable) => Code::HistoricalAccountUnavailable,
        Some(AE::ArchiveSlotMismatch) => Code::ArchiveSlotMismatch,
        Some(AE::CreationClosureUnsupported) => Code::CreationClosureUnsupported,
        Some(AE::SameSlotConflict) => Code::SameSlotConflict,
        Some(AE::BlockHistoryUnavailable) => Code::BlockHistoryUnavailable,
        Some(AE::BoundaryProofFailed) => Code::BoundaryProofFailed,
        None => Code::TransactionUnavailable,
    }
}
