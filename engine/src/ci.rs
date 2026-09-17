//! The gate a protocol team wires into their pipeline.
//!
//! Orchestration only: every piece of analysis below already exists and is
//! tested on its own. What this module adds is the order, the preflight, and a
//! deterministic exit code.
//!
//! ```text
//! open + verify bundle          exit 4 / 2 before anything executes
//! check baseline compatibility
//! load candidate
//!         ↓
//! replay the corpus             V1 fidelity gate, unchanged
//!         ↓
//! per observation:              what it can measure, what changed
//!         ↓
//! aggregate by fingerprint
//!         ↓
//! review against declarations
//!         ↓
//! report + exit code
//! ```
//!
//! Nothing here reaches the network. The bundle carries the corpus, the
//! baseline and every dependency binary, so a pull request needs no RPC URL, no
//! archive key and no keypair.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::bundle::CiBundle;
use crate::expectations::ExpectationFile;
use crate::replay::{hash_bytes, ReplayReport};
use crate::review::{
    review, FailureReason, ObservationCoverage, ObservedFinding, ReviewStatus, ReviewedFinding,
    UnmatchedExpectation,
};

pub const CI_REPORT_SCHEMA: u32 = 1;

/// Exit codes.
///
/// A team has to be able to tell "the upgrade contains a finding" from "Eplyx
/// could not complete the analysis", because those are different actions. The
/// preflight codes below happen before any VM execution and abort rather than
/// producing half a report.
pub const EXIT_PASSED: u8 = 0;
/// A configuration, fidelity or internal analysis error.
pub const EXIT_ERROR: u8 = 2;
/// The bundle and the baseline it was validated against do not match.
pub const EXIT_INCOMPATIBLE: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleRef {
    pub sha256: String,
    pub baseline_sha256: String,
    pub corpus_sha256: String,
    pub record_count: usize,
    pub program_id: String,
    pub adapter: String,
    pub adapter_version: u32,
    pub semantic_schema_version: u32,
    /// The production window the corpus was drawn from.
    pub source_slot_range: crate::bundle::SlotRange,
    /// What this corpus does not cover, carried from the bundle. A green result
    /// must never hide these, so they travel inside the report rather than
    /// being looked up somewhere else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<crate::bundle::BundledLimitation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRef {
    pub sha256: String,
    pub len: u64,
}

/// Which layer of evidence a change came from.
///
/// Three layers, kept distinct on purpose. Only the named layer is declarable;
/// the other two are still evidence, and a gate that consumed only the named
/// layer would report "no unexpected changes" for a candidate whose changes it
/// had detected and then discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLayer {
    /// Decoded by the protocol adapter, but not promoted to the public
    /// vocabulary, so no expectation can name it.
    DecodedEconomic,
    /// Bytes, balances, outcome or invocation shape, from the generic diff, on
    /// an observation whose adapter named nothing at all.
    Structural,
}

/// A real change that no expectation can currently be written against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndeclarableChange {
    pub layer: EvidenceLayer,
    /// What changed, in its own layer's vocabulary.
    pub description: String,
    pub observations: Vec<String>,
}

/// How widely the corpus can speak about one subject.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectCoverage {
    pub subject: String,
    pub observations: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub passed: bool,
    pub failure_reasons: Vec<FailureReason>,
    pub exit_code: u8,
    pub expected: usize,
    pub unexpected: usize,
    pub expected_but_exceeded: usize,
    pub stale: usize,
    pub unevaluable: usize,
}

/// The CI result contract.
///
/// Every list in it is sorted canonically — by fingerprint, then by observation
/// id — so two runs over the same inputs produce the same bytes and a diff
/// between two runs is meaningful. Execution order never reaches the output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiReport {
    pub schema_version: u32,
    pub bundle: BundleRef,
    pub candidate: CandidateRef,
    pub coverage: Vec<SubjectCoverage>,
    /// Detected changes outside the declarable vocabulary. Never empty-and-
    /// ignored: each one fails the gate, because the alternative is reporting a
    /// change as absent because it could not be named.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undeclarable: Vec<UndeclarableChange>,
    /// The review, inlined rather than nested.
    ///
    /// These were a `#[serde(flatten)]`-ed `Review`, so the Rust type read
    /// `report.findings` while the JSON carried `findings` at the top
    /// level. Anything written against the type was then wrong about the wire,
    /// which is exactly the mistake the frontend made. The type now says what
    /// the JSON says.
    pub findings: Vec<ReviewedFinding>,
    pub unmatched: Vec<UnmatchedExpectation>,
    pub failures: Vec<FailureReason>,
    pub summary: ReviewSummary,
}

impl CiReport {
    pub fn exit_code(&self) -> u8 {
        self.summary.exit_code
    }
}

/// Why the gate could not run.
///
/// Two kinds, because the fix is different. A bundle problem is repaired by
/// refreshing or re-pinning the bundle; a configuration problem is repaired in
/// the repository. Typed rather than matched on message text, so the exit code
/// cannot drift when an error string is reworded.
#[derive(Debug)]
pub enum CheckError {
    /// The bundle cannot be used for this comparison. Exit 4.
    Bundle(anyhow::Error),
    /// Configuration, fidelity, or an internal analysis failure. Exit 2.
    Configuration(anyhow::Error),
}

impl CheckError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Bundle(_) => EXIT_INCOMPATIBLE,
            Self::Configuration(_) => EXIT_ERROR,
        }
    }

    pub fn error(&self) -> &anyhow::Error {
        match self {
            Self::Bundle(error) | Self::Configuration(error) => error,
        }
    }
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.error())
    }
}

/// Run the gate.
///
/// A completed review, pass or fail, comes back as `Ok`. A [`CheckError`] means
/// the analysis could not run at all, which is a different thing for a team to
/// act on and gets its own exit code.
pub fn check(
    bundle_dir: &Path,
    candidate: &Path,
    expectations: Option<&Path>,
) -> std::result::Result<CiReport, CheckError> {
    // ---- preflight: nothing executes until all of this holds ------------
    let bundle = CiBundle::open(bundle_dir)
        .context("opening the CI bundle")
        .map_err(CheckError::Bundle)?;

    preflight(&bundle).map_err(CheckError::Bundle)?;

    let candidate_bytes = std::fs::read(candidate)
        .with_context(|| format!("reading the candidate program {}", candidate.display()))
        .map_err(CheckError::Configuration)?;
    let candidate_sha256 = hash_bytes(&candidate_bytes);

    let baseline_bytes = std::fs::read(bundle.baseline())
        .context("reading the bundled baseline")
        .map_err(CheckError::Bundle)?;

    let declarations = match expectations {
        Some(path) => ExpectationFile::load(path).map_err(CheckError::Configuration)?,
        None => ExpectationFile::empty(),
    };

    // ---- replay ----------------------------------------------------------
    let v1 = crate::executor::ProgramVersion {
        label: "baseline".to_string(),
        bytes: baseline_bytes,
    };
    let v2 = crate::executor::ProgramVersion {
        label: "candidate".to_string(),
        bytes: candidate_bytes.clone(),
    };
    let dependencies = crate::replay::load_dependencies(bundle.records(), &bundle.dependencies())
        .map_err(CheckError::Bundle)?;
    let replay =
        crate::replay::compare_with_dependencies(bundle.records(), &v1, &v2, &dependencies)
            .context("replaying the validated corpus")
            .map_err(CheckError::Configuration)?;

    Ok(assemble(
        &bundle,
        candidate_sha256,
        candidate_bytes.len() as u64,
        &replay,
        &declarations,
    ))
}

/// Changes the engine detected that no expectation can name.
///
/// Two sources, in order of specificity. An adapter that decodes a field but has
/// not promoted it produces a `DecodedEconomic` entry — the manager fee on a
/// stake-pool withdrawal is exactly this. An observation whose adapter named
/// nothing at all, yet whose bytes, balances, outcome or invocation shape
/// moved, produces a `Structural` entry; that is the case for every protocol
/// with no semantic surface yet, where the alternative is a green check over a
/// change the engine plainly saw.
///
/// Compute is excluded, as everywhere else: any recompilation moves it.
fn undeclarable_changes(bundle: &CiBundle, replay: &ReplayReport) -> Vec<UndeclarableChange> {
    let adapter = crate::protocol::adapter_for(&bundle.manifest().program_id);

    let mut decoded: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut structural: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    let diffs: BTreeMap<&str, &crate::diff::StateDiff> = replay
        .analysis
        .diffs
        .iter()
        .map(|diff| (diff.fixture_id.as_str(), diff))
        .collect();

    for observation in &replay.observations {
        // What the findings *actually emitted for this observation* account
        // for. Not a static list: `pool-mint/supply` is the burn on a
        // withdrawal and a by-product of the mint on a deposit, and treating it
        // as spoken for either way suppressed it even when nothing was named.
        let explained: BTreeSet<(&str, &str)> = observation
            .named_findings
            .iter()
            .filter_map(|finding| {
                adapter.and_then(|a| a.decoded_source_of(finding.fingerprint.subject.as_str()))
            })
            .collect();
        let outcome_named = observation
            .named_findings
            .iter()
            .any(|f| f.fingerprint.domain == crate::semantics::FindingDomain::Execution);

        // Every decoded change, whether a finding named it or it is reported
        // below as undeclarable. Either way it is *visible in the report*, and
        // the structural layer exists to catch what is not - repeating it as
        // raw bytes would describe one event twice.
        let mut reported: BTreeSet<(&str, &str)> = BTreeSet::new();
        for change in &observation.economic_changes {
            let key = (change.account_label.as_str(), change.field.as_str());
            reported.insert(key);
            if explained.contains(&key) {
                continue;
            }
            decoded
                .entry(format!("{} {}", change.account_label, change.field))
                .or_default()
                .insert(observation.id.clone());
        }
        let accounted_accounts: BTreeSet<&str> = reported.iter().map(|(label, _)| *label).collect();

        // Structural evidence is examined for every observation. Suppressing it
        // wholesale as soon as anything was named let a candidate hide an
        // unrelated mutation behind one declared change: altering a manager key
        // while also reducing a share calculation is two things, and only one
        // of them is nameable.
        let Some(diff) = diffs.get(observation.id.as_str()) else {
            continue;
        };
        for difference in &diff.differences {
            let label = match difference {
                // Any recompilation moves compute.
                crate::diff::Difference::ComputeChanged { .. } => continue,
                crate::diff::Difference::SuccessChanged { .. } => {
                    if outcome_named {
                        continue;
                    }
                    "transaction outcome".to_string()
                }
                crate::diff::Difference::CpiChanged { .. } => "invocation shape".to_string(),
                crate::diff::Difference::BalanceChanged { account, .. } => {
                    if reported.contains(&(account.as_str(), "lamports")) {
                        continue;
                    }
                    format!("{account} lamports")
                }
                crate::diff::Difference::RawDataChanged {
                    account, v1, v2, ..
                } => {
                    // Bytes are explained only where an economic finding
                    // demonstrably covers them. A decoded field describes a
                    // range; anything differing outside every such range is
                    // state nothing named.
                    match unexplained_offset(adapter, account, v1, v2, &accounted_accounts) {
                        None => continue,
                        Some(offset) => format!("{account} bytes at offset {offset}"),
                    }
                }
                crate::diff::Difference::FieldChanged { account, field, .. } => {
                    format!("{account} {field}")
                }
                crate::diff::Difference::LiquidationStatusChanged { account, .. } => {
                    format!("{account} liquidation status")
                }
            };
            structural
                .entry(label)
                .or_default()
                .insert(observation.id.clone());
        }
    }

    let mut out: Vec<UndeclarableChange> = decoded
        .into_iter()
        .map(|(description, observations)| UndeclarableChange {
            layer: EvidenceLayer::DecodedEconomic,
            description,
            observations: observations.into_iter().collect(),
        })
        .chain(
            structural
                .into_iter()
                .map(|(description, observations)| UndeclarableChange {
                    layer: EvidenceLayer::Structural,
                    description,
                    observations: observations.into_iter().collect(),
                }),
        )
        .collect();
    out.sort_by(|a, b| {
        a.layer
            .cmp(&b.layer)
            .then_with(|| a.description.cmp(&b.description))
    });
    out
}

/// The first differing byte an economic finding does not account for.
///
/// `None` means every difference falls inside a field the adapter decoded *and*
/// reported a change for on this account. Any other byte belongs to state the
/// adapter does not interpret, so no finding can speak for it.
fn unexplained_offset(
    adapter: Option<&'static dyn crate::protocol::ProtocolAdapter>,
    account: &str,
    v1_hex: &str,
    v2_hex: &str,
    accounted_accounts: &BTreeSet<&str>,
) -> Option<usize> {
    let (Ok(before), Ok(after)) = (crate::hexfmt::decode(v1_hex), crate::hexfmt::decode(v2_hex))
    else {
        // Undecodable evidence is not evidence of nothing.
        return Some(0);
    };
    if before.len() != after.len() {
        return Some(before.len().min(after.len()));
    }
    let ranges = match adapter {
        Some(adapter) if accounted_accounts.contains(account) => {
            adapter.decoded_byte_ranges(account)
        }
        // Nothing was reported for this account, so nothing explains any of it.
        _ => &[],
    };
    before
        .iter()
        .zip(after.iter())
        .enumerate()
        .find(|(offset, (a, b))| a != b && !ranges.iter().any(|r| r.contains(offset)))
        .map(|(offset, _)| offset)
}

/// Everything that must hold about the bundle before a single VM runs.
///
/// `require_baseline` against the bundle's own file would be tautological - it
/// would compare the baseline to itself and never fail. The checks that can
/// actually fail are these: that the records agree the bundled baseline is what
/// they were validated against, and that this build's adapter is the one the
/// bundle was built under. The second is what stops a pull request from going
/// green last week and red today because the interpretation moved underneath
/// it.
fn preflight(bundle: &CiBundle) -> Result<()> {
    check_compatibility(bundle.records(), bundle.manifest(), bundle.adapter())
}

/// The compatibility rules, over the pieces rather than over a directory.
///
/// Separated so each branch is reachable in a test without forging a bundle
/// whose hashes agree with the fault being tested.
fn check_compatibility(
    records: &[crate::replay::ReplayRecord],
    manifest: &crate::bundle::BundleManifest,
    adapter_metadata: &crate::bundle::AdapterMetadata,
) -> Result<()> {
    for record in records {
        // Not guaranteed just because our builder enforces it: a bundle can
        // arrive from anywhere, and this is the claim the whole comparison
        // rests on.
        if record.current_program_sha256 != manifest.baseline_program_sha256 {
            anyhow::bail!(
                "bundle baseline mismatch\n  \
                 record {} was validated against: {}\n  \
                 the bundle carries:              {}\n  \
                 Refusing comparison: the historical records prove nothing about \
                 a baseline they were not replayed against.",
                record.id,
                record.current_program_sha256,
                manifest.baseline_program_sha256
            );
        }
        if record.program_id != manifest.program_id {
            anyhow::bail!(
                "bundle holds record {} for program {}, but declares {}",
                record.id,
                record.program_id,
                manifest.program_id
            );
        }
    }

    // The vocabulary the bundle's subjects were named under. Checked
    // independently of the adapter version: an adapter fix must not invalidate
    // a bundle, but a change in what a subject means must, and only this pin
    // can say so.
    if manifest.semantic_schema_version != crate::semantics::SEMANTIC_SCHEMA_VERSION {
        anyhow::bail!(
            "this bundle names its subjects under semantic schema {}, and this build speaks {}. \
             A subject may no longer mean the same quantity, so the comparison is refused. \
             Rebuild the bundle.",
            manifest.semantic_schema_version,
            crate::semantics::SEMANTIC_SCHEMA_VERSION
        );
    }
    // The adapter name must also be the adapter that will actually run, or the
    // bundle's metadata describes something else entirely.
    if let Some(adapter) = crate::protocol::adapter_for(&manifest.program_id) {
        if adapter.name() != adapter_metadata.name {
            anyhow::bail!(
                "bundle metadata names adapter {:?}, but program {} resolves to {:?}",
                adapter_metadata.name,
                manifest.program_id,
                adapter.name()
            );
        }
    }

    // The adapter decides what a subject means. A bundle built under a
    // different interpretation cannot be reviewed against declarations written
    // for this one.
    if let Some(adapter) = crate::protocol::adapter_for(&manifest.program_id) {
        if adapter.adapter_version() != adapter_metadata.version {
            anyhow::bail!(
                "this bundle was built under {} adapter v{}, and this build speaks v{}. \
                 Subjects may no longer mean the same thing, so the comparison is refused. \
                 Rebuild the bundle.",
                adapter_metadata.name,
                adapter_metadata.version,
                adapter.adapter_version()
            );
        }
    }
    Ok(())
}

/// Turn a replay report and a declaration file into a reviewed CI result.
///
/// Split out from [`check`] so the review half can be exercised without a VM.
pub fn assemble(
    bundle: &CiBundle,
    candidate_sha256: String,
    candidate_len: u64,
    replay: &ReplayReport,
    declarations: &ExpectationFile,
) -> CiReport {
    let mut observed = Vec::new();
    let mut coverage = Vec::new();
    for observation in &replay.observations {
        coverage.push(ObservationCoverage {
            observation_id: observation.id.clone(),
            subjects: observation.evaluable_subjects.clone(),
        });
        for finding in &observation.named_findings {
            observed.push(ObservedFinding {
                observation_id: observation.id.clone(),
                entity: observation.economic_entity.clone(),
                finding: finding.clone(),
            });
        }
    }
    // Canonical order in, canonical order out: the review sorts by fingerprint
    // internally, and sorting the input too keeps the result independent of the
    // order records happened to replay in.
    observed.sort_by(|a, b| {
        a.finding
            .fingerprint
            .cmp(&b.finding.fingerprint)
            .then_with(|| a.observation_id.cmp(&b.observation_id))
    });
    coverage.sort_by(|a, b| a.observation_id.cmp(&b.observation_id));

    let mut reviewed = review(&observed, &coverage, declarations);

    // The other two evidence layers. Without these the gate consumes only what
    // the adapter chose to name, and a change it decoded but did not promote
    // disappears into a passing report.
    let undeclarable = undeclarable_changes(bundle, replay);
    if coverage.iter().all(|entry| entry.subjects.is_empty()) {
        reviewed.failures.push(FailureReason::NoSemanticCoverage);
    }
    if !undeclarable.is_empty() {
        reviewed.failures.push(FailureReason::UndeclarableChange);
    }
    // Precedence, so `failures.first()` remains the exit code: an analysis that
    // could not be performed outranks any verdict derived from it.
    reviewed.failures.sort();
    reviewed.failures.dedup();

    let mut per_subject: BTreeMap<String, usize> = BTreeMap::new();
    for observation in &coverage {
        for subject in observation
            .subjects
            .iter()
            .map(|s| s.to_string())
            .collect::<std::collections::BTreeSet<_>>()
        {
            *per_subject.entry(subject).or_default() += 1;
        }
    }

    let summary = ReviewSummary {
        passed: reviewed.passed(),
        failure_reasons: reviewed.failures.clone(),
        exit_code: reviewed.exit_code(),
        expected: reviewed.count(ReviewStatus::Expected),
        unexpected: reviewed.count(ReviewStatus::Unexpected),
        expected_but_exceeded: reviewed.count(ReviewStatus::ExpectedButExceeded),
        stale: reviewed.count(ReviewStatus::Stale),
        unevaluable: reviewed.count(ReviewStatus::Unevaluable),
    };

    let manifest = bundle.manifest();
    CiReport {
        schema_version: CI_REPORT_SCHEMA,
        bundle: BundleRef {
            sha256: manifest.bundle_sha256.clone(),
            baseline_sha256: manifest.baseline_program_sha256.clone(),
            corpus_sha256: manifest.corpus_sha256.clone(),
            record_count: manifest.record_count,
            program_id: manifest.program_id.clone(),
            adapter: bundle.adapter().name.clone(),
            adapter_version: bundle.adapter().version,
            // The bundle's own pin, not the running engine's constant: a
            // report must say which vocabulary the result speaks, and preflight
            // has already proved the two agree.
            semantic_schema_version: manifest.semantic_schema_version,
            source_slot_range: manifest.source_slot_range,
            limitations: bundle.adapter().limitations.clone(),
        },
        candidate: CandidateRef {
            sha256: candidate_sha256,
            len: candidate_len,
        },
        undeclarable,
        findings: reviewed.findings,
        unmatched: reviewed.unmatched,
        failures: reviewed.failures,
        coverage: per_subject
            .into_iter()
            .map(|(subject, observations)| SubjectCoverage {
                subject,
                observations,
            })
            .collect(),
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{AdapterMetadata, BundleManifest, SlotRange};
    use crate::replay::ReplayRecord;
    use crate::semantics::SEMANTIC_SCHEMA_VERSION;

    fn record() -> ReplayRecord {
        serde_json::from_str(include_str!(
            "../../docs/examples/mainnet-stake-pool-record.json"
        ))
        .expect("committed record")
    }

    fn manifest(record: &ReplayRecord) -> BundleManifest {
        BundleManifest {
            schema_version: crate::bundle::CI_BUNDLE_SCHEMA,
            program_id: record.program_id.clone(),
            genesis_hash: record.genesis_hash.clone(),
            baseline_program_sha256: record.current_program_sha256.clone(),
            baseline_program_len: 1,
            corpus_sha256: "corpus".into(),
            record_count: 1,
            record_ids: vec![record.id.clone()],
            source_slot_range: SlotRange {
                first: record.transaction.slot,
                last: record.transaction.slot,
            },
            dependencies: Vec::new(),
            adapter_metadata_sha256: "adapter".into(),
            semantic_schema_version: crate::semantics::SEMANTIC_SCHEMA_VERSION,
            selection_policy: None,
            selection_policy_version: None,
            bundle_sha256: "bundle".into(),
        }
    }

    fn adapter_metadata() -> AdapterMetadata {
        let adapter = crate::protocol::adapter_for(&record().program_id).expect("adapter");
        AdapterMetadata {
            name: adapter.name().to_string(),
            version: adapter.adapter_version(),
            supports_cpi: adapter.supports_cpi(),
            actions: Vec::new(),
            limitations: Vec::new(),
        }
    }

    #[test]
    fn a_consistent_bundle_passes_preflight() {
        let record = record();
        check_compatibility(
            std::slice::from_ref(&record),
            &manifest(&record),
            &adapter_metadata(),
        )
        .expect("consistent");
    }

    /// The claim the whole comparison rests on, and not guaranteed merely
    /// because our own builder enforces it: a bundle can arrive from anywhere.
    #[test]
    fn a_record_validated_against_another_baseline_is_refused() {
        let record = record();
        let mut manifest = manifest(&record);
        manifest.baseline_program_sha256 = "77ac".repeat(16);
        let error =
            check_compatibility(&[record], &manifest, &adapter_metadata()).expect_err("refuse");
        let text = format!("{error:#}");
        assert!(text.contains("bundle baseline mismatch"), "{text}");
        assert!(text.contains("Refusing comparison"), "{text}");
    }

    #[test]
    fn a_record_for_another_program_is_refused() {
        let mut record = record();
        let manifest = manifest(&record);
        record.program_id = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".into();
        let error =
            check_compatibility(&[record], &manifest, &adapter_metadata()).expect_err("refuse");
        assert!(format!("{error:#}").contains("but declares"), "{error:#}");
    }

    /// The check that stops a pull request going green last week and red today
    /// because the interpretation moved underneath it.
    /// A change in what a subject means invalidates a bundle even though the
    /// adapter version has not moved.
    #[test]
    fn a_bundle_naming_subjects_under_another_vocabulary_is_refused() {
        let record = record();
        let mut manifest = manifest(&record);
        manifest.semantic_schema_version = crate::semantics::SEMANTIC_SCHEMA_VERSION + 1;
        let error = check_compatibility(
            std::slice::from_ref(&record),
            &manifest,
            &adapter_metadata(),
        )
        .expect_err("refuse");
        assert!(
            format!("{error:#}").contains("semantic schema"),
            "{error:#}"
        );
    }

    #[test]
    fn a_bundle_naming_the_wrong_adapter_is_refused() {
        let record = record();
        let mut metadata = adapter_metadata();
        metadata.name = "some-other-protocol".to_string();
        let error =
            check_compatibility(std::slice::from_ref(&record), &manifest(&record), &metadata)
                .expect_err("refuse");
        assert!(format!("{error:#}").contains("resolves to"), "{error:#}");
    }

    #[test]
    fn a_bundle_built_under_another_adapter_version_is_refused() {
        let record = record();
        let mut metadata = adapter_metadata();
        metadata.version += 1;
        let error =
            check_compatibility(std::slice::from_ref(&record), &manifest(&record), &metadata)
                .expect_err("refuse");
        let text = format!("{error:#}");
        assert!(text.contains("Rebuild the bundle"), "{text}");
    }

    /// Build a real bundle from the committed record, re-pinned to synthetic
    /// binaries, so `assemble` can be driven without a VM.
    fn bundle(scratch: &std::path::Path) -> crate::bundle::CiBundle {
        use crate::dependencies::{DependencyDiscovery, ProgramDependency, ProgramSource};
        let baseline = vec![7_u8; 512];
        let mut record = record();
        record.current_program_sha256 = crate::replay::hash_bytes(&baseline);
        record.dependencies.programs = vec![ProgramDependency {
            program_id: record.program_id.clone(),
            source: ProgramSource::HistoricalMainnet,
            loader: None,
            deployed_slot: None,
            binary_sha256: Some(crate::replay::hash_bytes(&baseline)),
            binary_len: Some(baseline.len() as u64),
            observed_slot: None,
            discovered_by: vec![DependencyDiscovery::ProgramUnderTest],
            note: None,
        }];
        let baseline_path = scratch.join("current.so");
        std::fs::write(&baseline_path, &baseline).unwrap();
        std::fs::create_dir_all(scratch.join("deps")).unwrap();
        crate::bundle::build(
            crate::bundle::BundleInputs {
                records: std::slice::from_ref(&record),
                baseline: &baseline_path,
                dependencies: &scratch.join("deps"),
                selection_policy: None,
                selection_policy_version: None,
                limitations: Vec::new(),
                validation: crate::bundle::Validation::Skip {
                    reason: "this fixture pins a 512-byte stand-in, not an executable program",
                },
            },
            &scratch.join("bundle"),
        )
        .expect("bundle")
    }

    fn execution() -> crate::executor::ExecutionResult {
        crate::executor::ExecutionResult {
            version: "v".into(),
            success: true,
            error: None,
            compute_units: Some(1),
            fee: 0,
            logs: Vec::new(),
            cpi_calls: Vec::new(),
            accounts: BTreeMap::new(),
        }
    }

    /// A replay report with no semantic surface at all: the shape every adapter
    /// that has not implemented emission produces.
    ///
    /// Built from the real constructors rather than hand-written JSON, so it
    /// cannot drift away from the report the engine actually emits.
    fn replay_without_semantics(
        id: &str,
        differences: Vec<crate::diff::Difference>,
    ) -> ReplayReport {
        let mut record = record();
        record.id = id.to_string();
        let fixture = record.fixture();
        let diff = crate::diff::StateDiff {
            fixture_id: id.to_string(),
            category: fixture.category,
            scenario: fixture.scenario.clone(),
            notes: fixture.notes.clone(),
            differences,
            v1: execution(),
            v2: execution(),
        };
        let observation = crate::replay::ReplayObservation {
            id: id.to_string(),
            source_signature: record.transaction.signature.clone(),
            source_slot: record.transaction.slot,
            state_source: record.state_source.clone(),
            fidelity: crate::replay::ReplayFidelity::Matched,
            pre_state_hash: record.pre_state_hash.clone(),
            post_v1_state_hash: String::new(),
            post_v2_state_hash: String::new(),
            native_transfer_lamports: None,
            candidate_prevented_native_transfer_lamports: None,
            // The whole point of the fixture: the adapter named nothing.
            economic_changes: Vec::new(),
            economic_summary: Vec::new(),
            economic_entity: None,
            evaluable_subjects: Vec::new(),
            named_findings: Vec::new(),
            dependency_programs: Vec::new(),
            original_cpi_graph: Vec::new(),
            cpi_graph_changed: false,
            cpi_graph_v1: Vec::new(),
            cpi_graph_v2: Vec::new(),
        };
        ReplayReport {
            schema_version: crate::replay::REPLAY_SCHEMA,
            economic_findings: 0,
            timings: Vec::new(),
            observations: vec![observation],
            analysis: crate::report::Report::new(
                record.program_id.clone(),
                "v1".to_string(),
                "v2".to_string(),
                std::slice::from_ref(&fixture),
                vec![diff],
            ),
            native_impact: None,
        }
    }

    /// The exact failure the audit reproduced: an adapter with no semantic
    /// surface produced empty coverage and empty findings, and the gate called
    /// that a clean analysis while the replay had detected a change.
    #[test]
    fn empty_semantic_coverage_is_never_a_pass() {
        let scratch = tempfile::tempdir().unwrap();
        let bundle = bundle(scratch.path());
        let replay = replay_without_semantics(
            "obs-1",
            vec![crate::diff::Difference::RawDataChanged {
                account: "destination".into(),
                offset: 64,
                v1: "00".into(),
                v2: "01".into(),
            }],
        );

        let report = assemble(
            &bundle,
            "candidate".to_string(),
            1,
            &replay,
            &ExpectationFile::empty(),
        );
        assert!(
            report.coverage.is_empty(),
            "no adapter surface, by construction"
        );
        assert!(
            !report.summary.passed,
            "a pass here means 'we did not look', not 'nothing changed'"
        );
        assert!(report
            .summary
            .failure_reasons
            .contains(&FailureReason::NoSemanticCoverage));
        assert_eq!(report.summary.exit_code, 2);
    }

    /// A change the generic layer saw, on an observation nothing semantic spoke
    /// for, must survive into the report rather than being dropped at the CI
    /// boundary.
    #[test]
    fn a_structural_change_with_no_semantic_surface_is_reported() {
        let scratch = tempfile::tempdir().unwrap();
        let bundle = bundle(scratch.path());
        let replay = replay_without_semantics(
            "obs-1",
            vec![crate::diff::Difference::SuccessChanged {
                v1_success: true,
                v2_success: false,
                v1_error: None,
                v2_error: Some("failed".into()),
            }],
        );
        let report = assemble(
            &bundle,
            "candidate".to_string(),
            1,
            &replay,
            &ExpectationFile::empty(),
        );
        assert_eq!(report.undeclarable.len(), 1, "{:#?}", report.undeclarable);
        assert_eq!(report.undeclarable[0].layer, EvidenceLayer::Structural);
        assert_eq!(report.undeclarable[0].description, "transaction outcome");
        assert!(!report.summary.passed);
    }

    /// Compute moves on any recompilation and is not a behavioural change.
    #[test]
    fn a_compute_only_difference_is_not_undeclarable() {
        let scratch = tempfile::tempdir().unwrap();
        let bundle = bundle(scratch.path());
        let replay = replay_without_semantics(
            "obs-1",
            vec![crate::diff::Difference::ComputeChanged {
                v1: 1000,
                v2: 1200,
                delta: 200,
                pct_bps: 2000,
            }],
        );
        let report = assemble(
            &bundle,
            "candidate".to_string(),
            1,
            &replay,
            &ExpectationFile::empty(),
        );
        assert!(report.undeclarable.is_empty());
        // Coverage is still empty, so the run still cannot be called clean -
        // but not because of compute.
        assert_eq!(
            report.summary.failure_reasons,
            vec![FailureReason::NoSemanticCoverage]
        );
    }

    /// Build a replay report for one stake-pool observation: a named finding
    /// plus whatever decoded and structural evidence the caller specifies.
    fn replay_with(
        named: Vec<crate::semantics::NamedFinding>,
        economic: Vec<crate::protocol::EconomicChange>,
        differences: Vec<crate::diff::Difference>,
    ) -> ReplayReport {
        let mut replay = replay_without_semantics("obs-1", differences);
        replay.observations[0].named_findings = named;
        replay.observations[0].economic_changes = economic;
        replay.observations[0].evaluable_subjects = vec![
            "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased"
                .parse::<crate::semantics::FindingFingerprint>()
                .expect("a valid fingerprint")
                .evaluable_subject(),
        ];
        replay
    }

    fn received_finding() -> crate::semantics::NamedFinding {
        crate::semantics::NamedFinding {
            fingerprint: "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased"
                .parse()
                .unwrap(),
            baseline: Some(crate::semantics::SemanticValue::quantity(100_000, 9)),
            candidate: Some(crate::semantics::SemanticValue::quantity(99_900, 9)),
            relative_delta_bps: None,
            severity: crate::diff::Severity::High,
        }
    }

    fn economic(label: &str, field: &str) -> crate::protocol::EconomicChange {
        crate::protocol::EconomicChange {
            account_label: label.to_string(),
            account_kind: "stake-pool".to_string(),
            field: field.to_string(),
            v1: "1".to_string(),
            v2: "2".to_string(),
            delta: None,
        }
    }

    /// One declared change must not launder an unrelated mutation. A candidate
    /// that reduces the shares a depositor receives *and* rewrites a manager key
    /// has done two things; suppressing all structural evidence because
    /// something was named hid the second behind the first.
    #[test]
    fn a_named_finding_does_not_hide_an_unrelated_byte_change() {
        let scratch = tempfile::tempdir().unwrap();
        let bundle = bundle(scratch.path());

        // Byte 1 is inside the pool's manager key, far outside the fields the
        // adapter decodes.
        let mut before = vec![0_u8; 611];
        before[0] = 1;
        let mut after = before.clone();
        after[1] ^= 0xFF;

        let replay = replay_with(
            vec![received_finding()],
            vec![economic("destination-pool-token", "amount")],
            vec![crate::diff::Difference::RawDataChanged {
                account: "stake-pool".into(),
                offset: 1,
                v1: crate::hexfmt::encode(&before),
                v2: crate::hexfmt::encode(&after),
            }],
        );
        let report = assemble(
            &bundle,
            "candidate".to_string(),
            1,
            &replay,
            &ExpectationFile::empty(),
        );
        assert!(
            report
                .undeclarable
                .iter()
                .any(|u| u.description.contains("stake-pool bytes")),
            "{:#?}",
            report.undeclarable
        );
        assert!(!report.summary.passed);
    }

    /// The counterpart: bytes a reported economic change demonstrably covers
    /// are not reported again. `total_lamports` lives at 258.
    #[test]
    fn bytes_a_reported_change_covers_are_not_reported_twice() {
        let scratch = tempfile::tempdir().unwrap();
        let bundle = bundle(scratch.path());

        let mut before = vec![0_u8; 611];
        before[0] = 1;
        let mut after = before.clone();
        after[258] ^= 0xFF;

        let replay = replay_with(
            vec![crate::semantics::NamedFinding {
                fingerprint: "spl-stake-pool/withdraw_sol/economic/pool_tokens_burned/decreased"
                    .parse()
                    .unwrap(),
                baseline: None,
                candidate: None,
                relative_delta_bps: None,
                severity: crate::diff::Severity::High,
            }],
            vec![economic("pool-mint", "supply")],
            vec![crate::diff::Difference::RawDataChanged {
                account: "stake-pool".into(),
                offset: 258,
                v1: crate::hexfmt::encode(&before),
                v2: crate::hexfmt::encode(&after),
            }],
        );
        let report = assemble(
            &bundle,
            "candidate".to_string(),
            1,
            &replay,
            &ExpectationFile::empty(),
        );
        // The mint change is named, so it is not undeclarable; the stake-pool
        // bytes are not, because nothing was reported for that account.
        assert!(
            report
                .undeclarable
                .iter()
                .any(|u| u.description.contains("stake-pool bytes")),
            "{:#?}",
            report.undeclarable
        );
    }

    /// A decoded change is suppressed only when a finding was actually emitted
    /// for it on this observation. A static promoted list said `pool-mint`
    /// supply was spoken for on every operation, including when nothing named
    /// it, so a candidate that only moved the mint passed.
    #[test]
    fn a_promoted_field_still_reports_when_nothing_named_it() {
        let scratch = tempfile::tempdir().unwrap();
        let bundle = bundle(scratch.path());
        let replay = replay_with(
            Vec::new(),
            vec![economic("pool-mint", "supply")],
            Vec::new(),
        );
        let report = assemble(
            &bundle,
            "candidate".to_string(),
            1,
            &replay,
            &ExpectationFile::empty(),
        );
        assert_eq!(
            report
                .undeclarable
                .iter()
                .filter(|u| u.description == "pool-mint supply")
                .count(),
            1,
            "{:#?}",
            report.undeclarable
        );
        assert!(!report.summary.passed);
    }

    #[test]
    fn exit_codes_are_the_documented_ones() {
        assert_eq!(EXIT_PASSED, 0);
        assert_eq!(EXIT_ERROR, 2);
        assert_eq!(EXIT_INCOMPATIBLE, 4);
        assert_eq!(
            CheckError::Bundle(anyhow::anyhow!("x")).exit_code(),
            EXIT_INCOMPATIBLE
        );
        assert_eq!(
            CheckError::Configuration(anyhow::anyhow!("x")).exit_code(),
            EXIT_ERROR
        );
        // 1, 3 and 5 belong to a completed review and are pinned in `review`.
        assert_eq!(FailureReason::UndeclaredChange.exit_code(), 1);
        assert_eq!(FailureReason::StaleExpectation.exit_code(), 3);
        assert_eq!(FailureReason::UnevaluableExpectation.exit_code(), 5);
    }

    /// A consumer has to be able to see which vocabulary a result speaks, so
    /// the report carries it rather than leaving it implicit.
    #[test]
    fn the_semantic_schema_is_reported_so_a_consumer_can_check_it() {
        let scratch = tempfile::tempdir().unwrap();
        let bundle = bundle(scratch.path());
        let report = assemble(
            &bundle,
            "candidate".to_string(),
            1,
            &replay_without_semantics("obs-1", Vec::new()),
            &ExpectationFile::empty(),
        );
        assert_eq!(
            report.bundle.semantic_schema_version,
            SEMANTIC_SCHEMA_VERSION
        );
    }
}
