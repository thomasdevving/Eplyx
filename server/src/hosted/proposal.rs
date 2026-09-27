//! Browser terms resolve only to project-owned observations and saved checks.
//! Capture is read-only; proof is reconstructed in the credential-free worker.
use super::{
    observation::{accepted, invalid, key, unavailable, ReadOnly},
    Input,
};
use crate::{
    api::{project_for, ApiError, ApiResult, Shared},
    artifacts::{ArtifactClass, ArtifactRef},
    registry::RunMetadata,
};
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    Json,
};
use eplyx_engine::{
    canonical,
    lifecycle::{current, preflight as scenario},
    migration::account,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateRequest {
    pub request_key: String,
    pub request: account::Request,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightRequest {
    pub request_key: String,
    pub request: scenario::Request,
}
fn identity(prefix: &str, key: &str) -> String {
    format!(
        "{prefix}_{}",
        &eplyx_engine::replay::hash_bytes(key.as_bytes())[..32]
    )
}
fn capture_ref(state: &Shared, bytes: &[u8]) -> Result<ArtifactRef> {
    Ok(state
        .registry
        .artifacts()
        .put(ArtifactClass::Capture, bytes)?
        .reference)
}
pub async fn candidate(
    State(state): State<Shared>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<CandidateRequest>,
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
    account::validate(&wallet, &request.request).map_err(invalid)?;
    let service = state.observation.as_ref().ok_or_else(unavailable)?;
    let program = service.candidate.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "migration candidate is not registered",
        )
    })?;
    let _permit = service.permits.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "observation acquisition is busy",
        )
    })?;
    let task = Arc::clone(&state);
    let record = tokio::task::spawn_blocking(move || -> Result<RunMetadata> {
        let _permit = _permit;
        if let Some(record) = task
            .registry
            .hosted_request(&project, &request.request_key)?
        {
            let Input::CurrentCandidate {
                capture,
                parent_observation,
                wallet_sha256,
                candidate,
                ..
            } = task.registry.hosted_input(&record)?
            else {
                anyhow::bail!("request key conflict")
            };
            let frozen: account::Capture = serde_json::from_slice(
                &task
                    .registry
                    .artifacts()
                    .get(ArtifactClass::Capture, &capture)?,
            )?;
            ensure!(
                parent_observation == id
                    && wallet_sha256 == observation.capture.sha256
                    && frozen.request == request.request
                    && candidate == ArtifactRef::of(&program),
                "request key conflict"
            );
            return Ok(record);
        }
        let check_id = identity("candidate", &request.request_key);
        let rpc = ReadOnly::new(
            task.observation
                .as_ref()
                .context("observation service")?
                .provider
                .as_ref(),
        );
        let frozen = account::capture(
            wallet,
            request.request,
            id.clone(),
            check_id.clone(),
            &program,
            &rpc,
        )?;
        let change = account::declared_change(&frozen, &program)?
            .map(|s| eplyx_engine::canonical::document(&s))
            .transpose()?
            .map(|bytes| task.registry.document_ref(bytes.as_bytes()))
            .transpose()?;
        let capture = capture_ref(&task, &serde_json::to_vec(&frozen)?)?;
        let candidate = task
            .registry
            .artifacts()
            .put(ArtifactClass::Program, &program)?
            .reference;
        task.registry.create_hosted_analysis_with_key(
            &project,
            Input::CurrentCandidate {
                capture,
                candidate,
                change,
                parent_observation: id,
                wallet_sha256: observation.capture.sha256,
                check_id,
            },
            Some(request.request_key),
        )
    })
    .await
    .map_err(|_| ApiError::internal("candidate acquisition stopped"))?
    .map_err(invalid)?;
    crate::worker::spawn(Arc::clone(&state), record.run_id.clone());
    accepted(&record)
}
fn checked(state: &Shared, project: &str, id: &str) -> Result<RunMetadata> {
    let record = state.registry.load_run(id)?;
    ensure!(
        record.project_id == project && record.status.is_terminal() && record.report_available,
        "selected check unavailable"
    );
    Ok(record)
}
pub async fn preflight(
    State(state): State<Shared>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(mut request): Json<PreflightRequest>,
) -> ApiResult<Response> {
    project_for(&state, &project, &headers).await?;
    key(&request.request_key).map_err(invalid)?;
    ensure_request_bound(&request).map_err(invalid)?;
    let observation = state
        .registry
        .load_observation(&project, &id)
        .map_err(|_| ApiError::not_found("observation"))?;
    let service = state.observation.as_ref().ok_or_else(unavailable)?;
    let _permit = service.permits.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "observation acquisition is busy",
        )
    })?;
    let task = Arc::clone(&state);
    let record = tokio::task::spawn_blocking(move || -> Result<RunMetadata> {
        let _permit = _permit;
        let wallet = String::from_utf8(task.registry.observation_bytes(&observation)?)?;
        let mut checks = vec![];
        for selected in &mut request.request.check_ids {
            let record = checked(&task, &project, selected)?;
            let Input::CurrentPath {
                capture,
                parent_observation,
                wallet_sha256,
                check_id,
            } = task.registry.hosted_input(&record)?
            else {
                anyhow::bail!("selected run is not a path check")
            };
            ensure!(
                parent_observation == id && wallet_sha256 == observation.capture.sha256,
                "check belongs to another observation"
            );
            let projection = task.registry.analytical_projection(&record)?;
            let metadata: eplyx_engine::local_store::AnalyticalMetadata =
                serde_json::from_str(&projection.metadata.text)?;
            let text = String::from_utf8(
                task.registry
                    .artifacts()
                    .get(ArtifactClass::Capture, &capture)?,
            )?;
            *selected = check_id.clone();
            checks.push(scenario::CheckEvidence {
                id: check_id,
                capture_sha256: capture.sha256,
                result_sha256: projection.report.sha256,
                engine_sha256: metadata.engine_binary_sha256,
                capture: text,
            });
        }
        let mut candidate = None;
        let conversion = if let Some(selected) = &mut request.request.conversion_check_id {
            let record = checked(&task, &project, selected)?;
            let Input::CurrentCandidate {
                capture,
                candidate: program,
                parent_observation,
                wallet_sha256,
                check_id,
                ..
            } = task.registry.hosted_input(&record)?
            else {
                anyhow::bail!("selected run is not a candidate check")
            };
            ensure!(
                parent_observation == id && wallet_sha256 == observation.capture.sha256,
                "candidate belongs to another observation"
            );
            let projection = task.registry.analytical_projection(&record)?;
            let report: serde_json::Value = serde_json::from_str(&projection.report.text)?;
            let metadata: eplyx_engine::local_store::AnalyticalMetadata =
                serde_json::from_str(&projection.metadata.text)?;
            let text = String::from_utf8(
                task.registry
                    .artifacts()
                    .get(ArtifactClass::Capture, &capture)?,
            )?;
            *selected = check_id.clone();
            candidate = Some(program.clone());
            Some(scenario::ConversionEvidence {
                id: check_id,
                capture_sha256: capture.sha256,
                result_sha256: projection.report.sha256,
                engine_sha256: metadata.engine_binary_sha256,
                plan_sha256: report["plan_sha256"]
                    .as_str()
                    .context("candidate plan identity")?
                    .into(),
                program_sha256: program.sha256,
                capture: text,
            })
        } else {
            None
        };
        if let Some(record) = task
            .registry
            .hosted_request(&project, &request.request_key)?
        {
            let Input::CurrentPreflight {
                bundle,
                parent_observation,
                wallet_sha256,
                ..
            } = task.registry.hosted_input(&record)?
            else {
                anyhow::bail!("request key conflict")
            };
            let frozen: scenario::Bundle = serde_json::from_slice(
                &task
                    .registry
                    .artifacts()
                    .get(ArtifactClass::Capture, &bundle)?,
            )?;
            ensure!(
                parent_observation == id
                    && wallet_sha256 == observation.capture.sha256
                    && frozen.inputs.request == request.request,
                "request key conflict"
            );
            return Ok(record);
        }
        let preflight_id = identity("preflight", &request.request_key);
        let mut inputs = scenario::Inputs {
            run_id: id.clone(),
            preflight_id: preflight_id.clone(),
            created_at: chrono::Utc::now(),
            engine_sha256: eplyx_engine::replay::hash_bytes(&std::fs::read(
                std::env::current_exe()?,
            )?),
            wallet_capture: wallet,
            wallet_sha256: observation.capture.sha256.clone(),
            request: request.request,
            successor_capture: None,
            checks,
            conversion,
        };
        scenario::validate(&inputs)?;
        if let Some(mint) = &inputs.request.successor_mint {
            let rpc = ReadOnly::new(
                task.observation
                    .as_ref()
                    .context("observation service")?
                    .provider
                    .as_ref(),
            );
            let capture = current::capture_selected(
                current::InspectionSelection {
                    cluster: "solana-mainnet".into(),
                    mint: mint.clone(),
                    reference: None,
                    sample_accounts: false,
                    public_owner: None,
                },
                &rpc,
            )?;
            inputs.successor_capture = Some(serde_json::to_string(&capture)?);
        }
        let frozen = scenario::prepare(inputs)?;
        let scenario_sha256 = frozen.scenario_sha256.clone();
        let bundle = capture_ref(&task, canonical::document(&frozen)?.as_bytes())?;
        task.registry.create_hosted_analysis_with_key(
            &project,
            Input::CurrentPreflight {
                bundle,
                candidate,
                parent_observation: id,
                wallet_sha256: observation.capture.sha256,
                preflight_id,
                scenario_sha256,
            },
            Some(request.request_key),
        )
    })
    .await
    .map_err(|_| ApiError::internal("scenario preparation stopped"))?
    .map_err(invalid)?;
    crate::worker::spawn(Arc::clone(&state), record.run_id.clone());
    accepted(&record)
}
fn ensure_request_bound(request: &PreflightRequest) -> Result<()> {
    ensure!(
        request.request.check_ids.len() <= 4,
        "too many selected checks"
    );
    Ok(())
}

pub async fn checks(
    State(state): State<Shared>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    project_for(&state, &project, &headers).await?;
    state
        .registry
        .load_observation(&project, &id)
        .map_err(|_| ApiError::not_found("observation"))?;
    let task = Arc::clone(&state);
    let results=tokio::task::spawn_blocking(move ||->Result<Vec<serde_json::Value>>{
  let mut checks=vec![];
  for run in task.registry.project_run_ids(&project)?.into_iter().rev(){
   let record=task.registry.load_run(&run)?;if record.project_id!=project || !record.report_available || record.hosted_analysis.is_none(){continue;}
   let input=task.registry.hosted_input(&record)?;
   let parent=match &input{Input::CurrentPath{parent_observation,..}|Input::CurrentCandidate{parent_observation,..}=>parent_observation,_=>continue};
   if parent!=&id{continue;}
   let projection=task.registry.analytical_projection(&record)?;let report:serde_json::Value=serde_json::from_str(&projection.report.text)?;
   checks.push(serde_json::json!({"run_id":run,"kind":input.kind(),"source":report["source"],"path":report["path"],"status":report["status"],"replacement_mint":report["replacement_mint"],"amount_raw":report["amount_raw"]}));
   if checks.len()==100{break;}
  }Ok(checks)
 }).await.map_err(|_|ApiError::internal("check listing stopped"))?.map_err(invalid)?;
    Ok(Json(serde_json::json!({"checks":results})))
}
