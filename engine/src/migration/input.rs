//! Migration state inputs, separate from MAIN's identifying ChangeSpec.
//!
//! `change.json` names the proposal and executable. `state.json` names only state,
//! clock policy, execution bounds and declared invariants. Executable bytes live
//! at `programs/<sha256>` and can enter this module only through ChangeSpec::resolve.
use super::{
    fixture::{self, Recipe},
    invariants::{MigrationInvariant, INVARIANT_SCHEMA_VERSION, MAX_INVARIANTS},
    planner::RehearsalClockPolicy,
    rehearsal,
    spec::TokenMigrationV1,
};
use crate::{
    change::{CandidateSource, ChangeSpec, ResolvedCandidate},
    replay::hash_bytes as sha256,
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_CHANGE_BYTES: u64 = 64 * 1024;
pub const MAX_STATE_BYTES: u64 = 64 * 1024;
pub const MAX_PROGRAM_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_RECIPE_BYTES: u64 = fixture::MAX_RECIPE_BYTES as u64;
pub const MAX_STRESS_CASES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateSource {
    MainnetCapture,
    CapturedWorld {
        artifact: String,
        sha256: String,
    },
    SyntheticFixture {
        recipe: String,
        recipe_sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub state: StateSource,
    pub rehearsal_clock: RehearsalClockPolicy,
    pub max_rehearsal_units: usize,
    pub max_captured_holders: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateInput {
    pub schema_version: u32,
    pub config: Config,
    pub invariant_schema_version: u32,
    pub invariants: Vec<MigrationInvariant>,
}

/// Runtime aggregation, never another serialized proposal or registry.
/// Fields are private to migration; consumers cannot forge a validated input.
pub struct ValidatedInput {
    pub(super) root: PathBuf,
    pub(super) change: ChangeSpec,
    pub(super) state: StateInput,
    pub(super) config: Config,
    pub(super) candidate: ResolvedCandidate,
    pub(super) program_sha256: String,
    pub(super) state_input_sha256: String,
    pub(super) change_spec_id: String,
    pub(super) recipe: Option<Recipe>,
    pub(super) world: Option<super::world::World>,
    pub(super) analysis_input_sha256: String,
    terms: TokenMigrationV1,
}
impl ValidatedInput {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn program_sha256(&self) -> &str {
        &self.program_sha256
    }
    pub fn state_input_sha256(&self) -> &str {
        &self.state_input_sha256
    }
    pub fn spec(&self) -> &TokenMigrationV1 {
        &self.terms
    }
    pub fn change(&self) -> &ChangeSpec {
        &self.change
    }
    pub fn state(&self) -> &StateInput {
        &self.state
    }
    pub fn candidate(&self) -> &ResolvedCandidate {
        &self.candidate
    }
    pub fn change_spec_id(&self) -> &str {
        &self.change_spec_id
    }
    pub fn analysis_input_sha256(&self) -> &str {
        &self.analysis_input_sha256
    }
    pub fn program_id(&self) -> &str {
        &self
            .change
            .as_token_migration()
            .expect("validated kind")
            .mechanism
            .program_id
    }
}
fn bounded_read(path: &Path, max: u64) -> Result<Vec<u8>> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("missing input file {}", path.display()))?;
    ensure!(
        metadata.is_file() && metadata.len() <= max,
        "input file exceeds its size bound"
    );
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= max,
        "input file exceeds its size bound"
    );
    Ok(bytes)
}

fn member(root: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    ensure!(
        !name.is_empty()
            && !path.is_absolute()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "input path must be a relative member without traversal"
    );
    let resolved = root
        .join(path)
        .canonicalize()
        .context("missing input member")?;
    ensure!(
        resolved.starts_with(root),
        "input member escapes input root"
    );
    Ok(resolved)
}

fn valid_digest(input: &str) -> bool {
    input.len() == 64
        && input
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Only Solana SBF/BPF shared ELF can enter the VM.
pub fn validate_sbf(bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() >= 64, "candidate is not a supported SBF ELF");
    ensure!(
        &bytes[..4] == b"\x7fELF" && bytes[4] == 2 && bytes[5] == 1,
        "candidate is not a 64-bit little-endian ELF"
    );
    ensure!(
        u16::from_le_bytes([bytes[16], bytes[17]]) == 3,
        "candidate must be a loadable shared ELF"
    );
    ensure!(
        matches!(u16::from_le_bytes([bytes[18], bytes[19]]), 247 | 263),
        "candidate is not Solana SBF/BPF"
    );
    Ok(())
}

pub fn validate_state(state: &mut StateInput) -> Result<()> {
    ensure!(
        state.schema_version == SCHEMA_VERSION,
        "unsupported migration state input schema"
    );
    ensure!(
        state.invariant_schema_version == INVARIANT_SCHEMA_VERSION,
        "unsupported migration invariant schema"
    );
    ensure!(
        state.invariants.len() <= MAX_INVARIANTS,
        "invariant count exceeds bounds"
    );
    let mut kinds: Vec<&str> = state.invariants.iter().map(|i| i.kind()).collect();
    kinds.sort();
    ensure!(
        kinds.windows(2).all(|pair| pair[0] != pair[1]),
        "duplicate invariant definition"
    );
    state.invariants.sort();
    let config = &state.config;
    ensure!(
        (1..=rehearsal::MAX_UNITS).contains(&config.max_rehearsal_units),
        "max_rehearsal_units exceeds bounds"
    );
    ensure!(
        (1..=100_000).contains(&config.max_captured_holders),
        "max_captured_holders exceeds bounds"
    );
    if let StateSource::SyntheticFixture {
        recipe,
        recipe_sha256,
    } = &config.state
    {
        ensure!(
            recipe == "fixture.json",
            "recipe member must be fixture.json"
        );
        ensure!(valid_digest(recipe_sha256), "invalid recipe SHA-256");
    }
    if let StateSource::CapturedWorld { artifact, sha256 } = &config.state {
        ensure!(
            artifact == "world.json" && valid_digest(sha256),
            "invalid captured world reference"
        );
    }
    Ok(())
}

/// Identity of the analytical input, not a second ChangeSpec identity.
pub fn identity(change: &ChangeSpec, state: &StateInput) -> Result<String> {
    crate::canonical::digest(&("eplyx-migration-input-v1", change.id()?, state))
}

/// Read and validate public documents without accepting their claimed outcomes.
pub fn declared_identity(
    change_bytes: &[u8],
    state_bytes: &[u8],
) -> Result<(ChangeSpec, StateInput, String)> {
    ensure!(
        change_bytes.len() as u64 <= MAX_CHANGE_BYTES
            && state_bytes.len() as u64 <= MAX_STATE_BYTES,
        "input documents exceed bounds"
    );
    let change = ChangeSpec::parse(change_bytes)?;
    ensure!(
        change.as_token_migration().is_some(),
        "token_migration ChangeSpec required"
    );
    let mut state: StateInput = serde_json::from_slice(state_bytes)?;
    validate_state(&mut state)?;
    let id = identity(&change, &state)?;
    Ok((change, state, id))
}

pub fn load(directory: &Path) -> Result<ValidatedInput> {
    let root = directory
        .canonicalize()
        .context("input directory missing")?;
    ensure!(root.is_dir(), "input root is not a directory");
    let change_bytes = bounded_read(&member(&root, "change.json")?, MAX_CHANGE_BYTES)?;
    let state_bytes = bounded_read(&member(&root, "state.json")?, MAX_STATE_BYTES)?;
    let (change, state, analysis_input_sha256) = declared_identity(&change_bytes, &state_bytes)?;
    let artifact = change.candidate().context("migration mechanism missing")?;
    let program_sha256 = artifact.sha256.clone();
    // Bound and validate the file before resolving it. The resolver rechecks both
    // hash and length; a path swap cannot substitute different executable bytes.
    let program = bounded_read(
        &member(&root, &format!("programs/{}", artifact.sha256))?,
        MAX_PROGRAM_BYTES,
    )?;
    validate_sbf(&program)?;
    let candidate = change.resolve(CandidateSource::Bytes(&program))?;
    let config = state.config.clone();
    let recipe = match &config.state {
        StateSource::MainnetCapture | StateSource::CapturedWorld { .. } => None,
        StateSource::SyntheticFixture {
            recipe,
            recipe_sha256,
        } => {
            let bytes = bounded_read(&member(&root, recipe)?, MAX_RECIPE_BYTES)?;
            ensure!(
                sha256(&bytes) == *recipe_sha256,
                "fixture recipe SHA-256 mismatch"
            );
            Some(Recipe::parse(&bytes)?)
        }
    };
    let world = if let StateSource::CapturedWorld {
        artifact,
        sha256: expected,
    } = &config.state
    {
        let bytes = bounded_read(&member(&root, artifact)?, 512 * 1024 * 1024)?;
        super::error::compatible(
            sha256(&bytes) == *expected,
            "captured world digest mismatch",
        )?;
        let world: super::world::World = serde_json::from_slice(&bytes)?;
        world.validate()?;
        ensure!(
            world.base_kind() == super::world::WorldKind::ObservedCapture,
            "captured-world input must carry captured provenance"
        );
        Some(world)
    } else {
        None
    };
    let terms = change
        .as_token_migration()
        .context("migration kind")?
        .evaluation_spec(change.activation.as_ref())?;
    let change_spec_id = change.id()?;
    let state_input_sha256 = crate::canonical::digest(&state)?;
    Ok(ValidatedInput {
        root,
        change,
        state,
        config,
        candidate,
        program_sha256,
        state_input_sha256,
        change_spec_id,
        recipe,
        world,
        analysis_input_sha256,
        terms,
    })
}

/// Assemble separate proposal/state documents and a content-addressed executable.
pub fn assemble(
    destination: &Path,
    spec: &TokenMigrationV1,
    program_id: &str,
    program: &[u8],
    config: &Config,
    recipe: Option<&[u8]>,
    invariants: Vec<MigrationInvariant>,
) -> Result<ValidatedInput> {
    validate_sbf(program)?;
    let change = ChangeSpec::token_migration(spec.clone(), program_id, program)?;
    let mut state = StateInput {
        schema_version: SCHEMA_VERSION,
        config: config.clone(),
        invariant_schema_version: INVARIANT_SCHEMA_VERSION,
        invariants,
    };
    validate_state(&mut state)?;
    match (&config.state, recipe) {
        (StateSource::SyntheticFixture { recipe_sha256, .. }, Some(bytes)) => {
            ensure!(sha256(bytes) == *recipe_sha256, "recipe digest mismatch");
            Recipe::parse(bytes)?;
        }
        (
            StateSource::CapturedWorld {
                sha256: expected, ..
            },
            Some(bytes),
        ) => {
            ensure!(sha256(bytes) == *expected, "captured world digest mismatch");
            let world: super::world::World = serde_json::from_slice(bytes)?;
            world.validate()?;
            ensure!(
                world.base_kind() == super::world::WorldKind::ObservedCapture,
                "captured-world input must carry captured provenance"
            );
        }
        (StateSource::MainnetCapture, None) => {}
        _ => bail!("the state source and recipe disagree"),
    }
    std::fs::create_dir(destination).context("input directory must not already exist")?;
    let write = |name: &str, bytes: &[u8]| -> Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination.join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    };
    write(
        "change.json",
        crate::canonical::document(&change)?.as_bytes(),
    )?;
    write("state.json", crate::canonical::document(&state)?.as_bytes())?;
    std::fs::create_dir(destination.join("programs"))?;
    write(&format!("programs/{}", sha256(program)), program)?;
    if let Some(bytes) = recipe {
        write(
            if matches!(config.state, StateSource::CapturedWorld { .. }) {
                "world.json"
            } else {
                "fixture.json"
            },
            bytes,
        )?;
    }
    load(destination)
}
