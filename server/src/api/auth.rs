//! Who is asking: bearer, operator and project credentials.

use super::*;

/// Who is asking.
///
/// Two credentials, and deliberately no user model. A **project token** is a CI
/// secret: it may submit checks for its own project and read that project's
/// results, and nothing else. An **operator token** is the hosted dashboard's
/// credential, configured on the server rather than issued by it; it is what
/// creates projects, issues and revokes their tokens, and registers and
/// activates bundles.
///
/// The split is the Phase 10 rule applied to credentials: a token that lives in
/// a pull request must not be able to change what future pull requests are
/// measured against. Nothing here is a person, and no endpoint is public —
/// listing projects without a credential would hand an unauthenticated caller
/// every program this service watches.
pub enum Principal {
    Member {
        project: Box<Project>,
    },
    Operator,
    Project {
        project: Box<Project>,
        token: Box<ProjectToken>,
    },
}

impl Principal {
    fn is_operator(&self) -> bool {
        matches!(self, Self::Operator)
    }
}

pub(super) fn bearer(headers: &HeaderMap) -> ApiResult<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(ApiError::unauthorized)
}

/// Authenticate the operator credential, and nothing else.
pub(super) fn operator(state: &AppState, headers: &HeaderMap) -> ApiResult<Principal> {
    let secret = bearer(headers)?;
    let configured = state.config.operator_token.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "no operator credential is configured",
        )
    })?;
    if !constant_time_eq(configured, secret) {
        return Err(ApiError::unauthorized());
    }
    Ok(Principal::Operator)
}

/// Authenticate against a named project: its own live token, or the operator.
///
/// A token for another project fails exactly like a token for none. Which of
/// the two it was is not distinguishable from outside, and should not be.
pub(super) async fn authenticate(
    state: &AppState,
    project_id: &str,
    headers: &HeaderMap,
) -> ApiResult<Principal> {
    if let Ok(secret) = bearer(headers) {
        if state
            .config
            .operator_token
            .as_deref()
            .is_some_and(|c| constant_time_eq(c, secret))
        {
            return Ok(Principal::Operator);
        }
        if let (Ok(project), Ok(token)) = (
            state.registry.load_project(project_id),
            state.registry.authenticate_token(project_id, secret),
        ) {
            state.registry.note_token_use(&token);
            return Ok(Principal::Project {
                project: Box::new(project),
                token: Box::new(token),
            });
        }
    }
    if state.identity.is_none() {
        return Err(ApiError::unauthorized());
    }
    let principal = crate::cloud::auth::require_for(state, headers, project_id)
        .await
        .map_err(cloud_error)?;
    crate::cloud::workspaces::project_access(state, &principal, project_id)
        .await
        .map_err(cloud_error)?;
    let project = state
        .registry
        .load_project(project_id)
        .map_err(|_| ApiError::not_found("project"))?;
    Ok(Principal::Member {
        project: Box::new(project),
    })
}
pub(super) fn cloud_error(error: crate::cloud::error::ApiError) -> ApiError {
    ApiError::new(error.status, error.message)
}

pub(super) fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0_u8, |difference, (x, y)| difference | (x ^ y))
        == 0
}

/// Require the operator, for anything that changes what checks measure against.
pub(super) fn require_operator(principal: &Principal) -> ApiResult<()> {
    if principal.is_operator() {
        return Ok(());
    }
    // A CI token asking to rotate a baseline is not a permissions puzzle to
    // explain; from outside it looks like the thing does not exist.
    Err(ApiError::not_found("resource"))
}
