use borsh::BorshDeserialize;
use eplyx_engine::{
    bundle::CiBundle,
    change::{Change, ChangeMetadata, ChangeSpec},
    parameter_change::{self as p, stake_pool as s, ConfigTarget, Operation, ParameterChange},
    replay::hash_bytes,
};
use serde_json::{json, Value};
fn input() -> s::Input {
    s::Input::from_bundle(
        &CiBundle::open("../deploy/bundle").unwrap(),
        "mainnet-spl-stake-pool-151010f709e113e7",
    )
    .unwrap()
}
fn spec(input: &s::Input, numerator: u64, denominator: u64) -> ChangeSpec {
    let pool = input
        .record
        .accounts
        .iter()
        .find(|a| a.label == "stake-pool")
        .unwrap();
    let state =
        spl_stake_pool::state::StakePool::deserialize(&mut pool.account.data.as_slice()).unwrap();
    ChangeSpec {
        schema_version: 1,
        change_spec_id: None,
        activation: None,
        metadata: ChangeMetadata::default(),
        change: Change::ProtocolParameterChange(Box::new(ParameterChange {
            target: ConfigTarget {
                program_id: input.record.program_id.clone(),
                config_account: pool.address.clone(),
            },
            operation: Operation::SplStakePoolSolDepositFeeV1 {
                expected_current: s::ExpectedCurrent {
                    account_data_sha256: hash_bytes(&pool.account.data),
                    numerator: state.sol_deposit_fee.numerator,
                    denominator: state.sol_deposit_fee.denominator,
                    sol_referral_fee_percent: state.sol_referral_fee,
                    last_update_epoch: state.last_update_epoch,
                },
                proposed_fee: s::RationalFee {
                    numerator,
                    denominator,
                },
            },
        })),
    }
}
fn seal(mut v: Value) -> Value {
    v.as_object_mut().unwrap().remove("report_sha256");
    v["report_sha256"] = eplyx_engine::canonical::digest(&v).unwrap().into();
    v
}
#[test]
fn retained_real_set_fee_and_paired_economics_reproduce() {
    let input = input();
    let spec = spec(&input, 1, 100);
    let r = s::analyze(&spec, &input).unwrap();
    assert_eq!(r["status"], "semantic_consequence_observed");
    assert_eq!(r["baseline_fidelity"]["status"], "matched");
    assert_eq!(
        r["simulated_config_instruction"]["execution"]["success"],
        true
    );
    assert_eq!(
        r["simulated_config_instruction"]["manager_assumption"]["observed"],
        false
    );
    assert_eq!(
        r["simulated_config_instruction"]["execution"]["transaction_fee_lamports"],
        "10000"
    );
    assert_eq!(
        r["baseline"]["reconciliation"]["recipient_account_credit_raw"],
        "760985008"
    );
    assert_eq!(
        r["proposed"]["reconciliation"]["recipient_account_credit_raw"],
        "753375157"
    );
    assert_eq!(
        r["proposed"]["reconciliation"]["manager_fee_account_credit_raw"],
        "7609851"
    );
    assert_eq!(
        r["proposed"]["reconciliation"]["referral_account_credit_raw"],
        Value::Null
    );
    assert_eq!(
        r["proposed"]["reconciliation"]["mint_supply_delta_raw"],
        "760985008"
    );
    assert_eq!(
        r["proposed"]["reconciliation"]["pool_token_supply_delta_raw"],
        "760985008"
    );
    assert_eq!(
        r["proposed"]["reconciliation"]["reserve_lamport_delta"],
        "822000000"
    );
    assert_eq!(
        r["proposed"]["reconciliation"]["funding_payer_debit_excluding_transaction_fee"],
        "822000000"
    );
    assert_eq!(
        r["baseline"]["reconciliation"]["unique_token_account_credits"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    p::verify(&spec, &r).unwrap();
    p::reproduce(&spec, &r).unwrap();
    assert_eq!(r, s::analyze(&spec, &input).unwrap());
    if let Some(dir) = std::env::var_os("EPLYX_STAKE_PARAMETER_OUT") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, value) in [
            ("input.json", serde_json::to_value(&input).unwrap()),
            ("change.json", serde_json::to_value(&spec).unwrap()),
            ("report.json", r),
        ] {
            std::fs::write(
                dir.join(name),
                eplyx_engine::canonical::document(&value).unwrap(),
            )
            .unwrap();
        }
    }
}
#[test]
fn exact_rationals_identity_canonical_wire_and_unknown_fields() {
    let i = input();
    let s = spec(&i, 1, 100);
    let id = s.id().unwrap();
    let wire = serde_json::to_value(&s).unwrap();
    assert_eq!(
        ChangeSpec::parse(s.to_document().unwrap().as_bytes())
            .unwrap()
            .id()
            .unwrap(),
        id
    );
    assert_eq!(s.candidate(), None);
    assert_eq!(s.schema_version, 1);
    for path in [
        "/change/target/program_id",
        "/change/target/config_account",
        "/change/operation/expected_current/account_data_sha256",
        "/change/operation/expected_current/numerator",
        "/change/operation/expected_current/denominator",
        "/change/operation/expected_current/last_update_epoch",
        "/change/operation/expected_current/sol_referral_fee_percent",
        "/change/operation/proposed_fee/numerator",
        "/change/operation/proposed_fee/denominator",
    ] {
        let mut v = wire.clone();
        let field = v.pointer_mut(path).unwrap();
        *field = if field.is_number() {
            json!(1)
        } else {
            json!(
                if path.ends_with("config_account") || path.ends_with("program_id") {
                    i.record.transaction.payer.as_str()
                } else if path.ends_with("account_data_sha256") {
                    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                } else {
                    "99"
                }
            )
        };
        assert_ne!(
            serde_json::from_value::<ChangeSpec>(v)
                .unwrap()
                .id()
                .unwrap(),
            id,
            "{path}"
        );
    }
    let mut other_operation = wire.clone();
    other_operation["change"]["operation"] = json!({"kind":"token_2022_active_newer_transfer_fee_basis_points_v1","expected_current":{"account_data_sha256":"f".repeat(64),"basis_points":0,"schedule_epoch":"0","maximum_fee_raw":"0"},"proposed_basis_points":1});
    assert_ne!(
        serde_json::from_value::<ChangeSpec>(other_operation)
            .unwrap()
            .id()
            .unwrap(),
        id
    );
    let mut cosmetic = s.clone();
    cosmetic.metadata.label = Some("display".into());
    assert_eq!(cosmetic.id().unwrap(), id);
    assert_ne!(spec(&i, 2, 200).id().unwrap(), id);
    for field in [
        json!(1),
        json!("01"),
        json!("-1"),
        json!("18446744073709551616"),
    ] {
        let mut v = wire.clone();
        v["change"]["operation"]["proposed_fee"]["numerator"] = field;
        assert!(ChangeSpec::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    let mut v = wire.clone();
    v["change"]["operation"]["offset"] = json!(4);
    assert!(ChangeSpec::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    v = wire.clone();
    v["change"]["operation"]["kind"] = json!("anything_else");
    assert!(ChangeSpec::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    v = wire;
    v["activation"] = Value::Null;
    assert!(ChangeSpec::parse(&serde_json::to_vec(&v).unwrap()).is_err());
}
#[test]
fn stale_expectations_fail_before_any_execution() {
    let i = input();
    let s = spec(&i, 1, 100);
    let wire = serde_json::to_value(&s).unwrap();
    for name in [
        "account_data_sha256",
        "numerator",
        "denominator",
        "sol_referral_fee_percent",
        "last_update_epoch",
    ] {
        let mut v = wire.clone();
        v["change"]["operation"]["expected_current"][name] = if name == "account_data_sha256" {
            json!("f".repeat(64))
        } else if name == "sol_referral_fee_percent" {
            json!(1)
        } else {
            json!("99")
        };
        let s = ChangeSpec::parse(&serde_json::to_vec(&v).unwrap()).unwrap();
        let r = s::analyze(&s, &i).unwrap();
        assert_eq!(r["status"], "current_state_mismatch");
        assert_eq!(r["execution_performed"], false);
        assert!(r.get("baseline").is_none());
        assert!(r.get("simulated_config_instruction").is_none());
        p::verify(&s, &r).unwrap();
    }
}
#[test]
fn deployed_fee_rules_equal_and_rounding_controls() {
    let i = input();
    for (num, den, status) in [
        (0, 0, "no_observed_consequence"),
        (0, 100, "no_observed_consequence"),
        (1, u64::MAX, "semantic_consequence_observed"),
        (0, u64::MAX, "no_observed_consequence"),
        (100, 100, "semantic_consequence_observed"),
        (2, 1, "config_execution_rejected"),
        (1, 0, "config_execution_rejected"),
    ] {
        let s = spec(&i, num, den);
        let r = s::analyze(&s, &i).unwrap();
        assert_eq!(r["status"], status, "{num}/{den}: {}", r["failure"]);
        p::verify(&s, &r).unwrap();
        p::reproduce(&s, &r).unwrap();
    }
    // This retained deployed fee code rounds up: two tiny distinct valid
    // rationals each charge one token for this identical retained deposit.
    let a = s::analyze(&spec(&i, 1, u64::MAX), &i).unwrap();
    let b = s::analyze(&spec(&i, 2, u64::MAX), &i).unwrap();
    assert_eq!(
        a["proposed"]["reconciliation"],
        b["proposed"]["reconciliation"]
    );
}
#[test]
fn tampering_and_resealed_inconsistent_evidence_fail_closed() {
    let i = input();
    let s = spec(&i, 1, 100);
    let r = s::analyze(&s, &i).unwrap();
    for path in [
        "/retained_input/record/clock/slot",
        "/retained_input/record_sha256",
        "/retained_input/programs/0/elf",
        "/retained_input/programs/0/loader",
        "/retained_input/record/accounts/0/account/data",
        "/simulated_config_instruction/execution/success",
        "/simulated_config_instruction/manager_assumption/address",
        "/simulated_config_instruction/message/account_keys/0",
        "/simulated_config_instruction/execution_sha256",
        "/simulated_proposed_pre_state/pool/data",
        "/baseline/execution/fee",
        "/proposed/reconciliation/mint_supply_delta_raw",
        "/findings/0/proposed_raw",
    ] {
        let mut v = r.clone();
        let f = v.pointer_mut(path).unwrap();
        *f = if f.is_boolean() {
            json!(false)
        } else if f.is_number() {
            json!(1)
        } else {
            json!("corrupt")
        };
        assert!(p::verify(&s, &seal(v)).is_err(), "{path}");
    }
    let mut missing = i.clone();
    missing.programs.clear();
    let r = s::analyze(&s, &missing).unwrap();
    assert_eq!(r["status"], "config_evidence_missing");
    assert_eq!(r["execution_performed"], false);
    let mut v = s::analyze(&s, &i).unwrap();
    v["proposed"]["execution"]["accounts"]["pool-mint"]["data"] = json!("00");
    v["proposed"]["execution_sha256"] =
        eplyx_engine::canonical::digest(&v["proposed"]["execution"])
            .unwrap()
            .into();
    assert!(p::verify(&s, &seal(v)).is_err());
}
#[test]
fn local_bundle_cli_durable_reader_and_offline_reproduction() {
    let root = tempfile::tempdir().unwrap();
    let i = input();
    let s = spec(&i, 1, 100);
    let change = root.path().join("change.json");
    let report = root.path().join("report.json");
    std::fs::write(&change, s.to_document().unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["parameter", "analyse", "--change"])
        .arg(&change)
        .args([
            "--bundle",
            "../deploy/bundle",
            "--record-id",
            "mainnet-spl-stake-pool-151010f709e113e7",
            "--out",
        ])
        .arg(&report)
        .arg("--record")
        .arg(root.path())
        .env("SOLANA_RPC_URL", "https://unused.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    let store = eplyx_engine::dashboard::store::Store::open(root.path()).unwrap();
    let run = eplyx_engine::dashboard::view::load(&store, receipt["run_id"].as_str().unwrap());
    assert!(run.problems().is_empty(), "{:?}", run.problems());
    assert_eq!(eplyx_engine::dashboard::view::state(&run), "Complete");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["parameter", "reproduce", "--change"])
        .arg(&change)
        .arg("--report")
        .arg(&report)
        .env("SOLANA_RPC_URL", "https://unused.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["reproduced"],
        true
    );
}
