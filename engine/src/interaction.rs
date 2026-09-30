//! Bounded local V1/V2 × C0/C1 experiment. Verification is a pure reduction of
//! retained executions; reproduction additionally executes six fresh VM banks.
use crate::{
    canonical,
    change::{ChangeSpec, ExecutableArtifact},
    executor::{self, ExecutionResult, ProbeTransactionExecution},
    parameter_change::stake_pool as s,
    replay::{self, ReplayRecord},
    types::AccountSnapshot,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

pub const REVISION: &str = "eplyx-stake-pool-upgrade-parameter-interaction-v1";
pub const FIXTURE_SHA: &str = "a664f74b73dedc713f16934829b25f9a0c0c3a06c6ce21f03fdc7869ae5b555d";
const TOKEN_SHA: &str = "8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697";
const LIMITS: &str = "One retained DepositSol under fixed schema-1 Clock (epoch 0). Candidate is a constructed test fixture, not an upstream release. Code is overlaid in fresh banks; no installed loader world, actual Upgrade, activation, rollout order, atomicity, signing, authority possession, governance, population impact or valuation is established. Configuration manager and separate fee payer are explicit simulation assumptions. Configuration fees are excluded from action metrics. Aliased token roles are counted once. Read-only verification establishes internal consistency, not independent proof that a VM ran; reproduction reruns the VM.";
const METRICS: &[&str] = &[
    "recipient_account_credit_raw",
    "manager_fee_account_credit_raw",
    "referral_account_credit_raw",
    "mint_supply_delta_raw",
    "pool_token_supply_delta_raw",
    "reserve_lamport_delta",
    "pool_total_lamports_delta",
    "funding_payer_lamport_debit",
    "funding_payer_debit_excluding_transaction_fee",
    "action_transaction_fee_lamports",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub upgrade: ChangeSpec,
    pub parameter: ChangeSpec,
    pub historical: s::Input,
    pub candidate: s::ProgramEvidence,
}
impl Input {
    /// A no-op proposal used only to check retained-record eligibility. This
    /// never executes configuration or action transactions.
    pub fn retained_control(
        upgrade: &ChangeSpec,
        historical: s::Input,
        candidate: Vec<u8>,
    ) -> Result<Self> {
        use crate::change::{Change, ChangeMetadata};
        use crate::parameter_change::{ConfigTarget, Operation, ParameterChange};
        let pool = s::pool(&historical.record)?;
        let (state, _) = s::decode(&pool.account)?;
        let parameter = ChangeSpec {
            schema_version: 1,
            change_spec_id: None,
            activation: None,
            metadata: ChangeMetadata::default(),
            change: Change::ProtocolParameterChange(Box::new(ParameterChange {
                target: ConfigTarget {
                    program_id: historical.record.program_id.clone(),
                    config_account: pool.address.clone(),
                },
                operation: Operation::SplStakePoolSolDepositFeeV1 {
                    expected_current: s::ExpectedCurrent {
                        account_data_sha256: replay::hash_bytes(&pool.account.data),
                        numerator: state.sol_deposit_fee.numerator,
                        denominator: state.sol_deposit_fee.denominator,
                        sol_referral_fee_percent: state.sol_referral_fee,
                        last_update_epoch: state.last_update_epoch,
                    },
                    proposed_fee: s::RationalFee {
                        numerator: state.sol_deposit_fee.numerator,
                        denominator: state.sol_deposit_fee.denominator,
                    },
                },
            })),
        };
        Self::new(upgrade, &parameter, historical, candidate)
    }
    pub fn new(
        upgrade: &ChangeSpec,
        parameter: &ChangeSpec,
        historical: s::Input,
        candidate: Vec<u8>,
    ) -> Result<Self> {
        let normalize = |spec: &ChangeSpec| -> Result<ChangeSpec> {
            spec.validate()?;
            let mut copy = spec.clone();
            copy.change_spec_id = Some(spec.id()?);
            Ok(copy)
        };
        Ok(Self {
            upgrade: normalize(upgrade)?,
            parameter: normalize(parameter)?,
            candidate: s::ProgramEvidence {
                program_id: historical.record.program_id.clone(),
                loader: crate::versions::UPGRADEABLE_LOADER_ID.into(),
                elf_sha256: replay::hash_bytes(&candidate),
                elf: candidate,
            },
            historical,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    InteractionObserved,
    NoMeasuredInteraction,
    NotEstablished,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Verified,
    Rejected,
    Unreconciled,
    Unavailable,
    NotExecuted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parent {
    pub stage: String,
    pub stage_input_sha256: String,
    pub execution_sha256: String,
    pub pool_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage<T> {
    pub state: State,
    pub input: Value,
    pub input_sha256: String,
    pub parent: Option<Parent>,
    pub execution: Option<T>,
    pub execution_sha256: Option<String>,
    pub reason: Option<String>,
    pub infrastructure_error: Option<String>,
    pub derived: Value,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    pub value: Option<String>,
    pub unavailable_reason: Option<String>,
}
impl Quantity {
    fn absent(reason: &str) -> Self {
        Self {
            value: None,
            unavailable_reason: Some(reason.into()),
        }
    }
    fn of(value: i128) -> Self {
        Self {
            value: Some(value.to_string()),
            unavailable_reason: None,
        }
    }
    fn parse(&self) -> Result<Option<i128>> {
        match &self.value {
            Some(text) => {
                ensure!(
                    self.unavailable_reason.is_none(),
                    "available quantity has unavailable reason"
                );
                let n = text.parse::<i128>()?;
                ensure!(*text == n.to_string(), "noncanonical signed quantity");
                Ok(Some(n))
            }
            None => {
                ensure!(
                    self.unavailable_reason.is_some(),
                    "missing numeric availability reason"
                );
                Ok(None)
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effects {
    pub parameter_v1: Quantity,
    pub parameter_v2: Quantity,
    pub code_c0: Quantity,
    pub code_c1: Quantity,
    pub combined: Quantity,
    pub interaction: Quantity,
    pub identity_cross_check: Option<bool>,
}
/// Calculator inputs are independently reconciled, comparable measurements.
/// Missing values remain missing, including in otherwise additive experiments.
pub fn contrasts(cells: [Quantity; 4]) -> Result<Effects> {
    let [a, b, c, d] = cells;
    let subtract = |x: &Quantity, y: &Quantity| -> Result<Quantity> {
        Ok(match (x.parse()?, y.parse()?) {
            (Some(x), Some(y)) => match x.checked_sub(y) {
                Some(n) => Quantity::of(n),
                None => Quantity::absent("signed arithmetic overflow"),
            },
            _ => Quantity::absent("required comparable measurement unavailable"),
        })
    };
    let parameter_v1 = subtract(&b, &a)?;
    let parameter_v2 = subtract(&d, &c)?;
    let code_c0 = subtract(&c, &a)?;
    let code_c1 = subtract(&d, &b)?;
    let combined = subtract(&d, &a)?;
    let interaction = subtract(&parameter_v2, &parameter_v1)?;
    let identity_cross_check = match (
        code_c0.parse()?,
        parameter_v1.parse()?,
        interaction.parse()?,
        combined.parse()?,
    ) {
        (Some(c), Some(p), Some(i), Some(t)) => c
            .checked_add(p)
            .and_then(|n| n.checked_add(i))
            .map(|n| n == t),
        _ => None,
    };
    ensure!(
        identity_cross_check != Some(false),
        "contrast identity differs"
    );
    Ok(Effects {
        parameter_v1,
        parameter_v2,
        code_c0,
        code_c1,
        combined,
        interaction,
        identity_cross_check,
    })
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: String,
    pub question: String,
    pub input: Input,
    pub analysis_input_sha256: String,
    pub input_contract: Value,
    pub admission: Value,
    pub status: Status,
    pub k1: Stage<ProbeTransactionExecution>,
    pub k2: Stage<ProbeTransactionExecution>,
    pub r00: Stage<ExecutionResult>,
    pub r01: Stage<ExecutionResult>,
    pub r10: Stage<ExecutionResult>,
    pub r11: Stage<ExecutionResult>,
    pub measurements: BTreeMap<String, [Quantity; 4]>,
    pub effects: BTreeMap<String, Effects>,
    pub limitations: String,
    pub report_sha256: String,
}
#[derive(Clone)]
enum Attempt<T> {
    Executed(T),
    Unavailable(String),
}
#[derive(Default)]
struct Runs {
    k1: Option<Attempt<ProbeTransactionExecution>>,
    k2: Option<Attempt<ProbeTransactionExecution>>,
    r00: Option<Attempt<ExecutionResult>>,
    r01: Option<Attempt<ExecutionResult>>,
    r10: Option<Attempt<ExecutionResult>>,
    r11: Option<Attempt<ExecutionResult>>,
}
fn runtime() -> Value {
    json!({"revision":REVISION,"shared":s::runtime(),"analyzer_source_sha256":replay::hash_bytes(include_bytes!("interaction.rs"))})
}
fn admission(input: &Input) -> Result<(&'static str, s::ConfigPlan)> {
    ensure!(
        canonical::document(input)?.len() as u64 <= crate::lifecycle::artifact::MAX_BYTES,
        "interaction input exceeds local byte bound"
    );
    ensure!(
        input.candidate.elf.len() as u64 <= crate::migration::input::MAX_PROGRAM_BYTES
            && input
                .historical
                .programs
                .iter()
                .all(|p| p.elf.len() as u64 <= crate::migration::input::MAX_PROGRAM_BYTES),
        "program exceeds existing local executable bound"
    );
    ensure!(
        input.upgrade.to_document()?.len() as u64 <= crate::migration::input::MAX_CHANGE_BYTES
            && input.parameter.to_document()?.len() as u64
                <= crate::migration::input::MAX_CHANGE_BYTES,
        "proposal exceeds existing local change bound"
    );
    input.upgrade.validate()?;
    input.parameter.validate()?;
    let u = input
        .upgrade
        .as_program_upgrade()
        .context("program_upgrade proposal required")?;
    ensure!(input.upgrade.activation.is_none() && u.delivery.is_none() && u.expected_upgrade_authority.is_none() && u.target.programdata_address.is_none(), "unsupported upgrade activation/delivery/authority/ProgramData expectation; loader world not evaluated");
    input.historical.validate()?;
    ensure!(
        u.target.program_id == input.historical.record.program_id,
        "upgrade target differs from retained program"
    );
    ensure!(
        input.candidate.program_id == u.target.program_id
            && input.candidate.loader == crate::versions::UPGRADEABLE_LOADER_ID
            && input.candidate.elf_sha256 == replay::hash_bytes(&input.candidate.elf)
            && *u.candidate == ExecutableArtifact::of(&input.candidate.elf),
        "candidate bytes/hash/length/loader mismatch"
    );
    if let Some(replaces) = u.replaces {
        let original = input
            .historical
            .programs
            .iter()
            .find(|p| p.program_id == u.target.program_id)
            .context("V1 missing")?;
        ensure!(
            *replaces == ExecutableArtifact::of(&original.elf),
            "upgrade replaces differs from retained V1"
        );
    }
    let plan = s::prepare(&input.parameter, &input.historical)
        .map_err(|e| anyhow::anyhow!("{:?}: {}", e.status, e.detail))?;
    let (pool, _) = s::decode(&s::pool(&input.historical.record)?.account)?;
    ensure!(pool.sol_referral_fee==0 && pool.sol_deposit_authority.is_none() && pool.token_program_id.to_string()==crate::protocol::stake_pool::TOKEN_PROGRAM_ID && input.historical.record.clock.epoch==0, "unqualified scope: requires zero referral, ungated legacy Token DepositSol and fixed epoch zero");
    let dependencies = &input.historical.record.dependencies.programs;
    let allowed_dependencies = [
        crate::protocol::stake_pool::PROGRAM_ID,
        crate::protocol::stake_pool::TOKEN_PROGRAM_ID,
        crate::protocol::stake_pool::SYSTEM_PROGRAM_ID,
        "ComputeBudget111111111111111111111111111111",
    ];
    ensure!(
        dependencies.len() == 4
            && dependencies
                .iter()
                .all(|p| allowed_dependencies.contains(&p.program_id.as_str()))
            && input.historical.programs.len() == 2,
        "unqualified dependency set: only reviewed target, Token, System and ComputeBudget"
    );
    let token = input
        .historical
        .programs
        .iter()
        .find(|p| p.program_id == crate::protocol::stake_pool::TOKEN_PROGRAM_ID)
        .context("pinned Token dependency absent")?;
    ensure!(
        token.elf_sha256 == TOKEN_SHA
            && token.elf.len() == 108600
            && token.loader == crate::versions::UPGRADEABLE_LOADER_ID,
        "unqualified pinned Token dependency"
    );
    let profile = if input.candidate.elf_sha256 == input.historical.record.current_program_sha256 {
        "same-code-historical-v1"
    } else if input.candidate.elf_sha256 == FIXTURE_SHA && input.candidate.elf.len() == 133992 {
        "constructed-step10b-config-deposit-v1"
    } else {
        anyhow::bail!(
            "candidate_interpretation_unqualified: no reviewed profile for this executable"
        )
    };
    Ok((profile, plan))
}
fn profile(name: &str) -> Value {
    json!({"id":name,"revision":1,"trusted_source":"compiled reviewed profile; no caller qualification assertions","qualification_receipt_sha256":if name=="constructed-step10b-config-deposit-v1"{Some("2a5247b4dcad7d8c49ef0f7e0a3763d2960e0ddaef9a78fb8763c59220552cc9")}else{None},"scope":"official full StakePool 2.0.3 layout; SetFee SolDeposit only; nonzero ungated ten-account DepositSol; zero referral; legacy Token; fixed epoch 0; key/signer manager boundary; exact reviewed ELF, loader and dependency pins"})
}
fn overlay(mut value: Value, input: &Input, v2: bool) -> Value {
    // The historical dependency manifest stays intact. This separate field
    // explicitly commits the effective target executable and loader overlay.
    value["effective_target"] = if v2 {
        json!({"program_id":input.candidate.program_id,"loader":input.candidate.loader,"elf_sha256":input.candidate.elf_sha256,"len":input.candidate.elf.len()})
    } else {
        json!({"program_id":input.historical.record.program_id,"loader":crate::versions::UPGRADEABLE_LOADER_ID,"elf_sha256":input.historical.record.current_program_sha256,"len":input.historical.programs.iter().find(|p|p.program_id==input.historical.record.program_id).map(|p|p.elf.len())})
    };
    value
}
fn contract(input: &Input, admitted: &Result<(&str, s::ConfigPlan)>) -> Result<Value> {
    let mut semantic_input = input.clone();
    semantic_input.upgrade.metadata = Default::default();
    semantic_input.parameter.metadata = Default::default();
    semantic_input.upgrade.change_spec_id = Some(input.upgrade.id()?);
    semantic_input.parameter.change_spec_id = Some(input.parameter.id()?);
    Ok(
        json!({"input":semantic_input,"runtime":runtime(),"profile":admitted.as_ref().ok().map(|(p,_)|profile(p)),"configuration":admitted.as_ref().ok().map(|(_,p)|s::config_commitment(&input.historical,p)),"historical_action":s::action_commitment(&input.historical,&input.historical.record)?,"semantics":{"R00":"V1,S0,C0","K1":"SetFee C1,V1,S0","K2":"SetFee C1,V2,S0 independently","R01":"V1,S0 with only K1 verified pool","R10":"V2 overlay,S0,C0","R11":"V2 overlay,S0 with only K2 verified pool","isolation":"each execution starts a fresh VM; all normalized action inputs equal after restoring the original pool; target code differs only by explicit overlay","metrics":METRICS,"contrast_order":["R00","R01","R10","R11"],"parameter_v1":"R01-R00","parameter_v2":"R11-R10","code_c0":"R10-R00","code_c1":"R11-R01","combined":"R11-R00","interaction":"(R11-R10)-(R01-R00)","cross_check":"combined=code_c0+parameter_v1+interaction","arithmetic":"checked i128 canonical signed decimal strings; unavailable never zero"}}),
    )
}
/// Authoritative admission and input commitment, without VM execution or IO.
/// Contains private retained bytes; callers must select safe presentation facts.
pub fn prepare(input: &Input) -> Result<Value> {
    let admitted = admission(input);
    admitted.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
    contract(input, &admitted)
}
fn stage<T: Serialize>(
    input: Value,
    parent: Option<Parent>,
    attempt: Option<Attempt<T>>,
    allowed: bool,
    blocked: &str,
) -> Result<Stage<T>> {
    ensure!(
        allowed || attempt.is_none(),
        "execution recorded beyond admission/fidelity/configuration gate"
    );
    if allowed {
        ensure!(
            attempt.is_some(),
            "admitted stage must retain an execution or typed infrastructure failure"
        );
    }
    let infrastructure_error = match &attempt {
        Some(Attempt::Unavailable(detail)) => {
            ensure!(!detail.is_empty(), "infrastructure failure detail absent");
            Some(detail.clone())
        }
        _ => None,
    };
    let (state, execution, reason) = match attempt {
        Some(Attempt::Executed(x)) => (State::Verified, Some(x), None),
        Some(Attempt::Unavailable(_)) => (
            State::Unavailable,
            None,
            Some("local VM execution unavailable".into()),
        ),
        None => (State::NotExecuted, None, Some(blocked.into())),
    };
    let input_sha256 = canonical::digest(&input)?;
    let execution_sha256 = execution.as_ref().map(canonical::digest).transpose()?;
    Ok(Stage {
        state,
        input,
        input_sha256,
        parent,
        execution,
        execution_sha256,
        reason,
        infrastructure_error,
        derived: Value::Null,
    })
}
fn configuration(
    input: &Input,
    plan: Option<&s::ConfigPlan>,
    id: &str,
    v2: bool,
    attempt: Option<Attempt<ProbeTransactionExecution>>,
    allowed: bool,
    blocked: &str,
) -> Result<(Stage<ProbeTransactionExecution>, Option<AccountSnapshot>)> {
    let commitment = overlay(
        json!({"stage":id,"configuration":plan.map(|p|s::config_commitment(&input.historical,p))}),
        input,
        v2,
    );
    let mut out = stage(commitment, None, attempt, allowed, blocked)?;
    let mut pool = None;
    if let Some(x) = &out.execution {
        let p = plan.context("configuration without plan")?;
        let (_, fee) = s::values(&input.parameter)?;
        let checked = if x.success {
            s::verify_config(&input.historical, fee, p, x).map(Some)
        } else {
            s::verify_config_rejection(p, x).map(|()| None)
        };
        match checked {
            Ok(next) => {
                pool = next;
                out.state = if x.success {
                    State::Verified
                } else {
                    State::Rejected
                };
                out.derived = json!({"preservation_verified":x.success,"rollback_verified":!x.success,"verified_pool":pool});
                if !x.success {
                    out.reason = Some("configuration instruction rejected; no C1 pool".into());
                }
            }
            Err(e) => {
                out.state = State::Unreconciled;
                out.reason = Some(e.to_string());
            }
        }
    }
    Ok((out, pool))
}
fn parent<T>(id: &str, stage: &Stage<T>, pool: &Option<AccountSnapshot>) -> Result<Option<Parent>> {
    pool.as_ref()
        .map(|p| {
            Ok(Parent {
                stage: id.into(),
                stage_input_sha256: stage.input_sha256.clone(),
                execution_sha256: stage
                    .execution_sha256
                    .clone()
                    .context("parent execution absent")?,
                pool_sha256: canonical::digest(p)?,
            })
        })
        .transpose()
}
fn action(
    input: &Input,
    record: &ReplayRecord,
    id: &str,
    v2: bool,
    parent: Option<Parent>,
    attempt: Option<Attempt<ExecutionResult>>,
    gate: (bool, &str),
) -> Result<Stage<ExecutionResult>> {
    let (allowed, blocked) = gate;
    let requires_pool = match id {
        "R01" => Some("K1"),
        "R11" => Some("K2"),
        _ => None,
    };
    let action_input = if requires_pool.is_some() && parent.is_none() {
        // There is no C1 action input until its own configuration is verified.
        // Keep it absent rather than displaying S0/C0 as a fictitious C1 input.
        Value::Null
    } else {
        s::action_commitment(&input.historical, record)?
    };
    let commitment = overlay(
        json!({"stage":id,"action":action_input,"requires_verified_pool_from":requires_pool}),
        input,
        v2,
    );
    let mut out = stage(commitment, parent, attempt, allowed, blocked)?;
    if let Some(x) = &out.execution {
        // The executor uses a fixed display version for the overlay; labels are
        // not evidence of code identity, which is in the input commitment.
        ensure!(
            x.version == "historical-baseline",
            "action execution version binding differs"
        );
        let checked = if x.success == x.error.is_none() {
            s::reconcile(record, x)
        } else {
            Err(anyhow::anyhow!("action success/error outcome inconsistent"))
        };
        match checked {
            Ok(r) => {
                out.state = if x.success {
                    State::Verified
                } else {
                    State::Rejected
                };
                out.derived = json!({"reconciliation":r});
                if !x.success {
                    out.reason =
                        Some("action instruction rejected; numeric measurement unavailable".into());
                }
            }
            Err(e) => {
                out.state = State::Unreconciled;
                out.reason = Some(e.to_string());
            }
        }
        if id == "R00" {
            let failures = record.fidelity_failures(x)?;
            out.derived["historical_fidelity"] =
                json!({"status":record.fidelity(x)?,"failures":failures});
            if !failures.is_empty() || record.fidelity(x)? != replay::ReplayFidelity::Matched {
                out.state = State::Unreconciled;
                out.reason = Some("historical R00 fidelity gate failed".into());
            }
        }
    }
    Ok(out)
}
fn assemble(input: &Input, runs: Runs) -> Result<Report> {
    let admitted = admission(input);
    let input_contract = contract(input, &admitted)?;
    let analysis_input_sha256 = canonical::digest(&input_contract)?;
    let admission_value = match &admitted {
        Ok((p, _)) => json!({"qualified":true,"profile":profile(p)}),
        Err(e) => json!({"qualified":false,"reason":e.to_string()}),
    };
    let r00 = action(
        input,
        &input.historical.record,
        "R00",
        false,
        None,
        runs.r00,
        (admitted.is_ok(), "admission not established"),
    )?;
    let gate = r00.state == State::Verified;
    let plan = admitted.as_ref().ok().map(|(_, p)| p);
    let (k1, p1) = configuration(
        input,
        plan,
        "K1",
        false,
        runs.k1,
        gate,
        "R00 historical fidelity/reconciliation not established",
    )?;
    let (k2, p2) = configuration(
        input,
        plan,
        "K2",
        true,
        runs.k2,
        gate,
        "R00 historical fidelity/reconciliation not established",
    )?;
    let rec1 = p1
        .as_ref()
        .map(|p| s::proposed_record(&input.historical, p))
        .transpose()?
        .unwrap_or_else(|| input.historical.record.clone());
    let rec2 = p2
        .as_ref()
        .map(|p| s::proposed_record(&input.historical, p))
        .transpose()?
        .unwrap_or_else(|| input.historical.record.clone());
    let r01 = action(
        input,
        &rec1,
        "R01",
        false,
        parent("K1", &k1, &p1)?,
        runs.r01,
        (gate && p1.is_some(), "K1 verified C1 pool unavailable"),
    )?;
    let r10 = action(
        input,
        &input.historical.record,
        "R10",
        true,
        None,
        runs.r10,
        (
            gate,
            "R00 historical fidelity/reconciliation not established",
        ),
    )?;
    let r11 = action(
        input,
        &rec2,
        "R11",
        true,
        parent("K2", &k2, &p2)?,
        runs.r11,
        (gate && p2.is_some(), "K2 verified C1 pool unavailable"),
    )?;
    let mut measurements = BTreeMap::new();
    let mut effects = BTreeMap::new();
    for metric in METRICS {
        let numbers = [&r00, &r01, &r10, &r11].map(|r| {
            if r.state != State::Verified {
                return Quantity::absent(r.reason.as_deref().unwrap_or("cell not comparable"));
            }
            match r.derived["reconciliation"][metric].as_str() {
                Some(text) => Quantity {
                    value: Some(text.into()),
                    unavailable_reason: None,
                },
                None => Quantity::absent("aliased role lacks independently measured split"),
            }
        });
        effects.insert((*metric).into(), contrasts(numbers.clone())?);
        measurements.insert((*metric).into(), numbers);
    }
    let established = effects["recipient_account_credit_raw"]
        .interaction
        .parse()?;
    let status = if effects
        .values()
        .any(|e| e.interaction.value.as_deref().is_some_and(|s| s != "0"))
    {
        Status::InteractionObserved
    } else if established == Some(0) {
        Status::NoMeasuredInteraction
    } else {
        Status::NotEstablished
    };
    let mut report=Report {schema:REVISION.into(),question:"Does this proposed fee change have a different measured effect under V2 than under historical V1?".into(),input:input.clone(),analysis_input_sha256,input_contract,admission:admission_value,status,k1,k2,r00,r01,r10,r11,measurements,effects,limitations:LIMITS.into(),report_sha256:String::new()};
    report.report_sha256 = report_digest(&report)?;
    Ok(report)
}
fn report_digest(report: &Report) -> Result<String> {
    let mut v = serde_json::to_value(report)?;
    v.as_object_mut().unwrap().remove("report_sha256");
    canonical::digest(&v)
}
fn attempt<T>(result: Result<T>) -> Attempt<T> {
    match result {
        Ok(x) => Attempt::Executed(x),
        Err(e) => Attempt::Unavailable(e.to_string()),
    }
}
/// Analyse executes R00 first, then independent K1/K2 and available action cells.
pub fn analyse(input: &Input) -> Result<Report> {
    let mut runs = Runs::default();
    let Ok((_, plan)) = admission(input) else {
        return assemble(input, runs);
    };
    runs.r00 = Some(attempt(input.historical.execute(&input.historical.record)));
    let provisional = action(
        input,
        &input.historical.record,
        "R00",
        false,
        None,
        runs.r00.clone(),
        (true, ""),
    )?;
    if provisional.state != State::Verified {
        return assemble(input, runs);
    }
    let config = |v2: bool| -> Result<ProbeTransactionExecution> {
        let mut programs = input.historical.loaded()?;
        if v2 {
            let target = programs
                .iter_mut()
                .find(|p| p.program_id.to_string() == input.candidate.program_id)
                .context("target absent")?;
            target.bytes = input.candidate.elf.clone();
            target.loader = input.candidate.loader.parse()?;
        }
        executor::execute_probe_message(
            &plan.accounts,
            &plan.watch,
            plan.clock.clone(),
            &programs,
            plan.message.clone(),
        )
    };
    runs.k1 = Some(attempt(config(false)));
    runs.k2 = Some(attempt(config(true)));
    let (_, p1) = configuration(input, Some(&plan), "K1", false, runs.k1.clone(), true, "")?;
    let (_, p2) = configuration(input, Some(&plan), "K2", true, runs.k2.clone(), true, "")?;
    if let Some(p) = p1 {
        runs.r01 = Some(attempt(
            input
                .historical
                .execute(&s::proposed_record(&input.historical, &p)?),
        ));
    }
    runs.r10 = Some(attempt(
        input
            .historical
            .execute_code(&input.historical.record, Some(&input.candidate.elf)),
    ));
    if let Some(p) = p2 {
        runs.r11 = Some(attempt(input.historical.execute_code(
            &s::proposed_record(&input.historical, &p)?,
            Some(&input.candidate.elf),
        )));
    }
    assemble(input, runs)
}
fn retained<T: Clone>(stage: &Stage<T>) -> Option<Attempt<T>> {
    stage.execution.clone().map(Attempt::Executed).or_else(|| {
        (stage.state == State::Unavailable)
            .then(|| Attempt::Unavailable(stage.infrastructure_error.clone().unwrap_or_default()))
    })
}
/// Read-only; no VM, file mutation, provider, or evidence repair.
pub fn verify(report: &Report) -> Result<()> {
    ensure!(
        report.report_sha256 == report_digest(report)?,
        "interaction report seal mismatch"
    );
    let rebuilt = assemble(
        &report.input,
        Runs {
            k1: retained(&report.k1),
            k2: retained(&report.k2),
            r00: retained(&report.r00),
            r01: retained(&report.r01),
            r10: retained(&report.r10),
            r11: retained(&report.r11),
        },
    )?;
    ensure!(
        rebuilt == *report,
        "interaction cross-object inputs, stage handoff, results or metrics differ"
    );
    Ok(())
}
/// Verification precedes fresh independent VM runs and full canonical comparison.
pub fn reproduce(report: &Report) -> Result<()> {
    verify(report)?;
    ensure!(
        analyse(&report.input)? == *report,
        "offline interaction reproduction differs"
    );
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    analysis_input_sha256: String,
    report_sha256: String,
    objects: BTreeMap<String, String>,
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    ensure!(
        bytes.len() as u64 <= crate::lifecycle::artifact::MAX_BYTES,
        "artifact exceeds byte bound"
    );
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn put(root: &Path, bytes: &[u8]) -> Result<String> {
    let hash = replay::hash_bytes(bytes);
    let path = root.join("objects").join(&hash);
    if path.exists() {
        ensure!(
            crate::lifecycle::artifact::read(&path)? == bytes,
            "CAS collision"
        );
    } else {
        write_new(&path, bytes)?;
    }
    Ok(hash)
}
fn pretty_report(report: &Report) -> Result<String> {
    use std::fmt::Write;
    let mut text = format!("# Upgrade × parameter interaction\n\n{}\n\nStatus: `{}`\n\nAnalysis input: `{}`\nReport: `{}`\n\n", report.question, serde_json::to_value(&report.status)?.as_str().unwrap(), report.analysis_input_sha256, report.report_sha256);
    writeln!(text, "## Original proposals\n\nUpgrade `{}`:\n\n```json\n{}\n```\n\nParameter `{}`:\n\n```json\n{}\n```\n", report.input.upgrade.id()?, report.input.upgrade.to_document()?, report.input.parameter.id()?, report.input.parameter.to_document()?)?;
    writeln!(
        text,
        "Candidate `{}` ({} bytes). {}\n\nAdmission:\n\n```json\n{}\n```\n",
        report.input.candidate.elf_sha256,
        report.input.candidate.elf.len(),
        if report.input.candidate.elf_sha256 == FIXTURE_SHA {
            "Constructed Step 10B fixture; not an upstream release."
        } else {
            "Same-code control or unqualified bytes; consult admission."
        },
        serde_json::to_string_pretty(&report.admission)?
    )?;
    writeln!(text, "Retained record `{}`, slot {}, fixed epoch {}. C0 and C1 are stated in the parameter proposal above. K1 and K2 start from the same original accounts and configuration message, with separate V1/V2 code. Each user action starts from S0; R01 receives only K1's verified pool, and R11 receives only K2's verified pool.\n", report.input.historical.record.id, report.input.historical.record.clock.slot, report.input.historical.record.clock.epoch)?;
    writeln!(text, "## Configuration evidence\n\n| Run | State | Compute units | Fee lamports | Reason |\n| --- | --- | ---: | ---: | --- |")?;
    for (id, s) in [("K1", &report.k1), ("K2", &report.k2)] {
        writeln!(
            text,
            "| {} | {:?} | {} | {} | {} |",
            id,
            s.state,
            s.execution
                .as_ref()
                .map(|x| x.compute_units.to_string())
                .unwrap_or_else(|| "unavailable".into()),
            s.execution
                .as_ref()
                .map(|x| x.transaction_fee_lamports.to_string())
                .unwrap_or_else(|| "unavailable".into()),
            s.reason
                .as_deref()
                .unwrap_or("preservation verified")
                .replace('|', "/")
        )?;
    }
    let assumptions = report.k1.input["configuration"].clone();
    writeln!(text, "\nConfiguration instruction, compiled message and simulation assumptions:\n\n```json\n{}\n```\n\nVerified C1 handoffs (full pool and execution bytes retained in machine report):\n\n```json\n{}\n```\n", serde_json::to_string_pretty(&json!({"instruction":assumptions["instruction"], "message":assumptions["message"], "manager":assumptions["manager_assumption"], "fee_payer":assumptions["fee_payer"], "clock":assumptions["clock"]}))?, serde_json::to_string_pretty(&json!({"R01":report.r01.parent,"R11":report.r11.parent}))?)?;
    writeln!(
        text,
        "## Action matrix\n\n| Cell | State | Compute units | Reason |\n| --- | --- | ---: | --- |"
    )?;
    for (id, s) in [
        ("R00", &report.r00),
        ("R01", &report.r01),
        ("R10", &report.r10),
        ("R11", &report.r11),
    ] {
        writeln!(
            text,
            "| {} | {:?} | {} | {} |",
            id,
            s.state,
            s.execution
                .as_ref()
                .and_then(|x| x.compute_units)
                .map(|n| n.to_string())
                .unwrap_or_else(|| "unavailable".into()),
            s.reason
                .as_deref()
                .unwrap_or("reconciled")
                .replace('|', "/")
        )?;
    }
    writeln!(
        text,
        "\n| Reconciled measurement | R00 | R01 | R10 | R11 |\n| --- | ---: | ---: | ---: | ---: |"
    )?;
    for (metric, cells) in &report.measurements {
        let values = cells.each_ref().map(|q| {
            q.value.clone().unwrap_or_else(|| {
                format!(
                    "undefined: {}",
                    q.unavailable_reason.as_deref().unwrap_or("unavailable")
                )
            })
        });
        writeln!(
            text,
            "| {} | {} | {} | {} | {} |",
            metric, values[0], values[1], values[2], values[3]
        )?;
    }
    writeln!(text, "\n## Effects\n\nParameter V1 = R01−R00; parameter V2 = R11−R10; code C0 = R10−R00; code C1 = R11−R01; combined = R11−R00; interaction = parameter V2−parameter V1. Cross-check: combined = code C0 + parameter V1 + interaction. Checked signed integers; configuration fees excluded.\n\n```json\n{}\n```\n\n{}\n\n## Portable evidence\n\nBoth original proposals, full code/account bytes, exact inputs, outputs, logs, CPI traces, and derivations are in the CAS objects referenced by manifest.json. The manifest is written last. No original repository, bundle, recipe, provider or qualification file is needed.\n\nVerify without execution: `eplyx interaction verify --artifact <directory> --format json`\n\nRe-execute offline: `eplyx interaction reproduce --artifact <directory> --format json`\n", serde_json::to_string_pretty(&report.effects)?,report.limitations)?;
    Ok(text)
}
/// Fresh directory, CAS objects first, manifest last. No original paths retained.
pub fn save(report: &Report, directory: &Path) -> Result<()> {
    verify(report)?;
    ensure!(
        report.input.historical.programs.len() + 6 <= 32,
        "artifact object count exceeds bound"
    );
    std::fs::create_dir(directory).context("output directory must be fresh")?;
    std::fs::create_dir(directory.join("objects"))?;
    let mut objects = BTreeMap::new();
    for (name, value) in [
        ("input", serde_json::to_value(&report.input)?),
        ("report", serde_json::to_value(report)?),
        ("upgrade", serde_json::to_value(&report.input.upgrade)?),
        ("parameter", serde_json::to_value(&report.input.parameter)?),
    ] {
        objects.insert(
            name.into(),
            put(directory, canonical::document(&value)?.as_bytes())?,
        );
    }
    objects.insert(
        "candidate_elf".into(),
        put(directory, &report.input.candidate.elf)?,
    );
    for p in &report.input.historical.programs {
        objects.insert(format!("program_{}", p.program_id), put(directory, &p.elf)?);
    }
    let readable = pretty_report(report)?;
    objects.insert(
        "readable_report".into(),
        put(directory, readable.as_bytes())?,
    );
    write_new(&directory.join("report.md"), readable.as_bytes())?;
    let manifest = Manifest {
        schema: REVISION.into(),
        analysis_input_sha256: report.analysis_input_sha256.clone(),
        report_sha256: report.report_sha256.clone(),
        objects,
    };
    write_new(
        &directory.join("manifest.json"),
        canonical::document(&manifest)?.as_bytes(),
    )
}
fn get(root: &Path, hash: &str) -> Result<Vec<u8>> {
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid CAS reference"
    );
    let relative = format!("objects/{hash}");
    let path = crate::lifecycle::artifact::member(root, &relative)?;
    let bytes = crate::lifecycle::artifact::read(path)?;
    ensure!(replay::hash_bytes(&bytes) == hash, "CAS content mismatch");
    Ok(bytes)
}
/// All referenced objects and cross-object bindings are checked before VM use.
pub fn load(directory: &Path) -> Result<Report> {
    ensure!(
        !std::fs::symlink_metadata(directory)?
            .file_type()
            .is_symlink(),
        "artifact directory symlink forbidden"
    );
    let manifest_path = crate::lifecycle::artifact::member(directory, "manifest.json")?;
    let bytes = crate::lifecycle::artifact::read(manifest_path)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    ensure!(
        bytes == canonical::document(&manifest)?.as_bytes() && manifest.schema == REVISION,
        "manifest schema/canonical fields differ"
    );
    ensure!(
        manifest.objects.len() <= 32,
        "artifact object count exceeds bound"
    );
    let object = |name: &str| -> Result<Vec<u8>> {
        get(
            directory,
            manifest
                .objects
                .get(name)
                .context("missing manifest object")?,
        )
    };
    let raw = object("report")?;
    let report: Report = serde_json::from_slice(&raw)?;
    ensure!(
        raw == canonical::document(&serde_json::to_value(&report)?)?.as_bytes(),
        "report strict/canonical fields differ"
    );
    verify(&report)?;
    ensure!(
        manifest.analysis_input_sha256 == report.analysis_input_sha256
            && manifest.report_sha256 == report.report_sha256,
        "manifest report identities differ"
    );
    let mut expected = BTreeMap::new();
    for (name, value) in [
        ("input", serde_json::to_value(&report.input)?),
        ("report", serde_json::to_value(&report)?),
        ("upgrade", serde_json::to_value(&report.input.upgrade)?),
        ("parameter", serde_json::to_value(&report.input.parameter)?),
    ] {
        expected.insert(
            name.into(),
            replay::hash_bytes(canonical::document(&value)?.as_bytes()),
        );
    }
    expected.insert(
        "candidate_elf".into(),
        replay::hash_bytes(&report.input.candidate.elf),
    );
    for p in &report.input.historical.programs {
        expected.insert(
            format!("program_{}", p.program_id),
            replay::hash_bytes(&p.elf),
        );
    }
    let readable = pretty_report(&report)?;
    expected.insert(
        "readable_report".into(),
        replay::hash_bytes(readable.as_bytes()),
    );
    ensure!(
        manifest.objects == expected,
        "manifest CAS cross-object bindings differ"
    );
    for hash in manifest.objects.values() {
        get(directory, hash)?;
    }
    ensure!(
        crate::lifecycle::artifact::read(crate::lifecycle::artifact::member(
            directory,
            "report.md"
        )?)? == readable.as_bytes(),
        "readable report differs"
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bundle::CiBundle, change::Change};
    use std::sync::OnceLock;
    fn input() -> Input {
        let root = crate::repo_root();
        let historical = s::Input::from_bundle(
            &CiBundle::open(root.join("deploy/bundle")).unwrap(),
            "mainnet-spl-stake-pool-151010f709e113e7",
        )
        .unwrap();
        let parameter = ChangeSpec::parse(
            &std::fs::read(root.join("docs/examples/stake-pool-parameter-change.json")).unwrap(),
        )
        .unwrap();
        let candidate =
            std::fs::read(root.join("artifacts/fixture_stake_pool_config_v2.so")).unwrap();
        let upgrade = ChangeSpec::program_upgrade(&historical.record.program_id, &candidate);
        Input::new(&upgrade, &parameter, historical, candidate).unwrap()
    }
    fn report() -> Report {
        static REPORT: OnceLock<Report> = OnceLock::new();
        REPORT.get_or_init(|| analyse(&input()).unwrap()).clone()
    }
    fn seal(r: &mut Report) {
        r.report_sha256 = report_digest(r).unwrap();
    }
    fn runs(r: &Report) -> Runs {
        Runs {
            k1: retained(&r.k1),
            k2: retained(&r.k2),
            r00: retained(&r.r00),
            r01: retained(&r.r01),
            r10: retained(&r.r10),
            r11: retained(&r.r11),
        }
    }
    fn q(n: i128) -> Quantity {
        Quantity::of(n)
    }
    #[test]
    fn calculator_unit_measurements_nonzero_additive_signed_overflow_and_missing() {
        // Calculator unit-test values only; not VM/mainnet measurements.
        let e = contrasts([q(1000), q(900), q(1000), q(850)]).unwrap();
        assert_eq!(e.parameter_v1, q(-100));
        assert_eq!(e.parameter_v2, q(-150));
        assert_eq!(e.interaction, q(-50));
        assert_eq!(e.identity_cross_check, Some(true));
        let e = contrasts([q(-1000), q(-1100), q(1000), q(900)]).unwrap();
        assert_eq!(e.interaction, q(0));
        let e = contrasts([q(i128::MAX), q(i128::MIN), q(i128::MAX), q(i128::MIN)]).unwrap();
        assert!(e.parameter_v1.value.is_none());
        assert!(e.interaction.value.is_none());
        let e = contrasts([q(1000), q(900), q(1000), Quantity::absent("K2 rejected")]).unwrap();
        assert_eq!(e.parameter_v1, q(-100));
        assert!(e.parameter_v2.value.is_none());
        assert!(e.interaction.value.is_none());
        assert!(contrasts([
            Quantity {
                value: Some("01".into()),
                unavailable_reason: None
            },
            q(0),
            q(0),
            q(0)
        ])
        .is_err());
    }
    #[test]
    fn qualified_six_execution_matrix_and_frozen_repeat() {
        let r = report();
        assert_eq!(r.status, Status::NoMeasuredInteraction);
        verify(&r).unwrap();
        assert_eq!(r.r00.derived["historical_fidelity"]["status"], "matched");
        for stage in [&r.r00, &r.r01, &r.r10, &r.r11] {
            assert_eq!(stage.state, State::Verified);
            assert!(stage.execution.is_some());
            assert_eq!(
                stage.derived["reconciliation"]["config_transaction_fee_included"],
                false
            );
        }
        assert_eq!(
            r.measurements["recipient_account_credit_raw"],
            [q(760985008), q(753375157), q(760985008), q(753375157)]
        );
        assert_eq!(
            r.effects["recipient_account_credit_raw"].parameter_v1,
            q(-7609851)
        );
        assert_eq!(
            r.effects["recipient_account_credit_raw"].parameter_v2,
            q(-7609851)
        );
        assert_eq!(r.effects["recipient_account_credit_raw"].interaction, q(0));
        assert!(r.measurements["referral_account_credit_raw"]
            .iter()
            .all(|q| q.value.is_none()));
        assert_eq!(r.r01.parent.as_ref().unwrap().stage, "K1");
        assert_eq!(r.r11.parent.as_ref().unwrap().stage, "K2");
        assert_eq!(
            r.r01.parent.as_ref().unwrap().pool_sha256,
            r.r11.parent.as_ref().unwrap().pool_sha256
        );
        assert_ne!(r.k1.input_sha256, r.k2.input_sha256);
        assert_eq!(r.input.historical, input().historical);
        reproduce(&r).unwrap();
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        save(&r, &a).unwrap();
        save(&analyse(&r.input).unwrap(), &b).unwrap();
        assert_eq!(
            std::fs::read(a.join("manifest.json")).unwrap(),
            std::fs::read(b.join("manifest.json")).unwrap()
        );
        assert_eq!(load(&a).unwrap(), r);
        assert!(save(&r, &a).is_err());
    }
    #[test]
    fn actual_same_code_runs_all_six_instead_of_short_circuiting() {
        let mut i = input();
        let bytes = i
            .historical
            .programs
            .iter()
            .find(|p| p.program_id == i.historical.record.program_id)
            .unwrap()
            .elf
            .clone();
        let upgrade = ChangeSpec::program_upgrade(&i.historical.record.program_id, &bytes);
        i = Input::new(&upgrade, &i.parameter, i.historical, bytes).unwrap();
        let r = analyse(&i).unwrap();
        verify(&r).unwrap();
        assert_eq!(r.status, Status::NoMeasuredInteraction);
        assert_eq!(r.k1.state, State::Verified);
        assert_eq!(r.k2.state, State::Verified);
        for s in [&r.r00, &r.r01, &r.r10, &r.r11] {
            assert!(s.execution.is_some());
        }
        assert_ne!(r.k1.input_sha256, r.k2.input_sha256);
        assert_eq!(r.effects["recipient_account_credit_raw"].interaction, q(0));
    }
    #[test]
    fn invalid_fee_really_executes_and_rejects_both_configurations() {
        let mut i = input();
        if let Change::ProtocolParameterChange(p) = &mut i.parameter.change {
            if let crate::parameter_change::Operation::SplStakePoolSolDepositFeeV1 {
                proposed_fee,
                ..
            } = &mut p.operation
            {
                proposed_fee.numerator = 2;
                proposed_fee.denominator = 1;
            }
        }
        i.parameter.change_spec_id = None;
        let r = analyse(&i).unwrap();
        verify(&r).unwrap();
        assert_eq!(r.status, Status::NotEstablished);
        for k in [&r.k1, &r.k2] {
            assert_eq!(k.state, State::Rejected);
            assert!(k.execution.is_some());
            assert_eq!(k.derived["rollback_verified"], true);
        }
        assert_eq!(r.r00.state, State::Verified);
        assert_eq!(r.r10.state, State::Verified);
        assert_eq!(r.r01.state, State::NotExecuted);
        assert_eq!(r.r11.state, State::NotExecuted);
        assert!(r.r11.input["action"].is_null());
        assert!(r.r01.input["action"].is_null());
        assert!(r.effects["recipient_account_credit_raw"]
            .interaction
            .value
            .is_none());
    }
    #[test]
    fn partial_failures_retain_independent_results_and_never_make_zero() {
        let r = report();
        let mut raw = runs(&r);
        raw.k2 = Some(Attempt::Unavailable(
            "test-only infrastructure failure".into(),
        ));
        raw.r11 = None;
        let partial = assemble(&r.input, raw).unwrap();
        verify(&partial).unwrap();
        assert_eq!(partial.status, Status::NotEstablished);
        assert_eq!(
            partial.effects["recipient_account_credit_raw"].parameter_v1,
            q(-7609851)
        );
        assert!(partial.effects["recipient_account_credit_raw"]
            .interaction
            .value
            .is_none());
        assert_eq!(partial.r10, r.r10);
        let mut raw = runs(&r);
        if let Some(Attempt::Executed(x)) = &mut raw.r11 {
            x.accounts
                .get_mut("destination-pool-token")
                .unwrap()
                .lamports += 1;
        }
        let partial = assemble(&r.input, raw).unwrap();
        verify(&partial).unwrap();
        assert_eq!(partial.r11.state, State::Unreconciled);
        assert!(partial.effects["recipient_account_credit_raw"]
            .interaction
            .value
            .is_none());
        let mut raw = runs(&r);
        if let Some(Attempt::Executed(x)) = &mut raw.r00 {
            x.accounts.get_mut("destination-pool-token").unwrap().data[64] ^= 1;
        }
        raw.k1 = None;
        raw.k2 = None;
        raw.r01 = None;
        raw.r10 = None;
        raw.r11 = None;
        let partial = assemble(&r.input, raw).unwrap();
        verify(&partial).unwrap();
        assert_eq!(partial.r00.state, State::Unreconciled);
        assert_eq!(partial.k1.state, State::NotExecuted);
        let partial = assemble(
            &r.input,
            Runs {
                r00: Some(Attempt::Unavailable(
                    "test-only infrastructure failure".into(),
                )),
                ..Default::default()
            },
        )
        .unwrap();
        verify(&partial).unwrap();
        assert!(partial.r00.execution.is_none());
    }
    #[test]
    fn typed_admission_failures_and_input_identity_binding() {
        let base = input();
        let identity = report().analysis_input_sha256;
        let mut cases = Vec::new();
        let mut i = base.clone();
        i.candidate.elf[100] ^= 1;
        cases.push(i);
        let mut i = base.clone();
        i.historical.record.clock.epoch = 1;
        i.historical.record_sha256 = canonical::digest(&i.historical.record).unwrap();
        cases.push(i);
        let mut i = base.clone();
        i.historical.programs[0].loader = "BPFLoader2111111111111111111111111111111111".into();
        cases.push(i);
        let mut i = base.clone();
        i.upgrade = base.parameter.clone();
        cases.push(i);
        let mut i = base.clone();
        i.parameter = base.upgrade.clone();
        cases.push(i);
        let mut i = base.clone();
        if let Change::ProgramUpgrade { target, .. } = &mut i.upgrade.change {
            target.program_id = crate::protocol::stake_pool::TOKEN_PROGRAM_ID.into();
        }
        i.upgrade.change_spec_id = None;
        cases.push(i);
        let mut i = base.clone();
        if let Change::ProgramUpgrade { replaces, .. } = &mut i.upgrade.change {
            *replaces = Some(ExecutableArtifact::of(b"wrong"));
        }
        i.upgrade.change_spec_id = None;
        cases.push(i);
        let mut i = base.clone();
        if let Change::ProgramUpgrade { target, .. } = &mut i.upgrade.change {
            target.programdata_address = Some(base.historical.record.program_id.clone());
        }
        i.upgrade.change_spec_id = None;
        cases.push(i);
        let mut i = base.clone();
        if let Change::ProtocolParameterChange(p) = &mut i.parameter.change {
            p.target.config_account = base.historical.record.program_id.clone();
        }
        i.parameter.change_spec_id = None;
        cases.push(i);
        let mut i = base.clone();
        if let Change::ProtocolParameterChange(p) = &mut i.parameter.change {
            if let crate::parameter_change::Operation::SplStakePoolSolDepositFeeV1 {
                expected_current,
                ..
            } = &mut p.operation
            {
                expected_current.numerator = 99;
            }
        }
        i.parameter.change_spec_id = None;
        cases.push(i);
        let mut i = base.clone();
        i.historical.record.accounts[0].account.lamports += 1;
        i.historical.record.pre_state_hash =
            replay::state_hash(&i.historical.record.accounts).unwrap();
        i.historical.record_sha256 = canonical::digest(&i.historical.record).unwrap();
        cases.push(i);
        let mut i = base.clone();
        i.upgrade.activation = Some(crate::change::Activation {
            slot: Some(1),
            unix_timestamp: None,
        });
        i.upgrade.change_spec_id = None;
        cases.push(i);
        let mut i = base.clone();
        if let Change::ProgramUpgrade {
            expected_upgrade_authority,
            ..
        } = &mut i.upgrade.change
        {
            *expected_upgrade_authority = Some(base.historical.record.program_id.clone());
        }
        i.upgrade.change_spec_id = None;
        cases.push(i);
        let mut i = base.clone();
        if let Change::ProgramUpgrade { candidate, .. } = &mut i.upgrade.change {
            candidate.len -= 1;
        }
        i.upgrade.change_spec_id = None;
        cases.push(i);
        let mut i = base.clone();
        let account = &mut i
            .historical
            .record
            .accounts
            .iter_mut()
            .find(|a| a.label == "stake-pool")
            .unwrap()
            .account;
        let (mut state, trailing) = s::decode(account).unwrap();
        state.sol_deposit_fee.numerator = 1;
        account.data = borsh::to_vec(&state).unwrap();
        account.data.extend(trailing);
        i.historical.record.pre_state_hash =
            replay::state_hash(&i.historical.record.accounts).unwrap();
        i.historical.record_sha256 = canonical::digest(&i.historical.record).unwrap();
        cases.push(i);
        let mut i = base.clone();
        i.historical
            .programs
            .iter_mut()
            .find(|p| p.program_id == crate::protocol::stake_pool::TOKEN_PROGRAM_ID)
            .unwrap()
            .elf[100] ^= 1;
        cases.push(i);
        for i in cases {
            let r = analyse(&i).unwrap();
            verify(&r).unwrap();
            assert_eq!(r.status, Status::NotEstablished);
            assert_ne!(r.analysis_input_sha256, identity);
        }
        let mut i = base.clone();
        let bytes =
            std::fs::read(crate::repo_root().join("artifacts/fixture_stake_pool_v2.so")).unwrap();
        i = Input::new(
            &ChangeSpec::program_upgrade(&i.historical.record.program_id, &bytes),
            &i.parameter,
            i.historical,
            bytes,
        )
        .unwrap();
        let r = analyse(&i).unwrap();
        assert_eq!(r.admission["qualified"], false);
        assert!(r.admission["reason"]
            .as_str()
            .unwrap()
            .contains("unqualified"));
        assert_eq!(r.r00.state, State::NotExecuted);
        let mut p = base.parameter.clone();
        p.metadata.label = Some("cosmetic title".into());
        let i = Input::new(&base.upgrade, &p, base.historical, base.candidate.elf).unwrap();
        assert_eq!(
            canonical::digest(&contract(&i, &admission(&i)).unwrap()).unwrap(),
            identity
        );
        assert_eq!(i.parameter.id().unwrap(), input().parameter.id().unwrap());
        assert_eq!(
            i.parameter.metadata.label.as_deref(),
            Some("cosmetic title")
        );
    }
    #[test]
    fn resealed_stage_handoff_code_message_reconciliation_and_account_tamper_fail() {
        let r = report();
        let mut cases = Vec::new();
        let mut x = r.clone();
        x.r11.parent = x.r01.parent.clone();
        cases.push(x);
        let mut x = r.clone();
        x.r11.input["effective_target"]["elf_sha256"] =
            x.r00.input["effective_target"]["elf_sha256"].clone();
        cases.push(x);
        let mut x = r.clone();
        x.r10.input["action"]["message"]["header"] = json!("changed");
        cases.push(x);
        let mut x = r.clone();
        x.r11.derived["reconciliation"]["recipient_account_credit_raw"] = "0".into();
        cases.push(x);
        let mut x = r.clone();
        x.k2.derived["verified_pool"]["lamports"] = json!(1);
        cases.push(x);
        let mut x = r.clone();
        x.effects
            .get_mut("recipient_account_credit_raw")
            .unwrap()
            .interaction = q(1);
        cases.push(x);
        let mut x = r.clone();
        x.r01
            .execution
            .as_mut()
            .unwrap()
            .accounts
            .get_mut("destination-pool-token")
            .unwrap()
            .data[64] ^= 1;
        cases.push(x);
        let mut x = r.clone();
        x.input.candidate.elf[100] ^= 1;
        cases.push(x);
        let mut x = r.clone();
        x.input.historical.record.clock.slot += 1;
        cases.push(x);
        for mut x in cases {
            seal(&mut x);
            assert!(verify(&x).is_err());
        }
    }
    #[test]
    fn portable_artifact_tamper_and_resealed_manifest_rejected() {
        let r = report();
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("artifact");
        save(&r, &a).unwrap();
        let mut m: Manifest =
            serde_json::from_slice(&std::fs::read(a.join("manifest.json")).unwrap()).unwrap();
        let hash = m.objects["candidate_elf"].clone();
        let path = a.join("objects").join(hash);
        std::fs::write(&path, b"tampered ELF").unwrap();
        assert!(load(&a).is_err());
        std::fs::write(&path, &r.input.candidate.elf).unwrap();
        m.objects.insert(
            "candidate_elf".into(),
            put(&a, b"resealed unrelated ELF").unwrap(),
        );
        std::fs::write(a.join("manifest.json"), canonical::document(&m).unwrap()).unwrap();
        assert!(load(&a).is_err());
    }
    #[test]
    fn archived_parameter_receipt_keeps_its_source_identity() {
        // Frozen report produced before the small shared helper extraction.
        let path = Path::new("/private/tmp/eplyx-upgrade-parameter-probe/probe.json");
        let i = input();
        let spec = i.parameter;
        let r = if path.exists() {
            let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            v["v1_report"].clone()
        } else {
            let mut report = s::analyze(&spec, &i.historical).unwrap();
            fn retained_source(v: &mut Value) {
                match v {
                    Value::Object(map) => {
                        for (key, value) in map {
                            if key == "runtime" {
                                value["operation_source_sha256"]="63aa307015b17f79d6b07e71f024f624eb2388ad15e6ba8264a5b3fa2a9a9a87".into();
                            } else {
                                retained_source(value);
                            }
                        }
                    }
                    Value::Array(items) => {
                        for item in items {
                            retained_source(item);
                        }
                    }
                    _ => (),
                }
            }
            retained_source(&mut report);
            report.as_object_mut().unwrap().remove("report_sha256");
            report["report_sha256"] = canonical::digest(&report).unwrap().into();
            report
        };
        assert!(r.is_object(), "archived report key changed");
        s::verify(&spec, &r).unwrap();
        s::reproduce(&spec, &r).unwrap();
        let mut altered = r.clone();
        altered["runtime"]["executor_sha256"] = "unreviewed".into();
        altered.as_object_mut().unwrap().remove("report_sha256");
        let seal = canonical::digest(&altered).unwrap();
        altered["report_sha256"] = seal.into();
        assert!(s::verify(&spec, &altered).is_err());
    }
}
