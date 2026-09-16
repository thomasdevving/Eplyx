//! Durable, append-only corpus storage.
//!
//! A historical observation is evidence: once written it does not change. This
//! store therefore treats a record as immutable and addressed by its own stable
//! ID. Re-adding an identical record is a no-op; presenting different content
//! for an ID that already exists is an error rather than an overwrite, because
//! silently replacing evidence is the one failure that would make every
//! downstream number untrustworthy.
//!
//! ```text
//! corpus/
//! ├── manifest.json        what this corpus is, and its canonical hash
//! ├── records/
//! │   └── <observation-id>.json
//! └── corpus.json          canonical index: every record, in ID order
//! ```
//!
//! `corpus.json` is a *view*. The records directory is the source of truth, and
//! each record stays independently addressable and loadable on its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::replay::{hash_bytes, ReplayRecord};

pub const CORPUS_MANIFEST_SCHEMA: u32 = 1;

/// What a corpus is, and what it contains.
///
/// Deliberately free of wall-clock time, host names, endpoints and durations:
/// the manifest is part of the canonical output, and anything that varies
/// between two runs over the same evidence would make it unusable as an
/// identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusManifest {
    pub schema_version: u32,
    pub program_id: String,
    pub protocol: Option<String>,
    pub adapter_version: Option<u32>,
    pub genesis_hash: String,
    pub record_count: usize,
    /// Every observation, in canonical order.
    pub record_ids: Vec<String>,
    /// SHA-256 over the canonical encoding of every record, in ID order.
    pub canonical_hash: String,
}

/// What adding a record did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insert {
    Added,
    /// Byte-identical to what was already stored, so nothing was written.
    AlreadyPresent,
}

pub struct CorpusStore {
    root: PathBuf,
}

/// Canonical bytes for one record. Compact JSON in declaration order, which
/// serde produces deterministically for a struct.
fn canonical(record: &ReplayRecord) -> Result<Vec<u8>> {
    serde_json::to_vec(record).context("encoding a replay record")
}

impl CorpusStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("records"))
            .with_context(|| format!("creating corpus at {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn record_path(&self, id: &str) -> PathBuf {
        self.root.join("records").join(format!("{id}.json"))
    }

    pub fn corpus_path(&self) -> PathBuf {
        self.root.join("corpus.json")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    /// Add one observation.
    ///
    /// Idempotent for identical content. A differing record under an existing
    /// ID is refused: an observation ID is derived from the transaction, so two
    /// different bodies under one ID means the evidence disagrees with itself.
    pub fn insert(&self, record: &ReplayRecord) -> Result<Insert> {
        anyhow::ensure!(!record.id.is_empty(), "a record must carry a stable id");
        anyhow::ensure!(
            !record.id.contains(['/', '\\', '.']),
            "observation id {:?} is not a safe file name",
            record.id
        );
        let bytes = canonical(record)?;
        let path = self.record_path(&record.id);
        if path.exists() {
            let existing =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            if existing == bytes {
                return Ok(Insert::AlreadyPresent);
            }
            anyhow::bail!(
                "observation {} already exists with different content; historical records are \
                 immutable. Stored sha256 {}, offered sha256 {}",
                record.id,
                hash_bytes(&existing),
                hash_bytes(&bytes)
            );
        }
        std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
        Ok(Insert::Added)
    }

    /// Every stored record, in canonical ID order.
    pub fn load(&self) -> Result<Vec<ReplayRecord>> {
        let dir = self.root.join("records");
        let mut byid: BTreeMap<String, ReplayRecord> = BTreeMap::new();
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            let record: ReplayRecord = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", path.display()))?;
            anyhow::ensure!(
                path.file_stem().and_then(|s| s.to_str()) == Some(record.id.as_str()),
                "record file {} does not match the observation id it contains",
                path.display()
            );
            byid.insert(record.id.clone(), record);
        }
        Ok(byid.into_values().collect())
    }

    /// Write the canonical index and manifest from what is stored.
    pub fn publish(&self) -> Result<CorpusManifest> {
        let records = self.load()?;
        let manifest = self.describe(&records)?;
        crate::ingest::write_json(&self.corpus_path(), &records)?;
        crate::ingest::write_json(&self.manifest_path(), &manifest)?;
        Ok(manifest)
    }

    /// The manifest these records produce, without writing anything.
    ///
    /// Verifying a published corpus must not modify it: a reader that rewrites
    /// what it reads cannot be used to check that a bundle is unchanged.
    pub fn describe(&self, records: &[ReplayRecord]) -> Result<CorpusManifest> {
        let mut digest = Vec::new();
        for record in records {
            digest.extend_from_slice(hash_bytes(&canonical(record)?).as_bytes());
        }
        let first = records.first();
        let manifest = CorpusManifest {
            schema_version: CORPUS_MANIFEST_SCHEMA,
            program_id: first.map(|r| r.program_id.clone()).unwrap_or_default(),
            protocol: first
                .and_then(|r| crate::protocol::adapter_for(&r.program_id))
                .map(|a| a.name().to_string()),
            adapter_version: first
                .and_then(|r| crate::protocol::adapter_for(&r.program_id))
                .map(|a| a.adapter_version()),
            genesis_hash: first.map(|r| r.genesis_hash.clone()).unwrap_or_default(),
            record_count: records.len(),
            record_ids: records.iter().map(|r| r.id.clone()).collect(),
            canonical_hash: hash_bytes(&digest),
        };
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique scratch directory per test, removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let unique = format!(
                "eplyx-corpus-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            );
            let path = std::env::temp_dir().join(unique);
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch dir");
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(tag: &str) -> (Scratch, CorpusStore) {
        let scratch = Scratch::new(tag);
        let store = CorpusStore::open(&scratch.0).expect("open");
        (scratch, store)
    }

    /// Based on a committed real mainnet record, so the store is exercised
    /// against the shape it actually holds rather than a hand-built stub.
    fn record(id: &str, slot: u64) -> ReplayRecord {
        let mut r: ReplayRecord = serde_json::from_str(include_str!(
            "../../docs/examples/mainnet-stake-pool-record.json"
        ))
        .expect("committed mainnet record");
        r.id = id.to_string();
        r.transaction.slot = slot;
        r
    }

    #[test]
    fn adding_the_same_observation_twice_is_idempotent() {
        let (_scratch, store) = store("idempotent");
        let r = record("obs-a", 10);
        assert_eq!(store.insert(&r).unwrap(), Insert::Added);
        assert_eq!(store.insert(&r).unwrap(), Insert::AlreadyPresent);
        assert_eq!(store.load().unwrap().len(), 1);
    }

    #[test]
    fn a_conflicting_body_under_an_existing_id_is_refused() {
        let (_scratch, store) = store("conflict");
        store.insert(&record("obs-a", 10)).unwrap();
        let error = store
            .insert(&record("obs-a", 11))
            .expect_err("immutable records must not be overwritten")
            .to_string();
        assert!(error.contains("immutable"), "{error}");
        // The stored evidence is untouched.
        assert_eq!(store.load().unwrap()[0].transaction.slot, 10);
    }

    #[test]
    fn records_load_in_canonical_id_order_regardless_of_insertion_order() {
        let (_scratch, store) = store("order");
        for id in ["obs-c", "obs-a", "obs-b"] {
            store.insert(&record(id, 1)).unwrap();
        }
        let ids: Vec<String> = store.load().unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["obs-a", "obs-b", "obs-c"]);
    }

    #[test]
    fn publishing_the_same_evidence_twice_is_byte_identical() {
        let (_scratch, store) = store("publish");
        for id in ["obs-b", "obs-a"] {
            store.insert(&record(id, 7)).unwrap();
        }
        let first = store.publish().unwrap();
        let corpus_a = std::fs::read(store.corpus_path()).unwrap();
        let manifest_a = std::fs::read(store.manifest_path()).unwrap();

        // Re-inserting and re-publishing must reproduce the same bytes: the
        // canonical output carries no timing or transport metadata.
        for id in ["obs-a", "obs-b"] {
            store.insert(&record(id, 7)).unwrap();
        }
        let second = store.publish().unwrap();
        assert_eq!(first, second);
        assert_eq!(corpus_a, std::fs::read(store.corpus_path()).unwrap());
        assert_eq!(manifest_a, std::fs::read(store.manifest_path()).unwrap());
        assert_eq!(first.record_count, 2);
        assert_eq!(first.record_ids, vec!["obs-a", "obs-b"]);
    }

    #[test]
    fn an_unsafe_observation_id_is_refused() {
        let (_scratch, store) = store("unsafe-id");
        for bad in ["../escape", "a/b", "with.dot", ""] {
            assert!(
                store.insert(&record(bad, 1)).is_err(),
                "{bad:?} must be refused"
            );
        }
    }
}
