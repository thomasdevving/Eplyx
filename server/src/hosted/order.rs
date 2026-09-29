//! Hosted occurrences of the existing bounded, offline migration order engine.
//! The only public input is two sources. All analytical inputs come from one
//! immutable completed parent; portable evidence lives in the existing CAS.
use super::Input;
use crate::{
    api::{project_for, ApiError, ApiResult, Shared},
    artifacts::{ArtifactClass, ArtifactRef},
    projection::Projection,
    registry::{Registry, RunMetadata, RunStatus},
};
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json,
};
use eplyx_engine::{
    canonical,
    change::ChangeSpec,
    cloud::contract::Artifact,
    migration::{input, order, order_store, pipeline, world::World},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Component, Path as FsPath},
    sync::Arc,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub source_a: String,
    pub source_b: String,
}

pub struct Parent {
    pub change: ChangeSpec,
    pub world: World,
    pub candidate: Vec<u8>,
    pub units: Vec<eplyx_engine::migration::planner::MigrationUnit>,
    pub input: ArtifactRef,
    pub projection: ArtifactRef,
}

pub fn parent(registry: &Registry, record: &RunMetadata) -> Result<Parent> {
    ensure!(
        matches!(
            record.status,
            RunStatus::Passed | RunStatus::Failed | RunStatus::Completed
        ) && record.report_available,
        "parent has no completed retained result"
    );
    let job = record
        .hosted_analysis
        .as_ref()
        .context("parent has no retained hosted package")?;
    ensure!(
        job.kind == "token_migration",
        "parent is not a token migration"
    );
    let input = registry.hosted_input(record)?;
    let Input::TokenMigration { candidate, .. } = &input else {
        anyhow::bail!("wrong parent kind")
    };
    let projection = registry.analytical_projection(record)?;
    let work = tempfile::tempdir()?;
    super::worker::stage(registry, &input, work.path())?;
    let package = input::load(work.path())?;
    let bindings: pipeline::Bindings = serde_json::from_str(
        &projection
            .bindings
            .as_ref()
            .context("missing parent bindings")?
            .text,
    )?;
    ensure!(
        bindings.schema_version == pipeline::BINDINGS_SCHEMA
            && bindings.kind == pipeline::BINDINGS_KIND
            && bindings.change_spec_id == package.change_spec_id()
            && bindings.analysis_input_sha256 == package.analysis_input_sha256()
            && bindings.state_input_sha256 == package.state_input_sha256()
            && bindings.candidate_program_sha256 == package.program_sha256(),
        "parent package bindings disagree"
    );
    let world: World = match &projection.migration_world {
        Some(reference) => serde_json::from_slice(
            &registry
                .artifacts()
                .get(ArtifactClass::Capture, reference)?,
        )?,
        None if matches!(
            package.state().config.state,
            input::StateSource::CapturedWorld { .. }
        ) =>
        {
            pipeline::world_for(&package, work.path(), &bindings)?
        }
        None => anyhow::bail!("parent lacks retained world bytes"),
    };
    world.validate()?;
    // The report pins the planner's world independently of index metadata.
    let report: Value = serde_json::from_str(&projection.report.text)?;
    ensure!(
        report["coverage"]["world"]["world_sha256"] == world.sha256()?,
        "parent world differs from retained report"
    );
    let candidate_bytes = registry
        .artifacts()
        .get(ArtifactClass::Program, candidate)?;
    ensure!(
        record.candidate_artifact.as_ref() == Some(candidate)
            && record.candidate_sha256 == candidate.sha256,
        "parent candidate index mismatch"
    );
    let runtime = order::runtime_identity()?;
    match &projection.migration_runtime_id {
        Some(retained) => ensure!(*retained == runtime, "retained runtime differs"),
        None => {
            // Older retained jobs can prove their runtime when produced by this
            // exact binary. Otherwise availability must fail closed.
            let metadata: eplyx_engine::local_store::Metadata =
                serde_json::from_str(&projection.metadata.text)?;
            ensure!(
                metadata.engine_binary_sha256
                    == eplyx_engine::replay::hash_bytes(&std::fs::read(std::env::current_exe()?)?),
                "legacy parent runtime cannot be established"
            );
        }
    }
    ensure!(
        report["execution"]["vm"]["clock"]["clock"] == serde_json::to_value(world.clock)?,
        "parent Clock differs from order starting Clock"
    );
    let programs = eplyx_engine::migration::execute::programs(
        &world,
        package.spec(),
        package.program_id(),
        package.candidate(),
    )?;
    let retained_programs = report["execution"]["vm"]["programs"]
        .as_array()
        .context("missing retained dependency identities")?;
    ensure!(
        programs.len() == retained_programs.len()
            && programs.iter().all(|program| retained_programs
                .iter()
                .any(|p| p["program_id"] == program.program_id.to_string()
                    && p["loader"] == program.loader.to_string()
                    && p["elf_sha256"] == eplyx_engine::replay::hash_bytes(&program.bytes))),
        "parent dependency identities differ"
    );
    let units = order::eligible_units(package.change(), &world, &candidate_bytes)?;
    ensure!(
        units.len() >= 2,
        "fewer than two eligible solo-control units"
    );
    Ok(Parent {
        change: package.change().clone(),
        world,
        candidate: candidate_bytes,
        units,
        input: job.input.clone(),
        projection: job
            .projection
            .clone()
            .context("missing parent projection")?,
    })
}

pub fn eligibility(registry: &Registry, record: &RunMetadata) -> Value {
    match parent(registry, record) {
        Ok(p) => {
            json!({"eligible":true,"parent_run_id":record.run_id,"change_spec_id":p.change.id().ok(),
            "candidate":p.change.candidate(),"world_id":p.world.sha256().ok(),"clock":p.world.clock,
            "runtime_id":order::runtime_identity().ok(),"units":p.units,
            "retained_state":true,"state_refresh":false})
        }
        Err(e) => {
            let (code, reason) = if record
                .hosted_analysis
                .as_ref()
                .is_none_or(|j| j.kind != "token_migration")
            {
                ("wrong_analysis_kind", "Order analysis requires a completed hosted token-migration run with a retained package.")
            } else if !matches!(
                record.status,
                RunStatus::Passed | RunStatus::Failed | RunStatus::Completed
            ) || !record.report_available
            {
                (
                    "parent_not_completed",
                    "The parent migration has no completed retained result.",
                )
            } else if e
                .downcast_ref::<order::OrderError>()
                .is_some_and(|e| e.kind == order::FailureKind::UnsupportedComposition)
            {
                ("unsupported_parent", "Order-case v1 requires shared reserve transfer and an open migration window at the retained Clock.")
            } else {
                ("retained_evidence_unavailable", "The retained package, world, candidate, runtime or two eligible solo controls could not be verified.")
            };
            json!({"eligible":false,"parent_run_id":record.run_id,"reason_code":code,"reason":reason,"units":[]})
        }
    }
}

/// Persist only derived immutable references; no caller can replace the world.
pub fn create(registry: &Registry, record: &RunMetadata, request: &Request) -> Result<RunMetadata> {
    ensure!(
        request.source_a != request.source_b,
        "select two distinct sources"
    );
    let p = parent(registry, record)?;
    ensure!(
        [&request.source_a, &request.source_b]
            .iter()
            .all(|s| p.units.iter().any(|u| &u.source_account == *s)),
        "source is not an eligible retained unit"
    );
    let change = registry.document_ref(canonical::document(&p.change)?.as_bytes())?;
    let world = registry
        .artifacts()
        .put(
            ArtifactClass::Capture,
            canonical::document(&p.world)?.as_bytes(),
        )?
        .reference;
    let candidate = registry.artifacts().put_program(&p.candidate)?.reference;
    registry.create_hosted_analysis(
        &record.project_id,
        Input::MigrationOrder {
            change,
            world,
            candidate,
            parent_run: record.run_id.clone(),
            parent_input: p.input,
            parent_projection: p.projection,
            source_a: request.source_a.clone(),
            source_b: request.source_b.clone(),
        },
    )
}

pub(super) fn stage(registry: &Registry, input: &Input, directory: &FsPath) -> Result<()> {
    let Input::MigrationOrder {
        world,
        candidate,
        parent_run,
        parent_input,
        parent_projection,
        source_a,
        source_b,
        change,
    } = input
    else {
        anyhow::bail!("order input required")
    };
    let record = registry.load_run(parent_run)?;
    let p = parent(registry, &record)?;
    ensure!(
        p.input == *parent_input && p.projection == *parent_projection,
        "parent reference mismatch"
    );
    let world_bytes = registry.artifacts().get(ArtifactClass::Capture, world)?;
    let retained_world: World = serde_json::from_slice(&world_bytes)?;
    ensure!(
        retained_world == p.world
            && registry.document_bytes(change)? == canonical::document(&p.change)?.as_bytes()
            && registry
                .artifacts()
                .get(ArtifactClass::Program, candidate)?
                == p.candidate,
        "order input differs from parent"
    );
    ensure!(
        source_a != source_b
            && [source_a, source_b]
                .iter()
                .all(|s| p.units.iter().any(|u| &u.source_account == *s)),
        "invalid retained sources"
    );
    std::fs::write(directory.join("world.json"), world_bytes)?;
    std::fs::write(directory.join("candidate.so"), &p.candidate)?;
    Ok(())
}

fn analytical_failure(a: &order::Analysis) -> Option<Value> {
    // A measured candidate reconciliation mismatch is a valid NotEstablished
    // analytical result. An evidence or handoff failure is a failed job.
    a.scenarios
        .iter()
        .filter_map(|s| s.stopped)
        .find(|kind| *kind != order::FailureKind::ReconciliationMismatch)
        .map(failure)
}

fn failure(kind: order::FailureKind) -> Value {
    let detail = match kind {
        order::FailureKind::UnsupportedComposition => {
            "The selected pair is outside the supported bounded composition."
        }
        order::FailureKind::EvidenceGap => {
            "The retained evidence was insufficient to establish this analysis."
        }
        order::FailureKind::HandoffFailure => {
            "The retained state could not be handed off exactly between transactions."
        }
        order::FailureKind::UnexpectedWrite => {
            "Execution wrote outside the bounded account closure."
        }
        order::FailureKind::ReconciliationMismatch => {
            "Execution did not reconcile with the retained evidence."
        }
    };
    json!({"kind":kind,"detail":detail})
}

/// Presentation strips account byte content only. Every classification, unit,
/// scenario, state identity and failure signature is the engine's own result.
pub fn presentation(a: &order::Analysis) -> Result<Value> {
    let mut value = serde_json::to_value(a)?;
    value["binding_id"] = json!(a.binding.id()?);
    value["states"] = json!(a.states.iter().map(|(id,s)| (id.clone(),json!({
        "binding_id":s.binding_id,"accounts":s.accounts.iter().map(|(address,account)|
            (address.clone(),match account { order::AccountEvidence::KnownAbsent => json!({"kind":"KnownAbsent"}),
                order::AccountEvidence::Present { account: snapshot } => json!({"kind":"Present","owner":snapshot.owner,"data_len":snapshot.data.len()}) })
        ).collect::<BTreeMap<_,_>>()
    }))).collect::<BTreeMap<_,_>>());
    Ok(value)
}

pub(super) fn execute(input: &Input, directory: &FsPath) -> Result<Value> {
    let Input::MigrationOrder {
        change,
        world,
        candidate,
        parent_run,
        source_a,
        source_b,
        ..
    } = input
    else {
        anyhow::bail!("order input required")
    };
    let spec = ChangeSpec::parse(&super::worker::member(directory, "change.json", change)?)?;
    let world: World =
        serde_json::from_slice(&super::worker::member(directory, "world.json", world)?)?;
    let candidate = super::worker::member(directory, "candidate.so", candidate)?;
    let result = order_store::save(
        &directory.canonicalize()?.join("order-case"),
        &spec,
        &world,
        &candidate,
        [source_a, source_b],
    );
    match result {
        Ok(a) => Ok(
            json!({"kind":"migration_order","parent_run_id":parent_run,"source_a":source_a,"source_b":source_b,"analysis":presentation(&a)?,"failure":analytical_failure(&a)}),
        ),
        Err(e) => match e.downcast_ref::<order::OrderError>() {
            Some(e) => Ok(
                json!({"kind":"migration_order","parent_run_id":parent_run,"source_a":source_a,"source_b":source_b,"analysis":null,"failure":failure(e.kind)}),
            ),
            None => Err(e),
        },
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Portable {
    parent_run: String,
    parent_input: ArtifactRef,
    parent_projection: ArtifactRef,
    files: BTreeMap<String, ArtifactRef>,
}
fn safe_member(name: &str) -> bool {
    let p = FsPath::new(name);
    !p.is_absolute()
        && !name.contains('\\')
        && p.components().all(|c| matches!(c, Component::Normal(_)))
        && (matches!(name, "case.json" | "report.md") || name.starts_with("evidence/"))
}
fn class(name: &str) -> ArtifactClass {
    if name.starts_with("evidence/programs/") {
        ArtifactClass::Program
    } else {
        ArtifactClass::Capture
    }
}

/// Ingest the exact portable directory before terminal metadata is written.
pub(super) fn retain(
    registry: &Registry,
    input: &Input,
    directory: &FsPath,
    projection: &mut Projection,
) -> Result<()> {
    let Input::MigrationOrder {
        parent_run,
        parent_input,
        parent_projection,
        ..
    } = input
    else {
        return Ok(());
    };
    let report: Value = serde_json::from_str(&projection.report.text)?;
    let mut files = BTreeMap::new();
    if !report["analysis"].is_null() {
        order_store::verify(&directory.canonicalize()?.join("order-case"))?;
        fn visit(
            registry: &Registry,
            root: &FsPath,
            path: &FsPath,
            files: &mut BTreeMap<String, ArtifactRef>,
        ) -> Result<()> {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let meta = entry.file_type()?;
                ensure!(!meta.is_symlink(), "symlinked worker artifact");
                if meta.is_dir() {
                    visit(registry, root, &entry.path(), files)?;
                } else {
                    ensure!(meta.is_file(), "invalid portable member");
                    let name = entry
                        .path()
                        .strip_prefix(root)?
                        .to_str()
                        .context("invalid artifact member")?
                        .to_owned();
                    ensure!(safe_member(&name), "invalid portable member");
                    let bytes = std::fs::read(entry.path())?;
                    files.insert(
                        name.clone(),
                        registry.artifacts().put(class(&name), &bytes)?.reference,
                    );
                }
            }
            Ok(())
        }
        visit(
            registry,
            &directory.join("order-case"),
            &directory.join("order-case"),
            &mut files,
        )?;
    }
    projection.bindings = Some(Artifact::new(serde_json::to_vec(&Portable {
        parent_run: parent_run.clone(),
        parent_input: parent_input.clone(),
        parent_projection: parent_projection.clone(),
        files,
    })?)?);
    Ok(())
}

/// All members are hash-on-read CAS objects. No repair or VM execution on read.
pub fn verify(registry: &Registry, record: &RunMetadata, projection: &Projection) -> Result<()> {
    let input = registry.hosted_input(record)?;
    let Input::MigrationOrder {
        change,
        world,
        candidate,
        parent_run,
        parent_input,
        parent_projection,
        source_a,
        source_b,
    } = &input
    else {
        anyhow::bail!("order input required")
    };
    let parent_record = registry.load_run(parent_run)?;
    ensure!(
        parent_record.project_id == record.project_id,
        "cross-project parent binding"
    );
    let job = parent_record
        .hosted_analysis
        .as_ref()
        .context("missing parent job")?;
    ensure!(
        job.input == *parent_input && job.projection.as_ref() == Some(parent_projection),
        "parent binding mismatch"
    );
    let p = parent(registry, &parent_record)?;
    let spec = ChangeSpec::parse(&registry.document_bytes(change)?)?;
    let w: World =
        serde_json::from_slice(&registry.artifacts().get(ArtifactClass::Capture, world)?)?;
    let c = registry
        .artifacts()
        .get(ArtifactClass::Program, candidate)?;
    ensure!(
        spec == p.change && w == p.world && c == p.candidate,
        "input does not match parent"
    );
    let portable: Portable = serde_json::from_str(
        &projection
            .bindings
            .as_ref()
            .context("missing portable artifact binding")?
            .text,
    )?;
    ensure!(
        portable.parent_run == *parent_run
            && portable.parent_input == *parent_input
            && portable.parent_projection == *parent_projection,
        "artifact parent binding mismatch"
    );
    let report: Value = serde_json::from_str(&projection.report.text)?;
    ensure!(
        report["parent_run_id"] == *parent_run
            && report["source_a"] == *source_a
            && report["source_b"] == *source_b,
        "result parent/selection mismatch"
    );
    if !report["analysis"].is_null() {
        let temp = tempfile::tempdir()?;
        materialize(registry, &portable, temp.path())?;
        let a = order_store::verify(&temp.path().canonicalize()?)?;
        ensure!(
            a.binding.change_spec_id == spec.id()?
                && a.binding.world_id == w.sha256()?
                && a.binding.world_content_sha256 == order::world_content_id(&w)?
                && a.binding.candidate == *spec.candidate().context("missing candidate")?
                && a.binding.units[0].source_account == *source_a
                && a.binding.units[1].source_account == *source_b
                && presentation(&a)? == report["analysis"]
                && serde_json::to_value(analytical_failure(&a))? == report["failure"],
            "artifact/result identity mismatch"
        );
    } else {
        ensure!(
            portable.files.is_empty() && !report["failure"].is_null(),
            "invalid failed artifact"
        );
        let kind: order::FailureKind = serde_json::from_value(report["failure"]["kind"].clone())?;
        ensure!(report["failure"] == failure(kind), "invalid failure result");
    }
    Ok(())
}
fn materialize(registry: &Registry, portable: &Portable, directory: &FsPath) -> Result<()> {
    ensure!(
        portable.files.len() <= 10000,
        "portable artifact exceeds member bound"
    );
    for (name, reference) in &portable.files {
        ensure!(safe_member(name), "unsafe portable artifact member");
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().context("artifact parent")?)?;
        std::fs::write(path, registry.artifacts().get(class(name), reference)?)?;
    }
    Ok(())
}

pub fn result(registry: &Registry, record: &RunMetadata) -> Result<Value> {
    let Input::MigrationOrder {
        parent_run,
        source_a,
        source_b,
        ..
    } = registry.hosted_input(record)?
    else {
        anyhow::bail!("order run required")
    };
    let parent_record = registry.load_run(&parent_run)?;
    ensure!(
        parent_record.project_id == record.project_id
            && parent_record
                .hosted_analysis
                .as_ref()
                .is_some_and(|j| j.kind == "token_migration"),
        "order parent index mismatch"
    );
    let mut value = json!({"run_id":record.run_id,"project_id":record.project_id,"status":record.status,
        "parent_run_id":parent_run,"source_a":source_a,"source_b":source_b,"analysis":null,"failure":null});
    if record.report_available {
        let projection = registry.analytical_projection(record)?;
        let report: Value = serde_json::from_str(&projection.report.text)?;
        value["analysis"] = report["analysis"].clone();
        value["failure"] = report["failure"].clone();
        value["artifact_available"] = json!(!report["analysis"].is_null());
    } else if let Some(kind) = record.order_failure {
        value["failure"] = failure(kind);
    } else if record.status == RunStatus::ExecutionError {
        value["failure"] = json!({"kind":"InternalExecutionFailure","detail":"The worker could not complete; no analytical conclusion is available."});
    }
    Ok(value)
}

/// A minimal parent-child index uses the existing project occurrence index and
/// immutable input manifest, rather than a second generic relationship store.
pub fn children(registry: &Registry, project: &str, parent: &str) -> Result<Vec<Value>> {
    let mut results = Vec::new();
    for id in registry.project_run_ids(project)? {
        let record = registry.load_run(&id)?;
        if record
            .hosted_analysis
            .as_ref()
            .is_some_and(|j| j.kind == "migration_order")
        {
            if let Input::MigrationOrder { parent_run, .. } = registry.hosted_input(&record)? {
                ensure!(record.project_id == project, "order index mismatch");
                if parent_run == parent {
                    let mut summary = result(registry, &record)?;
                    if !summary["analysis"].is_null() {
                        summary["analysis"] = json!({"binding_id":summary["analysis"]["binding_id"],"comparison":summary["analysis"]["comparison"]});
                    }
                    results.push(summary);
                }
            }
        }
    }
    Ok(results)
}
async fn authorized(
    state: &Shared,
    headers: &HeaderMap,
    project: &str,
    id: &str,
) -> ApiResult<RunMetadata> {
    project_for(state, project, headers).await?;
    let record = state
        .registry
        .load_run(id)
        .map_err(|_| ApiError::not_found("run"))?;
    if record.project_id != project {
        return Err(ApiError::not_found("run"));
    }
    Ok(record)
}
async fn available(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let record = authorized(&state, &headers, &project, &id).await?;
    let state_for_job = Arc::clone(&state);
    Ok(Json(
        tokio::task::spawn_blocking(move || eligibility(&state_for_job.registry, &record))
            .await
            .map_err(|_| {
                ApiError::internal(
                    "Order evidence could not be verified; no analytical conclusion is available.",
                )
            })?,
    ))
}
async fn submit(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
    Json(request): Json<Request>,
) -> ApiResult<Response> {
    let record = authorized(&state, &headers, &project, &id).await?;
    let state_for_job = Arc::clone(&state);
    let record = tokio::task::spawn_blocking(move || create(&state_for_job.registry,&record,&request)).await.map_err(|_|ApiError::internal("Order evidence could not be verified; no analytical conclusion is available."))?
        .map_err(|_|ApiError::bad_request("Order analysis requires an eligible retained parent and two distinct eligible sources."))?;
    crate::worker::spawn(Arc::clone(&state), record.run_id.clone());
    Ok((
        StatusCode::ACCEPTED,
        Json(
            json!({"run_id":record.run_id,"status":record.status,"parent_run_id":id,
        "result_url":format!("/v1/projects/{project}/migration-orders/{}",record.run_id)}),
        ),
    )
        .into_response())
}
async fn list(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    authorized(&state, &headers, &project, &id).await?;
    let state_for_job = Arc::clone(&state);
    Ok(Json(
        json!({"orders":tokio::task::spawn_blocking(move || children(&state_for_job.registry,&project,&id)).await.map_err(|_|ApiError::internal("Order evidence could not be verified; no analytical conclusion is available."))?.map_err(|_|ApiError::internal("Order evidence could not be verified; no analytical conclusion is available."))?}),
    ))
}
async fn read(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let record = authorized(&state, &headers, &project, &id).await?;
    let state_for_job = Arc::clone(&state);
    Ok(Json(
        tokio::task::spawn_blocking(move || result(&state_for_job.registry, &record))
            .await
            .map_err(|_| {
                ApiError::internal(
                    "Order evidence could not be verified; no analytical conclusion is available.",
                )
            })?
            .map_err(|_| ApiError::internal("order evidence verification failed"))?,
    ))
}
async fn download(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path((project, id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let record = authorized(&state, &headers, &project, &id).await?;
    if !record
        .hosted_analysis
        .as_ref()
        .is_some_and(|j| j.kind == "migration_order")
    {
        return Err(ApiError::not_found("order analysis"));
    }
    if !record.report_available {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "No portable artifact is available for this order occurrence.",
        ));
    }
    let state_for_job = Arc::clone(&state);
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let registry = &state_for_job.registry;
        let projection = registry.analytical_projection(&record)?;
        let portable: Portable = serde_json::from_str(
            &projection
                .bindings
                .context("missing portable artifact")?
                .text,
        )?;
        ensure!(!portable.files.is_empty(), "no portable artifact");
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("order-case");
        materialize(registry, &portable, &root)?;
        let archive = std::process::Command::new("/usr/bin/tar")
            .args(["-cf", "-", "-C"])
            .arg(directory.path())
            .arg("order-case")
            .env_clear()
            .output()?;
        ensure!(archive.status.success(), "artifact packaging failed");
        Ok(archive.stdout)
    })
    .await
    .map_err(|_| {
        ApiError::internal(
            "Order evidence could not be verified; no analytical conclusion is available.",
        )
    })?
    .map_err(|_| {
        ApiError::internal(
            "Order evidence could not be verified; no analytical conclusion is available.",
        )
    })?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-tar"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=order-case.tar",
            ),
        ],
        bytes,
    )
        .into_response())
}
pub fn router() -> axum::Router<Shared> {
    axum::Router::new()
        .route(
            "/v1/projects/{project}/runs/{id}/migration-order/eligibility",
            get(available),
        )
        .route(
            "/v1/projects/{project}/runs/{id}/migration-order",
            post(submit).get(list),
        )
        .route("/v1/projects/{project}/migration-orders/{id}", get(read))
        .route(
            "/v1/projects/{project}/migration-orders/{id}/artifact",
            get(download),
        )
}

pub(super) fn evidence_error(error: anyhow::Error) -> anyhow::Error {
    if error.downcast_ref::<order::OrderError>().is_some() {
        error
    } else {
        order::OrderError {
            kind: order::FailureKind::EvidenceGap,
            detail: "Retained parent or order evidence could not be verified.".into(),
        }
        .into()
    }
}

pub(crate) fn failure_exit_code(kind: order::FailureKind) -> u8 {
    match kind {
        order::FailureKind::UnsupportedComposition => 4,
        _ => 2,
    }
}
