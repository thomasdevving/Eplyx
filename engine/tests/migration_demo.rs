//! The hackathon SPACEX/PreStocks demo, kept reproducible two ways: the original
//! fixed-ratio package replays its saved report byte for byte, and the same demo is
//! a normal TokenMigrationV1 example rehearsed against the world rebuilt from the
//! run's frozen mainnet captures. Neither is an official issuer migration.
#[path = "common/migration.rs"]
mod migration_common;
use eplyx_engine::{
    migration::gate::Policy,
    migration::{
        adapter, frozen,
        invariants::MigrationInvariant,
        pipeline::{self, RehearsalInputs},
        planner::RehearsalClockPolicy,
        spec::TokenMigrationV1,
        world::WorldKind,
    },
    replay::hash_bytes as sha256,
    repo_root,
};
use migration_common::*;
use std::collections::BTreeMap;

const RUN: &str = "fixtures/migration/spacex";
const PACKAGE: &str = "fixtures/migration/archived-terms";
const EXAMPLE: &str = "examples/migrations/spacex-demo/migration.json";

// The fixed-ratio product replay remains in STA; T0 records its outcome.

#[test]
fn spacex_demo_is_a_token_migration_v1_example_over_frozen_state() {
    let (manifest, config) = frozen::package_parts(&repo_root().join(PACKAGE)).unwrap();
    let world =
        frozen::world_from_package_run(&repo_root().join(RUN), RUN, &manifest.source_mint).unwrap();
    assert_eq!(world.kind, WorldKind::ObservedCapture);
    assert!(world.accounts.values().all(|a| a.origin.is_observed()));
    let spec = frozen::spec_from_fixed_ratio(&manifest, &config, &world).unwrap();
    let path = repo_root().join(EXAMPLE);
    let checked_in: TokenMigrationV1 =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        spec, checked_in,
        "the checked-in example is the exact TokenMigrationV1 equivalent of the demo package"
    );
    let program = reference();
    let input_root = temp_dir("spacex-input");
    let world_bytes = eplyx_engine::canonical::document(&world)
        .unwrap()
        .into_bytes();
    let config = eplyx_engine::migration::input::Config {
        state: eplyx_engine::migration::input::StateSource::CapturedWorld {
            artifact: "world.json".into(),
            sha256: sha256(&world_bytes),
        },
        rehearsal_clock: RehearsalClockPolicy::Activation,
        max_rehearsal_units: 5000,
        max_captured_holders: 5000,
    };
    let validated = eplyx_engine::migration::input::assemble(
        &input_root,
        &spec,
        adapter::REFERENCE_PROGRAM_ID,
        &program,
        &config,
        Some(&world_bytes),
        MigrationInvariant::recommended(),
    )
    .unwrap();
    let inputs = RehearsalInputs::new(&validated, Policy::BlockOnly);
    let mut artifacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let report = pipeline::evaluate_world(&inputs, &world, &mut |file, bytes| {
        artifacts.insert(file.into(), bytes.to_vec());
        Ok(())
    })
    .unwrap();
    assert_t0("spacex-demo-frozen", &report, serde_json::Value::Null);
    let population = &report["impact"]["population"];
    assert_eq!(population["token_accounts"], 17870);
    assert_eq!(population["positive_balance_accounts"], 10091);
    assert_eq!(population["enumeration"], "CompleteForQuery");
    assert_eq!(
        report["state"]["rehearsal_clock"]["basis"],
        "DerivedAtActivationTimestamp"
    );
    assert_eq!(report["official_transition"], "NotTested");
    let classes = report["impact"]["classes"].to_string();
    assert!(
        classes.contains("UnverifiableDestination"),
        "destinations the saved run never inspected stay unknown: {classes}"
    );
    assert!(
        report["coverage"]["rehearsal"]["attempted"]
            .as_u64()
            .unwrap()
            > 0,
        "holders with inspected destinations are rehearsed"
    );
    assert_eq!(
        report["readiness"]["mechanism"]["status"], "Ready",
        "{:#}",
        report["readiness"]
    );
    // The frozen capture gives an exact, honest picture of the demo terms applied
    // to every captured holder, in plan order with one shared proposed reserve.
    let count = |class: &str| {
        report["impact"]["classes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["class"] == class)
            .map_or(0, |c| c["accounts"].as_u64().unwrap())
    };
    assert_eq!(count("Migratable"), 8);
    assert_eq!(
        count("InsufficientReserve"),
        3,
        "the 1,000,000 USDC proposed reserve runs out in plan order"
    );
    assert_eq!(
        count("OutputBelowMinimum"),
        308,
        "1 raw converts to 0 at a 1/2 floor ratio"
    );
    assert_eq!(count("UnverifiableDestination"), 6779);
    assert_eq!(count("UnverifiableAuthority"), 2925);
    assert_eq!(count("AuthorityPathUnavailable"), 68);
    assert_eq!(count("ZeroBalance"), 7779);
    assert_eq!(report["readiness"]["funding"]["status"], "Blocked");
    assert!(report["readiness"]["funding"]["codes"]
        .to_string()
        .contains("INSUFFICIENT_RESERVE"));
    assert_eq!(report["coverage"]["rehearsal"]["migrated"], 8);
    assert!(report["reconciliation"]["equations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["holds"] == true));
    assert_eq!(
        report["reconciliation"]["status"],
        "ReconciledForExecutedUnits"
    );
    // MAIN canonical decimal strings and fixture provenance paths re-identify the world.
    // STA world ID remains frozen in T0; account bytes and classifications are unchanged.
    // (Byte-exact replay of complete runs is covered by the package tests.)
    assert_eq!(
        report["state"]["world_sha256"],
        "99491370b6381a4a21eee5b1aa07cb1a9ae88e19de7ac86f292930f783ec7dc6"
    );
    assert_eq!(
        artifacts.len(),
        7,
        "plan, stress plan, rehearsal, stress results, unsigned plan, report, markdown"
    );
}

/// Phase timings on the real 17,870-account frozen population. Opt-in:
/// `cargo test --release --test migration_demo -- --ignored --nocapture`.
#[test]
#[ignore]
fn performance_breakdown_on_the_frozen_population() {
    use eplyx_engine::migration::{execute, planner, rehearsal, stress};
    use std::time::Instant;
    let t = Instant::now();
    let (manifest, config) = frozen::package_parts(&repo_root().join(PACKAGE)).unwrap();
    let world =
        frozen::world_from_package_run(&repo_root().join(RUN), RUN, &manifest.source_mint).unwrap();
    let world_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let world_sha = world.sha256().unwrap();
    let hash_ms = t.elapsed().as_millis();
    let spec = frozen::spec_from_fixed_ratio(&manifest, &config, &world).unwrap();
    let program = reference();
    let digest = change_spec_id(&spec);
    let t = Instant::now();
    let plan = planner::plan(&planner::PlanInput {
        spec: &spec,
        change_spec_id: &digest,
        world: &world,
        program_id: adapter::REFERENCE_PROGRAM_ID,
        candidate_program_sha256: &sha256(&program),
        clock_policy: RehearsalClockPolicy::Activation,
        reserve_override: None,
        focus: None,
    })
    .unwrap();
    let plan_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let plan_bytes = eplyx_engine::canonical::document(&plan).unwrap();
    let plan_json_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let stress_plan = stress::select(&spec, &world, &plan).unwrap();
    let select_ms = t.elapsed().as_millis();
    let programs = execute::programs(
        &world,
        &spec,
        adapter::REFERENCE_PROGRAM_ID,
        &resolved_candidate(&spec, &program),
    )
    .unwrap();
    let t = Instant::now();
    let rehearsed = rehearsal::rehearse(&spec, &world, &plan, &programs, 5000).unwrap();
    let rehearsal_ms = t.elapsed().as_millis();
    let context = stress::CaseContext {
        spec: &spec,
        change_spec_id: &digest,
        world: &world,
        program_id: adapter::REFERENCE_PROGRAM_ID,
        candidate: &resolved_candidate(&spec, &program),
        candidate_sha256: &sha256(&program),
        clock_policy: RehearsalClockPolicy::Activation,
    };
    let t = Instant::now();
    let outcome = stress::run(&context, &stress_plan).unwrap();
    let stress_ms = t.elapsed().as_millis();
    eprintln!(
        "world: {} accounts built in {world_ms} ms, hashed in {hash_ms} ms ({world_sha})\nplan: {} units in {plan_ms} ms, canonical JSON {} bytes in {plan_json_ms} ms\nstress select: {} cases in {select_ms} ms; stress execution {stress_ms} ms ({} ms/case)\nsequential rehearsal: {} transactions in {rehearsal_ms} ms",
        world.accounts.len(),
        plan.units.len(),
        plan_bytes.len(),
        stress_plan.cases.len(),
        stress_ms / outcome.cases.len().max(1) as u128,
        rehearsed.executions.len(),
    );
}
