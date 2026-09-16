//! Projects, bundles and runs on top of [`Storage`].
//!
//! The rule that shapes this module: a bundle is immutable and content
//! addressed, and activation is a pointer change. A corpus refresh installs
//! bundle B and points the project at it; bundle A is never edited, so a run
//! recorded against A stays reproducible.

use anyhow::{bail, Context, Result};
use eplyx_engine::bundle::CiBundle;
use eplyx_engine::ci::CiReport;
use serde::{Deserialize, Serialize};

use crate::project::Project;
use crate::storage::Storage;

/// What a run was, without any of what it used to run.
///
/// No token, no endpoint, no candidate bytes. Wall-clock time is allowed here
/// because this is hosted metadata: it sits beside the canonical report rather
/// than inside it, so it cannot reach a determinism hash.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunMetadata {
    pub run_id: String,
    pub project_id: String,
    pub bundle_sha256: String,
    pub corpus_sha256: String,
    pub baseline_sha256: String,
    pub candidate_sha256: String,
    pub adapter: String,
    pub adapter_version: u32,
    pub semantic_schema_version: u32,
    pub record_count: usize,
    pub status: String,
    pub exit_code: u8,
    pub created_at_unix_seconds: u64,
}

pub struct Registry {
    storage: Storage,
}

impl Registry {
    pub fn new(storage: Storage) -> Self {
        Self { storage }
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

    pub fn save_run(
        &self,
        metadata: &RunMetadata,
        report: &CiReport,
        markdown: &str,
    ) -> Result<()> {
        let directory = self.storage.run_dir(&metadata.run_id)?;
        self.storage
            .write_json(&directory.join("metadata.json"), metadata)?;
        // The canonical report, byte for byte what the engine produced - down
        // to the trailing newline, so `report.json` from this endpoint and the
        // output of a local `eplyx ci check --format json` are the same file.
        let mut canonical = serde_json::to_vec_pretty(report)?;
        canonical.push(b'\n');
        self.storage
            .write_bytes(&directory.join("report.json"), &canonical)?;
        self.storage
            .write_bytes(&directory.join("report.md"), markdown.as_bytes())
    }

    pub fn load_run(&self, run_id: &str) -> Result<RunMetadata> {
        let path = self.storage.run_dir(run_id)?.join("metadata.json");
        if !self.storage.exists(&path) {
            bail!("no such run");
        }
        self.storage.read_json(&path)
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
