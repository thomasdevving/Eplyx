//! Bounded observed stress uses MAIN's frozen selection and final rebinding.
//! No VM executes during acquisition. Every frozen byte enters the same CAS.
use super::{
    observation::{accepted, invalid, key, unavailable, ReadOnly},
    Input,
};
use crate::{
    api::{project_for, ApiError, ApiResult, Shared},
    artifacts::{ArtifactClass, ArtifactRef},
    registry::{Registry, RunMetadata},
};
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    Json,
};
use eplyx_engine::{
    canonical,
    change::{Change, ChangeSpec},
    migration::{
        adapter, capture, current, input, planner::RehearsalClockPolicy, population,
        population_types::StressBudget, spec::AmountPolicy,
    },
};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path as FsPath, sync::Arc};
const MEMBERS: [&str; 6] = [
    "migration.capture.json",
    "current.plan.json",
    "population.capture.json",
    "discovery.world.json",
    "current.capture.json",
    "current.bindings.json",
];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_key: String,
    pub candidate_run_id: String,
}
fn budget() -> StressBudget {
    StressBudget {
        max_response_bytes: 32 * 1024 * 1024,
        max_decoded_accounts: 1000,
        max_authority_lookups: 1000,
        max_selected_cases: 10,
        population_timeout_seconds: 120,
        max_artifact_bytes: 192 * 1024 * 1024,
        ..StressBudget::default()
    }
}
pub(super) fn stage(
    registry: &Registry,
    directory: &FsPath,
    state: &ArtifactRef,
    candidate: &ArtifactRef,
    files: &BTreeMap<String, ArtifactRef>,
    budget: &StressBudget,
) -> Result<()> {
    budget.validate()?;
    ensure!(
        files.len() == MEMBERS.len() && MEMBERS.iter().all(|name| files.contains_key(*name)),
        "invalid frozen stress members"
    );
    std::fs::create_dir_all(directory.join("programs"))?;
    std::fs::create_dir_all(directory.join("stress"))?;
    std::fs::write(
        directory.join("state.json"),
        registry.document_bytes(state)?,
    )?;
    std::fs::write(
        directory.join("programs").join(&candidate.sha256),
        registry
            .artifacts()
            .get(ArtifactClass::Program, candidate)?,
    )?;
    for (name, reference) in files {
        std::fs::write(
            directory.join("stress").join(name),
            registry
                .artifacts()
                .get(ArtifactClass::Capture, reference)?,
        )?;
    }
    std::fs::copy(
        directory.join("stress/discovery.world.json"),
        directory.join("world.json"),
    )?;
    let validated = input::load(directory)?;
    ensure!(
        validated.spec().eligibility.amount_policy == AmountPolicy::FullBalance,
        "stress requires exact final full-balance rebinding"
    );
    let population = std::fs::read(directory.join("stress/population.capture.json"))?;
    let world = serde_json::from_slice(&std::fs::read(directory.join("world.json"))?)?;
    let initial = std::fs::read(directory.join("stress/migration.capture.json"))?;
    let rebuilt_world = capture::world(validated.spec(), &population, &initial, budget)?;
    ensure!(
        canonical::digest(&rebuilt_world)? == canonical::digest(&world)?,
        "discovery world differs from raw observations"
    );
    let frozen: current::FrozenPlan =
        serde_json::from_slice(&std::fs::read(directory.join("stress/current.plan.json"))?)?;
    let rebuilt = current::freeze(
        &validated,
        &population,
        budget,
        &world,
        &frozen.selection.frozen_at,
    )?;
    ensure!(rebuilt == frozen, "stress selection changed");
    Ok(())
}
pub async fn create(
    State(state): State<Shared>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> ApiResult<Response> {
    project_for(&state, &project, &headers).await?;
    key(&request.request_key).map_err(invalid)?;
    let observation = state
        .registry
        .load_observation(&project, &id)
        .map_err(|_| ApiError::not_found("observation"))?;
    let candidate_record = state
        .registry
        .load_run(&request.candidate_run_id)
        .map_err(|_| ApiError::not_found("candidate check"))?;
    if candidate_record.project_id != project {
        return Err(ApiError::not_found("candidate check"));
    }
    if !candidate_record.status.is_terminal() || !candidate_record.report_available {
        return Err(ApiError::bad_request(
            "a completed candidate check is required",
        ));
    }
    let candidate_input = state
        .registry
        .hosted_input(&candidate_record)
        .map_err(invalid)?;
    let Input::CurrentCandidate {
        change: Some(change),
        candidate,
        parent_observation,
        wallet_sha256,
        ..
    } = candidate_input
    else {
        return Err(ApiError::bad_request(
            "a saved candidate proposal is required",
        ));
    };
    if parent_observation != id || wallet_sha256 != observation.capture.sha256 {
        return Err(ApiError::bad_request(
            "candidate belongs to another observation",
        ));
    }
    let service = state.observation.as_ref().ok_or_else(unavailable)?;
    let _permit = service.permits.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "observation acquisition is busy",
        )
    })?;
    let task = Arc::clone(&state);
    let record = tokio::task::spawn_blocking(move || -> Result<RunMetadata> {
        let _permit = _permit;
        if let Some(record) = task
            .registry
            .hosted_request(&project, &request.request_key)?
        {
            let Input::CurrentStress {
                parent_observation,
                candidate_run_id,
                ..
            } = task.registry.hosted_input(&record)?
            else {
                anyhow::bail!("request key conflict")
            };
            ensure!(
                parent_observation == id && candidate_run_id == request.candidate_run_id,
                "request key conflict"
            );
            return Ok(record);
        }
        let mut change = ChangeSpec::parse(&task.registry.document_bytes(&change)?)?;
        let Change::TokenMigration(ref mut terms) = change.change else {
            anyhow::bail!("migration proposal required")
        };
        terms.eligibility.amount_policy = AmountPolicy::FullBalance;
        change.change_spec_id = None;
        change.validate()?;
        let spec = change
            .as_token_migration()
            .context("migration")?
            .evaluation_spec(change.activation.as_ref())?;
        let program = task
            .registry
            .artifacts()
            .get(ArtifactClass::Program, &candidate)?;
        let stress_id = format!(
            "stress_{}",
            &eplyx_engine::replay::hash_bytes(request.request_key.as_bytes())[..32]
        );
        let bounds = budget();
        let rpc = ReadOnly::stress(
            task.observation
                .as_ref()
                .context("observation service")?
                .provider
                .as_ref(),
        );
        let population = population::capture(
            spec.source.mint.clone(),
            id.clone(),
            stress_id.clone(),
            bounds.clone(),
            &rpc,
        )?;
        let population_bytes = canonical::document(&population)?.into_bytes();
        let population = population::evaluate_bytes(&population_bytes, &bounds)?;
        let overlay = adapter::derive(&spec, &change.id()?, adapter::REFERENCE_PROGRAM_ID)?;
        let initial = capture::capture(&spec, &overlay, &population, 1000, &id, &rpc)?;
        let initial_bytes = canonical::document(&initial)?.into_bytes();
        let world = capture::world(&spec, &population_bytes, &initial_bytes, &bounds)?;
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        std::fs::create_dir(root.join("programs"))?;
        std::fs::write(root.join("programs").join(&candidate.sha256), program)?;
        std::fs::write(root.join("change.json"), canonical::document(&change)?)?;
        let world_bytes = canonical::document(&world)?.into_bytes();
        std::fs::write(root.join("world.json"), &world_bytes)?;
        let descriptor = input::StateInput {
            schema_version: 1,
            config: input::Config {
                state: input::StateSource::CapturedWorld {
                    artifact: "world.json".into(),
                    sha256: eplyx_engine::replay::hash_bytes(&world_bytes),
                },
                rehearsal_clock: RehearsalClockPolicy::Captured,
                max_rehearsal_units: 10,
                max_captured_holders: 1000,
            },
            invariant_schema_version: eplyx_engine::migration::invariants::INVARIANT_SCHEMA_VERSION,
            invariants: vec![],
        };
        let state_bytes = canonical::document(&descriptor)?.into_bytes();
        std::fs::write(root.join("state.json"), &state_bytes)?;
        let validated = input::load(root)?;
        current::capture_with(
            &validated,
            &population_bytes,
            &bounds,
            &world,
            &root.join("stress"),
            &rpc,
        )?;
        std::fs::write(root.join("stress/migration.capture.json"), &initial_bytes)?;
        let mut files = BTreeMap::new();
        for name in MEMBERS {
            let bytes = std::fs::read(root.join("stress").join(name))?;
            files.insert(
                name.into(),
                task.registry
                    .artifacts()
                    .put(ArtifactClass::Capture, &bytes)?
                    .reference,
            );
        }
        let change = task
            .registry
            .document_ref(canonical::document(&change)?.as_bytes())?;
        let state_input = task.registry.document_ref(&state_bytes)?;
        task.registry.create_hosted_analysis_with_key(
            &project,
            Input::CurrentStress {
                change,
                state_input,
                candidate,
                files,
                budget: bounds,
                parent_observation: id,
                wallet_sha256: observation.capture.sha256,
                stress_id,
                candidate_run_id: request.candidate_run_id,
            },
            Some(request.request_key),
        )
    })
    .await
    .map_err(|_| ApiError::internal("stress acquisition stopped"))?
    .map_err(invalid)?;
    crate::worker::spawn(Arc::clone(&state), record.run_id.clone());
    accepted(&record)
}
