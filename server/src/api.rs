//! The HTTP surface.
//!
//! Transport only. Every decision about what a change *means* — severity,
//! review status, bounds, precedence, coverage — is made by the engine, and
//! this layer is forbidden from reinterpreting any of it. The `exit_code` a
//! caller receives is the same number `eplyx ci check` would have returned
//! locally for the same three inputs.
//!
//! HTTP status and the Eplyx gate are separate axes. A candidate that fails
//! policy is `HTTP 200` with `exit_code: 1`: the request succeeded, and the
//! answer is that the upgrade should not ship. HTTP errors are for transport
//! and API faults only, so a normal regression never arrives as a 500.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use eplyx_engine::ci::{self, CiReport};
use serde::Serialize;
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::config::Config;
use crate::project::Project;
use crate::registry::{Registry, RunMetadata};

pub struct AppState {
    pub config: Config,
    pub registry: Registry,
    /// Replay is CPU-bound and synchronous. The permit count bounds how many
    /// run at once on a pilot host; a queue is deliberately not built yet.
    pub runs: tokio::sync::Semaphore,
}

pub type Shared = Arc<AppState>;

/// An API-level fault, as opposed to a candidate failing policy.
pub struct ApiError {
    status: StatusCode,
    message: String,
    /// The Eplyx gate code this fault corresponds to, where it has one.
    ///
    /// A preflight abort still has a real exit code - 2 for a malformed
    /// configuration, 4 for a bundle incompatibility - and a caller has to be
    /// able to propagate it. Carrying it in the error body keeps HTTP semantics
    /// honest without losing the gate result inside them.
    exit_code: Option<u8>,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            exit_code: None,
        }
    }

    fn with_exit_code(mut self, code: u8) -> Self {
        self.exit_code = Some(code);
        self
    }
    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    fn unauthorized() -> Self {
        // Deliberately uniform: whether the project exists, whether the token
        // was malformed, and whether it simply did not match all look the same
        // from outside.
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized")
    }
    fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("no such {what}"))
    }
    fn too_large(what: &str, limit: usize) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("{what} exceeds the {limit} byte limit"),
        )
    }
    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message });
        if let Some(code) = self.exit_code {
            body["exit_code"] = json!(code);
        }
        (self.status, Json(body)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

pub fn router(state: Shared) -> Router {
    let limit = state.config.max_candidate_bytes + state.config.max_expectation_bytes + 64 * 1024;
    // A browser sends a preflight for any request carrying an Authorization
    // header, and without this the router answered it with 405 and no
    // Access-Control-Allow-Origin, so the documented separate-origin frontend
    // could not call the API at all. Named origins only: never a wildcard,
    // because these requests are authenticated.
    let cors = CorsLayer::new()
        .allow_origin(
            state
                .config
                .allowed_origins
                .iter()
                .filter_map(|origin| origin.parse::<axum::http::HeaderValue>().ok())
                .collect::<Vec<_>>(),
        )
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(std::time::Duration::from_secs(600));
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/projects/{project_id}/checks", post(create_check))
        .route("/v1/runs/{run_id}", get(get_run))
        .route("/v1/runs/{run_id}/report.json", get(get_report_json))
        .route("/v1/runs/{run_id}/report.md", get(get_report_markdown))
        .layer(DefaultBodyLimit::max(limit))
        .layer(cors)
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

/// Ready means the persistent volume is actually usable. Nothing about the
/// configuration itself is exposed.
async fn ready(State(state): State<Shared>) -> impl IntoResponse {
    match state.registry.storage().writable() {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "ready" }))),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "storage unavailable" })),
        ),
    }
}

/// Authenticate a request against one project.
///
/// A token authenticates exactly the project whose record verifies it, so a
/// token for project A used on project B's URL fails like any other bad token.
fn authenticate(state: &AppState, project_id: &str, headers: &HeaderMap) -> ApiResult<Project> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(ApiError::unauthorized)?;

    let project = state
        .registry
        .load_project(project_id)
        .map_err(|_| ApiError::unauthorized())?;

    if !project.token.verifies(token) {
        return Err(ApiError::unauthorized());
    }
    Ok(project)
}

#[derive(Serialize)]
struct CheckResponse {
    run_id: String,
    project_id: String,
    status: &'static str,
    exit_code: u8,
    bundle_sha256: String,
    corpus_sha256: String,
    baseline_sha256: String,
    candidate_sha256: String,
    summary: serde_json::Value,
    report: serde_json::Value,
}

async fn create_check(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    multipart: Multipart,
) -> ApiResult<Response> {
    let project = authenticate(&state, &project_id, &headers)?;
    let (candidate, expectations) = read_upload(&state, multipart).await?;

    let candidate = candidate.ok_or_else(|| ApiError::bad_request("candidate is required"))?;

    // The client never chooses the baseline. The project's active bundle is
    // server state, so a pull request cannot quietly measure itself against
    // something more forgiving.
    let bundle_sha256 = project
        .active_bundle_sha256
        .clone()
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "no active bundle for this project"))?;

    let run_id = new_run_id();
    let registry = Arc::clone(&state);
    let id_for_run = run_id.clone();

    // Replay is blocking, CPU-bound work. It runs on the blocking pool with a
    // permit so a pilot host cannot be driven into swap by concurrent pushes.
    let _permit = state
        .runs
        .acquire()
        .await
        .map_err(|_| ApiError::internal("run scheduler closed"))?;

    let outcome = tokio::task::spawn_blocking(move || {
        run_check(
            &registry,
            &project,
            &bundle_sha256,
            &id_for_run,
            &candidate,
            expectations.as_deref(),
        )
    })
    .await
    .map_err(|error| ApiError::internal(format!("run failed: {error}")))?;

    let (report, metadata) = outcome?;
    Ok((
        StatusCode::OK,
        Json(CheckResponse {
            run_id: run_id.clone(),
            project_id,
            status: if report.summary.passed {
                "passed"
            } else {
                "failed"
            },
            exit_code: report.summary.exit_code,
            bundle_sha256: metadata.bundle_sha256,
            corpus_sha256: metadata.corpus_sha256,
            baseline_sha256: metadata.baseline_sha256,
            candidate_sha256: metadata.candidate_sha256,
            summary: serde_json::to_value(&report.summary).unwrap_or(json!({})),
            report: json!({
                "json": format!("/v1/runs/{run_id}/report.json"),
                "markdown": format!("/v1/runs/{run_id}/report.md"),
            }),
        }),
    )
        .into_response())
}

/// Read and bound the two uploaded parts.
///
/// Sizes are checked as bytes arrive rather than after, and anything the
/// request names that we do not expect is refused rather than ignored.
async fn read_upload(
    state: &AppState,
    mut multipart: Multipart,
) -> ApiResult<(Option<Bytes>, Option<Vec<u8>>)> {
    let mut candidate = None;
    let mut expectations = None;
    loop {
        // Keep multipart's own status rather than flattening it: a body that
        // overruns the transport limit really is 413, and reporting it as a
        // malformed request would send a caller looking for a syntax error in a
        // file that is merely too big.
        let field = multipart.next_field().await.map_err(|error| {
            ApiError::new(error.status(), format!("malformed multipart: {error}"))
        })?;
        let Some(field) = field else { break };
        let name = field.name().unwrap_or_default().to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|error| ApiError::new(error.status(), format!("upload rejected: {error}")))?;
        match name.as_str() {
            "candidate" => {
                if bytes.len() > state.config.max_candidate_bytes {
                    return Err(ApiError::too_large(
                        "candidate",
                        state.config.max_candidate_bytes,
                    ));
                }
                candidate = Some(bytes);
            }
            "expected_changes" => {
                if bytes.len() > state.config.max_expectation_bytes {
                    return Err(ApiError::too_large(
                        "expected_changes",
                        state.config.max_expectation_bytes,
                    ));
                }
                expectations = Some(bytes.to_vec());
            }
            other => {
                return Err(ApiError::bad_request(format!(
                    "unexpected upload field {other:?}"
                )))
            }
        }
    }
    Ok((candidate, expectations))
}

/// Everything that touches the engine, on a blocking thread.
///
/// The uploaded candidate lives in a temporary directory that is removed on
/// both success and failure. What survives is its hash, the report, and the run
/// metadata — never the binary itself.
fn run_check(
    state: &AppState,
    project: &Project,
    bundle_sha256: &str,
    run_id: &str,
    candidate: &[u8],
    expectations: Option<&[u8]>,
) -> ApiResult<(CiReport, RunMetadata)> {
    let bundle_dir = state
        .registry
        .storage()
        .bundle_path(bundle_sha256)
        .map_err(|_| ApiError::internal("bundle path"))?;

    let workspace = tempfile::Builder::new()
        .prefix("eplyx-run-")
        .tempdir()
        .map_err(|error| ApiError::internal(format!("run workspace: {error}")))?;
    let candidate_path = workspace.path().join("candidate.so");
    std::fs::write(&candidate_path, candidate)
        .map_err(|error| ApiError::internal(format!("staging candidate: {error}")))?;

    let expectation_path = match expectations {
        Some(bytes) => {
            let path = workspace.path().join("expected-changes.toml");
            std::fs::write(&path, bytes)
                .map_err(|error| ApiError::internal(format!("staging expectations: {error}")))?;
            Some(path)
        }
        None => None,
    };

    // The engine, called directly. There is no second implementation of any of
    // this, and nothing is shelled out to.
    let report = ci::check(&bundle_dir, &candidate_path, expectation_path.as_deref());

    // Whatever happened, the uploaded binary goes away now.
    drop(workspace);

    let report = report.map_err(|error| {
        let code = error.exit_code();
        // Neither of these is a server fault, and neither is a normal
        // regression. A malformed expectation file is the caller's to fix; an
        // incompatible bundle is the operator's. The Eplyx code travels with
        // the response either way.
        let status = match code {
            ci::EXIT_ERROR => StatusCode::BAD_REQUEST,
            _ => StatusCode::CONFLICT,
        };
        ApiError::new(status, format!("{error}")).with_exit_code(code)
    })?;

    let markdown = eplyx_engine::ci_markdown::render(&report);
    let metadata = RunMetadata {
        run_id: run_id.to_string(),
        project_id: project.id.clone(),
        bundle_sha256: report.bundle.sha256.clone(),
        corpus_sha256: report.bundle.corpus_sha256.clone(),
        baseline_sha256: report.bundle.baseline_sha256.clone(),
        candidate_sha256: report.candidate.sha256.clone(),
        adapter: report.bundle.adapter.clone(),
        adapter_version: report.bundle.adapter_version,
        semantic_schema_version: report.bundle.semantic_schema_version,
        record_count: report.bundle.record_count,
        status: if report.summary.passed {
            "passed".to_string()
        } else {
            "failed".to_string()
        },
        exit_code: report.summary.exit_code,
        created_at_unix_seconds: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default(),
    };
    state
        .registry
        .save_run(&metadata, &report, &markdown)
        .map_err(|error| ApiError::internal(format!("persisting the run: {error}")))?;
    Ok((report, metadata))
}

/// A run belongs to one project, and only that project's token may read it.
fn authorize_run(state: &AppState, run_id: &str, headers: &HeaderMap) -> ApiResult<RunMetadata> {
    let metadata = state
        .registry
        .load_run(run_id)
        .map_err(|_| ApiError::not_found("run"))?;
    authenticate(state, &metadata.project_id, headers)?;
    Ok(metadata)
}

async fn get_run(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let metadata = authorize_run(&state, &run_id, &headers)?;
    Ok((StatusCode::OK, Json(metadata)).into_response())
}

/// Reports are served as stored. Nothing is re-analysed to answer a fetch.
async fn get_report_json(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    authorize_run(&state, &run_id, &headers)?;
    let bytes = state
        .registry
        .load_run_artifact(&run_id, "report.json")
        .map_err(|_| ApiError::not_found("report"))?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        bytes,
    )
        .into_response())
}

async fn get_report_markdown(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    authorize_run(&state, &run_id, &headers)?;
    let bytes = state
        .registry
        .load_run_artifact(&run_id, "report.md")
        .map_err(|_| ApiError::not_found("report"))?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        bytes,
    )
        .into_response())
}

/// Opaque, sortable-enough, and never derived from anything secret.
fn new_run_id() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let random: [u8; 8] = rand::random();
    format!("run_{seconds:010}_{}", hex::encode(random))
}
