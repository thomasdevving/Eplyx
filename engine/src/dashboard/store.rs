//! Guarded read access to one project's `.eplyx/` store plus a rebuildable
//! summary cache. Every read resolves a fixed, server-chosen member path under
//! the canonical store root and rejects symlinks at each component.
use super::view;
use crate::local_store::is_safe_id;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Bump whenever a cached summary's shape changes; older caches are rebuilt.
pub const INDEX_VERSION: u32 = 3;
pub const INDEX_FILE: &str = "dashboard-index.json";
pub const SMALL_LIMIT: u64 = 1024 * 1024;
/// The engine's own replay bound for a saved package report.
pub const REPORT_LIMIT: u64 = 128 * 1024 * 1024;
pub const SEARCH_LIMIT: u64 = 16 * 1024 * 1024;
pub const CAPTURE_LIMIT: u64 = 256 * 1024 * 1024;

/// Members whose size and modification time decide whether a cached run
/// summary is still current.
pub const RUN_SUMMARY_MEMBERS: [&str; 6] = [
    "metadata.json",
    "result/report.json",
    "result/bindings.json",
    "input/change.json",
    "input/state.json",
    "search/migration-search.json",
];

pub struct Store {
    root: PathBuf,
    base: PathBuf,
}

/// A cache only: every summary is re-derived from source artifacts whenever
/// their fingerprints change, and a missing or corrupt index is rebuilt.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    pub note: String,
    pub runs: BTreeMap<String, Entry>,
    pub counterexamples: BTreeMap<String, Entry>,
    #[serde(default)]
    pub reproductions: BTreeMap<String, Entry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub fingerprint: Vec<String>,
    pub summary: Value,
}

fn real_dir(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
        Ok(meta) => {
            ensure!(
                !meta.file_type().is_symlink(),
                "{} must not be a symlink",
                path.display()
            );
            ensure!(meta.is_dir(), "{} is not a directory", path.display());
            ensure!(
                path.canonicalize()? == path,
                "{} must be a real directory",
                path.display()
            );
            Ok(true)
        }
    }
}

fn valid_component(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        let root = root.canonicalize().context("project root missing")?;
        let base = root.join(".eplyx");
        ensure!(
            real_dir(&base)?,
            "no local run store at .eplyx/; run `eplyx init`, then `eplyx migration analyse`"
        );
        for child in ["runs", "counterexamples", "reproductions", "cache"] {
            real_dir(&base.join(child)).with_context(|| format!("invalid .eplyx/{child}"))?;
        }
        Ok(Self { root, base })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Resolve a store-relative member built from fixed server-side parts.
    /// Returns `None` when the member is absent; errors on any symlink,
    /// wrong file type, oversize file or escape from the store root.
    fn member(&self, parts: &[&str], limit: u64) -> Result<Option<(PathBuf, u64)>> {
        ensure!(!parts.is_empty(), "empty store member");
        let mut path = self.base.clone();
        for (position, part) in parts.iter().enumerate() {
            ensure!(valid_component(part), "invalid store member");
            path.push(part);
            let meta = match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
                Ok(meta) => meta,
            };
            ensure!(
                !meta.file_type().is_symlink(),
                "symlinked store member rejected: {}",
                parts[..=position].join("/")
            );
            if position + 1 < parts.len() {
                ensure!(meta.is_dir(), "store member parent is not a directory");
            } else {
                ensure!(meta.is_file(), "store member is not a file");
                ensure!(
                    meta.len() <= limit,
                    "{} exceeds the dashboard read bound",
                    parts.join("/")
                );
            }
        }
        ensure!(
            path.canonicalize()?.starts_with(&self.base),
            "store member escapes .eplyx/"
        );
        let len = fs::metadata(&path)?.len();
        Ok(Some((path, len)))
    }

    pub fn read(&self, parts: &[&str], limit: u64) -> Result<Option<Vec<u8>>> {
        let Some((path, _)) = self.member(parts, limit)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        File::open(path)?
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= limit,
            "store member grew while reading"
        );
        Ok(Some(bytes))
    }

    pub fn open_file(&self, parts: &[&str], limit: u64) -> Result<Option<(File, u64)>> {
        let Some((path, len)) = self.member(parts, limit)? else {
            return Ok(None);
        };
        Ok(Some((File::open(path)?, len)))
    }

    pub fn json(&self, parts: &[&str], limit: u64) -> Result<Option<Value>> {
        self.read(parts, limit)?
            .map(|bytes| serde_json::from_slice(&bytes).context("invalid JSON"))
            .transpose()
    }

    /// Size and modification time; `symlink` or `missing` are explicit so a
    /// changed file type always invalidates the cached summary.
    pub fn fingerprint(&self, relative: &str) -> String {
        let path = relative
            .split('/')
            .fold(self.base.clone(), |path, part| path.join(part));
        match fs::symlink_metadata(&path) {
            Err(_) => "missing".into(),
            Ok(meta) if meta.file_type().is_symlink() => "symlink".into(),
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |time| time.as_nanos());
                format!("{relative}:{}:{modified}", meta.len())
            }
        }
    }

    pub fn modified_millis(&self, parts: &[&str]) -> Option<i64> {
        let (path, _) = self.member(parts, u64::MAX).ok()??;
        let time = fs::metadata(path).ok()?.modified().ok()?;
        Some(time.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
    }

    /// Run directories: real directories with a safe `run_` ID. Symlinked or
    /// oddly named entries are counted as ignored, never followed.
    pub fn run_ids(&self) -> Result<(Vec<String>, usize)> {
        self.list("runs", "run_", true)
    }

    pub fn counterexample_ids(&self) -> Result<(Vec<String>, usize)> {
        self.list("counterexamples", "cx_", false)
    }

    pub fn reproduction_ids(&self) -> Result<(Vec<String>, usize)> {
        self.list("reproductions", "repro_", false)
    }

    fn list(&self, directory: &str, prefix: &str, dirs: bool) -> Result<(Vec<String>, usize)> {
        let path = self.base.join(directory);
        if !real_dir(&path)? {
            return Ok((Vec::new(), 0));
        }
        let mut ids = Vec::new();
        let mut ignored = 0;
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = if dirs {
                Some(name.as_str())
            } else {
                name.strip_suffix(".json")
            };
            let usable = if dirs { kind.is_dir() } else { kind.is_file() };
            match id {
                Some(id) if usable && is_safe_id(id, prefix) => ids.push(id.to_owned()),
                _ if name.starts_with('.') => {}
                _ => ignored += 1,
            }
        }
        ids.sort();
        Ok((ids, ignored))
    }

    fn index_path(&self) -> PathBuf {
        self.base.join("cache").join(INDEX_FILE)
    }

    /// Load the cache; anything unreadable, symlinked or from another version
    /// yields an empty index that the next refresh rebuilds.
    pub fn load_index(&self) -> Index {
        // Never trust persisted presentation claims on startup. The first refresh
        // reconstructs summaries from immutable records; subsequent reads use the
        // in-memory cache and invalidate it on source changes.
        Index::default()
    }

    /// Best-effort atomic cache write. The store stays usable when the cache
    /// directory is read-only; source records remain authoritative.
    pub fn save_index(&self, index: &Index) -> Result<()> {
        let cache = self.base.join("cache");
        if !real_dir(&cache)? {
            fs::create_dir(&cache)?;
        }
        let destination = self.index_path();
        if let Ok(meta) = fs::symlink_metadata(&destination) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                bail!("refusing to replace a non-file dashboard index");
            }
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&cache)?;
        temporary.write_all(&serde_json::to_vec(index)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&destination)?;
        Ok(())
    }

    /// Bring the index up to date with the store. Returns whether it changed.
    pub fn refresh(&self, index: &mut Index) -> Result<bool> {
        let mut changed = false;
        if index.version != INDEX_VERSION {
            *index = Index {
                version: INDEX_VERSION,
                ..Index::default()
            };
            changed = true;
        }
        index.note = "Cache of summaries derived from .eplyx/ source artifacts. Not evidence; safe to delete.".into();
        let (runs, _) = self.run_ids()?;
        let present: BTreeSet<&String> = runs.iter().collect();
        let before = index.runs.len();
        index.runs.retain(|id, _| present.contains(id));
        changed |= before != index.runs.len();
        for id in &runs {
            let fingerprint = RUN_SUMMARY_MEMBERS
                .iter()
                .map(|member| self.fingerprint(&format!("runs/{id}/{member}")))
                .collect::<Vec<_>>();
            if index.runs.get(id).map(|entry| &entry.fingerprint) != Some(&fingerprint) {
                let summary = view::run_summary(self, id);
                index.runs.insert(
                    id.clone(),
                    Entry {
                        fingerprint,
                        summary,
                    },
                );
                changed = true;
            }
        }
        let (counterexamples, _) = self.counterexample_ids()?;
        let present: BTreeSet<&String> = counterexamples.iter().collect();
        let before = index.counterexamples.len();
        index.counterexamples.retain(|id, _| present.contains(id));
        changed |= before != index.counterexamples.len();
        for id in &counterexamples {
            let fingerprint = vec![self.fingerprint(&format!("counterexamples/{id}.json"))];
            if index
                .counterexamples
                .get(id)
                .map(|entry| &entry.fingerprint)
                != Some(&fingerprint)
            {
                let summary = view::counterexample_summary(self, id);
                index.counterexamples.insert(
                    id.clone(),
                    Entry {
                        fingerprint,
                        summary,
                    },
                );
                changed = true;
            }
        }
        let (reproductions, _) = self.reproduction_ids()?;
        let present: BTreeSet<&String> = reproductions.iter().collect();
        let before = index.reproductions.len();
        index.reproductions.retain(|id, _| present.contains(id));
        changed |= before != index.reproductions.len();
        for id in &reproductions {
            let fingerprint = vec![self.fingerprint(&format!("reproductions/{id}.json"))];
            if index.reproductions.get(id).map(|entry| &entry.fingerprint) != Some(&fingerprint) {
                let summary = view::reproduction_summary(self, id);
                index.reproductions.insert(
                    id.clone(),
                    Entry {
                        fingerprint,
                        summary,
                    },
                );
                changed = true;
            }
        }
        Ok(changed)
    }
}
