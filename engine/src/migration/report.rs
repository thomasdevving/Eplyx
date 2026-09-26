//! Structured Token Migration V1 evidence.
//!
//! One versioned `report.json` whose sections are the migration impact,
//! reconciliation, compatibility, authority, coverage and execution reports, with
//! four analytical readiness axes carrying machine-readable codes. Counterexamples
//! live in the search artifact and the executable intent in the unsigned plan; the
//! report references both by digest instead of duplicating them. `report.md`
//! answers the operator's questions in plain language first.
use super::{
    authority::{AuthorityPath, OwnerClass},
    execute::{Outcome, UnitExecution},
    extensions,
    planner::{ImpactClass, MigrationPlan},
    rehearsal::{PopulationRehearsal, ReconciliationStatus},
    spec::{
        DestinationFunding, MigrationAuthority, SourceDisposition, TokenMigrationV1, TokenProgram,
        WindowState,
    },
    stress::{Finding, StressOutcome, StressPlan},
    world::World,
};
use crate::migration::requirements::Result as InvariantResult;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const REPORT_SCHEMA: u32 = 1;
pub const REPORT_KIND: &str = "token_migration_rehearsal";
const MAX_LISTED: usize = 50;

pub struct ReportInput<'a> {
    pub spec: &'a TokenMigrationV1,
    pub world: &'a World,
    pub plan: &'a MigrationPlan,
    pub stress_plan: &'a StressPlan,
    pub stress: &'a StressOutcome,
    pub rehearsal: &'a PopulationRehearsal,
    pub invariants: &'a [InvariantResult],
    pub identity: Value,
    pub artifacts: BTreeMap<String, String>,
    pub unsigned_units: usize,
    /// The unsigned plan's descriptors re-executed in a separate fresh VM.
    pub unsigned_cross_check: &'a [super::unsigned::CrossCheck],
    pub loaded_programs: Vec<Value>,
}

/// Measured restrictions on declared migration requirements. These are separate
/// from evidence gaps and from prose/reason-code presentation.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RequirementViolation {
    SourceFrozen,
    OutputBelowMinimum,
    AuthorityPathUnavailable,
    OutsideEligibility,
    SourceUninitialized,
    InsufficientReserve,
    WindowConflict,
    MintRestriction,
}
fn requirement_violations(plan: &MigrationPlan) -> BTreeSet<RequirementViolation> {
    use RequirementViolation::*;
    let mut out = BTreeSet::new();
    for unit in &plan.units {
        if unit.source_balance_raw == "0" {
            continue;
        }
        let violation = match unit.class {
            ImpactClass::Frozen => Some(SourceFrozen),
            ImpactClass::OutputBelowMinimum => Some(OutputBelowMinimum),
            ImpactClass::AuthorityPathUnavailable => Some(AuthorityPathUnavailable),
            ImpactClass::OutsideEligibility => Some(OutsideEligibility),
            ImpactClass::UninitializedSource => Some(SourceUninitialized),
            ImpactClass::InsufficientReserve => Some(InsufficientReserve),
            ImpactClass::WindowConflict => Some(WindowConflict),
            _ => None,
        };
        out.extend(violation);
    }
    // Known mint restrictions differ from an unsupported hook, encrypted supply,
    // or unrecognized extension whose effects cannot be judged by this executor.
    if plan.mint_findings.iter().any(|f| {
        f.support == extensions::Support::Unsupported
            && matches!(f.extension.as_str(), "NonTransferable" | "Pausable")
    }) {
        out.insert(MintRestriction);
    }
    out
}

fn axis(status: &str, codes: BTreeSet<String>, reasons: Vec<String>) -> Value {
    json!({"status": status, "codes": codes, "reasons": reasons})
}

fn finding_code(finding: Finding) -> &'static str {
    match finding {
        Finding::UnexpectedFailure => "UNEXPECTED_MIGRATION_FAILURE",
        Finding::UnexpectedSuccess => "UNEXPECTED_MIGRATION_SUCCESS",
        Finding::ReconciliationMismatch => "MIGRATION_ARITHMETIC_MISMATCH",
        Finding::RollbackViolation => "ROLLBACK_VIOLATION",
    }
}

fn readiness(input: &ReportInput<'_>) -> Value {
    let plan = input.plan;
    let unit_class = |id: &str| plan.units.iter().find(|u| u.unit_id == id).map(|u| u.class);
    // Mechanism: every executed case behaves exactly as the specification requires.
    let mut codes = BTreeSet::new();
    let mut reasons = vec![];
    for case in &input.stress.cases {
        if let Some(finding) = case.finding {
            codes.insert(finding_code(finding).to_string());
            reasons.push(format!(
                "Stress {} {} ({:?}) expected {:?}, candidate {:?}",
                case.case_id, case.kind, case.provenance, case.expected, case.execution.outcome
            ));
        }
    }
    for execution in &input.rehearsal.executions {
        let planned = unit_class(&execution.unit_id);
        let problem = match (execution.outcome, planned) {
            (Outcome::ReconciliationMismatch, _) => Some("MIGRATION_ARITHMETIC_MISMATCH"),
            (Outcome::Rejected, Some(ImpactClass::Migratable)) => {
                Some("UNEXPECTED_MIGRATION_FAILURE")
            }
            (Outcome::Migrated, Some(ImpactClass::InsufficientReserve)) => {
                Some("PLANNER_VM_DIVERGENCE")
            }
            _ if !execution.unexpected_changes.is_empty() => Some("UNINTENDED_ACCOUNT_CHANGE"),
            _ => None,
        };
        if let Some(code) = problem {
            codes.insert(code.to_string());
            if reasons.len() < MAX_LISTED {
                reasons.push(format!(
                    "Population unit {} ({}): {code}",
                    execution.unit_id, execution.source_account
                ));
            }
        }
    }
    let migrated_any = input
        .stress
        .cases
        .iter()
        .any(|c| c.execution.outcome == Outcome::Migrated)
        || input
            .rehearsal
            .executions
            .iter()
            .any(|e| e.outcome == Outcome::Migrated);
    for check in input
        .unsigned_cross_check
        .iter()
        .filter(|c| !c.matches_rehearsal)
    {
        codes.insert("UNSIGNED_PLAN_DIVERGENCE".to_string());
        reasons.push(format!(
            "Unsigned plan unit {} re-executed as {} with message {}, unlike its rehearsal.",
            check.unit_id, check.outcome, check.message_sha256
        ));
    }
    let mechanism = if !codes.is_empty() {
        axis("Blocked", codes, reasons)
    } else if !migrated_any {
        axis(
            "Incomplete",
            ["NOTHING_EXECUTED".to_string()].into(),
            vec!["No migration executed successfully in the VM.".into()],
        )
    } else {
        axis("Ready", BTreeSet::new(), vec![])
    };

    // Funding: the reserve covers every eligible holder, or mint authority is held.
    let reconciliation = &input.rehearsal.reconciliation;
    let mut codes = BTreeSet::new();
    let mut reasons = vec![];
    let mut blocked = false;
    for r in &plan.funding.reasons {
        codes.insert(r.code.clone());
        reasons.push(r.detail.clone());
        if r.code != "PROPOSED_ADDRESS_NOT_INSPECTED" {
            blocked = true;
        }
    }
    if !plan.funding.path_available {
        codes.insert("FUNDING_PATH_UNAVAILABLE".into());
    }
    let reserve = matches!(
        input.spec.destination_funding,
        DestinationFunding::ReserveTransfer { .. }
    );
    let mut unknown = false;
    if reserve {
        match (
            &reconciliation.required_reserve_raw,
            &plan.funding.available_raw,
        ) {
            (Some(required), Some(available)) => {
                match (required.parse::<u128>(), available.parse::<u128>()) {
                    (Ok(r), Ok(a)) if r > a => {
                        blocked = true;
                        codes.insert("INSUFFICIENT_RESERVE".into());
                        reasons.push(format!(
                            "Eligible holders need {required} raw destination units; the reserve holds {available} raw (shortfall {}).",
                            r - a
                        ));
                    }
                    (Ok(_), Ok(_)) => {}
                    // Never read an unparsable amount as zero.
                    _ => unknown = true,
                }
            }
            _ => unknown = true,
        }
    }
    let funding = if blocked {
        axis("Blocked", codes, reasons)
    } else if unknown || !codes.is_empty() {
        codes.insert("FUNDING_UNVERIFIED".into());
        axis("Incomplete", codes, reasons)
    } else {
        axis("Ready", BTreeSet::new(), vec![])
    };

    // Population: every enumerated positive holder migrated in the rehearsal.
    let mut codes = BTreeSet::new();
    let mut reasons = vec![];
    let complete = matches!(
        plan.population_enumeration.as_str(),
        "CompleteForQuery" | "CompleteSyntheticFixture"
    );
    if !complete {
        codes.insert("POPULATION_INCOMPLETE".to_string());
        reasons.push(format!(
            "Population enumeration is {}.",
            plan.population_enumeration
        ));
    }
    let mut window_blocked = false;
    if plan.window_state != WindowState::Open {
        window_blocked = true;
        codes.insert("MIGRATION_WINDOW_INVALID".to_string());
        reasons.push(format!(
            "The migration window is {:?} at the rehearsal Clock.",
            plan.window_state
        ));
    }
    let mut by_class: BTreeMap<&'static str, usize> = BTreeMap::new();
    for unit in plan.units.iter().filter(|u| u.source_balance_raw != "0") {
        if !matches!(unit.class, ImpactClass::Migratable) {
            *by_class.entry(unit.class.code()).or_default() += 1;
        }
    }
    let not_migrated = input
        .rehearsal
        .executions
        .iter()
        .filter(|e| e.outcome != Outcome::Migrated)
        .count();
    for (code, count) in &by_class {
        codes.insert(code.to_string());
        let holders = if *count == 1 { "holder" } else { "holders" };
        reasons.push(format!("{count} positive-balance {holders}: {code}"));
    }
    if input.rehearsal.truncated {
        codes.insert("REHEARSAL_BUDGET_EXHAUSTED".to_string());
        reasons.push(format!(
            "{} attempted units were beyond the rehearsal budget.",
            input.rehearsal.not_attempted_by_budget
        ));
    }
    if not_migrated > 0 {
        codes.insert("REHEARSAL_UNITS_NOT_MIGRATED".to_string());
        reasons.push(format!(
            "{not_migrated} attempted units did not migrate in the sequential rehearsal."
        ));
    }
    let population = if window_blocked {
        axis("Blocked", codes, reasons)
    } else if codes.is_empty() {
        axis("Ready", BTreeSet::new(), vec![])
    } else {
        axis("Incomplete", codes, reasons)
    };

    let reconciliation_axis = match reconciliation.status {
        ReconciliationStatus::Mismatch => axis(
            "Blocked",
            ["SUPPLY_RECONCILIATION_FAILED".to_string()].into(),
            reconciliation
                .equations
                .iter()
                .filter(|e| !e.holds)
                .map(|e| format!("{}: {} != {}", e.name, e.left, e.right))
                .collect(),
        ),
        ReconciliationStatus::NothingExecuted => axis(
            "Incomplete",
            ["NOTHING_EXECUTED".to_string()].into(),
            vec!["No unit was executed in the population rehearsal.".into()],
        ),
        _ => axis("Ready", BTreeSet::new(), vec![]),
    };
    json!({
        "mechanism": mechanism,
        "funding": funding,
        "population": population,
        "reconciliation": reconciliation_axis,
    })
}

fn declared(readiness: &Value) -> &'static str {
    let statuses: Vec<&str> = ["mechanism", "funding", "population", "reconciliation"]
        .iter()
        .map(|a| readiness[a]["status"].as_str().unwrap_or("Incomplete"))
        .collect();
    if statuses.contains(&"Blocked") {
        "Blocked"
    } else if statuses.iter().all(|s| *s == "Ready") {
        "Ready"
    } else {
        "Incomplete"
    }
}

fn side(spec_side: &super::spec::TokenSide) -> Value {
    json!({
        "mint": spec_side.mint,
        "token_program": spec_side.token_program,
        "token_program_label": spec_side.program().map(TokenProgram::label).unwrap_or("unknown"),
        "decimals": spec_side.decimals,
    })
}

fn impact(plan: &MigrationPlan) -> Value {
    let positive = plan
        .units
        .iter()
        .filter(|u| u.source_balance_raw != "0")
        .count();
    let classes: Vec<Value> = plan
        .class_totals
        .iter()
        .map(|(class, total)| {
            let code = plan
                .units
                .iter()
                .find(|u| format!("{:?}", u.class) == *class)
                .map(|u| u.class.code())
                .unwrap_or("UNKNOWN");
            json!({"class": class, "code": code, "accounts": total.accounts, "balance_raw": total.balance_raw, "migratable": class == "Migratable"})
        })
        .collect();
    let stranded: Vec<Value> = plan
        .units
        .iter()
        .filter(|u| u.source_balance_raw != "0" && u.class != ImpactClass::Migratable)
        .take(MAX_LISTED)
        .map(|u| {
            json!({
                "unit_id": u.unit_id, "source_account": u.source_account, "owner": u.owner,
                "balance_raw": u.source_balance_raw, "class": u.class, "code": u.class.code(),
                "reasons": u.reasons.iter().map(|r| json!({"code": r.code, "detail": r.detail})).collect::<Vec<_>>(),
                "required_authority": u.authority.required(),
            })
        })
        .collect();
    json!({
        "population": {
            "token_accounts": plan.units.len(),
            "positive_balance_accounts": positive,
            "zero_balance_accounts": plan.units.len() - positive,
            "enumeration": plan.population_enumeration,
            "authority_resolution": plan.authority_resolution,
            "undecoded_rows": plan.undecoded_accounts.len(),
        },
        "classes": classes,
        "required_authority": plan.authority_totals,
        "not_migratable_examples": stranded,
        "not_migratable_listed_limit": MAX_LISTED,
        "note": "Unsupported, unverifiable and ineligible holders are not successful migrations; each keeps its own class and reason.",
    })
}

fn compatibility(spec: &TokenMigrationV1, plan: &MigrationPlan) -> Value {
    let mut account_findings: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    for unit in &plan.units {
        for f in unit
            .account_findings
            .iter()
            .chain(unit.destination.findings.iter())
        {
            *account_findings
                .entry((
                    format!("{:?}", f.side),
                    f.code.clone(),
                    format!("{:?}", f.support),
                ))
                .or_default() += 1;
        }
    }
    json!({
        "matrix_version": extensions::MATRIX_VERSION,
        "source": side(&spec.source),
        "destination": side(&spec.destination),
        "token_programs_independent": "Each side is decoded, derived (ATA) and executed with its own token program; they are never treated as interchangeable.",
        "mint_findings": plan.mint_findings,
        "account_findings": account_findings.into_iter().map(|((side, code, support), accounts)| json!({"side": side, "code": code, "support": support, "accounts": accounts})).collect::<Vec<_>>(),
        "support_matrix": extensions::matrix().into_iter().map(|(ext, rule)| json!({"extension": ext, "classification": rule})).collect::<Vec<_>>(),
    })
}

fn authority(spec: &TokenMigrationV1, world: &World, plan: &MigrationPlan) -> Value {
    let mint = |address: &str| {
        world.mint(address).ok().map(
            |m| json!({"mint_authority": m.mint_authority, "freeze_authority": m.freeze_authority}),
        )
    };
    let mut owner_classes: BTreeMap<String, usize> = BTreeMap::new();
    let mut signer_roles: BTreeMap<String, usize> = BTreeMap::new();
    let mut alternatives = 0usize;
    for unit in plan.units.iter().filter(|u| u.source_balance_raw != "0") {
        *owner_classes
            .entry(format!("{:?}", unit.authority.owner_class()))
            .or_default() += 1;
        if unit.class == ImpactClass::Migratable {
            for signer in &unit.required_signers {
                *signer_roles
                    .entry(format!("{:?}", signer.role))
                    .or_default() += 1;
            }
        }
        if let AuthorityPath::Available {
            alternatives: a, ..
        } = &unit.authority
        {
            alternatives += usize::from(!a.is_empty());
        }
    }
    let mut capabilities = vec![];
    if spec.destination_funding == DestinationFunding::MintTo {
        capabilities.push(format!(
            "Destination mint authority must be the migration authority {} (external issuer action).",
            plan.overlay.migration_authority
        ));
    }
    if let MigrationAuthority::External { address } = &spec.authorities.migration_authority {
        capabilities.push(format!(
            "External migration authority {address} must co-sign every migration."
        ));
    }
    if spec.allows(super::spec::HolderAuthorization::PermanentDelegate) {
        capabilities.push(
            "Permanent-delegate migrations need the source mint's permanent delegate to sign (issuer capability)."
                .into(),
        );
    }
    json!({
        "migration_authority": {"kind": plan.overlay.authority_kind, "address": plan.overlay.migration_authority},
        "fee_payer_role": "relayer",
        "captured_mint_authorities": {"source": mint(&spec.source.mint), "destination": mint(&spec.destination.mint)},
        "expectations": plan.identity_checks.iter().filter(|c| c.code == "REQUIRED_AUTHORITY_MISMATCH").collect::<Vec<_>>(),
        "external_capabilities": capabilities,
        "owner_classes_positive_holders": owner_classes,
        "required_signer_roles_migratable_units": signer_roles,
        "holders_with_alternative_paths": alternatives,
        "key_possession": "Unknown for every signer; the VM assumes signatures locally.",
        "program_controlled_owners": plan.units.iter().filter(|u| u.authority.owner_class() == OwnerClass::ProgramControlled && u.source_balance_raw != "0").count(),
    })
}

fn coverage(input: &ReportInput<'_>) -> Value {
    let plan = input.plan;
    let mut by_provenance: BTreeMap<String, usize> = BTreeMap::new();
    for case in &input.stress.cases {
        *by_provenance
            .entry(format!("{:?}", case.provenance))
            .or_default() += 1;
    }
    let mut not_attempted: BTreeMap<String, usize> = BTreeMap::new();
    for unit in plan
        .units
        .iter()
        .filter(|u| !super::execute::attempted(u) && u.source_balance_raw != "0")
    {
        *not_attempted
            .entry(format!("{:?}", unit.class))
            .or_default() += 1;
    }
    let world = input.world;
    json!({
        "world": {
            "kind": world.kind, "base_kind": world.base_kind(), "cluster": world.cluster,
            "observed_slots": world.observed_slots.map(|(a,b)|[a.to_string(),b.to_string()]), "accounts": world.accounts.len(),
            "inspected_absent": world.inspected_absent.len(), "world_sha256": plan.world_sha256,
            "provenance_rule": "Observed accounts carry an RPC record and pointer; synthetic accounts carry a recipe step; derived accounts carry their parent and mutation.",
        },
        "rehearsal": {
            "attempted": input.rehearsal.executions.len(),
            "migrated": input.rehearsal.executions.iter().filter(|e| e.outcome == Outcome::Migrated).count(),
            "rejected": input.rehearsal.executions.iter().filter(|e| e.outcome == Outcome::Rejected).count(),
            "mismatched": input.rehearsal.executions.iter().filter(|e| e.outcome == Outcome::ReconciliationMismatch).count(),
            "truncated": input.rehearsal.truncated,
            "max_units": input.rehearsal.max_units,
            "not_attempted_positive_holders_by_class": not_attempted,
        },
        "stress": {
            "frozen_cases": input.stress_plan.cases.len(),
            "executed": input.stress.cases.len(),
            "by_provenance": by_provenance,
            "behaving": input.stress.cases.iter().filter(|c| c.behaves_as_specified).count(),
            "deviating": input.stress.cases.iter().filter(|c| !c.behaves_as_specified).count(),
            "not_reachable": input.stress_plan.not_reachable,
            "not_executable": input.stress.not_executable,
            "rule": "Cases were frozen before execution and never replaced.",
        },
        "not_inspected": {
            "destinations": plan.units.iter().filter(|u| u.destination.action == "NotInspected" && u.source_balance_raw != "0").count(),
            "owners": plan.units.iter().filter(|u| u.authority.owner_class() == OwnerClass::Unknown && u.source_balance_raw != "0").count(),
            "undecoded_rows": plan.undecoded_accounts.len(),
        },
        "search": "Counterexample search is a separate bounded step (eplyx search); preflight does not claim its coverage.",
    })
}

fn failure_row(e: &UnitExecution) -> Value {
    json!({
        "unit_id": e.unit_id, "source_account": e.source_account, "amount_raw": e.amount_raw,
        "outcome": e.outcome,
        "failure": e.failure,
        "failed_checks": e.reconciliation.iter().filter(|c| !c.holds).collect::<Vec<_>>(),
        "message_sha256": e.message_sha256,
    })
}

fn execution(input: &ReportInput<'_>) -> Value {
    let plan = input.plan;
    let compute: Vec<u64> = input
        .rehearsal
        .executions
        .iter()
        .map(|e| e.compute_units)
        .collect();
    json!({
        "vm": {
            "runtime": "LiteSVM 0.16 (fresh process-local bank; nothing is broadcast)",
            "signature_verification": "disabled: signer privileges are assumed locally, key possession stays unknown",
            "candidate": {"program_id": plan.overlay.program_id, "sha256": plan.candidate_program_sha256, "loader": super::adapter::CANDIDATE_LOADER},
            "programs": input.loaded_programs,
            "clock": plan.rehearsal_clock,
        },
        "population_rehearsal": {
            "mode": "Sequential transactions in plan order over one bank with a shared reserve",
            "executed": input.rehearsal.executions.len(),
            "compute_units_max": compute.iter().max().map(u64::to_string),
            "compute_units_total": compute.iter().sum::<u64>().to_string(),
            "not_migrated": input.rehearsal.executions.iter().filter(|e| e.outcome != Outcome::Migrated).take(MAX_LISTED).map(failure_row).collect::<Vec<_>>(),
        },
        "stress_cases": input.stress.cases.iter().map(|c| json!({
            "case_id": c.case_id, "kind": c.kind, "provenance": c.provenance,
            "source_account": c.source_account,
            "expected": c.expected, "planned_class": c.planned_class, "planned_reasons": c.planned_reasons,
            "outcome": c.execution.outcome,
            "error_name": c.execution.failure.as_ref().and_then(|f| f.error_name.clone()),
            "failure_stage": c.execution.failure.as_ref().map(|f| f.stage.clone()),
            "rollback_verified": c.execution.failure.as_ref().map(|f| f.rollback_verified),
            "reconciled": c.execution.reconciled,
            "behaves_as_specified": c.behaves_as_specified,
            "finding": c.finding,
            "derived_world_sha256": c.derived_world_sha256,
            "message_sha256": c.execution.message_sha256,
        })).collect::<Vec<_>>(),
    })
}

pub fn build(input: &ReportInput<'_>) -> Result<Value> {
    let spec = input.spec;
    let plan = input.plan;
    let readiness = readiness(input);
    let status = declared(&readiness);
    let mut report = input.identity.clone();
    let object = report
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("identity must be an object"))?;
    let entries = json!({
        "requirement_violations": requirement_violations(plan),
        "schema_version": REPORT_SCHEMA,
        "report_kind": REPORT_KIND,
        "transition_kind": "token_migration",
        "provenance": "OperatorSupplied",
        "deployment_origin": "Proposed",
        "migration": {
            "source": side(&spec.source),
            "destination": side(&spec.destination),
            "conversion": {"declared": spec.conversion, "effective_raw_terms": plan.effective_terms},
            "window": spec.window,
            "window_state_at_rehearsal": plan.window_state,
            "source_disposition": match spec.source_disposition { SourceDisposition::Burn => "burn", SourceDisposition::Escrow => "escrow" },
            "destination_funding": spec.destination_funding,
            "eligibility": spec.eligibility,
            "overlay": plan.overlay,
        },
        "state": {
            "world_kind": input.world.kind,
            "base_kind": input.world.base_kind(),
            "cluster": input.world.cluster,
            "world_sha256": plan.world_sha256,
            "rehearsal_clock": plan.rehearsal_clock,
            "limitations": input.world.limitations,
        },
        "readiness": readiness,
        "declared_preflight_status": status,
        "official_transition": "NotTested",
        "funds_moved": false,
        "impact": impact(plan),
        "reconciliation": input.rehearsal.reconciliation,
        "compatibility": compatibility(spec, plan),
        "authority": authority(spec, input.world, plan),
        "coverage": coverage(input),
        "execution": execution(input),
        "invariant_schema_version": super::invariants::INVARIANT_SCHEMA_VERSION,
        "invariants": input.invariants,
        "artifacts": input.artifacts,
        "unsigned_plan": {
            "artifact": "migration.unsigned-plan.json",
            "sha256": input.artifacts.get("migration.unsigned-plan.json"),
            "units": input.unsigned_units,
            "cross_check": {
                "method": "The serialized descriptors re-executed in plan order in a separate fresh LiteSVM built only from the world and the plan's setup requirements; each unit's message and outcome compared with the rehearsal. Nothing is signed or sent.",
                "executed_units": input.unsigned_cross_check.len(),
                "matching_units": input.unsigned_cross_check.iter().filter(|c| c.matches_rehearsal).count(),
            },
        },
        "limitations": [
            "This is a local VM rehearsal of an operator-supplied candidate. No mainnet transaction was built, signed or sent, and no funds moved.",
            "OfficialTransition is NotTested: a candidate mechanism is not an issuer-defined migration, and issuer authorization is out of scope.",
            "Signatures are assumed locally; whether anyone holds the required keys is unknown.",
            "Stress and derived cases are local states. A derived failure is not an observed mainnet failure.",
            "A rehearsal that succeeds for tested states does not show that every possible holder or state is safe.",
            "The candidate uses MAIN’s upgradeable-loader execution model.",
        ],
    });
    if let Value::Object(map) = entries {
        object.extend(map);
    }
    Ok(report)
}

fn status_word(status: &str) -> &str {
    match status {
        "Ready" => "Yes",
        "Blocked" => "No",
        _ => "Not established",
    }
}

/// Consumer-first human summary, in the order an operator asks.
pub fn markdown(report: &Value) -> String {
    let r = &report["readiness"];
    let rec = &report["reconciliation"];
    let imp = &report["impact"]["population"];
    let mut out = format!(
        "# Token migration rehearsal\n\nPackage `{}` · candidate `{}` (Proposed) · state {} ({})\n\n",
        report["analysis_input_sha256"].as_str().unwrap_or(""),
        report["candidate_program_sha256"].as_str().unwrap_or(""),
        report["state"]["world_kind"].as_str().unwrap_or(""),
        report["state"]["cluster"].as_str().unwrap_or(""),
    );
    out.push_str(&format!(
        "1. **Can the migration execute for the tested states?** {} (mechanism {}).\n",
        status_word(r["mechanism"]["status"].as_str().unwrap_or("")),
        r["mechanism"]["status"].as_str().unwrap_or("")
    ));
    out.push_str(&format!(
        "2. **Population coverage:** {} token accounts, {} with a positive balance; {} attempted and {} migrated in the sequential rehearsal (enumeration {}).\n",
        imp["token_accounts"], imp["positive_balance_accounts"],
        report["coverage"]["rehearsal"]["attempted"], report["coverage"]["rehearsal"]["migrated"],
        imp["enumeration"].as_str().unwrap_or("")
    ));
    let blocked: Vec<String> = report["impact"]["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["migratable"] == false && c["class"] != "ZeroBalance")
        .map(|c| {
            format!(
                "{} {} ({} raw)",
                c["accounts"],
                c["class"].as_str().unwrap_or(""),
                c["balance_raw"].as_str().unwrap_or("0")
            )
        })
        .collect();
    out.push_str(&format!(
        "3. **Who cannot migrate:** {}.\n",
        if blocked.is_empty() {
            "none among the enumerated positive holders".to_string()
        } else {
            blocked.join("; ")
        }
    ));
    out.push_str(&format!(
        "4. **Does accounting reconcile?** {} — {}.\n",
        status_word(r["reconciliation"]["status"].as_str().unwrap_or("")),
        rec["status"].as_str().unwrap_or("")
    ));
    out.push_str(&format!(
        "5. **Is funding sufficient?** {} (required {}, available {}).\n",
        status_word(r["funding"]["status"].as_str().unwrap_or("")),
        rec["required_reserve_raw"].as_str().unwrap_or("n/a"),
        rec["available_reserve_raw"].as_str().unwrap_or("n/a")
    ));
    let deviating = report["coverage"]["stress"]["deviating"]
        .as_u64()
        .unwrap_or(0);
    out.push_str(&format!(
        "6. **Failures:** {} stress cases deviate from the specification; {} population units did not migrate. Run `eplyx search` for bounded counterexamples.\n",
        deviating,
        report["execution"]["population_rehearsal"]["not_migrated"].as_array().map_or(0, Vec::len)
    ));
    out.push_str("7. **Limitations:** local VM rehearsal only; OfficialTransition NotTested; signatures assumed locally; derived states are not chain state.\n");
    out.push_str(
        "8. **Reproduce:** `eplyx reproduce <cx-id>` or replay this run offline (no RPC).\n",
    );
    out.push_str(&format!(
        "\nAnalytical status: **{}**\n",
        report["declared_preflight_status"]
            .as_str()
            .unwrap_or("Incomplete")
    ));
    if let Some(findings) = report["invariants"].as_array() {
        out.push_str("\n## Migration invariants\n\n");
        for f in findings {
            out.push_str(&format!(
                "- **{}** {} ({}): {}\n",
                f["status"].as_str().unwrap_or(""),
                f["invariant_type"].as_str().unwrap_or(""),
                f["severity"].as_str().unwrap_or(""),
                f["explanation"].as_str().unwrap_or("")
            ));
        }
    }
    if let Some(gate) = report.get("deployment_gate") {
        out.push_str(&format!(
            "\n## Deployment gate\n\n**{}** under `{}`.\n\n",
            match gate["outcome"].as_str().unwrap_or("") {
                "Pass" => "PASS",
                "Warn" => "PASS WITH WARNINGS",
                _ => "BLOCKED",
            },
            gate["policy"].as_str().unwrap_or("")
        ));
        let codes: Vec<&str> = gate["reason_codes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !codes.is_empty() {
            out.push_str(&format!("Reason codes: `{}`\n\n", codes.join("`, `")));
        }
        for reason in gate["reasons"].as_array().into_iter().flatten().take(40) {
            out.push_str(&format!("- {}\n", reason.as_str().unwrap_or("")));
        }
    }
    out.push_str("\nNo mainnet transaction was sent and no funds moved.\n");
    out
}
