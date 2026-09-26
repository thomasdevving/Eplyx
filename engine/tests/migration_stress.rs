//! Migration-aware stress: a frozen deterministic case matrix, executed case by
//! case in fresh VMs, judged against the specification (never the candidate).
#[path = "common/migration.rs"]
mod migration_common;
use eplyx_engine::{
    migration::{
        adapter,
        planner::RehearsalClockPolicy,
        stress::{self, CaseContext, CaseProvenance, Expected, Finding},
    },
    replay::hash_bytes as sha256,
};
use migration_common::*;
use serde_json::json;

fn t22_world_recipe() -> eplyx_engine::migration::fixture::Recipe {
    recipe(json!({
        "schemaVersion": 1, "id": "stress-t22", "description": "stress matrix world",
        "programs": "pinnedMainnetCapture",
        "clock": {"slot": "5000", "unixTimestamp": "1760000000", "epoch": "400"},
        "populationMint": "source",
        "wallets": wallets(&["alice", "bob", "carol", "issuer", "m1", "m2", "m3"]),
        "multisigs": [{"label": "treasury", "tokenProgram": T22, "threshold": 2, "signers": ["m1", "m2", "m3"]}],
        "mints": [
            {"label": "source", "tokenProgram": T22, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"},
             "extensions": [{"kind": "transferFee", "bps": 30, "maximumFee": "100000"}]},
            {"label": "destination", "tokenProgram": T22, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}}
        ],
        "tokenAccounts": [
            {"label": "alice-source", "mint": "source", "owner": {"label": "alice"}, "layout": "associated", "amount": "9000000"},
            {"label": "bob-source", "mint": "source", "owner": {"label": "bob"}, "layout": "associated", "amount": "123457"},
            {"label": "bob-destination", "mint": "destination", "owner": {"label": "bob"}, "layout": "associated", "amount": "0"},
            {"label": "carol-source", "mint": "source", "owner": {"label": "carol"}, "layout": "associated", "amount": "77"},
            {"label": "treasury-source", "mint": "source", "owner": {"label": "treasury"}, "layout": "associated", "amount": "5000"}
        ]
    }))
}

fn stress_spec(
    recipe: &eplyx_engine::migration::fixture::Recipe,
) -> eplyx_engine::migration::spec::TokenMigrationV1 {
    spec(
        recipe,
        T22,
        T22,
        json!({
            "conversion": {"ratioBasis": "raw", "numerator": "2", "denominator": "3", "rounding": "floor", "fee": {"kind": "sourceBps", "bps": 125}, "minimumOutputRaw": "10"},
            "window": {"activation": {"kind": "slot", "value": "6000"}, "deadline": {"kind": "slot", "value": "9000"}}
        }),
    )
}

fn run(program: &[u8]) -> (stress::StressPlan, stress::StressOutcome) {
    let recipe = t22_world_recipe();
    let spec = stress_spec(&recipe);
    let world = world(&recipe, &spec);
    let plan = plan(&spec, &world, program);
    let stress_plan = stress::select(&spec, &world, &plan).unwrap();
    let context = CaseContext {
        spec: &spec,
        change_spec_id: &change_spec_id(&spec),
        world: &world,
        program_id: adapter::REFERENCE_PROGRAM_ID,
        candidate: &resolved_candidate(&spec, program),
        candidate_sha256: &sha256(program),
        clock_policy: RehearsalClockPolicy::Activation,
    };
    let outcome = stress::run(&context, &stress_plan).unwrap();
    (stress_plan, outcome)
}

#[test]
fn reference_candidate_behaves_as_specified_across_the_matrix() {
    let program = reference();
    let (plan, outcome) = run(&program);
    let kinds: Vec<&str> = plan.cases.iter().map(|c| c.kind.as_str()).collect();
    for kind in [
        "SmallestMigratableHolder",
        "LargestMigratableHolder",
        "MinimumMigratableAmount",
        "BelowMinimumAmount",
        "RoundingExact",
        "RoundingDust",
        "FeeStep",
        "BelowFeeStep",
        "ReserveExact",
        "ReserveOneBelow",
        "ReserveOneAbove",
        "BeforeActivation",
        "AtActivation",
        "BeforeDeadline",
        "AtDeadline",
        "FrozenSource",
        "FrozenDestination",
        "CpiGuardSource",
        "MemoRequiredDestination",
        "DelegateExactAllowance",
        "DelegateInsufficientAllowance",
        "WrongSigner",
        "MultisigUnderThreshold",
    ] {
        assert!(
            kinds.contains(&kind),
            "missing stress case {kind}: {kinds:?}"
        );
    }
    assert!(
        outcome.not_executable.is_empty(),
        "{:#?}",
        outcome.not_executable
    );
    for case in &outcome.cases {
        assert!(
            case.behaves_as_specified,
            "{} {} expected {:?} ({:?} {:?}), got {:?}: {:?}",
            case.case_id,
            case.kind,
            case.expected,
            case.planned_class,
            case.planned_reasons,
            case.execution.outcome,
            case.execution.failure
        );
    }
    let get = |kind: &str| outcome.cases.iter().find(|c| c.kind == kind).unwrap();
    assert_eq!(get("AtDeadline").expected, Expected::Reject);
    assert_eq!(
        get("AtDeadline")
            .execution
            .failure
            .as_ref()
            .unwrap()
            .error_name
            .as_deref(),
        Some("MigrationWindowClosed")
    );
    assert_eq!(
        get("BeforeActivation")
            .execution
            .failure
            .as_ref()
            .unwrap()
            .error_name
            .as_deref(),
        Some("MigrationNotActive")
    );
    assert_eq!(
        get("ReserveOneBelow")
            .execution
            .failure
            .as_ref()
            .unwrap()
            .error_name
            .as_deref(),
        Some("InsufficientReserve")
    );
    // Matrix claims verified by the real Token-2022 program:
    assert_eq!(
        get("CpiGuardSource")
            .execution
            .failure
            .as_ref()
            .unwrap()
            .error_name
            .as_deref(),
        Some("CpiGuardBurnBlocked")
    );
    assert_eq!(
        get("MemoRequiredDestination")
            .execution
            .failure
            .as_ref()
            .unwrap()
            .error_name
            .as_deref(),
        Some("NoMemo")
    );
    assert_eq!(
        get("FrozenSource")
            .execution
            .failure
            .as_ref()
            .unwrap()
            .error_name
            .as_deref(),
        Some("AccountFrozen")
    );
    assert_eq!(
        get("DelegateInsufficientAllowance")
            .execution
            .failure
            .as_ref()
            .unwrap()
            .error_name
            .as_deref(),
        Some("InsufficientFunds")
    );
    assert_eq!(
        get("WrongSigner")
            .execution
            .failure
            .as_ref()
            .unwrap()
            .error_name
            .as_deref(),
        Some("UnauthorizedHolderAuthority")
    );
    assert_eq!(
        get("SmallestMigratableHolder").provenance,
        CaseProvenance::Synthetic
    );
    assert_eq!(
        get("FrozenSource").provenance,
        CaseProvenance::DerivedFromSynthetic
    );
}

#[test]
fn defective_candidates_are_caught_from_their_bytes() {
    let (_, deadline) = run(&candidate(
        "eplyx_token_migration_defect_deadline_inclusive",
    ));
    let at_deadline = deadline
        .cases
        .iter()
        .find(|c| c.kind == "AtDeadline")
        .unwrap();
    assert_eq!(at_deadline.finding, Some(Finding::UnexpectedSuccess));
    let (_, fee) = run(&candidate("eplyx_token_migration_defect_fee_ceiling"));
    let mismatches: Vec<_> = fee.cases.iter().filter(|c| c.finding.is_some()).collect();
    assert!(!mismatches.is_empty(), "fee-ceiling defect went undetected");
    assert!(
        mismatches
            .iter()
            .any(|c| c.finding == Some(Finding::ReconciliationMismatch)),
        "{:#?}",
        mismatches
            .iter()
            .map(|c| (&c.kind, c.finding))
            .collect::<Vec<_>>()
    );
}
