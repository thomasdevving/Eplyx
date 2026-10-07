//! Hosted multi-kind analytical checks.

use super::*;

/// Additive multi-kind check contract. Every part is data, never executable
/// source or a client-selected command. Captures are frozen before queuing.
pub(super) async fn create_analytical_check(
    state: Shared,
    project: &str,
    spec: ChangeSpec,
    mut upload: Upload,
) -> ApiResult<Response> {
    use crate::{artifacts::ArtifactClass, hosted::Input};
    #[derive(serde::Deserialize, Default)]
    #[serde(deny_unknown_fields)]
    struct Options {
        policy: Option<eplyx_engine::migration::gate::Policy>,
        before: Option<chrono::DateTime<chrono::Utc>>,
        at: Option<chrono::DateTime<chrono::Utc>>,
    }
    if upload.expectations.is_some() || upload.label.is_some() {
        return Err(ApiError::bad_request(
            "analytical checks take declared ChangeSpec and kind-specific inputs",
        ));
    }
    let options: Options = upload
        .analysis
        .remove("analysis_options")
        .map(|b| serde_json::from_slice(&b))
        .transpose()
        .map_err(|_| ApiError::bad_request("invalid analysis options"))?
        .unwrap_or_default();
    let canonical = spec
        .to_document()
        .map_err(|_| ApiError::bad_request("invalid change spec"))?;
    let change = state
        .registry
        .document_ref(canonical.as_bytes())
        .map_err(|_| ApiError::internal("input storage failed"))?;
    let mut evidence = std::collections::BTreeMap::new();
    if let Some(bytes) = upload.analysis.remove("lifecycle_evidence") {
        if !matches!(
            spec.change,
            eplyx_engine::change::Change::LifecycleChange(_)
        ) {
            return Err(ApiError::bad_request(
                "source evidence requires a lifecycle change",
            ));
        }
        let documents: std::collections::BTreeMap<String, eplyx_engine::cloud::contract::Artifact> =
            serde_json::from_slice(&bytes)
                .map_err(|_| ApiError::bad_request("invalid lifecycle evidence"))?;
        if documents.len() > 32 {
            return Err(ApiError::bad_request("too many source artifacts"));
        }
        for (id, artifact) in documents {
            if artifact.sha256 != eplyx_engine::replay::hash_bytes(artifact.text.as_bytes()) {
                return Err(ApiError::bad_request("source artifact digest mismatch"));
            }
            evidence.insert(
                id,
                state
                    .registry
                    .artifacts()
                    .put(ArtifactClass::Capture, artifact.text.as_bytes())
                    .map_err(|_| ApiError::internal("source storage failed"))?
                    .reference,
            );
        }
    }
    let program_capture = upload
        .analysis
        .remove("pinned_program_capture")
        .map(|bytes| {
            state
                .registry
                .artifacts()
                .put(ArtifactClass::Capture, &bytes)
                .map(|v| v.reference)
                .map_err(|_| ApiError::internal("input storage failed"))
        })
        .transpose()?;
    let mut take = |name: &str, class: ArtifactClass| -> ApiResult<ArtifactRef> {
        let bytes = upload
            .analysis
            .remove(name)
            .ok_or_else(|| ApiError::bad_request(format!("{name} is required")))?;
        state
            .registry
            .artifacts()
            .put(class, &bytes)
            .map(|v| v.reference)
            .map_err(|_| ApiError::internal("input storage failed"))
    };
    let input = match &spec.change {
        eplyx_engine::change::Change::TokenMigration(_) => {
            if options.before.is_some() || options.at.is_some() {
                return Err(ApiError::bad_request(
                    "migration clock comes from the frozen state input",
                ));
            }
            let candidate = match upload.candidate.take() {
                Some(b) => b.to_vec(),
                None => retained_candidate(&state, project, &spec)?,
            };
            spec.resolve(CandidateSource::Bytes(&candidate))
                .map_err(|_| ApiError::bad_request("candidate does not resolve the change spec"))?;
            let candidate = state
                .registry
                .artifacts()
                .put_program(&candidate)
                .map_err(|_| ApiError::internal("candidate storage failed"))?
                .reference;
            Input::TokenMigration {
                change,
                candidate,
                state_input: take("state_input", ArtifactClass::Document)?,
                state_artifact: take("state_artifact", ArtifactClass::Capture)?,
                program_capture,
                policy: options.policy.unwrap_or_default(),
            }
        }
        eplyx_engine::change::Change::LifecycleChange(_) => {
            if upload.candidate.is_some() || options.policy.is_some() || program_capture.is_some() {
                return Err(ApiError::bad_request("lifecycle evaluation does not take executable bytes or a deployment gate policy"));
            }
            Input::LifecycleChange {
                change,
                evidence,
                snapshot: take("snapshot", ArtifactClass::Capture)?,
                scenario: take("scenario", ArtifactClass::Document)?,
                before: options
                    .before
                    .ok_or_else(|| ApiError::bad_request("before is required"))?,
                at: options
                    .at
                    .ok_or_else(|| ApiError::bad_request("at is required"))?,
            }
        }
        _ => return Err(ApiError::bad_request("unsupported analytical kind")),
    };
    if !upload.analysis.is_empty() {
        return Err(ApiError::bad_request(
            "unexpected inputs for this analytical kind",
        ));
    }
    let registry_state = Arc::clone(&state);
    let project = project.to_owned();
    let record = tokio::task::spawn_blocking(move || {
        registry_state
            .registry
            .create_hosted_analysis(&project, input)
    })
    .await
    .map_err(|_| ApiError::internal("input validation stopped"))?
    .map_err(|_| ApiError::bad_request("analytical inputs do not validate together"))?;
    worker::spawn(Arc::clone(&state), record.run_id.clone());
    Ok((StatusCode::ACCEPTED,Json(json!({"run_id":record.run_id,"project_id":record.project_id,"status":record.status,"change":record.change,"status_url":format!("/v1/runs/{}",record.run_id)}))).into_response())
}
