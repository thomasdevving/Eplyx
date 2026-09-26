//! Bounded Token Migration V1 counterexample search over a verified run.
//!
//! Observed counterexamples come from the verified sequential population rehearsal,
//! which already executed every eligible captured holder: a unit whose VM outcome
//! contradicts the specification, or an eligible holder the proposal cannot migrate.
//! Derived counterexamples come from ordered probes of one parent holder along
//! declared dimensions (source amount, window, reserve, delegate allowance,
//! signer, account and destination state, Token-2022 guards), each executed in a
//! fresh VM through the stress case executor and judged against the planner. A
//! population reserve boundary is established by two sequential rehearsals. Every
//! domain, probe, budget and stopping condition is recorded; no finding within the
//! budget is never an absence claim. Search never replaces a failed case and needs
//! no RPC: it replays byte for byte.
use super::{
    derive::Mutation,
    economics,
    execute::{self, Outcome},
    input::ValidatedInput,
    pipeline,
    planner::{self, ImpactClass, MigrationPlan, PlanInput},
    rehearsal::{self, PopulationRehearsal},
    spec::{DestinationFunding, HolderAuthorization, Reserve},
    stress::{
        self, CaseContext, CaseProvenance, Expected, Finding, StressCase, StressPlan,
        TransactionVariant,
    },
    world::{World, WorldKind},
};
use crate::{replay::hash_bytes as sha256, standard_programs::token as decode};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

pub const SEARCH_VERSION: &str = "eplyx-migration-counterexample-search/v1";
pub const SEARCH_ARTIFACT: &str = "migration-search.json";
pub const MAX_PROBES: usize = 64;
pub const MAX_MINIMIZATION: usize = 32;
pub const NO_FINDING: &str = "No counterexample found within this search domain and budget.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Dimension {
    PopulationRehearsal,
    SourceAmount,
    ActivationBoundary,
    DeadlineBoundary,
    ProposedReserve,
    PopulationReserve,
    DelegateAllowance,
    SignerAuthority,
    SourceAccountState,
    DestinationAccountState,
    Token2022Guard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchFinding {
    UnexpectedFailure,
    UnexpectedSuccess,
    ReconciliationMismatch,
    RollbackViolation,
    /// An eligible holder the proposal cannot migrate (for example, funding runs out).
    EligibleHolderNotMigrated,
}

impl From<Finding> for SearchFinding {
    fn from(finding: Finding) -> Self {
        match finding {
            Finding::UnexpectedFailure => Self::UnexpectedFailure,
            Finding::UnexpectedSuccess => Self::UnexpectedSuccess,
            Finding::ReconciliationMismatch => Self::ReconciliationMismatch,
            Finding::RollbackViolation => Self::RollbackViolation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub outcome: Outcome,
    pub stage: Option<String>,
    pub error_name: Option<String>,
    pub error: Option<String>,
    pub rollback_verified: Option<bool>,
    pub failed_checks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub dimension: Dimension,
    pub kind: String,
    pub value_raw: Option<String>,
    pub expected: Expected,
    pub behaves_as_specified: bool,
    pub signature: Signature,
    pub message_sha256: String,
    pub derived_world_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    pub last_behaving_value_raw: Option<String>,
    pub first_deviating_value_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Counterexample {
    MigrationObserved {
        id: String,
        parent_run: String,
        analysis_input_sha256: String,
        candidate_program_sha256: String,
        world_sha256: String,
        unit_id: String,
        source_account: String,
        observed_balance_raw: String,
        planned_class: ImpactClass,
        finding: SearchFinding,
        signature: Signature,
        message_sha256: String,
        provenance: String,
        limitations: String,
    },
    MigrationDerived {
        id: String,
        parent_run: String,
        analysis_input_sha256: String,
        candidate_program_sha256: String,
        world_sha256: String,
        source_account: String,
        dimension: Dimension,
        case_kind: String,
        derived_value_raw: Option<String>,
        mutations: Vec<Mutation>,
        reserve_override_raw: Option<String>,
        transaction_variant: Option<TransactionVariant>,
        expected: Expected,
        finding: SearchFinding,
        signature: Signature,
        derived_world_sha256: String,
        message_sha256: String,
        minimized: bool,
        boundary: Option<Boundary>,
        provenance: CaseProvenance,
        limitations: String,
    },
}

impl Counterexample {
    pub fn claim(&self) -> &'static str {
        match self {
            Self::MigrationObserved { provenance, .. }
                if provenance == "ObservedPopulationRehearsal" =>
            {
                "Observed captured state"
            }
            Self::MigrationObserved { .. } => "Synthetic fixture state",
            Self::MigrationDerived { .. } => "Derived local state",
        }
    }
    pub fn finding(&self) -> SearchFinding {
        match self {
            Self::MigrationObserved { finding, .. } | Self::MigrationDerived { finding, .. } => {
                *finding
            }
        }
    }
    pub fn source_account(&self) -> &str {
        match self {
            Self::MigrationObserved { source_account, .. }
            | Self::MigrationDerived { source_account, .. } => source_account,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Domain {
    pub dimension: Dimension,
    pub description: String,
    pub low_raw: Option<String>,
    pub high_raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub max_probes: usize,
    pub probes: usize,
    pub max_minimization: usize,
    pub minimization: usize,
    pub population_rehearsals: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResult {
    pub version: String,
    pub parent_run: String,
    pub analysis_input_sha256: String,
    pub candidate_program_sha256: String,
    pub world_sha256: String,
    pub plan_sha256: String,
    pub stress_plan_sha256: String,
    pub parent_source_account: Option<String>,
    pub domains: Vec<Domain>,
    pub explored_dimensions: Vec<Dimension>,
    pub budget: Budget,
    pub stopping_condition: String,
    pub trace: Vec<Probe>,
    pub counterexamples: Vec<Counterexample>,
    pub conclusion: String,
    pub official_transition: String,
    pub funds_moved: bool,
}

fn signature(execution: &execute::UnitExecution) -> Signature {
    Signature {
        outcome: execution.outcome,
        stage: execution.failure.as_ref().map(|f| f.stage.clone()),
        error_name: execution
            .failure
            .as_ref()
            .and_then(|f| f.error_name.clone()),
        error: execution.failure.as_ref().map(|f| f.error.clone()),
        rollback_verified: execution.failure.as_ref().map(|f| f.rollback_verified),
        failed_checks: execution
            .reconciliation
            .iter()
            .filter(|c| !c.holds)
            .map(|c| c.check.clone())
            .collect(),
    }
}

pub fn counterexample_id(counterexample: &Counterexample) -> Result<String> {
    let mut value = serde_json::to_value(counterexample)?;
    value["id"] = serde_json::Value::String(String::new());
    Ok(format!(
        "cx_{}",
        &sha256(crate::canonical::document(&value)?.as_bytes())[..24]
    ))
}

fn with_id(mut counterexample: Counterexample) -> Result<Counterexample> {
    let id = counterexample_id(&counterexample)?;
    match &mut counterexample {
        Counterexample::MigrationObserved { id: slot, .. }
        | Counterexample::MigrationDerived { id: slot, .. } => *slot = id,
    }
    Ok(counterexample)
}

struct Searcher<'a> {
    context: CaseContext<'a>,
    budget: Budget,
    trace: Vec<Probe>,
}

impl Searcher<'_> {
    fn probe(
        &mut self,
        dimension: Dimension,
        case: StressCase,
        value: Option<u64>,
        minimizing: bool,
    ) -> Result<Option<(stress::CaseResult, StressCase)>> {
        if minimizing {
            if self.budget.minimization >= self.budget.max_minimization {
                return Ok(None);
            }
            self.budget.minimization += 1;
        } else {
            if self.budget.probes >= self.budget.max_probes {
                return Ok(None);
            }
            self.budget.probes += 1;
        }
        let result = match stress::execute_case(&self.context, &case) {
            Ok(result) => result,
            Err(_) => return Ok(None),
        };
        let probe = Probe {
            dimension,
            kind: case.kind.clone(),
            value_raw: value.map(|v| v.to_string()),
            expected: result.expected,
            behaves_as_specified: result.behaves_as_specified,
            signature: signature(&result.execution),
            message_sha256: result.execution.message_sha256.clone(),
            derived_world_sha256: result.derived_world_sha256.clone(),
        };
        self.trace.push(probe);
        Ok(Some((result, case)))
    }
}

fn case(
    kind: &str,
    source: &str,
    mutations: Vec<Mutation>,
    reserve: Option<u64>,
    variant: Option<TransactionVariant>,
    world: &World,
) -> StressCase {
    let derived = !mutations.is_empty() || reserve.is_some() || variant.is_some();
    StressCase {
        case_id: format!("search-{kind}"),
        kind: kind.into(),
        source_account: source.into(),
        selection_reason: format!("Counterexample search probe {kind}"),
        mutations,
        reserve_override_raw: reserve.map(|r| r.to_string()),
        transaction_variant: variant,
        provenance: match (world.base_kind(), derived) {
            (WorldKind::ObservedCapture, false) => CaseProvenance::Observed,
            (WorldKind::ObservedCapture, true) => CaseProvenance::DerivedFromObserved,
            (_, false) => CaseProvenance::Synthetic,
            (_, true) => CaseProvenance::DerivedFromSynthetic,
        },
    }
}

pub struct SearchInput<'a> {
    pub package: &'a ValidatedInput,
    pub world: &'a World,
    pub plan: &'a MigrationPlan,
    pub stress_plan: &'a StressPlan,
    pub rehearsal: &'a PopulationRehearsal,
    pub parent_run: &'a str,
}

pub fn search(input: &SearchInput<'_>) -> Result<SearchResult> {
    let package = input.package;
    let spec = package.spec();
    let plan = input.plan;
    let resolved = spec.resolve()?;
    let terms = resolved.terms;
    let tp = &package.analysis_input_sha256;
    let cps = &package.program_sha256;
    let observed_label = match input.world.base_kind() {
        WorldKind::ObservedCapture => "ObservedPopulationRehearsal",
        _ => "SyntheticPopulationRehearsal",
    };
    let mut counterexamples = vec![];
    // Observed: the verified sequential rehearsal of every eligible holder.
    for execution in &input.rehearsal.executions {
        let Some(unit) = plan.unit(&execution.source_account) else {
            continue;
        };
        let finding = match (unit.class, execution.outcome) {
            (_, Outcome::ReconciliationMismatch) => Some(SearchFinding::ReconciliationMismatch),
            (ImpactClass::Migratable, Outcome::Rejected) => Some(SearchFinding::UnexpectedFailure),
            (ImpactClass::InsufficientReserve, Outcome::Rejected) => {
                Some(SearchFinding::EligibleHolderNotMigrated)
            }
            (ImpactClass::InsufficientReserve, Outcome::Migrated) => {
                Some(SearchFinding::UnexpectedSuccess)
            }
            (_, Outcome::Rejected)
                if execution
                    .failure
                    .as_ref()
                    .is_some_and(|f| !f.rollback_verified) =>
            {
                Some(SearchFinding::RollbackViolation)
            }
            _ => None,
        };
        if let Some(finding) = finding {
            counterexamples.push(with_id(Counterexample::MigrationObserved {
                id: String::new(),
                parent_run: input.parent_run.into(),
                analysis_input_sha256: tp.clone(),
                candidate_program_sha256: cps.clone(),
                world_sha256: plan.world_sha256.clone(),
                unit_id: unit.unit_id.clone(),
                source_account: unit.source_account.clone(),
                observed_balance_raw: unit.source_balance_raw.clone(),
                planned_class: unit.class,
                finding,
                signature: signature(execution),
                message_sha256: execution.message_sha256.clone(),
                provenance: observed_label.into(),
                limitations: "Executed in a local VM against the exact captured (or fixture) bank at the rehearsal Clock; no mainnet transaction.".into(),
            })?);
        }
    }

    let context = CaseContext {
        spec,
        change_spec_id: &package.change_spec_id,
        world: input.world,
        program_id: package.program_id(),
        candidate: &package.candidate,
        candidate_sha256: cps,
        clock_policy: package.config.rehearsal_clock,
    };
    let mut s = Searcher {
        context,
        budget: Budget {
            max_probes: MAX_PROBES,
            probes: 0,
            max_minimization: MAX_MINIMIZATION,
            minimization: 0,
            population_rehearsals: 0,
        },
        trace: vec![],
    };
    let mut domains = vec![];
    let mut explored = vec![Dimension::PopulationRehearsal];
    let parent = input
        .stress_plan
        .parent_source_account
        .clone()
        .and_then(|p| plan.unit(&p).cloned());
    let mut derived = vec![];
    if let Some(parent) = &parent {
        let source = parent.source_account.clone();
        let balance: u64 = parent.source_balance_raw.parse().unwrap_or(0);
        let amount = |to: u64| Mutation::SourceAmount {
            account: source.clone(),
            to_raw: to.to_string(),
        };
        // Source amount: exact arithmetic at structured points, minimized upward.
        if let Some(minimum) = economics::minimum_migratable_amount(&terms) {
            explored.push(Dimension::SourceAmount);
            domains.push(Domain {
                dimension: Dimension::SourceAmount,
                description: "Structured amounts from one below the smallest migratable amount through the parent balance, at fee steps and rounding boundaries; minimized by ascending probes from the smallest migratable amount.".into(),
                low_raw: Some(minimum.saturating_sub(1).max(1).to_string()),
                high_raw: Some(balance.max(minimum).to_string()),
            });
            let mut points = vec![
                minimum.saturating_sub(1).max(1),
                minimum,
                minimum + 1,
                balance,
            ];
            if terms.fee_bps > 0 {
                let step = 10_000u64.div_ceil(u64::from(terms.fee_bps));
                for k in 1..=3u64 {
                    points.extend([k * step - 1, k * step, k * step + 1]);
                }
            }
            let window = minimum..minimum.saturating_add(4096);
            points.extend(
                window
                    .clone()
                    .find(|a| economics::quote(*a, &terms).is_ok_and(|q| q.remainder == 0)),
            );
            points.extend(
                window
                    .clone()
                    .find(|a| economics::quote(*a, &terms).is_ok_and(|q| q.remainder != 0)),
            );
            points.retain(|p| *p >= 1);
            points.sort();
            points.dedup();
            let mut first_deviation: Option<(u64, stress::CaseResult, StressCase)> = None;
            for point in points {
                if let Some((result, probe_case)) = s.probe(
                    Dimension::SourceAmount,
                    case(
                        "SourceAmount",
                        &source,
                        vec![amount(point)],
                        None,
                        None,
                        input.world,
                    ),
                    Some(point),
                    false,
                )? {
                    if !result.behaves_as_specified && first_deviation.is_none() {
                        first_deviation = Some((point, result, probe_case));
                    }
                }
            }
            if let Some(found) = first_deviation {
                // Minimize: the smallest deviating amount at or above the minimum,
                // by ascending probes; the adjacent behaving amount is recorded.
                let mut best = found;
                let mut last_behaving = None;
                let low = minimum.saturating_sub(1).max(1);
                let mut candidate = low;
                while candidate < best.0 {
                    let Some((result, probe_case)) = s.probe(
                        Dimension::SourceAmount,
                        case(
                            "SourceAmountMinimize",
                            &source,
                            vec![amount(candidate)],
                            None,
                            None,
                            input.world,
                        ),
                        Some(candidate),
                        true,
                    )?
                    else {
                        break;
                    };
                    if !result.behaves_as_specified {
                        best = (candidate, result, probe_case);
                        break;
                    }
                    last_behaving = Some(candidate);
                    candidate += 1;
                }
                let minimized = best.0 == low || last_behaving.is_some_and(|l| l + 1 == best.0);
                derived.push((
                    Dimension::SourceAmount,
                    best.2,
                    best.1,
                    Some(best.0),
                    minimized,
                    Some(Boundary {
                        last_behaving_value_raw: last_behaving.map(|l| l.to_string()),
                        first_deviating_value_raw: best.0.to_string(),
                    }),
                ));
            }
        }
        // Window: probe around both boundaries, then walk forward for deviations.
        for (dimension, boundary) in [
            (Dimension::ActivationBoundary, resolved.activation),
            (Dimension::DeadlineBoundary, resolved.deadline),
        ] {
            let Some((is_slot, value)) = boundary else {
                continue;
            };
            explored.push(dimension);
            domains.push(Domain {
                dimension,
                description: format!(
                    "Derived Clock {} from boundary-2 to boundary+2",
                    if is_slot { "slot" } else { "unix timestamp" }
                ),
                low_raw: Some(value.saturating_sub(2).to_string()),
                high_raw: Some(value.saturating_add(2).to_string()),
            });
            for offset in [-2i128, -1, 0, 1, 2] {
                let target = i128::from(value) + offset;
                if target < 0 {
                    continue;
                }
                let to = target.to_string();
                let mutation = if is_slot {
                    Mutation::ClockSlot { to }
                } else {
                    Mutation::ClockUnixTimestamp { to }
                };
                if let Some((result, probe_case)) = s.probe(
                    dimension,
                    case(
                        &format!("{dimension:?}{offset:+}"),
                        &source,
                        vec![mutation],
                        None,
                        None,
                        input.world,
                    ),
                    Some(target as u64),
                    false,
                )? {
                    if !result.behaves_as_specified {
                        derived.push((
                            dimension,
                            probe_case,
                            result,
                            Some(target as u64),
                            true,
                            Some(Boundary {
                                last_behaving_value_raw: None,
                                first_deviating_value_raw: target.to_string(),
                            }),
                        ));
                        break;
                    }
                }
            }
        }
        // Isolated reserve boundary for the parent unit.
        if matches!(
            spec.destination_funding,
            DestinationFunding::ReserveTransfer {
                reserve: Reserve::Proposed { .. }
            }
        ) {
            if let Some(required) = parent
                .quote
                .as_ref()
                .and_then(|q| q.output_raw.parse::<u64>().ok())
            {
                explored.push(Dimension::ProposedReserve);
                domains.push(Domain {
                    dimension: Dimension::ProposedReserve,
                    description: "Derived proposed reserve at the parent's exact output and one below".into(),
                    low_raw: Some(required.saturating_sub(1).to_string()),
                    high_raw: Some(required.to_string()),
                });
                for value in [required.saturating_sub(1), required] {
                    if let Some((result, probe_case)) = s.probe(
                        Dimension::ProposedReserve,
                        case(
                            "ProposedReserve",
                            &source,
                            vec![],
                            Some(value),
                            None,
                            input.world,
                        ),
                        Some(value),
                        false,
                    )? {
                        if !result.behaves_as_specified {
                            derived.push((
                                Dimension::ProposedReserve,
                                probe_case,
                                result,
                                Some(value),
                                true,
                                None,
                            ));
                        }
                    }
                }
            }
        }
        // Authority, state and guard variants.
        let delegate =
            super::fixture::label_address("eplyx-migration-search", "search-delegate").to_string();
        let impostor =
            super::fixture::label_address("eplyx-migration-search", "search-impostor").to_string();
        let sign_as = |allowance: u64| TransactionVariant::SignAs {
            authority: super::authority::HolderAuthority::Delegate {
                delegate: delegate.clone(),
                delegated_amount_raw: allowance.to_string(),
            },
        };
        let approve = |allowance: u64| Mutation::Approve {
            account: source.clone(),
            delegate: delegate.clone(),
            amount_raw: allowance.to_string(),
        };
        let mut variants: Vec<(Dimension, StressCase, Option<u64>)> = vec![];
        if balance > 1 {
            variants.push((
                Dimension::DelegateAllowance,
                case(
                    "DelegateAllowanceShort",
                    &source,
                    vec![approve(balance - 1)],
                    None,
                    Some(sign_as(balance - 1)),
                    input.world,
                ),
                Some(balance - 1),
            ));
        }
        variants.push((
            Dimension::DelegateAllowance,
            case(
                "DelegateAllowanceExact",
                &source,
                vec![approve(balance)],
                None,
                Some(sign_as(balance)),
                input.world,
            ),
            Some(balance),
        ));
        variants.push((
            Dimension::SignerAuthority,
            case(
                "WrongSigner",
                &source,
                vec![],
                None,
                Some(TransactionVariant::WrongSigner { signer: impostor }),
                input.world,
            ),
            None,
        ));
        if input
            .world
            .mint(&spec.source.mint)?
            .freeze_authority
            .is_some()
        {
            variants.push((
                Dimension::SourceAccountState,
                case(
                    "FrozenSource",
                    &source,
                    vec![Mutation::Freeze {
                        account: source.clone(),
                    }],
                    None,
                    None,
                    input.world,
                ),
                None,
            ));
        }
        if spec.source.token_program == decode::TOKEN_2022_PROGRAM {
            variants.push((
                Dimension::Token2022Guard,
                case(
                    "CpiGuardSource",
                    &source,
                    vec![Mutation::EnableCpiGuard {
                        account: source.clone(),
                    }],
                    None,
                    None,
                    input.world,
                ),
                None,
            ));
        }
        let mut destination_first = vec![];
        if parent.destination.action == "CreateAssociated" {
            destination_first.push(Mutation::CreateDestination {
                owner: parent.owner.clone(),
            });
        }
        if input
            .world
            .mint(&spec.destination.mint)?
            .freeze_authority
            .is_some()
        {
            variants.push((
                Dimension::DestinationAccountState,
                case(
                    "FrozenDestination",
                    &source,
                    [
                        destination_first.clone(),
                        vec![Mutation::Freeze {
                            account: parent.destination.address.clone(),
                        }],
                    ]
                    .concat(),
                    None,
                    None,
                    input.world,
                ),
                None,
            ));
        }
        if spec.destination.token_program == decode::TOKEN_2022_PROGRAM {
            variants.push((
                Dimension::Token2022Guard,
                case(
                    "MemoRequiredDestination",
                    &source,
                    [
                        destination_first,
                        vec![Mutation::RequireMemo {
                            account: parent.destination.address.clone(),
                        }],
                    ]
                    .concat(),
                    None,
                    None,
                    input.world,
                ),
                None,
            ));
        }
        for (dimension, probe_case, value) in variants {
            if !explored.contains(&dimension) {
                explored.push(dimension);
                domains.push(Domain {
                    dimension,
                    description: format!("Reachable {dimension:?} variants of the parent holder"),
                    low_raw: None,
                    high_raw: None,
                });
            }
            if !spec.allows(HolderAuthorization::Delegate)
                && dimension == Dimension::DelegateAllowance
                && probe_case.kind == "DelegateAllowanceShort"
            {
                continue;
            }
            if let Some((result, probe_case)) = s.probe(dimension, probe_case, value, false)? {
                if !result.behaves_as_specified {
                    derived.push((dimension, probe_case, result, value, false, None));
                }
            }
        }
    }
    // Population reserve boundary: the exact reserve that migrates every eligible holder.
    let required_total: Option<u64> = input
        .rehearsal
        .reconciliation
        .required_reserve_raw
        .as_deref()
        .and_then(|r| r.parse().ok());
    let available: Option<u64> = plan
        .funding
        .available_raw
        .as_deref()
        .and_then(|a| a.parse().ok());
    if let (Some(required), Some(available)) = (required_total, available) {
        if matches!(
            spec.destination_funding,
            DestinationFunding::ReserveTransfer {
                reserve: Reserve::Proposed { .. }
            }
        ) && required > available
            && required > 0
        {
            explored.push(Dimension::PopulationReserve);
            domains.push(Domain {
                dimension: Dimension::PopulationReserve,
                description: "Sequential population rehearsals at the exact total requirement and one unit below".into(),
                low_raw: Some((required - 1).to_string()),
                high_raw: Some(required.to_string()),
            });
            let programs =
                execute::programs(input.world, spec, package.program_id(), &package.candidate)?;
            for value in [required - 1, required] {
                let derived_plan = planner::plan(&PlanInput {
                    spec,
                    change_spec_id: &package.change_spec_id,
                    world: input.world,
                    program_id: package.program_id(),
                    candidate_program_sha256: cps,
                    clock_policy: package.config.rehearsal_clock,
                    reserve_override: Some(value),
                    focus: None,
                })?;
                s.budget.population_rehearsals += 1;
                let run = rehearsal::rehearse(
                    spec,
                    input.world,
                    &derived_plan,
                    &programs,
                    package.config.max_rehearsal_units,
                )?;
                let migrated_all = run
                    .executions
                    .iter()
                    .all(|e| e.outcome == Outcome::Migrated);
                s.trace.push(Probe {
                    dimension: Dimension::PopulationReserve,
                    kind: "PopulationReserve".into(),
                    value_raw: Some(value.to_string()),
                    expected: if value >= required {
                        Expected::Migrate
                    } else {
                        Expected::Reject
                    },
                    behaves_as_specified: migrated_all == (value >= required),
                    signature: Signature {
                        outcome: if migrated_all {
                            Outcome::Migrated
                        } else {
                            Outcome::Rejected
                        },
                        stage: None,
                        error_name: run
                            .executions
                            .iter()
                            .find_map(|e| e.failure.as_ref().and_then(|f| f.error_name.clone())),
                        error: None,
                        rollback_verified: None,
                        failed_checks: vec![],
                    },
                    message_sha256: String::new(),
                    derived_world_sha256: plan.world_sha256.clone(),
                });
            }
        }
    }
    for (dimension, probe_case, result, value, minimized, boundary) in derived {
        let finding =
            SearchFinding::from(result.finding.context("a deviating probe has a finding")?);
        counterexamples.push(with_id(Counterexample::MigrationDerived {
            id: String::new(),
            parent_run: input.parent_run.into(),
            analysis_input_sha256: tp.clone(),
            candidate_program_sha256: cps.clone(),
            world_sha256: plan.world_sha256.clone(),
            source_account: result.source_account.clone(),
            dimension,
            case_kind: result.kind.clone(),
            derived_value_raw: value.map(|v| v.to_string()),
            mutations: probe_case.mutations.clone(),
            reserve_override_raw: probe_case.reserve_override_raw.clone(),
            transaction_variant: probe_case.transaction_variant.clone(),
            expected: result.expected,
            finding,
            signature: signature(&result.execution),
            derived_world_sha256: result.derived_world_sha256.clone(),
            message_sha256: result.execution.message_sha256.clone(),
            minimized,
            boundary,
            provenance: result.provenance,
            limitations: "A derived local state built by typed mutation of captured or fixture state; it is not an observed mainnet failure.".into(),
        })?);
    }
    let stopping = if s.budget.probes >= s.budget.max_probes {
        "Probe budget exhausted"
    } else {
        "Every declared domain was explored within the budget"
    };
    let conclusion = if counterexamples.is_empty() {
        NO_FINDING.into()
    } else {
        format!(
            "{} counterexamples: {} from the population rehearsal, {} derived.",
            counterexamples.len(),
            counterexamples
                .iter()
                .filter(|c| matches!(c, Counterexample::MigrationObserved { .. }))
                .count(),
            counterexamples
                .iter()
                .filter(|c| matches!(c, Counterexample::MigrationDerived { .. }))
                .count()
        )
    };
    Ok(SearchResult {
        version: SEARCH_VERSION.into(),
        parent_run: input.parent_run.into(),
        analysis_input_sha256: tp.clone(),
        candidate_program_sha256: cps.clone(),
        world_sha256: plan.world_sha256.clone(),
        plan_sha256: plan.sha256()?,
        stress_plan_sha256: input.stress_plan.sha256()?,
        parent_source_account: parent.map(|p| p.source_account),
        domains,
        explored_dimensions: explored,
        budget: s.budget,
        stopping_condition: stopping.into(),
        trace: s.trace,
        counterexamples,
        conclusion,
        official_transition: "NotTested".into(),
        funds_moved: false,
    })
}

fn load_run(
    package: &ValidatedInput,
    result: &Path,
) -> Result<(
    World,
    MigrationPlan,
    StressPlan,
    PopulationRehearsal,
    String,
)> {
    pipeline::replay(&package.root, result)?;
    let b = pipeline::bindings(result)?;
    let world = pipeline::world_for(package, result, &b)?;
    let plan: MigrationPlan = serde_json::from_slice(&fs::read(result.join(pipeline::PLAN))?)?;
    let stress_plan: StressPlan =
        serde_json::from_slice(&fs::read(result.join(pipeline::STRESS_PLAN))?)?;
    let rehearsal: PopulationRehearsal =
        serde_json::from_slice(&fs::read(result.join(pipeline::REHEARSAL))?)?;
    let run = b.run_id;
    Ok((world, plan, stress_plan, rehearsal, run))
}

/// Verify the parent run offline, search it, and write the search artifact.
pub fn run(package_dir: &Path, result: &Path, search_dir: &Path) -> Result<SearchResult> {
    let package = super::input::load(package_dir)?;
    let (world, plan, stress_plan, rehearsal, parent) = load_run(&package, result)?;
    let outcome = search(&SearchInput {
        package: &package,
        world: &world,
        plan: &plan,
        stress_plan: &stress_plan,
        rehearsal: &rehearsal,
        parent_run: &parent,
    })?;
    fs::create_dir(search_dir).context("search directory must not already exist")?;
    let bytes = crate::canonical::document(&outcome)?;
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(search_dir.join(SEARCH_ARTIFACT))?;
    file.write_all(bytes.as_bytes())?;
    file.sync_all()?;
    Ok(outcome)
}

/// Offline, byte-exact re-execution of the whole search.
pub fn replay(package_dir: &Path, result: &Path, search_dir: &Path) -> Result<SearchResult> {
    let package = super::input::load(package_dir)?;
    let (world, plan, stress_plan, rehearsal, parent) = load_run(&package, result)?;
    let outcome = search(&SearchInput {
        package: &package,
        world: &world,
        plan: &plan,
        stress_plan: &stress_plan,
        rehearsal: &rehearsal,
        parent_run: &parent,
    })?;
    ensure!(
        crate::canonical::document(&outcome)?.as_bytes()
            == fs::read(search_dir.join(SEARCH_ARTIFACT))?,
        "saved migration search differs from offline replay"
    );
    Ok(outcome)
}
