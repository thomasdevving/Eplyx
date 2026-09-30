//! One executable active-state counterfactual. Never a fee-admin instruction.
use crate::{
    change::{Activation, BoundChange, Change, ChangeBinding, ChangeSpec},
    executor::{self, ProbeTransactionExecution},
    path::{
        token_transfer, CapturedExecutionFixture, ProbeClock, ProbeExecutionPlan, ProbeMessage,
    },
    replay::hash_bytes,
    standard_programs::{token, token2022, Decoded},
    types::AccountSnapshot,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const KIND: &str = "protocol_parameter_change";
pub const DERIVATION: &str = "token_2022_active_newer_transfer_fee_basis_points_v1";
pub const REPORT_SCHEMA: &str = "eplyx-protocol-parameter-report-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigTarget {
    pub program_id: String,
    pub config_account: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedCurrent {
    pub account_data_sha256: String,
    pub basis_points: u16,
    #[serde(with = "crate::numfmt::u64_string")]
    pub schedule_epoch: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub maximum_fee_raw: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    #[serde(rename = "token_2022_active_newer_transfer_fee_basis_points_v1")]
    Token2022ActiveNewerTransferFeeBasisPointsV1 {
        expected_current: ExpectedCurrent,
        proposed_basis_points: u16,
    },
}
impl Operation {
    pub fn values(&self) -> (&ExpectedCurrent, u16) {
        match self {
            Self::Token2022ActiveNewerTransferFeeBasisPointsV1 {
                expected_current,
                proposed_basis_points,
            } => (expected_current, *proposed_basis_points),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterChange {
    pub target: ConfigTarget,
    pub operation: Operation,
}
impl ParameterChange {
    pub fn validate(&self, activation: Option<&Activation>) -> Result<()> {
        ensure!(
            activation.is_none(),
            "parameter change activation must be absent"
        );
        crate::change::canonical_address(&self.target.program_id, "target program")?;
        crate::change::canonical_address(&self.target.config_account, "config mint")?;
        ensure!(
            self.target.program_id == token2022::PROGRAM_ID,
            "unsupported_config_field: only Token-2022 is supported"
        );
        let (expected, proposed) = self.operation.values();
        ensure!(
            expected.basis_points <= 10000 && proposed <= 10000,
            "invalid_proposed_value: basis points must be 0..=10000"
        );
        ensure!(
            expected.account_data_sha256.len() == 64
                && expected
                    .account_data_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid expected mint SHA-256"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    UnsupportedConfigField,
    InvalidProposedValue,
    ConfigEvidenceMissing,
    CurrentStateMismatch,
    ScheduleNotActive,
    MutationUnsupported,
    DownstreamActionUnsupported,
    ExecutionRejected,
    ExecutionUnavailable,
    ReconciliationFailed,
    SemanticConsequenceObserved,
    NoObservedConsequence,
}
#[derive(Debug)]
pub struct Failure {
    pub status: Status,
    pub detail: String,
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.status, self.detail)
    }
}
impl std::error::Error for Failure {}
fn failed(status: Status, detail: impl ToString) -> Failure {
    Failure {
        status,
        detail: detail.to_string(),
    }
}
fn strict_mint(bytes: &[u8]) -> Result<Vec<token2022::Extension>> {
    use spl_token_2022_interface::{extension::StateWithExtensions, state::Mint};
    let m = StateWithExtensions::<Mint>::unpack(bytes)?;
    ensure!(m.base.is_initialized, "mint not initialized");
    let decoded = match token2022::checked_extensions(bytes, token2022::Layout::Mint) {
        Decoded::Decoded(v) => v,
        other => anyhow::bail!("invalid mint extensions: {other:?}"),
    };
    ensure!(
        !decoded
            .iter()
            .any(|e| matches!(e, token2022::Extension::Unrecognized { .. })),
        "unknown extension"
    );
    Ok(decoded)
}

/// Official typed in-place POD mutation plus semantic and exhaustive byte preservation.
/// No partial reconstruction, account allocation, arbitrary offset or clock changes.
pub fn mutate(
    current: &AccountSnapshot,
    expected: &ExpectedCurrent,
    proposed: u16,
    epoch: u64,
) -> std::result::Result<AccountSnapshot, Failure> {
    use spl_token_2022_interface::{
        extension::{
            transfer_fee::TransferFeeConfig, BaseStateWithExtensionsMut, StateWithExtensionsMut,
        },
        state::Mint,
    };
    if proposed > 10000 {
        return Err(failed(Status::InvalidProposedValue, "bps outside range"));
    }
    if current.owner != token2022::PROGRAM_ID || current.executable {
        return Err(failed(
            Status::MutationUnsupported,
            "wrong mint owner/envelope",
        ));
    }
    let before = strict_mint(&current.data).map_err(|e| failed(Status::MutationUnsupported, e))?;
    let Some(token2022::Extension::TransferFeeConfig { newer, .. }) = before
        .iter()
        .find(|e| matches!(e, token2022::Extension::TransferFeeConfig { .. }))
    else {
        return Err(failed(
            Status::UnsupportedConfigField,
            "mint has no fee config",
        ));
    };
    if hash_bytes(&current.data) != expected.account_data_sha256
        || newer.basis_points != expected.basis_points
        || newer.epoch != expected.schedule_epoch
        || newer.maximum_fee != expected.maximum_fee_raw
    {
        return Err(failed(
            Status::CurrentStateMismatch,
            "mint hash/rate/epoch/cap differs from mandatory current expectation",
        ));
    }
    if epoch < newer.epoch {
        return Err(failed(
            Status::ScheduleNotActive,
            "newer schedule is pending at captured epoch",
        ));
    }
    let mut next = current.clone();
    {
        let mut state = StateWithExtensionsMut::<Mint>::unpack(&mut next.data)
            .map_err(|e| failed(Status::MutationUnsupported, e))?;
        let fee = state
            .get_extension_mut::<TransferFeeConfig>()
            .map_err(|e| failed(Status::MutationUnsupported, e))?;
        // Location comes from the actual official typed field, not a local wire offset.
        fee.newer_transfer_fee.transfer_fee_basis_points = proposed.into();
    }
    // Obtain the location relative to the mutated buffer (the allocation never changes).
    // Re-open read-only official state so location cannot come from a caller.
    use spl_token_2022_interface::extension::{BaseStateWithExtensions, StateWithExtensions};
    let state = StateWithExtensions::<Mint>::unpack(&next.data)
        .map_err(|e| failed(Status::MutationUnsupported, e))?;
    let fee = state
        .get_extension::<TransferFeeConfig>()
        .map_err(|e| failed(Status::MutationUnsupported, e))?;
    let offset = (&fee.newer_transfer_fee.transfer_fee_basis_points as *const _ as usize)
        - next.data.as_ptr() as usize;
    if offset
        .checked_add(2)
        .is_none_or(|end| end > next.data.len())
    {
        return Err(failed(
            Status::MutationUnsupported,
            "official bps field outside mint",
        ));
    }
    let mut restored = next.data.clone();
    restored[offset..offset + 2].copy_from_slice(&current.data[offset..offset + 2]);
    if restored != current.data || next.data.len() != current.data.len() {
        return Err(failed(
            Status::MutationUnsupported,
            "unrelated bytes changed",
        ));
    }
    let mut want = before;
    for e in &mut want {
        if let token2022::Extension::TransferFeeConfig { newer, .. } = e {
            newer.basis_points = proposed;
        }
    }
    let after = strict_mint(&next.data).map_err(|e| failed(Status::MutationUnsupported, e))?;
    if after != want
        || token::mint_base(&current.owner, &current.data)
            .map_err(|e| failed(Status::MutationUnsupported, e))?
            != token::mint_base(&next.owner, &next.data)
                .map_err(|e| failed(Status::MutationUnsupported, e))?
    {
        return Err(failed(
            Status::MutationUnsupported,
            "unrelated mint semantics changed",
        ));
    }
    Ok(next)
}

/// Immutable retained transfer transcript and exact interaction. No proposed bytes or ELF input.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub schema_version: u32,
    pub context: token_transfer::TransferContext,
    #[serde(with = "crate::numfmt::u64_string")]
    pub amount_raw: u64,
    pub fixture: CapturedExecutionFixture,
    pub fixture_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_capture_sha256: Option<String>,
}
impl Input {
    pub fn validate(&self) -> Result<ProbeExecutionPlan> {
        ensure!(
            self.schema_version == 1 && self.fixture_sha256 == self.fixture.sha256()?,
            "config_evidence_missing: capture digest/schema mismatch"
        );
        ensure!(
            self.context.program == token2022::PROGRAM_ID,
            "downstream_action_unsupported: Token-2022 required"
        );
        ensure!(
            self.fixture.evidence.len() == 4
                && self.fixture.evidence.iter().all(|r| !r.result.is_null()),
            "config_evidence_missing: incomplete transfer transcript"
        );
        let p = token_transfer::build_current(&self.context, self.amount_raw, &self.fixture)?;
        // Apply current-path admission as well as the low-level builder boundary.
        let account = |a: &str| {
            p.accounts
                .iter()
                .find(|n| n.address == a)
                .context("captured account absent")
        };
        let mint = token::decode_mint(&crate::migration::world::rpc_value(
            &account(&self.context.mint)?.account,
        ))?;
        let source = token::decode_token_account(
            &crate::migration::world::rpc_value(&account(&self.context.source)?.account),
            &self.context.program,
            &self.context.mint,
            self.context.decimals,
        )?;
        token::account_amounts(
            &self.context.program,
            &account(&self.context.source)?.account.data,
        )?;
        token::account_amounts(
            &self.context.program,
            &account(&self.context.destination)?.account.data,
        )?;
        let destination = token::decode_token_account(
            &crate::migration::world::rpc_value(&account(&self.context.destination)?.account),
            &self.context.program,
            &self.context.mint,
            self.context.decimals,
        )?;
        if let Some(reason) = crate::path::current::unsupported(&mint, &source, Some(&destination))
        {
            anyhow::bail!("downstream_action_unsupported: {reason}");
        }
        Ok(p)
    }
    pub fn sha256(&self) -> Result<String> {
        crate::canonical::digest(self)
    }
}

fn proposal(spec: &ChangeSpec) -> Result<&ParameterChange> {
    spec.validate()?;
    match &spec.change {
        Change::ProtocolParameterChange(c) => Ok(c),
        _ => anyhow::bail!("unsupported_config_field: wrong change kind"),
    }
}
pub fn binding(spec: &ChangeSpec) -> Result<ChangeBinding> {
    let c = proposal(spec)?;
    Ok(ChangeBinding {
        change_spec_id: spec.id()?,
        change: BoundChange::ProtocolParameterChange {
            target: c.target.clone(),
            operation: c.operation.clone(),
        },
    })
}
fn prepare(
    spec: &ChangeSpec,
    input: &Input,
) -> std::result::Result<(ProbeExecutionPlan, AccountSnapshot), Failure> {
    let c = proposal(spec).map_err(|e| failed(Status::UnsupportedConfigField, e))?;
    if input.context.mint != c.target.config_account {
        return Err(failed(
            Status::CurrentStateMismatch,
            "transfer mint differs from proposal target",
        ));
    }
    let plan = input.validate().map_err(|e| {
        failed(
            if e.to_string().starts_with("config_evidence_missing:") {
                Status::ConfigEvidenceMissing
            } else {
                Status::DownstreamActionUnsupported
            },
            e,
        )
    })?;
    let mint = &plan
        .accounts
        .iter()
        .find(|a| a.address == c.target.config_account)
        .ok_or_else(|| failed(Status::ConfigEvidenceMissing, "mint absent"))?
        .account;
    let (expect, bps) = c.operation.values();
    let proposed = mutate(mint, expect, bps, plan.clock.epoch)?;
    Ok((plan, proposed))
}
fn plan_commitment(p: &ProbeExecutionPlan) -> Result<Value> {
    Ok(
        json!({"programs":p.programs.iter().map(|p|json!({"program_id":p.program_id.to_string(),"loader":p.loader.to_string(),"elf_sha256":hash_bytes(&p.bytes)})).collect::<Vec<_>>(),"clock":ProbeClock::from(&p.clock),"message":ProbeMessage::from(&p.message),"watch":p.watch,"accounts":p.accounts,"account_evidence":p.account_evidence,"assumptions":p.assumptions,"runtime":runtime()}),
    )
}
pub fn runtime() -> Value {
    json!({"backend":"LiteSVM 0.16","profile":"mainnet/default","signature_verification":false,"recent_blockhash_verification":false,"eplyx_version":crate::build_info::VERSION,"revision":"eplyx-token-2022-paired-transfer-v1","lock_sha256":hash_bytes(include_bytes!("../../../Cargo.lock")),"executor_sha256":hash_bytes(include_bytes!("../executor.rs")),"transfer_sha256":hash_bytes(include_bytes!("../path/token_transfer.rs")),"derivation_source_sha256":hash_bytes(include_bytes!("mod.rs"))})
}
fn preamble(spec: &ChangeSpec, input: &Input) -> Result<Value> {
    Ok(
        json!({"schema":REPORT_SCHEMA,"kind":KIND,"change":binding(spec)?,"observed_capture_sha256":input.source_capture_sha256.as_ref().unwrap_or(&input.fixture_sha256),"transfer_fixture_sha256":input.fixture_sha256,"analysis_input_sha256":input.sha256()?,"retained_input":input,"derivation_revision":DERIVATION,"runtime":runtime(),"authorization":false,"funds_moved":false,"set_transfer_fee_executed":false,"limitations":["Typed active-state counterfactual only; no authorization, key possession, fee-admin execution or on-chain activation established.","One retained finalized capture and exact original-owner TransferChecked; no all-holder, safety or historical representativeness claim.","Independent fresh local VMs with captured Clock and identical deployed code; synthetic fee payer and assumed owner signature. Not a full validator bank."]}),
    )
}
fn seal(mut r: Value) -> Result<Value> {
    r["report_sha256"] = crate::canonical::digest(&r)?.into();
    Ok(r)
}
fn failure_report(spec: &ChangeSpec, input: &Input, f: &Failure) -> Result<Value> {
    let mut r = preamble(spec, input)?;
    r["status"] = serde_json::to_value(f.status)?;
    r["failure"] = json!({"status":f.status,"detail":f.detail});
    r["execution_performed"] = false.into();
    r["findings"] = json!([]);
    seal(r)
}
fn side(input: &Input, plan: &ProbeExecutionPlan, x: &ProbeTransactionExecution) -> Result<Value> {
    let (d, error) = match token_transfer::reconcile_current(
        &input.context.mint,
        &input.context.source,
        &input.context.destination,
        input.context.decimals,
        input.amount_raw,
        plan,
        x,
    ) {
        Ok(v) => (serde_json::to_value(v)?, None),
        Err(e) => (Value::Null, Some(e.to_string())),
    };
    let pre = |address: &str| {
        plan.accounts
            .iter()
            .find(|a| a.address == address)
            .map(|a| &a.account)
    };
    let amounts = |account: Option<&AccountSnapshot>| {
        account.and_then(|a| token::account_amounts(&input.context.program, &a.data).ok())
            .map(|(public, withheld)| json!({"public_amount_raw":public.to_string(),"withheld_amount_raw":withheld.to_string()}))
    };
    let raw_token_state = json!({
        "source":{"before":amounts(pre(&input.context.source)),"after":amounts(x.post_accounts.get(&input.context.source))},
        "destination":{"before":amounts(pre(&input.context.destination)),"after":amounts(x.post_accounts.get(&input.context.destination))}
    });
    let mint_state = json!({"before_sha256":pre(&input.context.mint).map(|a|hash_bytes(&a.data)),"after_sha256":x.post_accounts.get(&input.context.mint).map(|a|hash_bytes(&a.data))});
    Ok(
        json!({"pre_state":plan.accounts.iter().filter(|a|plan.watch.contains(&a.address)).collect::<Vec<_>>(),"execution":x,"execution_sha256":crate::canonical::digest(x)?,"raw_input":input.amount_raw.to_string(),"raw_token_state":raw_token_state,"mint_state":mint_state,"reconciliation":d,"reconciliation_error":error}),
    )
}
fn consequences(b: &Value, p: &Value) -> Result<(Status, Vec<Value>)> {
    let mut findings = vec![];
    let bs = b["execution"]["success"]
        .as_bool()
        .context("missing success")?;
    let ps = p["execution"]["success"]
        .as_bool()
        .context("missing success")?;
    if b["reconciliation"]["reconciled"] != true || p["reconciliation"]["reconciled"] != true {
        return Ok((Status::ReconciliationFailed, findings));
    }
    if bs != ps {
        let fingerprint: crate::semantics::FindingFingerprint = format!(
            "token-2022/transfer_checked/execution/transaction/{}",
            if ps { "now_succeeds" } else { "now_reverts" }
        )
        .parse()?;
        findings.push(json!({"fingerprint":fingerprint.to_string(),"baseline":bs,"proposed":ps}));
    }
    if bs && ps {
        for (subject, bv, pv) in [
            (
                "recipient_tokens_received",
                &b["reconciliation"]["output_received_raw"],
                &p["reconciliation"]["output_received_raw"],
            ),
            (
                "destination_withheld_transfer_fee",
                &b["reconciliation"]["token_accounts"][1]["withheld_fee_change_raw"],
                &p["reconciliation"]["token_accounts"][1]["withheld_fee_change_raw"],
            ),
        ] {
            let before = bv.as_str().context("missing output")?.parse::<i128>()?;
            let after = pv.as_str().context("missing output")?.parse::<i128>()?;
            if before != after {
                let direction = crate::semantics::ChangeKind::from_delta(after - before);
                let fingerprint: crate::semantics::FindingFingerprint = format!(
                    "token-2022/transfer_checked/economic/{subject}/{}",
                    direction.as_str()
                )
                .parse()?;
                findings.push(json!({"fingerprint":fingerprint.to_string(),"baseline_raw":before.to_string(),"proposed_raw":after.to_string(),"delta_raw":(after-before).to_string()}));
            }
        }
    }
    Ok((
        if !findings.is_empty() {
            Status::SemanticConsequenceObserved
        } else if !bs || !ps {
            Status::ExecutionRejected
        } else {
            Status::NoObservedConsequence
        },
        findings,
    ))
}
fn finish(
    spec: &ChangeSpec,
    input: &Input,
    plan: &ProbeExecutionPlan,
    next: &AccountSnapshot,
    b: Value,
    p: Value,
) -> Result<Value> {
    let mut r = preamble(spec, input)?;
    let c = proposal(spec)?;
    let (expected, bps) = c.operation.values();
    r["derived_proposed_pre_state"] = json!({"origin":"derived_from_observed","parent_observed_mint_sha256":expected.account_data_sha256,"proposed_mint_sha256":hash_bytes(&next.data),"derivation_revision":DERIVATION,"mutation":c.operation,"mint":next});
    r["observed_current_state"] = json!({"origin":"observed_finalized_capture","mint_sha256":expected.account_data_sha256,"capture_sha256":input.fixture_sha256});
    r["proposed_declaration"] = json!({"origin":"validated_changespec","change_spec_id":spec.id()?,"current_bps":expected.basis_points,"proposed_bps":bps,"newer_schedule_epoch":expected.schedule_epoch.to_string(),"captured_epoch":plan.clock.epoch.to_string(),"maximum_fee_raw":expected.maximum_fee_raw.to_string()});
    r["shared_execution"] = plan_commitment(plan)?;
    r["shared_execution_sha256"] = crate::canonical::digest(&r["shared_execution"])?.into();
    r["interaction_sha256"]=crate::canonical::digest(&json!({"context":input.context,"amount_raw":input.amount_raw.to_string(),"message":ProbeMessage::from(&plan.message)}))?.into();
    let (status, findings) = consequences(&b, &p)?;
    r["status"] = serde_json::to_value(status)?;
    r["findings"] = json!(findings);
    r["baseline"] = b;
    r["proposed"] = p;
    r["execution_performed"] = true.into();
    r["post_state_origin"] = "simulated_post_action".into();
    seal(r)
}
pub fn analyze(spec: &ChangeSpec, input: &Input) -> Result<Value> {
    proposal(spec)?;
    let (mut plan, next) = match prepare(spec, input) {
        Ok(v) => v,
        Err(f) => return failure_report(spec, input, &f),
    };
    let original = plan_commitment(&plan)?;
    let b = match executor::execute_probe_message(
        &plan.accounts,
        &plan.watch,
        plan.clock.clone(),
        &plan.programs,
        plan.message.clone(),
    ) {
        Ok(x) => x,
        Err(e) => return failure_report(spec, input, &failed(Status::ExecutionUnavailable, e)),
    };
    let bv = match side(input, &plan, &b) {
        Ok(v) => v,
        Err(e) => return failure_report(spec, input, &failed(Status::ReconciliationFailed, e)),
    };
    let i = plan
        .accounts
        .iter()
        .position(|a| a.address == input.context.mint)
        .context("mint absent")?;
    let observed = plan.accounts[i].account.clone();
    plan.accounts[i].account = next.clone();
    let proposed_commitment = plan_commitment(&plan)?;
    let mut restored = proposed_commitment.clone();
    restored["accounts"][i]["account"] = serde_json::to_value(&observed)?;
    ensure!(
        restored == original,
        "paired execution invariants differ beyond mint"
    );
    let p = match executor::execute_probe_message(
        &plan.accounts,
        &plan.watch,
        plan.clock.clone(),
        &plan.programs,
        plan.message.clone(),
    ) {
        Ok(x) => x,
        Err(e) => return failure_report(spec, input, &failed(Status::ExecutionUnavailable, e)),
    };
    let pv = match side(input, &plan, &p) {
        Ok(v) => v,
        Err(e) => return failure_report(spec, input, &failed(Status::ReconciliationFailed, e)),
    };
    plan.accounts[i].account = observed;
    finish(spec, input, &plan, &next, bv, pv)
}
/// Structural/integrity verification rebuilds inputs and reconciliation, never executes on read.
pub fn verify(spec: &ChangeSpec, report: &Value) -> Result<()> {
    let mut unsealed = report.clone();
    let recorded = unsealed
        .as_object_mut()
        .context("report object required")?
        .remove("report_sha256")
        .context("report commitment missing")?;
    ensure!(
        recorded == crate::canonical::digest(&unsealed)?,
        "report digest mismatch"
    );
    let input: Input = serde_json::from_value(report["retained_input"].clone())?;
    let (mut plan, next) = match prepare(spec, &input) {
        Ok(v) => v,
        Err(f) => {
            ensure!(
                *report == failure_report(spec, &input, &f)?,
                "failure report mismatch"
            );
            return Ok(());
        }
    };
    if report["execution_performed"] != true {
        let status: Status = serde_json::from_value(report["status"].clone())?;
        ensure!(
            status == Status::ExecutionUnavailable,
            "admissible input lacks execution evidence"
        );
        let f = failed(
            status,
            report["failure"]["detail"]
                .as_str()
                .context("missing execution-unavailable detail")?,
        );
        ensure!(
            *report == failure_report(spec, &input, &f)?,
            "unavailable report mismatch"
        );
        return Ok(());
    }
    let b: ProbeTransactionExecution =
        serde_json::from_value(report["baseline"]["execution"].clone())?;
    let p: ProbeTransactionExecution =
        serde_json::from_value(report["proposed"]["execution"].clone())?;
    let bv = side(&input, &plan, &b)?;
    let i = plan
        .accounts
        .iter()
        .position(|a| a.address == input.context.mint)
        .context("mint absent")?;
    let old = plan.accounts[i].account.clone();
    plan.accounts[i].account = next.clone();
    let pv = side(&input, &plan, &p)?;
    plan.accounts[i].account = old;
    ensure!(
        *report == finish(spec, &input, &plan, &next, bv, pv)?,
        "parameter report bindings/results mismatch"
    );
    Ok(())
}
pub fn reproduce(spec: &ChangeSpec, report: &Value) -> Result<()> {
    verify(spec, report)?;
    let input: Input = serde_json::from_value(report["retained_input"].clone())?;
    ensure!(
        *report == analyze(spec, &input)?,
        "offline paired execution differs from retained result"
    );
    Ok(())
}
