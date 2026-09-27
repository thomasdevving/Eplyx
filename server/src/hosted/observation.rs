//! Read-only acquisition is outside the offline run queue. No request can choose
//! a provider, RPC method, program bytes, transaction or asserted path status.
use crate::{
    api::{project_for, ApiError, ApiResult, Shared},
    artifacts::{ArtifactClass, ArtifactRef},
    hosted::Input,
    registry::Registry,
};
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use eplyx_engine::{
    ingest::rpc::RpcProvider,
    lifecycle::current::{self, InspectionSelection},
    path::current as path,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    sync::Arc,
    time::{Duration, Instant},
};

pub struct Service {
    pub provider: Arc<dyn RpcProvider + Send + Sync>,
    pub permits: Arc<tokio::sync::Semaphore>,
    pub candidate: Option<Vec<u8>>,
}
impl Service {
    pub fn with_candidate(mut self, bytes: Vec<u8>) -> Result<Self> {
        ensure!(
            eplyx_engine::replay::hash_bytes(&bytes)
                == "e5db6948abca1317eb12155f73cdaf619d378c22d1bfc992c977063e10e9c1bb",
            "registered migration candidate digest mismatch"
        );
        self.candidate = Some(bytes);
        Ok(self)
    }
    pub fn new(provider: Arc<dyn RpcProvider + Send + Sync>) -> Self {
        Self {
            provider,
            candidate: None,
            permits: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}
pub(super) struct ReadOnly<'a> {
    provider: &'a dyn RpcProvider,
    calls: AtomicUsize,
    started: Instant,
    limit: usize,
}
impl<'a> ReadOnly<'a> {
    pub(super) fn new(provider: &'a dyn RpcProvider) -> Self {
        Self {
            provider,
            calls: AtomicUsize::new(0),
            started: Instant::now(),
            limit: 32,
        }
    }
    pub(super) fn stress(provider: &'a dyn RpcProvider) -> Self {
        Self {
            provider,
            calls: AtomicUsize::new(0),
            started: Instant::now(),
            limit: 96,
        }
    }
}
impl RpcProvider for ReadOnly<'_> {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        ensure!(
            matches!(
                method,
                "getGenesisHash"
                    | "getAccountInfo"
                    | "getMultipleAccounts"
                    | "getTokenLargestAccounts"
                    | "getTokenAccountsByOwner"
                    | "getProgramAccounts"
            ),
            "unsupported observation method"
        );
        ensure!(
            self.calls.fetch_add(1, Ordering::Relaxed) < self.limit
                && self.started.elapsed() < Duration::from_secs(120),
            "observation budget exhausted"
        );
        let value = self
            .provider
            .call(method, params)
            .map_err(|_| anyhow::anyhow!("read-only observation unavailable"))?;
        ensure!(
            serde_json::to_vec(&value)?.len() <= 32 * 1024 * 1024,
            "observation response exceeds byte bound"
        );
        Ok(value)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub id: String,
    pub project_id: String,
    pub request_key: String,
    pub selection: InspectionSelection,
    pub capture: ArtifactRef,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub cluster: String,
    pub mint: String,
    pub catalogue_version: Option<String>,
    pub sample_accounts: bool,
    pub public_owner: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveRequest {
    pub request_key: String,
    pub selection: Selection,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyseRequest {
    pub request_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathRequest {
    pub request_key: String,
    pub request: path::CheckRequest,
}
pub(super) fn key(value: &str) -> Result<()> {
    ensure!(
        crate::storage::valid_id(value) && value.len() >= 16,
        "invalid request key"
    );
    Ok(())
}
pub(super) fn invalid(_: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request("observation request or frozen input is invalid")
}
pub(super) fn unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "current observation is not configured",
    )
}
impl Registry {
    fn observation_path(&self, project: &str, id: &str) -> Result<std::path::PathBuf> {
        ensure!(crate::storage::valid_id(id), "invalid observation identity");
        Ok(self
            .storage()
            .project_dir(project)?
            .join("observations")
            .join(format!("{id}.json")))
    }
    pub fn load_observation(&self, project: &str, id: &str) -> Result<Observation> {
        let observation: Observation = self
            .storage()
            .read_json(&self.observation_path(project, id)?)?;
        ensure!(
            observation.project_id == project && observation.id == id,
            "observation identity mismatch"
        );
        let bytes = self
            .artifacts()
            .get(ArtifactClass::Capture, &observation.capture)?;
        let capture: current::Capture = serde_json::from_slice(&bytes)?;
        ensure!(
            capture.selection.as_ref() == Some(&observation.selection),
            "observation selection mismatch"
        );
        current::evaluate(&capture)?;
        Ok(observation)
    }
    pub fn observation_bytes(&self, observation: &Observation) -> Result<Vec<u8>> {
        self.artifacts()
            .get(ArtifactClass::Capture, &observation.capture)
    }
}
pub async fn observe(
    State(state): State<Shared>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ObserveRequest>,
) -> ApiResult<Response> {
    project_for(&state, &project, &headers).await?;
    key(&request.request_key).map_err(invalid)?;
    let reference = request
        .selection
        .catalogue_version
        .as_ref()
        .map(|version| {
            super::catalogue::reference(&state.registry, version, &request.selection.mint)
        })
        .transpose()
        .map_err(invalid)?;
    let selection = InspectionSelection {
        cluster: request.selection.cluster,
        mint: request.selection.mint,
        reference,
        sample_accounts: request.selection.sample_accounts,
        public_owner: request.selection.public_owner,
    };
    selection.validate().map_err(invalid)?;
    let service = state.observation.as_ref().ok_or_else(unavailable)?;
    let _permit = service.permits.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "an observation is already acquiring; retry shortly",
        )
    })?;
    let state2 = Arc::clone(&state);
    let observation = tokio::task::spawn_blocking(move || -> Result<Observation> {
        let _permit = _permit;
        let registry = &state2.registry;
        let id = format!(
            "obs_{}",
            &eplyx_engine::replay::hash_bytes(request.request_key.as_bytes())[..40]
        );
        if registry.observation_path(&project, &id)?.exists() {
            let existing = registry.load_observation(&project, &id)?;
            ensure!(existing.selection == selection, "request key conflict");
            return Ok(existing);
        }
        let rpc = ReadOnly {
            provider: state2
                .observation
                .as_ref()
                .context("missing provider")?
                .provider
                .as_ref(),
            calls: AtomicUsize::new(0),
            started: Instant::now(),
            limit: 32,
        };
        let capture = current::capture_selected(selection.clone(), &rpc)?;
        current::evaluate(&capture)?;
        let bytes = serde_json::to_vec(&capture)?;
        ensure!(
            bytes.len() <= 32 * 1024 * 1024,
            "observation exceeds byte bound"
        );
        let artifact = registry
            .artifacts()
            .put(ArtifactClass::Capture, &bytes)?
            .reference;
        let observation = Observation {
            id,
            project_id: project.clone(),
            request_key: request.request_key,
            selection,
            capture: artifact,
        };
        registry.storage().write_json(
            &registry.observation_path(&project, &observation.id)?,
            &observation,
        )?;
        Ok(observation)
    })
    .await
    .map_err(|_| ApiError::internal("observation task stopped"))?
    .map_err(invalid)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"observation":observation,"execution_performed":false,"authorization":false})),
    )
        .into_response())
}
pub async fn get(
    State(state): State<Shared>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    project_for(&state, &project, &headers).await?;
    let observation = state
        .registry
        .load_observation(&project, &id)
        .map_err(|_| ApiError::not_found("observation"))?;
    let capture: current::Capture = serde_json::from_slice(
        &state
            .registry
            .observation_bytes(&observation)
            .map_err(invalid)?,
    )
    .map_err(invalid)?;
    Ok(Json(
        json!({"observation":observation,"result":current::evaluate(&capture).map_err(invalid)?}),
    )
    .into_response())
}
pub async fn analyse(
    State(state): State<Shared>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<AnalyseRequest>,
) -> ApiResult<Response> {
    project_for(&state, &project, &headers).await?;
    key(&request.request_key).map_err(invalid)?;
    let observation = state
        .registry
        .load_observation(&project, &id)
        .map_err(|_| ApiError::not_found("observation"))?;
    let record = state
        .registry
        .create_hosted_analysis_with_key(
            &project,
            Input::CurrentObservation {
                capture: observation.capture,
            },
            Some(request.request_key),
        )
        .map_err(invalid)?;
    crate::worker::spawn(Arc::clone(&state), record.run_id.clone());
    accepted(&record)
}
pub async fn capabilities(
    State(state): State<Shared>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    project_for(&state, &project, &headers).await?;
    let observation = state
        .registry
        .load_observation(&project, &id)
        .map_err(|_| ApiError::not_found("observation"))?;
    let wallet = String::from_utf8(
        state
            .registry
            .observation_bytes(&observation)
            .map_err(invalid)?,
    )
    .map_err(invalid)?;
    Ok(Json(path::capabilities(&wallet).map_err(invalid)?).into_response())
}
pub async fn check_path(
    State(state): State<Shared>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<PathRequest>,
) -> ApiResult<Response> {
    project_for(&state, &project, &headers).await?;
    key(&request.request_key).map_err(invalid)?;
    let observation = state
        .registry
        .load_observation(&project, &id)
        .map_err(|_| ApiError::not_found("observation"))?;
    let wallet = String::from_utf8(
        state
            .registry
            .observation_bytes(&observation)
            .map_err(invalid)?,
    )
    .map_err(invalid)?;
    path::validate(&wallet, &request.request).map_err(invalid)?;
    let service = state.observation.as_ref().ok_or_else(unavailable)?;
    let _permit = service.permits.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "an observation is already acquiring; retry shortly",
        )
    })?;
    let state2 = Arc::clone(&state);
    let record = tokio::task::spawn_blocking(move || -> Result<crate::registry::RunMetadata> {
        let _permit = _permit;
        if let Some(record) = state2
            .registry
            .hosted_request(&project, &request.request_key)?
        {
            let Input::CurrentPath {
                capture,
                parent_observation,
                wallet_sha256,
                ..
            } = state2.registry.hosted_input(&record)?
            else {
                anyhow::bail!("request key conflict")
            };
            let frozen: path::Capture = serde_json::from_slice(
                &state2
                    .registry
                    .artifacts()
                    .get(ArtifactClass::Capture, &capture)?,
            )?;
            ensure!(
                parent_observation == id
                    && wallet_sha256 == observation.capture.sha256
                    && frozen.request == request.request,
                "request key conflict"
            );
            return Ok(record);
        }
        let check_id = format!(
            "check_{}",
            &eplyx_engine::replay::hash_bytes(request.request_key.as_bytes())[..32]
        );
        let rpc = ReadOnly {
            provider: state2
                .observation
                .as_ref()
                .context("provider absent")?
                .provider
                .as_ref(),
            calls: AtomicUsize::new(0),
            started: Instant::now(),
            limit: 32,
        };
        let capture = path::capture(wallet, request.request, id.clone(), check_id.clone(), &rpc)?;
        let artifact = state2
            .registry
            .artifacts()
            .put(ArtifactClass::Capture, &serde_json::to_vec(&capture)?)?
            .reference;
        state2.registry.create_hosted_analysis_with_key(
            &project,
            Input::CurrentPath {
                capture: artifact,
                parent_observation: id,
                wallet_sha256: observation.capture.sha256,
                check_id,
            },
            Some(request.request_key),
        )
    })
    .await
    .map_err(|_| ApiError::internal("path acquisition stopped"))?
    .map_err(invalid)?;
    crate::worker::spawn(Arc::clone(&state), record.run_id.clone());
    accepted(&record)
}
pub(super) fn accepted(record: &crate::registry::RunMetadata) -> ApiResult<Response> {
    Ok((StatusCode::ACCEPTED,Json(json!({"run_id":record.run_id,"status":record.status,"status_url":format!("/v1/runs/{}",record.run_id)}))).into_response())
}
pub fn router() -> axum::Router<Shared> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route(
            "/v1/projects/{project}/observations/{id}/candidate-checks",
            post(super::proposal::candidate),
        )
        .route(
            "/v1/projects/{project}/observations/{id}/preflights",
            post(super::proposal::preflight),
        )
        .route(
            "/v1/projects/{project}/observations/{id}/checks",
            get(super::proposal::checks),
        )
        .route(
            "/v1/projects/{project}/observations/{id}/stress-checks",
            post(super::stress::create),
        )
        .route("/v1/catalogue", get(super::catalogue::current))
        .route("/v1/projects/{project}/observations", post(observe))
        .route("/v1/projects/{project}/observations/{id}", get(self::get))
        .route(
            "/v1/projects/{project}/observations/{id}/analyse",
            post(analyse),
        )
        .route(
            "/v1/projects/{project}/observations/{id}/capabilities",
            get(capabilities),
        )
        .route(
            "/v1/projects/{project}/observations/{id}/path-checks",
            post(check_path),
        )
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
}
