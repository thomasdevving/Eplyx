//! A selected pair under one migration proposal, never a workflow or population search.
use super::{
    adapter,
    execute::{self, Bank, BankAccount, BankOrigin, Outcome, Session, UnitExecution},
    planner::{self, ImpactClass, MigrationPlan, MigrationUnit, PlanInput, RehearsalClockPolicy},
    spec::{DestinationFunding, TokenMigrationV1, WindowState},
    world::{World, WorldClock, WorldKind},
};
use crate::{
    canonical,
    change::{CandidateSource, ChangeSpec, ExecutableArtifact},
    executor::LoadedProgram,
    replay::hash_bytes,
    types::AccountSnapshot,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const VERSION: &str = "eplyx-migration-order-case/v1";
pub const HANDOFF: &str = "same_bank_separate_transactions_no_time_advance";
pub const FINDING: &str = "migration/order/shared_reserve/order_changes_successful_holder";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureKind {
    UnsupportedComposition,
    EvidenceGap,
    HandoffFailure,
    UnexpectedWrite,
    ReconciliationMismatch,
}
#[derive(Debug)]
pub struct OrderError {
    pub kind: FailureKind,
    pub detail: String,
}
impl std::fmt::Display for OrderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)
    }
}
impl std::error::Error for OrderError {}
fn require(ok: bool, kind: FailureKind, detail: impl Into<String>) -> Result<()> {
    if !ok {
        return Err(OrderError {
            kind,
            detail: detail.into(),
        }
        .into());
    }
    Ok(())
}
pub(crate) fn evidence(error: anyhow::Error) -> anyhow::Error {
    if error.downcast_ref::<OrderError>().is_some() {
        error
    } else {
        OrderError {
            kind: FailureKind::EvidenceGap,
            detail: format!("{error:#}"),
        }
        .into()
    }
}

/// Local VM contract. The dependency lock and executor source pin defaults,
/// feature set, fee rules, loader behavior and disabled signature/blockhash checks.
pub fn runtime_identity() -> Result<String> {
    canonical::digest(&(
        "eplyx-migration-order-runtime/v1",
        execute::EXECUTOR_VERSION,
        hash_bytes(include_bytes!("../../../Cargo.lock")),
        hash_bytes(include_bytes!("../../../Cargo.toml")),
        hash_bytes(include_bytes!("../../Cargo.toml")),
        hash_bytes(include_bytes!("execute.rs")),
        "LiteSVM::new; sigverify=false; blockhash_check=false; transaction_history=0; no warp",
    ))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "existence", deny_unknown_fields)]
pub enum AccountEvidence {
    Present { account: AccountSnapshot },
    KnownAbsent,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateEvidence {
    pub version: String,
    pub binding_id: String,
    /// Missing key means unknown, NEVER known absent.
    pub accounts: BTreeMap<String, AccountEvidence>,
}
impl StateEvidence {
    pub fn id(&self) -> Result<String> {
        canonical::digest(self)
    }
    pub fn account(&self, address: &str) -> Result<Option<&AccountSnapshot>> {
        match self.accounts.get(address) {
            Some(AccountEvidence::Present { account }) => Ok(Some(account)),
            Some(AccountEvidence::KnownAbsent) => Ok(None),
            None => Err(OrderError {
                kind: FailureKind::HandoffFailure,
                detail: format!("required account {address} outside closure"),
            }
            .into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramIdentity {
    pub program_id: String,
    pub loader: String,
    pub artifact: ExecutableArtifact,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairBinding {
    pub version: String,
    pub change_spec_id: String,
    pub candidate: ExecutableArtifact,
    pub world_id: String,
    /// Covers every byte even for a derived World's parent-dependent identity.
    pub world_content_sha256: String,
    pub world_kind: WorldKind,
    pub planner: String,
    pub adapter: String,
    pub runtime_id: String,
    pub clock: WorldClock,
    pub programs: Vec<ProgramIdentity>,
    pub reserve: String,
    pub initial_reserve_raw: String,
    /// Focused plans: each unit's amount, quote, authority and required signers.
    pub units: [MigrationUnit; 2],
    pub closure: BTreeSet<String>,
    pub handoff: String,
}
impl PairBinding {
    pub fn id(&self) -> Result<String> {
        canonical::digest(self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderCase {
    pub version: String,
    pub binding_id: String,
    pub initial_state_id: String,
    pub ordered_unit_ids: Vec<String>,
}
impl OrderCase {
    pub fn id(&self) -> Result<String> {
        canonical::digest(self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub before_state_id: String,
    pub after_state_id: String,
    pub expected_reserve_before_raw: String,
    pub expected_funded: bool,
    pub observed_reserve_before_raw: String,
    pub observed_reserve_after_raw: String,
    pub execution: UnitExecution,
    pub unexpected_writes: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub case: OrderCase,
    pub case_id: String,
    pub run_id: String,
    pub initial_state_id: String,
    pub final_state_id: String,
    pub steps: Vec<Step>,
    pub stopped: Option<FailureKind>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonStatus {
    SharedReserveChangesSuccessfulUnit,
    NoSuccessfulUnitEffect,
    NotEstablished,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDifference {
    pub unit_id: String,
    pub a_then_b: UnitExecution,
    pub b_then_a: UnitExecution,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comparison {
    pub status: ComparisonStatus,
    pub finding: Option<String>,
    pub scenario_ids: [String; 4],
    pub order_case_ids: [String; 2],
    pub affected_units: Vec<UnitDifference>,
    pub limitations: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    pub binding: PairBinding,
    /// Content-addressed snapshots, shared between scenarios.
    pub states: BTreeMap<String, StateEvidence>,
    /// A solo, B solo, A→B, B→A, in that order.
    pub scenarios: [Scenario; 4],
    pub comparison: Comparison,
}

fn runtime_accounts() -> BTreeSet<String> {
    use solana_sdk_ids::sysvar;
    [
        sysvar::clock::id(),
        sysvar::rent::id(),
        sysvar::epoch_rewards::id(),
        sysvar::epoch_schedule::id(),
        sysvar::last_restart_slot::id(),
        sysvar::slot_hashes::id(),
        sysvar::slot_history::id(),
        sysvar::stake_history::id(),
        sysvar::fees::id(),
        sysvar::recent_blockhashes::id(),
    ]
    .into_iter()
    .map(|a| a.to_string())
    .collect()
}
fn snapshot(session: &Session, binding: &PairBinding) -> Result<StateEvidence> {
    Ok(StateEvidence {
        version: "eplyx-migration-order-state/v1".into(),
        binding_id: binding.id()?,
        accounts: binding
            .closure
            .iter()
            .map(|a| {
                (
                    a.clone(),
                    match session.account(a) {
                        Some(account) => AccountEvidence::Present { account },
                        None => AccountEvidence::KnownAbsent,
                    },
                )
            })
            .collect(),
    })
}
fn reserve(state: &StateEvidence, address: &str) -> Result<u64> {
    let a = state.account(address)?.context("reserve absent")?;
    Ok(crate::standard_programs::token::account_amounts(&a.owner, &a.data)?.0)
}
fn retain(states: &mut BTreeMap<String, StateEvidence>, state: StateEvidence) -> Result<String> {
    let id = state.id()?;
    states.insert(id.clone(), state);
    Ok(id)
}

/// Restore a byte-bearing bounded state, checking every declared present/absent
/// row after the normal loader has reconstructed its program cache and sysvars.
pub fn restore(
    state: &StateEvidence,
    binding: &PairBinding,
    programs: &[LoadedProgram],
) -> Result<Session> {
    restore_inner(state, binding, programs).map_err(evidence)
}
fn restore_inner(
    state: &StateEvidence,
    binding: &PairBinding,
    programs: &[LoadedProgram],
) -> Result<Session> {
    require(
        state.binding_id == binding.id()?
            && state.accounts.keys().cloned().collect::<BTreeSet<_>>() == binding.closure,
        FailureKind::HandoffFailure,
        "state binding or closure differs",
    )?;
    require(
        binding.runtime_id == runtime_identity()?
            && program_identities(programs) == binding.programs,
        FailureKind::EvidenceGap,
        "runtime or program identity differs",
    )?;
    let bank = Bank {
        accounts: state
            .accounts
            .iter()
            .filter_map(|(a, e)| match e {
                AccountEvidence::Present { account } => Some((
                    a.clone(),
                    BankAccount {
                        account: account.clone(),
                        origin: BankOrigin::Proposed {
                            derivation: "verified simulated order state".into(),
                        },
                    },
                )),
                AccountEvidence::KnownAbsent => None,
            })
            .collect(),
        clock: binding.clock,
        clock_basis: "Captured".into(),
        relayer: execute::relayer().to_string(),
    };
    let program_id = programs
        .last()
        .context("candidate program missing")?
        .program_id
        .to_string();
    let session = Session::new(&bank, programs, &program_id)?;
    require(
        snapshot(&session, binding)? == *state,
        FailureKind::HandoffFailure,
        "restored state differs from exact evidence",
    )?;
    Ok(session)
}
fn program_identities(programs: &[LoadedProgram]) -> Vec<ProgramIdentity> {
    programs
        .iter()
        .map(|p| ProgramIdentity {
            program_id: p.program_id.to_string(),
            loader: p.loader.to_string(),
            artifact: ExecutableArtifact::of(&p.bytes),
        })
        .collect()
}

pub fn analyse(
    change: &ChangeSpec,
    world: &World,
    candidate: &[u8],
    sources: [&str; 2],
) -> Result<Analysis> {
    analyse_inner(change, world, candidate, sources).map_err(evidence)
}
fn analyse_inner(
    change: &ChangeSpec,
    world: &World,
    candidate: &[u8],
    sources: [&str; 2],
) -> Result<Analysis> {
    change.validate()?;
    world.validate()?;
    let migration = change
        .as_token_migration()
        .context("a TokenMigration ChangeSpec is required")?;
    let spec = migration.evaluation_spec(change.activation.as_ref())?;
    require(
        matches!(
            spec.destination_funding,
            DestinationFunding::ReserveTransfer { .. }
        ),
        FailureKind::UnsupportedComposition,
        "only shared reserve transfer is supported",
    )?;
    require(
        sources[0] != sources[1],
        FailureKind::UnsupportedComposition,
        "duplicate unit",
    )?;
    require(
        spec.resolve()?
            .window_state(world.clock.slot, world.clock.unix_timestamp)
            == WindowState::Open,
        FailureKind::UnsupportedComposition,
        "migration window is not open at the pinned Clock",
    )?;
    let resolved = change.resolve(CandidateSource::Bytes(candidate))?;
    let change_id = change.id()?;
    let plans = sources
        .iter()
        .map(|source| {
            planner::plan(&PlanInput {
                spec: &spec,
                change_spec_id: &change_id,
                world,
                program_id: &migration.mechanism.program_id,
                candidate_program_sha256: &migration.mechanism.artifact.sha256,
                clock_policy: RehearsalClockPolicy::Captured,
                reserve_override: None,
                focus: Some(source),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut selected = Vec::new();
    for (plan, source) in plans.iter().zip(sources) {
        let unit = plan
            .unit(source)
            .context("selected source is not a unit in this world and ChangeSpec")?;
        require(
            unit.class == ImpactClass::Migratable
                && unit.quote.is_some()
                && unit.expected.is_some(),
            if matches!(
                unit.class,
                ImpactClass::UnverifiableAuthority
                    | ImpactClass::AuthorityPathUnavailable
                    | ImpactClass::UnverifiableDestination
                    | ImpactClass::FundingPathUnavailable
            ) {
                FailureKind::EvidenceGap
            } else {
                FailureKind::UnsupportedComposition
            },
            format!(
                "solo control {} cannot be constructed: {:?}: {:?}",
                unit.unit_id, unit.class, unit.reasons
            ),
        )?;
        // Signature verification is deliberately disabled in the local runtime.
        // Every assumed external signer still needs captured wallet-compatible
        // evidence; a declared address or PDA alone must not become a signer.
        for signer in &unit.required_signers {
            if signer.address != "relayer" {
                let (class, _, detail) = crate::evidence::authority::classify_authority(
                    &signer.address,
                    &world.rpc_value(&signer.address),
                )?;
                require(
                    class == crate::evidence::authority::EntityType::WalletCompatible,
                    FailureKind::EvidenceGap,
                    format!(
                        "signer {} lacks a supported direct signature path: {detail}",
                        signer.address
                    ),
                )?;
            }
        }
        selected.push(unit.clone());
    }
    let units: [MigrationUnit; 2] = selected
        .try_into()
        .map_err(|_| anyhow::anyhow!("two units required"))?;
    require(
        units[0].unit_id != units[1].unit_id,
        FailureKind::UnsupportedComposition,
        "duplicate unit identity",
    )?;
    let plan = &plans[0];
    let programs = execute::programs(world, &spec, &migration.mechanism.program_id, &resolved)?;
    let config = adapter::config_bytes(&spec, &spec.resolve()?, &plan.overlay, &change_id)?;
    let mut closure = runtime_accounts();
    closure.insert(execute::relayer().to_string());
    for unit in &units {
        closure.extend(execute::message_keys(
            &execute::unit_instructions(&spec, plan, unit, &execute::relayer().to_string())?.0,
        ));
        // Authority classification may read accounts absent from the transaction metas.
        closure.insert(unit.owner.clone());
        closure.extend(
            unit.required_signers
                .iter()
                .filter(|s| s.address != "relayer")
                .map(|s| s.address.clone()),
        );
    }
    for p in &programs {
        closure.insert(p.program_id.to_string());
        if p.loader.to_string() == crate::standard_programs::token::UPGRADEABLE_LOADER {
            closure.insert(
                crate::standard_programs::upgradeable_loader::programdata_address(&p.program_id)
                    .to_string(),
            );
        }
    }
    let bank = Bank::build_filtered(world, &spec, plan, &config, Some(&closure))?;
    let session = Session::new(&bank, &programs, &plan.overlay.program_id)?;
    let runtime = runtime_accounts();
    let builtins: BTreeSet<String> = [adapter::SYSTEM_PROGRAM, adapter::COMPUTE_BUDGET_PROGRAM]
        .into_iter()
        .map(str::to_string)
        .collect();
    let program_accounts: BTreeSet<String> = programs
        .iter()
        .flat_map(|p| {
            let mut keys = vec![p.program_id.to_string()];
            if p.loader.to_string() == crate::standard_programs::token::UPGRADEABLE_LOADER {
                keys.push(
                    crate::standard_programs::upgradeable_loader::programdata_address(
                        &p.program_id,
                    )
                    .to_string(),
                );
            }
            keys
        })
        .collect();
    for key in &closure {
        // Only named runtime defaults and exact program-loader outputs may supply
        // state without world evidence. VM defaults are not a live-state lookup.
        require(
            bank.get(key).is_some()
                || runtime.contains(key)
                || builtins.contains(key)
                || program_accounts.contains(key)
                || world.absence_known(key),
            FailureKind::EvidenceGap,
            format!("unknown required account/absence {key}"),
        )?;
        if !runtime.contains(key)
            && !builtins.contains(key)
            && !program_accounts.contains(key)
            && bank.get(key).is_none()
        {
            require(
                session.account(key).is_none(),
                FailureKind::EvidenceGap,
                format!("unexpected implicit runtime dependency {key}"),
            )?;
        }
    }
    let binding = PairBinding {
        version: VERSION.into(),
        change_spec_id: change_id,
        candidate: migration.mechanism.artifact.clone(),
        world_id: world.sha256()?,
        world_content_sha256: world_content_id(world)?,
        world_kind: world.kind,
        planner: planner::PLANNER_VERSION.into(),
        adapter: format!("{}/{}", adapter::ADAPTER, adapter::ADAPTER_VERSION),
        runtime_id: runtime_identity()?,
        clock: world.clock,
        programs: program_identities(&programs),
        reserve: plan
            .overlay
            .reserve_vault
            .clone()
            .context("reserve missing")?,
        initial_reserve_raw: plan
            .funding
            .available_raw
            .clone()
            .context("reserve amount unknown")?,
        units,
        closure,
        handoff: HANDOFF.into(),
    };
    let initial = snapshot(&session, &binding)?;
    require(
        reserve(&initial, &binding.reserve)?.to_string() == binding.initial_reserve_raw,
        FailureKind::EvidenceGap,
        "initial VM reserve differs from focused planning",
    )?;
    let mut states = BTreeMap::new();
    let initial_id = retain(&mut states, initial.clone())?;
    let mut scenarios = Vec::new();
    for order in [vec![0], vec![1], vec![0, 1], vec![1, 0]] {
        // Fresh Session every time, including the solos. Within scenario(), never reset.
        let fresh = restore(&initial, &binding, &programs)?;
        scenarios.push(scenario(
            &spec,
            plan,
            &binding,
            fresh,
            &initial_id,
            &order,
            &mut states,
        )?);
    }
    let scenarios: [Scenario; 4] = scenarios
        .try_into()
        .map_err(|_| anyhow::anyhow!("four scenarios required"))?;
    let comparison = compare(&binding, &scenarios);
    Ok(Analysis {
        binding,
        states,
        scenarios,
        comparison,
    })
}

/// World limitations are presentation; all account bytes and provenance are inputs.
pub fn world_content_id(world: &World) -> Result<String> {
    let mut value = serde_json::to_value(world)?;
    value
        .as_object_mut()
        .context("world object")?
        .remove("limitations");
    canonical::digest(&value)
}
#[allow(clippy::too_many_arguments)]
fn scenario(
    spec: &TokenMigrationV1,
    plan: &MigrationPlan,
    binding: &PairBinding,
    mut session: Session,
    initial_id: &str,
    order: &[usize],
    states: &mut BTreeMap<String, StateEvidence>,
) -> Result<Scenario> {
    let case = OrderCase {
        version: VERSION.into(),
        binding_id: binding.id()?,
        initial_state_id: initial_id.into(),
        ordered_unit_ids: order
            .iter()
            .map(|i| binding.units[*i].unit_id.clone())
            .collect(),
    };
    let mut result = Scenario {
        case_id: case.id()?,
        case,
        run_id: String::new(),
        initial_state_id: initial_id.into(),
        final_state_id: initial_id.into(),
        steps: vec![],
        stopped: None,
    };
    let mut expected_reserve: u64 = binding.initial_reserve_raw.parse()?;
    for index in order {
        let unit = &binding.units[*index];
        let before = snapshot(&session, binding)?;
        let clock_account = before
            .account(crate::standard_programs::token::CLOCK)?
            .context("Clock absent at handoff")?;
        require(
            WorldClock::from_bytes(&clock_account.data)? == binding.clock,
            FailureKind::HandoffFailure,
            "Clock changed at handoff",
        )?;
        require(
            before.id()? == result.final_state_id,
            FailureKind::HandoffFailure,
            "session state differs at handoff",
        )?;
        let (instructions, signers) =
            execute::unit_instructions(spec, plan, unit, &execute::relayer().to_string())?;
        validate_handoff(&before, &execute::message_keys(&instructions))?;
        let output: u64 = unit.quote.as_ref().context("quote")?.output_raw.parse()?;
        let expected_funded = output <= expected_reserve;
        // Distinct source units must retain their exact initial source/authority
        // evidence. Destination creation uses the existing idempotent ATA builder.
        let initial = states.get(initial_id).context("initial state missing")?;
        require(
            before.account(&unit.source_account)? == initial.account(&unit.source_account)?,
            FailureKind::HandoffFailure,
            "selected source changed before its transaction",
        )?;
        for signer in &unit.required_signers {
            if signer.address != "relayer" {
                require(
                    before.account(&signer.address)? == initial.account(&signer.address)?,
                    FailureKind::HandoffFailure,
                    "authority evidence changed at handoff",
                )?;
            }
        }
        let census_before = session.account_census();
        let raw = session.execute(&instructions, &execute::relayer().to_string())?;
        let census_after = session.account_census();
        let unexpected_writes = outside_writes(&binding.closure, &census_before, &census_after);
        let execution = execute::reconcile(spec, plan, unit, &instructions, signers, &raw)?;
        if !unexpected_writes.is_empty()
            || raw.keys.iter().any(|k| !binding.closure.contains(k))
            || raw.changed().iter().any(|k| !binding.closure.contains(k))
        {
            result.stopped = Some(FailureKind::UnexpectedWrite);
        } else if execution.outcome == Outcome::ReconciliationMismatch
            || (!raw.success && !raw.rolled_back())
            || (raw.success && !expected_funded)
        {
            result.stopped = Some(FailureKind::ReconciliationMismatch);
        }
        let after = snapshot(&session, binding)?;
        let after_id = retain(states, after.clone())?;
        result.steps.push(Step {
            before_state_id: result.final_state_id.clone(),
            after_state_id: after_id.clone(),
            expected_reserve_before_raw: expected_reserve.to_string(),
            expected_funded,
            observed_reserve_before_raw: reserve(&before, &binding.reserve)?.to_string(),
            observed_reserve_after_raw: reserve(&after, &binding.reserve)?.to_string(),
            execution,
            unexpected_writes,
        });
        result.final_state_id = after_id;
        if expected_funded {
            expected_reserve -= output;
        }
        if result.stopped.is_some() {
            break;
        }
    }
    result.run_id = canonical::digest(&(
        "eplyx-migration-order-run/v1",
        &result.case_id,
        &result.steps,
        &result.stopped,
    ))?;
    Ok(result)
}
pub fn validate_handoff(state: &StateEvidence, required: &BTreeSet<String>) -> Result<()> {
    for key in required {
        state.account(key)?;
    }
    Ok(())
}
fn compare(binding: &PairBinding, scenarios: &[Scenario; 4]) -> Comparison {
    let controls = scenarios[..2].iter().all(|s| {
        s.stopped.is_none()
            && s.steps.len() == 1
            && s.steps[0].execution.outcome == Outcome::Migrated
    });
    let complete = scenarios[2..]
        .iter()
        .all(|s| s.stopped.is_none() && s.steps.len() == 2);
    let mut affected_units = Vec::new();
    if complete {
        for unit in &binding.units {
            let a = scenarios[2]
                .steps
                .iter()
                .find(|s| s.execution.unit_id == unit.unit_id)
                .unwrap();
            let b = scenarios[3]
                .steps
                .iter()
                .find(|s| s.execution.unit_id == unit.unit_id)
                .unwrap();
            if a.execution.outcome != b.execution.outcome
                || a.execution.deltas != b.execution.deltas
            {
                affected_units.push(UnitDifference {
                    unit_id: unit.unit_id.clone(),
                    a_then_b: a.execution.clone(),
                    b_then_a: b.execution.clone(),
                });
            }
        }
    }
    let reserve_effect = controls
        && complete
        && !affected_units.is_empty()
        && affected_units.iter().all(|d| {
            let valid = |e: &UnitExecution| {
                e.outcome == Outcome::Migrated
                    || (e.outcome == Outcome::Rejected
                        && e.failure.as_ref().is_some_and(|f| {
                            f.rollback_verified
                                && f.error_name.as_deref() == Some("InsufficientReserve")
                                && f.stage == "candidate"
                        }))
            };
            d.a_then_b.outcome != d.b_then_a.outcome && valid(&d.a_then_b) && valid(&d.b_then_a)
        })
        && scenarios[2..].iter().all(|s| {
            s.steps.iter().all(|step| {
                step.expected_funded == (step.execution.outcome == Outcome::Migrated)
                    && step.expected_reserve_before_raw == step.observed_reserve_before_raw
            })
        });
    Comparison { status: if reserve_effect { ComparisonStatus::SharedReserveChangesSuccessfulUnit } else if controls && complete && affected_units.is_empty() { ComparisonStatus::NoSuccessfulUnitEffect } else { ComparisonStatus::NotEstablished },
        finding: reserve_effect.then(|| FINDING.into()), scenario_ids: std::array::from_fn(|i| scenarios[i].run_id.clone()),
        order_case_ids: [scenarios[2].case_id.clone(), scenarios[3].case_id.clone()], affected_units,
        limitations: vec!["One selected pair under the pinned world, candidate, runtime and assumed signer privileges; no private-key possession or issuer authorization established.".into(),
            "Separate transactions; no atomicity, production fairness, complete-holder coverage, future execution order, universal migration safety, population ordering or governance conclusion.".into(),
            "Bounded account census plus pinned runtime defaults; simulated state is not a complete Solana bank.".into()] }
}

fn outside_writes(
    closure: &BTreeSet<String>,
    before: &BTreeMap<String, AccountSnapshot>,
    after: &BTreeMap<String, AccountSnapshot>,
) -> BTreeSet<String> {
    before
        .keys()
        .chain(after.keys())
        .filter(|key| !closure.contains(*key) && before.get(*key) != after.get(*key))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn census_detects_creation_deletion_and_modified_bytes_outside_closure() {
        let account = AccountSnapshot {
            lamports: 1,
            data: vec![1],
            owner: adapter::SYSTEM_PROGRAM.into(),
            executable: false,
            rent_epoch: 0,
        };
        let before: BTreeMap<String, AccountSnapshot> = [
            ("deleted".into(), account.clone()),
            ("modified".into(), account.clone()),
            ("allowed".into(), account.clone()),
        ]
        .into();
        let mut after = before.clone();
        after.remove("deleted");
        after.get_mut("modified").unwrap().data[0] = 2;
        after.get_mut("allowed").unwrap().lamports = 2;
        after.insert("created".into(), account);
        assert_eq!(
            outside_writes(&["allowed".into()].into(), &before, &after),
            ["created".into(), "deleted".into(), "modified".into()].into()
        );
    }
}
