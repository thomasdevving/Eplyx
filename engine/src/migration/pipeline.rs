//! Token Migration V1 preflight, isolated offline evaluation and byte-exact replay.
//!
//! `run` validates every package byte before any request, captures read-only state
//! when the package asks for mainnet state, binds every input digest, then hands the
//! evaluation to an offline worker with an empty environment (no RPC URL, no cloud
//! token). The evaluation writes the plan and the frozen stress matrix before it
//! executes anything, then the rehearsal, stress results, unsigned plan and report.
//! `replay` recomputes every artifact offline and compares bytes; saved statuses are
//! never trusted.
use super::{
    adapter, capture, execute,
    fixture::{self, FixtureContext},
    input::{self as package, StateSource, ValidatedInput},
    invariants,
    planner::{self, PlanInput},
    rehearsal, report,
    stress::{self, CaseContext},
    unsigned,
    world::World,
};
use crate::{
    ingest::rpc::{HttpRpc as HttpSolanaRpc, RpcProvider as SolanaRpc},
    migration::gate::{self as package_gate, Policy},
    migration::{population, population_types::StressBudget},
    replay::hash_bytes as sha256,
};
use anyhow::{bail, ensure, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub const BINDINGS_SCHEMA: u32 = 1;
pub const BINDINGS_KIND: &str = "token-migration";
pub const WORKER_COMMAND: &str = "finish-migration-preflight";
pub const OFFLINE_TIMEOUT: Duration = Duration::from_secs(900);
pub const BINDINGS: &str = "bindings.json";
pub const PLAN: &str = "migration.plan.json";
pub const STRESS_PLAN: &str = "stress.plan.json";
pub const STRESS_RESULTS: &str = "stress.results.json";
pub const REHEARSAL: &str = "population.rehearsal.json";
pub const UNSIGNED: &str = "migration.unsigned-plan.json";
pub const REPORT: &str = "report.json";
pub const REPORT_MD: &str = "report.md";
const MAX_ARTIFACT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bindings {
    /// Missing on historical MAIN runs, whose exact version-1 Markdown remains reproducible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_presentation_version: Option<u32>,
    pub schema_version: u32,
    pub kind: String,
    pub gate_policy: Policy,
    pub analysis_input_sha256: String,
    pub candidate_program_sha256: String,
    pub state_input_sha256: String,
    pub change_spec_id: String,
    pub state: String,
    pub recipe_sha256: Option<String>,
    pub population_capture_sha256: Option<String>,
    pub migration_capture_sha256: Option<String>,
    pub population_budget: Option<StressBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_capture_binding_sha256: Option<String>,
    pub run_id: String,
    pub evaluated_at: String,
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn read(root: &Path, file: &str) -> Result<Vec<u8>> {
    let path = root.join(file);
    let len = fs::metadata(&path)
        .with_context(|| format!("missing run artifact {file}"))?
        .len();
    ensure!(len <= MAX_ARTIFACT, "run artifact {file} exceeds its bound");
    Ok(fs::read(path)?)
}

fn write_new(root: &Path, file: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(file))
        .with_context(|| format!("run artifact {file} already exists"))?;
    out.write_all(bytes)?;
    out.sync_all()?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Write,
    Verify,
}

fn artifact(
    root: &Path,
    mode: Mode,
    file: &str,
    bytes: &[u8],
    digests: &mut BTreeMap<String, String>,
) -> Result<()> {
    match mode {
        Mode::Write => write_new(root, file, bytes)?,
        Mode::Verify => ensure!(
            read(root, file)? == bytes,
            "saved {file} differs from offline replay"
        ),
    }
    digests.insert(file.into(), sha256(bytes));
    Ok(())
}

/// How the offline evaluation is isolated from the capturing process.
pub enum Isolation {
    /// A child process of `executable` with an empty environment.
    Process(PathBuf),
    /// The same process; for tests that call the library directly. Still offline.
    InProcess,
}

pub fn offline_worker(executable: &Path, package: &Path, result: &Path) -> Command {
    let mut worker = Command::new(executable);
    worker
        .arg(WORKER_COMMAND)
        .arg(package)
        .arg("--result")
        .arg(result)
        .env_clear();
    worker
}

/// The read-only providers and budget a mainnet-state run captures through.
pub struct ObservedSource<'a, P, E> {
    /// Answers the bounded population enumeration.
    pub population: &'a P,
    /// Answers the migration identity, destination and Clock batches.
    pub execution: &'a E,
    pub budget: StressBudget,
}

/// Validate, capture (mainnet state only), bind and evaluate a migration package.
/// Mainnet state is read through `SOLANA_RPC_URL`; a fixture reads no RPC.
pub fn run(
    package_dir: &Path,
    output: &Path,
    policy: Policy,
    isolation: Isolation,
) -> Result<Value> {
    if !matches!(
        package::load(package_dir)?.config.state,
        StateSource::MainnetCapture
    ) {
        return run_with::<HttpSolanaRpc, HttpSolanaRpc>(
            package_dir,
            output,
            policy,
            isolation,
            None,
        );
    }
    let url = std::env::var("SOLANA_RPC_URL")
        .context("mainnet state requires SOLANA_RPC_URL (read-only)")?;
    let budget = StressBudget::from_env();
    budget.validate()?;
    let population = HttpSolanaRpc::new(url.clone())?
        .with_response_limit(usize::try_from(budget.max_response_bytes)?)?
        .with_timeout(budget.population_timeout_seconds)?;
    let execution = HttpSolanaRpc::new(url)?;
    run_with(
        package_dir,
        output,
        policy,
        isolation,
        Some(ObservedSource {
            population: &population,
            execution: &execution,
            budget,
        }),
    )
}

/// [`run`] with explicit read-only providers for mainnet state. Every request is
/// recorded in the capture artifacts; evaluation afterwards is offline.
pub fn run_with<P: SolanaRpc, E: SolanaRpc>(
    package_dir: &Path,
    output: &Path,
    policy: Policy,
    isolation: Isolation,
    source: Option<ObservedSource<'_, P, E>>,
) -> Result<Value> {
    let package = package::load(package_dir)?;
    fs::create_dir(output).context("output directory must not already exist")?;
    let run_id = format!(
        "mig-{}-{}",
        &package.analysis_input_sha256[..12],
        Utc::now().timestamp_millis()
    );
    let mut bindings = Bindings {
        report_presentation_version: Some(2),
        schema_version: BINDINGS_SCHEMA,
        kind: BINDINGS_KIND.into(),
        gate_policy: policy,
        analysis_input_sha256: package.analysis_input_sha256.clone(),
        candidate_program_sha256: package.program_sha256.clone(),
        state_input_sha256: package.state_input_sha256.clone(),
        change_spec_id: package.change_spec_id.clone(),
        state: "SyntheticFixture".into(),
        recipe_sha256: None,
        population_capture_sha256: None,
        migration_capture_sha256: None,
        population_budget: None,
        current_capture_binding_sha256: None,
        run_id: run_id.clone(),
        evaluated_at: String::new(),
    };
    match &package.config.state {
        StateSource::CapturedWorld { .. } => {
            ensure!(source.is_none(), "a captured world input reads no RPC");
            bindings.state = "CapturedWorld".into();
        }
        StateSource::SyntheticFixture { recipe_sha256, .. } => {
            ensure!(source.is_none(), "a synthetic fixture package reads no RPC");
            bindings.recipe_sha256 = Some(recipe_sha256.clone());
        }
        StateSource::MainnetCapture => {
            let ObservedSource {
                population: population_rpc,
                execution: execution_rpc,
                budget,
            } = source.context("mainnet state requires a read-only RPC")?;
            budget.validate()?;
            let spec = package.spec();
            let capture_population = population::capture(
                spec.source.mint.clone(),
                run_id.clone(),
                format!("{run_id}-population"),
                budget.clone(),
                population_rpc,
            )?;
            population::save(
                &capture_population,
                &output.join(capture::POPULATION_ARTIFACT),
            )?;
            let population_bytes = read(output, capture::POPULATION_ARTIFACT)?;
            let observation = population::evaluate_bytes(&population_bytes, &budget)?;
            let overlay = adapter::derive(spec, &package.change_spec_id, package.program_id())?;
            let migration = capture::capture(
                spec,
                &overlay,
                &observation,
                package.config.max_captured_holders,
                &run_id,
                execution_rpc,
            )?;
            let migration_bytes = crate::canonical::document(&migration)?;
            write_new(
                output,
                capture::MIGRATION_ARTIFACT,
                migration_bytes.as_bytes(),
            )?;
            bindings.state = "MainnetCapture".into();
            bindings.population_capture_sha256 = Some(sha256(&population_bytes));
            bindings.migration_capture_sha256 = Some(sha256(migration_bytes.as_bytes()));
            let discovery =
                capture::world(spec, &population_bytes, migration_bytes.as_bytes(), &budget)?;
            bindings.current_capture_binding_sha256 = Some(super::current::capture_with(
                &package,
                &population_bytes,
                &budget,
                &discovery,
                &output.join("current"),
                execution_rpc,
            )?);
            bindings.population_budget = Some(budget);
        }
    }
    bindings.evaluated_at = now();
    write_new(
        output,
        BINDINGS,
        crate::canonical::document(&bindings)?.as_bytes(),
    )?;
    match isolation {
        Isolation::InProcess => finish(package_dir, output)?,
        Isolation::Process(executable) => {
            let mut child = offline_worker(&executable, package_dir, output)
                .spawn()
                .context("could not start the isolated offline VM worker")?;
            let start = Instant::now();
            loop {
                if let Some(status) = child.try_wait()? {
                    ensure!(status.success(), "the offline migration worker failed");
                    break;
                }
                if start.elapsed() >= OFFLINE_TIMEOUT {
                    child.kill()?;
                    child.wait()?;
                    bail!("the offline migration worker exceeded its deadline");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let report: Value = serde_json::from_slice(&read(output, REPORT)?)?;
    ensure!(
        report["analysis_input_sha256"] == bindings.analysis_input_sha256,
        "worker package identity mismatch"
    );
    Ok(report)
}

/// The offline evaluation entry point. It never reads an RPC URL.
pub fn finish(package_dir: &Path, output: &Path) -> Result<()> {
    let package = package::load(package_dir)?;
    evaluate(&package, output, Mode::Write).map(|_| ())
}

pub fn replay(package_dir: &Path, output: &Path) -> Result<Value> {
    let package = package::load(package_dir)?;
    evaluate(&package, output, Mode::Verify)
}

/// Verify the saved run, then apply a different gate policy to the same findings.
pub fn replay_with_policy(
    package_dir: &Path,
    output: &Path,
    policy: Option<Policy>,
) -> Result<Value> {
    let saved = replay(package_dir, output)?;
    match policy {
        None => Ok(saved),
        Some(policy) => {
            let mut analytical = saved;
            if let Some(object) = analytical.as_object_mut() {
                for field in [
                    "deployment_gate",
                    "gate_policy",
                    "gate_outcome",
                    "gate_reasons",
                    "gate_reason_codes",
                ] {
                    object.remove(field);
                }
            }
            with_gate(analytical, policy)
        }
    }
}

pub fn bindings(output: &Path) -> Result<Bindings> {
    let bytes = read(output, BINDINGS)?;
    ensure!(bytes.len() <= 64 * 1024, "bindings exceed their bound");
    Ok(serde_json::from_slice(&bytes)?)
}

/// Rebuild the exact rehearsal world for a bound run.
pub fn world_for(package: &ValidatedInput, output: &Path, b: &Bindings) -> Result<World> {
    let spec = package.spec();
    match &package.config.state {
        StateSource::CapturedWorld { .. } => {
            ensure!(b.state == "CapturedWorld", "state binding changed");
            package.world.clone().context("missing captured world")
        }
        StateSource::SyntheticFixture { recipe_sha256, .. } => {
            super::error::compatible(
                b.state == "SyntheticFixture" && b.recipe_sha256.as_deref() == Some(recipe_sha256),
                "fixture binding changed",
            )?;
            let overlay = adapter::derive(spec, &package.change_spec_id, package.program_id())?;
            let recipe = package.recipe.as_ref().context("missing fixture recipe")?;
            let context = FixtureContext {
                migration_authority: Some(overlay.migration_authority),
            };
            let portable = package.root().join("pinned-programs.capture.json");
            if matches!(
                recipe.programs,
                fixture::ProgramSource::PinnedMainnetCapture
            ) && portable.exists()
            {
                let bytes = std::fs::read(portable)?;
                fixture::build_with_programs(
                    recipe,
                    &context,
                    fixture::PinnedPrograms::from_capture(&bytes)?,
                )
            } else {
                fixture::build(recipe, &context)
            }
        }
        StateSource::MainnetCapture => {
            ensure!(b.state == "MainnetCapture", "state binding changed");
            let population_bytes = read(output, capture::POPULATION_ARTIFACT)?;
            let migration_bytes = read(output, capture::MIGRATION_ARTIFACT)?;
            super::error::compatible(
                Some(sha256(&population_bytes)) == b.population_capture_sha256
                    && Some(sha256(&migration_bytes)) == b.migration_capture_sha256,
                "captured state changed",
            )?;
            capture::world(
                spec,
                &population_bytes,
                &migration_bytes,
                b.population_budget
                    .as_ref()
                    .context("missing capture budget")?,
            )
        }
    }
}

fn with_gate(mut report: Value, policy: Policy) -> Result<Value> {
    let gate = package_gate::evaluate_migration(&report, policy)?;
    report["gate_policy"] = policy.name().into();
    report["gate_outcome"] = serde_json::to_value(gate.outcome)?;
    report["gate_reasons"] = serde_json::to_value(&gate.reasons)?;
    report["gate_reason_codes"] = serde_json::to_value(&gate.reason_codes)?;
    report["deployment_gate"] = serde_json::to_value(gate)?;
    Ok(report)
}

pub fn exit_code(report: &Value) -> Result<u8> {
    let saved: package_gate::DeploymentGate =
        serde_json::from_value(report["deployment_gate"].clone())?;
    let expected = package_gate::evaluate_migration(report, saved.policy)?;
    ensure!(saved == expected, "deployment gate result mismatch");
    package_gate::exit_code(report, &expected)
}

fn evaluate(package: &ValidatedInput, output: &Path, mode: Mode) -> Result<Value> {
    let b = bindings(output)?;
    super::error::compatible(
        b.schema_version == BINDINGS_SCHEMA
            && b.kind == BINDINGS_KIND
            && b.analysis_input_sha256 == package.analysis_input_sha256
            && b.candidate_program_sha256 == package.program_sha256
            && b.state_input_sha256 == package.state_input_sha256
            && b.change_spec_id == package.change_spec_id,
        "package identity changed since this run was bound",
    )?;
    let world = world_for(package, output, &b)?;
    let current = if let Some(expected) = &b.current_capture_binding_sha256 {
        ensure!(
            b.state == "MainnetCapture",
            "current capture binding on non-observed input"
        );
        let root = output.join("current");
        ensure!(
            sha256(&read(&root, "current.bindings.json")?) == *expected,
            "current capture binding changed"
        );
        ensure!(
            sha256(&read(&root, "population.capture.json")?)
                == b.population_capture_sha256
                    .as_deref()
                    .context("missing population digest")?,
            "current population differs from run"
        );
        let discovery: World = serde_json::from_slice(&read(&root, "discovery.world.json")?)?;
        super::error::compatible(
            discovery.sha256()? == world.sha256()?,
            "current discovery differs from migration world",
        )?;
        let budget = b
            .population_budget
            .as_ref()
            .context("current budget missing")?;
        Some(match mode {
            Mode::Write => super::current::finish(package, &root, budget)?,
            Mode::Verify => super::current::replay(package, &root, budget)?,
        })
    } else {
        None
    };
    let mut inputs = RehearsalInputs::new(package, b.gate_policy);
    inputs.current = current.as_ref();
    inputs.report_presentation_version = b.report_presentation_version.unwrap_or(1);
    ensure!(
        [1, 2].contains(&inputs.report_presentation_version),
        "unsupported report presentation version"
    );
    let mut digests = BTreeMap::new();
    evaluate_world(&inputs, &world, &mut |file, bytes| {
        artifact(output, mode, file, bytes, &mut digests)
    })
}

/// Everything an offline rehearsal needs besides the world.
pub struct RehearsalInputs<'a> {
    report_presentation_version: u32,
    spec: &'a super::spec::TokenMigrationV1,
    change_spec_id: &'a str,
    program_id: &'a str,
    program: &'a crate::change::ResolvedCandidate,
    program_sha256: &'a str,
    analysis_input_sha256: &'a str,
    state_input_sha256: &'a str,
    clock_policy: planner::RehearsalClockPolicy,
    max_rehearsal_units: usize,
    invariants: &'a [super::invariants::MigrationInvariant],
    policy: Policy,
    change: &'a crate::change::ChangeSpec,
    current: Option<&'a super::current::CurrentReport>,
}

impl<'a> RehearsalInputs<'a> {
    pub fn new(input: &'a ValidatedInput, policy: Policy) -> Self {
        Self {
            report_presentation_version: 2,
            spec: input.spec(),
            change_spec_id: &input.change_spec_id,
            program_id: input.program_id(),
            program: &input.candidate,
            program_sha256: &input.program_sha256,
            analysis_input_sha256: &input.analysis_input_sha256,
            state_input_sha256: &input.state_input_sha256,
            clock_policy: input.config.rehearsal_clock,
            max_rehearsal_units: input.config.max_rehearsal_units,
            invariants: &input.state.invariants,
            policy,
            change: &input.change,
            current: None,
        }
    }
}

/// The pure offline evaluation over one validated world. Artifacts go to `sink`
/// in a fixed order: plan and frozen stress matrix before anything executes.
pub fn evaluate_world(
    inputs: &RehearsalInputs<'_>,
    world: &World,
    sink: &mut dyn FnMut(&str, &[u8]) -> Result<()>,
) -> Result<Value> {
    let spec = inputs.spec;
    let mut digests: BTreeMap<String, String> = BTreeMap::new();
    let mut emit =
        |file: &str, bytes: &[u8], digests: &mut BTreeMap<String, String>| -> Result<()> {
            sink(file, bytes)?;
            digests.insert(file.into(), sha256(bytes));
            Ok(())
        };
    let plan = planner::plan(&PlanInput {
        spec,
        change_spec_id: inputs.change_spec_id,
        world,
        program_id: inputs.program_id,
        candidate_program_sha256: inputs.program_sha256,
        clock_policy: inputs.clock_policy,
        reserve_override: None,
        focus: None,
    })?;
    emit(
        PLAN,
        crate::canonical::document(&plan)?.as_bytes(),
        &mut digests,
    )?;
    // The stress matrix is durable and immutable before any case executes.
    let stress_plan = stress::select(spec, world, &plan)?;
    emit(
        STRESS_PLAN,
        crate::canonical::document(&stress_plan)?.as_bytes(),
        &mut digests,
    )?;
    let programs = execute::programs(world, spec, inputs.program_id, inputs.program)?;
    execute::assert_candidate(&programs, inputs.program_id, inputs.program_sha256)?;
    let loaded_programs: Vec<Value> = programs
        .iter()
        .map(|p| {
            let id = p.program_id.to_string();
            json!({
                "program_id": id,
                "loader": p.loader.to_string(),
                "elf_sha256": sha256(&p.bytes),
                "origin": if id == inputs.program_id { "CandidatePackage".to_string() } else {
                    world.get(&id).map(|a| a.origin.label().to_string()).unwrap_or_else(|| "Unknown".into())
                },
            })
        })
        .collect();
    let rehearsal = rehearsal::rehearse(spec, world, &plan, &programs, inputs.max_rehearsal_units)?;
    let context = CaseContext {
        spec,
        change_spec_id: inputs.change_spec_id,
        world,
        program_id: inputs.program_id,
        candidate: inputs.program,
        candidate_sha256: inputs.program_sha256,
        clock_policy: inputs.clock_policy,
    };
    let stress_outcome = stress::run(&context, &stress_plan)?;
    emit(
        REHEARSAL,
        crate::canonical::document(&rehearsal)?.as_bytes(),
        &mut digests,
    )?;
    emit(
        STRESS_RESULTS,
        crate::canonical::document(&stress_outcome)?.as_bytes(),
        &mut digests,
    )?;
    let resolved = spec.resolve()?;
    let config_bytes =
        adapter::config_bytes(spec, &resolved, &plan.overlay, inputs.change_spec_id)?;
    let unsigned_plan = unsigned::build(
        spec,
        &plan,
        inputs.analysis_input_sha256,
        &config_bytes,
        &rehearsal.executions,
    )?;
    emit(
        UNSIGNED,
        crate::canonical::document(&unsigned_plan)?.as_bytes(),
        &mut digests,
    )?;
    // The serialized descriptors alone must reproduce the rehearsal.
    let unsigned_cross_check = unsigned::cross_check(&unsigned_plan, world, &programs)?;
    let candidate_loaded = programs
        .iter()
        .find(|p| p.program_id.to_string() == inputs.program_id)
        .map(|p| sha256(&p.bytes));
    let evidence = invariants::Evidence {
        spec,
        plan: &plan,
        rehearsal: &rehearsal,
        stress: &stress_outcome,
        candidate_loaded_sha256: candidate_loaded.as_deref(),
        package_program_sha256: inputs.program_sha256,
        refs: [PLAN, STRESS_PLAN, REHEARSAL, STRESS_RESULTS]
            .iter()
            .filter_map(|f| digests.get(*f).cloned())
            .collect(),
    };
    let findings = invariants::evaluate(inputs.invariants, &evidence);
    let identity = json!({
        "change": crate::change::ChangeBinding {
            change_spec_id: inputs.change.id()?,
            change: crate::change::BoundChange::TokenMigration {
                source_mint: spec.source.mint.clone(), destination_mint: spec.destination.mint.clone(),
                candidate_sha256: inputs.program_sha256.into(),
            },
        },
        "analysis_input_sha256": inputs.analysis_input_sha256,
        "candidate_program_sha256": inputs.program_sha256,
        "state_input_sha256": inputs.state_input_sha256,
        "change_spec_id": inputs.change_spec_id,
        "adapter": adapter::ADAPTER,
        "adapter_version": adapter::ADAPTER_VERSION,
        "program_id": inputs.program_id,
    });
    let mut report = report::build(&report::ReportInput {
        spec,
        world,
        plan: &plan,
        stress_plan: &stress_plan,
        stress: &stress_outcome,
        rehearsal: &rehearsal,
        invariants: &findings,
        identity,
        artifacts: digests.clone(),
        unsigned_units: unsigned_plan.units.len(),
        unsigned_cross_check: &unsigned_cross_check,
        loaded_programs,
    })?;
    if let Some(current) = inputs.current {
        report["current_state_guarantees"] = serde_json::to_value(current)?;
    }
    let report = with_gate(report, inputs.policy)?;
    let mut ignored: BTreeMap<String, String> = BTreeMap::new();
    emit(
        REPORT,
        crate::canonical::document(&report)?.as_bytes(),
        &mut ignored,
    )?;
    emit(
        REPORT_MD,
        report::markdown_version(&report, inputs.report_presentation_version).as_bytes(),
        &mut ignored,
    )?;
    Ok(report)
}
