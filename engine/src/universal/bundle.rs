//! Schema-2 bundles share the ordinary bundle command and hash every CAS byte.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

use super::{evidence::EvidenceStore, model::ReplayObservationV2, resolver::ResolvedReplayInput};
use crate::{
    bundle::{ActionCoverage, AdapterMetadata, BundledLimitation, SlotRange},
    corpus_store::{CorpusManifest, CorpusStore},
    replay::hash_bytes,
    semantics::SEMANTIC_SCHEMA_VERSION,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceObjectEntry {
    pub path: String,
    pub sha256: String,
    pub len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleManifestV2 {
    pub schema_version: u32,
    pub program_id: String,
    pub protocol: String,
    pub genesis_hash: String,
    pub baseline_program_sha256: String,
    pub baseline_program_len: u64,
    pub corpus_sha256: String,
    pub record_count: usize,
    pub record_ids: Vec<String>,
    pub source_slot_range: SlotRange,
    pub adapter_metadata_sha256: String,
    pub semantic_schema_version: u32,
    pub evidence_objects: Vec<EvidenceObjectEntry>,
    /// Exact bytes of the published index, manifest and record files.
    pub corpus_files: Vec<EvidenceObjectEntry>,
    pub bundle_sha256: String,
}

impl BundleManifestV2 {
    fn digest(&self) -> Result<String> {
        let mut copy = self.clone();
        copy.bundle_sha256.clear();
        Ok(hash_bytes(&serde_json::to_vec(&copy)?))
    }
}

fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    fn visit(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            ensure!(!ty.is_symlink(), "bundle evidence cannot contain symlinks");
            if ty.is_dir() {
                visit(&entry.path(), out)?;
            } else if ty.is_file() {
                out.push(entry.path());
            } else {
                anyhow::bail!("bundle evidence contains unsupported file type");
            }
        }
        Ok(())
    }
    visit(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn inventory(root: &Path) -> Result<Vec<EvidenceObjectEntry>> {
    files(root)?
        .iter()
        .map(|path| {
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(path)?;
            let sha = hash_bytes(&bytes);
            ensure!(
                path.file_name().and_then(|s| s.to_str()) == Some(sha.as_str()),
                "CAS filename differs from object bytes"
            );
            Ok(EvidenceObjectEntry {
                path: relative,
                sha256: sha,
                len: bytes.len() as u64,
            })
        })
        .collect()
}

fn file_inventory(root: &Path) -> Result<Vec<EvidenceObjectEntry>> {
    files(root)?
        .iter()
        .map(|path| {
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(path)?;
            Ok(EvidenceObjectEntry {
                path: relative,
                sha256: hash_bytes(&bytes),
                len: bytes.len() as u64,
            })
        })
        .collect()
}

pub struct UniversalBundle {
    root: PathBuf,
    pub manifest: BundleManifestV2,
    pub adapter: AdapterMetadata,
    pub records: Vec<ReplayObservationV2>,
    pub resolved: Vec<ResolvedReplayInput>,
}

// A corpus published before an adapter existed keeps its signed `None`
// version. Attaching a later semantic adapter changes bundle metadata, not
// the immutable observations or their corpus identity.
fn corpus_manifest_matches(recorded: &CorpusManifest, described: &CorpusManifest) -> bool {
    let mut comparable = described.clone();
    if recorded.adapter_version.is_none() {
        comparable.adapter_version = None;
    }
    recorded == &comparable
}

impl UniversalBundle {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn baseline_path(&self) -> PathBuf {
        self.root.join("binaries/current.so")
    }
    pub fn evidence_store(&self) -> EvidenceStore {
        EvidenceStore::at(self.root.join("evidence"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let manifest: BundleManifestV2 =
            serde_json::from_slice(&fs::read(root.join("bundle.json"))?)?;
        ensure!(
            manifest.schema_version == 2 && manifest.digest()? == manifest.bundle_sha256,
            "schema-2 bundle manifest identity differs"
        );
        let baseline = fs::read(root.join("binaries/current.so"))?;
        ensure!(
            hash_bytes(&baseline) == manifest.baseline_program_sha256
                && baseline.len() as u64 == manifest.baseline_program_len,
            "schema-2 baseline binary differs"
        );
        let adapter_bytes = fs::read(root.join("adapters/metadata.json"))?;
        ensure!(
            hash_bytes(&adapter_bytes) == manifest.adapter_metadata_sha256,
            "schema-2 adapter metadata differs"
        );
        let adapter: AdapterMetadata = serde_json::from_slice(&adapter_bytes)?;
        ensure!(
            inventory(&root.join("evidence"))? == manifest.evidence_objects,
            "schema-2 evidence inventory differs"
        );
        ensure!(
            file_inventory(&root.join("corpus"))? == manifest.corpus_files,
            "schema-2 corpus file bytes differ"
        );
        let store = CorpusStore::open(root.join("corpus"))?;
        let records = store.load_v2()?;
        let corpus = store.describe_v2(&records)?;
        ensure!(
            corpus.canonical_hash == manifest.corpus_sha256
                && corpus.record_ids == manifest.record_ids
                && corpus.record_count == manifest.record_count
                && corpus.program_id == manifest.program_id
                && corpus.genesis_hash == manifest.genesis_hash
                && corpus.protocol.as_deref() == Some(manifest.protocol.as_str()),
            "schema-2 corpus differs from bundle manifest"
        );
        let recorded_manifest: CorpusManifest =
            serde_json::from_slice(&fs::read(store.manifest_path())?)?;
        ensure!(
            corpus_manifest_matches(&recorded_manifest, &corpus),
            "schema-2 corpus manifest differs"
        );
        let recorded_index: Vec<String> = serde_json::from_slice(&fs::read(store.corpus_path())?)?;
        ensure!(
            recorded_index == corpus.record_ids,
            "schema-2 corpus index differs"
        );
        ensure!(
            manifest.semantic_schema_version == SEMANTIC_SCHEMA_VERSION,
            "semantic schema differs from engine"
        );
        let actual_slots = SlotRange {
            first: records.iter().map(|r| r.slot).min().unwrap_or_default(),
            last: records.iter().map(|r| r.slot).max().unwrap_or_default(),
        };
        ensure!(
            actual_slots == manifest.source_slot_range,
            "source slot range differs"
        );
        let expected_adapter = crate::protocol::adapter_for(&manifest.program_id);
        let current = adapter.name == expected_adapter.map(|a| a.name()).unwrap_or("none")
            && adapter.version == expected_adapter.map(|a| a.adapter_version()).unwrap_or(0);
        let historical_none = adapter.name == "none"
            && adapter.version == 0
            && recorded_manifest.adapter_version.is_none();
        ensure!(
            current || historical_none,
            "adapter metadata differs from engine"
        );
        let evidence = EvidenceStore::at(root.join("evidence"));
        let resolved = records
            .iter()
            .map(|record| record.resolve(&evidence))
            .collect::<Result<Vec<_>>>()?;
        let binding_records = adapter.semantic_bindings.as_ref();
        if let Some(bindings) = binding_records {
            ensure!(
                current && bindings.len() == records.len(),
                "semantic binding observation set differs"
            );
        }
        for (index, (record, input)) in records.iter().zip(&resolved).enumerate() {
            ensure!(
                input.baseline_elf == baseline,
                "observation target binary differs from bundled baseline"
            );
            if record.fidelity_profile == super::model::FidelityProfile::CheckpointedExecutionV1
                || binding_records.is_some()
            {
                let (baseline_execution, _) = super::pipeline::baseline(record, input)?;
                if let Some(bindings) = binding_records {
                    let recorded = &bindings[index];
                    ensure!(
                        recorded.observation_id == record.id,
                        "semantic binding observation identity differs"
                    );
                    let derived = expected_adapter
                        .context("semantic binding adapter unavailable")?
                        .semantic_binding(
                            &input.message.transaction,
                            &input.seeds,
                            &input.baseline_elf,
                            &baseline_execution,
                        )?;
                    derived.validate(
                        &record.program_id,
                        &input.baseline_elf,
                        &baseline_execution,
                    )?;
                    ensure!(
                        recorded.binding == derived,
                        "semantic binding differs from reconstructed historical evidence"
                    );
                }
            }
        }
        Ok(Self {
            root,
            manifest,
            adapter,
            records,
            resolved,
        })
    }
}

pub fn build(corpus_dir: &Path, baseline: &Path, out: &Path) -> Result<UniversalBundle> {
    ensure!(
        !out.exists() || fs::read_dir(out)?.next().is_none(),
        "bundle output must be empty"
    );
    let store = CorpusStore::open(corpus_dir)?;
    let records = store.load_v2()?;
    let corpus = store.describe_v2(&records)?;
    let recorded: CorpusManifest = serde_json::from_slice(&fs::read(store.manifest_path())?)?;
    ensure!(
        corpus_manifest_matches(&recorded, &corpus),
        "corpus must be published before bundle build"
    );
    let baseline_bytes = fs::read(baseline)?;
    let evidence = EvidenceStore::at(corpus_dir.join("evidence"));
    let handle = crate::protocol::adapter_for(&corpus.program_id);
    let mut semantic_bindings = Vec::new();
    for record in &records {
        let resolved = record.resolve(&evidence)?;
        ensure!(
            resolved.baseline_elf == baseline_bytes,
            "record target binary differs from requested baseline"
        );
        let (baseline_execution, _) = super::pipeline::baseline(record, &resolved)?;
        if let Some(adapter) = handle {
            let binding = adapter.semantic_binding(
                &resolved.message.transaction,
                &resolved.seeds,
                &resolved.baseline_elf,
                &baseline_execution,
            )?;
            binding.validate(
                &record.program_id,
                &resolved.baseline_elf,
                &baseline_execution,
            )?;
            semantic_bindings.push(crate::semantic_binding::ObservationSemanticBinding {
                observation_id: record.id.clone(),
                binding,
            });
        }
    }
    fs::create_dir_all(out.join("corpus/records"))?;
    fs::create_dir_all(out.join("binaries"))?;
    fs::create_dir_all(out.join("adapters"))?;
    fs::create_dir_all(out.join("evidence"))?;
    for record in &records {
        fs::copy(
            store.record_path(&record.id),
            out.join("corpus/records")
                .join(format!("{}.json", record.id)),
        )?;
    }
    fs::copy(store.manifest_path(), out.join("corpus/manifest.json"))?;
    fs::copy(store.corpus_path(), out.join("corpus/corpus.json"))?;
    for path in files(evidence.root())? {
        let relative = path.strip_prefix(evidence.root())?;
        let destination = out.join("evidence").join(relative);
        fs::create_dir_all(
            destination
                .parent()
                .expect("evidence destination has parent"),
        )?;
        fs::copy(path, destination)?;
    }
    fs::write(out.join("binaries/current.so"), &baseline_bytes)?;
    let mut actions = BTreeMap::new();
    for record in &records {
        let resolved = record.execution.resolve(&evidence, &record.genesis_hash)?;
        let action = handle
            .map(|a| {
                a.semantic_action(&resolved.transaction)
                    .as_str()
                    .to_string()
            })
            .unwrap_or_else(|| "unknown".into());
        *actions.entry(action).or_insert(0usize) += 1;
    }
    let adapter = AdapterMetadata { name: handle.map(|a| a.name().into()).unwrap_or_else(|| "none".into()),
        version: handle.map(|a| a.adapter_version()).unwrap_or(0),
        supports_cpi: handle.is_some_and(|a| a.supports_cpi()),
        actions: actions.into_iter().map(|(semantic_action, observations)| ActionCoverage { semantic_action, observations }).collect(),
        limitations: vec![BundledLimitation { code: "bounded_observations".into(),
            detail: "This corpus covers only its recorded production interactions; no representativeness is claimed.".into() }],
        semantic_bindings: handle.map(|_| semantic_bindings) };
    let adapter_bytes = serde_json::to_vec_pretty(&adapter)?;
    fs::write(out.join("adapters/metadata.json"), &adapter_bytes)?;
    let mut manifest = BundleManifestV2 {
        schema_version: 2,
        program_id: corpus.program_id,
        protocol: corpus.protocol.context("schema-2 protocol missing")?,
        genesis_hash: corpus.genesis_hash,
        baseline_program_sha256: hash_bytes(&baseline_bytes),
        baseline_program_len: baseline_bytes.len() as u64,
        corpus_sha256: corpus.canonical_hash,
        record_count: records.len(),
        record_ids: records.iter().map(|r| r.id.clone()).collect(),
        source_slot_range: SlotRange {
            first: records.iter().map(|r| r.slot).min().unwrap_or_default(),
            last: records.iter().map(|r| r.slot).max().unwrap_or_default(),
        },
        adapter_metadata_sha256: hash_bytes(&adapter_bytes),
        semantic_schema_version: SEMANTIC_SCHEMA_VERSION,
        evidence_objects: inventory(&out.join("evidence"))?,
        corpus_files: file_inventory(&out.join("corpus"))?,
        bundle_sha256: String::new(),
    };
    manifest.bundle_sha256 = manifest.digest()?;
    fs::write(
        out.join("bundle.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    UniversalBundle::open(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_evidence_mutation_changes_or_invalidates_bundle_inventory() {
        let root = std::env::temp_dir().join(format!("eplyx-u4-inventory-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("programs")).unwrap();
        let original = b"historical ELF";
        let hash = hash_bytes(original);
        let path = root.join("programs").join(&hash);
        fs::write(&path, original).unwrap();
        let pinned = inventory(&root).unwrap();
        fs::write(&path, b"changed binary").unwrap();
        assert!(
            inventory(&root).is_err(),
            "shared evidence object mutation was not rejected by bundle hashing"
        );
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(pinned[0].sha256, hash);
    }
}
