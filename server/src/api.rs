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
use eplyx_engine::ci;
use serde::Serialize;
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::config::Config;
use crate::project::Project;
use crate::registry::{now_unix_seconds, Registry, RunMetadata, RunStatus};
use crate::worker;

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

/// What creating a check answers with.
///
/// Deliberately not a result. At this point no analysis has run, and inventing
/// a verdict to fill the shape would be a lie the caller could act on.
#[derive(Serialize)]
struct AcceptedResponse {
    run_id: String,
    project_id: String,
    status: RunStatus,
    status_url: String,
}

/// Accept a check and return immediately.
///
/// Everything expensive happens after the response. What this owes the caller
/// is a run id that is already durable: if a 202 was received, the run is
/// recoverable through `GET /v1/runs/{id}` even if this connection never
/// carried another byte.
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

    // Opened, not merely located. Accepting a run against a bundle that cannot
    // be read would buy a 202 and pay for it with an execution error minutes
    // later, once the caller has stopped watching.
    let bundle = state
        .registry
        .open_bundle(&bundle_sha256)
        .map_err(|error| {
            ApiError::new(
                StatusCode::CONFLICT,
                format!("the active bundle is unusable: {error}"),
            )
            .with_exit_code(ci::EXIT_INCOMPATIBLE)
        })?;

    let run_id = new_run_id();

    // Staged under the run, not in a request-scoped temporary directory: this
    // handler returns long before the worker reads these, and a TempDir dropped
    // with the response would delete the candidate out from under the run.
    let work = state
        .registry
        .run_work_dir(&run_id)
        .map_err(|error| ApiError::internal(format!("run workspace: {error}")))?;
    std::fs::create_dir_all(&work)
        .map_err(|error| ApiError::internal(format!("run workspace: {error}")))?;
    std::fs::write(work.join("candidate.so"), &candidate)
        .map_err(|error| ApiError::internal(format!("staging the candidate: {error}")))?;
    if let Some(bytes) = &expectations {
        std::fs::write(work.join("expected-changes.toml"), bytes)
            .map_err(|error| ApiError::internal(format!("staging the expectations: {error}")))?;
    }

    let manifest = bundle.manifest();
    let metadata = RunMetadata {
        run_id: run_id.clone(),
        project_id: project.id.clone(),
        status: RunStatus::Queued,
        bundle_sha256: manifest.bundle_sha256.clone(),
        corpus_sha256: Some(manifest.corpus_sha256.clone()),
        baseline_sha256: Some(manifest.baseline_program_sha256.clone()),
        candidate_sha256: eplyx_engine::replay::hash_bytes(&candidate),
        adapter: Some(bundle.adapter().name.clone()),
        adapter_version: Some(bundle.adapter().version),
        semantic_schema_version: Some(manifest.semantic_schema_version),
        record_count: Some(manifest.record_count),
        exit_code: None,
        report_available: false,
        detail: None,
        created_at_unix_seconds: now_unix_seconds(),
        started_at_unix_seconds: None,
        completed_at_unix_seconds: None,
    };

    // Durable before anything expensive begins. Only then is a worker started,
    // so there is no window in which work exists that nothing can be told about.
    state
        .registry
        .create_run(&metadata)
        .map_err(|error| ApiError::internal(format!("persisting the run: {error}")))?;
    worker::spawn(Arc::clone(&state), run_id.clone());

    Ok((
        StatusCode::ACCEPTED,
        Json(AcceptedResponse {
            status_url: format!("/v1/runs/{run_id}"),
            run_id,
            project_id,
            status: RunStatus::Queued,
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

/// Why a report cannot be served yet, or at all.
///
/// A run still in flight is a 409 the caller should retry; a run that ended
/// without a report is a 409 that will never become a 200, and says so. Neither
/// invents an empty report, because a consumer cannot tell a fabricated shape
/// from a real one.
fn report_unavailable(metadata: &RunMetadata) -> ApiError {
    let message = match metadata.status {
        RunStatus::Queued => "this run is queued; no report exists yet".to_string(),
        RunStatus::Running => "this run is still executing; no report exists yet".to_string(),
        _ => match &metadata.detail {
            Some(detail) => format!("this run produced no report: {detail}"),
            None => "this run produced no report".to_string(),
        },
    };
    let error = ApiError::new(StatusCode::CONFLICT, message);
    match metadata.exit_code {
        Some(code) => error.with_exit_code(code),
        None => error,
    }
}

/// Reports are served as stored. Nothing is re-analysed to answer a fetch.
async fn get_report_json(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let metadata = authorize_run(&state, &run_id, &headers)?;
    if !metadata.report_available {
        return Err(report_unavailable(&metadata));
    }
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
    let metadata = authorize_run(&state, &run_id, &headers)?;
    if !metadata.report_available {
        return Err(report_unavailable(&metadata));
    }
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
