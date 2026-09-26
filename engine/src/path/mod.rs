//! Concrete execution evidence, separate from issuer lifecycle transition truth.
pub(crate) mod captured;
pub mod position;
pub mod record;
pub use crate::protocol::meteora_dlmm::swap as meteora_dlmm;
pub mod token_transfer;

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

use crate::{
    executor::{LoadedProgram, ProbeTransactionExecution},
    lifecycle::{
        consequence::{LifecycleExecutionStatus, LifecycleImpact},
        exposure::sha256,
        policy::LifecycleScenario,
        LifecycleSnapshot, RpcEvidence,
    },
    types::NamedAccount,
};
use solana_clock::Clock;
use solana_message::Message;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitPathType {
    OfficialTransition,
    SecondaryMarketExit,
    Redemption,
    Withdrawal,
    Transfer,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeExecutionStatus {
    Succeeded,
    Failed,
    Indeterminate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeAdapter {
    MeteoraDlmm,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProbeSpec {
    pub schema_version: u32,
    pub id: String,
    pub path_type: ExitPathType,
    pub adapter: ProbeAdapter,
    pub target_entity: String,
    pub pool: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount_raw: String,
    pub minimum_output_raw: String,
    pub amount_reason: String,
    pub snapshot_sha256: String,
    pub scenario_sha256: String,
    pub fixture: String,
    pub fixture_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedExecutionFixture {
    pub schema_version: u32,
    pub decoder_revision: String,
    pub captured_at: DateTime<Utc>,
    pub rpc_origin: String,
    pub evidence: Vec<RpcEvidence>,
}
impl CapturedExecutionFixture {
    pub fn to_json(&self) -> Result<String> {
        crate::canonical::document(self)
    }
    pub fn sha256(&self) -> Result<String> {
        Ok(sha256(self.to_json()?.as_bytes()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbePrecondition {
    pub name: String,
    pub proven: bool,
    pub reason: String,
}

pub struct ProbeExecutionPlan {
    pub accounts: Vec<NamedAccount>,
    pub watch: Vec<String>,
    pub programs: Vec<LoadedProgram>,
    pub message: Message,
    pub clock: Clock,
    pub preconditions: Vec<ProbePrecondition>,
    pub account_evidence: Vec<ExecutionAccountEvidence>,
    pub assumptions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeMessage {
    pub required_signatures: u8,
    pub readonly_signed_accounts: u8,
    pub readonly_unsigned_accounts: u8,
    pub account_keys: Vec<String>,
    pub recent_blockhash: String,
    pub instructions: Vec<ProbeCompiledInstruction>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeCompiledInstruction {
    pub program_index: u8,
    pub account_indices: Vec<u8>,
    #[serde(with = "crate::hexfmt")]
    pub data: Vec<u8>,
}
impl From<&Message> for ProbeMessage {
    fn from(m: &Message) -> Self {
        Self {
            required_signatures: m.header.num_required_signatures,
            readonly_signed_accounts: m.header.num_readonly_signed_accounts,
            readonly_unsigned_accounts: m.header.num_readonly_unsigned_accounts,
            account_keys: m.account_keys.iter().map(ToString::to_string).collect(),
            recent_blockhash: m.recent_blockhash.to_string(),
            instructions: m
                .instructions
                .iter()
                .map(|i| ProbeCompiledInstruction {
                    program_index: i.program_id_index,
                    account_indices: i.accounts.clone(),
                    data: i.data.clone(),
                })
                .collect(),
        }
    }
}
pub use crate::standard_programs::clock::CapturedClock as ProbeClock;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAccountEvidence {
    pub address: String,
    pub rpc_record: usize,
    pub pointer: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub slot: u64,
    pub exists: bool,
    pub runtime_owner: Option<String>,
    pub raw_data_sha256: Option<String>,
}

/// Adapter-specific planning/account rules; shared backend and status semantics.
pub trait ExecutionProbe {
    fn build_execution(
        &self,
        snapshot: &LifecycleSnapshot,
        spec: &ExecutionProbeSpec,
        fixture: &CapturedExecutionFixture,
    ) -> Result<ProbeExecutionPlan>;
    fn classify_result(
        &self,
        spec: &ExecutionProbeSpec,
        plan: &ProbeExecutionPlan,
        execution: &ProbeTransactionExecution,
    ) -> Result<ExecutionDeltas>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenAccountDelta {
    pub address: String,
    pub mint: String,
    pub before_raw: String,
    pub after_raw: String,
    pub change_raw: String,
    pub change_decimal_base_units: String,
    pub withheld_fee_change_raw: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ByteRangeDelta {
    pub offset: usize,
    #[serde(with = "crate::hexfmt")]
    pub before: Vec<u8>,
    #[serde(with = "crate::hexfmt")]
    pub after: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountDataDelta {
    pub address: String,
    pub before_sha256: String,
    pub after_sha256: String,
    pub changed_ranges: Vec<ByteRangeDelta>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwapFeeDeltas {
    pub fee_asset_mint: String,
    pub token_2022_transfer_fee_raw: String,
    pub dlmm_swap_fee_raw: String,
    pub dlmm_protocol_fee_raw: String,
    pub dlmm_liquidity_provider_fee_raw: String,
    pub host_fee_raw: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub active_epoch: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionDeltas {
    pub input_debited_raw: String,
    pub output_received_raw: String,
    pub output_decimal_base_units: String,
    pub token_accounts: Vec<TokenAccountDelta>,
    pub account_data: Vec<AccountDataDelta>,
    pub fees: Option<SwapFeeDeltas>,
    pub reconciled: bool,
    pub reconciliation: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleExecutionAssessment {
    /// Original Phase 4 judgment, retained as baseline. The wrapper adds path-specific
    /// execution evidence without changing an untested official lifecycle transition.
    pub entity_impact: LifecycleImpact,
    pub tested_path: ExitPathType,
    pub secondary_market_exit: ProbeExecutionStatus,
    pub official_transition: LifecycleExecutionStatus,
    pub source_balance_at_phase5_raw: Option<String>,
    pub venue_vault_exitability_tested: bool,
    pub conclusion: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitabilityReport {
    pub schema_version: u32,
    pub evaluated_at: DateTime<Utc>,
    pub probe: ExecutionProbeSpec,
    pub fixture_sha256: String,
    pub execution_status: ProbeExecutionStatus,
    pub preconditions: Vec<ProbePrecondition>,
    pub assumptions: Vec<String>,
    pub account_evidence: Vec<ExecutionAccountEvidence>,
    pub execution_message: Option<ProbeMessage>,
    pub vm_clock: Option<ProbeClock>,
    pub local_accounts: Vec<NamedAccount>,
    pub execution: Option<record::RecordedExecution>,
    pub deltas: Option<ExecutionDeltas>,
    pub blocker: Option<String>,
    pub lifecycle_assessment: LifecycleExecutionAssessment,
}

pub fn load_probe(path: &Path) -> Result<(ExecutionProbeSpec, CapturedExecutionFixture)> {
    let spec: ExecutionProbeSpec =
        serde_json::from_slice(&crate::lifecycle::artifact::read(path)?)?;
    let bytes = crate::lifecycle::artifact::read(
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&spec.fixture),
    )?;
    ensure!(
        sha256(&bytes) == spec.fixture_sha256,
        "execution fixture file hash mismatch"
    );
    let fixture: CapturedExecutionFixture = serde_json::from_slice(&bytes)?;
    ensure!(
        fixture.sha256()? == spec.fixture_sha256,
        "execution fixture is not canonical JSON"
    );
    Ok((spec, fixture))
}

pub fn run(
    snapshot: &LifecycleSnapshot,
    scenario: &LifecycleScenario,
    spec: &ExecutionProbeSpec,
    fixture: &CapturedExecutionFixture,
    at: DateTime<Utc>,
) -> Result<ExitabilityReport> {
    snapshot.validate()?;
    scenario.validate()?;
    ensure!(
        spec.schema_version == 1 && spec.path_type == ExitPathType::SecondaryMarketExit,
        "only schema-1 SecondaryMarketExit is implemented"
    );
    ensure!(
        spec.snapshot_sha256 == sha256(snapshot.to_json()?.as_bytes())
            && spec.scenario_sha256 == scenario.sha256()?,
        "probe differs from frozen snapshot/scenario fingerprint"
    );
    ensure!(
        spec.fixture_sha256 == fixture.sha256()?,
        "probe fixture fingerprint mismatch"
    );
    let baseline = std::cmp::min(
        at,
        scenario
            .policy
            .effective_at
            .checked_sub_signed(chrono::Duration::nanoseconds(1))
            .context("no lifecycle baseline time")?,
    );
    let impact = crate::lifecycle::consequence::LifecycleConsequenceEvaluator::evaluate(
        snapshot, scenario, baseline, at,
    )?;
    let entity = impact
        .entities
        .into_iter()
        .find(|e| e.entity_id == spec.target_entity)
        .context("probe target absent from lifecycle entities")?;
    let adapter = meteora_dlmm::DexSwapExitProbe;
    let mut status = ProbeExecutionStatus::Indeterminate;
    let mut preconditions = Vec::new();
    let mut assumptions = Vec::new();
    let mut evidence = Vec::new();
    let mut execution = None;
    let mut execution_message = None;
    let mut vm_clock = None;
    let mut local_accounts = Vec::new();
    let mut deltas = None;
    let mut blocker = None;
    match adapter.build_execution(snapshot, spec, fixture) {
        Err(error) => {
            let reason = format!("{error:#}");
            preconditions.push(ProbePrecondition {
                name: "Complete execution preconditions".into(),
                proven: false,
                reason: reason.clone(),
            });
            blocker = Some(reason);
        }
        Ok(plan) => {
            execution_message = Some(ProbeMessage::from(&plan.message));
            vm_clock = Some(ProbeClock::from(&plan.clock));
            local_accounts = plan
                .accounts
                .iter()
                .filter(|a| !plan.account_evidence.iter().any(|e| e.address == a.address))
                .cloned()
                .collect();
            preconditions = plan.preconditions.clone();
            assumptions = plan.assumptions.clone();
            evidence = plan.account_evidence.clone();
            match crate::executor::execute_probe_message(
                &plan.accounts,
                &plan.watch,
                plan.clock.clone(),
                &plan.programs,
                plan.message.clone(),
            ) {
                Err(error) => {
                    blocker = Some(format!(
                        "Local runtime/dependency setup could not execute: {error:#}"
                    ))
                }
                Ok(result) => {
                    status = if result.success {
                        ProbeExecutionStatus::Succeeded
                    } else {
                        ProbeExecutionStatus::Failed
                    };
                    match adapter.classify_result(spec, &plan, &result) {
                        Ok(change) => {
                            if result.success && !change.reconciled {
                                status = ProbeExecutionStatus::Indeterminate;
                                blocker=Some("Successful VM transaction has unverified economic reconciliation".into());
                            }
                            deltas = Some(change);
                        }
                        Err(error) => {
                            status = ProbeExecutionStatus::Indeterminate;
                            blocker = Some(format!(
                                "Executed transaction evidence cannot be reconciled: {error:#}"
                            ));
                        }
                    }
                    execution = Some(result.into());
                }
            }
        }
    }
    let source_balance = deltas
        .as_ref()
        .and_then(|d| {
            d.token_accounts
                .iter()
                .find(|a| a.address == entity.token_account)
        })
        .map(|a| a.before_raw.clone());
    let conclusion=match status {
        ProbeExecutionStatus::Succeeded=>"A bounded secondary-market swap executed with reconciled token deltas in the captured local configuration under the stated signer/runtime assumptions. Official successor transition remains NotTested.",
        ProbeExecutionStatus::Failed=>"The actual local transaction failed under the captured configuration. This does not prove all exits fail or that a position is stranded. Official successor transition remains NotTested.",
        ProbeExecutionStatus::Indeterminate=>"Exitability is indeterminate because required preconditions, executable dependencies or economic reconciliation could not be proved. Official successor transition remains NotTested.",
    }.into();
    Ok(ExitabilityReport {
        schema_version: 1,
        evaluated_at: at,
        probe: spec.clone(),
        fixture_sha256: fixture.sha256()?,
        execution_status: status,
        preconditions,
        assumptions,
        account_evidence: evidence,
        execution_message,
        vm_clock,
        local_accounts,
        execution,
        deltas,
        blocker,
        lifecycle_assessment: LifecycleExecutionAssessment {
            entity_impact: entity,
            tested_path: ExitPathType::SecondaryMarketExit,
            secondary_market_exit: status,
            official_transition: LifecycleExecutionStatus::NotTested,
            source_balance_at_phase5_raw: source_balance,
            venue_vault_exitability_tested: false,
            conclusion,
        },
    })
}

pub fn save_json<T: Serialize>(value: &T, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all((serde_json::to_string_pretty(value)? + "\n").as_bytes())?;
    file.sync_all()?;
    Ok(())
}
impl ExitabilityReport {
    pub fn to_json(&self) -> Result<String> {
        crate::canonical::document(self)
    }
    pub fn validate(
        &self,
        snapshot: &LifecycleSnapshot,
        scenario: &LifecycleScenario,
        fixture: &CapturedExecutionFixture,
    ) -> Result<()> {
        ensure!(
            *self == run(snapshot, scenario, &self.probe, fixture, self.evaluated_at)?,
            "probe report disagrees with offline execution"
        );
        Ok(())
    }
}
