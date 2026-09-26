//! Bounded migration counterexample search: observed rehearsal findings, derived
//! probes with minimization, recorded domains and budgets, offline replay.
#[path = "common/migration.rs"]
mod migration_common;
use eplyx_engine::{
    migration::gate::Policy,
    migration::{
        invariants::MigrationInvariant,
        pipeline,
        search::{self, Counterexample, Dimension, SearchFinding},
    },
};
use migration_common::*;
use serde_json::json;

fn recipe_search() -> eplyx_engine::migration::fixture::Recipe {
    recipe(json!({
        "schemaVersion": 1, "id": "search-world", "description": "search world",
        "programs": "pinnedMainnetCapture",
        "clock": {"slot": "5000", "unixTimestamp": "1760000000", "epoch": "400"},
        "populationMint": "source",
        "wallets": wallets(&["alice", "bob", "carol", "issuer"]),
        "mints": [
            {"label": "source", "tokenProgram": T22, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}},
            {"label": "destination", "tokenProgram": LEGACY, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}}
        ],
        "tokenAccounts": [
            {"label": "alice-source", "mint": "source", "owner": {"label": "alice"}, "layout": "associated", "amount": "9000000"},
            {"label": "bob-source", "mint": "source", "owner": {"label": "bob"}, "layout": "associated", "amount": "400000"},
            {"label": "carol-source", "mint": "source", "owner": {"label": "carol"}, "layout": "associated", "amount": "7777"}
        ]
    }))
}

fn searched(
    label: &str,
    program: &[u8],
    patch: serde_json::Value,
) -> (search::SearchResult, std::path::PathBuf, std::path::PathBuf) {
    let recipe = recipe_search();
    let spec = spec(&recipe, T22, LEGACY, patch);
    let (root, validated) = package(
        label,
        &recipe,
        &spec,
        program,
        MigrationInvariant::recommended(),
    );
    let result = root.join("result");
    pipeline::run(
        validated.root(),
        &result,
        Policy::BlockOnly,
        pipeline::Isolation::InProcess,
    )
    .unwrap();
    let outcome = search::run(validated.root(), &result, &root.join("search")).unwrap();
    (outcome, validated.root().to_path_buf(), root)
}

const WINDOWED: &str = r#"{"conversion": {"ratioBasis": "raw", "numerator": "1", "denominator": "1", "rounding": "floor", "fee": {"kind": "sourceBps", "bps": 37}, "minimumOutputRaw": "5"}, "window": {"activation": {"kind": "slot", "value": "6000"}, "deadline": {"kind": "slot", "value": "7000"}}}"#;

#[test]
fn reference_candidate_has_no_counterexample_within_the_recorded_domain() {
    let (outcome, _, _) = searched(
        "search-reference",
        &reference(),
        serde_json::from_str(WINDOWED).unwrap(),
    );
    assert!(
        outcome.counterexamples.is_empty(),
        "{:#?}",
        outcome.counterexamples
    );
    assert_eq!(outcome.conclusion, search::NO_FINDING);
    for dimension in [
        Dimension::SourceAmount,
        Dimension::ActivationBoundary,
        Dimension::DeadlineBoundary,
        Dimension::ProposedReserve,
        Dimension::SignerAuthority,
        Dimension::Token2022Guard,
    ] {
        assert!(
            outcome.explored_dimensions.contains(&dimension),
            "{dimension:?} not explored"
        );
    }
    assert!(outcome.budget.probes <= outcome.budget.max_probes);
    assert!(!outcome.domains.is_empty());
}

#[test]
fn defects_become_minimal_reproducible_derived_counterexamples() {
    let (deadline, package_dir, root) = searched(
        "search-deadline",
        &candidate("eplyx_token_migration_defect_deadline_inclusive"),
        serde_json::from_str(WINDOWED).unwrap(),
    );
    let at_deadline = deadline
        .counterexamples
        .iter()
        .find(|c| {
            matches!(
                c,
                Counterexample::MigrationDerived {
                    dimension: Dimension::DeadlineBoundary,
                    ..
                }
            )
        })
        .expect("deadline defect found");
    match at_deadline {
        Counterexample::MigrationDerived {
            derived_value_raw,
            finding,
            mutations,
            ..
        } => {
            assert_eq!(derived_value_raw.as_deref(), Some("7000"));
            assert_eq!(*finding, SearchFinding::UnexpectedSuccess);
            assert!(
                !mutations.is_empty(),
                "the reproducing mutation is recorded"
            );
        }
        _ => unreachable!(),
    }
    assert_eq!(
        search::replay(&package_dir, &root.join("result"), &root.join("search")).unwrap(),
        deadline
    );

    let (fee, _, _) = searched(
        "search-fee",
        &candidate("eplyx_token_migration_defect_fee_ceiling"),
        serde_json::from_str(WINDOWED).unwrap(),
    );
    let amount = fee
        .counterexamples
        .iter()
        .find_map(|c| match c {
            Counterexample::MigrationDerived {
                dimension: Dimension::SourceAmount,
                derived_value_raw,
                minimized,
                boundary,
                finding,
                ..
            } => Some((
                derived_value_raw.clone(),
                *minimized,
                boundary.clone(),
                *finding,
            )),
            _ => None,
        })
        .expect("fee-ceiling defect found on the amount dimension");
    // Its extra fee unit either drops the output below the minimum (a rejected
    // eligible migration) or releases one unit less than specified (a mismatch).
    assert!(
        matches!(
            amount.3,
            SearchFinding::UnexpectedFailure | SearchFinding::ReconciliationMismatch
        ),
        "{amount:?}"
    );
    assert!(
        amount.1,
        "the smallest deviating amount is minimized: {amount:?}"
    );
    assert!(
        fee.counterexamples.iter().any(|c| matches!(
            c,
            Counterexample::MigrationObserved {
                finding: SearchFinding::ReconciliationMismatch,
                ..
            }
        )),
        "every holder with a fractional fee mismatches in the population rehearsal"
    );
}

#[test]
fn underfunded_population_yields_observed_counterexamples_and_an_exact_reserve_boundary() {
    let (outcome, _, _) = searched(
        "search-underfunded",
        &reference(),
        json!({"destinationFunding": {"kind": "reserveTransfer", "reserve": {"kind": "proposed", "fundedRaw": "4000000"}}}),
    );
    let observed: Vec<_> = outcome
        .counterexamples
        .iter()
        .filter(|c| {
            matches!(
                c,
                Counterexample::MigrationObserved {
                    finding: SearchFinding::EligibleHolderNotMigrated,
                    ..
                }
            )
        })
        .collect();
    assert!(!observed.is_empty());
    let boundary: Vec<_> = outcome
        .trace
        .iter()
        .filter(|p| p.dimension == Dimension::PopulationReserve)
        .collect();
    assert_eq!(boundary.len(), 2);
    assert!(
        boundary.iter().all(|p| p.behaves_as_specified),
        "{boundary:#?}"
    );
    assert_eq!(
        boundary[0].signature.error_name.as_deref(),
        Some("InsufficientReserve")
    );
    assert_eq!(
        boundary[1].signature.outcome,
        eplyx_engine::migration::execute::Outcome::Migrated
    );
}
