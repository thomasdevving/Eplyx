//! The hosted run lifecycle, driven through the real router.

mod common;

use axum::body::Body;
use axum::http::StatusCode;
use common::*;
use eplyx_server::registry::{RunOutcome, RunStatus};
use serde_json::{json, Value};
use std::sync::Arc;

// ------------------------------------------------------------ acceptance

#[tokio::test]
async fn a_check_is_accepted_without_waiting_for_the_analysis() {
    // No execution capacity at all, so nothing can possibly have run.
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (status, body) = harness.submit_check(&project_id, &token).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["status"], "queued");
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    assert_eq!(body["status_url"], format!("/v1/runs/{run_id}"));
    // No verdict is invented at creation time.
    assert!(body.get("exit_code").is_none(), "{body}");
    assert!(body.get("summary").is_none(), "{body}");
}

#[tokio::test]
async fn the_run_is_recoverable_the_moment_the_caller_has_its_id() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id");

    // This is the promise a 202 makes: the id already resolves.
    let (status, run) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run["status"], "queued");
    assert_eq!(run["exit_code"], Value::Null);
    assert_eq!(run["report_available"], false);
    // Known before execution, because the server resolved it, not the client.
    assert!(run["bundle_sha256"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(run["bundle_id"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(run["candidate_sha256"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
}

#[tokio::test]
async fn a_queued_run_does_not_start_until_capacity_exists() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();

    // Long enough that a run which was going to start would have.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (_, run) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    assert_eq!(run["status"], "queued", "started without a permit");
    assert_eq!(run["started_at_unix_seconds"], Value::Null);

    harness.state.runs.add_permits(1);
    let status = harness.wait_until(&run_id, RunStatus::is_terminal).await;
    assert!(status.is_terminal(), "{status:?}");

    let (_, run) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    // Having passed through running is visible afterwards even when the engine
    // was too quick to catch in the act.
    assert!(run["started_at_unix_seconds"].as_u64().is_some());
    assert!(run["completed_at_unix_seconds"].as_u64().is_some());
}

#[tokio::test]
async fn a_completed_run_carries_a_report_and_the_engine_s_exit_code() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    harness.wait_until(&run_id, RunStatus::is_terminal).await;

    let (_, run) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    assert_eq!(run["status"], "failed", "{run}");
    assert_eq!(run["report_available"], true);
    let exit_code = run["exit_code"].as_u64().expect("an exit code");

    let (status, bytes) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
    let report: Value = serde_json::from_slice(&bytes).expect("canonical json");
    assert_eq!(report["summary"]["exit_code"].as_u64(), Some(exit_code));
    assert_eq!(report["summary"]["passed"], false);

    let (status, markdown) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.md"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!markdown.is_empty());
}

#[tokio::test]
async fn a_report_is_refused_until_one_exists() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();

    for artifact in ["report.json", "report.md"] {
        let (status, body) = harness
            .get(&format!("/v1/runs/{run_id}/{artifact}"), &token)
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{artifact}: {body}");
        // No empty report object stands in for the one that does not exist.
        assert!(body.get("summary").is_none(), "{body}");
        assert!(body["error"].as_str().is_some_and(|e| e.contains("queued")));
    }
}

// ------------------------------------------------------------- lifecycle

#[tokio::test]
async fn a_run_can_only_be_claimed_once() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    let registry = &harness.state.registry;

    assert!(registry.begin_run(&run_id).expect("first claim"));
    assert!(
        !registry.begin_run(&run_id).expect("second claim"),
        "two workers both claimed the same run"
    );
    assert_eq!(
        registry.load_run(&run_id).expect("run").status,
        RunStatus::Running
    );
}

#[tokio::test]
async fn a_terminal_run_never_goes_back_to_running() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    let registry = &harness.state.registry;

    assert!(registry.begin_run(&run_id).expect("claim"));
    registry
        .finish_run(
            &run_id,
            RunOutcome::ExecutionError {
                detail: "deliberate".to_string(),
            },
        )
        .expect("finish");

    assert!(
        !registry.begin_run(&run_id).expect("re-claim"),
        "a terminal run was re-opened"
    );
    assert!(
        registry
            .finish_run(
                &run_id,
                RunOutcome::ExecutionError {
                    detail: "again".to_string()
                }
            )
            .is_err(),
        "a terminal run was finished twice"
    );
    assert_eq!(
        registry.load_run(&run_id).expect("run").status,
        RunStatus::ExecutionError
    );
}

#[tokio::test]
async fn an_execution_error_is_not_a_failed_gate() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    let registry = &harness.state.registry;
    assert!(registry.begin_run(&run_id).expect("claim"));
    registry
        .finish_run(
            &run_id,
            RunOutcome::ExecutionError {
                detail: "the volume went away".to_string(),
            },
        )
        .expect("finish");

    let (_, run) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    assert_eq!(run["status"], "execution_error");
    // No gate result exists, so none is reported. A candidate is not blamed for
    // an infrastructure fault.
    assert_eq!(run["exit_code"], Value::Null);
    assert_eq!(run["report_available"], false);
    assert!(run["detail"].as_str().is_some_and(|d| d.contains("volume")));
}

#[tokio::test]
async fn a_preflight_abort_keeps_its_exit_code_and_has_no_report() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    let registry = &harness.state.registry;
    assert!(registry.begin_run(&run_id).expect("claim"));
    registry
        .finish_run(
            &run_id,
            RunOutcome::PreflightAbort {
                exit_code: eplyx_engine::ci::EXIT_INCOMPATIBLE,
                detail: "bundle was built under another adapter version".to_string(),
            },
        )
        .expect("finish");

    let (_, run) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    // A real verdict that arrived before there was a report to put it in.
    assert_eq!(run["status"], "failed");
    assert_eq!(run["exit_code"], 4);
    assert_eq!(run["report_available"], false);

    let (status, body) = harness
        .get(&format!("/v1/runs/{run_id}/report.json"), &token)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["exit_code"], 4, "the gate code survives the refusal");
}

// -------------------------------------------------------------- recovery

#[tokio::test]
async fn a_reload_recovers_a_run_without_any_browser_state() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();

    // A different service instance over the same volume: no shared memory, no
    // continuation of the request that created the run.
    let reopened = harness.reopen();
    let (status, run) = harness
        .get_on(reopened, &format!("/v1/runs/{run_id}"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run["run_id"], run_id.as_str());
    assert_eq!(run["status"], "queued");
}

#[tokio::test]
async fn a_completed_run_reads_the_same_on_every_later_fetch() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    harness.wait_until(&run_id, RunStatus::is_terminal).await;

    let (_, first) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), &token)
        .await;
    let (_, second) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), &token)
        .await;
    assert_eq!(first, second, "a refetch produced different bytes");

    let reopened = harness.reopen();
    let (status, run) = harness
        .get_on(reopened, &format!("/v1/runs/{run_id}"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run["report_available"], true);
}

#[tokio::test]
async fn runs_interrupted_by_a_restart_are_resolved_rather_than_left_running() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let mut ids = Vec::new();
    for _ in 0..2 {
        let (_, body) = harness.submit_check(&project_id, &token).await;
        ids.push(body["run_id"].as_str().expect("run id").to_string());
    }
    // One queued, one that had already been claimed when the process died.
    harness
        .state
        .registry
        .begin_run(&ids[1])
        .expect("claim the second");

    let restarted = harness.reopen();
    let recovered = restarted
        .registry
        .recover_interrupted_runs()
        .expect("recover");
    assert_eq!(recovered.len(), 2, "{recovered:?}");

    for id in &ids {
        let (_, run) = harness
            .get_on(Arc::clone(&restarted), &format!("/v1/runs/{id}"), &token)
            .await;
        assert_eq!(run["status"], "execution_error", "{run}");
        assert!(run["detail"]
            .as_str()
            .is_some_and(|d| d.contains("restart")));
        // Nothing is left for a client to poll forever.
        assert!(!harness.work_dir(id).exists(), "inputs survived recovery");
    }
}

// ------------------------------------------------------------------ work

#[tokio::test]
async fn uploaded_inputs_outlive_the_request_and_are_cleared_at_the_end() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let expectations = "version = 1\nsemantic_schema_version = 2\n";
    let (content_type, body) = candidate_multipart(&candidate_bytes(), Some(expectations));
    let (status, body) = harness
        .send(
            authed("POST", &format!("/v1/projects/{project_id}/checks"), &token)
                .header("content-type", content_type)
                .body(Body::from(body))
                .expect("request"),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let run_id = body["run_id"].as_str().expect("run id").to_string();

    // The handler has returned. A request-scoped temporary directory would have
    // taken these with it.
    let work = harness.work_dir(&run_id);
    assert!(work.join("candidate.so").exists(), "candidate was deleted");
    assert!(
        work.join("expected-changes.toml").exists(),
        "expectations were deleted"
    );

    harness.state.runs.add_permits(1);
    harness.wait_until(&run_id, RunStatus::is_terminal).await;
    assert!(!work.exists(), "inputs were kept after the run finished");
}

#[tokio::test]
async fn a_disconnected_client_does_not_cancel_an_accepted_run() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (status, accepted) = harness.submit_check(&project_id, &token).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let run_id = accepted["run_id"].as_str().expect("run id").to_string();

    // Everything belonging to the request is now dropped: the router, the
    // response, the body. The run is owned by the runtime, not by any of them.
    drop(accepted);

    let status = harness.wait_until(&run_id, RunStatus::is_terminal).await;
    assert!(
        matches!(status, RunStatus::Passed | RunStatus::Failed),
        "the run did not complete after the caller left: {status:?}"
    );
    let (_, run) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    assert_eq!(run["report_available"], true);
}

#[tokio::test]
async fn capacity_is_shared_and_every_accepted_run_still_finishes() {
    // One permit, two runs: the second has to wait for the first.
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let mut ids = Vec::new();
    for _ in 0..2 {
        let (status, body) = harness.submit_check(&project_id, &token).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        ids.push(body["run_id"].as_str().expect("run id").to_string());
    }
    for id in &ids {
        let status = harness.wait_until(id, RunStatus::is_terminal).await;
        assert!(status.is_terminal(), "{id}: {status:?}");
    }
    // Both ran; neither was dropped for want of a slot.
    for id in &ids {
        let (_, run) = harness.get(&format!("/v1/runs/{id}"), &token).await;
        assert_eq!(run["report_available"], true, "{run}");
    }
}

// ------------------------------------------------------------------ auth

#[tokio::test]
async fn another_project_s_token_learns_nothing_about_a_run() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, other_token) = ready_project(&harness, "Other").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    harness.wait_until(&run_id, RunStatus::is_terminal).await;

    for path in [
        format!("/v1/runs/{run_id}"),
        format!("/v1/runs/{run_id}/report.json"),
        format!("/v1/runs/{run_id}/report.md"),
    ] {
        let (status, body) = harness.get(&path, &other_token).await;
        // 401 rather than 404, and uniformly so: the token is only ever matched
        // against the owning project's tokens, so this answer is identical
        // whether the run exists, belongs elsewhere, or never existed. Nothing
        // about project A is learnable with project B's credential.
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: {body}");
        assert_eq!(body["error"], "unauthorized");
        assert!(body.get("summary").is_none(), "{body}");
    }
}

#[tokio::test]
async fn a_check_cannot_name_its_own_bundle() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    // The multipart contract admits two fields. Anything else is refused rather
    // than ignored, so a client cannot smuggle a baseline of its choosing.
    let boundary = "eplyxtestboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"bundle\"\r\n\r\nsomething\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let (status, body) = harness
        .send(
            authed("POST", &format!("/v1/projects/{project_id}/checks"), &token)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("request"),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().is_some_and(|e| e.contains("bundle")));
}

#[tokio::test]
async fn a_project_without_an_active_bundle_cannot_be_checked() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Unbundled").await;
    let token = harness.create_token(&project_id, "CI").await;
    let (status, body) = harness.submit_check(&project_id, &token).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"]
        .as_str()
        .is_some_and(|e| e.contains("no active bundle")));
    // A configuration problem is not a verdict about the candidate.
    assert!(body.get("exit_code").is_none(), "{body}");
}

// ---------------------------------------------------------- determinism

#[tokio::test]
async fn the_hosted_report_is_byte_identical_to_the_local_one() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    harness.wait_until(&run_id, RunStatus::is_terminal).await;
    let (_, hosted) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), &token)
        .await;

    // The same three inputs, through the same engine call the CLI makes.
    let scratch = tempfile::tempdir().expect("scratch");
    let bundle = build_bundle(&scratch.path().join("built"), &committed_record());
    let candidate_path = scratch.path().join("candidate.so");
    std::fs::write(&candidate_path, candidate_bytes()).expect("candidate");
    let report = eplyx_engine::ci::check(&bundle, &candidate_path, None).expect("local check");
    let mut local = serde_json::to_vec_pretty(&report).expect("serialize");
    local.push(b'\n');

    assert_eq!(
        String::from_utf8_lossy(&hosted),
        String::from_utf8_lossy(&local),
        "asynchronous orchestration changed the canonical report"
    );
}

#[tokio::test]
async fn the_engine_s_exit_codes_are_unchanged() {
    // Guards the mapping this work depends on, in the engine's own constants.
    assert_eq!(eplyx_engine::ci::EXIT_PASSED, 0);
    assert_eq!(eplyx_engine::ci::EXIT_ERROR, 2);
    assert_eq!(eplyx_engine::ci::EXIT_INCOMPATIBLE, 4);
}

/// Emit the bundle the acceptance script drives a live server with.
///
/// Ignored by default: it produces no assertion, only a directory. A durable
/// corpus is otherwise only written by `historical acquire`, which needs an
/// archive endpoint, so there is no offline CLI route to a bundle yet and the
/// end-to-end demo would not be reproducible without this.
#[test]
#[ignore = "tooling: writes a bundle to EPLYX_DEMO_BUNDLE"]
fn writes_a_demo_bundle() {
    let out = std::env::var("EPLYX_DEMO_BUNDLE").expect("set EPLYX_DEMO_BUNDLE");
    let out = std::path::PathBuf::from(out);
    let _ = std::fs::remove_dir_all(&out);
    let built = build_bundle(&out.join("staging"), &committed_record());
    std::fs::rename(&built, out.join("bundle")).expect("place the bundle");
    // A second, distinct bundle, so baseline rotation can be shown end to end.
    let mut second = committed_record();
    second.id = format!("{}-b", second.id);
    let other = build_bundle(&out.join("staging-b"), &second);
    std::fs::rename(&other, out.join("bundle-b")).expect("place the second bundle");
    println!("bundles at {}", out.display());
    let _ = json!({});
}
