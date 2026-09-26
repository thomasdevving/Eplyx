//! File formats of a project's local `.eplyx/` run store, shared by the `eplyx`
//! CLI that writes them and the read-only dashboard that presents them. These
//! are local bookkeeping records around engine artifacts, never new evidence.
use crate::migration::search;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub schema_version: u32,
    pub run_id: String,
    pub timestamp: String,
    pub eplyx_version: String,
    pub engine_binary_sha256: String,
    pub git_commit: Option<String>,
    pub git_branch: Option<String>,
    pub git_dirty: Option<bool>,
    pub candidate_program_sha256: String,
    pub change_spec_id: String,
    pub analysis_input_sha256: String,
    pub gate_policy: String,
    pub gate_outcome: String,
    /// Where the run was produced. Absent on schema 1 metadata written before
    /// the field existed; never inferred for those runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_source: Option<RunSource>,
}

/// Metadata schema that carries `run_source`.
pub const METADATA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunSource {
    /// `eplyx migration analyse` on a developer machine.
    Local,
    /// `eplyx migration analyse` with a non-empty `CI` environment variable.
    Ci,
    /// Reserved for runs brought in from another store; nothing writes it yet.
    Imported,
}

impl RunSource {
    pub fn detect() -> Self {
        match std::env::var("CI") {
            Ok(value)
                if !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false") =>
            {
                Self::Ci
            }
            _ => Self::Local,
        }
    }
}

/// One local `eplyx migration reproduce` attempt. History only: it records what the
/// offline replay concluded and never substitutes for re-running it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reproduction {
    pub schema_version: u32,
    pub id: String,
    pub counterexample_id: String,
    pub parent_run: Option<String>,
    pub search_sha256: Option<String>,
    pub timestamp: String,
    pub outcome: ReproductionOutcome,
    /// The saved counterexample, including its failure signature and
    /// rollback, was found unchanged in the offline VM replay.
    pub failure_signature_matched: bool,
    pub error: Option<String>,
    pub eplyx_version: String,
    pub engine_binary_sha256: String,
    /// Replay ran in an empty-environment child; no provider was used.
    pub no_rpc: bool,
    /// Exact identities re-verified by a successful migration reproduction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<ReproductionBinding>,
}

/// What a migration reproduction bound: analytical input, candidate, rehearsal world,
/// counterexample, expected failure signature and the resulting outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReproductionBinding {
    pub analysis_input_sha256: String,
    pub candidate_program_sha256: String,
    pub world_sha256: String,
    pub counterexample_kind: String,
    pub finding: String,
    pub expected_signature: serde_json::Value,
    pub reproduced_signature: Option<serde_json::Value>,
    pub gate_outcome_with_finding: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReproductionOutcome {
    Reproduced,
    Failed,
}

pub const REPRODUCTION_VERSION: u32 = 1;

/// `repro_<UTC millis>_<cx digest>`: sortable, and safe as a local ID.
pub fn reproduction_id(timestamp: &str, counterexample: &str) -> String {
    format!(
        "repro_{timestamp}_{}",
        counterexample.strip_prefix("cx_").unwrap_or(counterexample)
    )
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplayInputs {
    pub input: String,
    pub result: String,
    pub search: String,
}

pub const MIGRATION_COUNTEREXAMPLE_KIND: &str = "token_migration";

/// A saved migration witness. Deserialization does not verify its claims.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedMigrationCounterexample {
    pub schema_version: u32,
    pub kind: String,
    pub id: String,
    pub parent_run: String,
    pub search_sha256: String,
    pub replay_inputs: ReplayInputs,
    pub counterexample: crate::migration::search::Counterexample,
}

/// Store-relative replay inputs; always forward slashes on every platform.
pub fn replay_inputs(run: &str) -> ReplayInputs {
    ReplayInputs {
        input: format!("runs/{run}/input"),
        result: format!("runs/{run}/result"),
        search: format!("runs/{run}/search"),
    }
}

/// Content-addressed local witness identity; verified only by offline replay.
pub fn counterexample_id(counterexample: &search::Counterexample) -> Result<String> {
    search::counterexample_id(counterexample)
}

/// Local IDs are lowercase ASCII, digits and underscores behind a fixed prefix,
/// so they can never carry a separator, dot, drive letter or encoded byte.
pub fn is_safe_id(id: &str, prefix: &str) -> bool {
    id.starts_with(prefix)
        && id.len() > prefix.len()
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub fn safe_id(id: &str, prefix: &str) -> Result<()> {
    ensure!(is_safe_id(id, prefix), "invalid local ID");
    Ok(())
}

/// All local VM/replay children inherit no credentials, RPC endpoints or user environment.
pub fn offline_command(executable: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new(executable);
    command.env_clear();
    command
}

/// macOS adds its text-encoding hint after exec even when Command::env_clear was
/// used. Drop that platform hint at the single-threaded worker entry point, then
/// reject every remaining variable before any evaluation or file replay begins.
pub fn verify_offline_environment() -> Result<()> {
    #[cfg(target_os = "macos")]
    std::env::remove_var("__CF_USER_TEXT_ENCODING");
    ensure!(
        std::env::vars_os().next().is_none(),
        "offline worker requires an empty environment"
    );
    Ok(())
}

/// Local bookkeeping for kinds that have no migration candidate or deployment gate.
/// Report bytes are the immutable source; this record supplies provenance only.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyticalMetadata {
    pub schema_version: u32,
    pub run_id: String,
    pub timestamp: String,
    pub kind: String,
    pub eplyx_version: String,
    pub engine_binary_sha256: String,
    pub run_source: RunSource,
    pub report_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_spec_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_input_sha256: Option<String>,
}
pub const ANALYTICAL_METADATA_VERSION: u32 = 3;

/// Append one already evaluated record. Metadata is written last; interrupted
/// records remain explicitly unfinished. Existing runs and project identity
/// are never replaced. Paths are local arguments, never serialized into evidence.
pub fn save_analysis(
    project: &std::path::Path,
    kind: &str,
    report: &[u8],
    change_spec: Option<&[u8]>,
    state_input: Option<&[u8]>,
    source: RunSource,
) -> Result<AnalyticalMetadata> {
    use crate::replay::hash_bytes as sha256;
    use std::{fs, io::Write, path::Path};
    ensure!(
        ["lifecycle_change", "current_observation", "current_path"].contains(&kind),
        "unsupported local analytical kind"
    );
    ensure!(
        report.len() <= 128 * 1024 * 1024,
        "analytical report exceeds byte bound"
    );
    let _: serde_json::Value = serde_json::from_slice(report)?;
    for bytes in [change_spec, state_input].into_iter().flatten() {
        ensure!(bytes.len() <= 1024 * 1024, "descriptor exceeds byte bound");
        let _: serde_json::Value = serde_json::from_slice(bytes)?;
    }
    fn directory(path: &Path) -> Result<()> {
        if !path.try_exists()? {
            fs::create_dir(path)?;
        }
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "local store directory must be real"
        );
        Ok(())
    }
    fn write(path: &Path, bytes: &[u8]) -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }
    let root = project
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("record project directory must already exist"))?;
    let base = root.join(".eplyx");
    directory(&base)?;
    for child in ["runs", "counterexamples", "reproductions", "cache", "sync"] {
        directory(&base.join(child))?;
    }
    let project_file = base.join("project.json");
    if !project_file.try_exists()? {
        let local_id = format!("local_{}", &sha256(root.to_string_lossy().as_bytes())[..20]);
        let name = root.file_name().unwrap_or_default().to_string_lossy();
        write(
            &project_file,
            &serde_json::to_vec_pretty(
                &serde_json::json!({"schema_version":1,"local_id":local_id,"name":name}),
            )?,
        )?;
    } else {
        let meta = fs::symlink_metadata(&project_file)?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "project record must be a regular file"
        );
    }
    let now = chrono::Utc::now();
    let digest = sha256(report);
    // create_dir gives append-only collision protection; a nanosecond timestamp
    // distinguishes repeated executions with byte-identical deterministic results.
    let run_id = format!("run_{}_{}", now.format("%Y%m%d%H%M%S%9f"), &digest[..12]);
    safe_id(&run_id, "run_")?;
    let run = base.join("runs").join(&run_id);
    fs::create_dir(&run)?;
    fs::create_dir(run.join("result"))?;
    fs::create_dir(run.join("input"))?;
    write(&run.join("result/report.json"), report)?;
    if let Some(bytes) = change_spec {
        write(&run.join("input/change.json"), bytes)?;
    }
    if let Some(bytes) = state_input {
        write(&run.join("input/state.json"), bytes)?;
    }
    let metadata = AnalyticalMetadata {
        schema_version: ANALYTICAL_METADATA_VERSION,
        run_id,
        timestamp: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        kind: kind.into(),
        eplyx_version: crate::build_info::VERSION.into(),
        engine_binary_sha256: sha256(&fs::read(std::env::current_exe()?)?),
        run_source: source,
        report_sha256: digest,
        change_spec_sha256: change_spec.map(sha256),
        state_input_sha256: state_input.map(sha256),
    };
    write(
        &run.join("metadata.json"),
        &serde_json::to_vec_pretty(&metadata)?,
    )?;
    Ok(metadata)
}
