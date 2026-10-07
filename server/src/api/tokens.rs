//! Project CI tokens.

use super::*;

// ------------------------------------------------------------------ tokens

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateTokenRequest {
    pub(super) label: String,
}

#[derive(Serialize)]
pub(super) struct TokenView {
    pub(super) token_id: String,
    pub(super) label: String,
    pub(super) created_at_unix_seconds: u64,
    pub(super) last_used_at_unix_seconds: Option<u64>,
    pub(super) revoked_at_unix_seconds: Option<u64>,
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
pub(super) async fn create_token(
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

pub(super) async fn list_tokens(
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

pub(super) async fn revoke_token(
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
