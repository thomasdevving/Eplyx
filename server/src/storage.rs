//! Durable state, behind an abstraction thin enough to replace.
//!
//! Railway's deployment filesystem is ephemeral, so everything here lives under
//! a mounted volume. The shape is deliberately object-storage-like — a path and
//! some bytes — so swapping the backend later does not reach into the handlers.
//! No S3 implementation is written until something needs one.
//!
//! ```text
//! /data
//!   projects/<project-id>/project.json
//!   projects/<project-id>/tokens/<token-id>.json
//!   projects/<project-id>/bundles/<bundle-id>.json   a project's record of one
//!   projects/<project-id>/runs/<run-id>              index marker, empty
//!   projects/<project-id>/changes/<change-spec-id>/<run-id>   index marker, empty
//!   bundles/<bundle-sha256>/          an installed, verified CI bundle
//!   runs/<run-id>/metadata.json
//!   runs/<run-id>/change_spec.json    the canonical proposal, re-verified on read
//!   runs/<run-id>/work/artifacts/programs/<sha256>   candidate, until terminal
//!   runs/<run-id>/report.json
//!   runs/<run-id>/report.md
//! ```
//!
//! Bundle *content* is addressed by its hash and shared; a project's record of
//! a bundle lives under the project. Two projects that upload identical bytes
//! store them once and own separate records of them.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Serialize};

#[derive(Clone, Debug)]
pub struct Storage {
    root: PathBuf,
}

/// Identifiers become path segments, so they are checked rather than trusted.
///
/// A project or run id arrives from a URL. Anything that could climb out of the
/// data directory, or name a different project's directory, is rejected before
/// it reaches the filesystem.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn checked(id: &str, what: &str) -> Result<()> {
    if !valid_id(id) {
        bail!("{what} {id:?} is not a valid identifier");
    }
    Ok(())
}

impl Storage {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        for directory in ["projects", "bundles", "runs"] {
            std::fs::create_dir_all(root.join(directory))
                .with_context(|| format!("creating {}", root.join(directory).display()))?;
        }
        Ok(Self { root })
    }

    /// Can the volume actually be written? This is what `/ready` reports.
    pub fn writable(&self) -> Result<()> {
        let probe = self.root.join(".ready");
        std::fs::write(&probe, b"ok").with_context(|| format!("writing {}", probe.display()))?;
        std::fs::remove_file(&probe).ok();
        Ok(())
    }

    pub fn project_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.project_dir(id)?.join("project.json"))
    }

    pub fn bundle_path(&self, sha256: &str) -> Result<PathBuf> {
        checked(sha256, "bundle hash")?;
        Ok(self.root.join("bundles").join(sha256))
    }

    pub fn runs_root(&self) -> PathBuf {
        self.root.join("runs")
    }

    pub fn projects_root(&self) -> PathBuf {
        self.root.join("projects")
    }

    pub fn project_dir(&self, project_id: &str) -> Result<PathBuf> {
        checked(project_id, "project id")?;
        Ok(self.projects_root().join(project_id))
    }

    pub fn project_tokens_dir(&self, project_id: &str) -> Result<PathBuf> {
        Ok(self.project_dir(project_id)?.join("tokens"))
    }

    pub fn project_token_path(&self, project_id: &str, token_id: &str) -> Result<PathBuf> {
        checked(token_id, "token id")?;
        Ok(self
            .project_tokens_dir(project_id)?
            .join(format!("{token_id}.json")))
    }

    pub fn project_bundles_dir(&self, project_id: &str) -> Result<PathBuf> {
        Ok(self.project_dir(project_id)?.join("bundles"))
    }

    pub fn project_bundle_path(&self, project_id: &str, bundle_id: &str) -> Result<PathBuf> {
        checked(bundle_id, "bundle id")?;
        Ok(self
            .project_bundles_dir(project_id)?
            .join(format!("{bundle_id}.json")))
    }

    pub fn project_runs_dir(&self, project_id: &str) -> Result<PathBuf> {
        Ok(self.project_dir(project_id)?.join("runs"))
    }

    pub fn project_run_marker(&self, project_id: &str, run_id: &str) -> Result<PathBuf> {
        checked(run_id, "run id")?;
        Ok(self.project_runs_dir(project_id)?.join(run_id))
    }

    pub fn project_change_runs_dir(
        &self,
        project_id: &str,
        change_spec_id: &str,
    ) -> Result<PathBuf> {
        checked(change_spec_id, "change spec id")?;
        Ok(self
            .project_dir(project_id)?
            .join("changes")
            .join(change_spec_id))
    }

    pub fn project_change_run_marker(
        &self,
        project_id: &str,
        change_spec_id: &str,
        run_id: &str,
    ) -> Result<PathBuf> {
        checked(run_id, "run id")?;
        Ok(self
            .project_change_runs_dir(project_id, change_spec_id)?
            .join(run_id))
    }

    pub fn run_dir(&self, id: &str) -> Result<PathBuf> {
        checked(id, "run id")?;
        Ok(self.root.join("runs").join(id))
    }

    pub fn read_json<T: DeserializeOwned>(&self, path: &Path) -> Result<T> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(value)?;
        self.write_bytes(path, &bytes)
    }

    /// Written to a temporary name and renamed, so an interrupted write never
    /// leaves a half-file that later parses as truth.
    pub fn write_bytes(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, bytes)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, path)
            .with_context(|| format!("renaming into {}", path.display()))?;
        Ok(())
    }

    pub fn read_bytes(&self, path: &Path) -> Result<Vec<u8>> {
        std::fs::read(path).with_context(|| format!("reading {}", path.display()))
    }

    pub fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Identifiers become path segments. Anything that could climb out of the
    /// data directory is refused before it reaches the filesystem.
    #[test]
    fn traversal_and_oddities_are_rejected() {
        for bad in [
            "..",
            ".",
            "../../etc/passwd",
            "a/b",
            "a\\b",
            "",
            "with space",
            "nul\0byte",
            &"x".repeat(65),
        ] {
            assert!(!valid_id(bad), "accepted {bad:?}");
        }
        for good in ["stake-pool", "run_01ABC", "a", &"x".repeat(64)] {
            assert!(valid_id(good), "rejected {good:?}");
        }
    }

    #[test]
    fn a_traversing_id_never_produces_a_path() {
        let scratch = tempfile::tempdir().unwrap();
        let storage = Storage::open(scratch.path()).unwrap();
        assert!(storage.project_path("../other").is_err());
        assert!(storage.run_dir("..").is_err());
        assert!(storage.bundle_path("../../secrets").is_err());
    }
}
