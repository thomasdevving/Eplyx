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

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::bundle::CiBundle;
use crate::expectations::ExpectationFile;
use crate::replay::{hash_bytes, ReplayReport};
use crate::review::{
    review, FailureReason, ObservationCoverage, ObservedFinding, Review, ReviewStatus,
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
    #[serde(flatten)]
    pub review: Review,
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

    let reviewed = review(&observed, &coverage, declarations);

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
            semantic_schema_version: crate::semantics::SEMANTIC_SCHEMA_VERSION,
            source_slot_range: manifest.source_slot_range,
            limitations: bundle.adapter().limitations.clone(),
        },
        candidate: CandidateRef {
            sha256: candidate_sha256,
            len: candidate_len,
        },
        coverage: per_subject
            .into_iter()
            .map(|(subject, observations)| SubjectCoverage {
                subject,
                observations,
            })
            .collect(),
        review: reviewed,
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

    #[test]
    fn the_semantic_schema_is_reported_so_a_consumer_can_check_it() {
        assert_eq!(SEMANTIC_SCHEMA_VERSION, 1);
    }
}
