//! Projects, bundles and runs on top of [`Storage`].
//!
//! The rule that shapes this module: a bundle is immutable and content
//! addressed, and activation is a pointer change. A corpus refresh installs
//! bundle B and points the project at it; bundle A is never edited, so a run
//! recorded against A stays reproducible.

use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use eplyx_engine::bundle::CiBundle;
use eplyx_engine::ci::CiReport;
use serde::{Deserialize, Serialize};

use crate::project::Project;
use crate::storage::Storage;

/// Where a run is in its life.
///
/// The two terminal failures are deliberately separate. `Failed` means the
/// engine reached a verdict and the verdict is no: a real gate result with a
/// real exit code, which is the product working. `ExecutionError` means this
/// service could not obtain a verdict at all, so there is no gate result to
/// report. Collapsing them would tell a caller that their upgrade was rejected
/// when in fact a disk filled up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Persisted and recoverable, waiting for execution capacity.
    Queued,
    /// Holding a permit; the engine is running.
    Running,
    Passed,
    Failed,
    ExecutionError,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Passed | Self::Failed | Self::ExecutionError)
    }
}

/// How a run ended.
///
/// A preflight abort is its own case because it is neither of the other two: a
/// malformed expectation file or an incompatible bundle is a genuine Eplyx
/// result carrying a genuine exit code, and it produces no report by design.
pub enum RunOutcome {
    Reported {
        report: Box<CiReport>,
        markdown: String,
    },
    PreflightAbort {
        exit_code: u8,
        detail: String,
    },
    ExecutionError {
        detail: String,
    },
}

/// What a run was, without any of what it used to run.
///
/// No token, no endpoint, no candidate bytes. Wall-clock time is allowed here
/// because this is hosted metadata: it sits beside the canonical report rather
/// than inside it, so it cannot reach a determinism hash.
///
/// Fields that only a finished run can know are optional, and are serialized as
/// `null` rather than omitted: a client polling a queued run should see the
/// same shape it will see later, with the answers still empty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunMetadata {
    pub run_id: String,
    pub project_id: String,
    pub status: RunStatus,
    /// Known at creation: the project's active bundle, resolved server-side.
    pub bundle_sha256: String,
    #[serde(default)]
    pub corpus_sha256: Option<String>,
    #[serde(default)]
    pub baseline_sha256: Option<String>,
    /// Hashed from the uploaded bytes before the run is accepted.
    pub candidate_sha256: String,
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub adapter_version: Option<u32>,
    #[serde(default)]
    pub semantic_schema_version: Option<u32>,
    #[serde(default)]
    pub record_count: Option<usize>,
    #[serde(default)]
    pub exit_code: Option<u8>,
    #[serde(default)]
    pub report_available: bool,
    /// Why there is no report, for the two terminal states that cannot have
    /// one. Never a place to restate a finding: findings live in the report.
    #[serde(default)]
    pub detail: Option<String>,
    pub created_at_unix_seconds: u64,
    #[serde(default)]
    pub started_at_unix_seconds: Option<u64>,
    #[serde(default)]
    pub completed_at_unix_seconds: Option<u64>,
}

pub fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

pub struct Registry {
    storage: Storage,
    /// A status change is read-modify-write over one file. Serializing them is
    /// what makes `begin_run` a compare-and-set rather than a race two workers
    /// could both win.
    transitions: Mutex<()>,
}

impl Registry {
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            transitions: Mutex::new(()),
        }
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    pub fn load_project(&self, id: &str) -> Result<Project> {
        let path = self.storage.project_path(id)?;
        if !self.storage.exists(&path) {
            bail!("no such project");
        }
        self.storage.read_project(&path)
    }

    pub fn save_project(&self, project: &Project) -> Result<()> {
        let path = self.storage.project_path(&project.id)?;
        self.storage.write_json(&path, project)
    }

    /// Install a verified bundle under its own content hash.
    ///
    /// Copied rather than referenced, and re-opened from its installed location
    /// so the copy itself is proved rather than assumed. Installing the same
    /// bundle twice is a no-op, not an overwrite.
    pub fn install_bundle(&self, source: &std::path::Path) -> Result<String> {
        let bundle = CiBundle::open(source).context("verifying the bundle to install")?;
        let sha256 = bundle.manifest().bundle_sha256.clone();
        let destination = self.storage.bundle_path(&sha256)?;
        if destination.exists() {
            // Already installed. Re-verify rather than trusting the directory
            // name, then leave it exactly as it is.
            CiBundle::open(&destination).context("verifying the installed bundle")?;
            return Ok(sha256);
        }
        copy_tree(source, &destination)?;
        let installed = CiBundle::open(&destination).context("verifying the installed copy")?;
        if installed.manifest().bundle_sha256 != sha256 {
            bail!("the installed copy does not match the bundle it came from");
        }
        Ok(sha256)
    }

    pub fn open_bundle(&self, sha256: &str) -> Result<CiBundle> {
        let path = self.storage.bundle_path(sha256)?;
        if !path.exists() {
            bail!("no such bundle");
        }
        CiBundle::open(&path)
    }

    /// Point a project at an installed bundle.
    ///
    /// Never automatic. A freshly built bundle sits installed but inactive
    /// until an operator selects it, because a corpus change moves what every
    /// pull request is measured against.
    pub fn activate_bundle(&self, project_id: &str, bundle_sha256: &str) -> Result<()> {
        let mut project = self.load_project(project_id)?;
        let bundle = self.open_bundle(bundle_sha256)?;

        if bundle.manifest().program_id != project.program_id {
            bail!(
                "bundle is for program {}, project {} protects {}",
                bundle.manifest().program_id,
                project.id,
                project.program_id
            );
        }
        // The same compatibility rules the gate applies, applied before a
        // bundle can ever be reached by a pull request.
        if let Some(adapter) = eplyx_engine::protocol::adapter_for(&project.program_id) {
            if adapter.adapter_version() != bundle.adapter().version {
                bail!(
                    "bundle was built under {} adapter v{}, this build speaks v{}",
                    bundle.adapter().name,
                    bundle.adapter().version,
                    adapter.adapter_version()
                );
            }
        } else {
            bail!(
                "no adapter compiled in for program {}; refusing to activate",
                project.program_id
            );
        }

        project.active_bundle_sha256 = Some(bundle_sha256.to_string());
        self.save_project(&project)
    }

    /// Persist a run before anything expensive happens to it.
    ///
    /// Once this returns, the run is recoverable by id: the client may vanish,
    /// the connection may be cut, and the work still has somewhere to land.
    pub fn create_run(&self, metadata: &RunMetadata) -> Result<()> {
        let path = self
            .storage
            .run_dir(&metadata.run_id)?
            .join("metadata.json");
        if self.storage.exists(&path) {
            bail!("run {} already exists", metadata.run_id);
        }
        self.storage.write_json(&path, metadata)
    }

    /// Claim a queued run. Compare-and-set: exactly one caller gets `true`.
    ///
    /// Anything already running or terminal answers `false` rather than an
    /// error, because a second worker finding the run taken is an ordinary
    /// outcome, not a fault.
    pub fn begin_run(&self, run_id: &str) -> Result<bool> {
        let _guard = self
            .transitions
            .lock()
            .unwrap_or_else(|held| held.into_inner());
        let mut metadata = self.load_run(run_id)?;
        if metadata.status != RunStatus::Queued {
            return Ok(false);
        }
        metadata.status = RunStatus::Running;
        metadata.started_at_unix_seconds = Some(now_unix_seconds());
        self.write_run(&metadata)?;
        Ok(true)
    }

    /// Record how a run ended.
    ///
    /// The report is written before the metadata that advertises it, so
    /// `report_available` is never true ahead of the bytes. A terminal run is
    /// never reopened: that is what keeps a completed result stable for anyone
    /// who fetches it later.
    pub fn finish_run(&self, run_id: &str, outcome: RunOutcome) -> Result<()> {
        let _guard = self
            .transitions
            .lock()
            .unwrap_or_else(|held| held.into_inner());
        let mut metadata = self.load_run(run_id)?;
        if metadata.status.is_terminal() {
            bail!("run {run_id} is already {:?}", metadata.status);
        }
        match outcome {
            RunOutcome::Reported { report, markdown } => {
                self.write_report(run_id, &report, &markdown)?;
                metadata.status = if report.summary.passed {
                    RunStatus::Passed
                } else {
                    RunStatus::Failed
                };
                metadata.exit_code = Some(report.summary.exit_code);
                metadata.report_available = true;
                metadata.corpus_sha256 = Some(report.bundle.corpus_sha256.clone());
                metadata.baseline_sha256 = Some(report.bundle.baseline_sha256.clone());
                metadata.adapter = Some(report.bundle.adapter.clone());
                metadata.adapter_version = Some(report.bundle.adapter_version);
                metadata.semantic_schema_version = Some(report.bundle.semantic_schema_version);
                metadata.record_count = Some(report.bundle.record_count);
            }
            RunOutcome::PreflightAbort { exit_code, detail } => {
                // A verdict was reached. It simply arrived before there was a
                // report to put it in.
                metadata.status = RunStatus::Failed;
                metadata.exit_code = Some(exit_code);
                metadata.report_available = false;
                metadata.detail = Some(detail);
            }
            RunOutcome::ExecutionError { detail } => {
                metadata.status = RunStatus::ExecutionError;
                metadata.exit_code = None;
                metadata.report_available = false;
                metadata.detail = Some(detail);
            }
        }
        metadata.completed_at_unix_seconds = Some(now_unix_seconds());
        self.write_run(&metadata)
    }

    /// The canonical report, byte for byte what the engine produced - down to
    /// the trailing newline, so `report.json` from this endpoint and the output
    /// of a local `eplyx ci check --format json` are the same file.
    fn write_report(&self, run_id: &str, report: &CiReport, markdown: &str) -> Result<()> {
        let directory = self.storage.run_dir(run_id)?;
        let mut canonical = serde_json::to_vec_pretty(report)?;
        canonical.push(b'\n');
        self.storage
            .write_bytes(&directory.join("report.json"), &canonical)?;
        self.storage
            .write_bytes(&directory.join("report.md"), markdown.as_bytes())
    }

    fn write_run(&self, metadata: &RunMetadata) -> Result<()> {
        let path = self
            .storage
            .run_dir(&metadata.run_id)?
            .join("metadata.json");
        self.storage.write_json(&path, metadata)
    }

    pub fn load_run(&self, run_id: &str) -> Result<RunMetadata> {
        let path = self.storage.run_dir(run_id)?.join("metadata.json");
        if !self.storage.exists(&path) {
            bail!("no such run");
        }
        self.storage.read_json(&path)
    }

    /// Resolve runs that a restart interrupted.
    ///
    /// The task queue is in-process, so a restart loses whatever it was
    /// holding. The honest thing is to say so: a run left queued or running has
    /// no worker behind it any more and would otherwise poll forever. Resuming
    /// them is a larger design than a pilot needs, and pretending they are
    /// still alive is the one option that is simply wrong.
    pub fn recover_interrupted_runs(&self) -> Result<Vec<String>> {
        let mut recovered = Vec::new();
        for run_id in self.list_run_ids()? {
            let Ok(metadata) = self.load_run(&run_id) else {
                continue;
            };
            if metadata.status.is_terminal() {
                continue;
            }
            self.finish_run(
                &run_id,
                RunOutcome::ExecutionError {
                    detail: "Run interrupted by server restart; resubmit the check.".to_string(),
                },
            )?;
            self.clear_run_work(&run_id).ok();
            recovered.push(run_id);
        }
        Ok(recovered)
    }

    pub fn list_run_ids(&self) -> Result<Vec<String>> {
        let directory = self.storage.runs_root();
        let mut ids = Vec::new();
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return Ok(ids);
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                if crate::storage::valid_id(name) {
                    ids.push(name.to_string());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Where a run's uploaded inputs wait for their worker.
    ///
    /// Not a request-scoped temporary directory: the handler returns long
    /// before the worker reads these, so a `TempDir` dropped with the response
    /// would delete the candidate out from under the run that was accepted.
    pub fn run_work_dir(&self, run_id: &str) -> Result<std::path::PathBuf> {
        Ok(self.storage.run_dir(run_id)?.join("work"))
    }

    /// Drop the uploaded inputs. The report and metadata stay.
    pub fn clear_run_work(&self, run_id: &str) -> Result<()> {
        let work = self.run_work_dir(run_id)?;
        if work.exists() {
            std::fs::remove_dir_all(&work)
                .with_context(|| format!("clearing {}", work.display()))?;
        }
        Ok(())
    }

    pub fn load_run_artifact(&self, run_id: &str, name: &str) -> Result<Vec<u8>> {
        if !matches!(name, "report.json" | "report.md") {
            bail!("no such artifact");
        }
        let path = self.storage.run_dir(run_id)?.join(name);
        if !self.storage.exists(&path) {
            bail!("no such artifact");
        }
        self.storage.read_bytes(&path)
    }
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
