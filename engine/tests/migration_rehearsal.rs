//! Token Migration V1 VM rehearsal over synthetic worlds built with the real
//! captured SPL Token / Token-2022 / ATA programs and the reference candidate.
#[path = "common/migration.rs"]
mod migration_common;
use eplyx_engine::{
    evidence::paths::PathStatus,
    migration::{execute::Outcome, planner::ImpactClass},
};
use migration_common::*;
use serde_json::json;

fn legacy_recipe() -> eplyx_engine::migration::fixture::Recipe {
    recipe(json!({
        "schemaVersion": 1, "id": "case-a-legacy", "description": "legacy to legacy",
        "programs": "pinnedMainnetCapture",
        "clock": {"slot": "1000", "unixTimestamp": "1760000000", "epoch": "400"},
        "populationMint": "source",
        "wallets": wallets(&["alice", "bob", "carol", "issuer"]),
        "mints": [
            {"label": "source", "tokenProgram": LEGACY, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}},
            {"label": "destination", "tokenProgram": LEGACY, "decimals": 6, "mintAuthority": {"label": "issuer"}}
        ],
        "tokenAccounts": [
            {"label": "alice-source", "mint": "source", "owner": {"label": "alice"}, "layout": "associated", "amount": "1001"},
            {"label": "bob-source", "mint": "source", "owner": {"label": "bob"}, "layout": "associated", "amount": "20"},
            {"label": "bob-destination", "mint": "destination", "owner": {"label": "bob"}, "layout": "associated", "amount": "7"},
            {"label": "carol-source", "mint": "source", "owner": {"label": "carol"}, "layout": "associated", "amount": "0"}
        ]
    }))
}

#[test]
fn legacy_to_legacy_burn_and_reserve_transfer_reconciles_exactly() {
    let recipe = legacy_recipe();
    let spec = spec(&recipe, LEGACY, LEGACY, json!({}));
    let world = world(&recipe, &spec);
    let program = reference();
    let plan = plan(&spec, &world, &program);
    let classes: Vec<_> = plan.units.iter().map(|u| u.class).collect();
    assert_eq!(
        classes
            .iter()
            .filter(|c| **c == ImpactClass::Migratable)
            .count(),
        2
    );
    assert!(classes.contains(&ImpactClass::ZeroBalance));
    let results = rehearse(&spec, &world, &plan, &program);
    assert_eq!(results.len(), 2);
    for result in &results {
        assert_eq!(
            result.outcome,
            Outcome::Migrated,
            "{:#?}",
            result.reconciliation
        );
        assert_eq!(result.status, PathStatus::Proven);
        assert!(result.unexpected_changes.is_empty());
    }
    let alice = results
        .iter()
        .find(|r| r.source_account == address(&recipe, "alice-source"))
        .unwrap();
    assert_eq!(alice.deltas.source_debit_raw, "1001");
    assert_eq!(alice.deltas.source_supply_decrease_raw, "1001");
    assert_eq!(alice.deltas.reserve_debit_raw, "500");
    assert_eq!(alice.deltas.destination_net_credit_raw, "500");
}

fn t22_recipe(
    id: &str,
    extra_accounts: serde_json::Value,
    destination_extensions: serde_json::Value,
) -> eplyx_engine::migration::fixture::Recipe {
    let mut accounts = vec![
        json!({"label": "alice-source", "mint": "source", "owner": {"label": "alice"}, "layout": "associated", "amount": "1000000"}),
        json!({"label": "bob-source", "mint": "source", "owner": {"label": "bob"}, "layout": "associated", "amount": "250000", "delegate": {"delegate": {"label": "dana"}, "amount": "250000"}}),
    ];
    accounts.extend(extra_accounts.as_array().cloned().unwrap_or_default());
    recipe(json!({
        "schemaVersion": 1, "id": id, "description": "token-2022 to token-2022",
        "programs": "pinnedMainnetCapture",
        "clock": {"slot": "2000", "unixTimestamp": "1760000000", "epoch": "400"},
        "populationMint": "source",
        "wallets": wallets(&["alice", "bob", "dana", "issuer", "m1", "m2", "m3"]),
        "multisigs": [{"label": "treasury", "tokenProgram": T22, "threshold": 2, "signers": ["m1", "m2", "m3"]}],
        "programOwned": [{"label": "pool"}],
        "mints": [
            {"label": "source", "tokenProgram": T22, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"},
             "extensions": [{"kind": "transferFee", "bps": 100, "maximumFee": "5000"}, {"kind": "permanentDelegate", "delegate": {"label": "issuer"}}]},
            {"label": "destination", "tokenProgram": T22, "decimals": 9, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"},
             "extensions": destination_extensions}
        ],
        "tokenAccounts": accounts
    }))
}

#[test]
fn token_2022_escrow_and_reserve_transfer_model_transfer_fees_exactly() {
    let recipe = t22_recipe(
        "case-c-t22-fees",
        json!([{"label": "treasury-source", "mint": "source", "owner": {"label": "treasury"}, "layout": "associated", "amount": "40000"},
               {"label": "pool-source", "mint": "source", "owner": {"label": "pool"}, "layout": "associated", "amount": "999"}]),
        json!([{"kind": "transferFee", "bps": 50, "maximumFee": "1000000"}]),
    );
    let spec = spec(
        &recipe,
        T22,
        T22,
        json!({
            "conversion": {"ratioBasis": "ui", "numerator": "3", "denominator": "2", "rounding": "ceiling", "fee": {"kind": "sourceBps", "bps": 25}, "minimumOutputRaw": "1"},
            "sourceDisposition": {"kind": "escrow"}
        }),
    );
    let world = world(&recipe, &spec);
    let program = reference();
    let plan = plan(&spec, &world, &program);
    let class_of = |label: &str| plan.unit(&address(&recipe, label)).unwrap().class;
    assert_eq!(class_of("alice-source"), ImpactClass::Migratable);
    assert_eq!(
        class_of("bob-source"),
        ImpactClass::Migratable,
        "owner wallet path is preferred over the delegate"
    );
    assert_eq!(
        class_of("treasury-source"),
        ImpactClass::Migratable,
        "2-of-3 SPL multisig owner"
    );
    assert_eq!(
        class_of("pool-source"),
        ImpactClass::AuthorityPathUnavailable,
        "program-controlled owner"
    );
    let results = rehearse(&spec, &world, &plan, &program);
    assert_eq!(results.len(), 3);
    for r in &results {
        assert_eq!(
            r.outcome,
            Outcome::Migrated,
            "{}: {:#?} {:?}",
            r.source_account,
            r.reconciliation,
            r.failure
        );
    }
    let alice = results
        .iter()
        .find(|r| r.source_account == address(&recipe, "alice-source"))
        .unwrap();
    // 1_000_000 raw at 6 decimals -> ui ratio 3/2 into 9 decimals -> raw ratio 1500/1.
    // fee 25 bps = 2500; converted 997_500 -> 1_496_250_000 destination raw.
    assert_eq!(alice.deltas.escrow_gross_credit_raw, "1000000");
    assert_eq!(
        alice.deltas.escrow_withheld_fee_raw, "5000",
        "source transfer fee capped at maximumFee"
    );
    assert_eq!(alice.deltas.reserve_debit_raw, "1496250000");
    assert_eq!(
        alice.deltas.destination_withheld_fee_raw, "1000000",
        "destination fee capped at maximumFee"
    );
    assert_eq!(alice.deltas.destination_net_credit_raw, "1495250000");
}

#[test]
fn mint_to_requires_the_migration_authority_and_never_assumes_issuer_authority() {
    let recipe = t22_recipe("case-mint-pda", json!([]), json!([]));
    let mint_spec = spec(
        &recipe,
        T22,
        T22,
        json!({"destinationFunding": {"kind": "mintTo"}}),
    );
    let world_without = world(&recipe, &mint_spec);
    let program = reference();
    let blocked = plan(&mint_spec, &world_without, &program);
    assert!(!blocked.funding.path_available);
    assert!(blocked
        .units
        .iter()
        .filter(|u| u.class != ImpactClass::AuthorityPathUnavailable)
        .all(|u| u.class == ImpactClass::FundingPathUnavailable));
    assert_eq!(blocked.funding.reasons[0].code, "MINT_AUTHORITY_MISMATCH");

    // The issuer transfers mint authority to the migration PDA (a real SetAuthority).
    let mut value = serde_json::to_value(&recipe).unwrap();
    value["id"] = "case-mint-pda-set".into();
    value["postSteps"] = json!([{"kind": "setMintAuthority", "mint": "destination", "to": {"migrationAuthority": true}}]);
    let recipe = migration_common::recipe(value);
    let mint_spec = spec(
        &recipe,
        T22,
        T22,
        json!({"destinationFunding": {"kind": "mintTo"}}),
    );
    let world = world(&recipe, &mint_spec);
    let plan = plan(&mint_spec, &world, &program);
    assert!(plan.funding.path_available, "{:?}", plan.funding.reasons);
    let results = rehearse(&mint_spec, &world, &plan, &program);
    assert!(!results.is_empty());
    for r in &results {
        assert_eq!(
            r.outcome,
            Outcome::Migrated,
            "{:#?} {:?}",
            r.reconciliation,
            r.failure
        );
        assert_eq!(r.deltas.reserve_debit_raw, "0");
        assert_ne!(r.deltas.destination_supply_increase_raw, "0");
    }
}

#[test]
fn sequential_rehearsal_exposes_cumulative_reserve_shortfall() {
    let recipe = legacy_recipe();
    // alice needs 500, bob 10: 505 funds alice but not bob (address order decides).
    let spec = spec(
        &recipe,
        LEGACY,
        LEGACY,
        json!({"destinationFunding": {"kind": "reserveTransfer", "reserve": {"kind": "proposed", "fundedRaw": "505"}}}),
    );
    let world = world(&recipe, &spec);
    let program = reference();
    let plan = plan(&spec, &world, &program);
    let short: Vec<_> = plan
        .units
        .iter()
        .filter(|u| u.class == ImpactClass::InsufficientReserve)
        .collect();
    let results = rehearse(&spec, &world, &plan, &program);
    let failed: Vec<_> = results
        .iter()
        .filter(|r| r.outcome == Outcome::Rejected)
        .collect();
    assert_eq!(failed.len(), short.len());
    for f in &failed {
        let failure = f.failure.as_ref().unwrap();
        assert_eq!(failure.stage, "candidate");
        assert_eq!(failure.error_name.as_deref(), Some("InsufficientReserve"));
        assert!(failure.rollback_verified);
    }
}
