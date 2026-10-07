//! The guided path to a project's first upgrade check.
//!
//! `capabilities` answers "may this kind be submitted now"; this answers "what
//! is still missing, in which order, and who can supply it". Every step is
//! derived from the same authoritative records the submission handlers read —
//! the project, its bundles, its tokens and its runs — and nothing here is a
//! capability or a verdict. A finished checklist means a check *can* run, not
//! that a candidate is safe.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StepStatus {
    /// Satisfied by a record the service holds.
    Done,
    /// Satisfied, with something the caller should know before relying on it.
    Attention,
    /// Accepted work exists and has not reached an outcome yet.
    InProgress,
    /// Missing, and nothing earlier stands in the way.
    Todo,
    /// Missing, and an earlier required step has to be finished first.
    Blocked,
    /// Not required for a first check.
    Optional,
}

impl StepStatus {
    fn satisfied(self) -> bool {
        matches!(self, Self::Done | Self::Attention)
    }
}

/// Who can perform an action. Instructions only: no credential is ever part
/// of a command, and a project token cannot perform operator actions just
/// because it can read them.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Actor {
    Operator,
    WorkspaceMember,
    Repository,
}

#[derive(Serialize)]
pub(super) struct SetupAction {
    pub(super) label: &'static str,
    pub(super) actor: Actor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) command: Option<String>,
}

#[derive(Serialize)]
pub(super) struct SetupStep {
    pub(super) id: &'static str,
    pub(super) title: &'static str,
    pub(super) required: bool,
    pub(super) status: StepStatus,
    pub(super) detail: String,
    /// What the service holds for this step, where it holds something.
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub(super) evidence: serde_json::Value,
    pub(super) actions: Vec<SetupAction>,
}

#[derive(Serialize)]
pub(super) struct ProjectSetupResponse {
    pub(super) schema_version: u32,
    pub(super) project_id: String,
    pub(super) kind: &'static str,
    /// Every required step before the first check is satisfied.
    pub(super) ready_for_first_check: bool,
    /// A first check has reached an analytical outcome.
    pub(super) first_check_complete: bool,
    /// The first required step that is not satisfied, if any.
    pub(super) next_step: Option<&'static str>,
    pub(super) steps: Vec<SetupStep>,
    /// The repository configuration `.github/workflows/eplyx.yml` reads.
    pub(super) repository: serde_json::Value,
}

/// What the project's run history says about upgrade checks.
#[derive(Default)]
pub(super) struct UpgradeHistory {
    pub(super) verdicts: usize,
    pub(super) pending: usize,
    pub(super) execution_errors: usize,
    pub(super) with_expectations: usize,
    pub(super) latest: Option<RunMetadata>,
}

/// Bounded: a checklist needs the recent past, not the whole history.
const HISTORY_WINDOW: usize = 200;

pub(super) fn upgrade_history(state: &AppState, project_id: &str) -> UpgradeHistory {
    let mut history = UpgradeHistory::default();
    let mut ids = state
        .registry
        .project_run_ids(project_id)
        .unwrap_or_default();
    // Run ids sort by creation time; newest first.
    ids.sort();
    for run_id in ids.iter().rev().take(HISTORY_WINDOW) {
        let Ok(run) = state.registry.load_run(run_id) else {
            continue;
        };
        if run.analysis.is_some() || run.hosted_analysis.is_some() {
            continue;
        }
        match run.status {
            RunStatus::Passed | RunStatus::Failed => history.verdicts += 1,
            RunStatus::Queued | RunStatus::Running => history.pending += 1,
            RunStatus::ExecutionError => history.execution_errors += 1,
            RunStatus::Completed => {}
        }
        if run.expectations_sha256.is_some() {
            history.with_expectations += 1;
        }
        if history.latest.is_none() {
            history.latest = Some(run);
        }
    }
    history
}

/// Live CI credentials for the project, across both token stores.
async fn live_project_tokens(state: &AppState, project_id: &str) -> usize {
    let filesystem = state
        .registry
        .list_tokens(project_id)
        .map(|tokens| {
            tokens
                .iter()
                .filter(|token| token.revoked_at_unix_seconds.is_none())
                .count()
        })
        .unwrap_or(0);
    let mut identity = 0;
    if let Some(cloud) = &state.identity {
        if let Ok(db) = cloud.db.get().await {
            if let Ok(row) = db
                .query_one(
                    "SELECT count(*) FROM api_tokens WHERE project_id=$1 AND kind='project' \
                     AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())",
                    &[&project_id],
                )
                .await
            {
                identity = row.get::<_, i64>(0).max(0) as usize;
            }
        }
    }
    filesystem + identity
}

pub(super) fn project_setup(
    state: &AppState,
    project: &Project,
    principal_is_project_token: bool,
    live_tokens: usize,
    history: &UpgradeHistory,
) -> ProjectSetupResponse {
    let p = &project.project_id;
    let mut steps = Vec::new();

    // 1. The project accepts work at all.
    let enabled = project.status != ProjectStatus::Disabled;
    steps.push(SetupStep {
        id: "project_enabled",
        title: "Project accepts hosted analyses",
        required: true,
        status: if enabled {
            StepStatus::Done
        } else {
            StepStatus::Todo
        },
        detail: if enabled {
            "The project is enabled.".into()
        } else {
            "The project is disabled and refuses every submission.".into()
        },
        evidence: json!({ "status": project.status }),
        actions: if enabled {
            vec![]
        } else {
            vec![SetupAction {
                label: "Ask the service operator to enable this project",
                actor: Actor::Operator,
                command: None,
            }]
        },
    });

    // 2. What is being upgraded, and whether this build can read it.
    let target = match &project.program_id {
        None => SetupStep {
            id: "upgrade_target",
            title: "Program to upgrade",
            required: true,
            status: StepStatus::Todo,
            detail: "The project has no program upgrade target, so no upgrade check can be bound to it.".into(),
            evidence: serde_json::Value::Null,
            actions: vec![SetupAction {
                label: "Create a project for the program being upgraded",
                actor: Actor::Operator,
                command: Some(
                    "eplyx-server admin create-project --name <name> --program-id <PROGRAM_ID>"
                        .into(),
                ),
            }],
        },
        Some(program) if !project.adapter_id.speaks_semantics() => SetupStep {
            id: "upgrade_target",
            title: "Program to upgrade",
            required: true,
            status: StepStatus::Attention,
            detail: format!(
                "Target {program}. This build has no semantic adapter for it, so every check will report no semantic coverage and exit 2: the engine did not look, which is not the same as nothing changed."
            ),
            evidence: json!({ "program_id": program, "adapter": project.adapter_id }),
            actions: vec![],
        },
        Some(program) => SetupStep {
            id: "upgrade_target",
            title: "Program to upgrade",
            required: true,
            status: StepStatus::Done,
            detail: format!("Target {program}, read by adapter {}.", project.adapter_id),
            evidence: json!({ "program_id": program, "adapter": project.adapter_id }),
            actions: vec![],
        },
    };
    steps.push(target);

    // 3. Replay evidence: a production-derived bundle the operator built.
    let bundles = state.registry.list_bundles(p).unwrap_or_default();
    let newest = bundles
        .iter()
        .max_by_key(|bundle| bundle.created_at_unix_seconds);
    steps.push(SetupStep {
        id: "bundle_registered",
        title: "Replay evidence registered",
        required: true,
        status: if newest.is_some() {
            StepStatus::Done
        } else {
            StepStatus::Todo
        },
        detail: match newest {
            Some(bundle) => format!(
                "{} bundle(s) registered; the newest replays {} validated record(s).",
                bundles.len(),
                bundle.record_count
            ),
            None => "No replay bundle is registered. A check measures a candidate against validated production transactions, and without them there is nothing to replay.".into(),
        },
        evidence: newest
            .map(|bundle| {
                json!({
                    "bundle_count": bundles.len(),
                    "newest_bundle_id": bundle.bundle_id,
                    "bundle_sha256": bundle.bundle_sha256,
                    "baseline_sha256": bundle.baseline_sha256,
                    "record_count": bundle.record_count,
                })
            })
            .unwrap_or(serde_json::Value::Null),
        actions: if newest.is_some() {
            vec![]
        } else {
            vec![
                SetupAction {
                    label: "Build and verify a bundle from acquired, reproducing records",
                    actor: Actor::Operator,
                    command: Some(
                        "eplyx bundle build --corpus <DIR> --baseline current.so --dependencies <DIR>/dependencies --target-size 10 --out bundle && eplyx bundle verify --bundle bundle"
                            .into(),
                    ),
                },
                SetupAction {
                    label: "Register it with this project",
                    actor: Actor::Operator,
                    command: Some(format!(
                        "eplyx-server admin register-bundle --project {p} --path bundle"
                    )),
                },
            ]
        },
    });

    // 4. Which bundle checks are measured against. Activation is deliberate.
    let active_ok = project
        .active_bundle
        .as_ref()
        .map(|active| state.registry.open_bundle(&active.bundle_sha256).is_ok());
    steps.push(SetupStep {
        id: "bundle_active",
        title: "Baseline activated",
        required: true,
        status: match (active_ok, newest.is_some()) {
            (Some(true), _) => StepStatus::Done,
            (Some(false), _) => StepStatus::Todo,
            (None, true) => StepStatus::Todo,
            (None, false) => StepStatus::Blocked,
        },
        detail: match (&project.active_bundle, active_ok) {
            (Some(active), Some(true)) => format!(
                "Checks are measured against bundle {}.",
                active.bundle_id
            ),
            (Some(active), _) => format!(
                "Active bundle {} is not readable on this volume; checks are refused until it is restored or replaced.",
                active.bundle_id
            ),
            (None, _) if newest.is_some() => "A bundle is registered but none is active. Activation is a separate, deliberate step because it moves what every later pull request is measured against.".into(),
            (None, _) => "Register a bundle first.".into(),
        },
        evidence: project
            .active_bundle
            .as_ref()
            .map(|active| json!({ "bundle_id": active.bundle_id, "bundle_sha256": active.bundle_sha256, "readable": active_ok == Some(true) }))
            .unwrap_or(serde_json::Value::Null),
        actions: match (active_ok, newest) {
            (Some(true), _) => vec![],
            (_, Some(bundle)) => vec![SetupAction {
                label: "Activate the reviewed bundle",
                actor: Actor::Operator,
                command: Some(format!(
                    "eplyx-server admin activate-bundle --project {p} --bundle {}",
                    bundle.bundle_id
                )),
            }],
            (_, None) => vec![],
        },
    });

    // 5. A credential the repository can submit with.
    let has_token = principal_is_project_token || live_tokens > 0;
    steps.push(SetupStep {
        id: "ci_token",
        title: "CI token issued",
        required: true,
        status: if has_token {
            StepStatus::Done
        } else {
            StepStatus::Todo
        },
        detail: if has_token {
            "A live project token exists. It can submit checks for this project and cannot change its baseline.".into()
        } else {
            "No live project token exists, so a repository cannot submit checks.".into()
        },
        evidence: if principal_is_project_token {
            json!({ "live_tokens": serde_json::Value::Null, "caller_is_project_token": true })
        } else {
            json!({ "live_tokens": live_tokens })
        },
        actions: if has_token {
            vec![]
        } else {
            vec![
                SetupAction {
                    label: "Issue a project token from the project's settings page",
                    actor: Actor::WorkspaceMember,
                    command: None,
                },
                SetupAction {
                    label: "Or issue one on the service host",
                    actor: Actor::Operator,
                    command: Some(format!(
                        "eplyx-server admin create-token --project {p} --label github-ci"
                    )),
                },
                SetupAction {
                    label: "Store it as the repository secret EPLYX_TOKEN",
                    actor: Actor::Repository,
                    command: Some("gh secret set EPLYX_TOKEN".into()),
                },
            ]
        },
    });

    // 6. Declared intent. A first check runs without it; an intended change
    // then simply reports as unexpected.
    steps.push(SetupStep {
        id: "expectations",
        title: "Intended changes declared",
        required: false,
        status: if history.with_expectations > 0 {
            StepStatus::Done
        } else {
            StepStatus::Optional
        },
        detail: if history.with_expectations > 0 {
            format!(
                "{} recent check(s) carried an expected-changes.toml.",
                history.with_expectations
            )
        } else {
            "No check has carried an expected-changes.toml. Without one, every semantic change is reported as unexpected — correct for a release that should change nothing.".into()
        },
        evidence: serde_json::Value::Null,
        actions: if history.with_expectations > 0 {
            vec![]
        } else {
            vec![SetupAction {
                label: "Declare intended changes narrowly in .eplyx/expected-changes.toml",
                actor: Actor::Repository,
                command: None,
            }]
        },
    });

    let prerequisites_done = steps
        .iter()
        .filter(|step| step.required)
        .all(|step| step.status.satisfied());

    // 7. The first check itself.
    let latest = history.latest.as_ref();
    let first_status = if history.verdicts > 0 {
        StepStatus::Done
    } else if history.pending > 0 {
        StepStatus::InProgress
    } else if history.execution_errors > 0 {
        StepStatus::Attention
    } else if prerequisites_done {
        StepStatus::Todo
    } else {
        StepStatus::Blocked
    };
    steps.push(SetupStep {
        id: "first_check",
        title: "First upgrade check",
        required: true,
        status: first_status,
        detail: match first_status {
            StepStatus::Done => format!(
                "{} check(s) reached a verdict; latest {} is {}.",
                history.verdicts,
                latest.map(|run| run.run_id.as_str()).unwrap_or("-"),
                latest
                    .map(|run| serde_json::to_value(run.status).unwrap_or_default())
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            ),
            StepStatus::InProgress => "A check is queued or running.".into(),
            StepStatus::Attention => format!(
                "{} check(s) ended without a verdict (execution_error). That says nothing about the candidate; see the run's detail.",
                history.execution_errors
            ),
            StepStatus::Todo => "Everything a first check needs is in place.".into(),
            _ => "Finish the required steps above first.".into(),
        },
        evidence: latest
            .map(|run| {
                json!({
                    "run_id": run.run_id,
                    "status": run.status,
                    "exit_code": run.exit_code,
                    "detail": run.detail,
                })
            })
            .unwrap_or(serde_json::Value::Null),
        actions: if matches!(first_status, StepStatus::Done | StepStatus::InProgress) {
            vec![]
        } else {
            vec![
                SetupAction {
                    label: "Set repository variables EPLYX_API_URL and EPLYX_PROJECT_ID, and add .github/workflows/eplyx.yml",
                    actor: Actor::Repository,
                    command: Some(format!("gh variable set EPLYX_PROJECT_ID --body {p}")),
                },
                SetupAction {
                    label: "Or submit a prebuilt candidate directly",
                    actor: Actor::Repository,
                    command: Some(format!(
                        "scripts/eplyx-submit.sh --api \"$EPLYX_API_URL\" --project {p} --candidate target/deploy/program.so --summary eplyx-summary.md"
                    )),
                },
            ]
        },
    });

    let next_step = steps
        .iter()
        .find(|step| step.required && !step.status.satisfied())
        .map(|step| step.id);
    ProjectSetupResponse {
        schema_version: 1,
        project_id: p.clone(),
        kind: "program_upgrade",
        ready_for_first_check: prerequisites_done,
        first_check_complete: history.verdicts > 0,
        next_step,
        steps,
        repository: json!({
            "workflow": ".github/workflows/eplyx.yml",
            "variables": {
                "EPLYX_API_URL": state.identity.as_ref().map(|identity| identity.config.public_url.clone()),
                "EPLYX_PROJECT_ID": p,
            },
            "secrets": ["EPLYX_TOKEN"],
        }),
    }
}

pub(super) async fn get_project_setup(
    State(state): State<Shared>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let principal = authenticate(&state, &project_id, &headers).await?;
    let (project, is_token) = match principal {
        Principal::Operator => (
            state
                .registry
                .load_project(&project_id)
                .map_err(|_| ApiError::not_found("project"))?,
            false,
        ),
        Principal::Member { project } => (*project, false),
        Principal::Project { project, .. } => (*project, true),
    };
    let live_tokens = live_project_tokens(&state, &project.project_id).await;
    let reader = Arc::clone(&state);
    let id = project.project_id.clone();
    let history = tokio::task::spawn_blocking(move || upgrade_history(&reader, &id))
        .await
        .map_err(|_| ApiError::internal("reading run history stopped"))?;
    Ok((
        StatusCode::OK,
        Json(project_setup(
            &state,
            &project,
            is_token,
            live_tokens,
            &history,
        )),
    )
        .into_response())
}
