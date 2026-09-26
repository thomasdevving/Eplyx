//! Capture budgets and explicit population/authority completeness, from STA.
use crate::evidence::authority::{AuthorityObservation, EntityType, EvidenceRef};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
/// Whether the token-account scan itself observed everything the query asked for.
///
/// This is about the `getProgramAccounts` enumeration only. Authority resolution
/// has its own independent axis; hitting the authority budget never downgrades
/// this value, because "we do not know how many token accounts exist" and "we
/// have every token account but not every authority model" are different facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EnumerationCompleteness {
    /// The scan returned, every row verified its mint and runtime program owner,
    /// and no response or decode bound was reached. Complete for this exact query
    /// at this context, and never a claim about any later slot.
    CompleteForQuery,
    /// The scan returned but a declared bound was reached, or some rows could not
    /// be verified or decoded. The trustworthy subset is retained.
    Partial,
    /// The provider errored, timed out or rate-limited the scan.
    Unavailable,
    /// The provider or the asset's token program cannot serve this enumeration.
    Unsupported,
}
impl EnumerationCompleteness {
    pub fn key(self) -> &'static str {
        match self {
            Self::CompleteForQuery => "CompleteForQuery",
            Self::Partial => "Partial",
            Self::Unavailable => "Unavailable",
            Self::Unsupported => "Unsupported",
        }
    }
}
/// Whether every recorded authority of the positive-balance population had its
/// own account inspected. Independent of [`EnumerationCompleteness`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AuthorityResolutionCompleteness {
    Complete,
    Partial,
    NotPerformed,
}
impl AuthorityResolutionCompleteness {
    pub fn key(self) -> &'static str {
        match self {
            Self::Complete => "Complete",
            Self::Partial => "Partial",
            Self::NotPerformed => "NotPerformed",
        }
    }
}
/// Whether this entity's authority account was actually inspected in this capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorityResolution {
    Resolved,
    /// Inside the population but outside the declared authority lookup budget.
    /// Unknown, never assumed wallet-compatible and never given an assumed signer.
    NotResolved,
}

/// Every bound this milestone operates under, persisted into the frozen plan so
/// no runtime or resource limit is implicit. Server-controlled: the browser can
/// request a stress test but can never raise, lower or retune a budget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StressBudget {
    pub budget_version: String,
    /// Response byte ceiling for the population scan.
    #[serde(with = "crate::numfmt::u64_string")]
    pub max_response_bytes: u64,
    /// Token-account rows this run will decode. Sized for known large current
    /// populations; reaching it makes enumeration Partial rather than silent.
    pub max_decoded_accounts: usize,
    /// Distinct positive-balance authorities whose accounts are inspected.
    pub max_authority_lookups: usize,
    pub authority_batch_size: usize,
    pub max_selected_cases: usize,
    pub executions_per_case: usize,
    pub rpc_requests_per_case: usize,
    pub max_concurrent_rpc_requests: usize,
    pub max_concurrent_vm_executions: usize,
    #[serde(with = "crate::numfmt::u64_string")]
    pub population_timeout_seconds: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub case_timeout_seconds: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub max_artifact_bytes: u64,
}
pub const BUDGET_VERSION: &str = "eplyx-conversion-stress-budget/v1";
impl Default for StressBudget {
    fn default() -> Self {
        Self {
            budget_version: BUDGET_VERSION.into(),
            max_response_bytes: 128 * 1024 * 1024,
            max_decoded_accounts: 100_000,
            max_authority_lookups: 40_000,
            authority_batch_size: 100,
            max_selected_cases: 10,
            executions_per_case: 1,
            rpc_requests_per_case: 5,
            max_concurrent_rpc_requests: 1,
            max_concurrent_vm_executions: 1,
            population_timeout_seconds: 900,
            case_timeout_seconds: 120,
            max_artifact_bytes: 192 * 1024 * 1024,
        }
    }
}
impl StressBudget {
    /// Server-side overrides only, each clamped to the validated range.
    ///
    /// The browser can ask for a stress test but never reaches this: the Node
    /// service passes no budget values through, so every bound here comes from
    /// the operator's own environment or from the defaults above.
    pub fn from_env() -> Self {
        let mut b = Self::default();
        let num = |key: &str| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
        };
        if let Some(v) = num("EPLYX_STRESS_MAX_ACCOUNTS") {
            b.max_decoded_accounts = (v as usize).clamp(1, 500_000);
        }
        if let Some(v) = num("EPLYX_STRESS_MAX_RESPONSE_MB") {
            b.max_response_bytes = v.saturating_mul(1024 * 1024).clamp(1, 512 * 1024 * 1024);
        }
        if let Some(v) = num("EPLYX_STRESS_MAX_AUTHORITIES") {
            b.max_authority_lookups = (v as usize).min(b.max_decoded_accounts);
        }
        if let Some(v) = num("EPLYX_STRESS_MAX_CASES") {
            b.max_selected_cases = (v as usize).clamp(1, 32);
        }
        if let Some(v) = num("EPLYX_STRESS_POPULATION_TIMEOUT_SECONDS") {
            b.population_timeout_seconds = v.clamp(1, 1800);
        }
        b.max_authority_lookups = b.max_authority_lookups.min(b.max_decoded_accounts);
        // The capture file holds the scan response, so it is always allowed to be
        // larger than one response by a fixed margin.
        b.max_artifact_bytes = b.max_response_bytes.saturating_add(64 * 1024 * 1024);
        b
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.budget_version == BUDGET_VERSION,
            "unknown stress budget version"
        );
        ensure!(
            self.max_response_bytes > 0
                && self.max_response_bytes <= 512 * 1024 * 1024
                && self.max_decoded_accounts > 0
                && self.max_decoded_accounts <= 500_000
                && self.max_authority_lookups <= self.max_decoded_accounts
                && self.authority_batch_size > 0
                && self.authority_batch_size <= 100,
            "population acquisition budget out of range"
        );
        ensure!(
            self.max_selected_cases > 0
                && self.max_selected_cases <= 32
                && self.executions_per_case == 1
                && self.rpc_requests_per_case == 5,
            "unsupported stress execution budget"
        );
        ensure!(
            self.max_concurrent_rpc_requests == 1 && self.max_concurrent_vm_executions == 1,
            "this milestone executes serially by design"
        );
        ensure!(
            self.population_timeout_seconds > 0
                && self.population_timeout_seconds <= 1800
                && self.case_timeout_seconds > 0
                && self.case_timeout_seconds <= 600
                && self.max_artifact_bytes > 0,
            "stress timeout or artifact budget out of range"
        );
        Ok(())
    }
}

/// One freshly observed current token account. No human holder is inferred, a
/// zero balance is not exposure, and an unknown confidential balance is not zero.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StressEntity {
    pub entity_id: String,
    pub token_account: String,
    pub mint: String,
    pub token_program: String,
    pub authority: String,
    pub authority_model: EntityType,
    pub authority_resolution: AuthorityResolution,
    pub authority_observation: AuthorityObservation,
    pub classification_reason: String,
    pub state: crate::standard_programs::token::TokenAccountState,
    pub token_account_evidence: EvidenceRef,
    pub authority_evidence: Option<EvidenceRef>,
}
impl StressEntity {
    pub fn balance(&self) -> Result<u64> {
        Ok(self.state.raw_balance.parse()?)
    }
}
/// A row the scan returned that this decoder could not turn into a supported
/// token account. Retained separately; never counted as a zero balance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndecodedAccount {
    pub address: String,
    pub reason: String,
    pub raw_data_sha256: Option<String>,
    pub evidence: EvidenceRef,
}

pub fn entity_id(population_sha256: &str, token_account: &str) -> String {
    format!("current-stress:{population_sha256}:{token_account}")
}

pub fn sum_once(balances: &std::collections::BTreeMap<String, u64>) -> String {
    balances
        .values()
        .fold(0u128, |a, b| a + u128::from(*b))
        .to_string()
}
