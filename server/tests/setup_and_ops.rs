//! The guided first check (`/setup`), the operator view of the queue
//! (`/v1/ops`), and the restored-volume check (`admin verify-volume`).

mod common;

use axum::http::StatusCode;
use common::*;
use eplyx_server::registry::{RunStatus, MAX_EXECUTION_ATTEMPTS};
use eplyx_server::worker;
use serde_json::Value;

fn step<'a>(setup: &'a Value, id: &str) -> &'a Value {
    setup["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["id"] == id)
        .unwrap_or_else(|| panic!("no step {id} in {setup}"))
}

fn status_of(setup: &Value, id: &str) -> String {
    step(setup, id)["status"].as_str().unwrap().to_owned()
}

async fn wait_terminal(state: &eplyx_server::api::Shared, run_id: &str) -> RunStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let status = state.registry.load_run(run_id).expect("run").status;
        if status.is_terminal() {
            return status;
        }
        assert!(std::time::Instant::now() < deadline, "stuck at {status:?}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn setup_walks_a_new_project_to_its_first_check() {
    let harness = Harness::new(1);
    let project = harness.create_project("Guided").await;
    let path = format!("/v1/projects/{project}/setup");

    // Nothing but the project: evidence is the first thing missing.
    let (code, setup) = harness.get(&path, OPERATOR).await;
    assert_eq!(code, StatusCode::OK, "{setup}");
    assert_eq!(setup["schema_version"], 1);
    assert_eq!(setup["kind"], "program_upgrade");
    assert_eq!(setup["ready_for_first_check"], false);
    assert_eq!(setup["first_check_complete"], false);
    assert_eq!(setup["next_step"], "bundle_registered");
    assert_eq!(status_of(&setup, "project_enabled"), "done");
    // The fixture program has no semantic adapter: satisfied, with a warning.
    assert_eq!(status_of(&setup, "upgrade_target"), "attention");
    assert_eq!(status_of(&setup, "bundle_registered"), "todo");
    assert_eq!(status_of(&setup, "bundle_active"), "blocked");
    // Independent of the bundle: never blocked by it.
    assert_eq!(status_of(&setup, "ci_token"), "todo");
    assert_eq!(status_of(&setup, "expectations"), "optional");
    assert_eq!(status_of(&setup, "first_check"), "blocked");
    let register = step(&setup, "bundle_registered")["actions"][1]["command"]
        .as_str()
        .unwrap();
    assert!(register.contains(&project), "{register}");
    assert_eq!(
        setup["repository"]["variables"]["EPLYX_PROJECT_ID"],
        project.as_str()
    );

    // Registered but not active: activation is named with the exact bundle.
    let bundle_id = harness.upload_bundle(&project).await;
    let (_, setup) = harness.get(&path, OPERATOR).await;
    assert_eq!(status_of(&setup, "bundle_registered"), "done");
    assert_eq!(status_of(&setup, "bundle_active"), "todo");
    let activate = step(&setup, "bundle_active")["actions"][0]["command"]
        .as_str()
        .unwrap();
    assert!(activate.contains(&bundle_id), "{activate}");
    assert_eq!(setup["next_step"], "bundle_active");

    let (code, _) = harness
        .post_json(
            &format!("/v1/projects/{project}/bundles/{bundle_id}/activate"),
            OPERATOR,
            serde_json::json!({}),
        )
        .await;
    assert_eq!(code, StatusCode::OK);
    let token = harness.create_token(&project, "CI").await;
    let (_, setup) = harness.get(&path, OPERATOR).await;
    assert_eq!(status_of(&setup, "bundle_active"), "done");
    assert_eq!(status_of(&setup, "ci_token"), "done");
    assert_eq!(step(&setup, "ci_token")["evidence"]["live_tokens"], 1);
    assert_eq!(setup["ready_for_first_check"], true);
    assert_eq!(setup["next_step"], "first_check");
    assert_eq!(status_of(&setup, "first_check"), "todo");

    // The project's own CI token may read its checklist; another may not.
    let (code, own) = harness.get(&path, &token).await;
    assert_eq!(code, StatusCode::OK, "{own}");
    assert_eq!(
        step(&own, "ci_token")["evidence"]["caller_is_project_token"],
        true
    );
    let other = harness.create_project("Other").await;
    let other_token = harness.create_token(&other, "CI").await;
    let (code, _) = harness.get(&path, &other_token).await;
    assert_eq!(code, StatusCode::UNAUTHORIZED);

    // The first check completes the checklist, whatever its verdict.
    let (code, accepted) = harness.submit_check(&project, &token).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let run_id = accepted["run_id"].as_str().unwrap().to_owned();
    let ended = wait_terminal(&harness.state, &run_id).await;
    let (_, setup) = harness.get(&path, OPERATOR).await;
    if ended == RunStatus::ExecutionError {
        assert_eq!(status_of(&setup, "first_check"), "attention");
    } else {
        assert_eq!(status_of(&setup, "first_check"), "done");
        assert_eq!(setup["first_check_complete"], true);
        assert_eq!(setup["next_step"], Value::Null);
    }
    assert_eq!(
        step(&setup, "first_check")["evidence"]["run_id"],
        run_id.as_str()
    );
}

#[tokio::test]
async fn ops_shows_queue_retries_and_worker_failures_to_the_operator_only() {
    // Zero permits: accepted runs stay queued, as on a saturated host.
    let harness = Harness::new(0);
    let (project, token) = ready_project(&harness, "Ops").await;
    let mut ids = Vec::new();
    for _ in 0..2 {
        let (code, body) = harness.submit_check(&project, &token).await;
        assert_eq!(code, StatusCode::ACCEPTED, "{body}");
        ids.push(body["run_id"].as_str().unwrap().to_owned());
    }

    let (code, ops) = harness.get("/v1/ops", OPERATOR).await;
    assert_eq!(code, StatusCode::OK, "{ops}");
    assert_eq!(ops["schema_version"], 1);
    assert_eq!(ops["queue"]["queued"], 2);
    assert_eq!(ops["queue"]["running"], 0);
    assert!(ops["queue"]["oldest_queued_age_seconds"].is_u64());
    assert_eq!(ops["workers"]["max_concurrent_runs"], 2);
    assert_eq!(ops["outcomes"]["created"], 2);
    // Nothing has started: no waits measured, which is absent, not zero.
    assert_eq!(ops["wait"]["count"], 0);
    assert_eq!(ops["wait"]["p50_seconds"], Value::Null);

    // Project credentials see none of this.
    let (code, _) = harness.get("/v1/ops", &token).await;
    assert_eq!(code, StatusCode::UNAUTHORIZED);
    let (code, _) = harness.get("/v1/ops?window_hours=0", OPERATOR).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);

    // One run keeps dying until it is given up on.
    for _ in 0..MAX_EXECUTION_ATTEMPTS {
        let state = harness.reopen();
        assert!(state.registry.begin_run(&ids[0]).expect("claim"));
        let recovery = state.registry.recover_runs().expect("recover");
        eplyx_server::ops::record_recovery(&state.registry, &recovery, 0).expect("record");
    }
    // A restart then executes the other one.
    let state = harness.reopen();
    let recovery = state.registry.recover_runs().expect("recover");
    eplyx_server::ops::record_recovery(&state.registry, &recovery, 0).expect("record");
    assert_eq!(recovery.requeued, vec![ids[1].clone()]);
    worker::resume(&state, &recovery.requeued);
    wait_terminal(&state, &ids[1]).await;

    let (code, ops) = harness.get_on(state, "/v1/ops", OPERATOR).await;
    assert_eq!(code, StatusCode::OK, "{ops}");
    assert_eq!(ops["queue"]["queued"], 0);
    assert_eq!(ops["outcomes"]["execution_error"], 1);
    assert_eq!(ops["retries"]["exhausted"], 1);
    assert_eq!(
        ops["retries"]["interrupted_attempts"],
        MAX_EXECUTION_ATTEMPTS as u64
    );
    assert_eq!(ops["wait"]["count"], 2);
    let failure = &ops["recent_worker_failures"][0];
    assert_eq!(failure["run_id"], ids[0].as_str());
    assert_eq!(failure["project_id"], project.as_str());
    assert_eq!(failure["attempts"], MAX_EXECUTION_ATTEMPTS as u64);
    assert!(failure["detail"].as_str().unwrap().contains("interrupted"));
    let recoveries = ops["recoveries"].as_array().unwrap();
    assert_eq!(recoveries.len(), MAX_EXECUTION_ATTEMPTS + 1);
    assert!(recoveries
        .iter()
        .any(|record| record["failed"][0] == ids[0].as_str()));
}

#[tokio::test]
async fn verify_volume_names_what_a_mismatched_restore_breaks() {
    let harness = Harness::new(1);
    let (project, token) = ready_project(&harness, "Restore").await;
    let (_, accepted) = harness.submit_check(&project, &token).await;
    let run_id = accepted["run_id"].as_str().unwrap().to_owned();
    wait_terminal(&harness.state, &run_id).await;
    let registry = &harness.state.registry;

    let clean = eplyx_server::ops::verify_volume(registry, Some(std::slice::from_ref(&project)));
    assert!(clean.ok(), "{:?}", clean.problems);
    assert_eq!(clean.projects, 1);
    assert_eq!(clean.runs, 1);
    assert_eq!(clean.artifacts_checked, 1);
    assert_eq!(clean.identity_projects, Some(1));

    // A database newer than the volume: it maps a project the files lack.
    let ahead = eplyx_server::ops::verify_volume(
        registry,
        Some(&[project.clone(), "proj_00000000000000000000000000".into()]),
    );
    assert!(!ahead.ok());
    assert!(ahead.problems[0].contains("proj_00000000000000000000000000"));

    // A volume restored without its artefact store.
    let run = registry.load_run(&run_id).unwrap();
    let candidate = harness.artifact_path(&run.candidate_artifact.unwrap().sha256);
    std::fs::remove_file(&candidate).unwrap();
    let broken = eplyx_server::ops::verify_volume(registry, None);
    assert!(
        broken
            .problems
            .iter()
            .any(|problem| problem.contains(&run_id) && problem.contains("candidate")),
        "{:?}",
        broken.problems
    );
}
