//! Phase P2: a run is reconstructible from durable identities, and survives
//! the process that accepted it.
//!
//! ```text
//! pinned bundle + stored ChangeSpec + content-addressed candidate artefact
//! ```
//!
//! "Restart" here is what a restart actually is to this service: a second
//! `AppState` over the same data volume, with none of the first one's memory,
//! reconciling what it finds with `recover_runs` and resuming it with
//! `worker::resume`. The first state's workers are parked behind a zero-permit
//! semaphore, so they never execute — the same as a process that has died.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use eplyx_engine::change::{Activation, ChangeSpec};
use eplyx_server::registry::{AttemptEnd, RunStatus, MAX_EXECUTION_ATTEMPTS};
use eplyx_server::worker;
use serde_json::{json, Value};

fn sha(bytes: &[u8]) -> String {
    eplyx_engine::replay::hash_bytes(bytes)
}

fn program_count(harness: &Harness) -> usize {
    std::fs::read_dir(harness.state.registry.artifacts().root().join("programs"))
        .expect("programs")
        .count()
}

async fn accepted(harness: &Harness, project_id: &str, token: &str) -> Value {
    let (status, body) = harness.submit_check(project_id, token).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body
}

fn id_of(body: &Value) -> String {
    body["run_id"].as_str().expect("run id").to_string()
}

/// What a fresh process does at startup, minus the socket.
fn restart(harness: &Harness) -> (eplyx_server::api::Shared, eplyx_server::registry::Recovery) {
    let state = harness.reopen();
    let recovery = state.registry.recover_runs().expect("recover");
    worker::resume(&state, &recovery.requeued);
    (state, recovery)
}

async fn wait_terminal(state: &eplyx_server::api::Shared, run_id: &str) -> RunStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let status = state.registry.load_run(run_id).expect("run").status;
        if status.is_terminal() {
            return status;
        }
        assert!(std::time::Instant::now() < deadline, "stuck at {status:?}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The report a local `eplyx ci check --candidate` produces, as bytes.
fn local_report() -> Vec<u8> {
    let scratch = tempfile::tempdir().expect("scratch");
    let bundle = build_bundle(&scratch.path().join("built"), &committed_record());
    let candidate = scratch.path().join("candidate.so");
    std::fs::write(&candidate, candidate_bytes()).expect("candidate");
    let report = eplyx_engine::ci::check(&bundle, &candidate, None).expect("local");
    let mut bytes = serde_json::to_vec_pretty(&report).expect("json");
    bytes.push(b'\n');
    bytes
}

// ------------------------------------------------------------- the store

/// 1, 2, 15. One object per distinct content, shared by every run and every
/// change spec that names it.
#[tokio::test]
async fn identical_candidates_are_stored_once_and_shared_by_distinct_changes() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let first = accepted(&harness, &project_id, &token).await;
    let second = accepted(&harness, &project_id, &token).await;
    assert_eq!(first["candidate_artifact"], second["candidate_artifact"]);
    assert_eq!(program_count(&harness), 1, "identical bytes stored twice");

    // A different proposal over the same bytes: another change id, the same
    // artefact.
    let mut spec = ChangeSpec::program_upgrade(&committed_record().program_id, &candidate_bytes());
    spec.activation = Some(Activation {
        slot: Some(9),
        unix_timestamp: None,
    });
    let (status, explicit) = harness
        .submit_parts(
            &project_id,
            &token,
            &[
                ("change_spec", spec.to_document().expect("doc").as_bytes()),
                ("candidate", &candidate_bytes()),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{explicit}");
    assert_ne!(
        explicit["change"]["change_spec_id"],
        first["change"]["change_spec_id"]
    );
    assert_eq!(explicit["candidate_artifact"], first["candidate_artifact"]);
    assert_eq!(program_count(&harness), 1);

    // Different bytes are a different artefact.
    let other = std::fs::read(BASELINE).expect("baseline");
    let (status, different) = harness
        .submit_parts(&project_id, &token, &[("candidate", &other)])
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{different}");
    assert_eq!(
        different["candidate_artifact"]["sha256"],
        sha(&other).as_str()
    );
    assert_eq!(program_count(&harness), 2);
}

/// Concurrent submissions of the same bytes all succeed and agree.
#[tokio::test]
async fn concurrent_identical_uploads_are_safe() {
    let harness = Arc::new(Harness::new(0));
    let (project_id, token) = ready_project(&harness, "Example").await;
    let tasks: Vec<_> = (0..6)
        .map(|_| {
            let harness = Arc::clone(&harness);
            let (project_id, token) = (project_id.clone(), token.clone());
            tokio::spawn(async move { harness.submit_check(&project_id, &token).await })
        })
        .collect();
    let mut artifacts = Vec::new();
    for task in tasks {
        let (status, body) = task.await.expect("task");
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        artifacts.push(body["candidate_artifact"].clone());
    }
    assert!(artifacts.iter().all(|a| *a == artifacts[0]));
    assert_eq!(program_count(&harness), 1);
    let tmp = harness.state.registry.artifacts().root().join("tmp");
    assert_eq!(std::fs::read_dir(tmp).expect("tmp").count(), 0);
}

/// 5, 6. A run is never created unless its artefact is already durable.
#[tokio::test]
async fn no_run_exists_whose_artifact_was_not_stored() {
    use std::os::unix::fs::PermissionsExt;
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let programs = harness.state.registry.artifacts().root().join("programs");
    std::fs::set_permissions(&programs, std::fs::Permissions::from_mode(0o555)).expect("ro");

    let (status, body) = harness.submit_check(&project_id, &token).await;
    let spec = ChangeSpec::program_upgrade(&committed_record().program_id, &candidate_bytes());
    let (explicit_status, _) = harness
        .submit_parts(
            &project_id,
            &token,
            &[
                ("change_spec", spec.to_document().expect("doc").as_bytes()),
                ("candidate", &candidate_bytes()),
            ],
        )
        .await;
    std::fs::set_permissions(&programs, std::fs::Permissions::from_mode(0o755)).expect("rw");

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(body["error"]
        .as_str()
        .is_some_and(|e| e.contains("storing the candidate")));
    assert_eq!(explicit_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        harness
            .state
            .registry
            .list_run_ids()
            .expect("ids")
            .is_empty(),
        "a run was accepted without its artifact"
    );

    // And when the store works, the artefact exists and verifies at the 202.
    let body = accepted(&harness, &project_id, &token).await;
    let reference = serde_json::from_value(body["candidate_artifact"].clone()).expect("ref");
    assert_eq!(
        harness
            .state
            .registry
            .artifacts()
            .get_program(&reference)
            .expect("verifies"),
        candidate_bytes()
    );
}

/// 7. Only the artefact the run names, verified, is ever executed. Anything
///    lying around in a run directory under a plausible name is ignored.
#[tokio::test]
async fn the_worker_executes_the_artifact_not_a_file_in_the_run() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let body = accepted(&harness, &project_id, &token).await;
    let id = id_of(&body);
    let impostor = std::fs::read(BASELINE).expect("other bytes");
    let work = harness.work_dir(&id);
    std::fs::create_dir_all(work.join("artifacts/programs")).expect("dirs");
    std::fs::write(work.join("candidate.so"), &impostor).expect("plant");
    std::fs::write(
        work.join("artifacts/programs")
            .join(sha(&candidate_bytes())),
        &impostor,
    )
    .expect("plant");

    harness.state.runs.add_permits(1);
    harness.wait_until(&id, RunStatus::is_terminal).await;
    let (status, bytes) = harness
        .get_bytes(&format!("/v1/runs/{id}/report.json"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        String::from_utf8_lossy(&local_report()),
        "the worker executed something other than the artifact"
    );
}

// --------------------------------------------------------------- restart

/// 8, 10. A queued run survives a restart and completes as the same run.
#[tokio::test]
async fn a_queued_run_survives_a_restart() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);

    let (state, recovery) = restart(&harness);
    assert_eq!(recovery.requeued, vec![id.clone()]);
    assert!(wait_terminal(&state, &id).await.is_terminal());

    let run = state.registry.load_run(&id).expect("run");
    assert!(run.report_available, "{run:?}");
    assert_eq!(run.attempts.len(), 1);
    assert_eq!(run.attempts[0].end, Some(AttemptEnd::Completed));
    let (_, listed) = harness
        .get_on(
            Arc::clone(&state),
            &format!("/v1/projects/{project_id}/runs"),
            &token,
        )
        .await;
    assert_eq!(
        listed["runs"].as_array().expect("runs").len(),
        1,
        "a duplicate run appeared"
    );
    let (_, bytes) = harness
        .get_bytes(&format!("/v1/runs/{id}/report.json"), &token)
        .await;
    assert_eq!(
        bytes,
        local_report(),
        "hosted differs from local after recovery"
    );
}

/// 9, 10. A run that was executing when the process died is not assumed to
///    have finished or failed. Its attempt is recorded as interrupted, it goes
///    back to the queue, and a second attempt of the same run completes it.
#[tokio::test]
async fn an_interrupted_run_is_retried_as_the_same_run() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    assert!(harness.state.registry.begin_run(&id).expect("claim"));

    let (state, recovery) = restart(&harness);
    assert_eq!(recovery.requeued, vec![id.clone()]);
    assert!(wait_terminal(&state, &id).await.is_terminal());

    let run = state.registry.load_run(&id).expect("run");
    let ends: Vec<_> = run.attempts.iter().map(|a| a.end).collect();
    assert_eq!(
        ends,
        vec![Some(AttemptEnd::Interrupted), Some(AttemptEnd::Completed)]
    );
    assert_eq!(run.attempts[1].attempt, 2);
    assert!(run.report_available);
    assert_eq!(project_runs(&harness, &project_id), vec![id]);
}

/// A run that keeps dying is eventually not retried.
#[tokio::test]
async fn a_run_interrupted_too_often_stops_being_retried() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    for _ in 0..MAX_EXECUTION_ATTEMPTS {
        let state = harness.reopen();
        assert!(state.registry.begin_run(&id).expect("claim"));
        let recovery = state.registry.recover_runs().expect("recover");
        let _ = recovery;
    }
    let run = harness.state.registry.load_run(&id).expect("run");
    assert_eq!(run.status, RunStatus::ExecutionError, "{run:?}");
    assert_eq!(run.exit_code, None);
    assert_eq!(run.attempts.len(), MAX_EXECUTION_ATTEMPTS);
    assert!(run
        .attempts
        .iter()
        .all(|attempt| attempt.end == Some(AttemptEnd::Interrupted)));
}

/// 11. Two workers for one run execute it once.
#[tokio::test]
async fn a_run_is_executed_once_however_often_it_is_enqueued() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    // The submission already spawned one; add two more, then open the gate.
    worker::spawn(Arc::clone(&harness.state), id.clone());
    worker::resume(&harness.state, std::slice::from_ref(&id));
    harness.state.runs.add_permits(3);
    harness.wait_until(&id, RunStatus::is_terminal).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let run = harness.state.registry.load_run(&id).expect("run");
    assert_eq!(run.attempts.len(), 1, "{:?}", run.attempts);
}

/// A crash between writing the report and recording it is reconciled from the
/// report, which is not recomputed — and a report that is not a complete,
/// verified result for this run is never trusted.
#[tokio::test]
async fn a_report_written_before_the_crash_is_recorded_not_recomputed() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    assert!(harness.state.registry.begin_run(&id).expect("claim"));
    // What finish_run writes first. No report.md, no terminal metadata.
    std::fs::write(harness.run_file(&id, "report.json"), local_report()).expect("report");

    let (state, recovery) = restart(&harness);
    assert_eq!(recovery.finalized, vec![id.clone()]);
    assert!(recovery.requeued.is_empty(), "a complete result was re-run");
    let run = state.registry.load_run(&id).expect("run");
    assert!(run.status.is_terminal());
    assert!(run.report_available);
    assert_eq!(
        run.exit_code,
        Some(2),
        "no semantic coverage for the fixture program"
    );
    assert_eq!(run.attempts.len(), 1);
    assert_eq!(run.attempts[0].end, Some(AttemptEnd::FinalizedOnRecovery));
    let (_, bytes) = harness
        .get_bytes(&format!("/v1/runs/{id}/report.json"), &token)
        .await;
    assert_eq!(bytes, local_report());
    let (status, markdown) = harness
        .get_bytes(&format!("/v1/runs/{id}/report.md"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!markdown.is_empty(), "the markdown was not re-rendered");
}

#[tokio::test]
async fn a_partial_or_foreign_report_is_never_trusted_on_recovery() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let local = local_report();

    // Truncated mid-write.
    let truncated = id_of(&accepted(&harness, &project_id, &token).await);
    assert!(harness.state.registry.begin_run(&truncated).expect("claim"));
    std::fs::write(
        harness.run_file(&truncated, "report.json"),
        &local[..local.len() / 2],
    )
    .expect("partial");

    // Complete and valid, but about a different change.
    let mut foreign: Value = serde_json::from_slice(&local).expect("json");
    foreign["change"]["change_spec_id"] = json!("f".repeat(64));
    let other = id_of(&accepted(&harness, &project_id, &token).await);
    assert!(harness.state.registry.begin_run(&other).expect("claim"));
    std::fs::write(
        harness.run_file(&other, "report.json"),
        serde_json::to_vec_pretty(&foreign).expect("json"),
    )
    .expect("foreign");

    let state = harness.reopen();
    let recovery = state.registry.recover_runs().expect("recover");
    assert!(recovery.finalized.is_empty(), "{recovery:?}");
    assert_eq!(recovery.requeued, vec![truncated.clone(), other.clone()]);
    for id in [&truncated, &other] {
        assert!(
            !harness.run_file(id, "report.json").exists(),
            "an untrusted report was kept"
        );
        let run = state.registry.load_run(id).expect("run");
        assert_eq!(run.status, RunStatus::Queued);
        assert!(!run.report_available);
    }
}

/// 13. A completed run is left exactly as it was.
#[tokio::test]
async fn a_completed_report_survives_a_restart_unchanged() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    harness.wait_until(&id, RunStatus::is_terminal).await;
    let before = std::fs::read(harness.run_file(&id, "metadata.json")).expect("metadata");
    let (_, report_before) = harness
        .get_bytes(&format!("/v1/runs/{id}/report.json"), &token)
        .await;

    let (state, recovery) = restart(&harness);
    assert!(recovery.requeued.is_empty() && recovery.finalized.is_empty());
    assert_eq!(
        std::fs::read(harness.run_file(&id, "metadata.json")).expect("metadata"),
        before
    );
    let (status, report_after) = harness
        .get_on(
            Arc::clone(&state),
            &format!("/v1/runs/{id}/report.json"),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        report_after,
        serde_json::from_slice::<Value>(&report_before).expect("json")
    );
}

// ------------------------------------------------------------ fail closed

/// 12. An artefact that vanished while the run waited, across a restart.
#[tokio::test]
async fn a_missing_artifact_after_restart_is_an_execution_error() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    std::fs::remove_file(harness.artifact_path(&sha(&candidate_bytes()))).expect("remove");
    let (state, _) = restart(&harness);
    assert_eq!(wait_terminal(&state, &id).await, RunStatus::ExecutionError);
    let run = state.registry.load_run(&id).expect("run");
    assert_eq!(
        run.exit_code, None,
        "an infrastructure fault became a verdict"
    );
    assert!(run
        .detail
        .as_deref()
        .is_some_and(|d| d.contains("not held")));
}

#[tokio::test]
async fn a_corrupted_artifact_is_an_execution_error() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    let path = harness.artifact_path(&sha(&candidate_bytes()));
    // Same length, one byte different: only the hash can tell.
    let mut bytes = std::fs::read(&path).expect("artifact");
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&path, bytes).expect("corrupt");
    harness.state.runs.add_permits(1);
    assert_eq!(
        harness.wait_until(&id, RunStatus::is_terminal).await,
        RunStatus::ExecutionError
    );
    let run = harness.state.registry.load_run(&id).expect("run");
    assert!(run
        .detail
        .as_deref()
        .is_some_and(|d| d.contains("does not match its address")));
    // The artefact endpoint refuses to vouch for it either.
    let (status, _) = harness
        .get(
            &format!(
                "/v1/projects/{project_id}/artifacts/{}",
                sha(&candidate_bytes())
            ),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn an_unavailable_pinned_bundle_is_an_execution_error() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let body = accepted(&harness, &project_id, &token).await;
    let id = id_of(&body);
    let bundle = harness
        .state
        .registry
        .storage()
        .bundle_path(body["bundle_sha256"].as_str().expect("sha"))
        .expect("path");
    std::fs::remove_dir_all(bundle).expect("remove");
    harness.state.runs.add_permits(1);
    assert_eq!(
        harness.wait_until(&id, RunStatus::is_terminal).await,
        RunStatus::ExecutionError
    );
    let run = harness.state.registry.load_run(&id).expect("run");
    assert_eq!(run.exit_code, None);
    assert!(run
        .detail
        .as_deref()
        .is_some_and(|d| d.contains("pinned bundle")));
}

#[tokio::test]
async fn altered_expectations_are_an_execution_error() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (status, body) = harness
        .submit_parts(
            &project_id,
            &token,
            &[
                ("candidate", &candidate_bytes()),
                (
                    "expected_changes",
                    b"version = 1\nsemantic_schema_version = 2\n",
                ),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let id = id_of(&body);
    std::fs::write(
        harness.run_file(&id, "expected-changes.toml"),
        "version = 1\nsemantic_schema_version = 2\n# edited\n",
    )
    .expect("edit");
    harness.state.runs.add_permits(1);
    assert_eq!(
        harness.wait_until(&id, RunStatus::is_terminal).await,
        RunStatus::ExecutionError
    );
}

/// The registry's artefact must be the spec's candidate. Pointing a run at a
/// different object the store also holds is a service fault, never a run of
/// those other bytes and never a configuration verdict about them.
#[tokio::test]
async fn a_run_pointed_at_another_held_artifact_is_an_execution_error() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let other = std::fs::read(BASELINE).expect("baseline");
    let (status, _) = harness
        .submit_parts(&project_id, &token, &[("candidate", &other)])
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    let path = harness.run_file(&id, "metadata.json");
    let mut metadata: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
    metadata["candidate_artifact"] = json!({ "sha256": sha(&other), "len": other.len() });
    std::fs::write(&path, metadata.to_string()).expect("write");

    harness.state.runs.add_permits(2);
    assert_eq!(
        harness.wait_until(&id, RunStatus::is_terminal).await,
        RunStatus::ExecutionError
    );
    let run = harness.state.registry.load_run(&id).expect("run");
    assert_eq!(run.exit_code, None);
    assert!(run
        .detail
        .as_deref()
        .is_some_and(|d| d.contains("is not the candidate its change spec names")));
}

/// 14. Every record of what ran names the same change and the same bytes.
#[tokio::test]
async fn registry_spec_artifact_and_report_all_agree() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    harness.wait_until(&id, RunStatus::is_terminal).await;
    let registry = &harness.state.registry;
    let run = registry.load_run(&id).expect("run");
    let change = run.change.clone().expect("change");
    let artifact = run.candidate_artifact.clone().expect("artifact");
    let spec = registry.load_change_spec(&id, &change).expect("spec");
    let (_, report) = harness
        .get(&format!("/v1/runs/{id}/report.json"), &token)
        .await;

    assert_eq!(spec.id().expect("id"), change.change_spec_id);
    assert_eq!(
        report["change"]["change_spec_id"],
        change.change_spec_id.as_str()
    );
    assert!(artifact.matches(spec.candidate()));
    assert_eq!(report["candidate"]["sha256"], artifact.sha256.as_str());
    assert_eq!(report["candidate"]["len"], artifact.len);
    assert_eq!(
        eplyx_server::artifacts::ArtifactRef::of(
            &registry.artifacts().get_program(&artifact).expect("bytes")
        ),
        artifact
    );
}

// ------------------------------------------------------------------ reuse

/// Acceptance 7: a proposal can be analysed again without re-uploading bytes
/// the project already supplied — and only bytes that project supplied.
#[tokio::test]
async fn a_retained_candidate_is_reused_without_re_upload() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (other_project, other_token) = ready_project(&harness, "Other").await;
    let spec = ChangeSpec::program_upgrade(&committed_record().program_id, &candidate_bytes());
    let document = spec.to_document().expect("doc");

    // Not yet supplied: the spec alone has nothing to execute.
    let (status, body) = harness
        .submit_parts(&project_id, &token, &[("change_spec", document.as_bytes())])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["exit_code"], 2);
    let artifact = format!(
        "/v1/projects/{project_id}/artifacts/{}",
        sha(&candidate_bytes())
    );
    assert_eq!(
        harness.get(&artifact, &token).await.0,
        StatusCode::NOT_FOUND
    );

    accepted(&harness, &project_id, &token).await;
    let (status, held) = harness.get(&artifact, &token).await;
    assert_eq!(status, StatusCode::OK, "{held}");
    assert_eq!(held["sha256"], sha(&candidate_bytes()).as_str());
    assert_eq!(held["len"], candidate_bytes().len() as u64);
    assert_eq!(held["retained"], true);
    assert!(!held.to_string().contains("/data"), "a storage path leaked");

    let (status, reused) = harness
        .submit_parts(&project_id, &token, &[("change_spec", document.as_bytes())])
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{reused}");
    assert_eq!(
        reused["change"]["change_spec_id"],
        spec.id().expect("id").as_str()
    );
    assert_eq!(program_count(&harness), 1);

    // The store is global; what a project may name is not. Another project
    // neither learns the bytes are held nor gets to run them.
    let foreign = format!(
        "/v1/projects/{other_project}/artifacts/{}",
        sha(&candidate_bytes())
    );
    assert_eq!(
        harness.get(&foreign, &other_token).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, _) = harness
        .submit_parts(
            &other_project,
            &other_token,
            &[("change_spec", document.as_bytes())],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    for bad in ["../../x", &"A".repeat(64)] {
        let (status, _) = harness
            .get(
                &format!("/v1/projects/{project_id}/artifacts/{bad}"),
                &token,
            )
            .await;
        assert_ne!(status, StatusCode::OK, "{bad}");
    }
}

// ----------------------------------------------------------------- legacy

/// 16, 23. A run accepted before durable artefacts, still queued at restart,
///     is not resumed with an invented candidate.
#[tokio::test]
async fn a_pre_durable_queued_run_is_not_resumed_with_invented_inputs() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let id = id_of(&accepted(&harness, &project_id, &token).await);
    // The P1 shape: a change, but no durable artefact identity.
    let path = harness.run_file(&id, "metadata.json");
    let mut metadata: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("json");
    metadata
        .as_object_mut()
        .expect("object")
        .remove("candidate_artifact");
    std::fs::write(&path, metadata.to_string()).expect("write");

    let (state, recovery) = restart(&harness);
    assert_eq!(recovery.failed, vec![id.clone()]);
    let run = state.registry.load_run(&id).expect("run");
    assert_eq!(run.status, RunStatus::ExecutionError);
    assert!(run
        .detail
        .as_deref()
        .is_some_and(|d| d.contains("before candidates were stored durably")));
    let (status, _) = harness
        .get_on(Arc::clone(&state), &format!("/v1/runs/{id}"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
}

// ------------------------------------------------------------------ lock

/// Two processes over one volume would both re-enqueue its runs. The second
/// is refused instead.
#[tokio::test]
async fn one_process_holds_the_data_directory() {
    let harness = Harness::new(0);
    let storage = harness.state.registry.storage();
    let held = storage.lock_exclusive().expect("first");
    let error = harness
        .reopen()
        .registry
        .storage()
        .lock_exclusive()
        .unwrap_err();
    assert!(format!("{error:#}").contains("another eplyx-server"));
    drop(held);
    harness
        .reopen()
        .registry
        .storage()
        .lock_exclusive()
        .expect("free again");
}

fn project_runs(harness: &Harness, project_id: &str) -> Vec<String> {
    harness
        .state
        .registry
        .project_run_ids(project_id)
        .expect("ids")
}
