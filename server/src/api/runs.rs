//! Reading runs: status, canonical reports, change specs and project history.

use super::*;

/// A run belongs to one project. Its own live token may read it, and so may
/// the operator.
///
/// A foreign credential gets `401`, not `404`, and that is deliberate: it is
/// matched only against the owning project's tokens, so it fails identically
/// whether the run exists, belongs to someone else, or never existed. Searching
/// every project's tokens to answer `404` instead would cost a verification per
/// project and reveal nothing that this does not already withhold.
pub(super) async fn authorize_run(
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

pub(super) async fn get_run(
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
pub(super) fn report_unavailable(metadata: &RunMetadata) -> ApiError {
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
pub(super) async fn get_report_json(
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

pub(super) async fn get_report_markdown(
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
pub(super) async fn get_change_spec(
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

// ------------------------------------------------------------- run history

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RunQuery {
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) cursor: Option<String>,
    #[serde(default)]
    pub(super) status: Option<RunStatus>,
    #[serde(default)]
    pub(super) exit_code: Option<u8>,
    /// Every analysis of one proposal. Served from its own index, so a
    /// governance binding can find the runs for the change being signed.
    #[serde(default)]
    pub(super) change_spec_id: Option<String>,
}

#[derive(Serialize)]
pub(super) struct RunSummaryView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source: Option<crate::analytical::RunSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source_run_id: Option<String>,
    pub(super) run_id: String,
    pub(super) status: RunStatus,
    pub(super) exit_code: Option<u8>,
    pub(super) created_at_unix_seconds: u64,
    pub(super) started_at_unix_seconds: Option<u64>,
    pub(super) completed_at_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(super) candidate_sha256: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(super) bundle_sha256: String,
    pub(super) bundle_id: Option<String>,
    /// `null` marks a legacy run, recorded before change identity.
    pub(super) change: Option<RunChange>,
    pub(super) report_available: bool,
}

/// A project's runs, newest first.
///
/// The cursor is a run id, and paging means "everything after this one in the
/// ordering". Ids lead with their own minting time, so that is both stable
/// under concurrent inserts and free to compute — a run created mid-page does
/// not shift what the next page contains.
pub(super) async fn list_project_runs(
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
