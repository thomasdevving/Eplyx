//! Strict project configuration and local analysis assembly.
use super::*;
use anyhow::bail;
use eplyx_engine::migration::{
    adapter,
    input::{self, Config as InputConfig, StateSource, ValidatedInput},
    invariants::MigrationInvariant,
    planner::RehearsalClockPolicy,
    rehearsal::DEFAULT_MAX_UNITS,
    spec::TokenMigrationV1,
};
use serde::Deserialize;
use std::path::Component;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gate {
    pub policy: Policy,
}
pub const ADAPTER: &str = adapter::ADAPTER;
const MAX_SPEC_BYTES: u64 = 64 * 1024;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub adapter: String,
    /// The TokenMigrationV1 specification (JSON), relative to the project root.
    pub spec: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub path: String,
    /// The program ID you intend to deploy under; defaults to the reference ID.
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    /// `mainnet` (bounded read-only capture) or `fixture` (synthetic recipe).
    pub source: String,
    #[serde(default)]
    pub fixture: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rehearsal {
    #[serde(default)]
    pub clock: Option<String>,
    #[serde(default)]
    pub max_units: Option<usize>,
    #[serde(default)]
    pub max_captured_holders: Option<usize>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationConfig {
    pub project: Project,
    pub transition: Transition,
    pub program: Program,
    pub state: State,
    #[serde(default)]
    pub rehearsal: Rehearsal,
    #[serde(default)]
    pub invariants: Vec<MigrationInvariant>,
    pub gate: Gate,
}

impl MigrationConfig {
    pub fn parse(value: &str) -> Result<Self> {
        let config: Self =
            toml::from_str(value).context("invalid eplyx.toml fields for token_migration_v1")?;
        ensure!(
            config.transition.adapter == ADAPTER,
            "unsupported adapter {}",
            config.transition.adapter
        );
        ensure!(
            !config.project.name.trim().is_empty() && config.project.name.len() <= 80,
            "set a project name of 1–80 characters"
        );
        ensure!(
            matches!(config.state.source.as_str(), "mainnet" | "fixture"),
            "set [state].source to \"mainnet\" or \"fixture\""
        );
        ensure!(
            (config.state.source == "fixture") == config.state.fixture.is_some(),
            "[state].fixture is required for fixture state and forbidden for mainnet state"
        );
        Ok(config)
    }
    pub fn uses_rpc(&self) -> bool {
        self.state.source == "mainnet"
    }
}

/// A project file that must stay inside the root, including through symlinks.
fn project_file(root: &Path, relative: &str, max: u64, what: &str) -> Result<Vec<u8>> {
    let path = Path::new(relative);
    ensure!(
        !path.is_absolute()
            && path
                .components()
                .all(|c| matches!(c, Component::CurDir | Component::Normal(_))),
        "{what} path must stay relative to the project root"
    );
    let full = root
        .join(path)
        .canonicalize()
        .with_context(|| format!("{what} {relative} is missing"))?;
    ensure!(
        full.starts_with(root) && full.is_file(),
        "{what} path escapes the project root or is not a file"
    );
    ensure!(
        fs::metadata(&full)?.len() <= max,
        "{what} exceeds its size bound"
    );
    let bytes = bounded_read(&full, max)?;
    ensure!(bytes.len() as u64 <= max, "{what} exceeds its size bound");
    Ok(bytes)
}

pub fn spec(root: &Path, config: &MigrationConfig) -> Result<TokenMigrationV1> {
    let bytes = project_file(
        root,
        &config.transition.spec,
        MAX_SPEC_BYTES,
        "migration spec",
    )?;
    let spec: TokenMigrationV1 =
        serde_json::from_slice(&bytes).context("invalid TokenMigrationV1 specification")?;
    spec.validate()?;
    Ok(spec)
}

pub fn candidate(root: &Path, config: &MigrationConfig) -> Result<Vec<u8>> {
    project_file(
        root,
        &config.program.path,
        input::MAX_PROGRAM_BYTES,
        "candidate program",
    )
    .context(
        "candidate program missing; use cargo build-sbf for your candidate and set [program].path",
    )
}

fn input_config(root: &Path, config: &MigrationConfig) -> Result<(InputConfig, Option<Vec<u8>>)> {
    let clock = match config.rehearsal.clock.as_deref() {
        None | Some("activation") => RehearsalClockPolicy::Activation,
        Some("captured") => RehearsalClockPolicy::Captured,
        Some(other) => {
            bail!("unsupported [rehearsal].clock {other:?}; use \"activation\" or \"captured\"")
        }
    };
    let recipe = match &config.state.fixture {
        Some(path) => Some(project_file(
            root,
            path,
            input::MAX_RECIPE_BYTES,
            "fixture recipe",
        )?),
        None => None,
    };
    let state = match &recipe {
        Some(bytes) => StateSource::SyntheticFixture {
            recipe: "fixture.json".into(),
            recipe_sha256: sha256(bytes),
        },
        None => StateSource::MainnetCapture,
    };
    Ok((
        InputConfig {
            state,
            rehearsal_clock: clock,
            max_rehearsal_units: config.rehearsal.max_units.unwrap_or(DEFAULT_MAX_UNITS),
            max_captured_holders: config.rehearsal.max_captured_holders.unwrap_or(5_000),
        },
        recipe,
    ))
}

pub fn input_in(
    root: &Path,
    config: &MigrationConfig,
    destination: &Path,
) -> Result<ValidatedInput> {
    let spec = spec(root, config)?;
    let program = candidate(root, config)?;
    let (input_config, recipe) = input_config(root, config)?;
    let program_id = config
        .program
        .id
        .clone()
        .unwrap_or_else(|| adapter::REFERENCE_PROGRAM_ID.into());
    input::assemble(
        destination,
        &spec,
        &program_id,
        &program,
        &input_config,
        recipe.as_deref(),
        config.invariants.clone(),
    )
}

pub fn validate_ephemeral(
    root: &Path,
    config: &MigrationConfig,
    base: &Path,
) -> Result<ValidatedInput> {
    let path = base.join("cache").join(format!(
        "validate_{}_{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let result = input_in(root, config, &path);
    if path.exists() {
        fs::remove_dir_all(&path)?;
    }
    result
}

pub fn metadata(
    root: &Path,
    id: &str,
    package: &ValidatedInput,
    policy: Policy,
    outcome: &str,
) -> Result<Metadata> {
    let status = git(root, &["status", "--porcelain"]);
    Ok(Metadata {
        schema_version: METADATA_VERSION,
        run_id: id.into(),
        timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        eplyx_version: env!("CARGO_PKG_VERSION").into(),
        engine_binary_sha256: sha256(&fs::read(std::env::current_exe()?)?),
        git_commit: git(root, &["rev-parse", "HEAD"]),
        git_branch: git(root, &["branch", "--show-current"]),
        git_dirty: status.map(|s| !s.is_empty()),
        candidate_program_sha256: package.program_sha256().into(),
        change_spec_id: package.change_spec_id().into(),
        analysis_input_sha256: package.analysis_input_sha256().into(),
        gate_policy: policy.name().into(),
        gate_outcome: outcome.into(),
        run_source: Some(RunSource::detect()),
    })
}

pub fn doctor_lines(root: &Path, config: &MigrationConfig, base: &Path) -> Vec<(bool, String)> {
    let mut rows = vec![];
    match spec(root, config) {
        Ok(spec) => rows.push((
            true,
            format!(
                "Migration spec        {} → {} ({} → {})",
                short(&spec.source.mint),
                short(&spec.destination.mint),
                program_label(&spec.source.token_program),
                program_label(&spec.destination.token_program)
            ),
        )),
        Err(error) => rows.push((false, format!("Migration spec        {error:#}"))),
    }
    match candidate(root, config) {
        Ok(bytes) => rows.push((
            true,
            format!(
                "Candidate program     {} ({})",
                config.program.path,
                &sha256(&bytes)[..12]
            ),
        )),
        Err(error) => rows.push((false, format!("Candidate program     {error:#}"))),
    }
    if let Some(fixture) = &config.state.fixture {
        rows.push((
            true,
            format!("State                 synthetic fixture {fixture} (never chain state)"),
        ));
    } else {
        rows.push((
            true,
            "State                 bounded read-only mainnet capture".into(),
        ));
    }
    match validate_ephemeral(root, config, base) {
        Ok(package) => rows.push((
            true,
            format!(
                "Analysis input    {} · {} · {} invariants",
                &package.analysis_input_sha256()[..12],
                ADAPTER,
                package.state().invariants.len()
            ),
        )),
        Err(error) => rows.push((
            false,
            format!("Analysis input    {error:#}; fix eplyx.toml or the spec"),
        )),
    }
    rows
}

fn short(address: &str) -> String {
    format!("{}…", address.chars().take(8).collect::<String>())
}

fn program_label(program: &str) -> &'static str {
    eplyx_engine::migration::spec::TokenProgram::from_address(program)
        .map(|p| p.label())
        .unwrap_or("unknown")
}
