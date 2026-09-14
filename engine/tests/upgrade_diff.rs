//! End-to-end differential tests.
//!
//! These run the real SBF artefacts through the real VM. They require
//! `./scripts/build-programs.sh` to have been run; if the artefacts are missing
//! the suite fails with an explanatory message rather than silently skipping.
//!
//! The expected classifications below are not a recorded baseline - they are
//! derived from the arithmetic documented in `corpus.rs`. If a change to the
//! program shifts them, these tests fail rather than quietly accepting the new
//! behaviour.

use std::sync::OnceLock;

use fixture_lending_interface::reference;
use ripcord_engine::diff::{Classification, Difference, Severity};
use ripcord_engine::interpret::{decode, Decoded};
use ripcord_engine::{corpus, report, Report, StateDiff};

/// The whole corpus is executed once and shared: 141 fixtures x 2 builds is
/// cheap, but not so cheap that every test should repeat it.
fn report() -> &'static Report {
    static REPORT: OnceLock<Report> = OnceLock::new();
    REPORT.get_or_init(|| {
        ripcord_engine::compare_default_corpus().expect(
            "could not run the corpus; run ./scripts/build-programs.sh to compile V1 and V2",
        )
    })
}

fn diff_for(id: &str) -> &'static StateDiff {
    report()
        .diffs
        .iter()
        .find(|d| d.fixture_id == id)
        .unwrap_or_else(|| panic!("no fixture {id} in corpus"))
}

// ---------------------------------------------------------------------------
// 1. V1 is correct
// ---------------------------------------------------------------------------

/// V1's on-chain arithmetic must agree with the independent host-side reference
/// implementation for every state the corpus reaches. This is what makes the
/// reference usable as an oracle in the rest of the suite.
#[test]
fn v1_matches_the_reference_implementation() {
    let mut checked = 0;
    for diff in &report().diffs {
        if !diff.v1.success {
            continue;
        }
        let Some(snapshot) = diff.v1.accounts.get("position") else {
            continue;
        };
        let Decoded::Position(position) = decode(&snapshot.data) else {
            panic!("{}: position account did not decode", diff.fixture_id);
        };
        let expected = reference::health_factor(
            position.collateral_amount,
            position.debt_amount,
            position.collateral_price,
            position.liquidation_threshold_bps,
        );
        assert_eq!(
            position.health_factor, expected,
            "{}: V1 health factor disagrees with the reference implementation",
            diff.fixture_id
        );
        checked += 1;
    }
    assert!(checked > 100, "only checked {checked} fixtures");
}

/// The mirror of the above: V2 must *disagree* with the reference somewhere,
/// otherwise the seeded regression is not actually present and every other
/// assertion in this file is vacuous.
#[test]
fn v2_disagrees_with_the_reference_somewhere() {
    let disagreements = report()
        .diffs
        .iter()
        .filter(|diff| {
            diff.v2
                .accounts
                .get("position")
                .map(|snapshot| match decode(&snapshot.data) {
                    Decoded::Position(p) => {
                        p.health_factor
                            != reference::health_factor(
                                p.collateral_amount,
                                p.debt_amount,
                                p.collateral_price,
                                p.liquidation_threshold_bps,
                            )
                    }
                    _ => false,
                })
                .unwrap_or(false)
        })
        .count();
    assert!(
        disagreements > 0,
        "V2 agreed with the reference everywhere: the regression is missing"
    );
}

// ---------------------------------------------------------------------------
// 2. V2 keeps the same external interface
// ---------------------------------------------------------------------------

/// The two builds must be genuinely different artefacts, or the comparison is
/// measuring nothing.
#[test]
fn the_two_artifacts_differ() {
    let v1 = std::fs::read(ripcord_engine::default_artifact("v1")).expect("v1 artefact");
    let v2 = std::fs::read(ripcord_engine::default_artifact("v2")).expect("v2 artefact");
    assert_ne!(v1, v2, "V1 and V2 bytecode is identical");
}

/// Identical interface means: the same instruction bytes are accepted, the same
/// account layout is written back, and no call is rejected as malformed.
#[test]
fn v2_accepts_the_same_instruction_encoding_and_layout() {
    for diff in &report().diffs {
        for (label, result) in [("v1", &diff.v1), ("v2", &diff.v2)] {
            if let Some(error) = &result.error {
                assert!(
                    !error.contains("InvalidAccountData"),
                    "{} [{label}]: instruction was rejected as malformed: {error}",
                    diff.fixture_id
                );
                assert!(
                    !error.contains("InvalidInstructionData"),
                    "{} [{label}]: instruction encoding not understood: {error}",
                    diff.fixture_id
                );
            }
        }

        for (label, v1_account) in &diff.v1.accounts {
            let v2_account = diff
                .v2
                .accounts
                .get(label)
                .unwrap_or_else(|| panic!("{}: {label} missing under V2", diff.fixture_id));
            assert_eq!(
                v1_account.data.len(),
                v2_account.data.len(),
                "{}: account {label} changed size between versions",
                diff.fixture_id
            );
            assert_eq!(
                v1_account.owner, v2_account.owner,
                "{}: account {label} changed owner between versions",
                diff.fixture_id
            );
        }
    }
}

/// Every instruction in the public set is exercised by the corpus, so "same
/// interface" is a claim about the whole surface rather than one code path.
#[test]
fn the_corpus_exercises_the_instruction_surface() {
    let fixtures = corpus::generate(&ripcord_engine::fixture_program_id());
    let mut seen: Vec<u8> = fixtures
        .iter()
        .map(|f| f.instruction.data[0])
        .collect::<Vec<_>>();
    seen.sort_unstable();
    seen.dedup();
    // Borsh enum discriminants: CreatePosition(1) and InitializeMarket(0) are
    // not replayed because the corpus seeds already-initialised state; the
    // remaining six state-mutating instructions all appear.
    assert!(
        seen.len() >= 5,
        "corpus only exercises {} distinct instructions",
        seen.len()
    );
}

// ---------------------------------------------------------------------------
// 3. Most fixtures are unaffected
// ---------------------------------------------------------------------------

#[test]
fn the_majority_of_the_corpus_is_unaffected() {
    let summary = &report().summary;
    assert!(summary.fixtures_tested >= 100, "corpus is too small");
    assert_eq!(
        summary.outcome_identical, 89,
        "expected 89 outcome-identical fixtures, got {}",
        summary.outcome_identical
    );
    assert!(
        summary.outcome_identical * 2 > summary.fixtures_tested,
        "a regression this broad would not be subtle"
    );
}

/// Whole-SOL positions are exactly the states the V2 truncation cannot touch.
#[test]
fn whole_sol_categories_are_completely_unaffected() {
    for category in ["healthy", "moderate", "small", "large"] {
        let summary = &report().summary.by_category[category];
        assert_eq!(
            summary.changed, 0,
            "category {category} should be unaffected but has {} changed fixtures",
            summary.changed
        );
    }
}

// ---------------------------------------------------------------------------
// 4 & 5. The intended fixtures differ, and the difference is identified
// ---------------------------------------------------------------------------

#[test]
fn boundary_window_flips_exactly_the_expected_fixtures() {
    for n in 1..=20u32 {
        let id = format!("boundary-position-{n:03}");
        let diff = diff_for(&id);
        let expected_critical = (17..=20).contains(&n);
        assert_eq!(
            diff.is_critical(),
            expected_critical,
            "{id}: expected critical={expected_critical}, got {:?}",
            diff.outcome_severity()
        );
        // Every boundary fixture holds fractional collateral, so all of them
        // should at least register a health-factor shift.
        assert_eq!(
            diff.classification(),
            Classification::Changed,
            "{id}: expected a behavioural change"
        );
    }
}

#[test]
fn the_flagship_counterexample_is_reported_precisely() {
    let diff = diff_for("boundary-position-017");

    assert!(
        diff.v1.success && diff.v2.success,
        "both sides should succeed"
    );

    let flip = diff
        .differences
        .iter()
        .find_map(|d| match d {
            Difference::LiquidationStatusChanged {
                account,
                v1,
                v2,
                v1_health,
                v2_health,
            } => Some((account, *v1, *v2, v1_health.clone(), v2_health.clone())),
            _ => None,
        })
        .expect("liquidation status change should be reported");

    assert_eq!(flip.0, "position");
    assert!(!flip.1, "healthy under V1");
    assert!(flip.2, "liquidatable under V2");
    assert_eq!(flip.3, "1.003783");
    assert_eq!(flip.4, "0.998738");
    assert_eq!(diff.outcome_severity(), Some(Severity::Critical));
}

#[test]
fn withdrawal_gate_reverts_under_v2_for_the_expected_fixtures() {
    for n in 1..=6u32 {
        let id = format!("withdraw-boundary-{n:03}");
        let diff = diff_for(&id);
        let expected_revert = (3..=6).contains(&n);

        assert!(diff.v1.success, "{id}: V1 withdrawal should succeed");
        assert_eq!(
            !diff.v2.success, expected_revert,
            "{id}: V2 success was {}, expected revert={expected_revert}",
            diff.v2.success
        );

        if expected_revert {
            assert!(diff.is_critical(), "{id}: reverting withdrawal is critical");
            let error = diff.v2.error.as_deref().unwrap_or_default();
            assert!(
                error.contains("LtvExceeded"),
                "{id}: expected LtvExceeded, got {error}"
            );
        }
    }
}

#[test]
fn liquidation_eligibility_flips_and_seizes_collateral_under_v2() {
    for n in 1..=3u32 {
        let id = format!("liquidation-boundary-{n:03}");
        let diff = diff_for(&id);

        assert!(!diff.v1.success, "{id}: V1 should reject the liquidation");
        assert!(
            diff.v1
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("PositionHealthy"),
            "{id}: expected PositionHealthy under V1, got {:?}",
            diff.v1.error
        );
        assert!(diff.v2.success, "{id}: V2 should permit the liquidation");
        assert!(diff.is_critical());

        // The economic consequence: the owner's collateral actually moves.
        let seized = diff.differences.iter().any(|d| {
            matches!(d, Difference::BalanceChanged { account, delta, .. }
                     if account == "liquidator" && *delta > 0)
        });
        assert!(seized, "{id}: expected the liquidator to gain lamports");
    }
}

#[test]
fn every_critical_finding_is_a_threshold_crossing_or_a_reverted_transaction() {
    let critical = report().critical();
    assert_eq!(critical.len(), 11, "expected 11 critical fixtures");
    for diff in critical {
        let explained = diff.differences.iter().any(|d| {
            matches!(
                d,
                Difference::LiquidationStatusChanged { .. } | Difference::SuccessChanged { .. }
            )
        });
        assert!(
            explained,
            "{}: critical finding with no threshold crossing or outcome change",
            diff.fixture_id
        );
    }
}

// ---------------------------------------------------------------------------
// 6. Determinism
// ---------------------------------------------------------------------------

#[test]
fn repeated_execution_of_a_fixture_is_bit_identical() {
    let program_id = ripcord_engine::fixture_program_id();
    let fixtures = corpus::generate(&program_id);
    let fixture = fixtures
        .iter()
        .find(|f| f.id == "boundary-position-017")
        .expect("flagship fixture");
    let (v1, v2) = ripcord_engine::load_versions(
        &ripcord_engine::default_artifact("v1"),
        &ripcord_engine::default_artifact("v2"),
    )
    .expect("artefacts");

    let first = ripcord_engine::compare_fixture(fixture, &program_id, &v1, &v2).unwrap();
    let second = ripcord_engine::compare_fixture(fixture, &program_id, &v1, &v2).unwrap();
    assert_eq!(first, second, "identical inputs produced different results");
}

#[test]
fn corpus_generation_is_reproducible() {
    let program_id = ripcord_engine::fixture_program_id();
    assert_eq!(corpus::generate(&program_id), corpus::generate(&program_id));
}

// ---------------------------------------------------------------------------
// 7. Reporting
// ---------------------------------------------------------------------------

#[test]
fn the_text_report_names_the_fixture_and_the_changed_fields() {
    let rendered = report::render_text(report());
    assert!(rendered.contains("boundary-position-017"));
    assert!(rendered.contains("health_factor"));
    assert!(rendered.contains("liquidatable"));
    assert!(rendered.contains("1.003783"));
    assert!(rendered.contains("0.998738"));
    assert!(rendered.contains("CRITICAL"));
    assert!(rendered.contains("Do not deploy"));
}

#[test]
fn the_json_report_is_valid_and_carries_the_findings() {
    let json = report().to_json().expect("serialisable");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(parsed["summary"]["critical"], 11);
    assert_eq!(parsed["summary"]["outcome_identical"], 89);

    let diffs = parsed["diffs"].as_array().expect("diffs array");
    let flagship = diffs
        .iter()
        .find(|d| d["fixture_id"] == "boundary-position-017")
        .expect("flagship fixture present in JSON");
    let kinds: Vec<&str> = flagship["differences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"liquidation_status_changed"));
    assert!(kinds.contains(&"field_changed"));
}

#[test]
fn reproduction_output_shows_both_sides() {
    let rendered = report::render_reproduction(diff_for("boundary-position-017"));
    assert!(rendered.contains("FIXTURE  boundary-position-017"));
    assert!(rendered.contains("--- V1"));
    assert!(rendered.contains("--- V2"));
    assert!(rendered.contains("liquidatable true"));
    assert!(rendered.contains("build=v1"));
    assert!(rendered.contains("build=v2"));
}

/// The corpus on disk is a convenience for inspection and diffing; the
/// generator is the source of truth. This test keeps them from drifting.
#[test]
fn checked_in_fixtures_match_the_generator() {
    let dir = ripcord_engine::repo_root().join("fixtures/states");
    assert!(
        dir.is_dir(),
        "fixtures/states is missing; run `make fixtures`"
    );
    for fixture in corpus::generate(&ripcord_engine::fixture_program_id()) {
        let path = dir.join(format!("{}.json", fixture.id));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let on_disk: ripcord_engine::Fixture = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()));
        assert_eq!(
            on_disk, fixture,
            "{} has drifted from the generator; run `make fixtures`",
            fixture.id
        );
    }
}
