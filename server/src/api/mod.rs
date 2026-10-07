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

mod analytical_checks;
mod auth;
mod bundles;
mod capabilities;
mod checks;
mod governance_checks;
mod health;
mod projects;
mod runs;
mod tokens;

use analytical_checks::*;
pub use auth::Principal;
use auth::*;
use bundles::*;
use capabilities::*;
use checks::*;
use governance_checks::*;
use health::*;
pub(crate) use projects::project_for;
use projects::*;
use runs::*;
use tokens::*;

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
