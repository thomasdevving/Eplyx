//! Versioned migration invariants over verified rehearsal evidence.
//!
//! Definitions are typed, bounded and part of the package identity. Evaluation reads
//! only results that were produced by actual VM execution and exact reconciliation in
//! this run; it never selects cases, executes transactions or upgrades evidence. The
//! result type, statuses and severities are the existing package-invariant types, so
//! the existing deployment gate applies unchanged: a blocking Violated invariant
//! blocks under every policy, a blocking Indeterminate one blocks only under strict.
use super::{
    execute::{Outcome, UnitExecution},
    planner::{ImpactClass, MigrationPlan},
    rehearsal::{PopulationRehearsal, ReconciliationStatus},
    spec::{DestinationFunding, TokenMigrationV1},
    stress::{Expected, Finding, StressOutcome},
};
use crate::migration::requirements::{Result as InvariantResult, Severity, Status};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const INVARIANT_SCHEMA_VERSION: u32 = 2;
pub const EVALUATION_VERSION: &str = "eplyx-migration-invariants/v1";
pub const MAX_INVARIANTS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MigrationInvariant {
    CandidateBinaryMatchesPackage { severity: Severity },
    MintIdentityMatchesSpec { severity: Severity },
    RequiredAuthoritiesMatch { severity: Severity },
    MigrationArithmeticMatchesSpec { severity: Severity },
    NoUnintendedAccountChanges { severity: Severity },
    FailedMigrationsRollBack { severity: Severity },
    SupplyReconciles { severity: Severity },
    ReserveNeverUnderflows { severity: Severity },
    ReserveCoversEligibleHolders { severity: Severity },
    WindowRulesHold { severity: Severity },
    ExtensionSemanticsHonored { severity: Severity },
    NoUnsupportedExecutionSucceeds { severity: Severity },
    NoStressCaseDeviates { severity: Severity },
    AllPositiveHoldersMigrate { severity: Severity },
}

impl MigrationInvariant {
    pub fn severity(&self) -> Severity {
        match self {
            Self::CandidateBinaryMatchesPackage { severity }
            | Self::MintIdentityMatchesSpec { severity }
            | Self::RequiredAuthoritiesMatch { severity }
            | Self::MigrationArithmeticMatchesSpec { severity }
            | Self::NoUnintendedAccountChanges { severity }
            | Self::FailedMigrationsRollBack { severity }
            | Self::SupplyReconciles { severity }
            | Self::ReserveNeverUnderflows { severity }
            | Self::ReserveCoversEligibleHolders { severity }
            | Self::WindowRulesHold { severity }
            | Self::ExtensionSemanticsHonored { severity }
            | Self::NoUnsupportedExecutionSucceeds { severity }
            | Self::NoStressCaseDeviates { severity }
            | Self::AllPositiveHoldersMigrate { severity } => *severity,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::CandidateBinaryMatchesPackage { .. } => "candidate_binary_matches_package",
            Self::MintIdentityMatchesSpec { .. } => "mint_identity_matches_spec",
            Self::RequiredAuthoritiesMatch { .. } => "required_authorities_match",
            Self::MigrationArithmeticMatchesSpec { .. } => "migration_arithmetic_matches_spec",
            Self::NoUnintendedAccountChanges { .. } => "no_unintended_account_changes",
            Self::FailedMigrationsRollBack { .. } => "failed_migrations_roll_back",
            Self::SupplyReconciles { .. } => "supply_reconciles",
            Self::ReserveNeverUnderflows { .. } => "reserve_never_underflows",
            Self::ReserveCoversEligibleHolders { .. } => "reserve_covers_eligible_holders",
            Self::WindowRulesHold { .. } => "window_rules_hold",
            Self::ExtensionSemanticsHonored { .. } => "extension_semantics_honored",
            Self::NoUnsupportedExecutionSucceeds { .. } => "no_unsupported_execution_succeeds",
            Self::NoStressCaseDeviates { .. } => "no_stress_case_deviates",
            Self::AllPositiveHoldersMigrate { .. } => "all_positive_holders_migrate",
        }
    }

    pub fn scope(&self) -> &'static str {
        match self {
            Self::CandidateBinaryMatchesPackage { .. } => "VmLoaderInput",
            Self::MintIdentityMatchesSpec { .. } | Self::RequiredAuthoritiesMatch { .. } => {
                "CapturedMintState"
            }
            Self::MigrationArithmeticMatchesSpec { .. }
            | Self::NoUnintendedAccountChanges { .. }
            | Self::FailedMigrationsRollBack { .. } => "AllExecutedMigrations",
            Self::SupplyReconciles { .. }
            | Self::ReserveNeverUnderflows { .. }
            | Self::ReserveCoversEligibleHolders { .. } => "SequentialPopulationRehearsal",
            Self::WindowRulesHold { .. }
            | Self::ExtensionSemanticsHonored { .. }
            | Self::NoUnsupportedExecutionSucceeds { .. }
            | Self::NoStressCaseDeviates { .. } => "FrozenStressMatrix",
            Self::AllPositiveHoldersMigrate { .. } => "EnumeratedPositiveBalancePopulation",
        }
    }

    pub fn id(&self) -> String {
        let identity = crate::canonical::document(self).unwrap_or_default();
        format!(
            "minv-{}",
            &crate::replay::hash_bytes(identity.as_bytes())[..20]
        )
    }

    /// The default blocking set written by `eplyx init --migration`.
    pub fn recommended() -> Vec<Self> {
        use Severity::{Blocking, Warning};
        let mut all = vec![
            Self::CandidateBinaryMatchesPackage { severity: Blocking },
            Self::MintIdentityMatchesSpec { severity: Blocking },
            Self::RequiredAuthoritiesMatch { severity: Blocking },
            Self::MigrationArithmeticMatchesSpec { severity: Blocking },
            Self::NoUnintendedAccountChanges { severity: Blocking },
            Self::FailedMigrationsRollBack { severity: Blocking },
            Self::SupplyReconciles { severity: Blocking },
            Self::ReserveNeverUnderflows { severity: Blocking },
            Self::ReserveCoversEligibleHolders { severity: Blocking },
            Self::WindowRulesHold { severity: Blocking },
            Self::ExtensionSemanticsHonored { severity: Blocking },
            Self::NoUnsupportedExecutionSucceeds { severity: Blocking },
            Self::NoStressCaseDeviates { severity: Blocking },
            Self::AllPositiveHoldersMigrate { severity: Warning },
        ];
        all.sort();
        all
    }
}

pub struct Evidence<'a> {
    pub spec: &'a TokenMigrationV1,
    pub plan: &'a MigrationPlan,
    pub rehearsal: &'a PopulationRehearsal,
    pub stress: &'a StressOutcome,
    pub candidate_loaded_sha256: Option<&'a str>,
    pub package_program_sha256: &'a str,
    pub refs: Vec<String>,
}

fn all_executions<'a>(evidence: &'a Evidence<'a>) -> impl Iterator<Item = &'a UnitExecution> {
    evidence
        .rehearsal
        .executions
        .iter()
        .chain(evidence.stress.cases.iter().map(|c| &c.execution))
}

const WINDOW_CASES: [&str; 4] = [
    "BeforeActivation",
    "AtActivation",
    "BeforeDeadline",
    "AtDeadline",
];
const EXTENSION_CASES: [&str; 4] = [
    "CpiGuardSource",
    "MemoRequiredDestination",
    "FrozenSource",
    "FrozenDestination",
];

pub fn evaluate(
    definitions: &[MigrationInvariant],
    evidence: &Evidence<'_>,
) -> Vec<InvariantResult> {
    definitions
        .iter()
        .map(|definition| {
            let (status, explanation) = evaluate_one(definition, evidence);
            InvariantResult {
                invariant_id: definition.id(),
                invariant_type: definition.kind().into(),
                severity: definition.severity(),
                status,
                scope: definition.scope().into(),
                config: json!({}),
                evidence_refs: evidence.refs.clone(),
                explanation,
                evaluation_version: EVALUATION_VERSION.into(),
            }
        })
        .collect()
}

fn evaluate_one(definition: &MigrationInvariant, e: &Evidence<'_>) -> (Status, String) {
    use MigrationInvariant::*;
    let executed = all_executions(e).count();
    match definition {
        CandidateBinaryMatchesPackage { .. } => match e.candidate_loaded_sha256 {
            Some(loaded) if loaded == e.package_program_sha256 => (
                Status::Satisfied,
                "The VM loaded exactly the packaged candidate bytes.".into(),
            ),
            Some(loaded) => (
                Status::Violated,
                format!(
                    "The VM loaded {loaded}, not the packaged {}.",
                    e.package_program_sha256
                ),
            ),
            None => (
                Status::Indeterminate,
                "No candidate was loaded into a VM.".into(),
            ),
        },
        MintIdentityMatchesSpec { .. } => {
            let checks: Vec<_> = e
                .plan
                .identity_checks
                .iter()
                .filter(|c| c.code == "MINT_IDENTITY_MISMATCH")
                .collect();
            if checks.iter().all(|c| c.satisfied) {
                (
                    Status::Satisfied,
                    format!(
                        "{} captured mint identity checks match the specification.",
                        checks.len()
                    ),
                )
            } else {
                (
                    Status::Violated,
                    "Captured mint identity differs from the specification.".into(),
                )
            }
        }
        RequiredAuthoritiesMatch { .. } => {
            let checks: Vec<_> = e
                .plan
                .identity_checks
                .iter()
                .filter(|c| c.code == "REQUIRED_AUTHORITY_MISMATCH")
                .collect();
            if checks.is_empty() {
                (
                    Status::NotApplicable,
                    "The specification declares no authority expectations.".into(),
                )
            } else if let Some(failed) = checks.iter().find(|c| !c.satisfied) {
                (
                    Status::Violated,
                    format!(
                        "{}: expected {}, captured {}.",
                        failed.check, failed.expected, failed.observed
                    ),
                )
            } else {
                (
                    Status::Satisfied,
                    format!(
                        "All {} declared authority expectations match captured state.",
                        checks.len()
                    ),
                )
            }
        }
        MigrationArithmeticMatchesSpec { .. } => {
            let mismatched = all_executions(e)
                .filter(|x| x.outcome == Outcome::ReconciliationMismatch)
                .count();
            let migrated = all_executions(e)
                .filter(|x| x.outcome == Outcome::Migrated)
                .count();
            if mismatched > 0 {
                (Status::Violated, format!("{mismatched} executed migrations differ from the specification's exact arithmetic or deltas."))
            } else if migrated == 0 {
                (
                    Status::Indeterminate,
                    "No migration executed successfully, so arithmetic was not exercised.".into(),
                )
            } else {
                (Status::Satisfied, format!("All {migrated} successful migrations reconciled exactly (debit, disposition, funding, credit, fees, log)."))
            }
        }
        NoUnintendedAccountChanges { .. } => {
            let offending = all_executions(e)
                .filter(|x| !x.unexpected_changes.is_empty())
                .count();
            if offending > 0 {
                (
                    Status::Violated,
                    format!("{offending} executions changed accounts outside the expected set."),
                )
            } else if executed == 0 {
                (Status::Indeterminate, "Nothing was executed.".into())
            } else {
                (
                    Status::Satisfied,
                    format!("{executed} executions changed only expected accounts."),
                )
            }
        }
        FailedMigrationsRollBack { .. } => {
            let failures: Vec<_> = all_executions(e)
                .filter(|x| x.outcome == Outcome::Rejected)
                .collect();
            if failures.is_empty() {
                (
                    Status::Indeterminate,
                    "No rejected execution exercised rollback.".into(),
                )
            } else if failures
                .iter()
                .all(|x| x.failure.as_ref().is_some_and(|f| f.rollback_verified))
            {
                (
                    Status::Satisfied,
                    format!(
                        "All {} rejected executions rolled back every referenced account.",
                        failures.len()
                    ),
                )
            } else {
                (
                    Status::Violated,
                    "A rejected execution left referenced state changed.".into(),
                )
            }
        }
        SupplyReconciles { .. } => match e.rehearsal.reconciliation.status {
            ReconciliationStatus::NothingExecuted => (
                Status::Indeterminate,
                "The population rehearsal executed nothing.".into(),
            ),
            ReconciliationStatus::Mismatch => (
                Status::Violated,
                format!(
                    "Reconciliation equations fail: {}.",
                    e.rehearsal
                        .reconciliation
                        .equations
                        .iter()
                        .filter(|q| !q.holds)
                        .map(|q| q.name.clone())
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            ),
            _ => (
                Status::Satisfied,
                "Every population reconciliation equation holds over the executed units.".into(),
            ),
        },
        ReserveNeverUnderflows { .. } => {
            if !matches!(
                e.spec.destination_funding,
                DestinationFunding::ReserveTransfer { .. }
            ) {
                return (
                    Status::NotApplicable,
                    "Destination tokens are minted; there is no reserve.".into(),
                );
            }
            let r = &e.rehearsal.reconciliation;
            match r
                .equations
                .iter()
                .find(|q| q.name.starts_with("available reserve"))
            {
                Some(q) if q.holds => (
                    Status::Satisfied,
                    format!(
                        "The reserve moved from {} to {} raw and never released more than it held.",
                        r.available_reserve_raw.clone().unwrap_or_default(),
                        r.remaining_reserve_raw.clone().unwrap_or_default()
                    ),
                ),
                Some(_) => (
                    Status::Violated,
                    "The reserve released an amount inconsistent with its balance.".into(),
                ),
                None => (
                    Status::Indeterminate,
                    "The reserve was not measured.".into(),
                ),
            }
        }
        ReserveCoversEligibleHolders { .. } => {
            if !matches!(
                e.spec.destination_funding,
                DestinationFunding::ReserveTransfer { .. }
            ) {
                return (
                    Status::NotApplicable,
                    "Destination tokens are minted; there is no reserve.".into(),
                );
            }
            let r = &e.rehearsal.reconciliation;
            match (&r.required_reserve_raw, &r.available_reserve_raw) {
                (Some(required), Some(available)) => {
                    match (required.parse::<u128>(), available.parse::<u128>()) {
                        (Ok(req), Ok(avail)) if req <= avail => (Status::Satisfied, format!("The reserve of {available} raw covers the {required} raw every eligible holder needs.")),
                        (Ok(req), Ok(avail)) => (Status::Violated, format!("INSUFFICIENT_RESERVE: eligible holders need {required} raw; the reserve holds {available} raw (shortfall {}).", req - avail)),
                        _ => (
                            Status::Indeterminate,
                            "The reserve amounts are not exact integers.".into(),
                        ),
                    }
                }
                _ => (
                    Status::Indeterminate,
                    "The reserve balance is unknown.".into(),
                ),
            }
        }
        WindowRulesHold { .. } => cases(e, &WINDOW_CASES, "no migration window is declared"),
        ExtensionSemanticsHonored { .. } => cases(
            e,
            &EXTENSION_CASES,
            "no extension-sensitive case is reachable",
        ),
        NoUnsupportedExecutionSucceeds { .. } => {
            let stress = e
                .stress
                .cases
                .iter()
                .filter(|c| c.finding == Some(Finding::UnexpectedSuccess))
                .count();
            let population = e
                .rehearsal
                .executions
                .iter()
                .filter(|x| x.outcome != Outcome::Rejected)
                .filter(|x| {
                    e.plan
                        .units
                        .iter()
                        .find(|u| u.unit_id == x.unit_id)
                        .is_some_and(|u| u.class != ImpactClass::Migratable)
                })
                .count();
            if stress + population > 0 {
                (
                    Status::Violated,
                    format!(
                        "{} executions succeeded although the specification requires rejection.",
                        stress + population
                    ),
                )
            } else {
                let rejects = e
                    .stress
                    .cases
                    .iter()
                    .filter(|c| c.expected == Expected::Reject)
                    .count();
                if rejects == 0 {
                    (
                        Status::Indeterminate,
                        "No case the specification rejects was executed.".into(),
                    )
                } else {
                    (Status::Satisfied, format!("All {rejects} cases the specification rejects were rejected by the candidate."))
                }
            }
        }
        NoStressCaseDeviates { .. } => {
            let deviating = e
                .stress
                .cases
                .iter()
                .filter(|c| !c.behaves_as_specified)
                .count();
            if e.stress.cases.is_empty() {
                (Status::Indeterminate, "No stress case executed.".into())
            } else if deviating > 0 {
                (
                    Status::Violated,
                    format!(
                        "{deviating} of {} frozen stress cases deviate from the specification.",
                        e.stress.cases.len()
                    ),
                )
            } else {
                (
                    Status::Satisfied,
                    format!(
                        "All {} frozen stress cases behave as specified.",
                        e.stress.cases.len()
                    ),
                )
            }
        }
        AllPositiveHoldersMigrate { .. } => {
            let complete = matches!(
                e.plan.population_enumeration.as_str(),
                "CompleteForQuery" | "CompleteSyntheticFixture"
            );
            let positive = e
                .plan
                .units
                .iter()
                .filter(|u| u.source_balance_raw != "0")
                .count();
            let migrated = e
                .rehearsal
                .executions
                .iter()
                .filter(|x| x.outcome == Outcome::Migrated)
                .count();
            if !complete {
                (Status::Indeterminate, format!("Population enumeration is {}; stranded holders outside it cannot be ruled out.", e.plan.population_enumeration))
            } else if positive == 0 {
                (
                    Status::NotApplicable,
                    "The enumerated population holds no positive balance.".into(),
                )
            } else if migrated == positive && !e.rehearsal.truncated {
                (Status::Satisfied, format!("All {positive} positive-balance holders migrated in the sequential rehearsal."))
            } else {
                (Status::Violated, format!("{} of {positive} positive-balance holders did not migrate under the current mechanism.", positive - migrated.min(positive)))
            }
        }
    }
}

fn cases(e: &Evidence<'_>, kinds: &[&str], absent: &str) -> (Status, String) {
    let relevant: Vec<_> = e
        .stress
        .cases
        .iter()
        .filter(|c| kinds.contains(&c.kind.as_str()))
        .collect();
    if relevant.is_empty() {
        return (Status::NotApplicable, format!("Not applicable: {absent}."));
    }
    let deviating: Vec<_> = relevant
        .iter()
        .filter(|c| !c.behaves_as_specified)
        .collect();
    if deviating.is_empty() {
        (
            Status::Satisfied,
            format!("{} boundary cases behave as specified.", relevant.len()),
        )
    } else {
        (
            Status::Violated,
            format!(
                "Deviating cases: {}.",
                deviating
                    .iter()
                    .map(|c| match c.finding {
                        Some(finding) => format!("{} {} ({finding:?})", c.case_id, c.kind),
                        None => format!("{} {}", c.case_id, c.kind),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    }
}
