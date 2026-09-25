//! Projects, bundles and runs on top of [`Storage`].
//!
//! The rule that shapes this module: a bundle is immutable and content
//! addressed, and activation is a pointer change. A corpus refresh installs
//! bundle B and points the project at it; bundle A is never edited, so a run
//! recorded against A stays reproducible.

use std::sync::Mutex;

use anyhow::{bail, ensure, Context, Result};
use eplyx_engine::bundle::CiBundle;
use eplyx_engine::change::{ChangeKind, ChangeSpec, Delivery};
use eplyx_engine::ci::CiReport;
use eplyx_engine::governance::GovernanceBinding;
use serde::{Deserialize, Serialize};

use crate::artifacts::{ArtifactRef, ArtifactStore};
use crate::project::{ActiveBundle, AdapterId, Project, ProjectToken};
use crate::storage::Storage;

/// A hard ceiling for any executable the artefact store will hold, beneath
/// which the configured upload limit sits. Solana programs are far smaller.
pub const MAX_STORED_PROGRAM_BYTES: usize = 32 * 1024 * 1024;

/// How many times one run may be interrupted mid-execution before recovery
/// stops retrying it. A run that has taken the process down three times is
/// more likely to be the cause than the victim, and re-running it forever would
/// turn one bad input into an outage.
pub const MAX_EXECUTION_ATTEMPTS: usize = 3;

/// A project's record of one immutable bundle.
///
/// The bytes live once in the content-addressed store; this says which project
/// registered them, when, and what they claimed to be. Nothing here is
/// editable: a bundle is not amended, a new one is uploaded.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectBundle {
    pub bundle_id: String,
    pub project_id: String,
    pub bundle_sha256: String,
    pub baseline_sha256: String,
    pub program_id: String,
    pub adapter_id: AdapterId,
    pub semantic_schema_version: u32,
    pub record_count: usize,
    #[serde(default)]
    pub source_filename: Option<String>,
    pub created_at_unix_seconds: u64,
}

/// Whether a bundle may ever be measured against by this project.
///
/// Three questions, all answered from the bundle's own verified manifest
/// rather than from anything a caller said: is it for this program, was it
/// built under the vocabulary this project declares, and does this build still
/// speak that vocabulary.
///
/// The adapter comparison reads both sides. Checking only the engine's version
/// left a program the engine has no adapter for unable to onboard at all, while
/// a bundle that declares `none@0` for such a program is telling the exact
/// truth — every check against it reports `no_semantic_coverage` and fails,
/// which is the engine saying it did not look rather than saying nothing is
/// wrong.
fn check_bundle_matches_project(project: &Project, bundle: &CiBundle) -> Result<()> {
    let manifest = bundle.manifest();
    if manifest.program_id != project.program_id {
        bail!(
            "bundle is for program {}, project {} protects {}",
            manifest.program_id,
            project.project_id,
            project.program_id
        );
    }
    let declared = AdapterId {
        name: bundle.adapter().name.clone(),
        version: bundle.adapter().version,
    };
    if declared != project.adapter_id {
        bail!(
            "bundle was built under adapter {declared}, project declares {}",
            project.adapter_id
        );
    }
    let engine = AdapterId::for_program(&project.program_id);
    if declared != engine {
        bail!("bundle was built under adapter {declared}, this build speaks {engine}");
    }
    let schema = eplyx_engine::semantics::SEMANTIC_SCHEMA_VERSION;
    if manifest.semantic_schema_version != schema {
        bail!(
            "bundle names subjects under semantic schema v{}, this build speaks v{schema}",
            manifest.semantic_schema_version
        );
    }
    Ok(())
}

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

/// How a run's change spec came to exist. Provenance, never identity: the same
/// spec reaches the same `change_spec_id` by either route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeOrigin {
    /// Only candidate bytes were uploaded. The server derived the minimal
    /// program upgrade of the pinned bundle's program to exactly those bytes,
    /// which is the spec `eplyx ci check --candidate` derives.
    DerivedFromCandidate,
    /// The caller submitted an explicit spec; the uploaded bytes only
    /// satisfied its artefact reference.
    Submitted,
}

/// The proposal a run evaluates, as the registry indexes it.
///
/// The index fields only. The complete canonical spec is stored once beside
/// the run (`change_spec.json`) and re-verified against these on every read,
/// so there is one copy of the proposal and one copy of its identity, and a
/// disagreement between them is detectable rather than silently resolved.
///
/// `change_spec_id` — not `candidate_sha256` — is the proposal's identity: the
/// same bytes proposed for two programs are two proposals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunChange {
    pub change_spec_id: String,
    pub kind: ChangeKind,
    pub target_program_id: String,
    pub candidate_sha256: String,
    pub candidate_len: u64,
    /// Display only. Outside identity, as in the spec itself.
    #[serde(default)]
    pub label: Option<String>,
    pub origin: ChangeOrigin,
    /// The governance proposal the spec is bound to, when it names one. Part
    /// of the spec's identity, so already inside `change_spec_id`; indexed
    /// here so history can say "Squads #42" without reading every spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
}

impl RunChange {
    pub fn of(spec: &ChangeSpec, origin: ChangeOrigin) -> Result<Self> {
        Ok(Self {
            change_spec_id: spec.id()?,
            kind: spec.kind(),
            target_program_id: spec.target_program_id().to_string(),
            candidate_sha256: spec.candidate().sha256.clone(),
            candidate_len: spec.candidate().len,
            label: spec.metadata.label.clone(),
            origin,
            delivery: spec.delivery().cloned(),
        })
    }

    /// The hard consistency rule between the registry and the engine:
    ///
    /// ```text
    /// registry change_spec_id == stored ChangeSpec id == report.change.change_spec_id
    /// ```
    ///
    /// A report about a different proposal is not a verdict about this one,
    /// however it came to be written, and is never recorded as if it were.
    pub fn verify_report(&self, report: &CiReport) -> Result<()> {
        let change = report.change.as_ref().context(
            "the report names no change, so it is not a verdict about this run's proposal",
        )?;
        ensure!(
            change.change_spec_id == self.change_spec_id,
            "the report is about change {} but the run was accepted for change {}",
            change.change_spec_id,
            self.change_spec_id
        );
        ensure!(
            change.kind == self.kind
                && change.delivery == self.delivery
                && change.target_program_id == self.target_program_id
                && change.candidate_sha256 == self.candidate_sha256
                && report.candidate.sha256 == self.candidate_sha256
                && report.candidate.len == self.candidate_len,
            "the report's change fields disagree with the change the run was accepted for"
        );
        Ok(())
    }
}

/// One governance check, as the registry lists it. The evidence is the
/// sealed binding it names; this record adds only when it was taken.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceCheck {
    pub check_id: String,
    pub project_id: String,
    pub change_spec_id: String,
    pub binding_id: String,
    pub checked_at_unix_seconds: u64,
}

/// How one execution attempt of a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptEnd {
    /// The attempt reached an outcome and the registry recorded it.
    Completed,
    /// The process died while the attempt held the run. Nothing it produced
    /// was recorded, and the run went back to the queue (or, past the retry
    /// limit, to `execution_error`).
    Interrupted,
    /// The process died after the attempt wrote a complete, verified report
    /// but before the registry recorded it. Recovery recorded that report
    /// rather than recomputing it.
    FinalizedOnRecovery,
}

/// One execution of a run. A retry after process death is another attempt of
/// the *same* run — same id, same change, same pinned bundle, same artefact —
/// never a new run a user has to find.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAttempt {
    pub attempt: u32,
    pub started_at_unix_seconds: u64,
    #[serde(default)]
    pub ended_at_unix_seconds: Option<u64>,
    #[serde(default)]
    pub end: Option<AttemptEnd>,
}

/// What startup reconciliation did, run by run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Durable runs back in (or still in) the queue, oldest first. The caller
    /// starts a worker for each.
    pub requeued: Vec<String>,
    /// Runs whose complete report was already written; recorded, not re-run.
    pub finalized: Vec<String>,
    /// Runs that cannot be resumed honestly, now `execution_error`.
    pub failed: Vec<String>,
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
    ///
    /// Pinned here, and never resolved again. A bundle activated after this run
    /// was accepted belongs to the next run, not to this one — otherwise a
    /// result would silently describe a comparison nobody asked for.
    pub bundle_sha256: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub corpus_sha256: Option<String>,
    #[serde(default)]
    pub baseline_sha256: Option<String>,
    /// Hashed from the uploaded bytes before the run is accepted. Kept for
    /// every client written before change identity; `change` is what names
    /// the proposal.
    pub candidate_sha256: String,
    /// The proposal this run evaluates, fixed at creation and bound to the
    /// pinned bundle before the run was accepted.
    ///
    /// `None` only on runs recorded before change identity existed. Those are
    /// legacy runs: their identity is not reconstructed, because the candidate
    /// length it would need was never recorded.
    #[serde(default)]
    pub change: Option<RunChange>,
    /// The candidate by durable identity: an immutable object in the service's
    /// content-addressed store, persisted before this run was accepted. It must
    /// equal the spec's `candidate`, and the bytes a worker reads must hash to
    /// it. `None` on runs accepted before durable artefacts, whose candidate
    /// lived only in their work directory.
    #[serde(default)]
    pub candidate_artifact: Option<ArtifactRef>,
    /// SHA-256 of the run's `expected-changes.toml`, stored beside it, when one
    /// was supplied. Pinned so a resumed run reads the declarations it was
    /// accepted with.
    #[serde(default)]
    pub expectations_sha256: Option<String>,
    /// Every execution of this run. More than one means infrastructure retried
    /// it; the run id, change and inputs never changed.
    #[serde(default)]
    pub attempts: Vec<ExecutionAttempt>,
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
    artifacts: ArtifactStore,
    /// A status change is read-modify-write over one file. Serializing them is
    /// what makes `begin_run` a compare-and-set rather than a race two workers
    /// could both win.
    transitions: Mutex<()>,
}

impl Registry {
    pub fn new(storage: Storage) -> Self {
        let artifacts = ArtifactStore::open(storage.artifacts_root(), MAX_STORED_PROGRAM_BYTES)
            .expect("the artifact store lives inside the data directory Storage just opened");
        Self {
            storage,
            artifacts,
            transitions: Mutex::new(()),
        }
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    pub fn artifacts(&self) -> &ArtifactStore {
        &self.artifacts
    }

    // ----------------------------------------------------------- artifacts

    /// Record that a project has supplied an artefact. Which artefacts a
    /// project may name without re-uploading is exactly this set: the store is
    /// global and deduplicated, but knowing a hash must never let one project
    /// run, or learn about, bytes only another project uploaded.
    pub fn index_project_artifact(&self, project_id: &str, sha256: &str) -> Result<()> {
        let path = self.storage.project_artifact_marker(project_id, sha256)?;
        self.storage.write_bytes(&path, b"")
    }

    pub fn project_holds_artifact(&self, project_id: &str, sha256: &str) -> Result<bool> {
        Ok(self
            .storage
            .exists(&self.storage.project_artifact_marker(project_id, sha256)?))
    }

    /// Persist a run's declarations beside it, returning the hash they are
    /// pinned by.
    pub fn save_expectations(&self, run_id: &str, bytes: &[u8]) -> Result<String> {
        let path = self.storage.run_dir(run_id)?.join("expected-changes.toml");
        self.storage.write_bytes(&path, bytes)?;
        Ok(eplyx_engine::replay::hash_bytes(bytes))
    }

    /// A run's declarations, proved to be the ones it was accepted with.
    pub fn load_expectations(&self, metadata: &RunMetadata) -> Result<Option<Vec<u8>>> {
        let Some(expected) = &metadata.expectations_sha256 else {
            return Ok(None);
        };
        let path = self
            .storage
            .run_dir(&metadata.run_id)?
            .join("expected-changes.toml");
        let bytes = self
            .storage
            .read_bytes(&path)
            .context("the run's expected changes are missing")?;
        ensure!(
            &eplyx_engine::replay::hash_bytes(&bytes) == expected,
            "the run's expected changes no longer match the hash they were accepted with"
        );
        Ok(Some(bytes))
    }

    // ------------------------------------------------------------ projects

    pub fn load_project(&self, project_id: &str) -> Result<Project> {
        let path = self.storage.project_path(project_id)?;
        if !self.storage.exists(&path) {
            bail!("no such project");
        }
        self.storage.read_json(&path)
    }

    pub fn save_project(&self, project: &Project) -> Result<()> {
        let path = self.storage.project_path(&project.project_id)?;
        self.storage.write_json(&path, project)
    }

    /// Create a project, refusing to write over one that already exists.
    pub fn create_project(&self, project: &Project) -> Result<()> {
        let path = self.storage.project_path(&project.project_id)?;
        if self.storage.exists(&path) {
            bail!("project {} already exists", project.project_id);
        }
        self.save_project(project)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut projects: Vec<Project> = self
            .child_ids(&self.storage.projects_root())?
            .into_iter()
            .filter_map(|id| self.load_project(&id).ok())
            .collect();
        // Newest first: the id carries its own minting time.
        projects.sort_by(|a, b| b.project_id.cmp(&a.project_id));
        Ok(projects)
    }

    // -------------------------------------------------------------- tokens

    /// Issue a token. The secret is returned to the caller and never stored.
    pub fn create_token(&self, token: &ProjectToken) -> Result<()> {
        let path = self
            .storage
            .project_token_path(&token.project_id, &token.token_id)?;
        if self.storage.exists(&path) {
            bail!("token {} already exists", token.token_id);
        }
        self.storage.write_json(&path, token)
    }

    pub fn list_tokens(&self, project_id: &str) -> Result<Vec<ProjectToken>> {
        let directory = self.storage.project_tokens_dir(project_id)?;
        let mut tokens: Vec<ProjectToken> = self
            .child_ids(&directory)?
            .into_iter()
            .filter_map(|id| {
                let path = self.storage.project_token_path(project_id, &id).ok()?;
                self.storage.read_json(&path).ok()
            })
            .collect();
        tokens.sort_by(|a, b| b.token_id.cmp(&a.token_id));
        Ok(tokens)
    }

    /// Find the token a secret belongs to, within one project.
    ///
    /// Every live token is tried, because a project may hold several and the
    /// caller sends only the secret. A revoked token is skipped rather than
    /// matched and then rejected: there is no state in which it authenticates.
    pub fn authenticate_token(&self, project_id: &str, secret: &str) -> Result<ProjectToken> {
        let matched = self
            .list_tokens(project_id)?
            .into_iter()
            .find(|token| !token.is_revoked() && token.verifier.verifies(secret));
        matched.context("no live token matches")
    }

    pub fn revoke_token(&self, project_id: &str, token_id: &str) -> Result<ProjectToken> {
        let _guard = self
            .transitions
            .lock()
            .unwrap_or_else(|held| held.into_inner());
        let path = self.storage.project_token_path(project_id, token_id)?;
        if !self.storage.exists(&path) {
            bail!("no such token");
        }
        let mut token: ProjectToken = self.storage.read_json(&path)?;
        if token.revoked_at_unix_seconds.is_none() {
            token.revoked_at_unix_seconds = Some(crate::project::now_unix_seconds());
            self.storage.write_json(&path, &token)?;
        }
        Ok(token)
    }

    /// Record that a token was used. Best effort: a failure here must never
    /// turn a successful request into a rejected one.
    pub fn note_token_use(&self, token: &ProjectToken) {
        let Ok(path) = self
            .storage
            .project_token_path(&token.project_id, &token.token_id)
        else {
            return;
        };
        let mut updated = token.clone();
        updated.last_used_at_unix_seconds = Some(crate::project::now_unix_seconds());
        let _ = self.storage.write_json(&path, &updated);
    }

    // ------------------------------------------------------------- bundles

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

    /// Register a verified bundle as belonging to a project.
    ///
    /// The content lives once, addressed by its hash; this is the project's
    /// record of it. Registering the same content twice returns the record
    /// that already exists rather than minting a second identity for the same
    /// bytes.
    pub fn register_bundle(
        &self,
        project: &Project,
        source: &std::path::Path,
        source_filename: Option<&str>,
    ) -> Result<ProjectBundle> {
        let bundle = CiBundle::open(source).context("verifying the uploaded bundle")?;
        check_bundle_matches_project(project, &bundle)?;
        let sha256 = self.install_bundle(source)?;

        if let Some(existing) = self
            .list_bundles(&project.project_id)?
            .into_iter()
            .find(|record| record.bundle_sha256 == sha256)
        {
            return Ok(existing);
        }

        let manifest = bundle.manifest();
        let record = ProjectBundle {
            bundle_id: crate::ids::bundle(),
            project_id: project.project_id.clone(),
            bundle_sha256: sha256,
            baseline_sha256: manifest.baseline_program_sha256.clone(),
            program_id: manifest.program_id.clone(),
            adapter_id: AdapterId {
                name: bundle.adapter().name.clone(),
                version: bundle.adapter().version,
            },
            semantic_schema_version: manifest.semantic_schema_version,
            record_count: manifest.record_count,
            source_filename: source_filename.map(str::to_string),
            created_at_unix_seconds: crate::project::now_unix_seconds(),
        };
        let path = self
            .storage
            .project_bundle_path(&project.project_id, &record.bundle_id)?;
        self.storage.write_json(&path, &record)?;
        Ok(record)
    }

    pub fn load_bundle_record(&self, project_id: &str, bundle_id: &str) -> Result<ProjectBundle> {
        let path = self.storage.project_bundle_path(project_id, bundle_id)?;
        if !self.storage.exists(&path) {
            bail!("no such bundle");
        }
        self.storage.read_json(&path)
    }

    pub fn list_bundles(&self, project_id: &str) -> Result<Vec<ProjectBundle>> {
        let directory = self.storage.project_bundles_dir(project_id)?;
        let mut bundles: Vec<ProjectBundle> = self
            .child_ids(&directory)?
            .into_iter()
            .filter_map(|id| self.load_bundle_record(project_id, &id).ok())
            .collect();
        bundles.sort_by(|a, b| b.bundle_id.cmp(&a.bundle_id));
        Ok(bundles)
    }

    /// Point a project at one of its registered bundles.
    ///
    /// Never automatic. A freshly uploaded bundle sits registered but inactive
    /// until someone selects it, because a corpus change moves what every pull
    /// request is measured against. The previous bundle is left exactly where
    /// it is: activation moves a pointer and destroys nothing.
    pub fn activate_bundle(&self, project_id: &str, bundle_id: &str) -> Result<Project> {
        let _guard = self
            .transitions
            .lock()
            .unwrap_or_else(|held| held.into_inner());
        let mut project = self.load_project(project_id)?;
        let record = self.load_bundle_record(project_id, bundle_id)?;
        // Re-opened from storage rather than trusted from the record: the
        // question at activation is whether the bytes are still there and still
        // verify, not what we wrote down when they arrived.
        let bundle = self.open_bundle(&record.bundle_sha256)?;
        check_bundle_matches_project(&project, &bundle)?;

        project.active_bundle = Some(ActiveBundle {
            bundle_id: record.bundle_id.clone(),
            bundle_sha256: record.bundle_sha256.clone(),
            activated_at_unix_seconds: crate::project::now_unix_seconds(),
        });
        project.refresh_status();
        project.touch();
        self.save_project(&project)?;
        Ok(project)
    }

    // ---------------------------------------------------------- run index

    /// Note that a run belongs to a project, so history is a listing rather
    /// than a scan of every run the service has ever executed.
    pub fn index_run(&self, project_id: &str, run_id: &str) -> Result<()> {
        let path = self.storage.project_run_marker(project_id, run_id)?;
        self.storage.write_bytes(&path, b"")
    }

    /// Run ids owned by a project, newest first.
    ///
    /// The id begins with its own minting time, so ordering is the listing
    /// reversed and paging is "everything before this id".
    pub fn project_run_ids(&self, project_id: &str) -> Result<Vec<String>> {
        let directory = self.storage.project_runs_dir(project_id)?;
        let mut ids = self.child_ids(&directory)?;
        ids.sort_by(|a, b| b.cmp(a));
        Ok(ids)
    }

    /// Note that a run evaluates a given proposal, so every analysis of one
    /// `change_spec_id` is a listing rather than a scan. This is what a later
    /// governance binding points at: an existing analysis, by the identity of
    /// the proposal being signed.
    pub fn index_change(&self, project_id: &str, change_spec_id: &str, run_id: &str) -> Result<()> {
        let path = self
            .storage
            .project_change_run_marker(project_id, change_spec_id, run_id)?;
        self.storage.write_bytes(&path, b"")
    }

    /// Runs of one project that evaluate one proposal, newest first.
    pub fn change_run_ids(&self, project_id: &str, change_spec_id: &str) -> Result<Vec<String>> {
        let directory = self
            .storage
            .project_change_runs_dir(project_id, change_spec_id)?;
        let mut ids = self.child_ids(&directory)?;
        ids.sort_by(|a, b| b.cmp(a));
        Ok(ids)
    }

    /// Persist a run's canonical spec: the document form, with its identity
    /// committed beside the fields. Never the candidate bytes; those are
    /// content-addressed separately and only for as long as the run needs them.
    pub fn save_change_spec(&self, run_id: &str, spec: &ChangeSpec) -> Result<()> {
        let path = self.storage.run_dir(run_id)?.join("change_spec.json");
        if self.storage.exists(&path) {
            bail!("run {run_id} already has a change spec");
        }
        let mut document = spec.to_document()?.into_bytes();
        document.push(b'\n');
        self.storage.write_bytes(&path, &document)
    }

    /// Read a run's spec back, trusting nothing that was written down.
    ///
    /// The stored document's own id is recomputed from its fields (`parse`),
    /// and the result must be exactly the change the registry indexed the run
    /// under. Either disagreement means the stored proposal is not the one the
    /// run was accepted for, and the read fails.
    pub fn load_change_spec(&self, run_id: &str, expected: &RunChange) -> Result<ChangeSpec> {
        let path = self.storage.run_dir(run_id)?.join("change_spec.json");
        if !self.storage.exists(&path) {
            bail!("run {run_id} has no stored change spec");
        }
        let bytes = self.storage.read_bytes(&path)?;
        let spec = ChangeSpec::parse(&bytes).context("the stored change spec does not verify")?;
        ensure!(
            spec.change_spec_id.is_some(),
            "the stored change spec does not commit to its identity"
        );
        let stored = RunChange::of(&spec, expected.origin)?;
        ensure!(
            stored.change_spec_id == expected.change_spec_id,
            "the stored change spec identifies {} but the run was accepted for {}",
            stored.change_spec_id,
            expected.change_spec_id
        );
        ensure!(
            &stored == expected,
            "the stored change spec disagrees with the run's indexed change"
        );
        Ok(spec)
    }

    // ------------------------------------------------------------ governance

    /// Record one governance check: the sealed binding, content-addressed by
    /// its own id, then a check record naming it.
    ///
    /// Indexed under the change it was asked about *and* the governance-bound
    /// change it derived, when those differ. Either alone leaves a hole: a
    /// re-check of a bound change that finds a different or unsupported
    /// message derives another bound id, and filed only there it would leave
    /// the bound change showing its last match. The record is written last in
    /// each index, so a listed check always has its evidence.
    pub fn record_governance_check(
        &self,
        project_id: &str,
        binding: &GovernanceBinding,
    ) -> Result<Vec<GovernanceCheck>> {
        let binding_id = binding.id()?;
        ensure!(
            binding.binding_id.as_deref() == Some(binding_id.as_str()),
            "refusing to store a governance binding that is not sealed"
        );
        let mut indexes = vec![binding.analysed_change_spec_id.clone()];
        if let Some(bound) = &binding.bound_change_spec_id {
            if bound != &binding.analysed_change_spec_id {
                indexes.push(bound.clone());
            }
        }
        let check_id = crate::ids::governance_check();
        let checked_at = now_unix_seconds();
        let mut document = binding.to_document()?.into_bytes();
        document.push(b'\n');
        let mut checks = Vec::new();
        for index in indexes {
            let directory = self.storage.project_governance_dir(project_id, &index)?;
            let evidence = directory
                .join("bindings")
                .join(format!("{binding_id}.json"));
            if !self.storage.exists(&evidence) {
                self.storage.write_bytes(&evidence, &document)?;
            }
            let check = GovernanceCheck {
                check_id: check_id.clone(),
                project_id: project_id.to_string(),
                change_spec_id: index,
                binding_id: binding_id.clone(),
                checked_at_unix_seconds: checked_at,
            };
            self.storage.write_json(
                &directory.join("checks").join(format!("{check_id}.json")),
                &check,
            )?;
            checks.push(check);
        }
        Ok(checks)
    }

    /// The most recent checks of one change, newest first, each with its
    /// evidence re-verified. Stored evidence that no longer verifies fails
    /// the read rather than being shown: a tampered "matched" is never served.
    pub fn governance_checks(
        &self,
        project_id: &str,
        change_spec_id: &str,
        limit: usize,
    ) -> Result<Vec<(GovernanceCheck, GovernanceBinding)>> {
        let directory = self
            .storage
            .project_governance_dir(project_id, change_spec_id)?;
        let mut ids = self.child_ids(&directory.join("checks"))?;
        ids.sort_by(|a, b| b.cmp(a));
        ids.truncate(limit);
        let mut checks = Vec::new();
        for id in ids {
            let check: GovernanceCheck = self
                .storage
                .read_json(&directory.join("checks").join(format!("{id}.json")))?;
            ensure!(
                check.check_id == id
                    && check.project_id == project_id
                    && check.change_spec_id == change_spec_id,
                "governance check {id} does not describe this change"
            );
            let bytes = self.storage.read_bytes(
                &directory
                    .join("bindings")
                    .join(format!("{}.json", check.binding_id)),
            )?;
            let binding = GovernanceBinding::parse(&bytes).with_context(|| {
                format!("governance evidence {} does not verify", check.binding_id)
            })?;
            ensure!(
                binding.binding_id.as_deref() == Some(check.binding_id.as_str()),
                "governance evidence is stored under another id"
            );
            ensure!(
                binding.analysed_change_spec_id == change_spec_id
                    || binding.bound_change_spec_id.as_deref() == Some(change_spec_id),
                "governance evidence {} is not about change {change_spec_id}",
                check.binding_id
            );
            checks.push((check, binding));
        }
        Ok(checks)
    }

    /// Keep a governance-bound spec a check derived, so it can be found by
    /// its id and analysed without the caller re-sending it.
    pub fn save_governance_spec(&self, project_id: &str, spec: &ChangeSpec) -> Result<()> {
        let id = spec.id()?;
        let path = self
            .storage
            .project_governance_dir(project_id, &id)?
            .join("change_spec.json");
        if let Some(existing) = self.load_governance_spec(project_id, &id)? {
            ensure!(
                existing.id()? == id,
                "a different spec is stored under {id}"
            );
            return Ok(());
        }
        let mut document = spec.to_document()?.into_bytes();
        document.push(b'\n');
        self.storage.write_bytes(&path, &document)
    }

    pub fn load_governance_spec(
        &self,
        project_id: &str,
        change_spec_id: &str,
    ) -> Result<Option<ChangeSpec>> {
        let path = self
            .storage
            .project_governance_dir(project_id, change_spec_id)?
            .join("change_spec.json");
        if !self.storage.exists(&path) {
            return Ok(None);
        }
        let spec = ChangeSpec::parse(&self.storage.read_bytes(&path)?)
            .context("a stored governance-bound spec does not verify")?;
        ensure!(
            spec.id()? == change_spec_id,
            "the spec stored under {change_spec_id} identifies {}",
            spec.id()?
        );
        Ok(Some(spec))
    }

    /// Valid identifiers naming entries directly inside a directory.
    fn child_ids(&self, directory: &std::path::Path) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Ok(ids);
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let name = name.strip_suffix(".json").unwrap_or(name);
            if crate::storage::valid_id(name) {
                ids.push(name.to_string());
            }
        }
        Ok(ids)
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
        let now = now_unix_seconds();
        metadata.status = RunStatus::Running;
        metadata.started_at_unix_seconds.get_or_insert(now);
        metadata.attempts.push(ExecutionAttempt {
            attempt: metadata.attempts.len() as u32 + 1,
            started_at_unix_seconds: now,
            ended_at_unix_seconds: None,
            end: None,
        });
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
        // The one place a run becomes terminal, so the one place the identity
        // rule is enforced. A report about some other proposal is recorded as
        // this service failing to obtain a verdict, never as a verdict.
        let outcome = match outcome {
            RunOutcome::Reported { report, markdown } => match verify_outcome(&metadata, &report) {
                Err(error) => RunOutcome::ExecutionError {
                    detail: format!("change identity check failed; no verdict recorded: {error:#}"),
                },
                Ok(()) => RunOutcome::Reported { report, markdown },
            },
            other => other,
        };
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
        let now = now_unix_seconds();
        close_attempt(&mut metadata, AttemptEnd::Completed, now);
        metadata.completed_at_unix_seconds = Some(now);
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

    /// Reconcile every run a previous process left unfinished.
    ///
    /// Runs only at startup, under the data directory's exclusive lock, so no
    /// worker of this or any other process can hold a run while it happens.
    ///
    /// ```text
    /// terminal                         untouched (its scratch cache is cleared)
    /// no durable inputs (pre-P2)       execution_error: never invented
    /// queued                           stays queued            → requeued
    /// running, complete valid report   recorded as it stands   → finalized
    /// running, anything less           attempt → interrupted,
    ///                                  status → queued         → requeued
    ///                                  (past MAX_EXECUTION_ATTEMPTS: execution_error)
    /// ```
    ///
    /// Re-executing is safe because nothing an execution reads is mutated by
    /// it: the bundle, the stored spec, the artefact and the declarations are
    /// all immutable and hash-verified, and the engine is deterministic. The
    /// only state an execution writes is its report and the run's terminal
    /// record, and those are written once, by `finish_run`.
    pub fn recover_runs(&self) -> Result<Recovery> {
        let mut recovery = Recovery::default();
        for run_id in self.list_run_ids()? {
            let Ok(mut metadata) = self.load_run(&run_id) else {
                continue;
            };
            if metadata.status.is_terminal() {
                self.clear_run_work(&run_id).ok();
                continue;
            }
            if metadata.change.is_none() || metadata.candidate_artifact.is_none() {
                self.finish_run(
                    &run_id,
                    RunOutcome::ExecutionError {
                        detail: "Run interrupted by server restart. It was accepted before \
                                 candidates were stored durably, so its inputs cannot be \
                                 recovered; resubmit the check."
                            .to_string(),
                    },
                )?;
                self.clear_run_work(&run_id).ok();
                recovery.failed.push(run_id);
                continue;
            }
            if metadata.status == RunStatus::Running {
                if let Some(report) = self.completed_report(&metadata) {
                    // The attempt finished; only its recording was lost.
                    let markdown = eplyx_engine::ci_markdown::render(&report);
                    self.finish_run(
                        &run_id,
                        RunOutcome::Reported {
                            report: Box::new(report),
                            markdown,
                        },
                    )?;
                    let mut recorded = self.load_run(&run_id)?;
                    if let Some(last) = recorded.attempts.last_mut() {
                        last.end = Some(AttemptEnd::FinalizedOnRecovery);
                    }
                    self.write_run(&recorded)?;
                    recovery.finalized.push(run_id);
                    continue;
                }
                let now = now_unix_seconds();
                close_attempt(&mut metadata, AttemptEnd::Interrupted, now);
                // Never a half-written or unverifiable report left for anyone
                // to mistake for this run's result.
                self.discard_report(&run_id);
                if metadata.attempts.len() >= MAX_EXECUTION_ATTEMPTS {
                    self.write_run(&metadata)?;
                    self.finish_run(
                        &run_id,
                        RunOutcome::ExecutionError {
                            detail: format!(
                                "Run interrupted by a server restart during {} execution \
                                 attempts; it is not retried again.",
                                metadata.attempts.len()
                            ),
                        },
                    )?;
                    // finish_run closed nothing new: the last attempt was
                    // already recorded as interrupted.
                    recovery.failed.push(run_id);
                    continue;
                }
                metadata.status = RunStatus::Queued;
                self.write_run(&metadata)?;
            }
            self.clear_run_work(&run_id).ok();
            recovery.requeued.push(run_id);
        }
        Ok(recovery)
    }

    /// A report already on disk for a running run, if and only if it is a
    /// complete, verified result for exactly this run's inputs.
    ///
    /// Complete: it parses as the canonical report contract. Verified: it names
    /// this run's change, candidate artefact and pinned bundle. Anything short
    /// of that is not trusted and the run is executed again.
    fn completed_report(&self, metadata: &RunMetadata) -> Option<CiReport> {
        let path = self
            .storage
            .run_dir(&metadata.run_id)
            .ok()?
            .join("report.json");
        let report: CiReport = self.storage.read_json(&path).ok()?;
        metadata.change.as_ref()?.verify_report(&report).ok()?;
        let artifact = metadata.candidate_artifact.as_ref()?;
        (report.bundle.sha256 == metadata.bundle_sha256
            && report.candidate.sha256 == artifact.sha256
            && report.candidate.len == artifact.len)
            .then_some(report)
    }

    fn discard_report(&self, run_id: &str) {
        if let Ok(directory) = self.storage.run_dir(run_id) {
            for name in ["report.json", "report.md"] {
                std::fs::remove_file(directory.join(name)).ok();
            }
        }
    }

    /// The pre-P2 name, kept for callers that only need the list of run ids
    /// that were not resumed.
    pub fn recover_interrupted_runs(&self) -> Result<Vec<String>> {
        Ok(self.recover_runs()?.failed)
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

    /// A run's scratch cache. Never authoritative: every input a worker uses is
    /// read from a durable, hash-verified location, and anything placed here is
    /// derived from those and may be deleted at any time.
    pub fn run_work_dir(&self, run_id: &str) -> Result<std::path::PathBuf> {
        Ok(self.storage.run_dir(run_id)?.join("work"))
    }

    /// Drop the scratch cache. Inputs, report and metadata all stay, and no
    /// shared artefact is ever touched.
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

/// A report is a verdict about this run only if it names this run's change and
/// was computed over this run's artefact:
///
/// ```text
/// registry change_spec_id == stored spec id == report.change.change_spec_id
/// registry candidate_artifact == spec candidate == report.candidate
/// ```
fn verify_outcome(metadata: &RunMetadata, report: &CiReport) -> Result<()> {
    if let Some(change) = &metadata.change {
        change.verify_report(report)?;
    }
    if let Some(artifact) = &metadata.candidate_artifact {
        ensure!(
            report.candidate.sha256 == artifact.sha256 && report.candidate.len == artifact.len,
            "the report's candidate is not the run's artifact {}",
            artifact.sha256
        );
    }
    Ok(())
}

/// Close the attempt currently holding a run, if one is open.
fn close_attempt(metadata: &mut RunMetadata, end: AttemptEnd, now: u64) {
    if let Some(last) = metadata.attempts.last_mut() {
        if last.end.is_none() {
            last.end = Some(end);
            last.ended_at_unix_seconds = Some(now);
        }
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
