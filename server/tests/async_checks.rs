//! The hosted run lifecycle, driven through the real router.
//!
//! Every test below talks to `api::router` in-process. Nothing is re-created
//! for the tests: the same handlers, the same registry, the same worker and the
//! same engine call the service uses. What is faked is time pressure — the
//! execution semaphore is opened and closed by hand — because a scheduler that
//! can only be observed by sleeping is a scheduler that cannot be tested.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use eplyx_server::api::{self, AppState, Shared};
use eplyx_server::config::Config;
use eplyx_server::project::{generate_token, Project};
use eplyx_server::registry::{Registry, RunOutcome, RunStatus};
use eplyx_server::storage::Storage;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const BASELINE: &str = "../artifacts/fixture_lending_v1.so";
const CANDIDATE: &str = "../artifacts/fixture_lending_v2.so";

// ---------------------------------------------------------------- harness

struct Harness {
    scratch: tempfile::TempDir,
    state: Shared,
    project: String,
    token: String,
    other_project: String,
    other_token: String,
}

impl Harness {
    /// `permits` is the execution capacity. Zero means every accepted run stays
    /// queued until the test opens the gate, which is how the queued state is
    /// observed without racing a fast engine.
    fn new(permits: usize) -> Self {
        let scratch = tempfile::tempdir().expect("scratch");
        let storage = Storage::open(scratch.path().join("data")).expect("storage");
        let registry = Registry::new(storage);

        let record = committed_record();
        let bundle = build_bundle(&scratch.path().join("built"), &record);
        let bundle_sha256 = registry.install_bundle(&bundle).expect("install");

        let project = make_project(&registry, "demo", &record.program_id, Some(&bundle_sha256));
        let other = make_project(&registry, "other", &record.program_id, Some(&bundle_sha256));

        Self {
            state: Arc::new(AppState {
                config: test_config(),
                registry,
                runs: tokio::sync::Semaphore::new(permits),
            }),
            project: "demo".to_string(),
            token: project,
            other_project: "other".to_string(),
            other_token: other,
            scratch,
        }
    }

    /// A second service over the same volume: what a restart, or simply a new
    /// request after the browser reloaded, actually sees.
    fn reopen(&self) -> Shared {
        let storage = Storage::open(self.scratch.path().join("data")).expect("storage");
        Arc::new(AppState {
            config: test_config(),
            registry: Registry::new(storage),
            runs: tokio::sync::Semaphore::new(1),
        })
    }

    async fn post_check(
        &self,
        token: &str,
        project: &str,
        candidate: &[u8],
    ) -> (StatusCode, Value) {
        self.post_check_with(token, project, candidate, None).await
    }

    async fn post_check_with(
        &self,
        token: &str,
        project: &str,
        candidate: &[u8],
        expectations: Option<&str>,
    ) -> (StatusCode, Value) {
        let (content_type, body) = multipart(candidate, expectations);
        let request = Request::builder()
            .method("POST")
            .uri(format!("/v1/projects/{project}/checks"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", content_type)
            .body(Body::from(body))
            .expect("request");
        let response = api::router(Arc::clone(&self.state))
            .oneshot(request)
            .await
            .expect("response");
        let status = response.status();
        (status, json_body(response.into_body()).await)
    }

    async fn get(&self, path: &str, token: &str) -> (StatusCode, Value) {
        self.get_on(Arc::clone(&self.state), path, token).await
    }

    async fn get_on(&self, state: Shared, path: &str, token: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request");
        let response = api::router(state).oneshot(request).await.expect("response");
        let status = response.status();
        (status, json_body(response.into_body()).await)
    }

    async fn get_bytes(&self, path: &str, token: &str) -> (StatusCode, Vec<u8>) {
        let request = Request::builder()
            .method("GET")
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request");
        let response = api::router(Arc::clone(&self.state))
            .oneshot(request)
            .await
            .expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
            .to_vec();
        (status, bytes)
    }

    /// Poll the way a browser does, with a deadline rather than a fixed sleep.
    async fn wait_until(&self, run_id: &str, want: impl Fn(RunStatus) -> bool) -> RunStatus {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let status = self
                .state
                .registry
                .load_run(run_id)
                .expect("run is readable")
                .status;
            if want(status) {
                return status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "run {run_id} never reached the expected state; stuck at {status:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    fn work_dir(&self, run_id: &str) -> PathBuf {
        self.state.registry.run_work_dir(run_id).expect("work dir")
    }
}

fn test_config() -> Config {
    Config {
        allowed_origins: Vec::new(),
        data_dir: PathBuf::from("unused"),
        bind: "127.0.0.1:0".parse().expect("addr"),
        max_candidate_bytes: 8 * 1024 * 1024,
        max_expectation_bytes: 256 * 1024,
        max_concurrent_runs: 2,
    }
}

/// The project is pointed at its bundle directly rather than through
/// `activate_bundle`, whose adapter-compatibility policy is separately tested
/// and is not what any of this file is about.
fn make_project(
    registry: &Registry,
    id: &str,
    program_id: &str,
    bundle_sha256: Option<&str>,
) -> String {
    let token = generate_token();
    let mut project = Project::new(id, id, program_id, &token).expect("project");
    project.active_bundle_sha256 = bundle_sha256.map(str::to_string);
    registry.save_project(&project).expect("save project");
    token
}

fn committed_record() -> eplyx_engine::replay::ReplayRecord {
    serde_json::from_str(include_str!("../../docs/examples/replay-record.json"))
        .expect("the committed replay record")
}

/// A real bundle, validated by replaying V1 against the baseline the record
/// pins. Not a stand-in: the runs in this file execute genuine SBF bytecode
/// through the engine.
fn build_bundle(out: &Path, record: &eplyx_engine::replay::ReplayRecord) -> PathBuf {
    let baseline = Path::new(BASELINE);
    assert!(
        baseline.exists(),
        "{BASELINE} is missing; run ./scripts/build-programs.sh first"
    );
    let dependencies = out.join("deps");
    std::fs::create_dir_all(&dependencies).expect("deps");
    let bundle = eplyx_engine::bundle::build(
        eplyx_engine::bundle::BundleInputs {
            records: std::slice::from_ref(record),
            baseline,
            dependencies: &dependencies,
            selection_policy: None,
            selection_policy_version: None,
            limitations: Vec::new(),
            validation: eplyx_engine::bundle::Validation::AgainstBaseline,
        },
        &out.join("bundle"),
    )
    .expect("build bundle");
    bundle.root().to_path_buf()
}

fn candidate_bytes() -> Vec<u8> {
    std::fs::read(CANDIDATE).unwrap_or_else(|_| panic!("{CANDIDATE} is missing; build first"))
}

fn multipart(candidate: &[u8], expectations: Option<&str>) -> (String, Vec<u8>) {
    let boundary = "eplyxtestboundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"candidate\"; \
             filename=\"candidate.so\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(candidate);
    body.extend_from_slice(b"\r\n");
    if let Some(text) = expectations {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"expected_changes\"; \
                 filename=\"expected-changes.toml\"\r\nContent-Type: text/plain\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn json_body(body: Body) -> Value {
    let bytes = body.collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

// ------------------------------------------------------------ acceptance

#[tokio::test]
async fn a_check_is_accepted_without_waiting_for_the_analysis() {
    // No execution capacity at all, so nothing can possibly have run.
    let harness = Harness::new(0);
    let (status, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;

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
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
    let run_id = body["run_id"].as_str().expect("run id");

    // This is the promise a 202 makes: the id already resolves.
    let (status, run) = harness
        .get(&format!("/v1/runs/{run_id}"), &harness.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run["status"], "queued");
    assert_eq!(run["exit_code"], Value::Null);
    assert_eq!(run["report_available"], false);
    // Known before execution, because the server resolved it, not the client.
    assert!(run["bundle_sha256"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(run["candidate_sha256"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
}

#[tokio::test]
async fn a_queued_run_does_not_start_until_capacity_exists() {
    let harness = Harness::new(0);
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();

    // Long enough that a run which was going to start would have.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (_, run) = harness
        .get(&format!("/v1/runs/{run_id}"), &harness.token)
        .await;
    assert_eq!(run["status"], "queued", "started without a permit");
    assert_eq!(run["started_at_unix_seconds"], Value::Null);

    harness.state.runs.add_permits(1);
    let status = harness.wait_until(&run_id, RunStatus::is_terminal).await;
    assert!(status.is_terminal(), "{status:?}");

    let (_, run) = harness
        .get(&format!("/v1/runs/{run_id}"), &harness.token)
        .await;
    // Having passed through running is visible afterwards even when the engine
    // was too quick to catch in the act.
    assert!(run["started_at_unix_seconds"].as_u64().is_some());
    assert!(run["completed_at_unix_seconds"].as_u64().is_some());
}

#[tokio::test]
async fn a_completed_run_carries_a_report_and_the_engine_s_exit_code() {
    let harness = Harness::new(1);
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    harness.wait_until(&run_id, RunStatus::is_terminal).await;

    let (_, run) = harness
        .get(&format!("/v1/runs/{run_id}"), &harness.token)
        .await;
    assert_eq!(run["status"], "failed", "{run}");
    assert_eq!(run["report_available"], true);
    let exit_code = run["exit_code"].as_u64().expect("an exit code");

    let (status, bytes) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), &harness.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    let report: Value = serde_json::from_slice(&bytes).expect("canonical json");
    assert_eq!(report["summary"]["exit_code"].as_u64(), Some(exit_code));
    assert_eq!(report["summary"]["passed"], false);

    let (status, markdown) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.md"), &harness.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!markdown.is_empty());
}

#[tokio::test]
async fn a_report_is_refused_until_one_exists() {
    let harness = Harness::new(0);
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();

    for artifact in ["report.json", "report.md"] {
        let (status, body) = harness
            .get(&format!("/v1/runs/{run_id}/{artifact}"), &harness.token)
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
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
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
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
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
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
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

    let (_, run) = harness
        .get(&format!("/v1/runs/{run_id}"), &harness.token)
        .await;
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
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
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

    let (_, run) = harness
        .get(&format!("/v1/runs/{run_id}"), &harness.token)
        .await;
    // A real verdict that arrived before there was a report to put it in.
    assert_eq!(run["status"], "failed");
    assert_eq!(run["exit_code"], 4);
    assert_eq!(run["report_available"], false);

    let (status, body) = harness
        .get(&format!("/v1/runs/{run_id}/report.json"), &harness.token)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["exit_code"], 4, "the gate code survives the refusal");
}

// -------------------------------------------------------------- recovery

#[tokio::test]
async fn a_reload_recovers_a_run_without_any_browser_state() {
    let harness = Harness::new(0);
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();

    // A different service instance over the same volume: no shared memory, no
    // continuation of the request that created the run.
    let reopened = harness.reopen();
    let (status, run) = harness
        .get_on(reopened, &format!("/v1/runs/{run_id}"), &harness.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run["run_id"], run_id.as_str());
    assert_eq!(run["status"], "queued");
}

#[tokio::test]
async fn a_completed_run_reads_the_same_on_every_later_fetch() {
    let harness = Harness::new(1);
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    harness.wait_until(&run_id, RunStatus::is_terminal).await;

    let (_, first) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), &harness.token)
        .await;
    let (_, second) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), &harness.token)
        .await;
    assert_eq!(first, second, "a refetch produced different bytes");

    let reopened = harness.reopen();
    let (status, run) = harness
        .get_on(reopened, &format!("/v1/runs/{run_id}"), &harness.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run["report_available"], true);
}

#[tokio::test]
async fn runs_interrupted_by_a_restart_are_resolved_rather_than_left_running() {
    let harness = Harness::new(0);
    let mut ids = Vec::new();
    for _ in 0..2 {
        let (_, body) = harness
            .post_check(&harness.token, &harness.project, &candidate_bytes())
            .await;
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
            .get_on(
                Arc::clone(&restarted),
                &format!("/v1/runs/{id}"),
                &harness.token,
            )
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
    let expectations = "version = 1\nsemantic_schema_version = 2\n";
    let (status, body) = harness
        .post_check_with(
            &harness.token,
            &harness.project,
            &candidate_bytes(),
            Some(expectations),
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
    let (content_type, body) = multipart(&candidate_bytes(), None);
    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/projects/{}/checks", harness.project))
        .header("authorization", format!("Bearer {}", harness.token))
        .header("content-type", content_type)
        .body(Body::from(body))
        .expect("request");

    let (status, accepted) = {
        let response = api::router(Arc::clone(&harness.state))
            .oneshot(request)
            .await
            .expect("response");
        let status = response.status();
        (status, json_body(response.into_body()).await)
    };
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
    let (_, run) = harness
        .get(&format!("/v1/runs/{run_id}"), &harness.token)
        .await;
    assert_eq!(run["report_available"], true);
}

#[tokio::test]
async fn capacity_is_shared_and_every_accepted_run_still_finishes() {
    // One permit, two runs: the second has to wait for the first.
    let harness = Harness::new(1);
    let mut ids = Vec::new();
    for _ in 0..2 {
        let (status, body) = harness
            .post_check(&harness.token, &harness.project, &candidate_bytes())
            .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        ids.push(body["run_id"].as_str().expect("run id").to_string());
    }
    for id in &ids {
        let status = harness.wait_until(id, RunStatus::is_terminal).await;
        assert!(status.is_terminal(), "{id}: {status:?}");
    }
    // Both ran; neither was dropped for want of a slot.
    for id in &ids {
        let (_, run) = harness.get(&format!("/v1/runs/{id}"), &harness.token).await;
        assert_eq!(run["report_available"], true, "{run}");
    }
}

// ------------------------------------------------------------------ auth

#[tokio::test]
async fn another_project_s_token_learns_nothing_about_a_run() {
    let harness = Harness::new(1);
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate_bytes())
        .await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    harness.wait_until(&run_id, RunStatus::is_terminal).await;

    for path in [
        format!("/v1/runs/{run_id}"),
        format!("/v1/runs/{run_id}/report.json"),
        format!("/v1/runs/{run_id}/report.md"),
    ] {
        let (status, body) = harness.get(&path, &harness.other_token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: {body}");
        // Not even the existence of the run leaks.
        assert_eq!(body["error"], "unauthorized");
    }
    let _ = &harness.other_project;
}

#[tokio::test]
async fn a_check_cannot_name_its_own_bundle() {
    let harness = Harness::new(0);
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
    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/projects/{}/checks", harness.project))
        .header("authorization", format!("Bearer {}", harness.token))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .expect("request");
    let response = api::router(Arc::clone(&harness.state))
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response.into_body()).await;
    assert!(body["error"].as_str().is_some_and(|e| e.contains("bundle")));
}

#[tokio::test]
async fn a_project_without_an_active_bundle_cannot_be_checked() {
    let harness = Harness::new(0);
    let token = make_project(
        &harness.state.registry,
        "unbundled",
        &committed_record().program_id,
        None,
    );
    let (status, body) = harness
        .post_check(&token, "unbundled", &candidate_bytes())
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"]
        .as_str()
        .is_some_and(|e| e.contains("no active bundle")));
}

// ---------------------------------------------------------- determinism

#[tokio::test]
async fn the_hosted_report_is_byte_identical_to_the_local_one() {
    let harness = Harness::new(1);
    let candidate = candidate_bytes();
    let (_, body) = harness
        .post_check(&harness.token, &harness.project, &candidate)
        .await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    harness.wait_until(&run_id, RunStatus::is_terminal).await;
    let (_, hosted) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), &harness.token)
        .await;

    // The same three inputs, through the same engine call the CLI makes.
    let scratch = tempfile::tempdir().expect("scratch");
    let bundle = build_bundle(&scratch.path().join("built"), &committed_record());
    let candidate_path = scratch.path().join("candidate.so");
    std::fs::write(&candidate_path, &candidate).expect("candidate");
    let report = eplyx_engine::ci::check(&bundle, &candidate_path, None).expect("local check");
    let mut local = serde_json::to_vec_pretty(&report).expect("serialize");
    local.push(b'\n');

    assert_eq!(
        String::from_utf8_lossy(&hosted),
        String::from_utf8_lossy(&local),
        "asynchronous orchestration changed the canonical report"
    );
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
    let out = PathBuf::from(out);
    let _ = std::fs::remove_dir_all(&out);
    let built = build_bundle(&out.join("staging"), &committed_record());
    std::fs::rename(&built, out.join("bundle")).expect("place the bundle");
    println!("bundle at {}", out.join("bundle").display());
}

#[tokio::test]
async fn the_engine_s_exit_codes_are_unchanged() {
    // Guards the mapping this work depends on, in the engine's own constants.
    assert_eq!(eplyx_engine::ci::EXIT_PASSED, 0);
    assert_eq!(eplyx_engine::ci::EXIT_ERROR, 2);
    assert_eq!(eplyx_engine::ci::EXIT_INCOMPATIBLE, 4);
}
