//! Per-kind analysis prerequisites for a project.

use super::*;

// ------------------------------------------------ project capabilities

/// Project-level prerequisites for one hosted analysis kind.
///
/// This is an informational pre-submission view. The submission handlers keep
/// validating the same authoritative state independently; this response is
/// never a capability token and says nothing about an analytical outcome.
#[derive(Clone, Serialize)]
pub(super) struct MissingPrerequisite {
    pub(super) code: &'static str,
    pub(super) message: &'static str,
    pub(super) action: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapabilityStatus {
    Ready,
    NotReady,
    Unsupported,
}

#[derive(Serialize)]
pub(super) struct AnalysisCapability {
    pub(super) kind: &'static str,
    pub(super) status: CapabilityStatus,
    pub(super) supported: bool,
    pub(super) can_submit: bool,
    pub(super) missing: Vec<MissingPrerequisite>,
}

#[derive(Serialize)]
pub(super) struct ProjectCapabilitiesResponse {
    pub(super) schema_version: u32,
    pub(super) project_id: String,
    pub(super) analyses: Vec<AnalysisCapability>,
}

pub(super) fn analysis_capability(
    kind: &'static str,
    supported: bool,
    common: &[MissingPrerequisite],
    specific: Vec<MissingPrerequisite>,
) -> AnalysisCapability {
    let missing = common.iter().cloned().chain(specific).collect::<Vec<_>>();
    let can_submit = supported && missing.is_empty();
    AnalysisCapability {
        kind,
        status: if !supported {
            CapabilityStatus::Unsupported
        } else if can_submit {
            CapabilityStatus::Ready
        } else {
            CapabilityStatus::NotReady
        },
        supported,
        can_submit,
        missing,
    }
}

pub(super) fn project_capabilities(
    state: &AppState,
    project: &Project,
) -> ProjectCapabilitiesResponse {
    let disabled = (project.status == ProjectStatus::Disabled).then_some(MissingPrerequisite {
        code: "project_disabled",
        message: "This project is disabled and does not accept hosted analyses.",
        action: "Ask the service operator to enable this project.",
    });
    let common = disabled.into_iter().collect::<Vec<_>>();

    let mut upgrade = Vec::new();
    if project.program_id.is_none() {
        upgrade.push(MissingPrerequisite {
            code: "upgrade_target_missing",
            message: "This project does not have a program upgrade target.",
            action: "Use a project configured for the program being upgraded.",
        });
    }
    match &project.active_bundle {
        None => upgrade.push(MissingPrerequisite {
            code: "active_bundle_missing",
            message: "This project does not have an active replay bundle.",
            action: "Ask the service operator to upload and activate a replay bundle.",
        }),
        Some(active) if state.registry.open_bundle(&active.bundle_sha256).is_err() => {
            upgrade.push(MissingPrerequisite {
                code: "active_bundle_unavailable",
                message: "This project's active replay bundle is unavailable.",
                action: "Ask the service operator to restore or replace the active replay bundle.",
            });
        }
        Some(_) => {}
    }

    let observation = || {
        if state.observation.is_none() {
            vec![MissingPrerequisite {
                code: "observation_service_unavailable",
                message: "Read-only current-state observation is not configured.",
                action: "Ask the service operator to configure the observation service.",
            }]
        } else {
            Vec::new()
        }
    };
    let mut candidate = observation();
    if state
        .observation
        .as_ref()
        .is_none_or(|service| service.candidate.is_none())
    {
        candidate.push(MissingPrerequisite {
            code: "migration_candidate_not_configured",
            message: "The hosted migration mechanism is not configured.",
            action: "Ask the service operator to register the hosted migration mechanism.",
        });
    }

    // Stable order: the public kinds first, followed by the current-state
    // workflow's durable subchecks using their existing hosted job names.
    let analyses = vec![
        analysis_capability("program_upgrade", true, &common, upgrade),
        analysis_capability("token_migration", true, &common, Vec::new()),
        analysis_capability("lifecycle_change", true, &common, Vec::new()),
        analysis_capability("protocol_parameter_change", true, &common, Vec::new()),
        analysis_capability("upgrade_parameter_interaction", true, &common, Vec::new()),
        analysis_capability("current_observation", true, &common, observation()),
        analysis_capability("current_path", true, &common, observation()),
        analysis_capability("current_candidate", true, &common, candidate),
        analysis_capability("current_preflight", true, &common, observation()),
        analysis_capability("current_stress", true, &common, observation()),
    ];
    ProjectCapabilitiesResponse {
        schema_version: 1,
        project_id: project.project_id.clone(),
        analyses,
    }
}

pub(super) async fn get_project_capabilities(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let project = project_for(&state, &project_id, &headers).await?;
    Ok((StatusCode::OK, Json(project_capabilities(&state, &project))).into_response())
}
