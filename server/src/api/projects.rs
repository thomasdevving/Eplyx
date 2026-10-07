//! Projects, adapters and workspace assignment.

use super::*;

// ---------------------------------------------------------------- projects
//
// Explicit response shapes throughout. Serializing a stored record straight to
// the wire would make every internal field a public promise, and would leak the
// next private one added to it by accident.

#[derive(Serialize)]
pub(super) struct AdapterView {
    pub(super) adapter_id: String,
    pub(super) name: String,
    pub(super) version: u32,
    pub(super) program_id: Option<String>,
    pub(super) speaks_semantics: bool,
}

/// What this build can onboard, derived from the engine's own registry.
pub(super) async fn list_adapters(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    operator(&state, &headers)?;
    let adapters: Vec<AdapterView> = AdapterId::supported()
        .into_iter()
        .map(|id| {
            let program_id = eplyx_engine::protocol::adapters()
                .iter()
                .find(|adapter| adapter.name() == id.name)
                .map(|adapter| adapter.program_id().to_string());
            AdapterView {
                adapter_id: id.to_string(),
                name: id.name.clone(),
                version: id.version,
                program_id,
                speaks_semantics: id.speaks_semantics(),
            }
        })
        .collect();
    Ok((
        StatusCode::OK,
        Json(json!({
            "adapters": adapters,
            "semantic_schema_version": eplyx_engine::semantics::SEMANTIC_SCHEMA_VERSION,
        })),
    )
        .into_response())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateProjectRequest {
    pub(super) name: String,
    pub(super) program_id: String,
    pub(super) adapter_id: String,
}

#[derive(Serialize)]
pub(super) struct ActiveBundleView {
    pub(super) bundle_id: String,
    pub(super) bundle_sha256: String,
    pub(super) activated_at_unix_seconds: u64,
}

#[derive(Serialize)]
pub(super) struct ProjectView {
    pub(super) project_id: String,
    pub(super) name: String,
    pub(super) chain: Chain,
    pub(super) program_id: Option<String>,
    pub(super) adapter_id: String,
    pub(super) status: ProjectStatus,
    pub(super) speaks_semantics: bool,
    pub(super) active_bundle: Option<ActiveBundleView>,
    pub(super) created_at_unix_seconds: u64,
    pub(super) updated_at_unix_seconds: u64,
}

impl From<&Project> for ProjectView {
    fn from(project: &Project) -> Self {
        Self {
            project_id: project.project_id.clone(),
            name: project.name.clone(),
            chain: project.chain,
            program_id: project.program_id.clone(),
            adapter_id: project.adapter_id.to_string(),
            status: project.status,
            // Surfaced rather than implied: a project on `none@0` runs checks
            // that report no semantic coverage, and a team should see that
            // before it reads a red gate as a finding.
            speaks_semantics: project.adapter_id.speaks_semantics(),
            active_bundle: project
                .active_bundle
                .as_ref()
                .map(|active| ActiveBundleView {
                    bundle_id: active.bundle_id.clone(),
                    bundle_sha256: active.bundle_sha256.clone(),
                    activated_at_unix_seconds: active.activated_at_unix_seconds,
                }),
            created_at_unix_seconds: project.created_at_unix_seconds,
            updated_at_unix_seconds: project.updated_at_unix_seconds,
        }
    }
}

pub(super) async fn create_project(
    State(state): State<Shared>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    operator(&state, &headers)?;
    let request: CreateProjectRequest = parse_json(&body)?;
    validate_name(&request.name).map_err(|error| ApiError::bad_request(format!("{error}")))?;
    validate_program_id(&request.program_id)
        .map_err(|error| ApiError::bad_request(format!("{error}")))?;
    let adapter_id = AdapterId::try_from(request.adapter_id.clone())
        .map_err(|error| ApiError::bad_request(format!("{error}")))?;
    if !AdapterId::supported().contains(&adapter_id) {
        return Err(ApiError::bad_request(format!(
            "adapter {adapter_id} is not one this build speaks"
        )));
    }

    let project = Project::new(
        &crate::ids::project(),
        &request.name,
        &request.program_id,
        adapter_id,
    )
    .map_err(|error| ApiError::bad_request(format!("{error}")))?;
    state
        .registry
        .create_project(&project)
        .map_err(|error| ApiError::internal(format!("persisting the project: {error}")))?;
    Ok((StatusCode::CREATED, Json(ProjectView::from(&project))).into_response())
}

pub(super) async fn list_projects(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let projects = state
        .registry
        .list_projects()
        .map_err(|_| ApiError::internal("project registry unavailable"))?;
    let selected = if operator(&state, &headers).is_ok() {
        projects
    } else {
        if state.identity.is_none() {
            return Err(ApiError::unauthorized());
        }
        let principal = crate::cloud::auth::require_user(&state, &headers)
            .await
            .map_err(cloud_error)?;
        let mut selected = Vec::new();
        for project in projects {
            if crate::cloud::workspaces::project_access(&state, &principal, &project.project_id)
                .await
                .is_ok()
            {
                selected.push(project);
            }
        }
        selected
    };
    let views: Vec<ProjectView> = selected.iter().map(ProjectView::from).collect();
    Ok(Json(json!({"projects":views})).into_response())
}

/// One project, with cheap run statistics.
///
/// Cheap means the index, not the history: the newest run id is the first entry
/// of a sorted directory listing, and only that one record is read.
pub(super) async fn get_project(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    let run_ids = state
        .registry
        .project_run_ids(&project_id)
        .unwrap_or_default();
    let run_ids: Vec<_> = run_ids
        .into_iter()
        .filter(|id| {
            state.registry.load_run(id).is_ok_and(|run| {
                !run.hosted_analysis.as_ref().is_some_and(|j| {
                    matches!(
                        j.kind.as_str(),
                        "migration_order" | "upgrade_parameter_interaction"
                    )
                })
            })
        })
        .collect();
    let last = run_ids
        .first()
        .and_then(|id| state.registry.load_run(id).ok());
    Ok((
        StatusCode::OK,
        Json(json!({
            "project": ProjectView::from(&project),
            "run_count": run_ids.len(),
            "last_run_id": last.as_ref().map(|run| run.run_id.clone()),
            "last_run_status": last.as_ref().map(|run| run.status),
        })),
    )
        .into_response())
}

/// Resolve a project for a caller entitled to see it.
///
/// The operator sees any; a project token sees only its own, because it is only
/// ever matched against that project's tokens.
pub(crate) async fn project_for(
    state: &AppState,
    project_id: &str,
    headers: &HeaderMap,
) -> ApiResult<Project> {
    match authenticate(state, project_id, headers).await? {
        Principal::Operator => state
            .registry
            .load_project(project_id)
            .map_err(|_| ApiError::not_found("project")),
        Principal::Project { project, .. } | Principal::Member { project } => Ok(*project),
    }
}

/// Explicit operator assignment for legacy registry projects. Never inferred
/// from names or tokens. Assignment creates authorization, not a second project.
pub(super) async fn assign_workspace(
    State(state): State<Shared>,
    Path(project): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let principal = operator(&state, &headers)?;
    require_operator(&principal)?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        workspace_id: String,
        owner_user_id: String,
    }
    let request: Request = parse_json(&body)?;
    state
        .registry
        .load_project(&project)
        .map_err(|_| ApiError::not_found("project"))?;
    if !eplyx_engine::cloud::contract::is_project_id(&project) {
        return Err(ApiError::bad_request(
            "project ID is not a minted MAIN identity",
        ));
    }
    let identity = state.identity().map_err(cloud_error)?;
    let _guard = identity.project_creation.lock().await;
    let db = identity
        .db
        .get()
        .await
        .map_err(|_| ApiError::internal("identity storage unavailable"))?;
    let changed=db.execute("INSERT INTO project_workspaces(project_id,workspace_id,linked_by,linked_via) SELECT $1,$2,$3,'operator' WHERE EXISTS (SELECT 1 FROM workspace_members WHERE workspace_id=$2 AND user_id=$3 AND role='owner') ON CONFLICT(project_id) DO NOTHING",&[&project,&request.workspace_id,&request.owner_user_id]).await.map_err(|_|ApiError::internal("project assignment failed"))?;
    if changed == 0 {
        let old = db
            .query_opt(
                "SELECT workspace_id,linked_by FROM project_workspaces WHERE project_id=$1",
                &[&project],
            )
            .await
            .map_err(|_| ApiError::internal("project assignment failed"))?;
        if !old.is_some_and(|r| {
            r.get::<_, String>(0) == request.workspace_id
                && r.get::<_, String>(1) == request.owner_user_id
        }) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "project already assigned differently or named user is not a workspace owner",
            ));
        }
    }
    let path = state
        .registry
        .storage()
        .project_dir(&project)
        .map_err(|_| ApiError::not_found("project"))?
        .join("creation-intent.json");
    if path.exists() {
        let mut intent: crate::cloud::workspaces::ProjectIntent = state
            .registry
            .storage()
            .read_json(&path)
            .map_err(|_| ApiError::internal("creation intent is invalid"))?;
        intent.completed = true;
        state
            .registry
            .storage()
            .write_json(&path, &intent)
            .map_err(|_| ApiError::internal("creation recovery update failed"))?;
    }
    Ok(
        Json(json!({"project_id":project,"workspace_id":request.workspace_id,"assigned":true}))
            .into_response(),
    )
}
