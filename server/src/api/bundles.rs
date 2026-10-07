//! Bundle registration, activation and upload staging.

use super::*;

// ----------------------------------------------------------------- bundles

#[derive(Serialize)]
pub(super) struct BundleView {
    pub(super) bundle_id: String,
    pub(super) bundle_sha256: String,
    pub(super) baseline_sha256: String,
    pub(super) program_id: String,
    pub(super) adapter_id: String,
    pub(super) semantic_schema_version: u32,
    pub(super) record_count: usize,
    pub(super) source_filename: Option<String>,
    pub(super) created_at_unix_seconds: u64,
    pub(super) active: bool,
}

pub(super) fn bundle_view(record: &ProjectBundle, project: &Project) -> BundleView {
    BundleView {
        bundle_id: record.bundle_id.clone(),
        bundle_sha256: record.bundle_sha256.clone(),
        baseline_sha256: record.baseline_sha256.clone(),
        program_id: record.program_id.clone(),
        adapter_id: record.adapter_id.to_string(),
        semantic_schema_version: record.semantic_schema_version,
        record_count: record.record_count,
        source_filename: record.source_filename.clone(),
        created_at_unix_seconds: record.created_at_unix_seconds,
        // Derived from the project's pointer, never stored on the bundle: a
        // bundle does not know whether it is in use, and two records claiming
        // to be active would be a contradiction nothing could resolve.
        active: project
            .active_bundle
            .as_ref()
            .is_some_and(|active| active.bundle_id == record.bundle_id),
    }
}

/// Register an uploaded bundle.
///
/// The bundle arrives as one multipart part per file, named by its path inside
/// the bundle, so a browser can send the directory the CLI produced without
/// anything being archived, and so this service needs no archive format of its
/// own. Every path is checked before it becomes one.
pub(super) async fn create_bundle(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    multipart: Multipart,
) -> ApiResult<Response> {
    let principal = authenticate(&state, &project_id, &headers).await?;
    require_operator(&principal)?;
    let project = state
        .registry
        .load_project(&project_id)
        .map_err(|_| ApiError::not_found("project"))?;

    let staging = tempfile::Builder::new()
        .prefix("eplyx-bundle-")
        .tempdir()
        .map_err(|error| ApiError::internal(format!("staging: {error}")))?;
    let filename = read_bundle_upload(&state, multipart, staging.path()).await?;

    // Verification is the engine's, not this layer's. `register_bundle` opens
    // the uploaded tree through `CiBundle::open` and refuses anything that does
    // not verify, belong to this program, or match the declared adapter.
    let record = state
        .registry
        .register_bundle(&project, staging.path(), filename.as_deref())
        .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, format!("{error:#}")))?;

    Ok((StatusCode::CREATED, Json(bundle_view(&record, &project))).into_response())
}

pub(super) async fn list_bundles(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    let bundles = state
        .registry
        .list_bundles(&project_id)
        .map_err(|error| ApiError::internal(format!("listing bundles: {error}")))?;
    let views: Vec<BundleView> = bundles
        .iter()
        .map(|record| bundle_view(record, &project))
        .collect();
    Ok((StatusCode::OK, Json(json!({ "bundles": views }))).into_response())
}

/// Move the project's pointer. Nothing is deleted, and history is kept.
pub(super) async fn activate_bundle(
    State(state): State<Shared>,
    Path((project_id, bundle_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let principal = authenticate(&state, &project_id, &headers).await?;
    require_operator(&principal)?;
    let project = state
        .registry
        .activate_bundle(&project_id, &bundle_id)
        .map_err(|error| ApiError::new(StatusCode::CONFLICT, format!("{error:#}")))?;
    Ok((StatusCode::OK, Json(ProjectView::from(&project))).into_response())
}

/// Read an uploaded bundle into a staging directory.
///
/// Each part is named by its path inside the bundle. That name is the one thing
/// a caller controls that could become a filesystem path, so it is validated
/// rather than sanitised: anything absolute, anything with a traversal segment,
/// anything with a character outside a narrow set, and anything unreasonably
/// deep is refused outright.
pub(super) async fn read_bundle_upload(
    state: &AppState,
    mut multipart: Multipart,
    into: &std::path::Path,
) -> ApiResult<Option<String>> {
    let mut total = 0_usize;
    let mut files = 0_usize;
    let mut first_name = None;
    loop {
        let field = multipart.next_field().await.map_err(|error| {
            ApiError::new(error.status(), format!("malformed multipart: {error}"))
        })?;
        let Some(field) = field else { break };
        let name = field.name().unwrap_or_default().to_string();
        let filename = field.file_name().map(str::to_string);
        let bytes = field
            .bytes()
            .await
            .map_err(|error| ApiError::new(error.status(), format!("upload rejected: {error}")))?;

        let relative = bundle_member_path(&name)
            .ok_or_else(|| ApiError::bad_request(format!("unsafe bundle path {name:?}")))?;
        total += bytes.len();
        files += 1;
        if total > state.config.max_bundle_bytes {
            return Err(ApiError::too_large("bundle", state.config.max_bundle_bytes));
        }
        if files > 512 {
            return Err(ApiError::bad_request("a bundle with more than 512 files"));
        }
        let destination = into.join(&relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ApiError::internal(format!("staging: {error}")))?;
        }
        std::fs::write(&destination, &bytes)
            .map_err(|error| ApiError::internal(format!("staging: {error}")))?;
        if first_name.is_none() {
            first_name = filename;
        }
    }
    if files == 0 {
        return Err(ApiError::bad_request("no bundle files were uploaded"));
    }
    Ok(first_name)
}

/// A relative path inside a bundle, or nothing.
pub(super) fn bundle_member_path(name: &str) -> Option<std::path::PathBuf> {
    if name.is_empty() || name.len() > 200 || name.starts_with('/') || name.contains('\\') {
        return None;
    }
    let segments: Vec<&str> = name.split('/').collect();
    if segments.len() > 4 {
        return None;
    }
    let mut path = std::path::PathBuf::new();
    for segment in segments {
        if segment.is_empty() || segment == "." || segment == ".." || segment.len() > 100 {
            return None;
        }
        if !segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
        {
            return None;
        }
        path.push(segment);
    }
    Some(path)
}
