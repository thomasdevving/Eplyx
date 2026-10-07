//! Squads governance verification, attestation and trail.

use super::*;

// -------------------------------------------------------------- governance
//
// Verification only. Nothing here signs, approves, rejects, cancels or
// executes a proposal, and the service holds no key that could. Every answer
// is re-read from the chain on request; a stored binding is shown as what it
// was, at its slot, never replayed as a current answer.

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SquadsVerifyRequest {
    pub(super) multisig: String,
    pub(super) transaction_index: u64,
    #[serde(default)]
    pub(super) change_spec_id: Option<String>,
    #[serde(default)]
    pub(super) run_id: Option<String>,
    #[serde(default)]
    pub(super) commitment: Option<Commitment>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SquadsAttestRequest {
    pub(super) change_spec_id: String,
    pub(super) binding_id: String,
}

/// Check a previously bound change against the exact successful Squads
/// execution, then persist a separate immutable post-execution proof.
pub(super) async fn attest_squads_governance(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    let request: SquadsAttestRequest = parse_json(&body)?;
    if !crate::artifacts::canonical_sha256(&request.change_spec_id)
        || !crate::artifacts::canonical_sha256(&request.binding_id)
    {
        return Err(ApiError::bad_request(
            "change_spec_id and binding_id must be canonical SHA-256 identifiers",
        ));
    }
    let rpc = state.governance.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance verification is not configured on this server (EPLYX_GOVERNANCE_RPC_URL)",
        )
    })?;
    let spec = state
        .registry
        .load_governance_spec(&project.project_id, &request.change_spec_id)
        .map_err(|e| ApiError::internal(format!("loading governance-bound change: {e:#}")))?
        .ok_or_else(|| ApiError::not_found("governance-bound change"))?;
    let binding_path = state
        .registry
        .storage()
        .project_governance_dir(&project.project_id, &request.change_spec_id)
        .map_err(|_| ApiError::internal("governance storage unavailable"))?
        .join("bindings")
        .join(format!("{}.json", request.binding_id));
    if !binding_path.is_file() {
        return Err(ApiError::not_found("matched G1 binding for this change"));
    }
    let binding = state
        .registry
        .governance_binding(
            &project.project_id,
            &request.change_spec_id,
            &request.binding_id,
        )
        .map_err(|_| ApiError::internal("stored G1 binding integrity check failed"))?;
    if !state
        .registry
        .project_holds_artifact(&project.project_id, &executable_candidate(&spec)?.sha256)
        .map_err(|e| ApiError::internal(format!("checking candidate ownership: {e:#}")))?
    {
        return Err(ApiError::not_found("candidate artifact in this project"));
    }
    let candidate = state
        .registry
        .artifacts()
        .get_program(&ArtifactRef::from(executable_candidate(&spec)?))
        .map_err(|e| ApiError::internal(format!("candidate artifact does not verify: {e:#}")))?;
    let attestation: DeploymentAttestation = tokio::task::spawn_blocking(move || {
        attestation::attest_squads_upgrade(rpc.as_ref(), &spec, &binding, &candidate)
    })
    .await
    .map_err(|e| ApiError::internal(format!("deployment attestation stopped: {e}")))?
    .map_err(|e| {
        ApiError::bad_request(format!("deployment attestation precondition failed: {e:#}"))
    })?;
    state
        .registry
        .record_deployment_attestation(&project.project_id, &attestation)
        .map_err(|e| ApiError::internal(format!("recording deployment attestation: {e:#}")))?;
    Ok((StatusCode::OK, Json(attestation)).into_response())
}

/// The analysed spec a request names: a run of this project, or a change this
/// project has analysed or bound before. Always re-verified on read.
pub(super) fn analysed_spec(
    state: &AppState,
    project_id: &str,
    request: &SquadsVerifyRequest,
) -> ApiResult<ChangeSpec> {
    let from_run = |run_id: &str| -> ApiResult<ChangeSpec> {
        let metadata = state
            .registry
            .load_run(run_id)
            .ok()
            .filter(|run| run.project_id == project_id)
            .ok_or_else(|| ApiError::not_found("run"))?;
        let change = metadata.change.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "this run was recorded before change identity existed and cannot be bound",
            )
        })?;
        state
            .registry
            .load_change_spec(run_id, change)
            .map_err(|error| ApiError::internal(format!("{error:#}")))
    };
    match (&request.run_id, &request.change_spec_id) {
        (Some(run_id), None) => from_run(run_id),
        (None, Some(id)) => {
            if !crate::artifacts::canonical_sha256(id) {
                return Err(ApiError::bad_request(
                    "a change_spec_id is 64 lowercase hex characters",
                ));
            }
            let runs = state
                .registry
                .change_run_ids(project_id, id)
                .map_err(|error| ApiError::internal(format!("{error}")))?;
            if let Some(run_id) = runs.first() {
                return from_run(run_id);
            }
            state
                .registry
                .load_governance_spec(project_id, id)
                .map_err(|error| ApiError::internal(format!("{error:#}")))?
                .ok_or_else(|| ApiError::not_found("change in this project"))
        }
        _ => Err(ApiError::bad_request(
            "name exactly one of `change_spec_id` or `run_id`",
        )),
    }
}

#[derive(Serialize)]
pub(super) struct GovernanceCheckView {
    pub(super) check_id: String,
    pub(super) checked_at_unix_seconds: u64,
    pub(super) status: BindingOutcome,
    /// The `eplyx governance` exit code for this outcome.
    pub(super) exit_code: u8,
    pub(super) statement: String,
    /// When this answer was true. Every consumer must show it.
    pub(super) observed_slot: Option<u64>,
    pub(super) commitment: Commitment,
    /// The governance-bound change: what an analysis must carry to be a
    /// verdict about this proposal. The analysed id when it was not decodable.
    pub(super) change_spec_id: String,
    pub(super) analysed_change_spec_id: String,
    /// The analysed spec already named this proposal.
    pub(super) governance_bound: bool,
    pub(super) proposal: serde_json::Value,
    pub(super) target: serde_json::Value,
    pub(super) buffer: serde_json::Value,
    pub(super) expected_candidate: serde_json::Value,
    pub(super) binding_id: String,
    pub(super) binding: GovernanceBinding,
}

impl GovernanceCheckView {
    fn of(
        check: GovernanceCheck,
        binding: GovernanceBinding,
        candidate_held: Option<bool>,
    ) -> Self {
        let observed = &binding.observation;
        let delivery = observed.delivery.as_ref();
        let proposal = json!({
            "multisig": binding.request.multisig,
            "transaction_index": binding.request.transaction_index,
            "vault_index": delivery.map(|d| d.vault_index),
            "vault": delivery.map(|d| d.vault.clone()),
            "transaction": delivery.map(|d| d.transaction.clone()),
            "proposal": delivery.map(|d| d.proposal.clone()),
            "message_sha256": delivery.map(|d| d.message_sha256.clone()),
            "status": observed.proposal.as_ref().map(|p| p.status),
            "stale": observed.proposal.as_ref().map(|p| p.stale),
            "approvals": observed.proposal.as_ref().map(|p| p.approvals),
            "threshold": observed.multisig.as_ref().map(|m| m.threshold),
        });
        let target = json!({
            "program": observed.upgrade.as_ref().map(|u| u.program.clone()),
            "programdata": observed.upgrade.as_ref().map(|u| u.programdata.clone()),
            "upgrade_authority": observed.current_program.as_ref().map(|c| c.upgrade_authority.clone()),
        });
        let buffer = json!({
            "address": observed.upgrade.as_ref().map(|u| u.buffer.clone()),
            "sha256": observed.buffer.as_ref().map(|b| b.artifact.sha256.clone()),
            "len": observed.buffer.as_ref().map(|b| b.artifact.len),
            "authority": observed.buffer.as_ref().map(|b| b.authority.clone()),
        });
        let mut expected_candidate = json!({
            "sha256": binding.expected.candidate.sha256,
            "len": binding.expected.candidate.len,
        });
        if let Some(held) = candidate_held {
            expected_candidate["held_by_project"] = json!(held);
        }
        Self {
            check_id: check.check_id,
            checked_at_unix_seconds: check.checked_at_unix_seconds,
            status: binding.outcome,
            exit_code: binding.outcome.exit_code(),
            statement: binding.statement.clone(),
            observed_slot: observed.slot,
            commitment: binding.commitment,
            change_spec_id: check.change_spec_id,
            analysed_change_spec_id: binding.analysed_change_spec_id.clone(),
            governance_bound: binding.bound_change_spec_id.as_deref()
                == Some(binding.analysed_change_spec_id.as_str()),
            proposal,
            target,
            buffer,
            expected_candidate,
            binding_id: check.binding_id,
            binding,
        }
    }
}

/// Re-read a Squads proposal and its buffer now, and bind it to an analysed
/// change of this project.
///
/// A project token suffices: this reads the chain and records what it saw,
/// and changes nothing any other check depends on. When the analysed spec was
/// not yet bound, a match returns the governance-bound spec to submit as a
/// check, so the report that follows names this proposal; the service never
/// relabels an existing report. Buffer bytes are never acquired from the chain
/// here: the candidate must be one this project already supplied.
pub(super) async fn verify_squads_governance(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    let request: SquadsVerifyRequest = parse_json(&body)?;
    let rpc = state.governance.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance verification is not configured on this server (EPLYX_GOVERNANCE_RPC_URL)",
        )
    })?;
    let spec = analysed_spec(&state, &project.project_id, &request)?;
    let proposal = SquadsProposalRef {
        multisig: request.multisig.clone(),
        transaction_index: request.transaction_index,
    };
    let commitment = request.commitment.unwrap_or_default();
    let checked = spec.clone();
    let binding = tokio::task::spawn_blocking(move || {
        governance::verify_squads_upgrade(rpc.as_ref(), &proposal, &checked, commitment)
    })
    .await
    .map_err(|error| ApiError::internal(format!("governance verification stopped: {error}")))?
    .map_err(|error| ApiError::bad_request(format!("{error:#}")).with_exit_code(ci::EXIT_ERROR))?;

    // The last index is the bound change when one was derived: the id an
    // analysis must carry to name this proposal.
    let check = state
        .registry
        .record_governance_check(&project.project_id, &binding)
        .map_err(|error| ApiError::internal(format!("recording the governance check: {error:#}")))?
        .pop()
        .ok_or_else(|| ApiError::internal("a governance check was indexed nowhere"))?;
    let bound = if binding.outcome == BindingOutcome::Matched {
        let bound = binding
            .bound_spec(&spec)
            .map_err(|error| ApiError::internal(format!("{error:#}")))?;
        if let Some(bound) = &bound {
            state
                .registry
                .save_governance_spec(&project.project_id, bound)
                .map_err(|error| ApiError::internal(format!("{error:#}")))?;
        }
        bound
    } else {
        None
    };
    let held = state
        .registry
        .project_holds_artifact(&project.project_id, &executable_candidate(&spec)?.sha256)
        .unwrap_or(false);
    let analysis_runs = state
        .registry
        .change_run_ids(&project.project_id, &check.change_spec_id)
        .unwrap_or_default();
    // Offered only when the analysed spec did not already name this proposal:
    // it is what to submit as a check so the report names the proposal.
    let bound_document = match &bound {
        Some(bound)
            if bound.id().ok().as_deref() != Some(binding.analysed_change_spec_id.as_str()) =>
        {
            let document = bound
                .to_document()
                .map_err(|error| ApiError::internal(format!("{error:#}")))?;
            serde_json::from_str::<serde_json::Value>(&document)
                .map_err(|error| ApiError::internal(format!("{error}")))?
        }
        _ => serde_json::Value::Null,
    };
    let mut view = serde_json::to_value(GovernanceCheckView::of(check, binding, Some(held)))
        .map_err(|error| ApiError::internal(format!("{error}")))?;
    view["analysis"] = json!({ "runs": analysis_runs, "bound_change_spec": bound_document });
    Ok((StatusCode::OK, Json(view)).into_response())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TrailQuery {
    pub(super) limit: Option<usize>,
    pub(super) cursor: Option<String>,
    pub(super) run_cursor: Option<String>,
}

pub(super) async fn governance_trail(
    State(state): State<Shared>,
    Path((project_id, change_spec_id)): Path<(String, String)>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<TrailQuery>,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    if !crate::artifacts::canonical_sha256(&change_spec_id) {
        return Err(ApiError::bad_request("invalid change_spec_id"));
    }
    if let Some(cursor) = &query.cursor {
        crate::governance_trail::event_key(cursor)
            .map_err(|_| ApiError::bad_request("invalid trail cursor"))?;
    }
    if query
        .run_cursor
        .as_ref()
        .is_some_and(|id| !crate::storage::valid_id(id) || !id.starts_with("run_"))
    {
        return Err(ApiError::bad_request("invalid run cursor"));
    }
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let spec = analysed_spec(
        &state,
        &project.project_id,
        &SquadsVerifyRequest {
            multisig: String::new(),
            transaction_index: 0,
            change_spec_id: Some(change_spec_id.clone()),
            run_id: None,
            commitment: None,
        },
    )
    .map_err(|e| {
        if e.status == StatusCode::INTERNAL_SERVER_ERROR {
            ApiError::internal("stored governance root integrity check failed")
        } else {
            e
        }
    })?;
    if spec
        .id()
        .map_err(|_| ApiError::internal("stored governance root integrity check failed"))?
        != change_spec_id
    {
        return Err(ApiError::internal(
            "stored governance root identity differs",
        ));
    }
    if spec.delivery().is_none() {
        return Err(ApiError::bad_request(
            "trail root must be a governance-bound ChangeSpec",
        ));
    }
    let trail = state
        .registry
        .governance_trail(
            &project.project_id,
            &spec,
            query.cursor.as_deref(),
            query.run_cursor.as_deref(),
            limit,
        )
        .map_err(|_| ApiError::internal("stored governance trail integrity check failed"))?;
    Ok(Json(trail).into_response())
}

/// The recorded checks of one change, newest first. What was observed and
/// when, each at its own slot; nothing here is re-read from the chain.
pub(super) async fn list_governance_checks(
    State(state): State<Shared>,
    Path((project_id, change_spec_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    if !crate::artifacts::canonical_sha256(&change_spec_id) {
        return Err(ApiError::bad_request(
            "a change_spec_id is 64 lowercase hex characters",
        ));
    }
    let checks = state
        .registry
        .governance_checks(&project.project_id, &change_spec_id, 20)
        .map_err(|error| ApiError::internal(format!("{error:#}")))?;
    let checks: Vec<GovernanceCheckView> = checks
        .into_iter()
        .map(|(check, binding)| GovernanceCheckView::of(check, binding, None))
        .collect();
    let attestations = state
        .registry
        .deployment_attestations(&project.project_id, &change_spec_id, 20)
        .map_err(|error| ApiError::internal(format!("{error:#}")))?;
    Ok((
        StatusCode::OK,
        Json(json!({ "change_spec_id": change_spec_id, "checks": checks, "attestations": attestations })),
    )
        .into_response())
}
