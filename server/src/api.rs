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
use eplyx_engine::change::{CandidateSource, ChangeSpec, ExecutableArtifact};
use eplyx_engine::ci;
use eplyx_engine::governance::attestation::{self, DeploymentAttestation};
use eplyx_engine::governance::{
    self, BindingOutcome, Commitment, GovernanceBinding, SquadsProposalRef,
};
use eplyx_engine::ingest::rpc::RpcProvider;
use serde::Serialize;
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::artifacts::ArtifactRef;
use crate::config::Config;
use crate::project::{
    validate_name, validate_program_id, AdapterId, Chain, Project, ProjectStatus, ProjectToken,
};
use crate::registry::{
    now_unix_seconds, ChangeOrigin, GovernanceCheck, ProjectBundle, Registry, RunChange,
    RunMetadata, RunStatus,
};
use crate::worker;

pub struct AppState {
    pub observation: Option<crate::hosted::observation::Service>,
    pub identity: Option<crate::cloud::Identity>,
    pub config: Config,
    pub registry: Registry,
    /// Replay is CPU-bound and synchronous. The permit count bounds how many
    /// run at once on a pilot host; a queue is deliberately not built yet.
    pub runs: tokio::sync::Semaphore,
    /// The chain, for governance verification only. `None` turns that
    /// endpoint off; no run ever reads it.
    pub governance: Option<Arc<dyn RpcProvider + Send + Sync>>,
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
    pub(crate) fn new(status: StatusCode, message: impl Into<String>) -> Self {
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
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    fn unauthorized() -> Self {
        // Deliberately uniform: whether the project exists, whether the token
        // was malformed, and whether it simply did not match all look the same
        // from outside.
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized")
    }
    pub(crate) fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("no such {what}"))
    }
    fn too_large(what: &str, limit: usize) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("{what} exceeds the {limit} byte limit"),
        )
    }
    pub(crate) fn internal(message: impl Into<String>) -> Self {
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

pub(crate) type ApiResult<T> = std::result::Result<T, ApiError>;

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
        .merge(crate::cloud::router())
        .merge(crate::hosted::observation::router())
        .merge(crate::hosted::order::router())
        .merge(crate::hosted::interaction::router())
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/v1/adapters", get(list_adapters))
        .route("/v1/projects", post(create_project).get(list_projects))
        .route("/v1/projects/{project_id}", get(get_project))
        .route(
            "/v1/projects/{project_id}/capabilities",
            get(get_project_capabilities),
        )
        .route(
            "/v1/projects/{project_id}/workspace-binding",
            post(assign_workspace),
        )
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
        .route(
            "/v1/projects/{project_id}/governance/squads/verify",
            post(verify_squads_governance),
        )
        .route(
            "/v1/projects/{project_id}/governance/squads/attest",
            post(attest_squads_governance),
        )
        .route(
            "/v1/projects/{project_id}/governance/changes/{change_spec_id}/trail",
            get(governance_trail),
        )
        .route(
            "/v1/projects/{project_id}/governance/changes/{change_spec_id}",
            get(list_governance_checks),
        )
        .route(
            "/v1/projects/{project_id}/runs/{parent_run}/parameter-changes",
            post(crate::hosted::parameter::submit),
        )
        .route(
            "/v1/projects/{project_id}/runs/{parent_run}/parameter-change/eligibility",
            get(crate::hosted::parameter::eligibility),
        )
        .route("/v1/runs/{run_id}", get(get_run))
        .route("/v1/runs/{run_id}/report.json", get(get_report_json))
        .route("/v1/runs/{run_id}/report.md", get(get_report_markdown))
        .route("/v1/runs/{run_id}/change_spec.json", get(get_change_spec))
        .layer(DefaultBodyLimit::max(limit))
        .layer(cors)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::cloud::cookie_write_guard,
        ))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

/// Ready means the persistent volume is actually usable. Nothing about the
/// configuration itself is exposed.
async fn ready(State(state): State<Shared>) -> impl IntoResponse {
    if let Some(identity) = &state.identity {
        let reachable = match identity.db.get().await {
            Ok(db) => db.query_one("SELECT 1", &[]).await.is_ok(),
            Err(_) => false,
        };
        if !reachable {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"status":"identity storage unavailable"})),
            );
        }
    }
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
    Member {
        project: Box<Project>,
    },
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
async fn authenticate(
    state: &AppState,
    project_id: &str,
    headers: &HeaderMap,
) -> ApiResult<Principal> {
    if let Ok(secret) = bearer(headers) {
        if state
            .config
            .operator_token
            .as_deref()
            .is_some_and(|c| constant_time_eq(c, secret))
        {
            return Ok(Principal::Operator);
        }
        if let (Ok(project), Ok(token)) = (
            state.registry.load_project(project_id),
            state.registry.authenticate_token(project_id, secret),
        ) {
            state.registry.note_token_use(&token);
            return Ok(Principal::Project {
                project: Box::new(project),
                token: Box::new(token),
            });
        }
    }
    if state.identity.is_none() {
        return Err(ApiError::unauthorized());
    }
    let principal = crate::cloud::auth::require_for(state, headers, project_id)
        .await
        .map_err(cloud_error)?;
    crate::cloud::workspaces::project_access(state, &principal, project_id)
        .await
        .map_err(cloud_error)?;
    let project = state
        .registry
        .load_project(project_id)
        .map_err(|_| ApiError::not_found("project"))?;
    Ok(Principal::Member {
        project: Box::new(project),
    })
}
fn cloud_error(error: crate::cloud::error::ApiError) -> ApiError {
    ApiError::new(error.status, error.message)
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
    let principal = authenticate(&state, &project_id, &headers).await?;
    let project = match &principal {
        Principal::Project { project, .. } | Principal::Member { project } => (**project).clone(),
        Principal::Operator => state
            .registry
            .load_project(&project_id)
            .map_err(|_| ApiError::not_found("project"))?,
    };
    // This is the same authoritative project state exposed by the capability
    // endpoint, checked again at the mutation boundary. In particular, a
    // prepared analytical submission that was opened while ready must report
    // a readiness conflict—not a generic input error—if the project was
    // disabled before POST.
    if project.status == ProjectStatus::Disabled {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "this project is disabled and accepts no checks",
        ));
    }
    let upload = read_upload(&state, multipart).await?;
    if let Some(bytes) = &upload.change_spec {
        let spec = ChangeSpec::parse(bytes).map_err(|_| {
            ApiError::bad_request("invalid change spec").with_exit_code(ci::EXIT_ERROR)
        })?;
        if !matches!(
            spec.change,
            eplyx_engine::change::Change::ProgramUpgrade { .. }
        ) {
            return create_analytical_check(state, &project.project_id, spec, upload).await;
        }
    }
    if !upload.analysis.is_empty() {
        return Err(ApiError::bad_request(
            "analytical inputs require an explicit analytical change spec",
        ));
    }

    // Readiness is a hosted configuration question, answered before a run
    // exists. It is deliberately not an Eplyx exit code: nothing was measured,
    // so there is no verdict to report about the candidate.
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
    if !stored.reference.matches(executable_candidate(&spec)?) {
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
        order_failure: None,
        hosted_analysis: None,
        analysis: None,
        run_id: run_id.clone(),
        project_id: project.project_id.clone(),
        status: RunStatus::Queued,
        bundle_sha256: manifest.bundle_sha256.clone(),
        bundle_id: Some(active.bundle_id.clone()),
        corpus_sha256: Some(manifest.corpus_sha256.clone()),
        baseline_sha256: Some(manifest.baseline_program_sha256.clone()),
        candidate_sha256: stored.reference.sha256.clone(),
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
            candidate_sha256: stored.reference.sha256.clone(),
            change,
            candidate_artifact: stored.reference,
            bundle_sha256: manifest.bundle_sha256.clone(),
        }),
    )
        .into_response())
}

fn executable_candidate(spec: &ChangeSpec) -> ApiResult<&ExecutableArtifact> {
    spec.candidate().ok_or_else(|| {
        ApiError::bad_request("this operation requires an executable candidate")
            .with_exit_code(ci::EXIT_ERROR)
    })
}

/// The candidate an explicit spec names, from what this project has already
/// supplied. Scoped to the project on purpose: the store is shared and
/// deduplicated, and knowing another project's candidate hash must not be
/// enough to execute or detect its bytes.
fn retained_candidate(state: &AppState, project_id: &str, spec: &ChangeSpec) -> ApiResult<Vec<u8>> {
    let wanted = ArtifactRef::from(executable_candidate(spec)?);
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
    project_for(&state, &project_id, &headers).await?;
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
    analysis: std::collections::BTreeMap<String, Vec<u8>>,
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
        analysis: std::collections::BTreeMap::new(),
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
            "state_input"
            | "state_artifact"
            | "snapshot"
            | "scenario"
            | "analysis_options"
            | "lifecycle_evidence"
            | "pinned_program_capture" => {
                let limit = if matches!(
                    name.as_str(),
                    "state_artifact" | "snapshot" | "lifecycle_evidence" | "pinned_program_capture"
                ) {
                    state.config.max_bundle_bytes
                } else {
                    MAX_CHANGE_SPEC_BYTES
                };
                if bytes.len() > limit {
                    return Err(ApiError::too_large(&name, limit));
                }
                upload
                    .analysis
                    .insert(name.clone(), bytes.to_vec())
                    .is_some()
            }
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
async fn authorize_run(
    state: &AppState,
    run_id: &str,
    headers: &HeaderMap,
) -> ApiResult<RunMetadata> {
    let metadata = state
        .registry
        .load_run(run_id)
        .map_err(|_| ApiError::not_found("run"))?;
    authenticate(state, &metadata.project_id, headers).await?;
    Ok(metadata)
}

async fn get_run(
    State(state): State<Shared>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let metadata = authorize_run(&state, &run_id, &headers).await?;
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
    let metadata = authorize_run(&state, &run_id, &headers).await?;
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
    let metadata = authorize_run(&state, &run_id, &headers).await?;
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
    let metadata = authorize_run(&state, &run_id, &headers).await?;
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

// -------------------------------------------------------------- governance
//
// Verification only. Nothing here signs, approves, rejects, cancels or
// executes a proposal, and the service holds no key that could. Every answer
// is re-read from the chain on request; a stored binding is shown as what it
// was, at its slot, never replayed as a current answer.

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SquadsVerifyRequest {
    multisig: String,
    transaction_index: u64,
    #[serde(default)]
    change_spec_id: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    commitment: Option<Commitment>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SquadsAttestRequest {
    change_spec_id: String,
    binding_id: String,
}

/// Check a previously bound change against the exact successful Squads
/// execution, then persist a separate immutable post-execution proof.
async fn attest_squads_governance(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    let request: SquadsAttestRequest = parse_json(&body)?;
    if !crate::artifacts::canonical_sha256(&request.change_spec_id)
        || !crate::artifacts::canonical_sha256(&request.binding_id)
    {
        return Err(ApiError::bad_request(
            "change_spec_id and binding_id must be canonical SHA-256 identifiers",
        ));
    }
    let rpc = state.governance.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance verification is not configured on this server (EPLYX_GOVERNANCE_RPC_URL)",
        )
    })?;
    let spec = state
        .registry
        .load_governance_spec(&project.project_id, &request.change_spec_id)
        .map_err(|e| ApiError::internal(format!("loading governance-bound change: {e:#}")))?
        .ok_or_else(|| ApiError::not_found("governance-bound change"))?;
    let binding_path = state
        .registry
        .storage()
        .project_governance_dir(&project.project_id, &request.change_spec_id)
        .map_err(|_| ApiError::internal("governance storage unavailable"))?
        .join("bindings")
        .join(format!("{}.json", request.binding_id));
    if !binding_path.is_file() {
        return Err(ApiError::not_found("matched G1 binding for this change"));
    }
    let binding = state
        .registry
        .governance_binding(
            &project.project_id,
            &request.change_spec_id,
            &request.binding_id,
        )
        .map_err(|_| ApiError::internal("stored G1 binding integrity check failed"))?;
    if !state
        .registry
        .project_holds_artifact(&project.project_id, &executable_candidate(&spec)?.sha256)
        .map_err(|e| ApiError::internal(format!("checking candidate ownership: {e:#}")))?
    {
        return Err(ApiError::not_found("candidate artifact in this project"));
    }
    let candidate = state
        .registry
        .artifacts()
        .get_program(&ArtifactRef::from(executable_candidate(&spec)?))
        .map_err(|e| ApiError::internal(format!("candidate artifact does not verify: {e:#}")))?;
    let attestation: DeploymentAttestation = tokio::task::spawn_blocking(move || {
        attestation::attest_squads_upgrade(rpc.as_ref(), &spec, &binding, &candidate)
    })
    .await
    .map_err(|e| ApiError::internal(format!("deployment attestation stopped: {e}")))?
    .map_err(|e| {
        ApiError::bad_request(format!("deployment attestation precondition failed: {e:#}"))
    })?;
    state
        .registry
        .record_deployment_attestation(&project.project_id, &attestation)
        .map_err(|e| ApiError::internal(format!("recording deployment attestation: {e:#}")))?;
    Ok((StatusCode::OK, Json(attestation)).into_response())
}

/// The analysed spec a request names: a run of this project, or a change this
/// project has analysed or bound before. Always re-verified on read.
fn analysed_spec(
    state: &AppState,
    project_id: &str,
    request: &SquadsVerifyRequest,
) -> ApiResult<ChangeSpec> {
    let from_run = |run_id: &str| -> ApiResult<ChangeSpec> {
        let metadata = state
            .registry
            .load_run(run_id)
            .ok()
            .filter(|run| run.project_id == project_id)
            .ok_or_else(|| ApiError::not_found("run"))?;
        let change = metadata.change.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "this run was recorded before change identity existed and cannot be bound",
            )
        })?;
        state
            .registry
            .load_change_spec(run_id, change)
            .map_err(|error| ApiError::internal(format!("{error:#}")))
    };
    match (&request.run_id, &request.change_spec_id) {
        (Some(run_id), None) => from_run(run_id),
        (None, Some(id)) => {
            if !crate::artifacts::canonical_sha256(id) {
                return Err(ApiError::bad_request(
                    "a change_spec_id is 64 lowercase hex characters",
                ));
            }
            let runs = state
                .registry
                .change_run_ids(project_id, id)
                .map_err(|error| ApiError::internal(format!("{error}")))?;
            if let Some(run_id) = runs.first() {
                return from_run(run_id);
            }
            state
                .registry
                .load_governance_spec(project_id, id)
                .map_err(|error| ApiError::internal(format!("{error:#}")))?
                .ok_or_else(|| ApiError::not_found("change in this project"))
        }
        _ => Err(ApiError::bad_request(
            "name exactly one of `change_spec_id` or `run_id`",
        )),
    }
}

#[derive(Serialize)]
struct GovernanceCheckView {
    check_id: String,
    checked_at_unix_seconds: u64,
    status: BindingOutcome,
    /// The `eplyx governance` exit code for this outcome.
    exit_code: u8,
    statement: String,
    /// When this answer was true. Every consumer must show it.
    observed_slot: Option<u64>,
    commitment: Commitment,
    /// The governance-bound change: what an analysis must carry to be a
    /// verdict about this proposal. The analysed id when it was not decodable.
    change_spec_id: String,
    analysed_change_spec_id: String,
    /// The analysed spec already named this proposal.
    governance_bound: bool,
    proposal: serde_json::Value,
    target: serde_json::Value,
    buffer: serde_json::Value,
    expected_candidate: serde_json::Value,
    binding_id: String,
    binding: GovernanceBinding,
}

impl GovernanceCheckView {
    fn of(
        check: GovernanceCheck,
        binding: GovernanceBinding,
        candidate_held: Option<bool>,
    ) -> Self {
        let observed = &binding.observation;
        let delivery = observed.delivery.as_ref();
        let proposal = json!({
            "multisig": binding.request.multisig,
            "transaction_index": binding.request.transaction_index,
            "vault_index": delivery.map(|d| d.vault_index),
            "vault": delivery.map(|d| d.vault.clone()),
            "transaction": delivery.map(|d| d.transaction.clone()),
            "proposal": delivery.map(|d| d.proposal.clone()),
            "message_sha256": delivery.map(|d| d.message_sha256.clone()),
            "status": observed.proposal.as_ref().map(|p| p.status),
            "stale": observed.proposal.as_ref().map(|p| p.stale),
            "approvals": observed.proposal.as_ref().map(|p| p.approvals),
            "threshold": observed.multisig.as_ref().map(|m| m.threshold),
        });
        let target = json!({
            "program": observed.upgrade.as_ref().map(|u| u.program.clone()),
            "programdata": observed.upgrade.as_ref().map(|u| u.programdata.clone()),
            "upgrade_authority": observed.current_program.as_ref().map(|c| c.upgrade_authority.clone()),
        });
        let buffer = json!({
            "address": observed.upgrade.as_ref().map(|u| u.buffer.clone()),
            "sha256": observed.buffer.as_ref().map(|b| b.artifact.sha256.clone()),
            "len": observed.buffer.as_ref().map(|b| b.artifact.len),
            "authority": observed.buffer.as_ref().map(|b| b.authority.clone()),
        });
        let mut expected_candidate = json!({
            "sha256": binding.expected.candidate.sha256,
            "len": binding.expected.candidate.len,
        });
        if let Some(held) = candidate_held {
            expected_candidate["held_by_project"] = json!(held);
        }
        Self {
            check_id: check.check_id,
            checked_at_unix_seconds: check.checked_at_unix_seconds,
            status: binding.outcome,
            exit_code: binding.outcome.exit_code(),
            statement: binding.statement.clone(),
            observed_slot: observed.slot,
            commitment: binding.commitment,
            change_spec_id: check.change_spec_id,
            analysed_change_spec_id: binding.analysed_change_spec_id.clone(),
            governance_bound: binding.bound_change_spec_id.as_deref()
                == Some(binding.analysed_change_spec_id.as_str()),
            proposal,
            target,
            buffer,
            expected_candidate,
            binding_id: check.binding_id,
            binding,
        }
    }
}

/// Re-read a Squads proposal and its buffer now, and bind it to an analysed
/// change of this project.
///
/// A project token suffices: this reads the chain and records what it saw,
/// and changes nothing any other check depends on. When the analysed spec was
/// not yet bound, a match returns the governance-bound spec to submit as a
/// check, so the report that follows names this proposal; the service never
/// relabels an existing report. Buffer bytes are never acquired from the chain
/// here: the candidate must be one this project already supplied.
async fn verify_squads_governance(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    let request: SquadsVerifyRequest = parse_json(&body)?;
    let rpc = state.governance.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance verification is not configured on this server (EPLYX_GOVERNANCE_RPC_URL)",
        )
    })?;
    let spec = analysed_spec(&state, &project.project_id, &request)?;
    let proposal = SquadsProposalRef {
        multisig: request.multisig.clone(),
        transaction_index: request.transaction_index,
    };
    let commitment = request.commitment.unwrap_or_default();
    let checked = spec.clone();
    let binding = tokio::task::spawn_blocking(move || {
        governance::verify_squads_upgrade(rpc.as_ref(), &proposal, &checked, commitment)
    })
    .await
    .map_err(|error| ApiError::internal(format!("governance verification stopped: {error}")))?
    .map_err(|error| ApiError::bad_request(format!("{error:#}")).with_exit_code(ci::EXIT_ERROR))?;

    // The last index is the bound change when one was derived: the id an
    // analysis must carry to name this proposal.
    let check = state
        .registry
        .record_governance_check(&project.project_id, &binding)
        .map_err(|error| ApiError::internal(format!("recording the governance check: {error:#}")))?
        .pop()
        .ok_or_else(|| ApiError::internal("a governance check was indexed nowhere"))?;
    let bound = if binding.outcome == BindingOutcome::Matched {
        let bound = binding
            .bound_spec(&spec)
            .map_err(|error| ApiError::internal(format!("{error:#}")))?;
        if let Some(bound) = &bound {
            state
                .registry
                .save_governance_spec(&project.project_id, bound)
                .map_err(|error| ApiError::internal(format!("{error:#}")))?;
        }
        bound
    } else {
        None
    };
    let held = state
        .registry
        .project_holds_artifact(&project.project_id, &executable_candidate(&spec)?.sha256)
        .unwrap_or(false);
    let analysis_runs = state
        .registry
        .change_run_ids(&project.project_id, &check.change_spec_id)
        .unwrap_or_default();
    // Offered only when the analysed spec did not already name this proposal:
    // it is what to submit as a check so the report names the proposal.
    let bound_document = match &bound {
        Some(bound)
            if bound.id().ok().as_deref() != Some(binding.analysed_change_spec_id.as_str()) =>
        {
            let document = bound
                .to_document()
                .map_err(|error| ApiError::internal(format!("{error:#}")))?;
            serde_json::from_str::<serde_json::Value>(&document)
                .map_err(|error| ApiError::internal(format!("{error}")))?
        }
        _ => serde_json::Value::Null,
    };
    let mut view = serde_json::to_value(GovernanceCheckView::of(check, binding, Some(held)))
        .map_err(|error| ApiError::internal(format!("{error}")))?;
    view["analysis"] = json!({ "runs": analysis_runs, "bound_change_spec": bound_document });
    Ok((StatusCode::OK, Json(view)).into_response())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TrailQuery {
    limit: Option<usize>,
    cursor: Option<String>,
    run_cursor: Option<String>,
}

async fn governance_trail(
    State(state): State<Shared>,
    Path((project_id, change_spec_id)): Path<(String, String)>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<TrailQuery>,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    if !crate::artifacts::canonical_sha256(&change_spec_id) {
        return Err(ApiError::bad_request("invalid change_spec_id"));
    }
    if let Some(cursor) = &query.cursor {
        crate::governance_trail::event_key(cursor)
            .map_err(|_| ApiError::bad_request("invalid trail cursor"))?;
    }
    if query
        .run_cursor
        .as_ref()
        .is_some_and(|id| !crate::storage::valid_id(id) || !id.starts_with("run_"))
    {
        return Err(ApiError::bad_request("invalid run cursor"));
    }
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let spec = analysed_spec(
        &state,
        &project.project_id,
        &SquadsVerifyRequest {
            multisig: String::new(),
            transaction_index: 0,
            change_spec_id: Some(change_spec_id.clone()),
            run_id: None,
            commitment: None,
        },
    )
    .map_err(|e| {
        if e.status == StatusCode::INTERNAL_SERVER_ERROR {
            ApiError::internal("stored governance root integrity check failed")
        } else {
            e
        }
    })?;
    if spec
        .id()
        .map_err(|_| ApiError::internal("stored governance root integrity check failed"))?
        != change_spec_id
    {
        return Err(ApiError::internal(
            "stored governance root identity differs",
        ));
    }
    if spec.delivery().is_none() {
        return Err(ApiError::bad_request(
            "trail root must be a governance-bound ChangeSpec",
        ));
    }
    let trail = state
        .registry
        .governance_trail(
            &project.project_id,
            &spec,
            query.cursor.as_deref(),
            query.run_cursor.as_deref(),
            limit,
        )
        .map_err(|_| ApiError::internal("stored governance trail integrity check failed"))?;
    Ok(Json(trail).into_response())
}

/// The recorded checks of one change, newest first. What was observed and
/// when, each at its own slot; nothing here is re-read from the chain.
async fn list_governance_checks(
    State(state): State<Shared>,
    Path((project_id, change_spec_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    if !crate::artifacts::canonical_sha256(&change_spec_id) {
        return Err(ApiError::bad_request(
            "a change_spec_id is 64 lowercase hex characters",
        ));
    }
    let checks = state
        .registry
        .governance_checks(&project.project_id, &change_spec_id, 20)
        .map_err(|error| ApiError::internal(format!("{error:#}")))?;
    let checks: Vec<GovernanceCheckView> = checks
        .into_iter()
        .map(|(check, binding)| GovernanceCheckView::of(check, binding, None))
        .collect();
    let attestations = state
        .registry
        .deployment_attestations(&project.project_id, &change_spec_id, 20)
        .map_err(|error| ApiError::internal(format!("{error:#}")))?;
    Ok((
        StatusCode::OK,
        Json(json!({ "change_spec_id": change_spec_id, "checks": checks, "attestations": attestations })),
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
    program_id: Option<String>,
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
    let projects = state
        .registry
        .list_projects()
        .map_err(|_| ApiError::internal("project registry unavailable"))?;
    let selected = if operator(&state, &headers).is_ok() {
        projects
    } else {
        if state.identity.is_none() {
            return Err(ApiError::unauthorized());
        }
        let principal = crate::cloud::auth::require_user(&state, &headers)
            .await
            .map_err(cloud_error)?;
        let mut selected = Vec::new();
        for project in projects {
            if crate::cloud::workspaces::project_access(&state, &principal, &project.project_id)
                .await
                .is_ok()
            {
                selected.push(project);
            }
        }
        selected
    };
    let views: Vec<ProjectView> = selected.iter().map(ProjectView::from).collect();
    Ok(Json(json!({"projects":views})).into_response())
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
    let project = project_for(&state, &project_id, &headers).await?;
    let run_ids = state
        .registry
        .project_run_ids(&project_id)
        .unwrap_or_default();
    let run_ids: Vec<_> = run_ids
        .into_iter()
        .filter(|id| {
            state.registry.load_run(id).is_ok_and(|run| {
                !run.hosted_analysis.as_ref().is_some_and(|j| {
                    matches!(
                        j.kind.as_str(),
                        "migration_order" | "upgrade_parameter_interaction"
                    )
                })
            })
        })
        .collect();
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

// ------------------------------------------------ project capabilities

/// Project-level prerequisites for one hosted analysis kind.
///
/// This is an informational pre-submission view. The submission handlers keep
/// validating the same authoritative state independently; this response is
/// never a capability token and says nothing about an analytical outcome.
#[derive(Clone, Serialize)]
struct MissingPrerequisite {
    code: &'static str,
    message: &'static str,
    action: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CapabilityStatus {
    Ready,
    NotReady,
    Unsupported,
}

#[derive(Serialize)]
struct AnalysisCapability {
    kind: &'static str,
    status: CapabilityStatus,
    supported: bool,
    can_submit: bool,
    missing: Vec<MissingPrerequisite>,
}

#[derive(Serialize)]
struct ProjectCapabilitiesResponse {
    schema_version: u32,
    project_id: String,
    analyses: Vec<AnalysisCapability>,
}

fn analysis_capability(
    kind: &'static str,
    supported: bool,
    common: &[MissingPrerequisite],
    specific: Vec<MissingPrerequisite>,
) -> AnalysisCapability {
    let missing = common.iter().cloned().chain(specific).collect::<Vec<_>>();
    let can_submit = supported && missing.is_empty();
    AnalysisCapability {
        kind,
        status: if !supported {
            CapabilityStatus::Unsupported
        } else if can_submit {
            CapabilityStatus::Ready
        } else {
            CapabilityStatus::NotReady
        },
        supported,
        can_submit,
        missing,
    }
}

fn project_capabilities(state: &AppState, project: &Project) -> ProjectCapabilitiesResponse {
    let disabled = (project.status == ProjectStatus::Disabled).then_some(MissingPrerequisite {
        code: "project_disabled",
        message: "This project is disabled and does not accept hosted analyses.",
        action: "Ask the service operator to enable this project.",
    });
    let common = disabled.into_iter().collect::<Vec<_>>();

    let mut upgrade = Vec::new();
    if project.program_id.is_none() {
        upgrade.push(MissingPrerequisite {
            code: "upgrade_target_missing",
            message: "This project does not have a program upgrade target.",
            action: "Use a project configured for the program being upgraded.",
        });
    }
    match &project.active_bundle {
        None => upgrade.push(MissingPrerequisite {
            code: "active_bundle_missing",
            message: "This project does not have an active replay bundle.",
            action: "Ask the service operator to upload and activate a replay bundle.",
        }),
        Some(active) if state.registry.open_bundle(&active.bundle_sha256).is_err() => {
            upgrade.push(MissingPrerequisite {
                code: "active_bundle_unavailable",
                message: "This project's active replay bundle is unavailable.",
                action: "Ask the service operator to restore or replace the active replay bundle.",
            });
        }
        Some(_) => {}
    }

    let observation = || {
        if state.observation.is_none() {
            vec![MissingPrerequisite {
                code: "observation_service_unavailable",
                message: "Read-only current-state observation is not configured.",
                action: "Ask the service operator to configure the observation service.",
            }]
        } else {
            Vec::new()
        }
    };
    let mut candidate = observation();
    if state
        .observation
        .as_ref()
        .is_none_or(|service| service.candidate.is_none())
    {
        candidate.push(MissingPrerequisite {
            code: "migration_candidate_not_configured",
            message: "The hosted migration mechanism is not configured.",
            action: "Ask the service operator to register the hosted migration mechanism.",
        });
    }

    // Stable order: the public kinds first, followed by the current-state
    // workflow's durable subchecks using their existing hosted job names.
    let analyses = vec![
        analysis_capability("program_upgrade", true, &common, upgrade),
        analysis_capability("token_migration", true, &common, Vec::new()),
        analysis_capability("lifecycle_change", true, &common, Vec::new()),
        analysis_capability("protocol_parameter_change", true, &common, Vec::new()),
        analysis_capability("upgrade_parameter_interaction", true, &common, Vec::new()),
        analysis_capability("current_observation", true, &common, observation()),
        analysis_capability("current_path", true, &common, observation()),
        analysis_capability("current_candidate", true, &common, candidate),
        analysis_capability("current_preflight", true, &common, observation()),
        analysis_capability("current_stress", true, &common, observation()),
    ];
    ProjectCapabilitiesResponse {
        schema_version: 1,
        project_id: project.project_id.clone(),
        analyses,
    }
}

async fn get_project_capabilities(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    Ok((StatusCode::OK, Json(project_capabilities(&state, &project))).into_response())
}

/// Resolve a project for a caller entitled to see it.
///
/// The operator sees any; a project token sees only its own, because it is only
/// ever matched against that project's tokens.
pub(crate) async fn project_for(
    state: &AppState,
    project_id: &str,
    headers: &HeaderMap,
) -> ApiResult<Project> {
    match authenticate(state, project_id, headers).await? {
        Principal::Operator => state
            .registry
            .load_project(project_id)
            .map_err(|_| ApiError::not_found("project")),
        Principal::Project { project, .. } | Principal::Member { project } => Ok(*project),
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
    let principal = authenticate(&state, &project_id, &headers).await?;
    if matches!(principal, Principal::Member { .. }) {
        return crate::cloud::workspaces::create_project_token(
            State(state),
            headers,
            Path(project_id),
            Ok(body),
        )
        .await
        .map_err(cloud_error);
    }
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
    let principal = authenticate(&state, &project_id, &headers).await?;
    if matches!(principal, Principal::Member { .. }) {
        return crate::cloud::workspaces::list_project_tokens(
            State(state),
            headers,
            Path(project_id),
        )
        .await
        .map(|v| v.into_response())
        .map_err(cloud_error);
    }
    require_operator(&principal)?;
    if state.identity.is_some() {
        let tokens = crate::cloud::workspaces::project_token_views(&state, &project_id)
            .await
            .map_err(cloud_error)?;
        return Ok(Json(json!({"tokens":tokens})).into_response());
    }

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
    let principal = authenticate(&state, &project_id, &headers).await?;
    if matches!(principal, Principal::Member { .. }) {
        return crate::cloud::workspaces::revoke_project_token(
            State(state),
            headers,
            Path((project_id, token_id)),
        )
        .await
        .map(|v| v.into_response())
        .map_err(cloud_error);
    }
    require_operator(&principal)?;
    if state.identity.is_some() {
        return crate::cloud::workspaces::revoke_any_project_token(&state, &project_id, &token_id)
            .await
            .map(|v| Json(v).into_response())
            .map_err(cloud_error);
    }

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
    let principal = authenticate(&state, &project_id, &headers).await?;
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
    let project = project_for(&state, &project_id, &headers).await?;
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
    let principal = authenticate(&state, &project_id, &headers).await?;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<crate::analytical::RunSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_run_id: Option<String>,
    run_id: String,
    status: RunStatus,
    exit_code: Option<u8>,
    created_at_unix_seconds: u64,
    started_at_unix_seconds: Option<u64>,
    completed_at_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    candidate_sha256: String,
    #[serde(skip_serializing_if = "String::is_empty")]
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
    project_for(&state, &project_id, &headers).await?;
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
        if run.hosted_analysis.as_ref().is_some_and(|j| {
            matches!(
                j.kind.as_str(),
                "migration_order" | "upgrade_parameter_interaction"
            )
        }) {
            continue;
        }
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
            source: run.analysis.as_ref().map(|a| a.source.clone()),
            source_run_id: run.analysis.as_ref().map(|a| a.source_run_id.clone()),
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

/// Explicit operator assignment for legacy registry projects. Never inferred
/// from names or tokens. Assignment creates authorization, not a second project.
async fn assign_workspace(
    State(state): State<Shared>,
    Path(project): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let principal = operator(&state, &headers)?;
    require_operator(&principal)?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        workspace_id: String,
        owner_user_id: String,
    }
    let request: Request = parse_json(&body)?;
    state
        .registry
        .load_project(&project)
        .map_err(|_| ApiError::not_found("project"))?;
    if !eplyx_engine::cloud::contract::is_project_id(&project) {
        return Err(ApiError::bad_request(
            "project ID is not a minted MAIN identity",
        ));
    }
    let identity = state.identity().map_err(cloud_error)?;
    let _guard = identity.project_creation.lock().await;
    let db = identity
        .db
        .get()
        .await
        .map_err(|_| ApiError::internal("identity storage unavailable"))?;
    let changed=db.execute("INSERT INTO project_workspaces(project_id,workspace_id,linked_by,linked_via) SELECT $1,$2,$3,'operator' WHERE EXISTS (SELECT 1 FROM workspace_members WHERE workspace_id=$2 AND user_id=$3 AND role='owner') ON CONFLICT(project_id) DO NOTHING",&[&project,&request.workspace_id,&request.owner_user_id]).await.map_err(|_|ApiError::internal("project assignment failed"))?;
    if changed == 0 {
        let old = db
            .query_opt(
                "SELECT workspace_id,linked_by FROM project_workspaces WHERE project_id=$1",
                &[&project],
            )
            .await
            .map_err(|_| ApiError::internal("project assignment failed"))?;
        if !old.is_some_and(|r| {
            r.get::<_, String>(0) == request.workspace_id
                && r.get::<_, String>(1) == request.owner_user_id
        }) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "project already assigned differently or named user is not a workspace owner",
            ));
        }
    }
    let path = state
        .registry
        .storage()
        .project_dir(&project)
        .map_err(|_| ApiError::not_found("project"))?
        .join("creation-intent.json");
    if path.exists() {
        let mut intent: crate::cloud::workspaces::ProjectIntent = state
            .registry
            .storage()
            .read_json(&path)
            .map_err(|_| ApiError::internal("creation intent is invalid"))?;
        intent.completed = true;
        state
            .registry
            .storage()
            .write_json(&path, &intent)
            .map_err(|_| ApiError::internal("creation recovery update failed"))?;
    }
    Ok(
        Json(json!({"project_id":project,"workspace_id":request.workspace_id,"assigned":true}))
            .into_response(),
    )
}

/// Additive multi-kind check contract. Every part is data, never executable
/// source or a client-selected command. Captures are frozen before queuing.
async fn create_analytical_check(
    state: Shared,
    project: &str,
    spec: ChangeSpec,
    mut upload: Upload,
) -> ApiResult<Response> {
    use crate::{artifacts::ArtifactClass, hosted::Input};
    #[derive(serde::Deserialize, Default)]
    #[serde(deny_unknown_fields)]
    struct Options {
        policy: Option<eplyx_engine::migration::gate::Policy>,
        before: Option<chrono::DateTime<chrono::Utc>>,
        at: Option<chrono::DateTime<chrono::Utc>>,
    }
    if upload.expectations.is_some() || upload.label.is_some() {
        return Err(ApiError::bad_request(
            "analytical checks take declared ChangeSpec and kind-specific inputs",
        ));
    }
    let options: Options = upload
        .analysis
        .remove("analysis_options")
        .map(|b| serde_json::from_slice(&b))
        .transpose()
        .map_err(|_| ApiError::bad_request("invalid analysis options"))?
        .unwrap_or_default();
    let canonical = spec
        .to_document()
        .map_err(|_| ApiError::bad_request("invalid change spec"))?;
    let change = state
        .registry
        .document_ref(canonical.as_bytes())
        .map_err(|_| ApiError::internal("input storage failed"))?;
    let mut evidence = std::collections::BTreeMap::new();
    if let Some(bytes) = upload.analysis.remove("lifecycle_evidence") {
        if !matches!(
            spec.change,
            eplyx_engine::change::Change::LifecycleChange(_)
        ) {
            return Err(ApiError::bad_request(
                "source evidence requires a lifecycle change",
            ));
        }
        let documents: std::collections::BTreeMap<String, eplyx_engine::cloud::contract::Artifact> =
            serde_json::from_slice(&bytes)
                .map_err(|_| ApiError::bad_request("invalid lifecycle evidence"))?;
        if documents.len() > 32 {
            return Err(ApiError::bad_request("too many source artifacts"));
        }
        for (id, artifact) in documents {
            if artifact.sha256 != eplyx_engine::replay::hash_bytes(artifact.text.as_bytes()) {
                return Err(ApiError::bad_request("source artifact digest mismatch"));
            }
            evidence.insert(
                id,
                state
                    .registry
                    .artifacts()
                    .put(ArtifactClass::Capture, artifact.text.as_bytes())
                    .map_err(|_| ApiError::internal("source storage failed"))?
                    .reference,
            );
        }
    }
    let program_capture = upload
        .analysis
        .remove("pinned_program_capture")
        .map(|bytes| {
            state
                .registry
                .artifacts()
                .put(ArtifactClass::Capture, &bytes)
                .map(|v| v.reference)
                .map_err(|_| ApiError::internal("input storage failed"))
        })
        .transpose()?;
    let mut take = |name: &str, class: ArtifactClass| -> ApiResult<ArtifactRef> {
        let bytes = upload
            .analysis
            .remove(name)
            .ok_or_else(|| ApiError::bad_request(format!("{name} is required")))?;
        state
            .registry
            .artifacts()
            .put(class, &bytes)
            .map(|v| v.reference)
            .map_err(|_| ApiError::internal("input storage failed"))
    };
    let input = match &spec.change {
        eplyx_engine::change::Change::TokenMigration(_) => {
            if options.before.is_some() || options.at.is_some() {
                return Err(ApiError::bad_request(
                    "migration clock comes from the frozen state input",
                ));
            }
            let candidate = match upload.candidate.take() {
                Some(b) => b.to_vec(),
                None => retained_candidate(&state, project, &spec)?,
            };
            spec.resolve(CandidateSource::Bytes(&candidate))
                .map_err(|_| ApiError::bad_request("candidate does not resolve the change spec"))?;
            let candidate = state
                .registry
                .artifacts()
                .put_program(&candidate)
                .map_err(|_| ApiError::internal("candidate storage failed"))?
                .reference;
            Input::TokenMigration {
                change,
                candidate,
                state_input: take("state_input", ArtifactClass::Document)?,
                state_artifact: take("state_artifact", ArtifactClass::Capture)?,
                program_capture,
                policy: options.policy.unwrap_or_default(),
            }
        }
        eplyx_engine::change::Change::LifecycleChange(_) => {
            if upload.candidate.is_some() || options.policy.is_some() || program_capture.is_some() {
                return Err(ApiError::bad_request("lifecycle evaluation does not take executable bytes or a deployment gate policy"));
            }
            Input::LifecycleChange {
                change,
                evidence,
                snapshot: take("snapshot", ArtifactClass::Capture)?,
                scenario: take("scenario", ArtifactClass::Document)?,
                before: options
                    .before
                    .ok_or_else(|| ApiError::bad_request("before is required"))?,
                at: options
                    .at
                    .ok_or_else(|| ApiError::bad_request("at is required"))?,
            }
        }
        _ => return Err(ApiError::bad_request("unsupported analytical kind")),
    };
    if !upload.analysis.is_empty() {
        return Err(ApiError::bad_request(
            "unexpected inputs for this analytical kind",
        ));
    }
    let registry_state = Arc::clone(&state);
    let project = project.to_owned();
    let record = tokio::task::spawn_blocking(move || {
        registry_state
            .registry
            .create_hosted_analysis(&project, input)
    })
    .await
    .map_err(|_| ApiError::internal("input validation stopped"))?
    .map_err(|_| ApiError::bad_request("analytical inputs do not validate together"))?;
    worker::spawn(Arc::clone(&state), record.run_id.clone());
    Ok((StatusCode::ACCEPTED,Json(json!({"run_id":record.run_id,"project_id":record.project_id,"status":record.status,"change":record.change,"status_url":format!("/v1/runs/{}",record.run_id)}))).into_response())
}
