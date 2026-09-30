//! Narrow hosted submission references one project-owned retained transfer case.
use super::Input;
use crate::{
    api::{project_for, ApiError, ApiResult, Shared},
    artifacts::ArtifactClass,
    registry::{Registry, RunMetadata},
};
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use eplyx_engine::change::ChangeSpec;
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_key: String,
    pub change_spec: ChangeSpec,
}
pub fn validate_parent(registry: &Registry, project: &str, input: &Input) -> Result<()> {
    let Input::ProtocolParameterChange {
        capture,
        parent_run,
        ..
    } = input
    else {
        anyhow::bail!("wrong parameter input kind")
    };
    let parent = registry.load_run(parent_run)?;
    ensure!(
        parent.project_id == project && parent.status.is_terminal() && parent.report_available,
        "parameter parent unavailable or wrong project"
    );
    let Input::CurrentPath {
        capture: retained, ..
    } = registry.hosted_input(&parent)?
    else {
        anyhow::bail!("parent must be retained current transfer");
    };
    ensure!(*capture == retained, "parent capture mismatch");
    registry.analytical_projection(&parent)?;
    Ok(())
}
pub async fn submit(
    State(state): State<Shared>,
    Path((project, parent_run)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> ApiResult<Response> {
    project_for(&state, &project, &headers).await?;
    super::observation::key(&request.request_key).map_err(super::observation::invalid)?;
    request
        .change_spec
        .validate()
        .map_err(super::observation::invalid)?;
    eplyx_engine::parameter_change::binding(&request.change_spec)
        .map_err(super::observation::invalid)?;
    let parent = state
        .registry
        .load_run(&parent_run)
        .map_err(|_| ApiError::not_found("retained transfer"))?;
    if parent.project_id != project {
        return Err(ApiError::not_found("retained transfer"));
    }
    let Input::CurrentPath { capture, .. } = state
        .registry
        .hosted_input(&parent)
        .map_err(super::observation::invalid)?
    else {
        return Err(ApiError::bad_request(
            "parent must be a retained current TransferChecked case",
        ));
    };
    let change = state
        .registry
        .document_ref(
            request
                .change_spec
                .to_document()
                .map_err(super::observation::invalid)?
                .as_bytes(),
        )
        .map_err(super::observation::invalid)?;
    let input = Input::ProtocolParameterChange {
        change,
        capture,
        parent_run,
    };
    validate_parent(&state.registry, &project, &input).map_err(super::observation::invalid)?;
    let task = state.clone();
    let record: RunMetadata = tokio::task::spawn_blocking(move || {
        task.registry
            .create_hosted_analysis_with_key(&project, input, Some(request.request_key))
    })
    .await
    .map_err(|_| ApiError::internal("parameter submission stopped"))?
    .map_err(super::observation::invalid)?;
    crate::worker::spawn(state, record.run_id.clone());
    super::observation::accepted(&record)
}
pub fn verify(registry: &Registry, input: &Input, report: &serde_json::Value) -> Result<()> {
    let Input::ProtocolParameterChange {
        change, capture, ..
    } = input
    else {
        anyhow::bail!("wrong parameter kind")
    };
    let parent = match input {
        Input::ProtocolParameterChange { parent_run, .. } => registry.load_run(parent_run)?,
        _ => unreachable!(),
    };
    validate_parent(registry, &parent.project_id, input)?;
    let spec = ChangeSpec::parse(&registry.document_bytes(change)?)?;
    let bytes = registry.artifacts().get(ArtifactClass::Capture, capture)?;
    let retained = eplyx_engine::path::current::parameter_input(&bytes)?;
    ensure!(
        report["retained_input"] == serde_json::to_value(retained)?,
        "parameter report capture differs from authoritative retained source"
    );
    eplyx_engine::parameter_change::verify(&spec, report).context("parameter projection mismatch")
}
