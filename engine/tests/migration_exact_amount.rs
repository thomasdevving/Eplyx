#[path = "common/migration.rs"]
mod common;
use common::*;
use eplyx_engine::migration::{planner::ImpactClass, spec::AmountPolicy};
use serde_json::json;
#[test]
fn explicit_amount_executes_against_unchanged_balance_and_changes_proposal_identity() {
    let recipe = recipe(
        json!({"schemaVersion":1,"programs":"pinnedMainnetCapture","populationMint":"source","description":"Explicit amount against unchanged synthetic state","id":"exact-amount","clock":{"slot":"1","epoch":"0","unixTimestamp":"100"},"wallets":[{"label":"holder","lamports":"1000000000"}],"mints":[{"label":"source","tokenProgram":LEGACY,"decimals":0,"mintAuthority":{"label":"holder"}},{"label":"destination","tokenProgram":LEGACY,"decimals":0,"mintAuthority":{"label":"holder"}}],"tokenAccounts":[{"label":"holding","mint":"source","owner":{"label":"holder"},"layout":"associated","amount":"100"}]}),
    );
    let mut spec = spec(&recipe, LEGACY, LEGACY, json!({}));
    let full_id = change_spec_id(&spec);
    spec.eligibility.amount_policy = AmountPolicy::ExactRaw {
        amount_raw: "10".into(),
    };
    spec.validate().unwrap();
    assert_ne!(change_spec_id(&spec), full_id);
    let world = world(&recipe, &spec);
    let candidate = reference();
    let planned = plan(&spec, &world, &candidate);
    let unit = planned
        .units
        .iter()
        .find(|u| u.source_account == address(&recipe, "holding"))
        .unwrap();
    assert_eq!(unit.source_balance_raw, "100");
    assert_eq!(unit.amount_raw, "10");
    assert_eq!(unit.class, ImpactClass::Migratable);
    let executions = rehearse(&spec, &world, &planned, &candidate);
    assert_eq!(executions.len(), 1);
    let v = serde_json::to_value(&executions[0]).unwrap();
    assert_eq!(v["reconciled"], true, "{v}");
    assert_eq!(
        serde_json::to_value(AmountPolicy::FullBalance).unwrap(),
        json!("full_balance")
    );
    spec.eligibility.amount_policy = AmountPolicy::ExactRaw {
        amount_raw: "101".into(),
    };
    let rejected = plan(&spec, &world, &candidate);
    let unit = rejected
        .units
        .iter()
        .find(|u| u.source_account == address(&recipe, "holding"))
        .unwrap();
    assert_eq!(unit.class, ImpactClass::OutsideEligibility);
    assert!(unit
        .reasons
        .iter()
        .any(|r| r.code == "AMOUNT_EXCEEDS_BALANCE"));
    for amount in ["0", "00", "01", "18446744073709551616"] {
        spec.eligibility.amount_policy = AmountPolicy::ExactRaw {
            amount_raw: amount.into(),
        };
        assert!(spec.validate().is_err(), "{amount}");
    }
}
