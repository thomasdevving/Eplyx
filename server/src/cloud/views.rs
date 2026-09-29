//! Hosted dashboard views. Every payload has the same shape as the local
//! dashboard API and is produced by the engine's own `dashboard::view`
//! functions over synced bytes: the same summaries, gate views, joins and
//! Milestone 16 comparison semantics. Nothing is recomputed in the browser,
//! and no view executes, replays or calls a provider.
use crate::{
    api::Shared,
    cloud::{
        auth,
        error::{ApiError, ApiResult},
        workspaces::{self as api, project_access, Access},
    },
};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use eplyx_engine::{
    dashboard::view::{self, DetailContext},
    local_store::{is_safe_id, SavedMigrationCounterexample},
};
use serde::Deserialize;
use serde_json::{json, Value};

/// Who is viewing: a workspace member, or anyone on the single demo project.
async fn viewer(state: &Shared, headers: &HeaderMap, project: Option<&str>) -> ApiResult<Access> {
    match project {
        Some(project) => {
            let principal = auth::require_for(state, headers, project).await?;
            project_access(state, &principal, project).await
        }
        None => {
            let id = state
                .identity()?
                .config
                .demo_project
                .as_deref()
                .ok_or_else(|| ApiError::not_found("no public demo project on this server"))?;
            let principal = auth::Principal::Project {
                project_id: id.into(),
                token_id: String::new(),
                label: "read-only demo".into(),
            };
            project_access(state, &principal, id).await
        }
    }
}

struct Snapshot {
    runs: Vec<Value>,
    files: Vec<Value>,
    reproductions: Vec<Value>,
}

/// Summaries for one project, joined by the engine exactly as locally.
async fn snapshot(state: &Shared, project: &str) -> ApiResult<Snapshot> {
    let mut summaries = Vec::new();
    let mut ids = state.registry.project_run_ids(project)?;
    ids.reverse();
    for id in ids {
        if !state
            .registry
            .storage()
            .run_dir(&id)?
            .join("metadata.json")
            .exists()
        {
            continue;
        }
        let record = state.registry.load_run(&id)?;
        if record.project_id != project {
            return Err(ApiError::internal("run index mismatch"));
        }
        // Derived order occurrences are listed on their immutable parent only.
        if record
            .hosted_analysis
            .as_ref()
            .is_some_and(|j| j.kind == "migration_order")
        {
            continue;
        }
        if let Some(analysis) = &record.analysis {
            let doc = state.registry.analytical_projection(&record)?;
            let mut summary = view::summary(&doc.view());
            summary["hosted_run_id"] = json!(id);
            summary["run_source"] = json!(analysis.source);
            summary["synced"] = json!({"by":analysis.submitted_by,"via":match analysis.source{crate::analytical::RunSource::Ci=>"ci",_=>"cli"},
                "at":chrono::DateTime::from_timestamp(record.created_at_unix_seconds as i64,0),"local_project_id":analysis.local_project_id,"search_at":null});
            summaries.push(summary);
        } else if record
            .hosted_analysis
            .as_ref()
            .is_some_and(|j| j.projection.is_some())
        {
            let projection = state.registry.analytical_projection(&record)?;
            let mut summary = view::summary(&projection.view());
            summary["hosted_run_id"] = json!(id);
            summary["run_source"] = json!("hosted");
            summaries.push(summary);
        } else {
            summaries.push(json!({"id":id,"kind":record.hosted_analysis.as_ref().map(|j|j.kind.as_str()).unwrap_or("program_upgrade"),"run_source":"hosted","state":if record.report_available {"Complete"} else if record.status.is_terminal(){"ExecutionError"}else{"Pending"},
                "timestamp":chrono::DateTime::from_timestamp(record.created_at_unix_seconds as i64,0),"status":record.status,"hosted":record,
                "gate":{"outcome":record.exit_code.map(|c|if c==0 {"PASS"}else{"BLOCKED"})}}));
        }
    }
    let mut counterexamples = Vec::new();
    for entry in state.registry.saved_documents(project, "counterexamples")? {
        let saved: SavedMigrationCounterexample =
            serde_json::from_slice(&state.registry.document_bytes(&entry.artifact)?)
                .map_err(ApiError::internal)?;
        let mut summary =
            eplyx_engine::dashboard::migration::counterexample_fields(&saved, &entry.id);
        summary["saved_at_ms"] = Value::Null;
        summary["synced_at"] = json!(chrono::DateTime::from_timestamp(
            entry.created_at_unix_seconds as i64,
            0
        ));
        counterexamples.push(summary);
    }
    let mut reproductions = Vec::new();
    for entry in state
        .registry
        .saved_documents(project, "reproductions")?
        .into_iter()
        .rev()
    {
        let record = serde_json::from_slice(&state.registry.document_bytes(&entry.artifact)?)
            .map_err(ApiError::internal)?;
        reproductions.push(eplyx_engine::cloud::contract::reproduction_summary(&record));
    }

    let (runs, files) = view::assemble(summaries, counterexamples, &reproductions);
    Ok(Snapshot {
        runs,
        files,
        reproductions,
    })
}

fn latest_by_source(runs: &[Value], source: &str) -> Value {
    runs.iter()
        .find(|r| r["state"] == "Complete" && r["run_source"] == source)
        .map_or(Value::Null, |r| {
            json!({"id": r["id"], "number": r["number"], "gate": r["gate"]["outcome"], "timestamp": r["timestamp"],
                   "commit": r["git"]["commit"], "branch": r["git"]["branch"]})
        })
}

async fn project_view(state: &Shared, access: &Access, demo: bool) -> ApiResult<Value> {
    let snap = snapshot(state, &access.project_id).await?;
    let links = if demo {
        Value::Null
    } else {
        api::project_json(state, access).await?["links"].clone()
    };
    let mut payload = view::project_payload(
        json!({"name": access.project_name, "id": access.project_id}),
        json!({
            "config": {"state": "Cloud"},
            "cloud": {
                "workspace": {"id": access.workspace_id, "name": access.workspace_name},
                "project": {"id": access.project_id, "name": access.project_name, "visibility": "workspace", "created_at": access.created_at},
                "role": access.role,
                "demo": demo,
                "links": links,
            },
        }),
        json!({"path": "cloud", "root_display": "", "index": ""}),
        &snap.runs,
        &snap.files,
        &snap.reproductions,
        0,
    );
    let reproduced = snap
        .files
        .iter()
        .filter(|c| c["reproductions"]["succeeded"].as_u64().unwrap_or(0) > 0)
        .count();
    payload["cloud"] = json!({
        "latest_local": latest_by_source(&snap.runs, "local"),
        "latest_ci": latest_by_source(&snap.runs, "ci"),
        "counterexamples_reproduced": reproduced,
        "synced_note": "Hosted analyses and synced local and CI results. Viewing results never reruns observation, execution or replay.",
    });
    Ok(payload)
}

type ProjectPath = Path<String>;

pub async fn project(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(project): ProjectPath,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, Some(&project)).await?;
    Ok(Json(project_view(&state, &access, false).await?))
}

pub async fn demo_project(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, None).await?;
    Ok(Json(project_view(&state, &access, true).await?))
}

async fn runs_payload(state: &Shared, access: &Access) -> ApiResult<Value> {
    let snap = snapshot(state, &access.project_id).await?;
    Ok(json!({"runs": snap.runs, "ignored_store_entries": 0}))
}

pub async fn runs(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(project): ProjectPath,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, Some(&project)).await?;
    Ok(Json(runs_payload(&state, &access).await?))
}

pub async fn demo_runs(State(state): State<Shared>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, None).await?;
    Ok(Json(runs_payload(&state, &access).await?))
}

async fn run_payload(state: &Shared, access: &Access, run: &str) -> ApiResult<Value> {
    if !valid_run_id(run) {
        return Err(ApiError::not_found("unknown run"));
    }
    let record = view_record(state, &access.project_id, run)?;
    if record
        .hosted_analysis
        .as_ref()
        .is_some_and(|j| j.kind == "migration_order")
    {
        return crate::hosted::order::result(&state.registry, &record).map_err(ApiError::internal);
    }
    if record.analysis.is_none()
        && record
            .hosted_analysis
            .as_ref()
            .is_none_or(|j| j.projection.is_none())
    {
        return Ok(
            json!({"id":record.run_id,"kind":record.hosted_analysis.as_ref().map(|j|j.kind.as_str()).unwrap_or("program_upgrade"),"state":if record.report_available {"Complete"} else if record.status.is_terminal(){"ExecutionError"}else{"Pending"},"run_source":"hosted","status":record.status,"hosted":record,"report_url":format!("/v1/runs/{run}/report.json")}),
        );
    }
    let document = state.registry.analytical_projection(&record)?;
    let snap = snapshot(state, &access.project_id).await?;
    let ordered: Vec<Option<u64>> = view::ARTIFACTS
        .iter()
        .map(|(name, ..)| document.local_artifact_sizes.get(*name).copied().flatten())
        .collect();
    let bindings = document
        .bindings
        .as_ref()
        .map(|a| serde_json::from_str::<Value>(&a.text))
        .transpose()
        .map_err(ApiError::internal)?;
    let mut detail = view::run_detail_for(
        &document.view(),
        DetailContext {
            bindings,
            artifacts: view::artifact_rows(run, &ordered),
        },
        &snap.files,
    );
    if let Some(summary) = snap.runs.iter().find(|r| r["id"] == run) {
        for field in [
            "number",
            "saved_counterexamples",
            "synced",
            "hosted_run_id",
            "run_source",
        ] {
            detail[field] = summary[field].clone();
        }
    }
    let position = snap.runs.iter().position(|r| r["id"] == run);
    detail["previous_run"] = position
        .and_then(|p| snap.runs.get(p + 1))
        .map_or(Value::Null, |r| r["id"].clone());
    // Synced results retain source artifacts locally; hosted inputs live in CAS.
    detail["artifacts_local_only"] = json!(record.hosted_analysis.is_none());
    Ok(detail)
}

pub async fn run(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((project, run)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, Some(&project)).await?;
    Ok(Json(run_payload(&state, &access, &run).await?))
}

pub async fn demo_run(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(run): Path<String>,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, None).await?;
    let record = view_record(&state, &access.project_id, &run)?;
    if record
        .hosted_analysis
        .as_ref()
        .is_some_and(|j| j.kind == "migration_order")
    {
        return Err(ApiError::not_found("unknown run"));
    }
    Ok(Json(run_payload(&state, &access, &run).await?))
}

async fn counterexamples_payload(state: &Shared, access: &Access) -> ApiResult<Value> {
    let snap = snapshot(state, &access.project_id).await?;
    Ok(json!({"counterexamples": snap.files}))
}

pub async fn counterexamples(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(project): ProjectPath,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, Some(&project)).await?;
    Ok(Json(counterexamples_payload(&state, &access).await?))
}

pub async fn demo_counterexamples(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, None).await?;
    Ok(Json(counterexamples_payload(&state, &access).await?))
}

async fn counterexample_payload(state: &Shared, access: &Access, id: &str) -> ApiResult<Value> {
    if !is_safe_id(id, "cx_") {
        return Err(ApiError::not_found("unknown counterexample"));
    }
    let entry = state
        .registry
        .saved_document(&access.project_id, "counterexamples", id)
        .map_err(|_| ApiError::not_found("unknown counterexample"))?;
    let saved: SavedMigrationCounterexample =
        serde_json::from_slice(&state.registry.document_bytes(&entry.artifact)?)
            .map_err(ApiError::internal)?;
    let parent = state.registry.load_run(&entry.parent_run)?;
    let document = state.registry.analytical_projection(&parent)?;
    let snap = snapshot(state, &access.project_id).await?;
    let summary = snap
        .files
        .iter()
        .find(|c| c["id"] == id)
        .cloned()
        .ok_or_else(|| ApiError::not_found("unknown counterexample"))?;
    let mut detail = eplyx_engine::dashboard::migration::counterexample_detail_for(
        &saved,
        &summary,
        &document.view(),
    )?;
    detail["parent_summary"] = snap
        .runs
        .iter()
        .find(|r| r["id"] == summary["parent_run"])
        .cloned()
        .unwrap_or(Value::Null);
    Ok(detail)
}

pub async fn counterexample(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, Some(&project)).await?;
    Ok(Json(counterexample_payload(&state, &access, &id).await?))
}

pub async fn demo_counterexample(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, None).await?;
    Ok(Json(counterexample_payload(&state, &access, &id).await?))
}

async fn raw_counterexample(state: &Shared, access: &Access, id: &str) -> ApiResult<Response> {
    if !is_safe_id(id, "cx_") {
        return Err(ApiError::not_found("unknown counterexample"));
    }
    let entry = state
        .registry
        .saved_document(&access.project_id, "counterexamples", id)
        .map_err(|_| ApiError::not_found("unknown counterexample"))?;
    let mut response = state
        .registry
        .document_bytes(&entry.artifact)?
        .into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{id}.json\""))
            .map_err(ApiError::internal)?,
    );
    Ok(response)
}

pub async fn counterexample_raw(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let access = viewer(&state, &headers, Some(&project)).await?;
    raw_counterexample(&state, &access, &id).await
}

pub async fn demo_counterexample_raw(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let access = viewer(&state, &headers, None).await?;
    raw_counterexample(&state, &access, &id).await
}

#[derive(Deserialize)]
pub struct CompareQuery {
    left: Option<String>,
    right: Option<String>,
}

async fn compare_payload(state: &Shared, access: &Access, query: CompareQuery) -> ApiResult<Value> {
    let (Some(left), Some(right)) = (query.left, query.right) else {
        return Err(ApiError::bad_request("choose two runs: left and right"));
    };
    if !valid_run_id(&left) || !valid_run_id(&right) {
        return Err(ApiError::not_found("unknown run"));
    }
    let mut parsed = Vec::new();
    for id in [&left, &right] {
        let record = view_record(state, &access.project_id, id)?;
        if record
            .hosted_analysis
            .as_ref()
            .is_some_and(|j| j.kind == "migration_order")
        {
            return Err(ApiError::bad_request(
                "Order analyses are reviewed through their parent migration run.",
            ));
        }
        parsed.push(state.registry.analytical_projection(&record)?.view());
    }
    let snap = snapshot(state, &access.project_id).await?;
    let (b, a) = (
        parsed.pop().expect("two runs"),
        parsed.pop().expect("two runs"),
    );
    let mut comparison = tokio::task::spawn_blocking(move || view::compare_runs(&a, &b))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::invalid(format!("{e:#}")))?;
    for (side, id) in [("left", &left), ("right", &right)] {
        if let Some(run) = snap.runs.iter().find(|r| r["id"] == id.as_str()) {
            comparison[side]["number"] = run["number"].clone();
            comparison[side]["synced"] = run["synced"].clone();
        }
    }
    Ok(comparison)
}

pub async fn compare(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(project): ProjectPath,
    Query(query): Query<CompareQuery>,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, Some(&project)).await?;
    Ok(Json(compare_payload(&state, &access, query).await?))
}

pub async fn demo_compare(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<CompareQuery>,
) -> ApiResult<Json<Value>> {
    let access = viewer(&state, &headers, None).await?;
    Ok(Json(compare_payload(&state, &access, query).await?))
}

pub fn router() -> axum::Router<Shared> {
    use axum::{routing::get, Router};
    Router::new()
        .route("/v1/projects/{project_id}/view/project", get(project))
        .route("/v1/projects/{project_id}/view/runs", get(runs))
        .route("/v1/projects/{project_id}/view/runs/{run}", get(run))
        .route(
            "/v1/projects/{project_id}/view/counterexamples",
            get(counterexamples),
        )
        .route(
            "/v1/projects/{project_id}/view/counterexamples/{id}",
            get(counterexample),
        )
        .route(
            "/v1/projects/{project_id}/view/counterexamples/{id}/raw",
            get(counterexample_raw),
        )
        .route("/v1/projects/{project_id}/view/compare", get(compare))
        .route("/v1/demo/view/project", get(demo_project))
        .route("/v1/demo/view/runs", get(demo_runs))
        .route("/v1/demo/view/runs/{run}", get(demo_run))
        .route("/v1/demo/view/counterexamples", get(demo_counterexamples))
        .route(
            "/v1/demo/view/counterexamples/{id}",
            get(demo_counterexample),
        )
        .route(
            "/v1/demo/view/counterexamples/{id}/raw",
            get(demo_counterexample_raw),
        )
        .route("/v1/demo/view/compare", get(demo_compare))
}

fn valid_run_id(id: &str) -> bool {
    (crate::storage::valid_id(id) || eplyx_engine::local_store::is_safe_id(id, "run_"))
        && id.starts_with("run_")
}
fn view_record(state: &Shared, project: &str, id: &str) -> ApiResult<crate::registry::RunMetadata> {
    if let Ok(record) = state.registry.load_run(id) {
        if record.project_id == project && record.analysis.is_none() {
            return Ok(record);
        }
    }
    state
        .registry
        .source_run(project, id)?
        .ok_or_else(|| ApiError::not_found("unknown run"))
}
