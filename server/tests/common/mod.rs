//! A hosted service, driven through its real router.
//!
//! Nothing here re-creates the service: the same handlers, registry, worker and
//! engine call that ships. What is faked is time pressure — the execution
//! semaphore is opened and closed by hand — because a scheduler that can only
//! be observed by sleeping is a scheduler that cannot be tested.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use eplyx_server::api::{self, AppState, Shared};
use eplyx_server::config::Config;
use eplyx_server::project::{AdapterId, Project, ProjectStatus};
use eplyx_server::registry::{Registry, RunStatus};
use eplyx_server::storage::Storage;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

pub const BASELINE: &str = "../artifacts/fixture_lending_v1.so";
pub const CANDIDATE: &str = "../artifacts/fixture_lending_v2.so";
pub const OPERATOR: &str = "operator-secret-for-tests";
pub const STAKE_POOL_PROGRAM: &str = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";

pub struct Harness {
    pub scratch: tempfile::TempDir,
    pub state: Shared,
}

impl Harness {
    /// `permits` is the execution capacity. Zero means every accepted run stays
    /// queued until the test opens the gate, which is how the queued state is
    /// observed without racing a fast engine.
    pub fn new(permits: usize) -> Self {
        let scratch = tempfile::tempdir().expect("scratch");
        let storage = Storage::open(scratch.path().join("data")).expect("storage");
        Self {
            state: Arc::new(AppState {
                config: test_config(),
                registry: Registry::new(storage),
                runs: tokio::sync::Semaphore::new(permits),
            }),
            scratch,
        }
    }

    /// A second service over the same volume: what a restart, or simply a new
    /// request after the browser reloaded, actually sees.
    pub fn reopen(&self) -> Shared {
        let storage = Storage::open(self.scratch.path().join("data")).expect("storage");
        Arc::new(AppState {
            config: test_config(),
            registry: Registry::new(storage),
            runs: tokio::sync::Semaphore::new(1),
        })
    }

    pub async fn send(&self, request: Request<Body>) -> (StatusCode, Value) {
        self.send_on(Arc::clone(&self.state), request).await
    }

    pub async fn send_on(&self, state: Shared, request: Request<Body>) -> (StatusCode, Value) {
        let response = api::router(state).oneshot(request).await.expect("response");
        let status = response.status();
        (status, json_body(response.into_body()).await)
    }

    pub async fn get(&self, path: &str, token: &str) -> (StatusCode, Value) {
        self.send(
            authed("GET", path, token)
                .body(Body::empty())
                .expect("request"),
        )
        .await
    }

    pub async fn get_on(&self, state: Shared, path: &str, token: &str) -> (StatusCode, Value) {
        self.send_on(
            state,
            authed("GET", path, token)
                .body(Body::empty())
                .expect("request"),
        )
        .await
    }

    pub async fn get_bytes(&self, path: &str, token: &str) -> (StatusCode, Vec<u8>) {
        let response = api::router(Arc::clone(&self.state))
            .oneshot(
                authed("GET", path, token)
                    .body(Body::empty())
                    .expect("request"),
            )
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

    pub async fn post_json(&self, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
        self.send(
            authed("POST", path, token)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
    }

    pub async fn delete(&self, path: &str, token: &str) -> (StatusCode, Value) {
        self.send(
            authed("DELETE", path, token)
                .body(Body::empty())
                .expect("request"),
        )
        .await
    }

    // ------------------------------------------------------------ shortcuts

    /// A project on the fixture program, which this build speaks no semantics
    /// for. Checks against it run and report exactly that.
    pub async fn create_project(&self, name: &str) -> String {
        let record = committed_record();
        let (status, body) = self
            .post_json(
                "/v1/projects",
                OPERATOR,
                json!({
                    "name": name,
                    "program_id": record.program_id,
                    "adapter_id": AdapterId::for_program(&record.program_id).to_string(),
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["project_id"].as_str().expect("project id").to_string()
    }

    pub async fn create_token(&self, project_id: &str, label: &str) -> String {
        let (status, body) = self
            .post_json(
                &format!("/v1/projects/{project_id}/tokens"),
                OPERATOR,
                json!({ "label": label }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["token"].as_str().expect("token").to_string()
    }

    /// Build, upload and activate a bundle, returning its id.
    pub async fn activate_bundle(&self, project_id: &str) -> String {
        let bundle_id = self.upload_bundle(project_id).await;
        let (status, body) = self
            .post_json(
                &format!("/v1/projects/{project_id}/bundles/{bundle_id}/activate"),
                OPERATOR,
                json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "ready", "{body}");
        bundle_id
    }

    pub async fn upload_bundle(&self, project_id: &str) -> String {
        let built = build_bundle(
            &self
                .scratch
                .path()
                .join(format!("built-{project_id}-{}", rand_suffix())),
            &committed_record(),
        );
        self.upload_bundle_from(project_id, &built).await
    }

    pub async fn upload_bundle_from(&self, project_id: &str, dir: &Path) -> String {
        let (status, body) = self.upload_bundle_result(project_id, dir).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["bundle_id"].as_str().expect("bundle id").to_string()
    }

    pub async fn upload_bundle_result(&self, project_id: &str, dir: &Path) -> (StatusCode, Value) {
        let (content_type, body) = bundle_multipart(dir);
        self.send(
            authed(
                "POST",
                &format!("/v1/projects/{project_id}/bundles"),
                OPERATOR,
            )
            .header("content-type", content_type)
            .body(Body::from(body))
            .expect("request"),
        )
        .await
    }

    pub async fn submit_check(&self, project_id: &str, token: &str) -> (StatusCode, Value) {
        let (content_type, body) = candidate_multipart(&candidate_bytes(), None);
        self.send(
            authed("POST", &format!("/v1/projects/{project_id}/checks"), token)
                .header("content-type", content_type)
                .body(Body::from(body))
                .expect("request"),
        )
        .await
    }

    /// Poll the way a browser does, with a deadline rather than a fixed sleep.
    pub async fn wait_until(&self, run_id: &str, want: impl Fn(RunStatus) -> bool) -> RunStatus {
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

    pub fn work_dir(&self, run_id: &str) -> PathBuf {
        self.state.registry.run_work_dir(run_id).expect("work dir")
    }

    pub fn set_status(&self, project_id: &str, status: ProjectStatus) {
        let mut project = self
            .state
            .registry
            .load_project(project_id)
            .expect("project");
        project.status = status;
        self.state.registry.save_project(&project).expect("save");
    }
}

pub fn test_config() -> Config {
    Config {
        allowed_origins: Vec::new(),
        operator_token: Some(OPERATOR.to_string()),
        data_dir: PathBuf::from("unused"),
        bind: "127.0.0.1:0".parse().expect("addr"),
        max_candidate_bytes: 8 * 1024 * 1024,
        max_expectation_bytes: 256 * 1024,
        max_bundle_bytes: 192 * 1024 * 1024,
        max_concurrent_runs: 2,
    }
}

pub fn authed(method: &str, path: &str, token: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
}

pub fn committed_record() -> eplyx_engine::replay::ReplayRecord {
    serde_json::from_str(include_str!("../../../docs/examples/replay-record.json"))
        .expect("the committed replay record")
}

pub fn stake_pool_record() -> eplyx_engine::replay::ReplayRecord {
    serde_json::from_str(include_str!(
        "../../../docs/examples/mainnet-stake-pool-record.json"
    ))
    .expect("the committed stake-pool record")
}

/// A real bundle, validated by replaying V1 against the baseline the record
/// pins. Not a stand-in: the runs in these tests execute genuine SBF bytecode
/// through the engine.
pub fn build_bundle(out: &Path, record: &eplyx_engine::replay::ReplayRecord) -> PathBuf {
    let baseline = Path::new(BASELINE);
    assert!(
        baseline.exists(),
        "{BASELINE} is missing; run ./scripts/build-programs.sh first"
    );
    build_bundle_with(
        out,
        record,
        baseline,
        eplyx_engine::bundle::Validation::AgainstBaseline,
    )
}

/// A bundle around a record that cannot be replayed here, assembled only.
///
/// Used where the point is what a bundle *claims* — a different program, a
/// different adapter — rather than whether it executes.
pub fn build_unvalidated_bundle(
    out: &Path,
    record: &eplyx_engine::replay::ReplayRecord,
) -> PathBuf {
    use eplyx_engine::dependencies::{DependencyDiscovery, ProgramDependency, ProgramSource};
    let baseline = vec![7_u8; 512];
    let mut record = record.clone();
    record.current_program_sha256 = eplyx_engine::replay::hash_bytes(&baseline);
    record.dependencies.programs = vec![ProgramDependency {
        program_id: record.program_id.clone(),
        source: ProgramSource::HistoricalMainnet,
        loader: None,
        deployed_slot: None,
        binary_sha256: Some(eplyx_engine::replay::hash_bytes(&baseline)),
        binary_len: Some(baseline.len() as u64),
        observed_slot: None,
        discovered_by: vec![DependencyDiscovery::ProgramUnderTest],
        note: None,
    }];
    std::fs::create_dir_all(out).expect("out");
    let baseline_path = out.join("standin.so");
    std::fs::write(&baseline_path, &baseline).expect("baseline");
    build_bundle_with(
        out,
        &record,
        &baseline_path,
        eplyx_engine::bundle::Validation::Skip {
            reason: "this fixture pins a stand-in, not an executable program",
        },
    )
}

fn build_bundle_with(
    out: &Path,
    record: &eplyx_engine::replay::ReplayRecord,
    baseline: &Path,
    validation: eplyx_engine::bundle::Validation,
) -> PathBuf {
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
            validation,
        },
        &out.join("bundle"),
    )
    .expect("build bundle");
    bundle.root().to_path_buf()
}

pub fn candidate_bytes() -> Vec<u8> {
    std::fs::read(CANDIDATE).unwrap_or_else(|_| panic!("{CANDIDATE} is missing; build first"))
}

fn rand_suffix() -> String {
    format!("{:x}", rand::random::<u32>())
}

/// One multipart part per bundle file, named by its path inside the bundle.
pub fn bundle_multipart(dir: &Path) -> (String, Vec<u8>) {
    let boundary = "eplyxbundleboundary";
    let mut body = Vec::new();
    for (relative, bytes) in walk(dir, dir) {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{relative}\"; \
                 filename=\"{relative}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn walk(root: &Path, dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(root, &path));
        } else if let Ok(relative) = path.strip_prefix(root) {
            let name = relative.to_string_lossy().replace('\\', "/");
            files.push((name, std::fs::read(&path).unwrap_or_default()));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

pub fn candidate_multipart(candidate: &[u8], expectations: Option<&str>) -> (String, Vec<u8>) {
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

pub async fn json_body(body: Body) -> Value {
    let bytes = body.collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// A project with a token and an active bundle: the state onboarding leaves.
pub async fn ready_project(harness: &Harness, name: &str) -> (String, String) {
    let project_id = harness.create_project(name).await;
    let token = harness.create_token(&project_id, "CI").await;
    harness.activate_bundle(&project_id).await;
    (project_id, token)
}

pub fn assert_project(project: &Project, status: ProjectStatus) {
    assert_eq!(project.status, status, "{project:?}");
}

/// Any set of named parts, for requests the two-part helper cannot express.
pub fn parts_multipart(parts: &[(&str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "eplyxpartsboundary";
    let mut body = Vec::new();
    for (name, bytes) in parts {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; \
                 filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

impl Harness {
    pub async fn submit_parts(
        &self,
        project_id: &str,
        token: &str,
        parts: &[(&str, &[u8])],
    ) -> (StatusCode, Value) {
        let (content_type, body) = parts_multipart(parts);
        self.send(
            authed("POST", &format!("/v1/projects/{project_id}/checks"), token)
                .header("content-type", content_type)
                .body(Body::from(body))
                .expect("request"),
        )
        .await
    }
}
