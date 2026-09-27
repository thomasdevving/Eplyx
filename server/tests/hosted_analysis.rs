//! Offline multi-kind checks use MAIN's durable queue and actual child binary.
mod common;
use axum::http::StatusCode;
use common::*;
use eplyx_engine::{
    migration::{
        adapter,
        input::{self, Config, StateSource},
        planner::RehearsalClockPolicy,
        spec::TokenMigrationV1,
    },
    replay::hash_bytes,
};
use eplyx_server::{
    hosted::worker,
    project::Project,
    registry::{RunOutcome, RunStatus},
};
use serde_json::{json, Value};
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}
fn project(h: &Harness) -> String {
    let id = eplyx_server::ids::project();
    h.state
        .registry
        .create_project(&Project::analytical(&id, "Offline analysis").unwrap())
        .unwrap();
    id
}
fn migration_parts() -> Vec<(&'static str, Vec<u8>)> {
    migration_parts_with_programs(false)
}
fn migration_parts_with_programs(pinned: bool) -> Vec<(&'static str, Vec<u8>)> {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let mut value: Value = serde_json::from_slice(
        &std::fs::read(root.join("examples/migrations/minimal/migration.json")).unwrap(),
    )
    .unwrap();
    if pinned {
        value["window"]["activation"]["value"] = json!("500000000");
        value["window"]["deadline"]["value"] = json!("600000000");
    }
    let terms: TokenMigrationV1 = serde_json::from_value(value).unwrap();
    let candidate = std::fs::read(root.join("artifacts/eplyx_token_migration.so")).unwrap();
    let mut fixture =
        std::fs::read(root.join("examples/migrations/minimal/fixtures/world.json")).unwrap();
    if pinned {
        let mut value: Value = serde_json::from_slice(&fixture).unwrap();
        value["programs"] = json!("pinnedMainnetCapture");
        value["clock"]["slot"] = json!("500000000");
        fixture = serde_json::to_vec(&value).unwrap();
    }
    let config = Config {
        state: StateSource::SyntheticFixture {
            recipe: "fixture.json".into(),
            recipe_sha256: hash_bytes(&fixture),
        },
        rehearsal_clock: RehearsalClockPolicy::Activation,
        max_rehearsal_units: 100,
        max_captured_holders: 100,
    };
    let input = temp.path().join("input");
    input::assemble(
        &input,
        &terms,
        adapter::REFERENCE_PROGRAM_ID,
        &candidate,
        &config,
        Some(&fixture),
        vec![],
    )
    .unwrap();
    let mut parts = vec![
        ("candidate", candidate),
        (
            "change_spec",
            std::fs::read(input.join("change.json")).unwrap(),
        ),
        (
            "state_input",
            std::fs::read(input.join("state.json")).unwrap(),
        ),
        ("state_artifact", fixture),
    ];
    if pinned {
        parts.push((
            "pinned_program_capture",
            std::fs::read(root.join(eplyx_engine::migration::fixture::PROGRAM_CAPTURE)).unwrap(),
        ));
    }
    parts
}
async fn submit(h: &Harness, p: &str, parts: &[(&str, Vec<u8>)]) -> Value {
    let refs: Vec<_> = parts.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    let (code, v) = h.submit_parts(p, OPERATOR, &refs).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{v}");
    v
}
#[tokio::test]
async fn migration_queues_without_active_bundle_and_runs_reference_under_both_policies() {
    let h = Harness::new(0);
    let p = project(&h);
    let mut parts = migration_parts();
    let accepted = submit(&h, &p, &parts).await;
    let id = accepted["run_id"].as_str().unwrap();
    let queued = h.state.registry.load_run(id).unwrap();
    assert_eq!(queued.status, RunStatus::Queued);
    assert!(queued.exit_code.is_none());
    assert!(queued.bundle_sha256.is_empty());
    assert!(!queued.report_available);
    h.state.runs.add_permits(1);
    assert_eq!(
        h.wait_until(id, RunStatus::is_terminal).await,
        RunStatus::Passed
    );
    let (status, bytes) = h
        .get_bytes(&format!("/v1/runs/{id}/report.json"), OPERATOR)
        .await;
    assert_eq!(status, StatusCode::OK);
    let report: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["deployment_gate"]["outcome"], "Warn");
    assert_eq!(report["coverage"]["stress"]["behaving"], 20);
    let first = h.state.registry.load_run(id).unwrap();
    assert_eq!(first.attempts.len(), 1);
    assert!(first.hosted_analysis.as_ref().unwrap().projection.is_some());
    parts.push((
        "analysis_options",
        serde_json::to_vec(&json!({"policy":"strict"})).unwrap(),
    ));
    let strict = submit(&h, &p, &parts).await;
    let second = strict["run_id"].as_str().unwrap();
    assert_eq!(
        h.wait_until(second, RunStatus::is_terminal).await,
        RunStatus::Failed
    );
    assert_eq!(
        h.state.registry.load_run(second).unwrap().exit_code,
        Some(1)
    );
    assert_eq!(h.state.registry.project_run_ids(&p).unwrap().len(), 2);
    let reopened = h.reopen();
    assert!(reopened
        .registry
        .recover_runs()
        .unwrap()
        .requeued
        .is_empty());
    assert_eq!(
        bytes,
        reopened
            .registry
            .load_run_artifact(id, "report.json")
            .unwrap()
    );
}
#[tokio::test]
async fn interrupted_analysis_requeues_same_inputs_and_completed_projection_finalizes_without_reexecution(
) {
    let h = Harness::new(0);
    let p = project(&h);
    let accepted = submit(&h, &p, &migration_parts()).await;
    let id = accepted["run_id"].as_str().unwrap();
    assert!(h.state.registry.begin_run(id).unwrap());
    let restarted = h.reopen();
    assert_eq!(
        restarted.registry.recover_runs().unwrap().requeued,
        vec![id]
    );
    assert!(restarted.registry.begin_run(id).unwrap());
    let record = restarted.registry.load_run(id).unwrap();
    let result = worker::run_isolated(
        &restarted.registry,
        &record,
        &restarted.config.worker_binary,
    )
    .unwrap();
    // Crash after immutable output commit but before terminal metadata replacement.
    let reference = restarted
        .registry
        .artifacts()
        .put(
            eplyx_server::artifacts::ArtifactClass::Document,
            &serde_json::to_vec(&result).unwrap(),
        )
        .unwrap()
        .reference;
    restarted
        .registry
        .storage()
        .write_json(
            &restarted
                .registry
                .storage()
                .run_dir(id)
                .unwrap()
                .join("completed-projection.json"),
            &reference,
        )
        .unwrap();
    let recovered = restarted.registry.recover_runs().unwrap();
    assert_eq!(recovered.finalized, vec![id]);
    let done = restarted.registry.load_run(id).unwrap();
    assert_eq!(done.attempts.len(), 2);
    assert_eq!(done.status, RunStatus::Passed);
    assert!(restarted
        .registry
        .finish_run(
            id,
            RunOutcome::Analytical {
                projection: Box::new(result)
            }
        )
        .is_err());
}
#[tokio::test]
async fn inconsistent_or_online_migration_inputs_never_enter_the_queue() {
    let h = Harness::new(0);
    let p = project(&h);
    let parts = migration_parts();
    for name in ["candidate", "state_artifact"] {
        let mut bad = parts.clone();
        bad.iter_mut().find(|(n, _)| *n == name).unwrap().1.push(1);
        let refs: Vec<_> = bad.iter().map(|(n, b)| (*n, b.as_slice())).collect();
        let (code, _) = h.submit_parts(&p, OPERATOR, &refs).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }
    let mut bad = parts;
    let state = bad.iter_mut().find(|(n, _)| *n == "state_input").unwrap();
    let mut value: Value = serde_json::from_slice(&state.1).unwrap();
    value["config"]["state"] = json!({"kind":"mainnet_capture"});
    state.1 = serde_json::to_vec(&value).unwrap();
    let refs: Vec<_> = bad.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    assert_eq!(
        h.submit_parts(&p, OPERATOR, &refs).await.0,
        StatusCode::BAD_REQUEST
    );
    assert!(h.state.registry.project_run_ids(&p).unwrap().is_empty());
}
#[tokio::test]
async fn lifecycle_completes_without_a_deployment_gate_and_matches_local_engine_bytes() {
    use eplyx_engine::{
        change::ChangeSpec,
        lifecycle::{policy::LifecycleScenario, LifecycleSnapshot},
    };
    let h = Harness::new(1);
    let p = project(&h);
    let root = eplyx_engine::lifecycle::artifact::reference_root();
    let snapshot = LifecycleSnapshot::load(&root.join("snapshots/spacex-exposure.json")).unwrap();
    let scenario = LifecycleScenario::load(&root.join("scenarios/spacex-transition.json")).unwrap();
    let spec = ChangeSpec::lifecycle(&scenario).unwrap();
    let before = scenario.policy.effective_at - chrono::Duration::seconds(1);
    let at = scenario.policy.effective_at + chrono::Duration::seconds(1);
    let mut parts = vec![
        ("change_spec", spec.to_document().unwrap().into_bytes()),
        ("snapshot", snapshot.to_json().unwrap().into_bytes()),
        (
            "scenario",
            eplyx_engine::canonical::document(&scenario)
                .unwrap()
                .into_bytes(),
        ),
        (
            "analysis_options",
            serde_json::to_vec(&json!({"before":before,"at":at})).unwrap(),
        ),
    ];
    let evidence: std::collections::BTreeMap<_, _> = scenario
        .sources
        .iter()
        .filter_map(|source| {
            source.artifact.as_ref().map(|path| {
                (
                    source.id.clone(),
                    eplyx_engine::cloud::contract::Artifact::new(
                        eplyx_engine::lifecycle::artifact::read_relative(
                            &root.join("scenarios"),
                            path,
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                )
            })
        })
        .collect();
    parts.push(("lifecycle_evidence", serde_json::to_vec(&evidence).unwrap()));
    let accepted = submit(&h, &p, &parts).await;
    let id = accepted["run_id"].as_str().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        let status = h.state.registry.load_run(id).unwrap().status;
        if status.is_terminal() {
            assert_eq!(status, RunStatus::Completed);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "lifecycle worker did not finish"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let record = h.state.registry.load_run(id).unwrap();
    assert_eq!(record.exit_code, Some(0));
    assert!(record.candidate_artifact.is_none());
    assert!(record.bundle_sha256.is_empty());
    let local=eplyx_engine::canonical::document(&json!({"schema_version":1,"change":spec.bind_lifecycle(&scenario).unwrap(),"impact":spec.compare_lifecycle(&snapshot,&scenario,before,at).unwrap(),"limitations":["Declared lifecycle policy, not proof of issuer eligibility, conversion or redemption."]})).unwrap();
    assert_eq!(
        h.state
            .registry
            .load_run_artifact(id, "report.json")
            .unwrap(),
        local.into_bytes()
    );
    let mut projection = h.state.registry.analytical_projection(&record).unwrap();
    projection.run_id = eplyx_server::ids::run();
    assert!(h
        .state
        .registry
        .verify_hosted_projection(&record, &projection)
        .is_err());
}

#[tokio::test]
async fn captured_program_recipes_require_exact_portable_bytes_and_execute_offline() {
    let h = Harness::new(1);
    let p = project(&h);
    let parts = migration_parts_with_programs(true);
    let missing: Vec<_> = parts
        .iter()
        .filter(|(n, _)| *n != "pinned_program_capture")
        .map(|(n, b)| (*n, b.as_slice()))
        .collect();
    assert_eq!(
        h.submit_parts(&p, OPERATOR, &missing).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut bad = parts.clone();
    bad.iter_mut()
        .find(|(n, _)| *n == "pinned_program_capture")
        .unwrap()
        .1
        .push(1);
    let refs: Vec<_> = bad.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    assert_eq!(
        h.submit_parts(&p, OPERATOR, &refs).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut bundled = migration_parts();
    bundled.push(
        parts
            .iter()
            .find(|(n, _)| *n == "pinned_program_capture")
            .unwrap()
            .clone(),
    );
    let refs: Vec<_> = bundled.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    assert_eq!(
        h.submit_parts(&p, OPERATOR, &refs).await.0,
        StatusCode::BAD_REQUEST
    );
    assert!(h.state.registry.project_run_ids(&p).unwrap().is_empty());
    let accepted = submit(&h, &p, &parts).await;
    let id = accepted["run_id"].as_str().unwrap();
    assert_eq!(
        h.wait_until(id, RunStatus::is_terminal).await,
        RunStatus::Passed
    );
    let record = h.state.registry.load_run(id).unwrap();
    assert!(record.report_available);
    let scratch = tempfile::tempdir().unwrap();
    worker::stage(
        &h.state.registry,
        &h.state.registry.hosted_input(&record).unwrap(),
        scratch.path(),
    )
    .unwrap();
    assert_eq!(
        hash_bytes(&std::fs::read(scratch.path().join("pinned-programs.capture.json")).unwrap()),
        eplyx_engine::migration::fixture::PROGRAM_CAPTURE_SHA256
    );
}
