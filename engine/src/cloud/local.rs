//! Cloud bookkeeping inside `.eplyx/`: the optional link from the stable local
//! project ID to at most one cloud project, and the per-run sync sidecar.
//! Both are operational metadata. Canonical run artifacts are never touched.
use super::contract::is_cloud_id;
use crate::local_store::is_safe_id;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const PROJECT_LIMIT: u64 = 64 * 1024;
const STATE_LIMIT: u64 = 64 * 1024;

/// Stored under the `cloud` key of `.eplyx/project.json`. The local `id` is
/// never replaced; these are only the cloud IDs and where they live.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudLink {
    pub server: String,
    pub workspace_id: String,
    pub project_id: String,
    pub linked_at: String,
}

pub struct LocalProject {
    pub id: String,
    pub name: String,
    pub link: Option<CloudLink>,
}

fn regular_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
        Ok(meta) => {
            ensure!(
                !meta.file_type().is_symlink() && meta.is_file(),
                "{} must be a regular file",
                path.display()
            );
            Ok(true)
        }
    }
}

fn real_dir(path: &Path, create: bool) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if create {
                fs::create_dir(path)?;
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Err(error) => Err(error.into()),
        Ok(meta) => {
            ensure!(
                !meta.file_type().is_symlink() && meta.is_dir(),
                "{} must be a real directory",
                path.display()
            );
            Ok(true)
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("path has no parent")?;
    use std::io::Write;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ensure!(
        fs::metadata(path)?.len() <= limit,
        "{} is too large",
        path.display()
    );
    Ok(fs::read(path)?)
}

pub fn project(base: &Path) -> Result<LocalProject> {
    ensure!(real_dir(base, false)?, "local store missing");
    let file = base.join("project.json");
    ensure!(
        regular_file(&file)?,
        "no .eplyx/project.json; run `eplyx init` first"
    );
    let value: Value = serde_json::from_slice(&read_bounded(&file, PROJECT_LIMIT)?)
        .context("invalid .eplyx/project.json")?;
    let id = value["local_id"]
        .as_str()
        .filter(|id| is_safe_id(id, "local_"))
        .context(".eplyx/project.json has no valid local project ID")?
        .to_owned();
    let name = value["name"].as_str().unwrap_or("").to_owned();
    let link = match value.get("cloud") {
        None | Some(Value::Null) => None,
        Some(link) => {
            let link: CloudLink = serde_json::from_value(link.clone())
                .context("invalid cloud link in project.json")?;
            ensure!(
                is_cloud_id(&link.workspace_id, "ws_")
                    && super::contract::is_project_id(&link.project_id),
                "invalid cloud link in project.json"
            );
            ensure!(
                super::credentials::normalize_server(&link.server)? == link.server,
                "invalid cloud origin"
            );
            Some(link)
        }
    };
    Ok(LocalProject { id, name, link })
}

/// Set or clear the cloud link, preserving every other field of project.json.
pub fn write_link(base: &Path, link: Option<&CloudLink>) -> Result<()> {
    ensure!(real_dir(base, false)?, "local store missing");
    let file = base.join("project.json");
    ensure!(
        regular_file(&file)?,
        "no .eplyx/project.json; run `eplyx init` first"
    );
    let mut value: Value = serde_json::from_slice(&read_bounded(&file, PROJECT_LIMIT)?)
        .context("invalid .eplyx/project.json")?;
    let object = value
        .as_object_mut()
        .context(".eplyx/project.json must be an object")?;
    match link {
        Some(link) => {
            ensure!(
                super::credentials::normalize_server(&link.server)? == link.server
                    && is_cloud_id(&link.workspace_id, "ws_")
                    && super::contract::is_project_id(&link.project_id),
                "invalid cloud link"
            );
            object.insert("cloud".into(), serde_json::to_value(link)?);
        }
        None => {
            object.remove("cloud");
        }
    }
    atomic_write(&file, &serde_json::to_vec_pretty(&value)?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncStatus {
    Synced,
    Failed,
}

/// `.eplyx/sync/runs/<run>.json`: the last sync attempt for one run.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSyncState {
    pub schema_version: u32,
    pub run_id: String,
    pub server: String,
    pub cloud_project_id: String,
    pub status: SyncStatus,
    pub core_sha256: Option<String>,
    pub search_sha256: Option<String>,
    pub counterexamples_synced: usize,
    pub reproductions_synced: usize,
    pub last_attempt_at: String,
    pub last_synced_at: Option<String>,
    pub error: Option<String>,
    pub url: Option<String>,
}

fn state_path(base: &Path, run_id: &str) -> Result<PathBuf> {
    ensure!(is_safe_id(run_id, "run_"), "invalid run ID");
    Ok(base
        .join("sync")
        .join("runs")
        .join(format!("{run_id}.json")))
}

pub fn read_state(base: &Path, run_id: &str) -> Result<Option<RunSyncState>> {
    let path = state_path(base, run_id)?;
    for dir in [base.join("sync"), base.join("sync").join("runs")] {
        if !real_dir(&dir, false)? {
            return Ok(None);
        }
    }
    if !regular_file(&path)? {
        return Ok(None);
    }
    let state: RunSyncState =
        serde_json::from_slice(&read_bounded(&path, STATE_LIMIT)?).context("invalid sync state")?;
    if state.run_id != run_id {
        bail!("sync state run ID mismatch");
    }
    ensure!(
        super::credentials::normalize_server(&state.server)? == state.server
            && super::contract::is_project_id(&state.cloud_project_id),
        "invalid sync destination"
    );
    if let Some(url) = &state.url {
        ensure!(
            url.starts_with(&format!("{}/p/{}/", state.server, state.cloud_project_id)),
            "invalid sync link"
        );
    }
    Ok(Some(state))
}

pub fn write_state(base: &Path, state: &RunSyncState) -> Result<()> {
    let path = state_path(base, &state.run_id)?;
    real_dir(&base.join("sync"), true)?;
    real_dir(&base.join("sync").join("runs"), true)?;
    if let Ok(meta) = fs::symlink_metadata(&path) {
        ensure!(
            !meta.file_type().is_symlink() && meta.is_file(),
            "refusing to replace a non-file sync state"
        );
    }
    atomic_write(&path, &serde_json::to_vec_pretty(state)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_preserves_local_identity_and_round_trips() {
        let base = std::env::temp_dir().join(format!("eplyx-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(
            base.join("project.json"),
            r#"{"schema_version":1,"local_id":"local_0123456789abcdef0123","name":"demo"}"#,
        )
        .unwrap();
        assert!(project(&base).unwrap().link.is_none());
        let link = CloudLink {
            server: "https://cloud.example".into(),
            workspace_id: "ws_0123456789abcdef0123".into(),
            project_id: "proj_01JQZ3K5R8V2X7M9N4T6W1Y0BC".into(),
            linked_at: "2026-09-25T00:00:00Z".into(),
        };
        write_link(&base, Some(&link)).unwrap();
        let loaded = project(&base).unwrap();
        assert_eq!(loaded.id, "local_0123456789abcdef0123");
        assert_eq!(loaded.name, "demo");
        assert_eq!(loaded.link.as_ref(), Some(&link));
        write_link(&base, None).unwrap();
        assert!(project(&base).unwrap().link.is_none());
        fs::remove_dir_all(base).unwrap();
    }
}
