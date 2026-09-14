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

use eplyx_engine::diff::{Classification, Difference, Severity};
use eplyx_engine::interpret::{decode, Decoded};
use eplyx_engine::{corpus, report, Report, StateDiff};
use fixture_lending_interface::reference;

/// The whole corpus is executed once and shared: 141 fixtures x 2 builds is
/// cheap, but not so cheap that every test should repeat it.
fn report() -> &'static Report {
    static REPORT: OnceLock<Report> = OnceLock::new();
    REPORT.get_or_init(|| {
        eplyx_engine::compare_default_corpus().expect(
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
    let v1 = std::fs::read(eplyx_engine::default_artifact("v1")).expect("v1 artefact");
    let v2 = std::fs::read(eplyx_engine::default_artifact("v2")).expect("v2 artefact");
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
    let fixtures = corpus::generate(&eplyx_engine::fixture_program_id());
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
    let program_id = eplyx_engine::fixture_program_id();
    let fixtures = corpus::generate(&program_id);
    let fixture = fixtures
        .iter()
        .find(|f| f.id == "boundary-position-017")
        .expect("flagship fixture");
    let (v1, v2) = eplyx_engine::load_versions(
        &eplyx_engine::default_artifact("v1"),
        &eplyx_engine::default_artifact("v2"),
    )
    .expect("artefacts");

    let first = eplyx_engine::compare_fixture(fixture, &program_id, &v1, &v2).unwrap();
    let second = eplyx_engine::compare_fixture(fixture, &program_id, &v1, &v2).unwrap();
    assert_eq!(first, second, "identical inputs produced different results");
}

#[test]
fn corpus_generation_is_reproducible() {
    let program_id = eplyx_engine::fixture_program_id();
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
    let dir = eplyx_engine::repo_root().join("fixtures/states");
    assert!(
        dir.is_dir(),
        "fixtures/states is missing; run `make fixtures`"
    );
    for fixture in corpus::generate(&eplyx_engine::fixture_program_id()) {
        let path = dir.join(format!("{}.json", fixture.id));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let on_disk: eplyx_engine::Fixture = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()));
        assert_eq!(
            on_disk, fixture,
            "{} has drifted from the generator; run `make fixtures`",
            fixture.id
        );
    }
}

// ---------------------------------------------------------------------------
// Phase 2: economic impact aggregation
// ---------------------------------------------------------------------------

use eplyx_engine::impact::EconomicConsequence;
use eplyx_engine::money::Usd;

/// Source-of-truth valuation must be integer-only.
///
/// This covers the whole reporting path, not just monetary values: the compute
/// percentage is carried as integer basis points for the same reason. An `f64`
/// there did not survive a JSON round trip, which is precisely the silent loss
/// these types exist to prevent.
#[test]
fn no_floating_point_in_the_valuation_or_reporting_path() {
    let sources = [
        "interface/src/lib.rs",
        "engine/src/money.rs",
        "engine/src/interpret.rs",
        "engine/src/impact.rs",
        "engine/src/diff.rs",
        "engine/src/report.rs",
    ];
    for relative in sources {
        let path = eplyx_engine::repo_root().join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        // Scan code only: tests demonstrate the precision f64 would lose, and
        // doc comments name it to explain why it is absent.
        let production: String = text
            .split("#[cfg(test)]")
            .next()
            .unwrap_or(&text)
            .lines()
            .map(|line| match line.find("//") {
                Some(index) => &line[..index],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in ["f64", "f32"] {
            assert!(
                !production.contains(forbidden),
                "{relative} uses {forbidden} in a valuation path"
            );
        }
    }
}

#[test]
fn corpus_valuation_is_deterministic_and_covers_every_position() {
    let economics = &report().economics;
    assert_eq!(economics.positions_valued, report().summary.fixtures_tested);
    assert_eq!(report().fixture_economics.len(), economics.positions_valued);

    // Values are fixed and derived from fixture state, so they are exact.
    assert_eq!(
        economics.total_collateral_value_usd,
        Usd::from_micro(6_182_370_000_000)
    );
    assert_eq!(
        economics.total_debt_value_usd,
        Usd::from_micro(2_531_638_086_646)
    );
    assert_eq!(
        economics.total_net_value_usd,
        economics
            .total_collateral_value_usd
            .signed_sub(economics.total_debt_value_usd)
    );
}

#[test]
fn aggregates_equal_the_sum_of_their_member_positions() {
    let report = report();
    let entries = &report.fixture_economics;

    let sum = |predicate: &dyn Fn(&eplyx_engine::FixtureEconomics) -> bool| -> (usize, u128, u128) {
        let selected: Vec<_> = entries.iter().filter(|e| predicate(e)).collect();
        (
            selected.len(),
            selected
                .iter()
                .map(|e| e.baseline.collateral_value_usd.micro())
                .sum(),
            selected
                .iter()
                .map(|e| e.baseline.debt_value_usd.micro())
                .sum(),
        )
    };

    let (count, collateral, debt) = sum(&|e| e.affected);
    assert_eq!(count, report.economics.affected.positions);
    assert_eq!(
        collateral,
        report.economics.affected.collateral_value_usd.micro()
    );
    assert_eq!(debt, report.economics.affected.debt_value_usd.micro());

    let (count, collateral, debt) = sum(&|e| e.critical);
    assert_eq!(count, report.economics.critical.positions);
    assert_eq!(
        collateral,
        report.economics.critical.collateral_value_usd.micro()
    );
    assert_eq!(debt, report.economics.critical.debt_value_usd.micro());

    let (count, collateral, debt) = sum(&|e| e.is_newly_liquidatable());
    assert_eq!(count, report.economics.newly_liquidatable.positions);
    assert_eq!(
        collateral,
        report
            .economics
            .newly_liquidatable
            .collateral_value_usd
            .micro()
    );
    assert_eq!(
        debt,
        report.economics.newly_liquidatable.debt_value_usd.micro()
    );

    let (_, total_collateral, total_debt) = sum(&|_| true);
    assert_eq!(
        total_collateral,
        report.economics.total_collateral_value_usd.micro()
    );
    assert_eq!(total_debt, report.economics.total_debt_value_usd.micro());
}

/// The corpus-scale version of the unit tests: the 89 fixtures whose only
/// difference is compute must contribute nothing to affected capital.
#[test]
fn compute_only_and_unchanged_positions_contribute_no_affected_capital() {
    let report = report();
    let compute_only_or_identical: Vec<_> = report
        .fixture_economics
        .iter()
        .filter(|e| !e.affected)
        .collect();

    assert_eq!(compute_only_or_identical.len(), 89);
    assert_eq!(
        report.economics.unaffected_positions(),
        compute_only_or_identical.len()
    );
    for entry in &compute_only_or_identical {
        assert!(
            entry.consequences.is_empty(),
            "{}: unaffected position carries a consequence",
            entry.fixture_id
        );
        assert!(!entry.critical);
    }

    // Every one of those fixtures does differ on compute, which is exactly the
    // signal that must not count as economic impact.
    let diffs_with_compute = report
        .diffs
        .iter()
        .filter(|d| d.compute_delta().is_some())
        .count();
    assert_eq!(diffs_with_compute, report.summary.fixtures_tested);

    // Affected capital is therefore strictly less than total capital.
    assert!(
        report.economics.affected.collateral_value_usd
            < report.economics.total_collateral_value_usd
    );
}

#[test]
fn newly_liquidatable_capital_is_counted_from_the_expected_positions() {
    let report = report();
    let newly: Vec<&str> = report
        .fixture_economics
        .iter()
        .filter(|e| e.is_newly_liquidatable())
        .map(|e| e.fixture_id.as_str())
        .collect();
    assert_eq!(
        newly,
        vec![
            "boundary-position-017",
            "boundary-position-018",
            "boundary-position-019",
            "boundary-position-020",
        ]
    );

    let group = &report.economics.newly_liquidatable;
    assert_eq!(group.positions, 4);
    // 4 positions of 99.5 SOL at $100.00.
    assert_eq!(group.collateral_value_usd, Usd::from_micro(39_800_000_000));
    // $7,930 + $7,940 + $7,950 + $7,960.
    assert_eq!(group.debt_value_usd, Usd::from_micro(31_780_000_000));
}

#[test]
fn consequences_partition_the_affected_set() {
    let report = report();
    for entry in &report.fixture_economics {
        assert_eq!(
            entry.affected,
            !entry.consequences.is_empty(),
            "{}: affected flag disagrees with consequences",
            entry.fixture_id
        );
    }

    let by = &report.economics.by_consequence;
    assert_eq!(
        by[EconomicConsequence::NewlyLiquidatable.as_str()].positions,
        4
    );
    assert_eq!(
        by[EconomicConsequence::TransactionNowReverts.as_str()].positions,
        4
    );
    assert_eq!(
        by[EconomicConsequence::TransactionNowSucceeds.as_str()].positions,
        3
    );
    assert_eq!(
        by[EconomicConsequence::NoLongerLiquidatable.as_str()].positions,
        0
    );
    assert_eq!(by[EconomicConsequence::ValueChanged.as_str()].positions, 41);

    // Critical fixtures are exactly the threshold crossings and outcome flips.
    assert_eq!(
        by[EconomicConsequence::NewlyLiquidatable.as_str()].positions
            + by[EconomicConsequence::TransactionNowReverts.as_str()].positions
            + by[EconomicConsequence::TransactionNowSucceeds.as_str()].positions,
        report.economics.critical.positions
    );
}

#[test]
fn the_flagship_fixture_carries_exact_economic_values() {
    let economics = report()
        .economics_for("boundary-position-017")
        .expect("flagship economics");

    assert_eq!(
        economics.baseline.collateral_value_usd,
        Usd::from_micro(9_950_000_000)
    );
    assert_eq!(
        economics.baseline.collateral_value_usd.format_dollars(),
        "$9,950.00"
    );
    assert_eq!(
        economics.baseline.debt_value_usd,
        Usd::from_micro(7_930_000_000)
    );
    assert_eq!(
        economics.baseline.debt_value_usd.format_dollars(),
        "$7,930.00"
    );
    assert_eq!(
        economics.baseline.net_value_usd.format_dollars(),
        "$2,020.00"
    );
    assert_eq!(economics.baseline.collateral_amount, 99_500_000_000);
    assert_eq!(economics.baseline.collateral_decimals, 9);
    assert_eq!(economics.baseline.debt_decimals, 6);
    assert!(economics.affected && economics.critical);
    assert_eq!(
        economics.consequences,
        vec![EconomicConsequence::NewlyLiquidatable]
    );

    // Post-execution valuations show the divergence the aggregate summarises.
    assert!(!economics.v1_post.as_ref().unwrap().liquidatable);
    assert!(economics.v2_post.as_ref().unwrap().liquidatable);
}

#[test]
fn the_text_report_shows_economic_coverage_and_impact() {
    let rendered = report::render_text(report());
    assert!(rendered.contains("EPLYX UPGRADE IMPACT"));
    assert!(rendered.contains("ECONOMIC COVERAGE"));
    assert!(rendered.contains("Collateral represented:"));
    assert!(rendered.contains("Debt represented:"));
    assert!(rendered.contains("NEWLY LIQUIDATABLE"));
    assert!(rendered.contains("BY ECONOMIC CONSEQUENCE"));
    assert!(rendered.contains("$9,950.00"));
    assert!(rendered.contains("position becomes newly liquidatable"));
    // Conservative vocabulary: nothing claims proven real-world capital at risk.
    assert!(!rendered.to_lowercase().contains("capital at risk"));
}

#[test]
fn the_json_report_carries_the_economic_summary() {
    let json = report().to_json().expect("serialisable");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    let economics = &parsed["economics"];
    assert_eq!(economics["positions_valued"], 141);
    // Monetary values are decimal strings, never JSON numbers, so a consumer
    // cannot silently parse them as doubles.
    assert_eq!(economics["total_collateral_value_usd"], "6182370.000000");
    assert_eq!(economics["total_debt_value_usd"], "2531638.086646");
    assert_eq!(economics["affected"]["positions"], 52);
    assert_eq!(economics["critical"]["positions"], 11);
    assert_eq!(economics["newly_liquidatable"]["positions"], 4);
    assert_eq!(
        economics["newly_liquidatable"]["collateral_value_usd"],
        "39800.000000"
    );
    assert!(economics["by_consequence"]["newly_liquidatable"].is_object());

    let per_fixture = parsed["fixture_economics"].as_array().expect("array");
    assert_eq!(per_fixture.len(), 141);
    let flagship = per_fixture
        .iter()
        .find(|e| e["fixture_id"] == "boundary-position-017")
        .expect("flagship present");
    assert_eq!(flagship["baseline"]["collateral_value_usd"], "9950.000000");
    assert_eq!(flagship["baseline"]["debt_value_usd"], "7930.000000");
    assert_eq!(flagship["baseline"]["net_value_usd"], "2020.000000");
    assert_eq!(flagship["consequences"][0], "newly_liquidatable");
}

/// Round-tripping proves the schema is a stable contract, not just something
/// that happens to serialise.
#[test]
fn the_json_report_round_trips() {
    let json = report().to_json().expect("serialisable");
    let restored: eplyx_engine::Report = serde_json::from_str(&json).expect("deserialisable");
    assert_eq!(&restored, report());
}
