//! Deterministic differential execution for Solana program upgrades.
//!
//! The engine answers one question: *given identical initial state and an
//! identical transaction, what changes solely because the program version
//! changed?*
//!
//! Layering, outermost to innermost:
//!
//! ```text
//!   corpus     protocol-specific: builds fixtures (state + transaction)
//!   executor   protocol-agnostic: runs one fixture against one program build
//!   diff       protocol-agnostic: structural comparison of two results
//!   interpret  protocol-specific: turns byte deltas into economic meaning
//!   impact     protocol-specific: aggregates economics across the corpus
//!   cluster    protocol-specific: groups findings by shared trigger
//!   shrink     protocol-specific: minimizes a counterexample by re-executing
//!   money      protocol-agnostic: integer-only fixed-point USD
//!   report     protocol-agnostic: text / JSON rendering
//! ```
//!
//! Execution is bytecode-based; executor error naming and diff interpretation
//! currently use the fixture protocol. The adapter seam is a module convention.
//! Everything that understands what a health factor or a position *is* lives in
//! `corpus`, `interpret` and `impact`, which is the seam a protocol adapter
//! would plug into in a later phase.

pub mod bundle;
pub mod ci;
pub mod ci_markdown;
pub mod cluster;
pub mod corpus;
pub mod corpus_store;
pub mod dependencies;
pub mod diff;
pub mod discovery;
pub mod executor;
pub mod expectations;
pub mod hexfmt;
pub mod historical;
pub mod impact;
pub mod ingest;
pub mod interpret;
pub mod money;
pub mod numfmt;
pub mod protocol;
pub mod replay;
pub mod report;
pub mod review;
pub mod screening;
pub mod select;
pub mod semantics;
pub mod shrink;
pub mod types;
pub mod versions;

use std::path::{Path, PathBuf};

use anyhow::Result;
use solana_address::Address;

pub use cluster::{MinimizedCase, RegressionCluster};
pub use diff::{Classification, Difference, Severity, StateDiff};
pub use executor::{ExecutionResult, ProgramVersion};
pub use impact::{EconomicConsequence, EconomicImpactSummary, FixtureEconomics};
pub use money::{SignedUsd, Usd};
pub use report::Report;
pub use types::{AccountSnapshot, Category, Fixture, InstructionSpec};

/// Program ID of the fixture protocol.
///
/// V1 and V2 deliberately share an address: they are never loaded side by side,
/// only into separate VM instances, so an identical ID is both possible and the
/// honest representation of an in-place upgrade.
pub const FIXTURE_PROGRAM_ID: &str = include_str!("../../fixtures/program-id.txt");

pub fn fixture_program_id() -> Address {
    FIXTURE_PROGRAM_ID
        .trim()
        .parse()
        .expect("fixtures/program-id.txt must contain a valid base58 address")
}

/// Repository root, resolved from the crate location rather than the working
/// directory so the CLI and the test suite agree regardless of where they run.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("engine crate always has a parent directory")
        .to_path_buf()
}

pub fn default_artifact(version: &str) -> PathBuf {
    repo_root().join(format!("artifacts/fixture_lending_{version}.so"))
}

/// Load both program builds from disk.
pub fn load_versions(v1: &Path, v2: &Path) -> Result<(ProgramVersion, ProgramVersion)> {
    Ok((
        ProgramVersion::from_file("v1", v1)?,
        ProgramVersion::from_file("v2", v2)?,
    ))
}

/// Execute one fixture against both builds and diff the results.
///
/// Each side gets a freshly constructed VM seeded from the same fixture, which
/// is what makes "reset to identical initial state" a structural property rather
/// than a procedure that could drift.
pub fn compare_fixture(
    fixture: &Fixture,
    program_id: &Address,
    v1: &ProgramVersion,
    v2: &ProgramVersion,
) -> Result<StateDiff> {
    let result_v1 = executor::execute(fixture, program_id, v1)?;
    let result_v2 = executor::execute(fixture, program_id, v2)?;
    Ok(diff::compare(fixture, result_v1, result_v2))
}

pub fn compare_all(
    fixtures: &[Fixture],
    program_id: &Address,
    v1: &ProgramVersion,
    v2: &ProgramVersion,
) -> Result<Vec<StateDiff>> {
    fixtures
        .iter()
        .map(|fixture| compare_fixture(fixture, program_id, v1, v2))
        .collect()
}

/// Fill in minimized counterexamples for every critical cluster.
///
/// Separate from [`Report::new`] because minimization has to execute candidates,
/// while report construction is pure. Non-critical clusters are skipped: the
/// search costs real time and a minimized witness matters most where the finding
/// is severe.
pub fn minimize_clusters(
    report: &mut Report,
    fixtures: &[Fixture],
    program_id: &Address,
    v1: &ProgramVersion,
    v2: &ProgramVersion,
    config: shrink::ShrinkConfig,
) -> Result<()> {
    for index in 0..report.clusters.len() {
        if !report.clusters[index].critical {
            continue;
        }
        let representative_id = report.clusters[index].representative_fixture_id.clone();
        let Some(fixture) = fixtures.iter().find(|f| f.id == representative_id) else {
            continue;
        };
        let Some(state_diff) = report.diff_for(&representative_id).cloned() else {
            continue;
        };
        let Some(economics) = impact::evaluate(fixture, &state_diff) else {
            continue;
        };
        let target = cluster::signature(fixture, &state_diff, &economics);
        report.clusters[index].minimized_counterexample =
            shrink::minimize(fixture, target, program_id, v1, v2, config)?;
    }
    Ok(())
}

/// Convenience used by the test suite: generate the corpus, load the default
/// artefacts, and compare everything. Minimization is *not* run - see
/// [`compare_default_corpus_minimized`].
pub fn compare_default_corpus() -> Result<Report> {
    let program_id = fixture_program_id();
    let fixtures = corpus::generate(&program_id);
    let v1_path = default_artifact("v1");
    let v2_path = default_artifact("v2");
    let (v1, v2) = load_versions(&v1_path, &v2_path)?;
    let diffs = compare_all(&fixtures, &program_id, &v1, &v2)?;
    Ok(Report::new(
        program_id.to_string(),
        v1_path.display().to_string(),
        v2_path.display().to_string(),
        &fixtures,
        diffs,
    ))
}

/// As [`compare_default_corpus`], with minimized counterexamples filled in.
pub fn compare_default_corpus_minimized() -> Result<Report> {
    let program_id = fixture_program_id();
    let fixtures = corpus::generate(&program_id);
    let v1_path = default_artifact("v1");
    let v2_path = default_artifact("v2");
    let (v1, v2) = load_versions(&v1_path, &v2_path)?;
    let diffs = compare_all(&fixtures, &program_id, &v1, &v2)?;
    let mut report = Report::new(
        program_id.to_string(),
        v1_path.display().to_string(),
        v2_path.display().to_string(),
        &fixtures,
        diffs,
    );
    minimize_clusters(
        &mut report,
        &fixtures,
        &program_id,
        &v1,
        &v2,
        shrink::ShrinkConfig::default(),
    )?;
    Ok(report)
}
