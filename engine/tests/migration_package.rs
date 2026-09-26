//! Token Migration V1 packages end to end: schema 3 identity, offline evaluation,
//! structured evidence, gate policies and byte-exact offline replay.
#[path = "common/migration.rs"]
mod migration_common;
use eplyx_engine::{
    migration::gate::Policy,
    migration::{input as package, invariants::MigrationInvariant, pipeline},
};
use migration_common::*;
use serde_json::json;

fn recipe_a() -> eplyx_engine::migration::fixture::Recipe {
    recipe(json!({
        "schemaVersion": 1, "id": "package-case-a", "description": "legacy to legacy package",
        "programs": "pinnedMainnetCapture",
        "clock": {"slot": "1000", "unixTimestamp": "1760000000", "epoch": "400"},
        "populationMint": "source",
        "wallets": wallets(&["alice", "bob", "carol", "issuer"]),
        "programOwned": [{"label": "pool"}],
        "mints": [
            {"label": "source", "tokenProgram": LEGACY, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}},
            {"label": "destination", "tokenProgram": LEGACY, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}}
        ],
        "tokenAccounts": [
            {"label": "alice-source", "mint": "source", "owner": {"label": "alice"}, "layout": "associated", "amount": "1001"},
            {"label": "bob-source", "mint": "source", "owner": {"label": "bob"}, "layout": "associated", "amount": "20"},
            {"label": "carol-source", "mint": "source", "owner": {"label": "carol"}, "layout": "associated", "amount": "33", "frozen": true},
            {"label": "pool-source", "mint": "source", "owner": {"label": "pool"}, "layout": "associated", "amount": "400"}
        ]
    }))
}

#[test]
fn separate_input_identity_binds_spec_program_config_and_invariants() {
    let recipe = recipe_a();
    let spec = spec(&recipe, LEGACY, LEGACY, json!({}));
    let program = reference();
    let (_, a) = package(
        "identity-a",
        &recipe,
        &spec,
        &program,
        MigrationInvariant::recommended(),
    );
    let (_, b) = package(
        "identity-b",
        &recipe,
        &spec,
        &program,
        MigrationInvariant::recommended(),
    );
    assert_eq!(
        a.analysis_input_sha256(),
        b.analysis_input_sha256(),
        "identity is content-addressed"
    );
    let mut changed = spec.clone();
    changed.conversion.rounding = eplyx_engine::migration::spec::Rounding::Ceiling;
    let (_, c) = package(
        "identity-c",
        &recipe,
        &changed,
        &program,
        MigrationInvariant::recommended(),
    );
    assert_ne!(a.analysis_input_sha256(), c.analysis_input_sha256());
    let (_, d) = package(
        "identity-d",
        &recipe,
        &spec,
        &candidate("eplyx_token_migration_defect_fee_ceiling"),
        MigrationInvariant::recommended(),
    );
    assert_ne!(a.analysis_input_sha256(), d.analysis_input_sha256());
    let (_, e) = package("identity-e", &recipe, &spec, &program, vec![]);
    assert_ne!(a.analysis_input_sha256(), e.analysis_input_sha256());
    let manifest = std::fs::read(a.root().join("change.json")).unwrap();
    let config = std::fs::read(a.root().join("state.json")).unwrap();
    let (_, _, declared) = package::declared_identity(&manifest, &config).unwrap();
    assert_eq!(declared, a.analysis_input_sha256());
    // No path, URL or secret can enter identity inputs.
    let text = String::from_utf8(manifest).unwrap();
    assert!(!text.contains("://") && !text.contains("/Users/") && !text.contains("/tmp/"));
}

#[test]
fn preflight_writes_structured_evidence_and_replays_offline_byte_for_byte() {
    let recipe = recipe_a();
    let spec = spec(&recipe, LEGACY, LEGACY, json!({}));
    let program = reference();
    let (root, validated) = package(
        "pipeline",
        &recipe,
        &spec,
        &program,
        MigrationInvariant::recommended(),
    );
    let result = root.join("result");
    let report = pipeline::run(
        validated.root(),
        &result,
        Policy::BlockOnly,
        pipeline::Isolation::InProcess,
    )
    .unwrap();
    assert_eq!(report["transition_kind"], "token_migration");
    assert_eq!(report["official_transition"], "NotTested");
    assert_eq!(report["funds_moved"], false);
    assert_eq!(report["readiness"]["mechanism"]["status"], "Ready");
    assert_eq!(report["readiness"]["funding"]["status"], "Ready");
    assert_eq!(
        report["readiness"]["population"]["status"], "Incomplete",
        "a frozen and a program-controlled holder cannot migrate"
    );
    let codes = report["readiness"]["population"]["codes"].to_string();
    assert!(
        codes.contains("SOURCE_FROZEN") && codes.contains("AUTHORITY_PATH_UNAVAILABLE"),
        "{codes}"
    );
    assert_eq!(
        report["reconciliation"]["status"],
        "ReconciledForExecutedUnits"
    );
    assert_eq!(report["gate_outcome"], "Warn");
    for section in [
        "impact",
        "reconciliation",
        "compatibility",
        "authority",
        "coverage",
        "execution",
    ] {
        assert!(report[section].is_object(), "missing section {section}");
    }
    assert_eq!(pipeline::exit_code(&report).unwrap(), 0);
    let replayed = pipeline::replay(validated.root(), &result).unwrap();
    assert_eq!(replayed, report);
    let strict =
        pipeline::replay_with_policy(validated.root(), &result, Some(Policy::Strict)).unwrap();
    assert_eq!(strict["gate_outcome"], "Block");
    assert_eq!(
        strict["readiness"], report["readiness"],
        "policy never changes analytical findings"
    );
    // Tampering with a saved artifact fails the replay.
    let path = result.join("migration.plan.json");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.push(b' ');
    std::fs::write(&path, bytes).unwrap();
    assert!(pipeline::replay(validated.root(), &result).is_err());
}

#[test]
fn insufficient_reserve_blocks_under_every_policy_with_a_machine_readable_reason() {
    let recipe = recipe_a();
    let spec = spec(
        &recipe,
        LEGACY,
        LEGACY,
        json!({"destinationFunding": {"kind": "reserveTransfer", "reserve": {"kind": "proposed", "fundedRaw": "505"}}}),
    );
    let (root, validated) = package(
        "underfunded",
        &recipe,
        &spec,
        &reference(),
        MigrationInvariant::recommended(),
    );
    let result = root.join("result");
    let report = pipeline::run(
        validated.root(),
        &result,
        Policy::BlockOnly,
        pipeline::Isolation::InProcess,
    )
    .unwrap();
    assert_eq!(report["readiness"]["funding"]["status"], "Blocked");
    assert_eq!(report["gate_outcome"], "Block");
    assert!(report["gate_reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("INSUFFICIENT_RESERVE")));
    assert_eq!(pipeline::exit_code(&report).unwrap(), 1);
    assert_eq!(pipeline::replay(validated.root(), &result).unwrap(), report);
}
