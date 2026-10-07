//! Operator view of the run queue, worker failures and recovery.

use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OpsQuery {
    /// Outcomes, waits and failures cover runs created this recently.
    window_hours: Option<u64>,
}

/// One week; long enough to see a pattern, short enough to stay cheap.
const MAX_WINDOW_HOURS: u64 = 24 * 7;

/// Operator-only. Run ids, project ids and failure details of every project
/// are in here, which no project token or workspace member may see.
pub(super) async fn get_ops(
    State(state): State<Shared>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OpsQuery>,
) -> ApiResult<Response> {
    operator(&state, &headers)?;
    let hours = query.window_hours.unwrap_or(24);
    if hours == 0 || hours > MAX_WINDOW_HOURS {
        return Err(ApiError::bad_request(format!(
            "window_hours must be between 1 and {MAX_WINDOW_HOURS}"
        )));
    }
    let max = state.config.max_concurrent_runs;
    let workers = crate::ops::Workers {
        max_concurrent_runs: max,
        busy: max.saturating_sub(state.runs.available_permits()),
    };
    let reader = Arc::clone(&state);
    let snapshot = tokio::task::spawn_blocking(move || {
        crate::ops::snapshot(&reader.registry, hours * 3600, workers)
    })
    .await
    .map_err(|_| ApiError::internal("reading run records stopped"))?;
    Ok((StatusCode::OK, Json(snapshot)).into_response())
}
