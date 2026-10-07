//! Submitting an upgrade check: uploads, retained candidates and artefacts.

use super::*;

/// What creating a check answers with.
///
/// Deliberately not a result. At this point no analysis has run, and inventing
/// a verdict to fill the shape would be a lie the caller could act on.
#[derive(Serialize)]
pub(super) struct AcceptedResponse {
    pub(super) run_id: String,
    pub(super) project_id: String,
    pub(super) status: RunStatus,
    pub(super) status_url: String,
    /// What was accepted for analysis, already bound to the pinned bundle.
    /// Identity is known before any replay; a verdict is not.
    pub(super) change: RunChange,
    pub(super) candidate_sha256: String,
    /// The durable, content-addressed object the run will execute.
    pub(super) candidate_artifact: ArtifactRef,
    pub(super) bundle_sha256: String,
}

/// Accept a check and return immediately.
///
/// Everything expensive happens after the response. What this owes the caller
/// is a run id that is already durable: if a 202 was received, the run is
/// recoverable through `GET /v1/runs/{id}` even if this connection never
/// carried another byte.
pub(super) async fn create_check(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    multipart: Multipart,
) -> ApiResult<Response> {
    let principal = authenticate(&state, &project_id, &headers).await?;
    let project = match &principal {
        Principal::Project { project, .. } | Principal::Member { project } => (**project).clone(),
        Principal::Operator => state
            .registry
            .load_project(&project_id)
            .map_err(|_| ApiError::not_found("project"))?,
    };
    // This is the same authoritative project state exposed by the capability
    // endpoint, checked again at the mutation boundary. In particular, a
    // prepared analytical submission that was opened while ready must report
    // a readiness conflict—not a generic input error—if the project was
    // disabled before POST.
    if project.status == ProjectStatus::Disabled {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "this project is disabled and accepts no checks",
        ));
    }
    let upload = read_upload(&state, multipart).await?;
    if let Some(bytes) = &upload.change_spec {
        let spec = ChangeSpec::parse(bytes).map_err(|_| {
            ApiError::bad_request("invalid change spec").with_exit_code(ci::EXIT_ERROR)
        })?;
        if !matches!(
            spec.change,
            eplyx_engine::change::Change::ProgramUpgrade { .. }
        ) {
            return create_analytical_check(state, &project.project_id, spec, upload).await;
        }
    }
    if !upload.analysis.is_empty() {
        return Err(ApiError::bad_request(
            "analytical inputs require an explicit analytical change spec",
        ));
    }

    // Readiness is a hosted configuration question, answered before a run
    // exists. It is deliberately not an Eplyx exit code: nothing was measured,
    // so there is no verdict to report about the candidate.
    // The client never chooses the baseline. The project's active bundle is
    // server state, so a pull request cannot quietly measure itself against
    // something more forgiving.
    let active = project.active_bundle.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "this project has no active bundle; upload and activate one before running checks",
        )
    })?;
    let bundle_sha256 = active.bundle_sha256.clone();

    // Opened, not merely located. Accepting a run against a bundle that cannot
    // be read would buy a 202 and pay for it with an execution error minutes
    // later, once the caller has stopped watching.
    let bundle = state
        .registry
        .open_bundle(&bundle_sha256)
        .map_err(|error| {
            ApiError::new(
                StatusCode::CONFLICT,
                format!("the active bundle is unusable: {error}"),
            )
            .with_exit_code(ci::EXIT_INCOMPATIBLE)
        })?;
    let manifest = bundle.manifest();
    let bundle_dir = state
        .registry
        .storage()
        .bundle_path(&bundle_sha256)
        .map_err(|error| ApiError::internal(format!("bundle path: {error}")))?;

    // ---- the proposal, fixed before the run exists ------------------------
    //
    // Candidate bytes alone stand for the minimal program upgrade of the pinned
    // bundle's program, exactly as `eplyx ci check --candidate` does. An
    // explicit spec is authoritative, and the bytes only satisfy its artefact
    // reference. Either way the spec is bound to the pinned bundle and the bytes
    // are verified against it here, so a proposal that does not fit, or bytes
    // it did not describe, never become a queued run.
    let submitted = match &upload.change_spec {
        Some(document) => {
            if upload.label.is_some() {
                return Err(ApiError::bad_request(
                    "`label` cannot accompany an explicit change spec; put it in the spec's metadata",
                )
                .with_exit_code(ci::EXIT_ERROR));
            }
            Some(ChangeSpec::parse(document).map_err(|error| {
                ApiError::bad_request(format!("invalid change spec: {error:#}"))
                    .with_exit_code(ci::EXIT_ERROR)
            })?)
        }
        None => None,
    };
    // The bytes: uploaded, or — for an explicit spec only — an artefact this
    // project already supplied, so a proposal can be analysed again without
    // re-uploading what the service already holds immutably.
    let candidate: Vec<u8> = match (upload.candidate, &submitted) {
        (Some(bytes), _) => bytes.to_vec(),
        (None, None) => return Err(ApiError::bad_request("candidate is required")),
        (None, Some(spec)) => retained_candidate(&state, &project.project_id, spec)?,
    };
    let (spec, origin) = match submitted {
        Some(spec) => (spec, ChangeOrigin::Submitted),
        None => {
            let mut spec = ChangeSpec::program_upgrade(&manifest.program_id, &candidate);
            spec.metadata.label = upload.label.clone();
            (spec, ChangeOrigin::DerivedFromCandidate)
        }
    };
    let binding = ci::bind_change(&bundle_dir, &spec).map_err(|error| {
        let status = match error {
            ci::CheckError::Bundle(_) => StatusCode::CONFLICT,
            ci::CheckError::Configuration(_) => StatusCode::BAD_REQUEST,
        };
        ApiError::new(status, format!("{error}")).with_exit_code(error.exit_code())
    })?;
    spec.resolve(CandidateSource::Bytes(&candidate))
        .map_err(|error| {
            ApiError::bad_request(format!("{error:#}")).with_exit_code(ci::EXIT_ERROR)
        })?;
    let change = RunChange::of(&spec, origin)
        .map_err(|error| ApiError::internal(format!("identifying the change: {error}")))?;
    if binding.change_spec_id != change.change_spec_id {
        return Err(ApiError::internal(
            "the bound change and the resolved change disagree",
        ));
    }

    // ---- durable inputs, then durable intent --------------------------------
    //
    // Every input the worker will read is persisted, content-addressed or
    // hash-pinned, before the run record exists; the run record is the last
    // thing written, and the 202 follows it. So an accepted run never names an
    // input the service does not hold, and a restart at any point before the
    // record leaves no run at all rather than one that cannot execute.
    let stored = state
        .registry
        .artifacts()
        .put_program(&candidate)
        .map_err(|error| ApiError::internal(format!("storing the candidate: {error:#}")))?;
    if !stored.reference.matches(executable_candidate(&spec)?) {
        return Err(ApiError::internal(
            "the stored artifact is not the spec's candidate",
        ));
    }
    state
        .registry
        .index_project_artifact(&project.project_id, &stored.reference.sha256)
        .map_err(|error| ApiError::internal(format!("indexing the artifact: {error}")))?;

    let run_id = new_run_id();
    state
        .registry
        .save_change_spec(&run_id, &spec)
        .map_err(|error| ApiError::internal(format!("persisting the change spec: {error}")))?;
    let expectations_sha256 =
        match &upload.expectations {
            Some(bytes) => Some(state.registry.save_expectations(&run_id, bytes).map_err(
                |error| ApiError::internal(format!("persisting the expectations: {error}")),
            )?),
            None => None,
        };

    let metadata = RunMetadata {
        order_failure: None,
        hosted_analysis: None,
        analysis: None,
        run_id: run_id.clone(),
        project_id: project.project_id.clone(),
        status: RunStatus::Queued,
        bundle_sha256: manifest.bundle_sha256.clone(),
        bundle_id: Some(active.bundle_id.clone()),
        corpus_sha256: Some(manifest.corpus_sha256.clone()),
        baseline_sha256: Some(manifest.baseline_program_sha256.clone()),
        candidate_sha256: stored.reference.sha256.clone(),
        change: Some(change.clone()),
        candidate_artifact: Some(stored.reference.clone()),
        expectations_sha256,
        attempts: Vec::new(),
        adapter: Some(bundle.adapter().name.clone()),
        adapter_version: Some(bundle.adapter().version),
        semantic_schema_version: Some(manifest.semantic_schema_version),
        record_count: Some(manifest.record_count),
        exit_code: None,
        report_available: false,
        detail: None,
        created_at_unix_seconds: now_unix_seconds(),
        started_at_unix_seconds: None,
        completed_at_unix_seconds: None,
    };

    // Indexed before the record exists: an index entry whose run never got
    // written is skipped by every listing, while a run that exists but is in no
    // index would be invisible in history.
    state
        .registry
        .index_run(&project.project_id, &run_id)
        .map_err(|error| ApiError::internal(format!("indexing the run: {error}")))?;
    state
        .registry
        .index_change(&project.project_id, &change.change_spec_id, &run_id)
        .map_err(|error| ApiError::internal(format!("indexing the change: {error}")))?;
    // The run record is the durable queue entry. Only once it exists is a
    // worker started, so there is no window in which work exists that nothing
    // can be told about, and a restart from here re-enqueues it.
    state
        .registry
        .create_run(&metadata)
        .map_err(|error| ApiError::internal(format!("persisting the run: {error}")))?;
    worker::spawn(Arc::clone(&state), run_id.clone());

    Ok((
        StatusCode::ACCEPTED,
        Json(AcceptedResponse {
            status_url: format!("/v1/runs/{run_id}"),
            run_id,
            project_id,
            status: RunStatus::Queued,
            candidate_sha256: stored.reference.sha256.clone(),
            change,
            candidate_artifact: stored.reference,
            bundle_sha256: manifest.bundle_sha256.clone(),
        }),
    )
        .into_response())
}

pub(super) fn executable_candidate(spec: &ChangeSpec) -> ApiResult<&ExecutableArtifact> {
    spec.candidate().ok_or_else(|| {
        ApiError::bad_request("this operation requires an executable candidate")
            .with_exit_code(ci::EXIT_ERROR)
    })
}

/// The candidate an explicit spec names, from what this project has already
/// supplied. Scoped to the project on purpose: the store is shared and
/// deduplicated, and knowing another project's candidate hash must not be
/// enough to execute or detect its bytes.
pub(super) fn retained_candidate(
    state: &AppState,
    project_id: &str,
    spec: &ChangeSpec,
) -> ApiResult<Vec<u8>> {
    let wanted = ArtifactRef::from(executable_candidate(spec)?);
    let held = state
        .registry
        .project_holds_artifact(project_id, &wanted.sha256)
        .unwrap_or(false);
    if !held {
        return Err(ApiError::bad_request(format!(
            "the change spec names candidate {}, which this project has not supplied; \
             upload it as the `candidate` part",
            wanted.sha256
        ))
        .with_exit_code(ci::EXIT_ERROR));
    }
    state
        .registry
        .artifacts()
        .get_program(&wanted)
        .map_err(|error| {
            ApiError::internal(format!("the retained candidate does not verify: {error:#}"))
        })
}

#[derive(Serialize)]
pub(super) struct ArtifactView {
    pub(super) sha256: String,
    pub(super) len: u64,
    /// Held immutably, verified on this read. Never a path.
    pub(super) retained: bool,
}

/// Whether this project's candidate `sha256` is held, verified now.
///
/// Metadata only: no bytes, no storage path, no mutation. A hash the project
/// never supplied answers exactly like one the service has never seen.
pub(super) async fn get_artifact(
    State(state): State<Shared>,
    Path((project_id, sha256)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    project_for(&state, &project_id, &headers).await?;
    if !crate::artifacts::canonical_sha256(&sha256) {
        return Err(ApiError::bad_request(
            "an artifact is named by 64 lowercase hex characters",
        ));
    }
    let held = state
        .registry
        .project_holds_artifact(&project_id, &sha256)
        .unwrap_or(false);
    if !held {
        return Err(ApiError::not_found("artifact"));
    }
    let reference = state
        .registry
        .artifacts()
        .describe(&sha256)
        .map_err(|error| ApiError::internal(format!("the artifact does not verify: {error:#}")))?
        .ok_or_else(|| ApiError::not_found("artifact"))?;
    Ok((
        StatusCode::OK,
        Json(ArtifactView {
            sha256: reference.sha256,
            len: reference.len,
            retained: true,
        }),
    )
        .into_response())
}

/// What a check request carried. Every part is optional here; which
/// combinations are meaningful is `create_check`'s decision.
pub(super) struct Upload {
    pub(super) analysis: std::collections::BTreeMap<String, Vec<u8>>,
    pub(super) candidate: Option<Bytes>,
    pub(super) expectations: Option<Vec<u8>>,
    pub(super) change_spec: Option<Vec<u8>>,
    pub(super) label: Option<String>,
}

/// Read and bound the uploaded parts.
///
/// Sizes are checked as bytes arrive rather than after, and anything the
/// request names that we do not expect is refused rather than ignored. The
/// contract is additive: `candidate` and `expected_changes` mean what they
/// always meant, and `change_spec` and `label` are new and optional.
pub(super) async fn read_upload(state: &AppState, mut multipart: Multipart) -> ApiResult<Upload> {
    let mut upload = Upload {
        analysis: std::collections::BTreeMap::new(),
        candidate: None,
        expectations: None,
        change_spec: None,
        label: None,
    };
    loop {
        // Keep multipart's own status rather than flattening it: a body that
        // overruns the transport limit really is 413, and reporting it as a
        // malformed request would send a caller looking for a syntax error in a
        // file that is merely too big.
        let field = multipart.next_field().await.map_err(|error| {
            ApiError::new(error.status(), format!("malformed multipart: {error}"))
        })?;
        let Some(field) = field else { break };
        let name = field.name().unwrap_or_default().to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|error| ApiError::new(error.status(), format!("upload rejected: {error}")))?;
        let slot_taken = match name.as_str() {
            "state_input"
            | "state_artifact"
            | "snapshot"
            | "scenario"
            | "analysis_options"
            | "lifecycle_evidence"
            | "pinned_program_capture" => {
                let limit = if matches!(
                    name.as_str(),
                    "state_artifact" | "snapshot" | "lifecycle_evidence" | "pinned_program_capture"
                ) {
                    state.config.max_bundle_bytes
                } else {
                    MAX_CHANGE_SPEC_BYTES
                };
                if bytes.len() > limit {
                    return Err(ApiError::too_large(&name, limit));
                }
                upload
                    .analysis
                    .insert(name.clone(), bytes.to_vec())
                    .is_some()
            }
            "candidate" => {
                if bytes.len() > state.config.max_candidate_bytes {
                    return Err(ApiError::too_large(
                        "candidate",
                        state.config.max_candidate_bytes,
                    ));
                }
                upload.candidate.replace(bytes).is_some()
            }
            "expected_changes" => {
                if bytes.len() > state.config.max_expectation_bytes {
                    return Err(ApiError::too_large(
                        "expected_changes",
                        state.config.max_expectation_bytes,
                    ));
                }
                upload.expectations.replace(bytes.to_vec()).is_some()
            }
            "change_spec" => {
                if bytes.len() > MAX_CHANGE_SPEC_BYTES {
                    return Err(ApiError::too_large("change_spec", MAX_CHANGE_SPEC_BYTES));
                }
                upload.change_spec.replace(bytes.to_vec()).is_some()
            }
            "label" => {
                let label = String::from_utf8(bytes.to_vec())
                    .map_err(|_| ApiError::bad_request("label must be UTF-8 text"))?;
                let label = label.trim().to_string();
                if label.chars().count() > MAX_LABEL_CHARS || label.chars().any(char::is_control) {
                    return Err(ApiError::bad_request(format!(
                        "label must be at most {MAX_LABEL_CHARS} characters with no control characters"
                    )));
                }
                // An empty label is no label, not a label that is empty.
                !label.is_empty() && upload.label.replace(label).is_some()
            }
            other => {
                return Err(ApiError::bad_request(format!(
                    "unexpected upload field {other:?}"
                )))
            }
        };
        // Two candidates in one request would leave which one was meant to the
        // order the parts happened to arrive in.
        if slot_taken {
            return Err(ApiError::bad_request(format!(
                "upload field {name:?} was sent more than once"
            )));
        }
    }
    Ok(upload)
}

/// Opaque, ordered, and never derived from anything secret.
///
/// Seconds were not enough: two runs accepted in the same second ordered by
/// their random tail, so "newest first" was only true when a project was quiet.
pub(super) fn new_run_id() -> String {
    crate::ids::run()
}
