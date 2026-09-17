//! Immutable, offline-executable CI bundles.
//!
//! Corpus construction is not CI. Corpus consumption is CI.
//!
//! Building a validated corpus needs an archive endpoint, a multi-hundred-block
//! scan and a funnel that discards most of what it sees. None of that belongs on
//! the path of a pull request: it needs credentials a protocol team should not
//! have to put in their CI, it takes minutes per run, and — worst of all — it
//! makes the thing under test move. A pull request that was green last week and
//! red today, because the corpus changed underneath it, teaches a team to stop
//! reading the check.
//!
//! So the two are split. A bundle is built rarely, deliberately, with network
//! access, and reviewed when it changes. A bundle is *consumed* on every pull
//! request, offline, with nothing configured: no RPC URL, no archive key, no
//! wallet, no keypair. Everything a comparison needs is inside the directory,
//! addressed by hash.
//!
//! ```text
//! eplyx-bundle/
//! ├── bundle.json              what this is, and every hash below
//! ├── corpus/                  the validated historical records
//! │   ├── manifest.json
//! │   ├── corpus.json
//! │   └── records/
//! ├── binaries/
//! │   ├── current.so           the baseline every record was validated against
//! │   └── dependencies/        pinned to the deployment live at each slot
//! └── adapters/
//!     └── metadata.json
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::corpus_store::CorpusStore;
use crate::replay::{hash_bytes, ReplayRecord};

pub const CI_BUNDLE_SCHEMA: u32 = 1;

/// The span of production the corpus was drawn from.
///
/// Reported by the gate so a team can see what window their check covers, and
/// so two bundles can be compared by more than their hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotRange {
    pub first: u64,
    pub last: u64,
}

/// One executable the replay needs, pinned by content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundledProgram {
    pub program_id: String,
    pub sha256: String,
    pub len: u64,
}

/// How many observations of each action the bundle carries.
///
/// The gate prints this as coverage. It is deliberately not a claim about
/// production's distribution — see the corpus limitations, which travel with
/// the bundle for exactly that reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCoverage {
    pub semantic_action: String,
    pub observations: usize,
}

/// What the adapter that decoded these records can and cannot do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterMetadata {
    pub name: String,
    pub version: u32,
    pub supports_cpi: bool,
    /// Actions present in this corpus, not every action the adapter can decode.
    pub actions: Vec<ActionCoverage>,
    /// Carried verbatim from corpus selection. A bundle that has lost its
    /// limitations is a bundle that overclaims.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<BundledLimitation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundledLimitation {
    pub code: String,
    pub detail: String,
}

/// The bundle's identity and every hash it pins.
///
/// Free of wall-clock time, endpoints and host names for the same reason
/// [`CorpusManifest`] is: this is an identity, and anything that varies between
/// two builds over the same evidence would destroy it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub program_id: String,
    pub genesis_hash: String,
    /// The V1 binary every record in this corpus was validated against, and the
    /// only baseline a comparison from this bundle may use.
    pub baseline_program_sha256: String,
    pub baseline_program_len: u64,
    /// [`CorpusManifest::canonical_hash`].
    pub corpus_sha256: String,
    pub record_count: usize,
    pub record_ids: Vec<String>,
    pub source_slot_range: SlotRange,
    pub dependencies: Vec<BundledProgram>,
    pub adapter_metadata_sha256: String,
    /// The vocabulary this bundle's subjects were named under.
    ///
    /// Separate from the adapter version on purpose: an adapter bugfix must not
    /// invalidate a bundle, but a change to what a subject *means* must. Without
    /// this the bundle carried no record of its own semantics at all, and a
    /// consumer could only be told the running engine's constant.
    pub semantic_schema_version: u32,
    /// Which selector produced the corpus, where one did. A bundle built from a
    /// hand-assembled store has no policy and says so rather than implying one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_policy_version: Option<u32>,
    /// SHA-256 over every field above. Computed with this field empty, so it
    /// cannot depend on itself.
    pub bundle_sha256: String,
}

impl BundleManifest {
    /// The hash of everything except the hash itself.
    fn digest(&self) -> Result<String> {
        let mut bare = self.clone();
        bare.bundle_sha256 = String::new();
        Ok(hash_bytes(
            &serde_json::to_vec(&bare).context("encoding a bundle manifest")?,
        ))
    }
}

/// A bundle on disk, with every hash checked.
#[derive(Debug)]
pub struct CiBundle {
    root: PathBuf,
    manifest: BundleManifest,
    adapter: AdapterMetadata,
    records: Vec<ReplayRecord>,
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

impl CiBundle {
    pub fn manifest_path(root: &Path) -> PathBuf {
        root.join("bundle.json")
    }
    pub fn corpus_dir(root: &Path) -> PathBuf {
        root.join("corpus")
    }
    pub fn baseline_path(root: &Path) -> PathBuf {
        root.join("binaries").join("current.so")
    }
    pub fn dependencies_dir(root: &Path) -> PathBuf {
        root.join("binaries").join("dependencies")
    }
    pub fn adapter_path(root: &Path) -> PathBuf {
        root.join("adapters").join("metadata.json")
    }

    pub fn manifest(&self) -> &BundleManifest {
        &self.manifest
    }
    pub fn adapter(&self) -> &AdapterMetadata {
        &self.adapter
    }
    pub fn records(&self) -> &[ReplayRecord] {
        &self.records
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn baseline(&self) -> PathBuf {
        Self::baseline_path(&self.root)
    }
    pub fn dependencies(&self) -> PathBuf {
        Self::dependencies_dir(&self.root)
    }

    /// Open a bundle and verify every byte it pins.
    ///
    /// Nothing is taken on the manifest's word: the baseline, each dependency
    /// and the corpus are re-hashed from disk, and the manifest is re-hashed
    /// from its own fields. A bundle that was partially copied, edited by hand,
    /// or assembled from two sources is an error here rather than a quietly
    /// different analysis later.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let manifest: BundleManifest = serde_json::from_slice(&read(&Self::manifest_path(&root))?)
            .with_context(|| format!("reading bundle manifest in {}", root.display()))?;

        if manifest.schema_version != CI_BUNDLE_SCHEMA {
            bail!(
                "bundle schema {} is not supported by this build (expected {CI_BUNDLE_SCHEMA})",
                manifest.schema_version
            );
        }
        let recomputed = manifest.digest()?;
        if recomputed != manifest.bundle_sha256 {
            bail!(
                "bundle manifest hashes to {recomputed} but declares {}; \
                 the manifest has been edited since it was built",
                manifest.bundle_sha256
            );
        }

        let baseline = read(&Self::baseline_path(&root))?;
        let digest = hash_bytes(&baseline);
        if digest != manifest.baseline_program_sha256 {
            bail!(
                "bundle baseline binary hashes to {digest} but the manifest pins {}",
                manifest.baseline_program_sha256
            );
        }

        for dependency in &manifest.dependencies {
            let path = Self::dependencies_dir(&root).join(format!("{}.so", dependency.program_id));
            let digest = hash_bytes(&read(&path)?);
            if digest != dependency.sha256 {
                bail!(
                    "bundled dependency {} hashes to {digest} but the manifest pins {}",
                    dependency.program_id,
                    dependency.sha256
                );
            }
        }

        let store = CorpusStore::open(Self::corpus_dir(&root))?;
        let records = store.load()?;
        let corpus = store.describe(&records)?;
        if corpus.canonical_hash != manifest.corpus_sha256 {
            bail!(
                "bundled corpus hashes to {} but the manifest pins {}",
                corpus.canonical_hash,
                manifest.corpus_sha256
            );
        }

        // A hash proves the bytes were not edited. It does not prove the
        // manifest's *description* of them is true: `record_count` could claim
        // 999 over one record and still hash consistently once recomputed. So
        // the claims are checked against the content they describe.
        if manifest.record_count != records.len() {
            bail!(
                "bundle manifest declares {} records but the corpus holds {}",
                manifest.record_count,
                records.len()
            );
        }
        let actual_ids: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
        if manifest.record_ids != actual_ids {
            bail!(
                "bundle manifest lists different records than the corpus holds ({} declared, {} present)",
                manifest.record_ids.len(),
                actual_ids.len()
            );
        }
        let slots: Vec<u64> = records.iter().map(|r| r.transaction.slot).collect();
        let actual_range = SlotRange {
            first: slots.iter().copied().min().unwrap_or_default(),
            last: slots.iter().copied().max().unwrap_or_default(),
        };
        if manifest.source_slot_range != actual_range {
            bail!(
                "bundle manifest declares slots {}..{} but the corpus spans {}..{}",
                manifest.source_slot_range.first,
                manifest.source_slot_range.last,
                actual_range.first,
                actual_range.last
            );
        }

        let adapter_bytes = read(&Self::adapter_path(&root))?;
        let digest = hash_bytes(&adapter_bytes);
        if digest != manifest.adapter_metadata_sha256 {
            bail!(
                "bundled adapter metadata hashes to {digest} but the manifest pins {}",
                manifest.adapter_metadata_sha256
            );
        }
        let adapter: AdapterMetadata =
            serde_json::from_slice(&adapter_bytes).context("reading adapter metadata")?;

        Ok(Self {
            records,
            root,
            manifest,
            adapter,
        })
    }

    /// Refuse a comparison whose baseline is not the binary the records were
    /// validated against.
    ///
    /// Without this the whole evidential chain breaks: the records prove that
    /// *this* binary reproduces mainnet's post-state, and a comparison against
    /// some other binary inherits none of that. It is a hard failure, never a
    /// warning, because a warning here produces a green check that means
    /// nothing.
    pub fn require_baseline(&self, candidate_baseline_sha256: &str) -> Result<()> {
        if candidate_baseline_sha256 != self.manifest.baseline_program_sha256 {
            bail!(
                "bundle baseline mismatch\n  \
                 bundle was validated against: {}\n  \
                 provided baseline:            {candidate_baseline_sha256}\n  \
                 Refusing comparison: the historical records prove nothing about \
                 a baseline they were not replayed against.",
                self.manifest.baseline_program_sha256
            );
        }
        Ok(())
    }
}

/// Whether assembling this bundle also reproduces its records.
///
/// Not a boolean and not defaulted: the caller has to name the choice, and
/// `Skip` has to give a reason, so a reviewer reading a diff can see when a
/// bundle was assembled without being validated.
#[derive(Clone, Copy, Debug)]
pub enum Validation {
    /// Run V1 for every record and refuse the bundle unless the original
    /// outcome comes back. What the product path always does.
    AgainstBaseline,
    /// Assemble only. For callers that have already validated, and for tests
    /// whose binaries are not executable programs.
    Skip { reason: &'static str },
}

/// Everything a bundle is built from.
pub struct BundleInputs<'a> {
    pub records: &'a [ReplayRecord],
    /// The V1 binary. Must be the one every record was validated against.
    pub baseline: &'a Path,
    /// Directory holding the dependency artefacts the records pin.
    pub dependencies: &'a Path,
    pub selection_policy: Option<String>,
    pub selection_policy_version: Option<u32>,
    pub limitations: Vec<BundledLimitation>,
    pub validation: Validation,
}

/// Assemble an offline-executable bundle.
///
/// The result is verified by reading it back through [`CiBundle::open`], so a
/// build that produced something unopenable fails at build time rather than in
/// somebody's pull request.
pub fn build(inputs: BundleInputs<'_>, out: &Path) -> Result<CiBundle> {
    let records = inputs.records;
    if records.is_empty() {
        bail!("refusing to build a bundle from an empty corpus");
    }

    let program_id = single("program id", records.iter().map(|r| r.program_id.clone()))?;
    let genesis_hash = single(
        "genesis hash",
        records.iter().map(|r| r.genesis_hash.clone()),
    )?;

    // Every record must have been validated against the same V1. A corpus
    // spanning an upgrade has no single baseline, and pretending otherwise
    // would silently compare half the records against a binary that never ran
    // them.
    let baseline_sha256 = single(
        "baseline program hash",
        records.iter().map(|r| r.current_program_sha256.clone()),
    )
    .context(
        "this corpus spans more than one deployment of the program under test; \
         split it at the upgrade boundary and build one bundle per baseline",
    )?;

    let baseline_bytes = read(inputs.baseline)?;
    let digest = hash_bytes(&baseline_bytes);
    if digest != baseline_sha256 {
        bail!(
            "baseline binary {} hashes to {digest}, but the records were validated against {baseline_sha256}",
            inputs.baseline.display()
        );
    }

    let dependencies =
        collect_dependencies(records, inputs.dependencies, &program_id, &baseline_sha256)?;

    match inputs.validation {
        Validation::AgainstBaseline => {
            validate_against_baseline(records, &baseline_bytes, inputs.dependencies)?
        }
        Validation::Skip { .. } => {}
    }

    // --- write the tree -------------------------------------------------
    // The corpus store is append-only, so building into a directory that
    // already holds a bundle keeps the old records and writes a manifest that
    // describes only the new ones. The result reopens happily and is wrong.
    // Building is therefore refused unless the destination is empty.
    if out.exists()
        && std::fs::read_dir(out)
            .with_context(|| format!("reading {}", out.display()))?
            .next()
            .is_some()
    {
        bail!(
            "{} is not empty. A bundle is immutable and content addressed: build into a fresh \
             directory rather than over an existing one, and activate the new bundle when it is \
             reviewed.",
            out.display()
        );
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let store = CorpusStore::open(CiBundle::corpus_dir(out))?;
    for record in records {
        store.insert(record)?;
    }
    let corpus = store.publish()?;

    write_bytes(&CiBundle::baseline_path(out), &baseline_bytes)?;
    for (dependency, bytes) in &dependencies {
        write_bytes(
            &CiBundle::dependencies_dir(out).join(format!("{}.so", dependency.program_id)),
            bytes,
        )?;
    }

    let adapter_handle = crate::protocol::adapter_for(&program_id);
    let mut actions: BTreeMap<String, usize> = BTreeMap::new();
    for record in records {
        let action = adapter_handle
            .map(|a| a.semantic_action(&record.transaction))
            .unwrap_or(crate::protocol::SemanticAction::Unknown);
        *actions.entry(action.as_str().to_string()).or_default() += 1;
    }
    let adapter = AdapterMetadata {
        name: adapter_handle
            .map(|a| a.name().to_string())
            .unwrap_or_else(|| "none".to_string()),
        version: adapter_handle.map(|a| a.adapter_version()).unwrap_or(0),
        supports_cpi: adapter_handle.is_some_and(|a| a.supports_cpi()),
        actions: actions
            .into_iter()
            .map(|(semantic_action, observations)| ActionCoverage {
                semantic_action,
                observations,
            })
            .collect(),
        // A bundle built without selection still inherits every limit of the
        // replay contract it was acquired under. Shipping an empty list would
        // let a report imply there are none.
        limitations: if inputs.limitations.is_empty() {
            crate::select::contract_limitations()
                .into_iter()
                .map(|l| BundledLimitation {
                    code: l.code,
                    detail: l.detail,
                })
                .collect()
        } else {
            inputs.limitations
        },
    };
    let adapter_bytes = serde_json::to_vec_pretty(&adapter).context("encoding adapter metadata")?;
    write_bytes(&CiBundle::adapter_path(out), &adapter_bytes)?;

    let slots: Vec<u64> = records.iter().map(|r| r.transaction.slot).collect();
    let mut manifest = BundleManifest {
        schema_version: CI_BUNDLE_SCHEMA,
        program_id,
        genesis_hash,
        baseline_program_sha256: baseline_sha256,
        baseline_program_len: baseline_bytes.len() as u64,
        corpus_sha256: corpus.canonical_hash,
        record_count: records.len(),
        record_ids: corpus.record_ids,
        source_slot_range: SlotRange {
            first: slots.iter().copied().min().unwrap_or_default(),
            last: slots.iter().copied().max().unwrap_or_default(),
        },
        dependencies: dependencies.iter().map(|(d, _)| d.clone()).collect(),
        adapter_metadata_sha256: hash_bytes(&adapter_bytes),
        semantic_schema_version: crate::semantics::SEMANTIC_SCHEMA_VERSION,
        selection_policy: inputs.selection_policy,
        selection_policy_version: inputs.selection_policy_version,
        bundle_sha256: String::new(),
    };
    manifest.bundle_sha256 = manifest.digest()?;
    crate::ingest::write_json(&CiBundle::manifest_path(out), &manifest)?;

    CiBundle::open(out).context("verifying the bundle that was just built")
}

/// Reproduce every record under the baseline before bundling it.
///
/// This is what makes "validated corpus" a description rather than a hope.
/// Acquisition publishes what it could read; selection describes what it was
/// handed. Neither reproduces anything, so up to this point a corpus is
/// *acquired*, not validated — and a record whose V1 does not reproduce the
/// original post-state proves nothing about any candidate.
///
/// Building is the first step that holds the baseline and every dependency, so
/// it is the first step that can actually check. It runs V1 for each record and
/// refuses the bundle unless the original outcome comes back.
///
/// The replay gate at comparison time still performs this check. It is not
/// redundant: catching it here means a corpus is never published under a name
/// it has not earned, and a team is never handed a bundle that will fail on
/// their first pull request for a reason that predates their candidate.
fn validate_against_baseline(
    records: &[ReplayRecord],
    baseline: &[u8],
    dependency_dir: &Path,
) -> Result<()> {
    let v1 = crate::executor::ProgramVersion {
        label: "baseline".to_string(),
        bytes: baseline.to_vec(),
    };
    let loaded = crate::replay::load_dependencies(records, dependency_dir)
        .context("loading dependencies to validate the corpus")?;
    for record in records {
        record
            .validate()
            .with_context(|| format!("record {} is not a valid replay record", record.id))?;
        let original = record
            .execute(&v1, &loaded)
            .with_context(|| format!("replaying record {} under the baseline", record.id))?;
        let fidelity = record.fidelity(&original)?;
        if !matches!(
            fidelity,
            crate::replay::ReplayFidelity::Exact | crate::replay::ReplayFidelity::Matched
        ) {
            let failures = record.fidelity_failures(&original)?;
            bail!(
                "record {} does not reproduce under the baseline (fidelity {fidelity:?}); a \
                 corpus cannot be called validated while it contains it.{}",
                record.id,
                failures
                    .iter()
                    .map(|failure| format!("\n  - {failure}"))
                    .collect::<String>()
            );
        }
    }
    Ok(())
}

/// One value, or an error naming how many there were.
fn single(what: &str, values: impl Iterator<Item = String>) -> Result<String> {
    let distinct: std::collections::BTreeSet<String> = values.filter(|v| !v.is_empty()).collect();
    match distinct.len() {
        1 => Ok(distinct.into_iter().next().expect("one")),
        0 => bail!("the corpus declares no {what}"),
        n => bail!(
            "the corpus declares {n} different values for {what}: {}",
            distinct.into_iter().collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Every dependency artefact the corpus needs, verified against the hash the
/// records pin.
///
/// Dependencies are addressed flat, as `<program id>.so`, because that is how a
/// replay looks them up. So two records that pin *different bytes* for the same
/// program cannot share one directory. That happens when a corpus spans an
/// upgrade of a dependency rather than of the program under test, and it is an
/// error here: silently keeping one of the two would replay half the corpus
/// against a program version that was not live at its slot.
fn collect_dependencies(
    records: &[ReplayRecord],
    source: &Path,
    program_under_test: &str,
    baseline_sha256: &str,
) -> Result<Vec<(BundledProgram, Vec<u8>)>> {
    let mut pinned: BTreeMap<String, (String, String)> = BTreeMap::new();
    for record in records {
        for dependency in record.dependencies.loadable() {
            // The program under test is the one thing a comparison varies, so
            // it is supplied as the two artefacts rather than from the bundle's
            // dependency set - the same rule `replay::load_dependencies`
            // applies. Its pinned hash must still agree with the baseline,
            // because that is the record's own claim about what it replayed.
            if dependency.program_id == program_under_test {
                if dependency.binary_sha256.as_deref() != Some(baseline_sha256) {
                    bail!(
                        "record {} pins {} as the program under test but was validated against {baseline_sha256}",
                        record.id,
                        dependency.binary_sha256.clone().unwrap_or_else(|| "nothing".into())
                    );
                }
                continue;
            }
            let Some(sha256) = dependency.binary_sha256.clone() else {
                bail!(
                    "record {} needs program {} but pins no hash for it",
                    record.id,
                    dependency.program_id
                );
            };
            if let Some((existing, first_record)) = pinned.get(&dependency.program_id) {
                if existing != &sha256 {
                    bail!(
                        "the corpus needs two different builds of program {}:\n  \
                         {first_record} pins {existing}\n  \
                         {} pins {sha256}\n  \
                         A bundle addresses dependencies by program id, so it cannot carry both. \
                         Split the corpus at that dependency's upgrade boundary.",
                        dependency.program_id,
                        record.id
                    );
                }
            } else {
                pinned.insert(dependency.program_id.clone(), (sha256, record.id.clone()));
            }
        }
    }

    let mut collected = Vec::new();
    for (program_id, (sha256, _)) in pinned {
        let path = source.join(format!("{program_id}.so"));
        let bytes = read(&path).with_context(|| {
            format!("the corpus needs program {program_id}; run `eplyx historical acquire` to rebuild its dependencies")
        })?;
        let digest = hash_bytes(&bytes);
        if digest != sha256 {
            bail!(
                "dependency {program_id} at {} hashes to {digest}, but the corpus pins {sha256}",
                path.display()
            );
        }
        let len = bytes.len() as u64;
        collected.push((
            BundledProgram {
                program_id,
                sha256,
                len,
            },
            bytes,
        ));
    }
    Ok(collected)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dependencies::{DependencyDiscovery, ProgramDependency, ProgramSource};

    /// A unique scratch directory per test, removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "eplyx-bundle-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch");
            Self(path)
        }
        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Stand-in program bytes. The bundle checks hashes, not ELF validity, and
    /// a real binary cannot be committed for a unit test.
    fn program_bytes(tag: u8) -> Vec<u8> {
        vec![tag; 512]
    }

    /// A committed mainnet record, re-pinned to synthetic binaries so the whole
    /// build path — baseline check, dependency collection, hash verification —
    /// runs against bytes the test controls.
    fn record(id: &str, baseline: &[u8], dependency: (&str, &[u8])) -> ReplayRecord {
        let mut record: ReplayRecord = serde_json::from_str(include_str!(
            "../../docs/examples/mainnet-stake-pool-record.json"
        ))
        .expect("committed mainnet record");
        record.id = id.to_string();
        record.current_program_sha256 = hash_bytes(baseline);
        record.dependencies.programs = vec![
            ProgramDependency {
                program_id: record.program_id.clone(),
                source: ProgramSource::HistoricalMainnet,
                loader: None,
                deployed_slot: None,
                binary_sha256: Some(hash_bytes(baseline)),
                binary_len: Some(baseline.len() as u64),
                observed_slot: None,
                discovered_by: vec![DependencyDiscovery::ProgramUnderTest],
                note: None,
            },
            ProgramDependency {
                program_id: dependency.0.to_string(),
                source: ProgramSource::HistoricalMainnet,
                loader: None,
                deployed_slot: None,
                binary_sha256: Some(hash_bytes(dependency.1)),
                binary_len: Some(dependency.1.len() as u64),
                observed_slot: None,
                discovered_by: vec![DependencyDiscovery::InnerInstruction],
                note: None,
            },
        ];
        record
    }

    const DEP: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

    /// A scratch directory holding a baseline, a dependency directory and the
    /// records that pin them.
    struct Fixture {
        scratch: Scratch,
        records: Vec<ReplayRecord>,
        baseline: PathBuf,
        dependencies: PathBuf,
    }

    fn fixture(tag: &str) -> Fixture {
        let scratch = Scratch::new(tag);
        let baseline = program_bytes(1);
        let dependency = program_bytes(2);
        std::fs::write(scratch.join("current.so"), &baseline).unwrap();
        std::fs::create_dir_all(scratch.join("dependencies")).unwrap();
        std::fs::write(
            scratch.join("dependencies").join(format!("{DEP}.so")),
            &dependency,
        )
        .unwrap();
        let records = vec![
            record("obs-a", &baseline, (DEP, &dependency)),
            record("obs-b", &baseline, (DEP, &dependency)),
        ];
        Fixture {
            baseline: scratch.join("current.so"),
            dependencies: scratch.join("dependencies"),
            scratch,
            records,
        }
    }

    impl Fixture {
        fn inputs(&self) -> BundleInputs<'_> {
            BundleInputs {
                records: &self.records,
                baseline: &self.baseline,
                dependencies: &self.dependencies,
                selection_policy: Some("stratified-diversity".to_string()),
                selection_policy_version: Some(1),
                validation: Validation::Skip {
                    reason: "these fixtures pin 512-byte stand-ins, not executable programs",
                },
                limitations: vec![BundledLimitation {
                    code: "failed_original_transactions_unsupported".to_string(),
                    detail: "no failure path is represented".to_string(),
                }],
            }
        }
        fn build(&self, name: &str) -> Result<CiBundle> {
            build(self.inputs(), &self.scratch.join(name))
        }
    }

    #[test]
    fn a_built_bundle_verifies_itself() {
        let fixture = fixture("verifies");
        let bundle = fixture.build("out").expect("build");
        assert_eq!(bundle.manifest().record_count, 2);
        assert_eq!(
            bundle.manifest().dependencies.len(),
            1,
            "not the program under test"
        );
        assert_eq!(bundle.manifest().dependencies[0].program_id, DEP);
        // Reopening is a full re-verification of every byte.
        CiBundle::open(bundle.root()).expect("reopen");
    }

    /// The bundle is an identity. Two builds over the same evidence must be the
    /// same bundle, or pinning one in a repository means nothing.
    #[test]
    fn building_twice_produces_the_same_identity() {
        let fixture = fixture("identity");
        let first = fixture.build("one").expect("first");
        let second = fixture.build("two").expect("second");
        assert_eq!(
            first.manifest().bundle_sha256,
            second.manifest().bundle_sha256
        );
        assert_eq!(first.manifest(), second.manifest());
    }

    #[test]
    fn a_tampered_dependency_binary_is_refused() {
        let fixture = fixture("dep-tamper");
        let bundle = fixture.build("out").expect("build");
        let path = CiBundle::dependencies_dir(bundle.root()).join(format!("{DEP}.so"));
        std::fs::write(&path, program_bytes(9)).unwrap();
        let error = CiBundle::open(bundle.root()).expect_err("must refuse");
        assert!(error.to_string().contains(DEP), "{error}");
    }

    #[test]
    fn a_tampered_baseline_binary_is_refused() {
        let fixture = fixture("baseline-tamper");
        let bundle = fixture.build("out").expect("build");
        std::fs::write(CiBundle::baseline_path(bundle.root()), program_bytes(9)).unwrap();
        let error = CiBundle::open(bundle.root()).expect_err("must refuse");
        assert!(error.to_string().contains("baseline"), "{error}");
    }

    #[test]
    fn a_tampered_record_is_refused() {
        let fixture = fixture("record-tamper");
        let bundle = fixture.build("out").expect("build");
        let mut records = bundle.records().to_vec();
        records[0].assumptions.push("tampered".to_string());
        let path = CiBundle::corpus_dir(bundle.root())
            .join("records")
            .join(format!("{}.json", records[0].id));
        std::fs::write(&path, serde_json::to_vec(&records[0]).unwrap()).unwrap();
        let error = CiBundle::open(bundle.root()).expect_err("must refuse");
        assert!(error.to_string().contains("corpus hashes to"), "{error}");
    }

    /// Editing the manifest to agree with tampered content must not rescue it.
    #[test]
    fn an_edited_manifest_is_refused() {
        let fixture = fixture("manifest-tamper");
        let bundle = fixture.build("out").expect("build");
        let mut manifest = bundle.manifest().clone();
        manifest.record_count = 99;
        crate::ingest::write_json(&CiBundle::manifest_path(bundle.root()), &manifest).unwrap();
        let error = CiBundle::open(bundle.root()).expect_err("must refuse");
        assert!(error.to_string().contains("has been edited"), "{error}");
    }

    /// The records prove that one specific binary reproduces mainnet. A
    /// comparison against any other baseline inherits none of that evidence, so
    /// it is refused rather than run and reported.
    #[test]
    fn a_baseline_mismatch_refuses_the_comparison() {
        let fixture = fixture("baseline-mismatch");
        let bundle = fixture.build("out").expect("build");
        let expected = bundle.manifest().baseline_program_sha256.clone();

        bundle
            .require_baseline(&expected)
            .expect("the real baseline");

        let error = bundle
            .require_baseline(&hash_bytes(&program_bytes(7)))
            .expect_err("must refuse");
        let text = error.to_string();
        assert!(text.contains("Refusing comparison"), "{text}");
        assert!(text.contains(&expected), "names what it expected: {text}");
    }

    /// A corpus spanning an upgrade of the program under test has no single
    /// baseline. Picking one would compare half the records against a binary
    /// that never ran them.
    #[test]
    fn a_corpus_spanning_two_baselines_is_refused() {
        let mut fixture = fixture("two-baselines");
        fixture.records[1].current_program_sha256 = hash_bytes(&program_bytes(8));
        let error = fixture.build("out").expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("baseline program hash"),
            "{error:#}"
        );
    }

    #[test]
    fn a_baseline_that_ran_no_record_is_refused() {
        let fixture = fixture("wrong-baseline");
        std::fs::write(fixture.scratch.join("current.so"), program_bytes(9)).unwrap();
        let error = fixture.build("out").expect_err("must refuse");
        assert!(error.to_string().contains("validated against"), "{error}");
    }

    /// Dependencies are addressed flat, as `<program id>.so`, so one directory
    /// cannot hold two builds of the same program.
    #[test]
    fn conflicting_dependency_versions_are_refused_by_name() {
        let mut fixture = fixture("dep-conflict");
        fixture.records[1].dependencies.programs[1].binary_sha256 =
            Some(hash_bytes(&program_bytes(8)));
        let error = fixture.build("out").expect_err("must refuse");
        let text = error.to_string();
        assert!(text.contains(DEP), "{text}");
        assert!(
            text.contains("obs-a") && text.contains("obs-b"),
            "names both records: {text}"
        );
    }

    #[test]
    fn an_empty_corpus_is_refused() {
        let fixture = fixture("empty");
        let inputs = BundleInputs {
            records: &[],
            ..fixture.inputs()
        };
        let error = build(inputs, &fixture.scratch.join("out")).expect_err("must refuse");
        assert!(error.to_string().contains("empty corpus"), "{error}");
    }

    /// A hash proves bytes were not edited. It does not prove the manifest's
    /// description of them is true: this count hashes consistently and is a lie.
    #[test]
    fn a_manifest_that_miscounts_its_records_is_refused() {
        let fixture = fixture("miscount");
        let bundle = fixture.build("out").expect("build");
        let mut manifest = bundle.manifest().clone();
        manifest.record_count = 999;
        manifest.bundle_sha256 = String::new();
        manifest.bundle_sha256 = manifest.digest().unwrap();
        crate::ingest::write_json(&CiBundle::manifest_path(bundle.root()), &manifest).unwrap();

        let error = CiBundle::open(bundle.root()).expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("declares 999 records"),
            "{error:#}"
        );
    }

    #[test]
    fn a_manifest_that_lists_other_records_is_refused() {
        let fixture = fixture("mislist");
        let bundle = fixture.build("out").expect("build");
        let mut manifest = bundle.manifest().clone();
        manifest.record_ids = vec!["not-a-real-record".to_string(), "nor-this".to_string()];
        manifest.bundle_sha256 = String::new();
        manifest.bundle_sha256 = manifest.digest().unwrap();
        crate::ingest::write_json(&CiBundle::manifest_path(bundle.root()), &manifest).unwrap();

        let error = CiBundle::open(bundle.root()).expect_err("must refuse");
        assert!(
            format!("{error:#}").contains("different records"),
            "{error:#}"
        );
    }

    #[test]
    fn a_manifest_that_misstates_its_slot_window_is_refused() {
        let fixture = fixture("misslot");
        let bundle = fixture.build("out").expect("build");
        let mut manifest = bundle.manifest().clone();
        manifest.source_slot_range = SlotRange { first: 1, last: 2 };
        manifest.bundle_sha256 = String::new();
        manifest.bundle_sha256 = manifest.digest().unwrap();
        crate::ingest::write_json(&CiBundle::manifest_path(bundle.root()), &manifest).unwrap();

        assert!(CiBundle::open(bundle.root()).is_err());
    }

    /// The corpus store is append-only, so building over an existing bundle
    /// keeps its records while the new manifest describes only the new ones.
    /// The result would reopen happily and be wrong.
    #[test]
    fn building_over_an_existing_bundle_is_refused() {
        let fixture = fixture("rebuild");
        fixture.build("out").expect("first build");
        let error = fixture.build("out").expect_err("must refuse");
        assert!(format!("{error:#}").contains("is not empty"), "{error:#}");
    }

    /// An adapter fix must not invalidate a bundle, but a change in what a
    /// subject means must - and only a pin inside the bundle can say which
    /// happened.
    #[test]
    fn the_bundle_pins_the_vocabulary_its_subjects_were_named_under() {
        let fixture = fixture("schema-pin");
        let bundle = fixture.build("out").expect("build");
        assert_eq!(
            bundle.manifest().semantic_schema_version,
            crate::semantics::SEMANTIC_SCHEMA_VERSION
        );
    }

    /// A bundle built without selection still inherits every limit of the
    /// contract it was acquired under.
    #[test]
    fn a_bundle_without_selection_still_carries_contract_limitations() {
        let fixture = fixture("no-selection");
        let inputs = BundleInputs {
            limitations: Vec::new(),
            ..fixture.inputs()
        };
        let bundle = build(inputs, &fixture.scratch.join("out")).expect("build");
        assert!(
            !bundle.adapter().limitations.is_empty(),
            "an empty list would imply there are none"
        );
        assert!(bundle
            .adapter()
            .limitations
            .iter()
            .any(|l| l.code == "failed_original_transactions_unsupported"));
    }

    /// `AgainstBaseline` really executes. These fixtures pin stand-ins rather
    /// than programs, so asking for validation must fail — which is also the
    /// proof that `Skip` in the other tests is doing something.
    #[test]
    fn validation_against_the_baseline_actually_replays() {
        let fixture = fixture("validates");
        let inputs = BundleInputs {
            validation: Validation::AgainstBaseline,
            ..fixture.inputs()
        };
        let error = build(inputs, &fixture.scratch.join("out")).expect_err("must refuse");
        let text = format!("{error:#}");
        assert!(text.contains("obs-a"), "{text}");

        // The same inputs assemble fine when validation is skipped, which is
        // what makes the failure above evidence that it ran.
        build(fixture.inputs(), &fixture.scratch.join("skipped")).expect("assembles");
    }

    /// A bundle that has lost its limitations is a bundle that overclaims.
    #[test]
    fn coverage_limitations_travel_with_the_bundle() {
        let fixture = fixture("limitations");
        let bundle = fixture.build("out").expect("build");
        assert_eq!(bundle.adapter().limitations.len(), 1);
        assert_eq!(
            bundle.adapter().limitations[0].code,
            "failed_original_transactions_unsupported"
        );
        assert_eq!(
            bundle.manifest().selection_policy.as_deref(),
            Some("stratified-diversity")
        );
    }
}
