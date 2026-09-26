//! Migration-aware stress coverage.
//!
//! A deterministic case matrix is selected from the plan and frozen before any case
//! executes; a case is never replaced after a failure. Cases are real holders of the
//! world (observed or synthetic) and typed derivations of one parent holder at the
//! declared boundaries. The specification, through the planner, is the oracle: a case
//! the planner classifies as migratable must migrate and reconcile exactly; any other
//! case must be rejected with every referenced account rolled back. The candidate's
//! own behavior is never used to decide what should have happened.
use super::{
    adapter,
    authority::{AuthorityPath, HolderAuthority},
    derive::{self, Mutation, Reachability},
    economics,
    execute::{self, Bank, Outcome, Session, UnitExecution},
    planner::{self, ImpactClass, MigrationPlan, MigrationUnit, PlanInput, RehearsalClockPolicy},
    spec::{DestinationFunding, HolderAuthorization, Reserve, TokenMigrationV1},
    world::{World, WorldKind},
};
use crate::standard_programs::token as decode;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const STRESS_VERSION: &str = "eplyx-migration-stress/v1";
pub const MAX_OBSERVED_CASES: usize = 8;
const ROUNDING_PROBE_WINDOW: u64 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CaseProvenance {
    /// An exact holder of an observed capture, unmodified.
    Observed,
    /// An exact holder of a synthetic fixture, unmodified.
    Synthetic,
    /// A typed derivation of an observed holder or bank.
    DerivedFromObserved,
    /// A typed derivation of a synthetic holder or bank.
    DerivedFromSynthetic,
}

/// A transaction-level variant: the bank is unchanged, the signer set differs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TransactionVariant {
    /// An unrelated wallet signs as the holder authority.
    WrongSigner { signer: String },
    /// Sign with this alternative authority instead of the planned one.
    SignAs { authority: HolderAuthority },
    /// An SPL multisig threshold minus one signers.
    MultisigUnderThreshold,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StressCase {
    pub case_id: String,
    pub kind: String,
    pub source_account: String,
    pub selection_reason: String,
    pub mutations: Vec<Mutation>,
    pub reserve_override_raw: Option<String>,
    pub transaction_variant: Option<TransactionVariant>,
    pub provenance: CaseProvenance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotReachable {
    pub kind: String,
    pub reason: String,
}

/// Frozen before execution; its digest binds every case result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StressPlan {
    pub schema_version: u32,
    pub version: String,
    pub plan_sha256: String,
    pub world_sha256: String,
    pub parent_source_account: Option<String>,
    pub cases: Vec<StressCase>,
    pub not_reachable: Vec<NotReachable>,
}

impl StressPlan {
    pub fn sha256(&self) -> Result<String> {
        crate::canonical::digest(self)
    }
}

fn provenance(world: &World, derived: bool) -> CaseProvenance {
    match (world.base_kind(), derived) {
        (WorldKind::ObservedCapture, false) => CaseProvenance::Observed,
        (WorldKind::ObservedCapture, true) => CaseProvenance::DerivedFromObserved,
        (_, false) => CaseProvenance::Synthetic,
        (_, true) => CaseProvenance::DerivedFromSynthetic,
    }
}

fn balance(unit: &MigrationUnit) -> u64 {
    unit.source_balance_raw.parse().unwrap_or(0)
}

/// Deterministic case selection over a plan. Pure: no execution happens here.
pub fn select(spec: &TokenMigrationV1, world: &World, plan: &MigrationPlan) -> Result<StressPlan> {
    let resolved = spec.resolve()?;
    let terms = resolved.terms;
    let mut cases: Vec<StressCase> = vec![];
    let mut not_reachable = vec![];
    let push = |cases: &mut Vec<StressCase>,
                kind: &str,
                source: &str,
                reason: String,
                mutations: Vec<Mutation>,
                reserve: Option<u64>,
                variant: Option<TransactionVariant>| {
        let derived = !mutations.is_empty() || reserve.is_some() || variant.is_some();
        cases.push(StressCase {
            case_id: format!("stress-{:02}", cases.len() + 1),
            kind: kind.into(),
            source_account: source.into(),
            selection_reason: reason,
            mutations,
            reserve_override_raw: reserve.map(|r| r.to_string()),
            transaction_variant: variant,
            provenance: provenance(world, derived),
        });
    };

    // Exact holders: smallest, median and largest migratable balances, then one per
    // authority kind and destination action not yet covered.
    let mut migratable: Vec<&MigrationUnit> = plan
        .units
        .iter()
        .filter(|u| u.class == ImpactClass::Migratable)
        .collect();
    migratable.sort_by(|a, b| {
        balance(a)
            .cmp(&balance(b))
            .then(a.source_account.cmp(&b.source_account))
    });
    let mut chosen: BTreeSet<String> = BTreeSet::new();
    let mut observed = vec![];
    if let (Some(first), Some(last)) = (migratable.first(), migratable.last()) {
        observed.push((*first, "SmallestMigratableHolder"));
        observed.push((migratable[migratable.len() / 2], "MedianMigratableHolder"));
        observed.push((*last, "LargestMigratableHolder"));
    }
    let mut kinds = BTreeSet::new();
    let mut actions = BTreeSet::new();
    for unit in migratable.iter().rev() {
        let kind = format!("{:?}", unit.authority.required());
        if kinds.insert(kind.clone()) {
            observed.push((unit, "NewAuthorityKind"));
        }
        if actions.insert(unit.destination.action.clone()) {
            observed.push((unit, "NewDestinationAction"));
        }
    }
    for (unit, why) in observed {
        if chosen.len() >= MAX_OBSERVED_CASES || !chosen.insert(unit.source_account.clone()) {
            continue;
        }
        push(
            &mut cases,
            why,
            &unit.source_account,
            format!(
                "{why}: exact holder with balance {} raw, {:?}, destination {}",
                unit.source_balance_raw,
                unit.authority.required(),
                unit.destination.action
            ),
            vec![],
            None,
            None,
        );
    }
    // Derived boundaries around one parent: the largest wallet-owned migratable holder.
    let parent = migratable
        .iter()
        .rev()
        .find(|u| {
            matches!(
                u.authority,
                AuthorityPath::Available {
                    authority: HolderAuthority::OwnerWallet { .. },
                    ..
                }
            )
        })
        .or_else(|| migratable.last())
        .copied();
    let Some(parent) = parent else {
        not_reachable.push(NotReachable {
            kind: "DerivedBoundaries".into(),
            reason: "No migratable holder exists to derive boundary cases from.".into(),
        });
        return Ok(StressPlan {
            schema_version: 1,
            version: STRESS_VERSION.into(),
            plan_sha256: plan.sha256()?,
            world_sha256: plan.world_sha256.clone(),
            parent_source_account: None,
            cases,
            not_reachable,
        });
    };
    let source = parent.source_account.as_str();
    let parent_balance = balance(parent);
    let amount = |to: u64| Mutation::SourceAmount {
        account: source.into(),
        to_raw: to.to_string(),
    };
    if let Some(minimum) = economics::minimum_migratable_amount(&terms) {
        push(
            &mut cases,
            "MinimumMigratableAmount",
            source,
            format!("Smallest amount with output >= minimum output: {minimum} raw"),
            vec![amount(minimum)],
            None,
            None,
        );
        if minimum > 1 {
            push(&mut cases, "BelowMinimumAmount", source, format!("One unit below the smallest migratable amount: {} raw; the specification requires rejection", minimum - 1), vec![amount(minimum - 1)], None, None);
        }
        // Rounding: the first amounts with zero and with nonzero remainder.
        let exact = (minimum..minimum.saturating_add(ROUNDING_PROBE_WINDOW))
            .find(|a| economics::quote(*a, &terms).is_ok_and(|q| q.remainder == 0));
        let dusty = (minimum..minimum.saturating_add(ROUNDING_PROBE_WINDOW))
            .find(|a| economics::quote(*a, &terms).is_ok_and(|q| q.remainder != 0));
        match (exact, dusty) {
            (Some(e), Some(d)) => {
                push(
                    &mut cases,
                    "RoundingExact",
                    source,
                    format!("Amount {e} raw divides exactly (no rounding dust)"),
                    vec![amount(e)],
                    None,
                    None,
                );
                push(
                    &mut cases,
                    "RoundingDust",
                    source,
                    format!(
                        "Amount {d} raw leaves rounding dust under {:?}",
                        terms.rounding
                    ),
                    vec![amount(d)],
                    None,
                    None,
                );
            }
            _ => not_reachable.push(NotReachable {
                kind: "RoundingBoundary".into(),
                reason: format!(
                    "No exact/dusty pair within {ROUNDING_PROBE_WINDOW} raw of the minimum amount."
                ),
            }),
        }
        if terms.fee_bps > 0 {
            let step = economics::first_where(1, u64::MAX, |a| {
                u128::from(a) * u128::from(terms.fee_bps) >= 10_000
            });
            if let Some(step) = step {
                for (kind, value) in [("FeeStep", step), ("BelowFeeStep", step.saturating_sub(1))] {
                    if value >= minimum {
                        push(
                            &mut cases,
                            kind,
                            source,
                            format!("Amount {value} raw at the first migration-fee unit boundary"),
                            vec![amount(value)],
                            None,
                            None,
                        );
                    }
                }
            }
        }
    }
    if let Some(maximum) = economics::maximum_migratable_amount(&terms) {
        let supply: u64 = world
            .mint(&spec.source.mint)
            .map(|m| m.raw_supply.parse().unwrap_or(0))
            .unwrap_or(0);
        let headroom = u64::MAX - (supply - parent_balance.min(supply));
        let value = maximum.min(headroom);
        if matches!(
            spec.destination_funding,
            DestinationFunding::ReserveTransfer {
                reserve: Reserve::Proposed { .. }
            }
        ) {
            if let Some(output) = economics::raw_output(value, &terms) {
                push(&mut cases, "LargestRepresentableAmount", source, format!("Largest amount {value} raw whose output fits in u64 and whose mint supply stays representable, with a derived reserve of exactly the output"), vec![amount(value)], Some(output), None);
            }
        }
    }
    // Reserve boundaries for a proposed reserve.
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
            push(
                &mut cases,
                "ReserveExact",
                source,
                format!("Derived proposed reserve of exactly the required {required} raw"),
                vec![],
                Some(required),
                None,
            );
            push(
                &mut cases,
                "ReserveOneBelow",
                source,
                format!(
                    "Derived proposed reserve of {} raw, one unit below the requirement",
                    required - 1
                ),
                vec![],
                Some(required - 1),
                None,
            );
            push(
                &mut cases,
                "ReserveOneAbove",
                source,
                format!(
                    "Derived proposed reserve of {} raw, one unit above the requirement",
                    required.saturating_add(1)
                ),
                vec![],
                Some(required.saturating_add(1)),
                None,
            );
        }
    }
    // Window boundaries on the Clock basis the specification declares.
    for (kind, boundary, offset) in [
        ("BeforeActivation", resolved.activation, -1i128),
        ("AtActivation", resolved.activation, 0),
        ("BeforeDeadline", resolved.deadline, -1),
        ("AtDeadline", resolved.deadline, 0),
    ] {
        let Some((is_slot, value)) = boundary else {
            continue;
        };
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
        push(
            &mut cases,
            kind,
            source,
            format!("Derived Clock at {kind} ({target}); all other state as captured"),
            vec![mutation],
            None,
            None,
        );
    }
    // Destination state.
    if parent.destination.action == "CreateAssociated" {
        push(
            &mut cases,
            "ExistingDestination",
            source,
            "The holder's destination ATA created beforehand through the real ATA program".into(),
            vec![Mutation::CreateDestination {
                owner: parent.owner.clone(),
            }],
            None,
            None,
        );
    }
    let destination_program = spec.destination.token_program.as_str();
    let mut destination_first = vec![];
    if parent.destination.action == "CreateAssociated" {
        destination_first.push(Mutation::CreateDestination {
            owner: parent.owner.clone(),
        });
    }
    let freeze_destination = [
        destination_first.clone(),
        vec![Mutation::Freeze {
            account: parent.destination.address.clone(),
        }],
    ]
    .concat();
    let destination_mint = world.mint(&spec.destination.mint)?;
    if destination_mint.freeze_authority.is_some() {
        push(
            &mut cases,
            "FrozenDestination",
            source,
            "The destination frozen by the real destination freeze authority".into(),
            freeze_destination,
            None,
            None,
        );
    } else {
        not_reachable.push(NotReachable {
            kind: "FrozenDestination".into(),
            reason:
                "The destination mint has no freeze authority; a frozen destination is unreachable."
                    .into(),
        });
    }
    if destination_program == decode::TOKEN_2022_PROGRAM {
        push(&mut cases, "MemoRequiredDestination", source, "The destination requires incoming memos (real Reallocate + EnableRequiredTransferMemos)".into(), [destination_first.clone(), vec![Mutation::RequireMemo { account: parent.destination.address.clone() }]].concat(), None, None);
    } else {
        not_reachable.push(NotReachable {
            kind: "MemoRequiredDestination".into(),
            reason: "Required memos exist only on Token-2022 destinations.".into(),
        });
    }
    // Source state.
    let freeze = Mutation::Freeze {
        account: source.into(),
    };
    match derive::reachability(world, &freeze)? {
        Reachability::Reachable => push(
            &mut cases,
            "FrozenSource",
            source,
            "The source frozen by the real source freeze authority".into(),
            vec![freeze],
            None,
            None,
        ),
        Reachability::NotReachable { reason } => not_reachable.push(NotReachable {
            kind: "FrozenSource".into(),
            reason,
        }),
    }
    if spec.source.token_program == decode::TOKEN_2022_PROGRAM {
        push(
            &mut cases,
            "CpiGuardSource",
            source,
            "CPI Guard enabled on the source by its owner (real Reallocate + EnableCpiGuard)"
                .into(),
            vec![Mutation::EnableCpiGuard {
                account: source.into(),
            }],
            None,
            None,
        );
    } else {
        not_reachable.push(NotReachable {
            kind: "CpiGuardSource".into(),
            reason: "CPI Guard exists only on Token-2022 source accounts.".into(),
        });
    }
    // Authority variants.
    let delegate =
        super::fixture::label_address("eplyx-migration-stress", "stress-delegate").to_string();
    let as_delegate = |amount: u64| TransactionVariant::SignAs {
        authority: HolderAuthority::Delegate {
            delegate: delegate.clone(),
            delegated_amount_raw: amount.to_string(),
        },
    };
    let approve = |amount: u64| Mutation::Approve {
        account: source.into(),
        delegate: delegate.clone(),
        amount_raw: amount.to_string(),
    };
    if spec.allows(HolderAuthorization::Delegate) {
        push(
            &mut cases,
            "DelegateExactAllowance",
            source,
            "An approved delegate with exactly the full balance signs".into(),
            vec![approve(parent_balance)],
            None,
            Some(as_delegate(parent_balance)),
        );
        if parent_balance > 1 {
            push(
                &mut cases,
                "DelegateInsufficientAllowance",
                source,
                "An approved delegate with one unit less than the full balance signs".into(),
                vec![approve(parent_balance - 1)],
                None,
                Some(as_delegate(parent_balance - 1)),
            );
        }
    } else {
        push(
            &mut cases,
            "DelegateNotAuthorized",
            source,
            "An approved delegate signs although the specification does not allow delegates".into(),
            vec![approve(parent_balance)],
            None,
            Some(as_delegate(parent_balance)),
        );
    }
    let impostor =
        super::fixture::label_address("eplyx-migration-stress", "stress-impostor").to_string();
    push(
        &mut cases,
        "WrongSigner",
        source,
        "An unrelated wallet signs as the holder authority".into(),
        vec![],
        None,
        Some(TransactionVariant::WrongSigner { signer: impostor }),
    );
    if let Some(multisig) = migratable.iter().find(|u| {
        matches!(
            u.authority,
            AuthorityPath::Available {
                authority: HolderAuthority::OwnerMultisig { threshold, .. },
                ..
            } if threshold > 1
        )
    }) {
        push(
            &mut cases,
            "MultisigUnderThreshold",
            &multisig.source_account,
            "An SPL multisig owner with one signer fewer than its threshold".into(),
            vec![],
            None,
            Some(TransactionVariant::MultisigUnderThreshold),
        );
    } else {
        not_reachable.push(NotReachable {
            kind: "MultisigUnderThreshold".into(),
            reason: "No migratable holder is owned by an SPL multisig with a threshold above one."
                .into(),
        });
    }
    Ok(StressPlan {
        schema_version: 1,
        version: STRESS_VERSION.into(),
        plan_sha256: plan.sha256()?,
        world_sha256: plan.world_sha256.clone(),
        parent_source_account: Some(parent.source_account.clone()),
        cases,
        not_reachable,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expected {
    Migrate,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Finding {
    /// The specification requires a migration; the candidate rejected it.
    UnexpectedFailure,
    /// The specification requires a rejection; the candidate migrated. A safety violation.
    UnexpectedSuccess,
    /// The candidate migrated but the state differs from the specification.
    ReconciliationMismatch,
    /// A rejection that left referenced state changed.
    RollbackViolation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseResult {
    pub case_id: String,
    pub kind: String,
    pub provenance: CaseProvenance,
    pub source_account: String,
    pub expected: Expected,
    pub planned_class: ImpactClass,
    pub planned_reasons: Vec<String>,
    pub derived_world_sha256: String,
    pub derived_plan_sha256: String,
    pub execution: UnitExecution,
    pub behaves_as_specified: bool,
    pub finding: Option<Finding>,
}

pub struct CaseContext<'a> {
    pub spec: &'a TokenMigrationV1,
    pub change_spec_id: &'a str,
    pub world: &'a World,
    pub program_id: &'a str,
    pub candidate: &'a crate::change::ResolvedCandidate,
    pub candidate_sha256: &'a str,
    pub clock_policy: RehearsalClockPolicy,
}

/// Execute one frozen case in its own fresh VM.
pub fn execute_case(context: &CaseContext<'_>, case: &StressCase) -> Result<CaseResult> {
    let spec = context.spec;
    let world = if case.mutations.is_empty() {
        context.world.clone()
    } else {
        derive::derive(context.world, spec, &case.mutations)?
    };
    let clock_mutated = case.mutations.iter().any(|m| {
        matches!(
            m,
            Mutation::ClockSlot { .. } | Mutation::ClockUnixTimestamp { .. }
        )
    });
    let reserve_override = case
        .reserve_override_raw
        .as_deref()
        .map(str::parse::<u64>)
        .transpose()?;
    let plan = planner::plan(&PlanInput {
        spec,
        change_spec_id: context.change_spec_id,
        world: &world,
        program_id: context.program_id,
        candidate_program_sha256: context.candidate_sha256,
        clock_policy: if clock_mutated {
            RehearsalClockPolicy::Captured
        } else {
            context.clock_policy
        },
        reserve_override,
        focus: Some(&case.source_account),
    })?;
    let mut unit = plan
        .unit(&case.source_account)
        .with_context(|| format!("case {} source is not a planned unit", case.case_id))?
        .clone();
    // A case runs alone in a fresh bank, so reserve sufficiency is judged for this
    // unit only; cumulative shortfalls belong to the sequential population rehearsal.
    if matches!(
        unit.class,
        ImpactClass::Migratable | ImpactClass::InsufficientReserve
    ) {
        if let (Some(available), Some(quote)) = (
            plan.funding
                .available_raw
                .as_deref()
                .map(str::parse::<u64>)
                .transpose()?,
            unit.quote.as_ref(),
        ) {
            let own: u64 = quote.output_raw.parse()?;
            unit.reasons.retain(|r| r.code != "INSUFFICIENT_RESERVE");
            unit.class = if own > available {
                unit.reasons.push(planner::Reason {
                    code: "INSUFFICIENT_RESERVE".into(),
                    detail: format!(
                        "This unit alone needs {own} raw; {available} raw are available."
                    ),
                });
                ImpactClass::InsufficientReserve
            } else {
                ImpactClass::Migratable
            };
        }
    }
    let planned_class = unit.class;
    let mut expected = if planned_class == ImpactClass::Migratable {
        Expected::Migrate
    } else {
        Expected::Reject
    };
    match &case.transaction_variant {
        None => {}
        Some(TransactionVariant::WrongSigner { signer }) => {
            unit.authority = AuthorityPath::Available {
                owner_class: unit.authority.owner_class(),
                authority: HolderAuthority::OwnerWallet {
                    owner: signer.clone(),
                },
                alternatives: vec![],
            };
            expected = Expected::Reject;
        }
        Some(TransactionVariant::SignAs { authority }) => {
            // The planner decides whether this alternative path is valid.
            let valid = match &unit.authority {
                AuthorityPath::Available {
                    authority: chosen,
                    alternatives,
                    ..
                } => chosen == authority || alternatives.contains(authority),
                AuthorityPath::Unavailable { .. } => false,
            };
            expected = if valid && planned_class == ImpactClass::Migratable {
                Expected::Migrate
            } else {
                Expected::Reject
            };
            unit.authority = AuthorityPath::Available {
                owner_class: unit.authority.owner_class(),
                authority: authority.clone(),
                alternatives: vec![],
            };
        }
        Some(TransactionVariant::MultisigUnderThreshold) => {
            if let AuthorityPath::Available {
                authority: HolderAuthority::OwnerMultisig { threshold, .. },
                ..
            } = &mut unit.authority
            {
                *threshold -= 1;
            }
            expected = Expected::Reject;
        }
    }
    if !matches!(unit.authority, AuthorityPath::Available { .. }) {
        // No signer exists to even attempt this: record it as a rejection by
        // construction with no VM claim.
        anyhow::bail!(
            "case {} has no authority path to execute; it cannot be rehearsed",
            case.case_id
        );
    }
    let resolved = spec.resolve()?;
    let config = adapter::config_bytes(spec, &resolved, &plan.overlay, context.change_spec_id)?;
    let relayer = execute::relayer().to_string();
    let (instructions, _) = execute::unit_instructions(spec, &plan, &unit, &relayer)?;
    let keys = execute::message_keys(&instructions);
    let mut bank = Bank::build_filtered(&world, spec, &plan, &config, Some(&keys))?;
    bank.restrict_to(&instructions);
    let programs = execute::programs(&world, spec, context.program_id, context.candidate)?;
    execute::assert_candidate(&programs, context.program_id, context.candidate_sha256)?;
    let mut session = Session::new(&bank, &programs, context.program_id)?;
    let execution = execute::execute_unit(&mut session, spec, &plan, &unit, &bank.relayer)?;
    let finding = match (expected, execution.outcome) {
        (Expected::Migrate, Outcome::Migrated) => None,
        (Expected::Reject, Outcome::Rejected) => {
            if execution
                .failure
                .as_ref()
                .is_some_and(|f| f.rollback_verified)
            {
                None
            } else {
                Some(Finding::RollbackViolation)
            }
        }
        (Expected::Migrate, Outcome::Rejected) => Some(Finding::UnexpectedFailure),
        (Expected::Reject, _) => Some(Finding::UnexpectedSuccess),
        (Expected::Migrate, Outcome::ReconciliationMismatch) => {
            Some(Finding::ReconciliationMismatch)
        }
    };
    Ok(CaseResult {
        case_id: case.case_id.clone(),
        kind: case.kind.clone(),
        provenance: case.provenance,
        source_account: case.source_account.clone(),
        expected,
        planned_class,
        planned_reasons: unit.reasons.iter().map(|r| r.code.clone()).collect(),
        derived_world_sha256: world.sha256()?,
        derived_plan_sha256: plan.sha256()?,
        behaves_as_specified: finding.is_none(),
        finding,
        execution,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StressOutcome {
    pub stress_plan_sha256: String,
    pub cases: Vec<CaseResult>,
    pub not_executable: Vec<NotReachable>,
}

pub fn run(context: &CaseContext<'_>, plan: &StressPlan) -> Result<StressOutcome> {
    let mut cases = vec![];
    let mut not_executable = vec![];
    for case in &plan.cases {
        match execute_case(context, case) {
            Ok(result) => cases.push(result),
            Err(error) => not_executable.push(NotReachable {
                kind: format!("{} {}", case.case_id, case.kind),
                reason: format!("{error:#}"),
            }),
        }
    }
    Ok(StressOutcome {
        stress_plan_sha256: plan.sha256()?,
        cases,
        not_executable,
    })
}
