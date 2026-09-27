//! Workspace, project, link, CI-token and sync endpoints. Sync verifies every
//! document with the engine's contract checks, binds it to its parent record
//! and stores it immutably: the same content twice is a no-op, and different
//! content under an existing identity is a conflict, never an overwrite.
use crate::{
    api::Shared,
    cloud::{
        auth::{self, clean_name, parse_json, Principal},
        error::{ApiError, ApiResult},
    },
};
use axum::{
    body::Bytes,
    extract::{rejection::BytesRejection, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use eplyx_engine::{
    cloud::contract::{is_cloud_id, is_project_id},
    local_store::is_safe_id,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub struct Access {
    pub project_id: String,
    pub project_name: String,
    pub workspace_id: String,
    pub workspace_name: String,
    /// `owner` or `member`; `None` for a CI token.
    pub role: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Resolve a project the caller may see. Unknown and inaccessible projects
/// are indistinguishable (404), so project IDs cannot be probed.
pub async fn project_access(
    state: &crate::api::AppState,
    principal: &Principal,
    project_id: &str,
) -> ApiResult<Access> {
    let unknown = || ApiError::not_found("unknown project");
    if !is_project_id(project_id) {
        return Err(unknown());
    }
    let client = state.identity()?.db.get().await?;
    let scoped = match principal {
        Principal::User { id, .. } => Some(id),
        Principal::Project {
            project_id: scoped, ..
        } => {
            if scoped != project_id {
                return Err(unknown());
            }
            None
        }
    };
    let row=if let Some(user)=scoped {
        client.query_opt("SELECT w.id,w.name,m.role FROM project_workspaces p JOIN workspaces w ON w.id=p.workspace_id JOIN workspace_members m ON m.workspace_id=p.workspace_id AND m.user_id=$2 WHERE p.project_id=$1",&[&project_id,user]).await?
    } else {
        client.query_opt("SELECT w.id,w.name,NULL::TEXT FROM project_workspaces p JOIN workspaces w ON w.id=p.workspace_id WHERE p.project_id=$1",&[&project_id]).await?
    }.ok_or_else(unknown)?;
    let project = state
        .registry
        .load_project(project_id)
        .map_err(|_| unknown())?;
    Ok(Access {
        project_id: project.project_id,
        project_name: project.name,
        workspace_id: row.get(0),
        workspace_name: row.get(1),
        role: row.get(2),
        created_at: chrono::DateTime::from_timestamp(project.created_at_unix_seconds as i64, 0)
            .ok_or_else(unknown)?,
    })
}

pub async fn workspace_role(
    state: &crate::api::AppState,
    user_id: &str,
    workspace_id: &str,
) -> ApiResult<String> {
    if !is_cloud_id(workspace_id, "ws_") {
        return Err(ApiError::not_found("unknown workspace"));
    }
    let client = state.identity()?.db.get().await?;
    client
        .query_opt(
            "SELECT role FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
            &[&workspace_id, &user_id],
        )
        .await?
        .map(|row| row.get(0))
        .ok_or_else(|| ApiError::not_found("unknown workspace"))
}

fn body(body: Result<Bytes, BytesRejection>) -> ApiResult<Bytes> {
    body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "document exceeds the sync size bound",
            )
        } else {
            ApiError::bad_request("could not read the request body")
        }
    })
}

fn owner(role: &Option<String>) -> ApiResult<()> {
    if role.as_deref() == Some("owner") {
        Ok(())
    } else {
        Err(ApiError::forbidden("only a workspace owner can do this"))
    }
}

// --------------------------------------------------------------- workspaces

pub async fn list_workspaces(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let principal = auth::require_user(&state, &headers).await?;
    let user = principal.user_id().unwrap_or_default().to_owned();
    let client = state.identity()?.db.get().await?;
    let rows = client
        .query(
            "SELECT w.id, w.name, m.role FROM workspaces w JOIN workspace_members m ON m.workspace_id = w.id
             WHERE m.user_id = $1 ORDER BY w.created_at, w.id",
            &[&user],
        )
        .await?;
    let mut workspaces = Vec::new();
    for row in rows {
        let id: String = row.get(0);
        let ids=client.query("SELECT project_id FROM project_workspaces WHERE workspace_id=$1 ORDER BY project_id",&[&id]).await?;
        let projects=ids.iter().filter_map(|r|state.registry.load_project(r.get::<_,String>(0).as_str()).ok()).map(|p| {
            let runs=state.registry.project_run_ids(&p.project_id).unwrap_or_default();
            json!({"id":p.project_id,"name":p.name,"runs":runs.len(),"latest_gate":null,
                "latest_run_status":runs.first().and_then(|id|state.registry.load_run(id).ok()).map(|m|m.status)})
        }).collect::<Vec<_>>();
        workspaces.push(json!({"id": id, "name": row.get::<_, String>(1), "role": row.get::<_, String>(2), "projects": projects}));
    }
    Ok(Json(json!({"workspaces": workspaces})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameRequest {
    name: String,
}

pub async fn create_workspace(
    State(state): State<Shared>,
    headers: HeaderMap,
    raw: Result<Bytes, BytesRejection>,
) -> ApiResult<Response> {
    let principal = auth::require_user(&state, &headers).await?;
    auth::same_origin(&state, &headers, &principal)?;
    let request: NameRequest = parse_json(&body(raw)?)?;
    let name = clean_name(&request.name, "workspace name")?;
    let id = auth::new_id("ws_");
    let user = principal.user_id().unwrap_or_default().to_owned();
    let mut client = state.identity()?.db.get().await?;
    let tx = client.transaction().await?;
    tx.execute(
        "INSERT INTO workspaces (id, name, created_by) VALUES ($1, $2, $3)",
        &[&id, &name, &user],
    )
    .await?;
    tx.execute(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, 'owner')",
        &[&id, &user],
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"workspace": {"id": id, "name": name, "role": "owner"}})),
    )
        .into_response())
}

pub async fn list_members(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
) -> ApiResult<Json<Value>> {
    let principal = auth::require_user(&state, &headers).await?;
    let role = workspace_role(&state, principal.user_id().unwrap_or_default(), &workspace).await?;
    let client = state.identity()?.db.get().await?;
    let members = client
        .query(
            "SELECT u.id, u.email, u.name, m.role, m.added_at FROM workspace_members m JOIN users u ON u.id = m.user_id
             WHERE m.workspace_id = $1 ORDER BY m.added_at, u.email",
            &[&workspace],
        )
        .await?
        .iter()
        .map(|r| json!({"id": r.get::<_, String>(0), "email": r.get::<_, String>(1), "name": r.get::<_, String>(2), "role": r.get::<_, String>(3), "added_at": r.get::<_, chrono::DateTime<chrono::Utc>>(4)}))
        .collect::<Vec<_>>();
    Ok(Json(json!({"role": role, "members": members})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberRequest {
    email: String,
}

pub async fn add_member(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
    raw: Result<Bytes, BytesRejection>,
) -> ApiResult<Json<Value>> {
    let principal = auth::require_user(&state, &headers).await?;
    auth::same_origin(&state, &headers, &principal)?;
    let role = workspace_role(&state, principal.user_id().unwrap_or_default(), &workspace).await?;
    owner(&Some(role))?;
    let request: MemberRequest = parse_json(&body(raw)?)?;
    let email = request.email.trim().to_ascii_lowercase();
    let client = state.identity()?.db.get().await?;
    let user = client
        .query_opt("SELECT id FROM users WHERE email = $1", &[&email])
        .await?
        .ok_or_else(|| {
            ApiError::not_found("no Eplyx account uses this email; ask them to sign up first")
        })?;
    let user_id: String = user.get(0);
    client
        .execute(
            "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, 'member') ON CONFLICT DO NOTHING",
            &[&workspace, &user_id],
        )
        .await?;
    Ok(Json(json!({"added": email})))
}

pub async fn remove_member(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((workspace, member)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let principal = auth::require_user(&state, &headers).await?;
    auth::same_origin(&state, &headers, &principal)?;
    let role = workspace_role(&state, principal.user_id().unwrap_or_default(), &workspace).await?;
    owner(&Some(role))?;
    if principal.user_id() == Some(member.as_str()) {
        return Err(ApiError::bad_request("owners cannot remove themselves"));
    }
    let client = state.identity()?.db.get().await?;
    let removed = client
        .execute(
            "DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2 AND role = 'member'",
            &[&workspace, &member],
        )
        .await?;
    if removed == 0 {
        return Err(ApiError::not_found("no such member"));
    }
    Ok(Json(json!({"removed": member})))
}

// ----------------------------------------------------------------- projects

#[derive(Clone, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectIntent {
    pub project: crate::project::Project,
    pub workspace_id: String,
    pub user_id: String,
    pub request_key: String,
    pub completed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectRequest {
    name: String,
    #[serde(default)]
    program_id: Option<String>,
    #[serde(default)]
    request_key: Option<String>,
}

fn project_intents(state: &crate::api::AppState) -> ApiResult<Vec<ProjectIntent>> {
    let mut intents = Vec::new();
    for entry in
        std::fs::read_dir(state.registry.storage().projects_root()).map_err(ApiError::internal)?
    {
        let entry = entry.map_err(ApiError::internal)?;
        if !entry.file_type().map_err(ApiError::internal)?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        if !is_project_id(&id) {
            continue;
        }
        let path = entry.path().join("creation-intent.json");
        if path.exists() {
            let intent: ProjectIntent = state.registry.storage().read_json(&path)?;
            if intent.project.project_id != id {
                return Err(ApiError::internal("intent identity mismatch"));
            }
            intents.push(intent);
        }
    }
    Ok(intents)
}
/// Filesystem intent retains pending creation inputs; it never authorizes access.
/// Both the canonical registry project and current Postgres membership must exist.
pub async fn finish_project_intent(
    state: &crate::api::AppState,
    intent: &ProjectIntent,
) -> ApiResult<()> {
    if intent.completed {
        return Ok(());
    }
    let id = &intent.project.project_id;
    let path = state.registry.storage().project_path(id)?;
    if !path.exists() {
        state.registry.create_project(&intent.project)?;
    }
    let project = state.registry.load_project(id)?;
    if project.name != intent.project.name || project.program_id != intent.project.program_id {
        return Err(ApiError::conflict(
            "pending project creation disagrees with the registry",
        ));
    }
    let db = state.identity()?.db.get().await?;
    let inserted=db.execute("INSERT INTO project_workspaces (project_id,workspace_id,linked_by) SELECT $1,$2,$3 WHERE EXISTS (SELECT 1 FROM workspace_members WHERE workspace_id=$2 AND user_id=$3) ON CONFLICT (project_id) DO NOTHING",&[id,&intent.workspace_id,&intent.user_id]).await?;
    if inserted == 0 {
        let existing = db
            .query_opt(
                "SELECT workspace_id FROM project_workspaces WHERE project_id=$1",
                &[id],
            )
            .await?;
        if !existing.is_some_and(|r| r.get::<_, String>(0) == intent.workspace_id) {
            return Err(ApiError::forbidden(
                "project creation requires current workspace membership",
            ));
        }
    }
    let mut completed = intent.clone();
    completed.completed = true;
    state.registry.storage().write_json(
        &state
            .registry
            .storage()
            .project_dir(id)?
            .join("creation-intent.json"),
        &completed,
    )?;
    Ok(())
}
pub async fn recover_project_intents(state: &crate::api::AppState) -> ApiResult<()> {
    if state.identity.is_none() {
        return Ok(());
    }
    for intent in project_intents(state)? {
        // Revoked membership leaves a private pending intent. An owner can
        // explicitly assign the project; recovery never restores membership.
        if let Err(e) = finish_project_intent(state, &intent).await {
            if e.status != StatusCode::FORBIDDEN {
                return Err(e);
            }
        }
    }
    Ok(())
}
pub async fn create_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(workspace): Path<String>,
    raw: Result<Bytes, BytesRejection>,
) -> ApiResult<Response> {
    let principal = auth::require_user(&state, &headers).await?;
    auth::same_origin(&state, &headers, &principal)?;
    let user = principal.user_id().unwrap_or_default();
    workspace_role(&state, user, &workspace).await?;
    let request: ProjectRequest = parse_json(&body(raw)?)?;
    let name = clean_name(&request.name, "project name")?;
    let request_key = request
        .request_key
        .unwrap_or_else(|| auth::new_id("request_"));
    if request_key.is_empty()
        || request_key.len() > 100
        || !request_key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(ApiError::invalid("invalid project request key"));
    }
    let _creation = state.identity()?.project_creation.lock().await;
    let mut project = None;
    for intent in project_intents(&state)? {
        if intent.workspace_id == workspace
            && intent.user_id == user
            && intent.request_key == request_key
        {
            if intent.project.name != name || intent.project.program_id != request.program_id {
                return Err(ApiError::conflict(
                    "project request key already identifies different terms",
                ));
            }
            finish_project_intent(&state, &intent).await?;
            // A completed request never reassigns a project whose authorization changed.
            project_access(&state, &principal, &intent.project.project_id).await?;
            project = Some(state.registry.load_project(&intent.project.project_id)?);
            break;
        }
    }
    let project = match project {
        Some(p) => p,
        None => {
            let id = crate::ids::project();
            let project = match &request.program_id {
                Some(program) => crate::project::Project::new(
                    &id,
                    &name,
                    program,
                    crate::project::AdapterId::for_program(program),
                )?,
                None => crate::project::Project::analytical(&id, &name)?,
            };
            let intent = ProjectIntent {
                project: project.clone(),
                workspace_id: workspace.clone(),
                user_id: user.into(),
                request_key,
                completed: false,
            };
            // Intent first, then canonical project, then authorization link. A lost
            // response is retryable with the same request key, never a second project.
            state.registry.storage().write_json(
                &state
                    .registry
                    .storage()
                    .project_dir(&id)?
                    .join("creation-intent.json"),
                &intent,
            )?;
            state.registry.create_project(&project)?;
            finish_project_intent(&state, &intent).await?;
            project
        }
    };
    Ok((StatusCode::CREATED,Json(json!({"project":{"id":project.project_id,"name":project.name,"workspace_id":workspace,"visibility":"workspace"}}))).into_response())
}

pub async fn project_json(state: &crate::api::AppState, access: &Access) -> ApiResult<Value> {
    let client = state.identity()?.db.get().await?;
    let links = client
        .query(
            "SELECT local_project_id, linked_by, linked_via, linked_at FROM project_links WHERE project_id = $1 ORDER BY linked_at",
            &[&access.project_id],
        )
        .await?
        .iter()
        .map(|r| json!({"local_project_id": r.get::<_, String>(0), "linked_by": r.get::<_, String>(1), "linked_via": r.get::<_, String>(2), "linked_at": r.get::<_, chrono::DateTime<chrono::Utc>>(3)}))
        .collect::<Vec<_>>();
    Ok(json!({
        "project": {"id": access.project_id, "name": access.project_name, "visibility": "workspace", "created_at": access.created_at,
                    "demo": state.identity()?.config.demo_project.as_deref() == Some(access.project_id.as_str())},
        "workspace": {"id": access.workspace_id, "name": access.workspace_name},
        "role": access.role,
        "links": links,
        "url": format!("{}/p/{}", state.identity()?.config.public_url, access.project_id),
    }))
}

pub async fn get_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    let principal = auth::require_for(&state, &headers, &project).await?;
    let access = project_access(&state, &principal, &project).await?;
    Ok(Json(project_json(&state, &access).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkRequest {
    local_project_id: String,
}

pub(crate) async fn record_link(
    state: &crate::api::AppState,
    project: &str,
    local_project_id: &str,
    principal: &Principal,
) -> ApiResult<()> {
    let client = state.identity()?.db.get().await?;
    client
        .execute(
            "INSERT INTO project_links (project_id, local_project_id, linked_by, linked_via) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
            &[&project, &local_project_id, &principal.label(), &principal.via()],
        )
        .await?;
    Ok(())
}

pub async fn link_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(project): Path<String>,
    raw: Result<Bytes, BytesRejection>,
) -> ApiResult<Json<Value>> {
    let principal = auth::require_for(&state, &headers, &project).await?;
    // Linking binds a developer's local store; browser sessions never do it.
    if matches!(principal, Principal::User { session: true, .. }) {
        return Err(ApiError::forbidden(
            "link a local project with `eplyx link`",
        ));
    }
    let access = project_access(&state, &principal, &project).await?;
    let request: LinkRequest = parse_json(&body(raw)?)?;
    if !is_safe_id(&request.local_project_id, "local_") {
        return Err(ApiError::invalid("invalid local project ID"));
    }
    record_link(
        &state,
        &access.project_id,
        &request.local_project_id,
        &principal,
    )
    .await?;
    Ok(Json(project_json(&state, &access).await?))
}

// ---------------------------------------------------------------- CI tokens

pub async fn list_project_tokens(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> ApiResult<Json<Value>> {
    let principal = auth::require_user(&state, &headers).await?;
    let access = project_access(&state, &principal, &project).await?;
    owner(&access.role)?;
    Ok(Json(
        json!({"tokens":project_token_views(&state,&project).await?}),
    ))
}

pub async fn project_token_views(
    state: &crate::api::AppState,
    project: &str,
) -> ApiResult<Vec<Value>> {
    let mut tokens=state.registry.list_tokens(project)?.iter().map(|t|json!({
        "id":t.token_id,"token_id":t.token_id,"label":t.label,
        "created_at":chrono::DateTime::from_timestamp(t.created_at_unix_seconds as i64,0),
        "last_used_at":t.last_used_at_unix_seconds.and_then(|n|chrono::DateTime::from_timestamp(n as i64,0)),
        "revoked_at":t.revoked_at_unix_seconds.and_then(|n|chrono::DateTime::from_timestamp(n as i64,0)),
        "created_at_unix_seconds":t.created_at_unix_seconds,"last_used_at_unix_seconds":t.last_used_at_unix_seconds,"revoked_at_unix_seconds":t.revoked_at_unix_seconds,
    })).collect::<Vec<_>>();
    if let Some(identity) = &state.identity {
        let client = identity.db.get().await?;
        let rows=client.query("SELECT t.id,t.label,t.created_at,t.last_used_at,t.revoked_at,u.email FROM api_tokens t JOIN users u ON u.id=t.user_id WHERE t.project_id=$1 AND t.kind='project' ORDER BY t.created_at DESC",&[&project]).await?;
        tokens.extend(rows.iter().map(|r|{
            type Time=Option<chrono::DateTime<chrono::Utc>>;
            let created:chrono::DateTime<chrono::Utc>=r.get(2);let used:Time=r.get(3);let revoked:Time=r.get(4);
            json!({"id":r.get::<_,String>(0),"token_id":r.get::<_,String>(0),"label":r.get::<_,String>(1),"created_at":created,"last_used_at":used,"revoked_at":revoked,"created_by":r.get::<_,String>(5),
                "created_at_unix_seconds":created.timestamp(),"last_used_at_unix_seconds":used.map(|t|t.timestamp()),"revoked_at_unix_seconds":revoked.map(|t|t.timestamp())})
        }));
    }
    Ok(tokens)
}
pub async fn revoke_any_project_token(
    state: &crate::api::AppState,
    project: &str,
    token: &str,
) -> ApiResult<Value> {
    if let Ok(record) = state.registry.revoke_token(project, token) {
        return Ok(
            json!({"revoked":token,"token_id":token,"revoked_at_unix_seconds":record.revoked_at_unix_seconds}),
        );
    }
    let db = state.identity()?.db.get().await?;
    let changed=db.execute("UPDATE api_tokens SET revoked_at=now() WHERE id=$1 AND project_id=$2 AND kind='project' AND revoked_at IS NULL",&[&token,&project]).await?;
    if changed == 0 {
        return Err(ApiError::not_found("no such active token"));
    }
    Ok(json!({"revoked":token}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenRequest {
    label: String,
}

pub async fn create_project_token(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(project): Path<String>,
    raw: Result<Bytes, BytesRejection>,
) -> ApiResult<Response> {
    let principal = auth::require_user(&state, &headers).await?;
    auth::same_origin(&state, &headers, &principal)?;
    let access = project_access(&state, &principal, &project).await?;
    owner(&access.role)?;
    let request: TokenRequest = parse_json(&body(raw)?)?;
    let label = clean_name(&request.label, "token label")?;
    let token = crate::project::generate_token();
    let id = auth::new_id("tok_");
    let client = state.identity()?.db.get().await?;
    client
        .execute(
            "INSERT INTO api_tokens (id, token_sha256, kind, user_id, project_id, label) VALUES ($1, $2, 'project', $3, $4, $5)",
            &[&id, &auth::digest(&token), &principal.user_id().unwrap_or_default(), &access.project_id, &label],
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "label": label, "token": token, "project_id": access.project_id,
                    "note": "Shown once. Store it as the EPLYX_TOKEN secret of trusted CI workflows; it can submit checks, sync and read this project’s results. It cannot change the active bundle."})),
    )
        .into_response())
}

pub async fn revoke_project_token(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((project, token)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let principal = auth::require_user(&state, &headers).await?;
    auth::same_origin(&state, &headers, &principal)?;
    let access = project_access(&state, &principal, &project).await?;
    owner(&access.role)?;
    Ok(Json(
        revoke_any_project_token(&state, &project, &token).await?,
    ))
}
