//! Narrow hosted submission references one project-owned retained transfer case.
use super::Input;
use crate::{
    api::{project_for, ApiError, ApiResult, Shared},
    artifacts::ArtifactClass,
    registry::{Registry, RunMetadata},
};
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use eplyx_engine::change::ChangeSpec;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

fn unavailable(code: &str, reason: &str) -> Value {
    json!({"schema_version":1,"eligible":false,"reason_code":code,"reason":reason})
}

/// Safe configuration facts only. Reuses admission and the existing no-op
/// preservation proof; never executes a VM, refreshes state or returns bytes.
pub fn retained_eligibility(input: &eplyx_engine::parameter_change::Input) -> Value {
    use eplyx_engine::{
        parameter_change as p,
        replay::hash_bytes,
        standard_programs::{token2022, Decoded},
    };
    if input.context.program != token2022::PROGRAM_ID {
        return unavailable(
            "wrong_token_program",
            "This retained transfer does not use Token-2022.",
        );
    }
    let plan = match input.validate() {
        Ok(plan) => plan,
        Err(e) => {
            return if e.to_string().starts_with("config_evidence_missing:") {
                unavailable(
                    "capture_incomplete",
                    "Required retained transfer evidence is incomplete or inconsistent.",
                )
            } else {
                unavailable("downstream_action_unsupported", "The retained transfer account or extension shape is outside the supported action contract.")
            }
        }
    };
    let Some(mint) = plan
        .accounts
        .iter()
        .find(|a| a.address == input.context.mint)
    else {
        return unavailable(
            "capture_incomplete",
            "The retained mint evidence is unavailable.",
        );
    };
    let Decoded::Decoded(extensions) =
        token2022::checked_extensions(&mint.account.data, token2022::Layout::Mint)
    else {
        return unavailable(
            "mutation_unsupported",
            "The retained mint layout cannot establish the isolated fee-field mutation.",
        );
    };
    let Some(token2022::Extension::TransferFeeConfig { newer, .. }) = extensions
        .iter()
        .find(|e| matches!(e, token2022::Extension::TransferFeeConfig { .. }))
    else {
        return unavailable(
            "missing_fee_config",
            "The retained mint has no TransferFeeConfig.",
        );
    };
    let expected = p::ExpectedCurrent {
        account_data_sha256: hash_bytes(&mint.account.data),
        basis_points: newer.basis_points,
        schedule_epoch: newer.epoch,
        maximum_fee_raw: newer.maximum_fee,
    };
    if let Err(failure) = p::mutate(
        &mint.account,
        &expected,
        newer.basis_points,
        plan.clock.epoch,
    ) {
        return match failure.status {
            p::Status::ScheduleNotActive => unavailable("schedule_not_active", "The newer fee schedule is pending at the retained Clock epoch. A newer state requires a new retained transfer run."),
            _ => unavailable("mutation_unsupported", "The retained mint cannot establish isolated preservation of the supported fee field."),
        };
    }
    json!({"schema_version":1,"eligible":true,"operation":p::DERIVATION,"program_id":token2022::PROGRAM_ID,"mint":input.context.mint,"current_basis_points":newer.basis_points,"schedule_epoch":newer.epoch.to_string(),"captured_epoch":plan.clock.epoch.to_string(),"maximum_fee_raw":newer.maximum_fee.to_string(),"mint_data_sha256":expected.account_data_sha256,"capture_sha256":input.source_capture_sha256.as_ref().unwrap_or(&input.fixture_sha256),"token_2022_elf_sha256":plan.programs.iter().find(|p|p.program_id.to_string()==token2022::PROGRAM_ID).map(|p|hash_bytes(&p.bytes)),"transfer":{"source":input.context.source,"destination":input.context.destination,"amount_raw":input.amount_raw.to_string(),"decimals":input.context.decimals}})
}

fn eligibility_for(registry: &Registry, parent: &RunMetadata) -> Value {
    if parent.hosted_analysis.as_ref().map(|job| job.kind.as_str()) != Some("current_path") {
        return unavailable(
            "wrong_run_kind",
            "Select a retained hosted current-state TransferChecked run.",
        );
    }
    if !parent.status.is_terminal() || !parent.report_available {
        return unavailable(
            "run_not_complete",
            "The retained transfer run must have a completed result.",
        );
    }
    let Ok(Input::CurrentPath { capture, .. }) = registry.hosted_input(parent) else {
        return unavailable(
            "capture_unavailable",
            "Authoritative retained hosted transfer evidence is unavailable.",
        );
    };
    if registry.analytical_projection(parent).is_err() {
        return unavailable(
            "capture_unavailable",
            "The retained parent result and capture bindings could not be verified.",
        );
    }
    let Ok(bytes) = registry.artifacts().get(ArtifactClass::Capture, &capture) else {
        return unavailable(
            "capture_incomplete",
            "The retained transfer capture is unavailable.",
        );
    };
    let Ok(captured) = serde_json::from_slice::<eplyx_engine::path::current::Capture>(&bytes)
    else {
        return unavailable(
            "capture_incomplete",
            "The retained capture could not be decoded.",
        );
    };
    if captured.request.path != eplyx_engine::path::ExitPathType::Transfer {
        return unavailable(
            "wrong_path",
            "This retained path is not the supported original-owner TransferChecked action.",
        );
    }
    match eplyx_engine::path::current::parameter_input(&bytes) {
        Ok(input) => retained_eligibility(&input),
        Err(e) if e.to_string().starts_with("downstream_action_unsupported:") => unavailable("downstream_action_unsupported", "The retained transfer account or extension shape is outside the supported action contract."),
        Err(_) => unavailable("capture_incomplete", "Required retained transfer evidence is incomplete or inconsistent."),
    }
}

pub async fn eligibility(
    State(state): State<Shared>,
    Path((project, parent_run)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    project_for(&state, &project, &headers).await?;
    let parent = state
        .registry
        .load_run(&parent_run)
        .map_err(|_| ApiError::not_found("retained transfer"))?;
    if parent.project_id != project {
        return Err(ApiError::not_found("retained transfer"));
    }
    let task = state.clone();
    let mut result = tokio::task::spawn_blocking(move || eligibility_for(&task.registry, &parent))
        .await
        .map_err(|_| ApiError::internal("eligibility check stopped"))?;
    result["project_id"] = project.into();
    result["run_id"] = parent_run.into();
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_key: String,
    pub change_spec: ChangeSpec,
    #[serde(default)]
    pub record_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalParent {
    pub bundle_sha256: String,
    pub record_id: String,
    pub record_sha256: String,
    pub baseline_sha256: String,
}
/// Reconstruct only from the parent upgrade run's immutable, project-owned
/// bundle. Its candidate never supplies this analysis's executable.
pub fn historical_input(
    registry: &Registry,
    parent: &RunMetadata,
    record_id: &str,
) -> Result<eplyx_engine::parameter_change::stake_pool::Input> {
    ensure!(
        parent.hosted_analysis.is_none()
            && parent.analysis.is_none()
            && parent.status.is_terminal()
            && parent.report_available,
        "parent must be a completed retained program-upgrade run"
    );
    let registered = registry.load_bundle_record(
        &parent.project_id,
        parent
            .bundle_id
            .as_deref()
            .context("parent bundle ID missing")?,
    )?;
    ensure!(
        registered.bundle_sha256 == parent.bundle_sha256,
        "parent project bundle differs"
    );
    let bundle = registry.open_bundle(&parent.bundle_sha256)?;
    ensure!(
        parent.baseline_sha256.as_ref() == Some(&bundle.manifest().baseline_program_sha256)
            && parent.corpus_sha256.as_ref() == Some(&bundle.manifest().corpus_sha256),
        "parent baseline/corpus projection mismatch"
    );
    let input = eplyx_engine::parameter_change::stake_pool::Input::from_bundle(&bundle, record_id)?;
    ensure!(
        input.record.state_source == eplyx_engine::replay::ReplayStateSource::HistoricalArchive,
        "hosted parent requires retained historical evidence"
    );
    Ok(input)
}
pub fn validate_parent(registry: &Registry, project: &str, input: &Input) -> Result<()> {
    let Input::ProtocolParameterChange {
        capture,
        parent_run,
        historical,
        change,
    } = input
    else {
        anyhow::bail!("wrong parameter input kind")
    };
    let parent = registry.load_run(parent_run)?;
    ensure!(
        parent.project_id == project && parent.status.is_terminal() && parent.report_available,
        "parameter parent unavailable or wrong project"
    );
    let spec = ChangeSpec::parse(&registry.document_bytes(change)?)?;
    let operation = &spec
        .as_protocol_parameter_change()
        .context("parameter operation absent")?
        .operation;
    if let Some(source) = historical {
        ensure!(
            matches!(
                operation,
                eplyx_engine::parameter_change::Operation::SplStakePoolSolDepositFeeV1 { .. }
            ),
            "historical parent requires Stake Pool operation"
        );
        let input = historical_input(registry, &parent, &source.record_id)?;
        ensure!(
            source.bundle_sha256 == parent.bundle_sha256
                && source.record_sha256 == input.record_sha256
                && source.baseline_sha256 == input.record.current_program_sha256,
            "pinned historical parent/record/executable mismatch"
        );
        let bytes = registry.artifacts().get(ArtifactClass::Capture, capture)?;
        ensure!(
            serde_json::from_slice::<eplyx_engine::parameter_change::stake_pool::Input>(&bytes)?
                == input,
            "historical input differs from authoritative retained bundle"
        );
        return Ok(());
    }
    ensure!(matches!(operation, eplyx_engine::parameter_change::Operation::Token2022ActiveNewerTransferFeeBasisPointsV1 { .. }), "Token-2022 current parent requires Token-2022 operation");
    let Input::CurrentPath {
        capture: retained, ..
    } = registry.hosted_input(&parent)?
    else {
        anyhow::bail!("parent must be retained current transfer");
    };
    ensure!(*capture == retained, "parent capture mismatch");
    registry.analytical_projection(&parent)?;
    Ok(())
}
pub async fn submit(
    State(state): State<Shared>,
    Path((project, parent_run)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> ApiResult<Response> {
    project_for(&state, &project, &headers).await?;
    super::observation::key(&request.request_key).map_err(super::observation::invalid)?;
    request
        .change_spec
        .validate()
        .map_err(super::observation::invalid)?;
    eplyx_engine::parameter_change::binding(&request.change_spec)
        .map_err(super::observation::invalid)?;
    let parent = state
        .registry
        .load_run(&parent_run)
        .map_err(|_| ApiError::not_found("retained transfer"))?;
    if parent.project_id != project {
        return Err(ApiError::not_found("retained transfer"));
    }
    let (capture, historical) = match &request.change_spec.as_protocol_parameter_change().context("parameter change missing").map_err(super::observation::invalid)?.operation {
        eplyx_engine::parameter_change::Operation::SplStakePoolSolDepositFeeV1 { .. } => {
            let record_id = request.record_id.as_deref().ok_or_else(|| ApiError::bad_request("explicit historical record_id required"))?;
            let input = historical_input(&state.registry, &parent, record_id).map_err(super::observation::invalid)?;
            let source = HistoricalParent { bundle_sha256: parent.bundle_sha256.clone(), record_id: record_id.into(), record_sha256: input.record_sha256.clone(), baseline_sha256: input.record.current_program_sha256.clone() };
            let bytes = eplyx_engine::canonical::document(&input).map_err(super::observation::invalid)?.into_bytes();
            (state.registry.artifacts().put(ArtifactClass::Capture, &bytes).map_err(super::observation::invalid)?.reference, Some(source))
        }
        eplyx_engine::parameter_change::Operation::Token2022ActiveNewerTransferFeeBasisPointsV1 { .. } => {
            if request.record_id.is_some() { return Err(ApiError::bad_request("record_id is only supported for a historical Stake Pool parent")); }
            let Input::CurrentPath { capture, .. } = state.registry.hosted_input(&parent).map_err(super::observation::invalid)? else { return Err(ApiError::bad_request("parent must be a retained current TransferChecked case")); };
            (capture, None)
        }
    };
    let change = state
        .registry
        .document_ref(
            request
                .change_spec
                .to_document()
                .map_err(super::observation::invalid)?
                .as_bytes(),
        )
        .map_err(super::observation::invalid)?;
    let input = Input::ProtocolParameterChange {
        change,
        capture,
        parent_run,
        historical,
    };
    validate_parent(&state.registry, &project, &input).map_err(super::observation::invalid)?;
    let task = state.clone();
    let record: RunMetadata = tokio::task::spawn_blocking(move || {
        task.registry
            .create_hosted_analysis_with_key(&project, input, Some(request.request_key))
    })
    .await
    .map_err(|_| ApiError::internal("parameter submission stopped"))?
    .map_err(super::observation::invalid)?;
    crate::worker::spawn(state, record.run_id.clone());
    super::observation::accepted(&record)
}
pub fn verify(registry: &Registry, input: &Input, report: &serde_json::Value) -> Result<()> {
    let Input::ProtocolParameterChange {
        change, capture, ..
    } = input
    else {
        anyhow::bail!("wrong parameter kind")
    };
    let parent = match input {
        Input::ProtocolParameterChange { parent_run, .. } => registry.load_run(parent_run)?,
        _ => unreachable!(),
    };
    validate_parent(registry, &parent.project_id, input)?;
    let spec = ChangeSpec::parse(&registry.document_bytes(change)?)?;
    let bytes = registry.artifacts().get(ArtifactClass::Capture, capture)?;
    let retained = match input {
        Input::ProtocolParameterChange {
            historical: Some(_),
            ..
        } => serde_json::from_slice::<serde_json::Value>(&bytes)?,
        _ => serde_json::to_value(eplyx_engine::path::current::parameter_input(&bytes)?)?,
    };
    ensure!(
        report["retained_input"] == retained,
        "parameter report capture differs from authoritative retained source"
    );
    eplyx_engine::parameter_change::verify(&spec, report).context("parameter projection mismatch")
}
