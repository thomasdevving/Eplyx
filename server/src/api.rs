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
use eplyx_engine::change::{CandidateSource, ChangeSpec};
use eplyx_engine::ci;
use serde::Serialize;
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::artifacts::ArtifactRef;
use crate::config::Config;
use crate::project::{
    validate_name, validate_program_id, AdapterId, Chain, Project, ProjectStatus, ProjectToken,
};
use crate::registry::{
    now_unix_seconds, ChangeOrigin, ProjectBundle, Registry, RunChange, RunMetadata, RunStatus,
};
use crate::worker;

pub struct AppState {
    pub config: Config,
    pub registry: Registry,
    /// Replay is CPU-bound and synchronous. The permit count bounds how many
    /// run at once on a pilot host; a queue is deliberately not built yet.
    pub runs: tokio::sync::Semaphore,
}

pub type Shared = Arc<AppState>;

/// A change spec is a few hundred bytes of identity. Anything near this is not
/// one, and parsing it would only cost memory.
pub const MAX_CHANGE_SPEC_BYTES: usize = 64 * 1024;
/// A display label, not a document.
pub const MAX_LABEL_CHARS: usize = 120;

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

/// One answer for a request body this API cannot use.
///
/// Axum's own JSON extractor reports a rejected body as 422 while every
/// hand-written validation here reports 400, which left one class of mistake
/// arriving under two different statuses depending on which check caught it.
fn parse_json<T: serde::de::DeserializeOwned>(body: &Bytes) -> ApiResult<T> {
    serde_json::from_slice(body)
        .map_err(|error| ApiError::bad_request(format!("invalid request body: {error}")))
}

pub fn router(state: Shared) -> Router {
    let limit = state.config.max_bundle_bytes.max(
        state.config.max_candidate_bytes
            + state.config.max_expectation_bytes
            + MAX_CHANGE_SPEC_BYTES,
    ) + 64 * 1024;
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
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(std::time::Duration::from_secs(600));
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/adapters", get(list_adapters))
        .route("/v1/projects", post(create_project).get(list_projects))
        .route("/v1/projects/{project_id}", get(get_project))
        .route(
            "/v1/projects/{project_id}/tokens",
            post(create_token).get(list_tokens),
        )
        .route(
            "/v1/projects/{project_id}/tokens/{token_id}",
            axum::routing::delete(revoke_token),
        )
        .route(
            "/v1/projects/{project_id}/bundles",
            post(create_bundle).get(list_bundles),
        )
        .route(
            "/v1/projects/{project_id}/bundles/{bundle_id}/activate",
            post(activate_bundle),
        )
        .route("/v1/projects/{project_id}/runs", get(list_project_runs))
        .route("/v1/projects/{project_id}/checks", post(create_check))
        .route(
            "/v1/projects/{project_id}/artifacts/{sha256}",
            get(get_artifact),
        )
        .route("/v1/runs/{run_id}", get(get_run))
        .route("/v1/runs/{run_id}/report.json", get(get_report_json))
        .route("/v1/runs/{run_id}/report.md", get(get_report_markdown))
        .route("/v1/runs/{run_id}/change_spec.json", get(get_change_spec))
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

/// Who is asking.
///
/// Two credentials, and deliberately no user model. A **project token** is a CI
/// secret: it may submit checks for its own project and read that project's
/// results, and nothing else. An **operator token** is the hosted dashboard's
/// credential, configured on the server rather than issued by it; it is what
/// creates projects, issues and revokes their tokens, and registers and
/// activates bundles.
///
/// The split is the Phase 10 rule applied to credentials: a token that lives in
/// a pull request must not be able to change what future pull requests are
/// measured against. Nothing here is a person, and no endpoint is public —
/// listing projects without a credential would hand an unauthenticated caller
/// every program this service watches.
pub enum Principal {
    Operator,
    Project {
        project: Box<Project>,
        token: Box<ProjectToken>,
    },
}

impl Principal {
    fn is_operator(&self) -> bool {
        matches!(self, Self::Operator)
    }
}

fn bearer(headers: &HeaderMap) -> ApiResult<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(ApiError::unauthorized)
}

/// Authenticate the operator credential, and nothing else.
fn operator(state: &AppState, headers: &HeaderMap) -> ApiResult<Principal> {
    let secret = bearer(headers)?;
    let configured = state.config.operator_token.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "no operator credential is configured",
        )
    })?;
    if !constant_time_eq(configured, secret) {
        return Err(ApiError::unauthorized());
    }
    Ok(Principal::Operator)
}

/// Authenticate against a named project: its own live token, or the operator.
///
/// A token for another project fails exactly like a token for none. Which of
/// the two it was is not distinguishable from outside, and should not be.
fn authenticate(state: &AppState, project_id: &str, headers: &HeaderMap) -> ApiResult<Principal> {
    let secret = bearer(headers)?;
    if let Some(configured) = state.config.operator_token.as_deref() {
        if constant_time_eq(configured, secret) {
            return Ok(Principal::Operator);
        }
    }
    let project = state
        .registry
        .load_project(project_id)
        .map_err(|_| ApiError::unauthorized())?;
    let token = state
        .registry
        .authenticate_token(project_id, secret)
        .map_err(|_| ApiError::unauthorized())?;
    state.registry.note_token_use(&token);
    Ok(Principal::Project {
        project: Box::new(project),
        token: Box::new(token),
    })
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0_u8, |difference, (x, y)| difference | (x ^ y))
        == 0
}

/// Require the operator, for anything that changes what checks measure against.
fn require_operator(principal: &Principal) -> ApiResult<()> {
    if principal.is_operator() {
        return Ok(());
    }
    // A CI token asking to rotate a baseline is not a permissions puzzle to
    // explain; from outside it looks like the thing does not exist.
    Err(ApiError::not_found("resource"))
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
    /// What was accepted for analysis, already bound to the pinned bundle.
    /// Identity is known before any replay; a verdict is not.
    change: RunChange,
    candidate_sha256: String,
    /// The durable, content-addressed object the run will execute.
    candidate_artifact: ArtifactRef,
    bundle_sha256: String,
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
    let principal = authenticate(&state, &project_id, &headers)?;
    let project = match &principal {
        Principal::Project { project, .. } => (**project).clone(),
        Principal::Operator => state
            .registry
            .load_project(&project_id)
            .map_err(|_| ApiError::not_found("project"))?,
    };
    let upload = read_upload(&state, multipart).await?;

    // Readiness is a hosted configuration question, answered before a run
    // exists. It is deliberately not an Eplyx exit code: nothing was measured,
    // so there is no verdict to report about the candidate.
    if project.status == ProjectStatus::Disabled {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "this project is disabled and accepts no checks",
        ));
    }
    // The client never chooses the baseline. The project's active bundle is
    // server state, so a pull request cannot quietly measure itself against
    // something more forgiving.
    let active = project.active_bundle.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "this project has no active bundle; upload and activate one before running checks",
        )
    })?;
    let bundle_sha256 = active.bundle_sha256.clone();

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
    let manifest = bundle.manifest();
    let bundle_dir = state
        .registry
        .storage()
        .bundle_path(&bundle_sha256)
        .map_err(|error| ApiError::internal(format!("bundle path: {error}")))?;

    // ---- the proposal, fixed before the run exists ------------------------
    //
    // Candidate bytes alone stand for the minimal program upgrade of the pinned
    // bundle's program, exactly as `eplyx ci check --candidate` does. An
    // explicit spec is authoritative, and the bytes only satisfy its artefact
    // reference. Either way the spec is bound to the pinned bundle and the bytes
    // are verified against it here, so a proposal that does not fit, or bytes
    // it did not describe, never become a queued run.
    let submitted = match &upload.change_spec {
        Some(document) => {
            if upload.label.is_some() {
                return Err(ApiError::bad_request(
                    "`label` cannot accompany an explicit change spec; put it in the spec's metadata",
                )
                .with_exit_code(ci::EXIT_ERROR));
            }
            Some(ChangeSpec::parse(document).map_err(|error| {
                ApiError::bad_request(format!("invalid change spec: {error:#}"))
                    .with_exit_code(ci::EXIT_ERROR)
            })?)
        }
        None => None,
    };
    // The bytes: uploaded, or — for an explicit spec only — an artefact this
    // project already supplied, so a proposal can be analysed again without
    // re-uploading what the service already holds immutably.
    let candidate: Vec<u8> = match (upload.candidate, &submitted) {
        (Some(bytes), _) => bytes.to_vec(),
        (None, None) => return Err(ApiError::bad_request("candidate is required")),
        (None, Some(spec)) => retained_candidate(&state, &project.project_id, spec)?,
    };
    let (spec, origin) = match submitted {
        Some(spec) => (spec, ChangeOrigin::Submitted),
        None => {
            let mut spec = ChangeSpec::program_upgrade(&manifest.program_id, &candidate);
            spec.metadata.label = upload.label.clone();
            (spec, ChangeOrigin::DerivedFromCandidate)
        }
    };
    let binding = ci::bind_change(&bundle_dir, &spec).map_err(|error| {
        let status = match error {
            ci::CheckError::Bundle(_) => StatusCode::CONFLICT,
            ci::CheckError::Configuration(_) => StatusCode::BAD_REQUEST,
        };
        ApiError::new(status, format!("{error}")).with_exit_code(error.exit_code())
    })?;
    spec.resolve(CandidateSource::Bytes(&candidate))
        .map_err(|error| {
            ApiError::bad_request(format!("{error:#}")).with_exit_code(ci::EXIT_ERROR)
        })?;
    let change = RunChange::of(&spec, origin)
        .map_err(|error| ApiError::internal(format!("identifying the change: {error}")))?;
    if binding.change_spec_id != change.change_spec_id {
        return Err(ApiError::internal(
            "the bound change and the resolved change disagree",
        ));
    }

    // ---- durable inputs, then durable intent --------------------------------
    //
    // Every input the worker will read is persisted, content-addressed or
    // hash-pinned, before the run record exists; the run record is the last
    // thing written, and the 202 follows it. So an accepted run never names an
    // input the service does not hold, and a restart at any point before the
    // record leaves no run at all rather than one that cannot execute.
    let stored = state
        .registry
        .artifacts()
        .put_program(&candidate)
        .map_err(|error| ApiError::internal(format!("storing the candidate: {error:#}")))?;
    if !stored.reference.matches(spec.candidate()) {
        return Err(ApiError::internal(
            "the stored artifact is not the spec's candidate",
        ));
    }
    state
        .registry
        .index_project_artifact(&project.project_id, &stored.reference.sha256)
        .map_err(|error| ApiError::internal(format!("indexing the artifact: {error}")))?;

    let run_id = new_run_id();
    state
        .registry
        .save_change_spec(&run_id, &spec)
        .map_err(|error| ApiError::internal(format!("persisting the change spec: {error}")))?;
    let expectations_sha256 =
        match &upload.expectations {
            Some(bytes) => Some(state.registry.save_expectations(&run_id, bytes).map_err(
                |error| ApiError::internal(format!("persisting the expectations: {error}")),
            )?),
            None => None,
        };

    let metadata = RunMetadata {
        run_id: run_id.clone(),
        project_id: project.project_id.clone(),
        status: RunStatus::Queued,
        bundle_sha256: manifest.bundle_sha256.clone(),
        bundle_id: Some(active.bundle_id.clone()),
        corpus_sha256: Some(manifest.corpus_sha256.clone()),
        baseline_sha256: Some(manifest.baseline_program_sha256.clone()),
        candidate_sha256: change.candidate_sha256.clone(),
        change: Some(change.clone()),
        candidate_artifact: Some(stored.reference.clone()),
        expectations_sha256,
        attempts: Vec::new(),
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

    // Indexed before the record exists: an index entry whose run never got
    // written is skipped by every listing, while a run that exists but is in no
    // index would be invisible in history.
    state
        .registry
        .index_run(&project.project_id, &run_id)
        .map_err(|error| ApiError::internal(format!("indexing the run: {error}")))?;
    state
        .registry
        .index_change(&project.project_id, &change.change_spec_id, &run_id)
        .map_err(|error| ApiError::internal(format!("indexing the change: {error}")))?;
    // The run record is the durable queue entry. Only once it exists is a
    // worker started, so there is no window in which work exists that nothing
    // can be told about, and a restart from here re-enqueues it.
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
            candidate_sha256: change.candidate_sha256.clone(),
            change,
            candidate_artifact: stored.reference,
            bundle_sha256: manifest.bundle_sha256.clone(),
        }),
    )
        .into_response())
}

/// The candidate an explicit spec names, from what this project has already
/// supplied. Scoped to the project on purpose: the store is shared and
/// deduplicated, and knowing another project's candidate hash must not be
/// enough to execute or detect its bytes.
fn retained_candidate(state: &AppState, project_id: &str, spec: &ChangeSpec) -> ApiResult<Vec<u8>> {
    let wanted = ArtifactRef::from(spec.candidate());
    let held = state
        .registry
        .project_holds_artifact(project_id, &wanted.sha256)
        .unwrap_or(false);
    if !held {
        return Err(ApiError::bad_request(format!(
            "the change spec names candidate {}, which this project has not supplied; \
             upload it as the `candidate` part",
            wanted.sha256
        ))
        .with_exit_code(ci::EXIT_ERROR));
    }
    state
        .registry
        .artifacts()
        .get_program(&wanted)
        .map_err(|error| {
            ApiError::internal(format!("the retained candidate does not verify: {error:#}"))
        })
}

#[derive(Serialize)]
struct ArtifactView {
    sha256: String,
    len: u64,
    /// Held immutably, verified on this read. Never a path.
    retained: bool,
}

/// Whether this project's candidate `sha256` is held, verified now.
///
/// Metadata only: no bytes, no storage path, no mutation. A hash the project
/// never supplied answers exactly like one the service has never seen.
async fn get_artifact(
    State(state): State<Shared>,
    Path((project_id, sha256)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    project_for(&state, &project_id, &headers)?;
    if !crate::artifacts::canonical_sha256(&sha256) {
        return Err(ApiError::bad_request(
            "an artifact is named by 64 lowercase hex characters",
        ));
    }
    let held = state
        .registry
        .project_holds_artifact(&project_id, &sha256)
        .unwrap_or(false);
    if !held {
        return Err(ApiError::not_found("artifact"));
    }
    let reference = state
        .registry
        .artifacts()
        .describe(&sha256)
        .map_err(|error| ApiError::internal(format!("the artifact does not verify: {error:#}")))?
        .ok_or_else(|| ApiError::not_found("artifact"))?;
    Ok((
        StatusCode::OK,
        Json(ArtifactView {
            sha256: reference.sha256,
            len: reference.len,
            retained: true,
        }),
    )
        .into_response())
}

/// What a check request carried. Every part is optional here; which
/// combinations are meaningful is `create_check`'s decision.
struct Upload {
    candidate: Option<Bytes>,
    expectations: Option<Vec<u8>>,
    change_spec: Option<Vec<u8>>,
    label: Option<String>,
}

/// Read and bound the uploaded parts.
///
/// Sizes are checked as bytes arrive rather than after, and anything the
/// request names that we do not expect is refused rather than ignored. The
/// contract is additive: `candidate` and `expected_changes` mean what they
/// always meant, and `change_spec` and `label` are new and optional.
async fn read_upload(state: &AppState, mut multipart: Multipart) -> ApiResult<Upload> {
    let mut upload = Upload {
        candidate: None,
        expectations: None,
        change_spec: None,
        label: None,
    };
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
        let slot_taken = match name.as_str() {
            "candidate" => {
                if bytes.len() > state.config.max_candidate_bytes {
                    return Err(ApiError::too_large(
                        "candidate",
                        state.config.max_candidate_bytes,
                    ));
                }
                upload.candidate.replace(bytes).is_some()
            }
            "expected_changes" => {
                if bytes.len() > state.config.max_expectation_bytes {
                    return Err(ApiError::too_large(
                        "expected_changes",
                        state.config.max_expectation_bytes,
                    ));
                }
                upload.expectations.replace(bytes.to_vec()).is_some()
            }
            "change_spec" => {
                if bytes.len() > MAX_CHANGE_SPEC_BYTES {
                    return Err(ApiError::too_large("change_spec", MAX_CHANGE_SPEC_BYTES));
                }
                upload.change_spec.replace(bytes.to_vec()).is_some()
            }
            "label" => {
                let label = String::from_utf8(bytes.to_vec())
                    .map_err(|_| ApiError::bad_request("label must be UTF-8 text"))?;
                let label = label.trim().to_string();
                if label.chars().count() > MAX_LABEL_CHARS || label.chars().any(char::is_control) {
                    return Err(ApiError::bad_request(format!(
                        "label must be at most {MAX_LABEL_CHARS} characters with no control characters"
                    )));
                }
                // An empty label is no label, not a label that is empty.
                !label.is_empty() && upload.label.replace(label).is_some()
            }
            other => {
                return Err(ApiError::bad_request(format!(
                    "unexpected upload field {other:?}"
                )))
            }
        };
        // Two candidates in one request would leave which one was meant to the
        // order the parts happened to arrive in.
        if slot_taken {
            return Err(ApiError::bad_request(format!(
                "upload field {name:?} was sent more than once"
            )));
        }
    }
    Ok(upload)
}

/// A run belongs to one project. Its own live token may read it, and so may
/// the operator.
///
/// A foreign credential gets `401`, not `404`, and that is deliberate: it is
/// matched only against the owning project's tokens, so it fails identically
/// whether the run exists, belongs to someone else, or never existed. Searching
/// every project's tokens to answer `404` instead would cost a verification per
/// project and reveal nothing that this does not already withhold.
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

/// The run's canonical proposal, on request.
///
/// Read through the registry, which recomputes the stored document's identity
/// and requires it to be the one the run was indexed under. A stored spec that
/// fails that check is never served as though it described the run.
async fn get_change_spec(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let metadata = authorize_run(&state, &run_id, &headers)?;
    let change = metadata.change.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "this run was recorded before change identity existed and has no change spec",
        )
    })?;
    let spec = state
        .registry
        .load_change_spec(&run_id, change)
        .map_err(|error| ApiError::internal(format!("{error:#}")))?;
    let document = spec
        .to_document()
        .map_err(|error| ApiError::internal(format!("{error:#}")))?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        document,
    )
        .into_response())
}

/// Opaque, ordered, and never derived from anything secret.
///
/// Seconds were not enough: two runs accepted in the same second ordered by
/// their random tail, so "newest first" was only true when a project was quiet.
fn new_run_id() -> String {
    crate::ids::run()
}

// ---------------------------------------------------------------- projects
//
// Explicit response shapes throughout. Serializing a stored record straight to
// the wire would make every internal field a public promise, and would leak the
// next private one added to it by accident.

#[derive(Serialize)]
struct AdapterView {
    adapter_id: String,
    name: String,
    version: u32,
    program_id: Option<String>,
    speaks_semantics: bool,
}

/// What this build can onboard, derived from the engine's own registry.
async fn list_adapters(State(state): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    operator(&state, &headers)?;
    let adapters: Vec<AdapterView> = AdapterId::supported()
        .into_iter()
        .map(|id| {
            let program_id = eplyx_engine::protocol::adapters()
                .iter()
                .find(|adapter| adapter.name() == id.name)
                .map(|adapter| adapter.program_id().to_string());
            AdapterView {
                adapter_id: id.to_string(),
                name: id.name.clone(),
                version: id.version,
                program_id,
                speaks_semantics: id.speaks_semantics(),
            }
        })
        .collect();
    Ok((
        StatusCode::OK,
        Json(json!({
            "adapters": adapters,
            "semantic_schema_version": eplyx_engine::semantics::SEMANTIC_SCHEMA_VERSION,
        })),
    )
        .into_response())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProjectRequest {
    name: String,
    program_id: String,
    adapter_id: String,
}

#[derive(Serialize)]
struct ActiveBundleView {
    bundle_id: String,
    bundle_sha256: String,
    activated_at_unix_seconds: u64,
}

#[derive(Serialize)]
struct ProjectView {
    project_id: String,
    name: String,
    chain: Chain,
    program_id: String,
    adapter_id: String,
    status: ProjectStatus,
    speaks_semantics: bool,
    active_bundle: Option<ActiveBundleView>,
    created_at_unix_seconds: u64,
    updated_at_unix_seconds: u64,
}

impl From<&Project> for ProjectView {
    fn from(project: &Project) -> Self {
        Self {
            project_id: project.project_id.clone(),
            name: project.name.clone(),
            chain: project.chain,
            program_id: project.program_id.clone(),
            adapter_id: project.adapter_id.to_string(),
            status: project.status,
            // Surfaced rather than implied: a project on `none@0` runs checks
            // that report no semantic coverage, and a team should see that
            // before it reads a red gate as a finding.
            speaks_semantics: project.adapter_id.speaks_semantics(),
            active_bundle: project
                .active_bundle
                .as_ref()
                .map(|active| ActiveBundleView {
                    bundle_id: active.bundle_id.clone(),
                    bundle_sha256: active.bundle_sha256.clone(),
                    activated_at_unix_seconds: active.activated_at_unix_seconds,
                }),
            created_at_unix_seconds: project.created_at_unix_seconds,
            updated_at_unix_seconds: project.updated_at_unix_seconds,
        }
    }
}

async fn create_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    operator(&state, &headers)?;
    let request: CreateProjectRequest = parse_json(&body)?;
    validate_name(&request.name).map_err(|error| ApiError::bad_request(format!("{error}")))?;
    validate_program_id(&request.program_id)
        .map_err(|error| ApiError::bad_request(format!("{error}")))?;
    let adapter_id = AdapterId::try_from(request.adapter_id.clone())
        .map_err(|error| ApiError::bad_request(format!("{error}")))?;
    if !AdapterId::supported().contains(&adapter_id) {
        return Err(ApiError::bad_request(format!(
            "adapter {adapter_id} is not one this build speaks"
        )));
    }

    let project = Project::new(
        &crate::ids::project(),
        &request.name,
        &request.program_id,
        adapter_id,
    )
    .map_err(|error| ApiError::bad_request(format!("{error}")))?;
    state
        .registry
        .create_project(&project)
        .map_err(|error| ApiError::internal(format!("persisting the project: {error}")))?;
    Ok((StatusCode::CREATED, Json(ProjectView::from(&project))).into_response())
}

async fn list_projects(State(state): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    operator(&state, &headers)?;
    let projects = state
        .registry
        .list_projects()
        .map_err(|error| ApiError::internal(format!("listing projects: {error}")))?;
    let views: Vec<ProjectView> = projects.iter().map(ProjectView::from).collect();
    Ok((StatusCode::OK, Json(json!({ "projects": views }))).into_response())
}

/// One project, with cheap run statistics.
///
/// Cheap means the index, not the history: the newest run id is the first entry
/// of a sorted directory listing, and only that one record is read.
async fn get_project(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers)?;
    let run_ids = state
        .registry
        .project_run_ids(&project_id)
        .unwrap_or_default();
    let last = run_ids
        .first()
        .and_then(|id| state.registry.load_run(id).ok());
    Ok((
        StatusCode::OK,
        Json(json!({
            "project": ProjectView::from(&project),
            "run_count": run_ids.len(),
            "last_run_id": last.as_ref().map(|run| run.run_id.clone()),
            "last_run_status": last.as_ref().map(|run| run.status),
        })),
    )
        .into_response())
}

/// Resolve a project for a caller entitled to see it.
///
/// The operator sees any; a project token sees only its own, because it is only
/// ever matched against that project's tokens.
fn project_for(state: &AppState, project_id: &str, headers: &HeaderMap) -> ApiResult<Project> {
    match authenticate(state, project_id, headers)? {
        Principal::Operator => state
            .registry
            .load_project(project_id)
            .map_err(|_| ApiError::not_found("project")),
        Principal::Project { project, .. } => Ok(*project),
    }
}

// ------------------------------------------------------------------ tokens

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTokenRequest {
    label: String,
}

#[derive(Serialize)]
struct TokenView {
    token_id: String,
    label: String,
    created_at_unix_seconds: u64,
    last_used_at_unix_seconds: Option<u64>,
    revoked_at_unix_seconds: Option<u64>,
}

impl From<&ProjectToken> for TokenView {
    fn from(token: &ProjectToken) -> Self {
        Self {
            token_id: token.token_id.clone(),
            label: token.label.clone(),
            created_at_unix_seconds: token.created_at_unix_seconds,
            last_used_at_unix_seconds: token.last_used_at_unix_seconds,
            revoked_at_unix_seconds: token.revoked_at_unix_seconds,
        }
    }
}

/// Issue a token. The secret appears in this response and nowhere else, ever.
async fn create_token(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let principal = authenticate(&state, &project_id, &headers)?;
    require_operator(&principal)?;
    let request: CreateTokenRequest = parse_json(&body)?;
    state
        .registry
        .load_project(&project_id)
        .map_err(|_| ApiError::not_found("project"))?;

    let secret = crate::project::generate_token();
    let token = ProjectToken::new(&crate::ids::token(), &project_id, &request.label, &secret)
        .map_err(|error| ApiError::bad_request(format!("{error}")))?;
    state
        .registry
        .create_token(&token)
        .map_err(|error| ApiError::internal(format!("persisting the token: {error}")))?;

    let mut body = serde_json::to_value(TokenView::from(&token)).unwrap_or(json!({}));
    body["token"] = json!(secret);
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

async fn list_tokens(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let principal = authenticate(&state, &project_id, &headers)?;
    require_operator(&principal)?;
    let tokens = state
        .registry
        .list_tokens(&project_id)
        .map_err(|error| ApiError::internal(format!("listing tokens: {error}")))?;
    let views: Vec<TokenView> = tokens.iter().map(TokenView::from).collect();
    Ok((StatusCode::OK, Json(json!({ "tokens": views }))).into_response())
}

async fn revoke_token(
    State(state): State<Shared>,
    Path((project_id, token_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let principal = authenticate(&state, &project_id, &headers)?;
    require_operator(&principal)?;
    let token = state
        .registry
        .revoke_token(&project_id, &token_id)
        .map_err(|_| ApiError::not_found("token"))?;
    Ok((StatusCode::OK, Json(TokenView::from(&token))).into_response())
}

// ----------------------------------------------------------------- bundles

#[derive(Serialize)]
struct BundleView {
    bundle_id: String,
    bundle_sha256: String,
    baseline_sha256: String,
    program_id: String,
    adapter_id: String,
    semantic_schema_version: u32,
    record_count: usize,
    source_filename: Option<String>,
    created_at_unix_seconds: u64,
    active: bool,
}

fn bundle_view(record: &ProjectBundle, project: &Project) -> BundleView {
    BundleView {
        bundle_id: record.bundle_id.clone(),
        bundle_sha256: record.bundle_sha256.clone(),
        baseline_sha256: record.baseline_sha256.clone(),
        program_id: record.program_id.clone(),
        adapter_id: record.adapter_id.to_string(),
        semantic_schema_version: record.semantic_schema_version,
        record_count: record.record_count,
        source_filename: record.source_filename.clone(),
        created_at_unix_seconds: record.created_at_unix_seconds,
        // Derived from the project's pointer, never stored on the bundle: a
        // bundle does not know whether it is in use, and two records claiming
        // to be active would be a contradiction nothing could resolve.
        active: project
            .active_bundle
            .as_ref()
            .is_some_and(|active| active.bundle_id == record.bundle_id),
    }
}

/// Register an uploaded bundle.
///
/// The bundle arrives as one multipart part per file, named by its path inside
/// the bundle, so a browser can send the directory the CLI produced without
/// anything being archived, and so this service needs no archive format of its
/// own. Every path is checked before it becomes one.
async fn create_bundle(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    multipart: Multipart,
) -> ApiResult<Response> {
    let principal = authenticate(&state, &project_id, &headers)?;
    require_operator(&principal)?;
    let project = state
        .registry
        .load_project(&project_id)
        .map_err(|_| ApiError::not_found("project"))?;

    let staging = tempfile::Builder::new()
        .prefix("eplyx-bundle-")
        .tempdir()
        .map_err(|error| ApiError::internal(format!("staging: {error}")))?;
    let filename = read_bundle_upload(&state, multipart, staging.path()).await?;

    // Verification is the engine's, not this layer's. `register_bundle` opens
    // the uploaded tree through `CiBundle::open` and refuses anything that does
    // not verify, belong to this program, or match the declared adapter.
    let record = state
        .registry
        .register_bundle(&project, staging.path(), filename.as_deref())
        .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, format!("{error:#}")))?;

    Ok((StatusCode::CREATED, Json(bundle_view(&record, &project))).into_response())
}

async fn list_bundles(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers)?;
    let bundles = state
        .registry
        .list_bundles(&project_id)
        .map_err(|error| ApiError::internal(format!("listing bundles: {error}")))?;
    let views: Vec<BundleView> = bundles
        .iter()
        .map(|record| bundle_view(record, &project))
        .collect();
    Ok((StatusCode::OK, Json(json!({ "bundles": views }))).into_response())
}

/// Move the project's pointer. Nothing is deleted, and history is kept.
async fn activate_bundle(
    State(state): State<Shared>,
    Path((project_id, bundle_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let principal = authenticate(&state, &project_id, &headers)?;
    require_operator(&principal)?;
    let project = state
        .registry
        .activate_bundle(&project_id, &bundle_id)
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, format!("{error:#}")))?;
    Ok((StatusCode::OK, Json(ProjectView::from(&project))).into_response())
}

// ------------------------------------------------------------- run history

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RunQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    status: Option<RunStatus>,
    #[serde(default)]
    exit_code: Option<u8>,
    /// Every analysis of one proposal. Served from its own index, so a
    /// governance binding can find the runs for the change being signed.
    #[serde(default)]
    change_spec_id: Option<String>,
}

#[derive(Serialize)]
struct RunSummaryView {
    run_id: String,
    status: RunStatus,
    exit_code: Option<u8>,
    created_at_unix_seconds: u64,
    started_at_unix_seconds: Option<u64>,
    completed_at_unix_seconds: Option<u64>,
    candidate_sha256: String,
    bundle_sha256: String,
    bundle_id: Option<String>,
    /// `null` marks a legacy run, recorded before change identity.
    change: Option<RunChange>,
    report_available: bool,
}

/// A project's runs, newest first.
///
/// The cursor is a run id, and paging means "everything after this one in the
/// ordering". Ids lead with their own minting time, so that is both stable
/// under concurrent inserts and free to compute — a run created mid-page does
/// not shift what the next page contains.
async fn list_project_runs(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<RunQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    project_for(&state, &project_id, &headers)?;
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let ids = match &query.change_spec_id {
        Some(id) => {
            if id.len() != 64 || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return Err(ApiError::bad_request(
                    "change_spec_id must be 64 lowercase hex characters",
                ));
            }
            state.registry.change_run_ids(&project_id, id)
        }
        None => state.registry.project_run_ids(&project_id),
    }
    .map_err(|error| ApiError::internal(format!("listing runs: {error}")))?;

    let mut runs = Vec::new();
    let mut next_cursor = None;
    for id in ids {
        if let Some(cursor) = &query.cursor {
            if id.as_str() >= cursor.as_str() {
                continue;
            }
        }
        let Ok(run) = state.registry.load_run(&id) else {
            continue;
        };
        if query.status.is_some_and(|want| want != run.status) {
            continue;
        }
        if query.exit_code.is_some() && query.exit_code != run.exit_code {
            continue;
        }
        if runs.len() == limit {
            next_cursor = Some(runs.last().map(|last: &RunSummaryView| last.run_id.clone()));
            break;
        }
        runs.push(RunSummaryView {
            run_id: run.run_id,
            status: run.status,
            exit_code: run.exit_code,
            created_at_unix_seconds: run.created_at_unix_seconds,
            started_at_unix_seconds: run.started_at_unix_seconds,
            completed_at_unix_seconds: run.completed_at_unix_seconds,
            candidate_sha256: run.candidate_sha256,
            bundle_sha256: run.bundle_sha256,
            bundle_id: run.bundle_id,
            change: run.change,
            report_available: run.report_available,
        });
    }
    Ok((
        StatusCode::OK,
        Json(json!({ "runs": runs, "next_cursor": next_cursor.flatten() })),
    )
        .into_response())
}

/// Read an uploaded bundle into a staging directory.
///
/// Each part is named by its path inside the bundle. That name is the one thing
/// a caller controls that could become a filesystem path, so it is validated
/// rather than sanitised: anything absolute, anything with a traversal segment,
/// anything with a character outside a narrow set, and anything unreasonably
/// deep is refused outright.
async fn read_bundle_upload(
    state: &AppState,
    mut multipart: Multipart,
    into: &std::path::Path,
) -> ApiResult<Option<String>> {
    let mut total = 0_usize;
    let mut files = 0_usize;
    let mut first_name = None;
    loop {
        let field = multipart.next_field().await.map_err(|error| {
            ApiError::new(error.status(), format!("malformed multipart: {error}"))
        })?;
        let Some(field) = field else { break };
        let name = field.name().unwrap_or_default().to_string();
        let filename = field.file_name().map(str::to_string);
        let bytes = field
            .bytes()
            .await
            .map_err(|error| ApiError::new(error.status(), format!("upload rejected: {error}")))?;

        let relative = bundle_member_path(&name)
            .ok_or_else(|| ApiError::bad_request(format!("unsafe bundle path {name:?}")))?;
        total += bytes.len();
        files += 1;
        if total > state.config.max_bundle_bytes {
            return Err(ApiError::too_large("bundle", state.config.max_bundle_bytes));
        }
        if files > 512 {
            return Err(ApiError::bad_request("a bundle with more than 512 files"));
        }
        let destination = into.join(&relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ApiError::internal(format!("staging: {error}")))?;
        }
        std::fs::write(&destination, &bytes)
            .map_err(|error| ApiError::internal(format!("staging: {error}")))?;
        if first_name.is_none() {
            first_name = filename;
        }
    }
    if files == 0 {
        return Err(ApiError::bad_request("no bundle files were uploaded"));
    }
    Ok(first_name)
}

/// A relative path inside a bundle, or nothing.
fn bundle_member_path(name: &str) -> Option<std::path::PathBuf> {
    if name.is_empty() || name.len() > 200 || name.starts_with('/') || name.contains('\\') {
        return None;
    }
    let segments: Vec<&str> = name.split('/').collect();
    if segments.len() > 4 {
        return None;
    }
    let mut path = std::path::PathBuf::new();
    for segment in segments {
        if segment.is_empty() || segment == "." || segment == ".." || segment.len() > 100 {
            return None;
        }
        if !segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
        {
            return None;
        }
        path.push(segment);
    }
    Some(path)
}
