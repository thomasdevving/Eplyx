//! Lifecycle commands share MAIN's binary and isolate all offline evaluation.
use super::Format;
use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use eplyx_engine::{
    change::ChangeSpec,
    lifecycle::{frozen::selection as expansion, policy::LifecycleScenario, LifecycleSnapshot},
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::ExitCode};

#[derive(Subcommand, Serialize, Deserialize)]
pub enum LifecycleCommand {
    /// Evaluate declared policy over one immutable snapshot.
    Analyse(ImpactArgs),
    /// Capture a read-only token population. Transport credentials stay in the parent.
    Snapshot {
        #[arg(long)]
        asset: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Compare policy times over pinned evidence.
    CompareScenarios(CounterfactualArgs),
    IngestNotice(NoticeArgs),
    ScenarioFromEvent(NoticeArgs),
    PreflightFromNotice(NoticeArgs),
    Readiness(ReadinessArgs),
    ResolvePaths(ResolvePathsArgs),
    EvaluateRollout(RolloutArgs),
    GuardRollout(GuardRolloutArgs),
    DemoRollout(DemoRolloutArgs),
}

#[derive(Parser)]
pub struct ChangeArgs {
    #[arg(long)]
    scenario: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

pub fn change(args: ChangeArgs) -> Result<ExitCode> {
    let result = (|| {
        let scenario = LifecycleScenario::load(&args.scenario)?;
        let mut spec = ChangeSpec::lifecycle(&scenario)?;
        spec.change_spec_id = Some(spec.id()?);
        expansion::save(&spec, &args.out)?;
        Ok(super::cli_local::Response {
            exit_code: 0,
            data: serde_json::to_value(&spec)?,
            text: format!("Lifecycle change {}", spec.id()?),
        })
    })();
    Ok(super::cli_local::emit(args.format, result))
}

impl LifecycleCommand {
    fn format(&self) -> Format {
        match self {
            Self::Snapshot { .. } => Format::Text,
            Self::Analyse(a) => a.format,
            Self::CompareScenarios(a) => a.format,
            Self::IngestNotice(a) | Self::ScenarioFromEvent(a) | Self::PreflightFromNotice(a) => {
                a.format
            }
            Self::Readiness(a) => a.format,
            Self::ResolvePaths(a) => a.format,
            Self::EvaluateRollout(a) => a.format,
            Self::GuardRollout(a) => a.evaluation.format,
            Self::DemoRollout(a) => a.format,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerRequest {
    command: LifecycleCommand,
    temporary_root: PathBuf,
}
pub fn execute(command: LifecycleCommand) -> Result<ExitCode> {
    let format = command.format();
    Ok(execute_inner(command).unwrap_or_else(|error| super::cli_local::emit(format, Err(error))))
}
fn execute_inner(command: LifecycleCommand) -> Result<ExitCode> {
    if let LifecycleCommand::Snapshot { asset, out } = command {
        use eplyx_engine::{
            ingest::rpc::HttpRpc,
            lifecycle::{LifecycleStateSource, SolanaTokenAssetSource},
        };
        anyhow::ensure!(!out.exists(), "snapshot output already exists");
        let asset = expansion::load(&asset)?;
        let provider = HttpRpc::new(
            std::env::var("SOLANA_RPC_URL").context("SOLANA_RPC_URL is required for capture")?,
        )?;
        let snapshot = SolanaTokenAssetSource { rpc: provider }.capture(asset)?;
        snapshot.save(&out)?;
        println!("Captured {} token accounts.", snapshot.entities.len());
        return Ok(ExitCode::SUCCESS);
    }
    let temporary_root = std::env::temp_dir().canonicalize()?;
    if let LifecycleCommand::GuardRollout(args) = &command {
        eplyx_engine::lifecycle::rollout::validate_temporary_marker(&args.marker, &temporary_root)?;
    }
    let request = serde_json::to_string(&WorkerRequest {
        command,
        temporary_root,
    })?;
    let status = eplyx_engine::local_store::offline_command(&std::env::current_exe()?)
        .arg("lifecycle-worker")
        .arg(request)
        .status()?;
    Ok(ExitCode::from(status.code().unwrap_or(2) as u8))
}

pub fn worker(request: &str) -> Result<ExitCode> {
    eplyx_engine::local_store::verify_offline_environment()?;
    anyhow::ensure!(
        request.len() <= 64 * 1024,
        "lifecycle request exceeds byte bound"
    );
    let WorkerRequest {
        command,
        temporary_root,
    } = serde_json::from_str(request)?;
    let format = command.format();
    let result = (|| match command {
        LifecycleCommand::Snapshot { .. } => {
            anyhow::bail!("capture is unavailable inside an offline worker")
        }
        LifecycleCommand::Analyse(args) => lifecycle_impact(args),
        LifecycleCommand::CompareScenarios(args) => compare_scenarios(args),
        LifecycleCommand::IngestNotice(args) => notice(args, 0),
        LifecycleCommand::ScenarioFromEvent(args) => notice(args, 1),
        LifecycleCommand::PreflightFromNotice(args) => notice(args, 2),
        LifecycleCommand::Readiness(args) => readiness(args),
        LifecycleCommand::ResolvePaths(args) => resolve_paths(args),
        LifecycleCommand::EvaluateRollout(args) => evaluate_rollout(args, None, &temporary_root),
        LifecycleCommand::GuardRollout(args) => {
            evaluate_rollout(args.evaluation, Some(args.marker), &temporary_root)
        }
        LifecycleCommand::DemoRollout(args) => demo_rollout(args, &temporary_root),
    })();
    Ok(result.unwrap_or_else(|error| super::cli_local::emit(format, Err(error))))
}
#[derive(Parser, Serialize, Deserialize)]
pub struct RolloutArgs {
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    binding: PathBuf,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[arg(long)]
    out: Option<PathBuf>,
    /// Select an existing pinned lifecycle view without changing historical proof.
    #[arg(long, value_enum)]
    target_view: Option<RolloutTargetView>,
}
#[derive(Parser, Serialize, Deserialize)]
pub struct GuardRolloutArgs {
    #[command(flatten)]
    evaluation: RolloutArgs,
    #[arg(long)]
    marker: PathBuf,
}
#[derive(Parser, Serialize, Deserialize)]
pub struct DemoRolloutArgs {
    #[arg(long)]
    cases: PathBuf,
    #[arg(long)]
    binding: PathBuf,
    #[arg(long)]
    marker_directory: PathBuf,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[arg(long)]
    out: Option<PathBuf>,
}
#[derive(Parser, Serialize, Deserialize)]
pub struct CounterfactualArgs {
    #[arg(long)]
    snapshot: PathBuf,
    #[arg(long)]
    scenario: PathBuf,
    #[arg(long)]
    readiness_policy: PathBuf,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[arg(long)]
    out: Option<PathBuf>,
}
#[derive(Parser, Serialize, Deserialize)]
pub struct NoticeArgs {
    #[arg(long)]
    workflow: PathBuf,
    #[arg(long)]
    event: Option<PathBuf>,
    #[arg(long)]
    scenario: Option<PathBuf>,
    #[arg(long)]
    binding: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    out_binding: Option<PathBuf>,
    #[arg(long)]
    out_impact: Option<PathBuf>,
    #[arg(long)]
    out_resolution: Option<PathBuf>,
    #[arg(long)]
    out_readiness: Option<PathBuf>,
}
#[derive(Parser, Serialize, Deserialize)]
pub struct ReadinessArgs {
    #[arg(long)]
    snapshot: PathBuf,
    #[arg(long)]
    scenario: PathBuf,
    #[arg(long)]
    policy: PathBuf,
    #[arg(long)]
    direct_resolution: PathBuf,
    #[arg(long)]
    position_resolution: PathBuf,
    #[arg(long)]
    coverage: PathBuf,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[arg(long)]
    out: Option<PathBuf>,
}
#[derive(Parser, Serialize, Deserialize)]
pub struct ResolvePathsArgs {
    #[arg(long)]
    snapshot: PathBuf,
    #[arg(long)]
    scenario: PathBuf,
    #[arg(long)]
    entity: String,
    #[arg(long)]
    coverage: PathBuf,
    #[arg(long)]
    discovery: PathBuf,
    #[arg(long)]
    evidence_bundle: PathBuf,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[arg(long)]
    out: Option<PathBuf>,
}
#[derive(Parser, Serialize, Deserialize)]
pub struct ImpactArgs {
    #[arg(long)]
    change_spec: Option<PathBuf>,
    #[arg(long)]
    snapshot: PathBuf,
    #[arg(long)]
    scenario: PathBuf,
    /// Hypothetical lifecycle evaluation time (RFC3339); never a new chain capture.
    #[arg(long)]
    at: chrono::DateTime<chrono::Utc>,
    /// Baseline semantic time. Defaults to one nanosecond before effective_at.
    #[arg(long)]
    before: Option<chrono::DateTime<chrono::Utc>>,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// New deterministic JSON report path; existing artifacts are never overwritten.
    #[arg(long)]
    out: Option<PathBuf>,
}
#[derive(Clone, Copy, clap::ValueEnum, Serialize, Deserialize)]
enum RolloutTargetView {
    #[value(name = "before_transition")]
    BeforeTransition,
    #[value(name = "after_transition")]
    AfterTransition,
    #[value(name = "after_deadline")]
    AfterDeadline,
}
impl RolloutTargetView {
    fn id(self) -> &'static str {
        match self {
            Self::BeforeTransition => "before_transition",
            Self::AfterTransition => "after_transition",
            Self::AfterDeadline => "after_deadline",
        }
    }
}
fn notice(args: NoticeArgs, stage: u8) -> Result<ExitCode> {
    use eplyx_engine::lifecycle::notice::workflow::NoticeWorkflow;
    let outputs = [
        &args.out,
        &args.out_binding,
        &args.out_impact,
        &args.out_resolution,
        &args.out_readiness,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for p in outputs.into_iter().flatten() {
        anyhow::ensure!(!p.exists(), "notice output already exists: {}", p.display());
        let parent = p
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let absolute = parent
            .canonicalize()?
            .join(p.file_name().context("output filename missing")?);
        anyhow::ensure!(seen.insert(absolute), "duplicate notice output");
    }
    anyhow::ensure!(
        stage == 2
            || (args.out_impact.is_none()
                && args.out_resolution.is_none()
                && args.out_readiness.is_none()),
        "pipeline outputs require preflight-from-notice"
    );
    anyhow::ensure!(
        stage != 0
            || (args.event.is_none()
                && args.scenario.is_none()
                && args.binding.is_none()
                && args.out_binding.is_none()),
        "ingest-notice accepts workflow and event output only"
    );
    anyhow::ensure!(
        stage != 1 || (args.scenario.is_none() && args.binding.is_none()),
        "scenario-from-event regenerates scenario/binding"
    );
    let workflow = NoticeWorkflow::load(&args.workflow)?;
    let base = args.workflow.parent().unwrap_or(std::path::Path::new("."));
    let event = if let Some(p) = &args.event {
        expansion::load(p)?
    } else {
        workflow.normalize(base)?.event().clone()
    };
    if stage == 0 {
        if let Some(p) = &args.out {
            expansion::save(&event, p)?;
        }
        match args.format {
            Format::Json => print!("{}", event.to_json()?),
            Format::Text => print!("{}", event.render_text()),
        }
        return Ok(ExitCode::SUCCESS);
    }
    let (expected, expected_binding) = workflow.generate(base, &event)?;
    let scenario = if let Some(p) = &args.scenario {
        expansion::load(p)?
    } else {
        expected
    };
    let binding = if let Some(p) = &args.binding {
        expansion::load(p)?
    } else {
        expected_binding
    };
    if stage == 1 {
        workflow.verify_scenario(base, &event, &scenario, &binding)?;
        if let Some(p) = &args.out {
            expansion::save(&scenario, p)?;
        }
        if let Some(p) = &args.out_binding {
            expansion::save(&binding, p)?;
        }
        match args.format {Format::Json=>print!("{}",scenario.to_json()?),Format::Text=>println!("Generated lifecycle scenario {}\nOfficialTransition: NotTested; evaluation boundary: DemoConfigured",scenario.id)}
        return Ok(ExitCode::SUCCESS);
    }
    let (report, impact) = workflow.preflight(base, &event, &scenario, &binding)?;
    if let Some(p) = &args.out {
        expansion::save(&report, p)?;
    }
    if let Some(p) = &args.out_binding {
        expansion::save(&binding, p)?;
    }
    if let Some(p) = &args.out_impact {
        expansion::save(&impact, p)?;
    }
    if let Some(p) = &args.out_resolution {
        expansion::save(&report.resolution, p)?;
    }
    if let Some(p) = &args.out_readiness {
        expansion::save(&report.readiness, p)?;
    }
    match args.format {
        Format::Json => print!("{}", report.to_json()?),
        Format::Text => print!("{}", report.render_text()),
    }
    Ok(ExitCode::from(report.readiness.overall_status.exit_code()))
}
fn readiness(args: ReadinessArgs) -> Result<ExitCode> {
    use eplyx_engine::lifecycle::readiness::{
        self, evidence::ReadinessEvidenceManifest, LifecycleReadinessPolicy,
    };
    anyhow::ensure!(
        !args.out.as_ref().is_some_and(|p| p.exists()),
        "readiness output already exists"
    );
    let policy: LifecycleReadinessPolicy = expansion::load(&args.policy)?;
    let normalized = policy.normalized()?;
    let policy_base = args.policy.parent().unwrap_or(std::path::Path::new("."));
    let manifest: ReadinessEvidenceManifest =
        serde_json::from_slice(&normalized.evidence_manifest.read(policy_base)?)?;
    let manifest_path = policy_base.join(&normalized.evidence_manifest.file);
    let evidence = manifest.verify(
        manifest_path.parent().unwrap_or(std::path::Path::new(".")),
        &args.snapshot,
        &args.scenario,
        &args.direct_resolution,
        &args.position_resolution,
        &args.coverage,
    )?;
    let report = readiness::evaluate(&normalized, &evidence)?;
    if let Some(out) = &args.out {
        expansion::save(&report, out)?;
    }
    match args.format {
        Format::Json => print!("{}", report.to_json()?),
        Format::Text => print!("{}", report.render_text()),
    }
    Ok(ExitCode::from(report.overall_status.exit_code()))
}
fn resolve_paths(args: ResolvePathsArgs) -> Result<ExitCode> {
    use eplyx_engine::lifecycle::resolution;
    if args.out.as_ref().is_some_and(|p| p.exists()) {
        return Err(anyhow!("path resolution output already exists"));
    }
    let s = LifecycleSnapshot::load(&args.snapshot)?;
    let scenario = LifecycleScenario::load(&args.scenario)?;
    let report = resolution::phase7::resolve(
        &args.evidence_bundle,
        &s,
        &scenario,
        &args.entity,
        &args.coverage,
        &args.discovery,
    )?;
    if let Some(path) = &args.out {
        expansion::save(&report, path)?;
    }
    match args.format {
        Format::Json => print!("{}", report.to_json()?),
        Format::Text => print!("{}", report.render_text()),
    }
    Ok(ExitCode::SUCCESS)
}
fn compare_scenarios(args: CounterfactualArgs) -> Result<ExitCode> {
    use eplyx_engine::lifecycle::counterfactual::FrozenCounterfactualWorld;
    anyhow::ensure!(
        !args.out.as_ref().is_some_and(|p| p.exists()),
        "counterfactual output already exists"
    );
    let world =
        FrozenCounterfactualWorld::load(&args.snapshot, &args.scenario, &args.readiness_policy)?;
    let report = world.evaluate(&world.scenarios()?)?;
    if let Some(out) = &args.out {
        expansion::save(&report, out)?;
    }
    match args.format {
        Format::Json => print!("{}", report.to_json()?),
        Format::Text => print!("{}", report.render_text()),
    }
    Ok(ExitCode::SUCCESS)
}
fn evaluate_rollout(
    args: RolloutArgs,
    marker: Option<PathBuf>,
    temporary_root: &std::path::Path,
) -> Result<ExitCode> {
    use eplyx_engine::lifecycle::rollout::{
        run_guarded_stub_in, CandidateRolloutPlan, GateCommandCompletion, GuardRun,
        RolloutValidator,
    };
    anyhow::ensure!(
        !args.out.as_ref().is_some_and(|p| p.exists()),
        "rollout output already exists"
    );
    anyhow::ensure!(
        !marker.as_ref().is_some_and(|p| p.exists()),
        "guard marker already exists"
    );
    let plan: CandidateRolloutPlan = expansion::load(&args.plan)?;
    let mut plan = plan.normalized()?;
    let validator = RolloutValidator::load(&args.binding)?;
    if let Some(target) = args.target_view {
        plan.target_view = validator
            .counterfactual()
            .scenarios
            .iter()
            .find(|view| view.scenario.id == target.id())
            .context("requested lifecycle view is unavailable")?
            .scenario
            .clone();
    }
    let evaluated = validator.evaluate(
        &plan,
        args.plan.parent().unwrap_or(std::path::Path::new(".")),
    )?;
    let code = evaluated.command_exit_code();
    if let Some(marker) = marker {
        let observation = run_guarded_stub_in(
            &evaluated,
            GateCommandCompletion::AssuranceEvaluation {
                exit_code: evaluated.report().readiness_exit_code,
            },
            &marker,
            temporary_root,
        )?;
        let result = GuardRun {
            assessment: evaluated.report().clone(),
            observation,
        };
        if let Some(out) = &args.out {
            expansion::save(&result, out)?;
        }
        match args.format {
            Format::Json => print!("{}", expansion::canonical(&result)?),
            Format::Text => println!(
                "{}Marker created: {}",
                evaluated.render_text(),
                result.observation.marker_created
            ),
        }
    } else {
        if let Some(out) = &args.out {
            expansion::save(evaluated.report(), out)?;
        }
        match args.format {
            Format::Json => print!("{}", evaluated.to_json()?),
            Format::Text => print!("{}", evaluated.render_text()),
        }
    }
    Ok(ExitCode::from(code))
}
fn demo_rollout(args: DemoRolloutArgs, temporary_root: &std::path::Path) -> Result<ExitCode> {
    use eplyx_engine::lifecycle::rollout::{run_demo_in, RolloutDemoCases, RolloutValidator};
    anyhow::ensure!(
        !args.out.as_ref().is_some_and(|p| p.exists()),
        "rollout demo output already exists"
    );
    let cases: RolloutDemoCases = expansion::load(&args.cases)?;
    let validator = RolloutValidator::load(&args.binding)?;
    let report = run_demo_in(
        &validator,
        &cases,
        args.cases.parent().unwrap_or(std::path::Path::new(".")),
        &args.marker_directory,
        temporary_root,
    )?;
    if let Some(out) = &args.out {
        expansion::save(&report, out)?;
    }
    match args.format {
        Format::Json => print!("{}", expansion::canonical(&report)?),
        Format::Text => {
            println!("ROLLOUT ASSUMPTION DEMONSTRATIONS (local-only)");
            for case in &report.cases {
                println!("{}: candidate {:?}; readiness {:?} ({:?}, exit {}); workflow {:?}; marker created {}",case.assessment.candidate.id,case.assessment.candidate_acceptance,case.assessment.readiness.overall_status,case.assessment.readiness.evaluated_scope,case.assessment.readiness_exit_code,case.observation.disposition,case.observation.marker_created);
            }
            println!("Demo command exit 0 is analysis completion, not population readiness or rollout authorization.");
        }
    }
    Ok(ExitCode::SUCCESS)
}
fn lifecycle_impact(args: ImpactArgs) -> Result<ExitCode> {
    if let Some(path) = &args.out {
        if path.exists() {
            return Err(anyhow!("impact output already exists: {}", path.display()));
        }
    }
    let snapshot = LifecycleSnapshot::load(&args.snapshot)?;
    let scenario = LifecycleScenario::load(&args.scenario)?;
    let before = match args.before {
        Some(before) => before,
        None => std::cmp::min(
            args.at,
            scenario
                .policy
                .effective_at
                .checked_sub_signed(chrono::Duration::nanoseconds(1))
                .context("effective_at has no preceding baseline time")?,
        ),
    };
    let spec = args
        .change_spec
        .as_ref()
        .map(|p| ChangeSpec::parse(&eplyx_engine::lifecycle::artifact::read(p)?))
        .transpose()?
        .unwrap_or(ChangeSpec::lifecycle(&scenario)?);
    let change = spec.bind_lifecycle(&scenario)?;
    let report = spec.compare_lifecycle(&snapshot, &scenario, before, args.at)?;
    let document = eplyx_engine::canonical::document(
        &serde_json::json!({"schema_version":1,"change":change,"impact":report,"limitations":["Declared lifecycle policy, not proof of issuer eligibility, conversion or redemption."]}),
    )?;
    if let Some(path) = &args.out {
        eplyx_engine::path::save_json(
            &serde_json::from_str::<serde_json::Value>(&document)?,
            path,
        )?;
    }
    print!(
        "{}",
        match args.format {
            Format::Text => report.render_text(),
            Format::Json => document,
        }
    );
    Ok(ExitCode::SUCCESS)
}
