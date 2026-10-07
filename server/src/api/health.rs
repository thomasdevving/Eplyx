//! Liveness and readiness probes.

use super::*;

pub(super) async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

/// Ready means the persistent volume is actually usable. Nothing about the
/// configuration itself is exposed.
pub(super) async fn ready(State(state): State<Shared>) -> impl IntoResponse {
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
