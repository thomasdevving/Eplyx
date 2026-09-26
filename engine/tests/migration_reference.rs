//! Frozen minimal examples and deterministic reports through MAIN's actual port.
#[path = "common/migration.rs"]
mod migration_common;
use eplyx_engine::{
    migration::{
        fixture::Recipe,
        gate::Policy,
        invariants::MigrationInvariant,
        pipeline::{self, Isolation},
        search,
        spec::TokenMigrationV1,
    },
    repo_root,
};
use migration_common::*;
use serde_json::Value;

#[test]
fn minimal_reference_and_deadline_defect_match_t0_and_replay_offline() {
    let root = repo_root().join("examples/migrations/minimal");
    let recipe = Recipe::parse(&std::fs::read(root.join("fixtures/world.json")).unwrap()).unwrap();
    let spec: TokenMigrationV1 =
        serde_json::from_slice(&std::fs::read(root.join("migration.json")).unwrap()).unwrap();
    // Exact nine declarations from STA minimal/eplyx.toml, not the broader defaults.
    let minimal_invariants: Vec<_> = MigrationInvariant::recommended()
        .into_iter()
        .filter(|i| {
            [
                "candidate_binary_matches_package",
                "required_authorities_match",
                "migration_arithmetic_matches_spec",
                "failed_migrations_roll_back",
                "supply_reconciles",
                "reserve_covers_eligible_holders",
                "window_rules_hold",
                "no_unsupported_execution_succeeds",
                "all_positive_holders_migrate",
            ]
            .contains(&i.kind())
        })
        .collect();
    for (label, name) in [
        ("minimal", "eplyx_token_migration"),
        (
            "minimal-deadline-defect",
            "eplyx_token_migration_defect_deadline_inclusive",
        ),
    ] {
        let (root, input) = package(
            label,
            &recipe,
            &spec,
            &candidate(name),
            minimal_invariants.clone(),
        );
        let report = pipeline::run(
            input.root(),
            &root.join("result"),
            Policy::BlockOnly,
            Isolation::InProcess,
        )
        .unwrap();
        let findings = if label == "minimal" {
            Value::Null
        } else {
            let result =
                search::run(input.root(), &root.join("result"), &root.join("search")).unwrap();
            assert_eq!(
                search::replay(input.root(), &root.join("result"), &root.join("search")).unwrap(),
                result
            );
            assert_eq!(result.counterexamples.len(), 1);
            serde_json::to_value(result).unwrap()
        };
        assert_t0(label, &report, findings);
        assert_eq!(
            pipeline::replay(input.root(), &root.join("result")).unwrap(),
            report
        );
        let second = pipeline::run(
            input.root(),
            &root.join("second"),
            Policy::BlockOnly,
            Isolation::InProcess,
        )
        .unwrap();
        assert_eq!(second, report, "run metadata cannot alter analytical bytes");
        for artifact in [
            pipeline::REPORT,
            pipeline::REPORT_MD,
            pipeline::PLAN,
            pipeline::REHEARSAL,
            pipeline::STRESS_PLAN,
            pipeline::STRESS_RESULTS,
            pipeline::UNSIGNED,
        ] {
            assert_eq!(
                std::fs::read(root.join("result").join(artifact)).unwrap(),
                std::fs::read(root.join("second").join(artifact)).unwrap(),
                "{artifact} determinism"
            );
        }
        let strict =
            pipeline::replay_with_policy(input.root(), &root.join("result"), Some(Policy::Strict))
                .unwrap();
        assert_eq!(
            pipeline::exit_code(&strict).unwrap(),
            1,
            "a frozen holder violates the declared requirement"
        );
    }
}
