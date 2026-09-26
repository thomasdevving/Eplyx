//! One protocol-position shape and exact native withdrawal evidence; no portfolio inheritance.
pub use crate::protocol::meteora_dlmm::position as meteora_dlmm;

use crate::{
    lifecycle::frozen::selection::canonical,
    lifecycle::resolution::{PathStatus, SignerAssumption},
    path::ExitPathType,
    types::{AccountSnapshot, InstructionSpec},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorityModel {
    DirectSigner,
    PDA,
    ProgramControlled,
    Multisig,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolPosition {
    pub schema_version: u32,
    pub position_id: String,
    pub protocol: String,
    pub pool: String,
    pub authority: String,
    pub authority_model: AuthorityModel,
    pub signer: SignerAssumption,
    pub assets: [String; 2],
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
    pub liquidity_shares: Vec<String>,
    pub principal_exposure_raw: [String; 2],
    pub pending_fees_raw: [String; 2],
    pub calculated_accrued_fees_raw: [String; 2],
    pub position_account_fields: Value,
    pub snapshot_sha256: String,
    pub scenario_sha256: String,
    pub discovery_sha256: String,
    pub fixture_sha256: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub captured_slot: u64,
    pub captured_at: String,
    pub raw_position_sha256: String,
    pub selection_reason: String,
    pub lifecycle_status: crate::lifecycle::policy::LifecycleStatus,
    pub policy_evaluated_at: String,
    pub limitations: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawalProbe {
    pub position_id: String,
    pub pool: String,
    pub authority: String,
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
    pub bps_to_remove: u16,
    pub compute_unit_limit: u32,
}
impl WithdrawalProbe {
    pub fn full(position: &ProtocolPosition) -> Self {
        Self {
            position_id: position.position_id.clone(),
            pool: position.pool.clone(),
            authority: position.authority.clone(),
            lower_bin_id: position.lower_bin_id,
            upper_bin_id: position.upper_bin_id,
            bps_to_remove: 10_000,
            compute_unit_limit: 1_400_000,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionPath {
    pub path_type: ExitPathType,
    pub status: PathStatus,
    pub reason: String,
}
/// Exact scope is checked again before any execution evidence can enter a path row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawalScope {
    pub position_id: String,
    pub pool: String,
    pub authority: String,
    pub fixture_sha256: String,
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
    pub bps_to_remove: u16,
}
pub use crate::protocol::meteora_dlmm::position::VerifiedWithdrawal;
pub fn resolve_position_paths(
    scope: &WithdrawalScope,
    proof: Option<&VerifiedWithdrawal>,
) -> Vec<PositionPath> {
    let withdrawal = proof
        .filter(|p| p.path_type() == ExitPathType::Withdrawal && p.scope() == scope)
        .map_or(PathStatus::NotTested, |p| p.status());
    [
        (ExitPathType::OfficialTransition, PathStatus::NotTested, "No official conversion execution for this position; withdrawal does not establish lifecycle completion."),
        (ExitPathType::Redemption, PathStatus::Unsupported, "No independent redemption adapter or issuer entitlement evidence."),
        (ExitPathType::SecondaryMarketExit, PathStatus::NotApplicable, "This LP position requires native unwind before a separate holder market-exit path; no sale is executed."),
        (ExitPathType::Transfer, PathStatus::NotApplicable, "Direct SPL token transfer is not this protocol-position unwind; position transfer is outside the tested path."),
        (ExitPathType::Withdrawal, withdrawal, "Native withdrawal evidence applies only to the exact position, pool, authority, fixture, range and fraction."),
    ].into_iter().map(|(path_type,status,reason)|PositionPath{path_type,status,reason:reason.into()}).collect()
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountEvidence {
    pub address: String,
    pub pointer: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub slot: u64,
    pub exists: bool,
    pub runtime_owner: Option<String>,
    pub raw_data_sha256: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramEvidence {
    pub program: String,
    pub loader: String,
    pub programdata: Option<String>,
    #[serde(with = "crate::numfmt::optional_u64_string")]
    pub deployment_slot: Option<u64>,
    pub upgrade_authority: Option<String>,
    pub elf_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenReconciliation {
    pub mint: String,
    pub destination: String,
    pub reserve: String,
    pub destination_before_raw: String,
    pub destination_after_raw: String,
    pub user_credit_raw: String,
    pub destination_withheld_before_raw: String,
    pub destination_withheld_after_raw: String,
    pub withheld_credit_raw: String,
    pub reserve_before_raw: String,
    pub reserve_after_raw: String,
    pub reserve_debit_raw: String,
    pub protocol_calculated_principal_raw: String,
    pub bin_principal_decrease_raw: String,
    pub transfer_fee_raw: String,
    pub decimal_user_credit: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinReconciliation {
    pub bin_id: i32,
    pub bin_array: String,
    pub shares_before: String,
    pub shares_after: String,
    pub removed_shares: String,
    pub supply_before: String,
    pub supply_after: String,
    pub amounts_before: [String; 2],
    pub amounts_after: [String; 2],
    pub principal_removed_raw: [String; 2],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawalReport {
    pub schema_version: u32,
    pub position: ProtocolPosition,
    pub probe: WithdrawalProbe,
    pub scope: WithdrawalScope,
    pub status: PathStatus,
    pub execution_attempted: bool,
    pub precondition_error: Option<String>,
    pub captured_clock: Option<crate::path::ProbeClock>,
    pub account_evidence: Vec<AccountEvidence>,
    pub programs: Vec<ProgramEvidence>,
    pub instructions: Vec<InstructionSpec>,
    pub normalized_message: Option<Value>,
    pub local_accounts: Vec<crate::types::NamedAccount>,
    pub execution: Option<crate::path::record::RecordedExecution>,
    pub token_reconciliation: Vec<TokenReconciliation>,
    pub bin_reconciliation: Vec<BinReconciliation>,
    pub post_position_fields: Option<Value>,
    pub position_retained: Option<bool>,
    pub all_position_liquidity_removed: Option<bool>,
    pub fees_claimed_raw: Option<[String; 2]>,
    pub rollback_verified: Option<bool>,
    pub watched_state_changes: BTreeMap<String, Value>,
    pub paths: Vec<PositionPath>,
    pub limitations: Vec<String>,
}
impl WithdrawalReport {
    pub fn to_json(&self) -> Result<String> {
        canonical(self)
    }
    pub fn render_text(&self) -> String {
        let mut s = format!(
            "Position {}\nPool {}\nWithdrawal {:?}\n",
            self.position.position_id, self.position.pool, self.status
        );
        for p in &self.paths {
            s.push_str(&format!("{:?}: {:?}\n", p.path_type, p.status));
        }
        s
    }
}
pub(crate) fn account_json(a: &AccountSnapshot) -> Value {
    use base64::{engine::general_purpose::STANDARD, Engine};
    serde_json::json!({"lamports":a.lamports,"owner":a.owner,"executable":a.executable,
        "rentEpoch":a.rent_epoch,"data":[STANDARD.encode(&a.data),"base64"]})
}
