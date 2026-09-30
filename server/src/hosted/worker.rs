//! The child entry point calls the engine as a library. No identity, service
//! configuration or RPC client is loaded in this process.
use super::Input;
use crate::{
    artifacts::{ArtifactClass, ArtifactRef},
    projection::Projection,
    registry::Registry,
};
use anyhow::{ensure, Context, Result};
use eplyx_engine::{
    change::ChangeSpec,
    cloud::contract::Artifact,
    local_store,
    migration::{input, pipeline},
    replay::hash_bytes,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub run_id: String,
    pub input: Input,
}

/// Rebuild a disposable workspace exclusively from verified CAS objects.
pub fn stage(registry: &Registry, input: &Input, directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let write = |name: &str, class: ArtifactClass, reference: &ArtifactRef| -> Result<()> {
        std::fs::write(
            directory.join(name),
            registry.artifacts().get(class, reference)?,
        )?;
        Ok(())
    };
    if let Some(change) = input.change() {
        write("change.json", ArtifactClass::Document, change)?;
    }
    match input {
        Input::ProtocolParameterChange { capture, .. } => {
            write("capture.json", ArtifactClass::Capture, capture)?;
            let spec = ChangeSpec::parse(&std::fs::read(directory.join("change.json"))?)?;
            eplyx_engine::parameter_change::binding(&spec)?;
            eplyx_engine::path::current::parameter_input(
                &registry.artifacts().get(ArtifactClass::Capture, capture)?,
            )?;
        }
        Input::MigrationOrder { .. } => super::order::stage(registry, input, directory)?,
        Input::CurrentStress {
            state_input,
            candidate,
            files,
            budget,
            ..
        } => super::stress::stage(registry, directory, state_input, candidate, files, budget)?,
        Input::CurrentCandidate {
            capture,
            candidate,
            change,
            parent_observation,
            wallet_sha256,
            check_id,
        } => {
            write("capture.json", ArtifactClass::Capture, capture)?;
            write("candidate.so", ArtifactClass::Program, candidate)?;
            let frozen: eplyx_engine::migration::account::Capture = serde_json::from_slice(
                &registry.artifacts().get(ArtifactClass::Capture, capture)?,
            )?;
            ensure!(
                frozen.run_id == *parent_observation
                    && frozen.check_id == *check_id
                    && frozen.wallet_sha256 == *wallet_sha256
                    && hash_bytes(frozen.wallet_capture.as_bytes()) == *wallet_sha256,
                "candidate capture binding mismatch"
            );
            let declared = eplyx_engine::migration::account::declared_change(
                &frozen,
                &registry
                    .artifacts()
                    .get(ArtifactClass::Program, candidate)?,
            )?
            .map(|s| eplyx_engine::canonical::document(&s))
            .transpose()?;
            let expected = change
                .as_ref()
                .map(|r| registry.document_bytes(r))
                .transpose()?;
            ensure!(
                declared.as_ref().map(|s| s.as_bytes()) == expected.as_deref(),
                "candidate change binding mismatch"
            );
        }
        Input::CurrentPreflight {
            bundle,
            candidate,
            parent_observation,
            wallet_sha256,
            preflight_id,
            scenario_sha256,
        } => {
            write("preflight.json", ArtifactClass::Capture, bundle)?;
            if let Some(candidate) = candidate {
                write("candidate.so", ArtifactClass::Program, candidate)?;
            }
            let frozen: eplyx_engine::lifecycle::preflight::Bundle =
                serde_json::from_slice(&registry.artifacts().get(ArtifactClass::Capture, bundle)?)?;
            ensure!(
                frozen.inputs.run_id == *parent_observation
                    && frozen.inputs.preflight_id == *preflight_id
                    && frozen.inputs.wallet_sha256 == *wallet_sha256
                    && frozen.scenario_sha256 == *scenario_sha256,
                "preflight binding mismatch"
            );
            eplyx_engine::lifecycle::preflight::validate(&frozen.inputs)?;
        }
        Input::CurrentObservation { capture } => {
            write("capture.json", ArtifactClass::Capture, capture)?;
            let frozen: eplyx_engine::lifecycle::current::Capture = serde_json::from_slice(
                &registry.artifacts().get(ArtifactClass::Capture, capture)?,
            )?;
            eplyx_engine::lifecycle::current::evaluate(&frozen)?;
        }
        Input::CurrentPath {
            capture,
            parent_observation,
            wallet_sha256,
            check_id,
        } => {
            write("capture.json", ArtifactClass::Capture, capture)?;
            let frozen: eplyx_engine::path::current::Capture = serde_json::from_slice(
                &registry.artifacts().get(ArtifactClass::Capture, capture)?,
            )?;
            ensure!(
                frozen.run_id == *parent_observation
                    && frozen.check_id == *check_id
                    && frozen.wallet_capture_sha256 == *wallet_sha256,
                "path capture binding mismatch"
            );
            eplyx_engine::path::current::validate(&frozen.wallet_capture, &frozen.request)?;
        }
        Input::TokenMigration {
            state_input,
            state_artifact,
            program_capture,
            candidate,
            ..
        } => {
            write("state.json", ArtifactClass::Document, state_input)?;
            let mut state: input::StateInput =
                serde_json::from_slice(&std::fs::read(directory.join("state.json"))?)?;
            input::validate_state(&mut state)?;
            let (name, digest) = match state.config.state {
                input::StateSource::SyntheticFixture {
                    recipe,
                    recipe_sha256,
                } => (recipe, recipe_sha256),
                input::StateSource::CapturedWorld { artifact, sha256 } => (artifact, sha256),
                input::StateSource::MainnetCapture => {
                    anyhow::bail!("hosted jobs require captured or fixture state")
                }
            };
            ensure!(
                digest == state_artifact.sha256,
                "state artifact disagrees with descriptor"
            );
            write(&name, ArtifactClass::Capture, state_artifact)?;
            std::fs::create_dir_all(directory.join("programs"))?;
            write(
                &format!("programs/{}", candidate.sha256),
                ArtifactClass::Program,
                candidate,
            )?;
            let validated = input::load(directory)?;
            if let input::StateSource::SyntheticFixture { recipe, .. } =
                &validated.state().config.state
            {
                let recipe: eplyx_engine::migration::fixture::Recipe =
                    serde_json::from_slice(&std::fs::read(directory.join(recipe))?)?;
                if matches!(
                    recipe.programs,
                    eplyx_engine::migration::fixture::ProgramSource::PinnedMainnetCapture
                ) {
                    let reference = program_capture
                        .as_ref()
                        .context("pinned_program_capture is required for this recipe")?;
                    let bytes = registry
                        .artifacts()
                        .get(ArtifactClass::Capture, reference)?;
                    eplyx_engine::migration::fixture::PinnedPrograms::from_capture(&bytes)?;
                    std::fs::write(directory.join("pinned-programs.capture.json"), bytes)?;
                } else {
                    ensure!(
                        program_capture.is_none(),
                        "bundled recipe does not take a capture"
                    );
                }
            } else {
                ensure!(
                    program_capture.is_none(),
                    "captured world does not take a fixture capture"
                );
            }
            ensure!(
                validated.program_sha256() == candidate.sha256,
                "mechanism identity mismatch"
            );
        }
        Input::LifecycleChange {
            snapshot,
            scenario,
            evidence,
            before,
            at,
            ..
        } => {
            write("snapshot.json", ArtifactClass::Capture, snapshot)?;
            write("scenario.json", ArtifactClass::Document, scenario)?;
            let change = ChangeSpec::parse(&std::fs::read(directory.join("change.json"))?)?;
            let snapshot =
                eplyx_engine::lifecycle::LifecycleSnapshot::load(&directory.join("snapshot.json"))?;
            std::fs::create_dir_all(directory.join("evidence"))?;
            for reference in evidence.values() {
                write(
                    &format!("evidence/{}", reference.sha256),
                    ArtifactClass::Capture,
                    reference,
                )?;
            }
            let scenario = load_scenario(directory, evidence)?;
            change.bind_lifecycle(&scenario)?;
            ensure!(
                before <= at
                    && snapshot.asset.mint
                        == change.as_lifecycle().context("lifecycle kind")?.asset.mint,
                "lifecycle snapshot or evaluation times disagree with proposal"
            );
        }
    }
    Ok(())
}

pub(super) fn member(directory: &Path, name: &str, reference: &ArtifactRef) -> Result<Vec<u8>> {
    use std::io::Read;
    let path = directory.join(name);
    let metadata = std::fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() == reference.len,
        "worker input length mismatch"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(reference.len + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        ArtifactRef::of(&bytes) == *reference,
        "worker input digest mismatch"
    );
    Ok(bytes)
}

pub fn execute(directory: &Path) -> Result<()> {
    local_store::verify_offline_environment()?;
    use std::io::Read;
    let mut request_bytes = Vec::new();
    std::fs::File::open(directory.join("request.json"))?
        .take(64 * 1024 + 1)
        .read_to_end(&mut request_bytes)?;
    ensure!(
        request_bytes.len() <= 64 * 1024,
        "worker request exceeds bound"
    );
    let Request { run_id, input } = serde_json::from_slice(&request_bytes)?;
    ensure!(
        crate::storage::valid_id(&run_id) && run_id.starts_with("run_"),
        "invalid worker run identity"
    );
    let change_bytes = input
        .change()
        .map(|r| member(directory, "change.json", r))
        .transpose()?;
    let spec = change_bytes
        .as_ref()
        .map(|b| ChangeSpec::parse(b))
        .transpose()?;
    let binary = hash_bytes(&std::fs::read(std::env::current_exe()?)?);
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut projection = Projection {
        run_id: run_id.clone(),
        migration_runtime_id: None,
        migration_world: None,
        metadata: Artifact::new(b"{}".to_vec())?,
        report: Artifact::new(b"{}".to_vec())?,
        change_spec: change_bytes.map(Artifact::new).transpose()?,
        state_input: None,
        bindings: None,
        search: None,
        local_artifact_sizes: BTreeMap::new(),
    };
    match &input {
        Input::ProtocolParameterChange { capture, .. } => {
            let bytes = member(directory, "capture.json", capture)?;
            let retained = eplyx_engine::path::current::parameter_input(&bytes)?;
            let report = eplyx_engine::parameter_change::analyze(
                spec.as_ref().context("missing parameter spec")?,
                &retained,
            )?;
            projection.report =
                Artifact::new(eplyx_engine::canonical::document(&report)?.into_bytes())?;
            analytical_metadata(&mut projection, input.kind(), timestamp, binary)?;
        }
        Input::MigrationOrder { .. } => {
            projection.report = Artifact::new(
                eplyx_engine::canonical::document(&super::order::execute(&input, directory)?)?
                    .into_bytes(),
            )?;
            analytical_metadata(&mut projection, input.kind(), timestamp, binary)?;
        }
        Input::CurrentStress {
            state_input,
            candidate,
            files,
            budget,
            parent_observation,
            wallet_sha256,
            stress_id,
            candidate_run_id,
            ..
        } => {
            member(directory, "state.json", state_input)?;
            member(
                directory,
                &format!("programs/{}", candidate.sha256),
                candidate,
            )?;
            for (name, reference) in files {
                member(directory, &format!("stress/{name}"), reference)?;
            }
            member(directory, "world.json", &files["discovery.world.json"])?;
            let validated = input::load(directory)?;
            let report = eplyx_engine::migration::current::finish(
                &validated,
                &directory.join("stress"),
                budget,
            )?;
            let value = json!({"schema_version":1,"kind":"current-stress","run_id":parent_observation,"stress_id":stress_id,"wallet_capture_sha256":wallet_sha256,"candidate_run_id":candidate_run_id,"report":report,"authorization":false,"funds_moved":false,"population_readiness":"Incomplete","limitations":["Fresh bounded population and deterministic frozen cases; no historical holder list or peer inherits a result.","Each case uses its exact coherent final state independently. Outcomes are not summed into rollout capacity.","The candidate reserve is proposed; signing possession and issuer binding remain unknown."]});
            projection.report =
                Artifact::new(eplyx_engine::canonical::document(&value)?.into_bytes())?;
            projection.state_input = Some(Artifact::new(member(
                directory,
                "state.json",
                state_input,
            )?)?);
            analytical_metadata(&mut projection, input.kind(), timestamp, binary)?;
        }
        Input::CurrentCandidate {
            capture,
            candidate,
            parent_observation,
            wallet_sha256,
            check_id,
            ..
        } => {
            let bytes = member(directory, "capture.json", capture)?;
            let program = member(directory, "candidate.so", candidate)?;
            let proof = eplyx_engine::migration::account::replay(
                &bytes,
                parent_observation,
                check_id,
                wallet_sha256,
                &program,
            )?;
            projection.report =
                Artifact::new(eplyx_engine::canonical::document(proof.value())?.into_bytes())?;
            analytical_metadata(&mut projection, input.kind(), timestamp, binary)?;
        }
        Input::CurrentPreflight {
            bundle,
            candidate,
            parent_observation,
            wallet_sha256,
            preflight_id,
            scenario_sha256,
        } => {
            let bytes = member(directory, "preflight.json", bundle)?;
            let program = candidate
                .as_ref()
                .map(|r| member(directory, "candidate.so", r))
                .transpose()?;
            let result = eplyx_engine::lifecycle::preflight::replay(
                &bytes,
                parent_observation,
                preflight_id,
                wallet_sha256,
                scenario_sha256,
                &bundle.sha256,
                program.as_deref(),
            )?;
            projection.report =
                Artifact::new(eplyx_engine::canonical::document(&result)?.into_bytes())?;
            analytical_metadata(&mut projection, input.kind(), timestamp, binary)?;
        }
        Input::CurrentObservation { capture } => {
            let frozen: eplyx_engine::lifecycle::current::Capture =
                serde_json::from_slice(&member(directory, "capture.json", capture)?)?;
            let report = eplyx_engine::lifecycle::current::evaluate(&frozen)?;
            projection.report =
                Artifact::new(eplyx_engine::canonical::document(&report)?.into_bytes())?;
            analytical_metadata(&mut projection, input.kind(), timestamp, binary)?;
        }
        Input::CurrentPath {
            capture,
            parent_observation,
            wallet_sha256,
            check_id,
        } => {
            let bytes = member(directory, "capture.json", capture)?;
            let verified = eplyx_engine::path::current::replay(
                &bytes,
                parent_observation,
                check_id,
                wallet_sha256,
                &capture.sha256,
            )?;
            projection.report =
                Artifact::new(eplyx_engine::canonical::document(verified.value())?.into_bytes())?;
            analytical_metadata(&mut projection, input.kind(), timestamp, binary)?;
        }
        Input::TokenMigration {
            state_input,
            state_artifact,
            candidate,
            policy,
            ..
        } => {
            let state = member(directory, "state.json", state_input)?;
            let descriptor: input::StateInput = serde_json::from_slice(&state)?;
            let name = match descriptor.config.state {
                input::StateSource::SyntheticFixture { recipe, .. } => recipe,
                input::StateSource::CapturedWorld { artifact, .. } => artifact,
                input::StateSource::MainnetCapture => anyhow::bail!("worker cannot observe state"),
            };
            member(directory, &name, state_artifact)?;
            member(
                directory,
                &format!("programs/{}", candidate.sha256),
                candidate,
            )?;
            let output = directory.join("result");
            // run() cannot enter its observation branch: input::load has already
            // validated a CapturedWorld or SyntheticFixture above.
            let report =
                pipeline::run(directory, &output, *policy, pipeline::Isolation::InProcess)?;
            projection.migration_runtime_id =
                Some(eplyx_engine::migration::order::runtime_identity()?);
            let bindings = pipeline::bindings(&output)?;
            let package = input::load(directory)?;
            let world = pipeline::world_for(&package, &output, &bindings)?;
            std::fs::write(
                directory.join("retained-world.json"),
                eplyx_engine::canonical::document(&world)?,
            )?;

            let required = |field: &str| {
                report[field]
                    .as_str()
                    .map(str::to_owned)
                    .with_context(|| format!("engine report lacks {field}"))
            };
            let metadata = local_store::Metadata {
                schema_version: local_store::METADATA_VERSION,
                run_id: run_id.clone(),
                timestamp,
                eplyx_version: eplyx_engine::build_info::VERSION.into(),
                engine_binary_sha256: binary,
                git_commit: None,
                git_branch: None,
                git_dirty: None,
                candidate_program_sha256: required("candidate_program_sha256")?,
                change_spec_id: required("change_spec_id")?,
                analysis_input_sha256: required("analysis_input_sha256")?,
                gate_policy: required("gate_policy")?,
                gate_outcome: required("gate_outcome")?,
                run_source: Some(local_store::RunSource::Hosted),
            };
            projection.metadata = Artifact::new(serde_json::to_vec_pretty(&metadata)?)?;
            projection.report = Artifact::new(std::fs::read(output.join("report.json"))?)?;
            projection.state_input = Some(Artifact::new(state)?);
            projection.bindings =
                Some(Artifact::new(std::fs::read(output.join("bindings.json"))?)?);
        }
        Input::LifecycleChange {
            snapshot,
            scenario,
            evidence,
            before,
            at,
            ..
        } => {
            member(directory, "snapshot.json", snapshot)?;
            member(directory, "scenario.json", scenario)?;
            let snapshot =
                eplyx_engine::lifecycle::LifecycleSnapshot::load(&directory.join("snapshot.json"))?;
            let scenario = load_scenario(directory, evidence)?;
            let spec = spec.as_ref().context("missing lifecycle proposal")?;
            let change = spec.bind_lifecycle(&scenario)?;
            let report = spec.compare_lifecycle(&snapshot, &scenario, *before, *at)?;
            let descriptor = eplyx_engine::canonical::document(
                &json!({"schema_version":1,"kind":"lifecycle_snapshot","snapshot_sha256":report.before.snapshot_sha256,"before":report.before.evaluated_at,"after":report.after.evaluated_at}),
            )?;
            let document = eplyx_engine::canonical::document(
                &json!({"schema_version":1,"change":change,"impact":report,"limitations":["Declared lifecycle policy, not proof of issuer eligibility, conversion or redemption."]}),
            )?;
            projection.report = Artifact::new(document.into_bytes())?;
            projection.state_input = Some(Artifact::new(descriptor.into_bytes())?);
            let metadata = local_store::AnalyticalMetadata {
                schema_version: local_store::ANALYTICAL_METADATA_VERSION,
                run_id: run_id.clone(),
                timestamp,
                kind: input.kind().into(),
                eplyx_version: eplyx_engine::build_info::VERSION.into(),
                engine_binary_sha256: binary,
                run_source: local_store::RunSource::Hosted,
                report_sha256: projection.report.sha256.clone(),
                change_spec_sha256: projection.change_spec.as_ref().map(|a| a.sha256.clone()),
                state_input_sha256: projection.state_input.as_ref().map(|a| a.sha256.clone()),
            };
            projection.metadata = Artifact::new(serde_json::to_vec_pretty(&metadata)?)?;
        }
    }
    projection.verify()?;
    for (name, artifact) in [
        ("metadata.json", Some(&projection.metadata)),
        ("report.json", Some(&projection.report)),
        ("change_spec.json", projection.change_spec.as_ref()),
        ("state_input.json", projection.state_input.as_ref()),
        ("bindings.json", projection.bindings.as_ref()),
    ] {
        projection
            .local_artifact_sizes
            .insert(name.into(), artifact.map(|a| a.text.len() as u64));
    }
    let bytes = serde_json::to_vec(&projection)?;
    ensure!(
        bytes.len() <= crate::projection::MAX_PROJECTION_BYTES,
        "worker output exceeds bound"
    );
    std::fs::write(directory.join("projection.json"), bytes)?;
    Ok(())
}

pub fn exit_code(projection: &Projection) -> Result<u8> {
    let metadata: Value = serde_json::from_str(&projection.metadata.text)?;
    if metadata["kind"] == "migration_order" {
        let report: Value = serde_json::from_str(&projection.report.text)?;
        return if report["failure"].is_null() {
            Ok(0)
        } else {
            let kind: eplyx_engine::migration::order::FailureKind =
                serde_json::from_value(report["failure"]["kind"].clone())?;
            Ok(super::order::failure_exit_code(kind))
        };
    }
    if serde_json::from_str::<Value>(&projection.metadata.text)?["schema_version"]
        == local_store::METADATA_VERSION
    {
        let report: Value = serde_json::from_str(&projection.report.text)?;
        pipeline::exit_code(&report)
    } else {
        Ok(0)
    }
}

/// The only executable here is the configured server binary. Request fields
/// cannot choose a command, environment, pathname or network transport.
pub fn run_isolated(
    registry: &Registry,
    record: &crate::registry::RunMetadata,
    binary: &Path,
) -> Result<Projection> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::Duration,
    };
    let input = registry.hosted_input(record)?;
    registry.clear_run_work(&record.run_id)?;
    let work = registry.run_work_dir(&record.run_id)?;
    stage(registry, &input, &work).map_err(|e| {
        if matches!(input, Input::MigrationOrder { .. }) {
            super::order::evidence_error(e)
        } else {
            e
        }
    })?;
    std::fs::write(
        work.join("request.json"),
        serde_json::to_vec(&Request {
            run_id: record.run_id.clone(),
            input,
        })?,
    )?;
    let mut child = Command::new(binary)
        .arg("offline-analysis")
        .arg(&work)
        .env_clear()
        .current_dir(&work)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    super::process::wait(&mut child, Duration::from_secs(300))?;
    let mut bytes = Vec::new();
    std::fs::File::open(work.join("projection.json"))?
        .take(crate::projection::MAX_PROJECTION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= crate::projection::MAX_PROJECTION_BYTES,
        "offline worker output exceeds bound"
    );
    let mut projection: Projection = serde_json::from_slice(&bytes)?;
    if matches!(registry.hosted_input(record)?, Input::TokenMigration { .. }) {
        let bytes = std::fs::read(work.join("retained-world.json"))?;
        let world: eplyx_engine::migration::world::World = serde_json::from_slice(&bytes)?;
        world.validate()?;
        let report: Value = serde_json::from_str(&projection.report.text)?;
        ensure!(
            report["coverage"]["world"]["world_sha256"] == world.sha256()?,
            "retained world differs from report"
        );
        projection.migration_world = Some(
            registry
                .artifacts()
                .put(ArtifactClass::Capture, &bytes)?
                .reference,
        );
    }
    super::order::retain(
        registry,
        &registry.hosted_input(record)?,
        &work,
        &mut projection,
    )?;
    registry
        .verify_hosted_projection(record, &projection)
        .map_err(|e| {
            if matches!(
                registry.hosted_input(record),
                Ok(Input::MigrationOrder { .. })
            ) {
                super::order::evidence_error(e)
            } else {
                e
            }
        })?;
    Ok(projection)
}

fn analytical_metadata(
    projection: &mut Projection,
    kind: &str,
    timestamp: String,
    binary: String,
) -> Result<()> {
    let metadata = local_store::AnalyticalMetadata {
        schema_version: local_store::ANALYTICAL_METADATA_VERSION,
        run_id: projection.run_id.clone(),
        timestamp,
        kind: kind.into(),
        eplyx_version: eplyx_engine::build_info::VERSION.into(),
        engine_binary_sha256: binary,
        run_source: local_store::RunSource::Hosted,
        report_sha256: projection.report.sha256.clone(),
        change_spec_sha256: projection.change_spec.as_ref().map(|a| a.sha256.clone()),
        state_input_sha256: projection.state_input.as_ref().map(|a| a.sha256.clone()),
    };
    projection.metadata = Artifact::new(serde_json::to_vec_pretty(&metadata)?)?;
    Ok(())
}

fn load_scenario(
    directory: &Path,
    evidence: &BTreeMap<String, ArtifactRef>,
) -> Result<eplyx_engine::lifecycle::policy::LifecycleScenario> {
    let scenario: eplyx_engine::lifecycle::policy::LifecycleScenario =
        serde_json::from_slice(&std::fs::read(directory.join("scenario.json"))?)?;
    scenario.validate()?;
    let mut used = 0;
    for source in &scenario.sources {
        if let (Some(artifact), Some(hash)) = (&source.artifact, &source.content_sha256) {
            ensure!(
                !Path::new(artifact).is_absolute() && !artifact.contains('\\'),
                "source artifact must be a relative logical reference"
            );
            let reference = evidence
                .get(&source.id)
                .context("missing lifecycle source evidence")?;
            ensure!(
                reference.sha256 == *hash,
                "source evidence identity mismatch"
            );
            member(
                directory,
                &format!("evidence/{}", reference.sha256),
                reference,
            )?;
            used += 1;
        }
    }
    ensure!(used == evidence.len(), "unexpected lifecycle evidence");
    Ok(scenario)
}
