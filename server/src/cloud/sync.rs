//! Sync is an authenticated copy operation. It never schedules execution.
use super::{
    auth,
    error::{ApiError, ApiResult},
    workspaces,
};
use crate::{analytical::RunSource, api::Shared};
use axum::{
    body::Bytes,
    extract::{rejection::BytesRejection, Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use eplyx_engine::cloud::contract::{CounterexampleDocument, ReproductionDocument, RunDocument};
use serde_json::{json, Value};

fn body(raw: Result<Bytes, BytesRejection>) -> ApiResult<Bytes> {
    raw.map_err(|e| {
        ApiError::new(
            e.status(),
            "sync document body is invalid or exceeds its bound",
        )
    })
}
async fn writer(state: &Shared, headers: &HeaderMap, project: &str) -> ApiResult<auth::Principal> {
    let principal = auth::require_for(state, headers, project).await?;
    auth::same_origin(state, headers, &principal)?;
    workspaces::project_access(state, &principal, project).await?;
    Ok(principal)
}
fn accepted(status: &str, value: Value) -> (StatusCode, Json<Value>) {
    (
        if status == "created" {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(value),
    )
}
pub async fn run(
    State(state): State<Shared>,
    Path(project): Path<String>,
    headers: HeaderMap,
    raw: Result<Bytes, BytesRejection>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let principal = writer(&state, &headers, &project).await?;
    let document: RunDocument = auth::parse_json(&body(raw)?)?;
    let id = document.run_id.clone();
    let local = document.local_project_id.clone();
    let source = if principal.via() == "ci" {
        RunSource::Ci
    } else {
        RunSource::Local
    };
    let by = principal.label();
    let cloned = state.clone();
    let scoped = project.clone();
    let result=tokio::task::spawn_blocking(move ||{
        document.verify().map_err(|_|ApiError::invalid("run document failed identity, completeness, size or privacy validation (URLs, paths and credentials are refused)"))?;
        cloned.registry.sync_run_document(&scoped,&document,source,&by).map_err(|_|ApiError::conflict("run conflicts with immutable retained bytes or storage could not verify it"))
    }).await.map_err(ApiError::internal)??;
    workspaces::record_link(&state, &project, &local, &principal).await?;
    let url = format!(
        "{}/p/{project}/runs/{id}",
        state.identity()?.config.public_url
    );
    Ok(accepted(
        result.status,
        json!({"status":result.status,"run_id":id,"hosted_run_id":result.run_id,"url":url}),
    ))
}
pub async fn counterexample(
    State(state): State<Shared>,
    Path(project): Path<String>,
    headers: HeaderMap,
    raw: Result<Bytes, BytesRejection>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let _principal = writer(&state, &headers, &project).await?;
    let document: CounterexampleDocument = auth::parse_json(&body(raw)?)?;
    let id = document.counterexample_id.clone();
    let cloned = state.clone();
    let scoped = project.clone();
    let status = tokio::task::spawn_blocking(move || {
        let checked = document.verify().map_err(|_| {
            ApiError::invalid("counterexample failed identity, size or privacy validation")
        })?;
        let parent = cloned
            .registry
            .source_run(&scoped, &checked.saved.parent_run)?
            .ok_or_else(|| ApiError::invalid("sync the parent run first"))?;
        if parent
            .analysis
            .as_ref()
            .is_none_or(|a| a.local_project_id != document.local_project_id)
        {
            return Err(ApiError::invalid("counterexample local project mismatch"));
        }
        eplyx_engine::cloud::contract::bind_counterexample(
            &checked.saved,
            &cloned.registry.analytical_document(&parent)?.view(),
        )
        .map_err(|_| {
            ApiError::invalid("counterexample does not bind to its synced parent search")
        })?;
        cloned
            .registry
            .sync_counterexample_document(&scoped, &document)
            .map_err(|_| {
                ApiError::conflict(
                    "counterexample conflicts with retained bytes or its synced parent is missing",
                )
            })
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(accepted(
        status,
        json!({"status":status,"counterexample_id":id,"url":format!("{}/p/{project}/counterexamples/{id}",state.identity()?.config.public_url)}),
    ))
}
pub async fn reproduction(
    State(state): State<Shared>,
    Path(project): Path<String>,
    headers: HeaderMap,
    raw: Result<Bytes, BytesRejection>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let _principal = writer(&state, &headers, &project).await?;
    let document: ReproductionDocument = auth::parse_json(&body(raw)?)?;
    let id = document.reproduction_id.clone();
    let cloned = state.clone();
    let scoped = project.clone();
    let status=tokio::task::spawn_blocking(move ||{
        let checked=document.verify().map_err(|_|ApiError::invalid("reproduction failed identity, size or privacy validation"))?;
        let parent=cloned.registry.saved_document(&scoped,"counterexamples",&document.counterexample_id).map_err(|_|ApiError::invalid("sync the parent counterexample first"))?;
        if parent.local_project_id!=document.local_project_id{return Err(ApiError::invalid("reproduction local project mismatch"));}
        let saved=serde_json::from_slice(&cloned.registry.document_bytes(&parent.artifact)?).map_err(ApiError::internal)?;
        eplyx_engine::cloud::contract::bind_reproduction(&checked,&saved).map_err(|_|ApiError::invalid("reproduction does not bind to its synced counterexample"))?;
        cloned.registry.sync_reproduction_document(&scoped,&document).map_err(|_|ApiError::conflict("reproduction conflicts with retained bytes or its synced counterexample is missing"))
    }).await.map_err(ApiError::internal)??;
    Ok(accepted(
        status,
        json!({"status":status,"reproduction_id":id}),
    ))
}
pub fn router() -> axum::Router<Shared> {
    use axum::{extract::DefaultBodyLimit, routing::post, Router};
    use eplyx_engine::cloud::contract::*;
    Router::new()
        .route(
            "/v1/projects/{project_id}/sync/runs",
            post(run).layer(DefaultBodyLimit::max(MAX_RUN_BODY)),
        )
        .route(
            "/v1/projects/{project_id}/sync/counterexamples",
            post(counterexample).layer(DefaultBodyLimit::max(MAX_COUNTEREXAMPLE_BODY)),
        )
        .route(
            "/v1/projects/{project_id}/sync/reproductions",
            post(reproduction).layer(DefaultBodyLimit::max(MAX_REPRODUCTION_BODY)),
        )
}
