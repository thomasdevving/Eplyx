//! Bounded feasibility probe, not a composite API or candidate admission path.
use eplyx_engine::{
    bundle::CiBundle,
    change::ChangeSpec,
    executor::{execute_probe_message, LoadedProgram, ProgramVersion},
    parameter_change::stake_pool as s,
    path::ProbeMessage,
    replay::{hash_bytes, load_dependencies},
    types::NamedAccount,
};
use serde_json::json;
use solana_clock::Clock;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_message::Message;

#[test]
fn same_code_control_and_deposit_only_candidate_boundary() {
    let root = eplyx_engine::repo_root();
    let bundle = CiBundle::open(root.join("deploy/bundle")).unwrap();
    let input = s::Input::from_bundle(&bundle, "mainnet-spl-stake-pool-151010f709e113e7").unwrap();
    let spec = ChangeSpec::parse(
        &std::fs::read(root.join("docs/examples/stake-pool-parameter-change.json")).unwrap(),
    )
    .unwrap();
    let original_input = input.clone();
    // Each analysis independently executes its own SetFee and both actions.
    let v1 = s::analyze(&spec, &input).unwrap();
    let control = s::analyze(&spec, &input).unwrap();
    s::verify(&spec, &v1).unwrap();
    s::reproduce(&spec, &control).unwrap();
    assert_eq!(v1, control);
    assert_eq!(v1["baseline_fidelity"]["status"], "matched");
    let credit = |r: &serde_json::Value, side: &str| {
        assert_eq!(r[side]["reconciliation"]["reconciled"], true);
        r[side]["reconciliation"]["recipient_account_credit_raw"]
            .as_str()
            .unwrap()
            .parse::<i128>()
            .unwrap()
    };
    let effect_v1 = credit(&v1, "proposed") - credit(&v1, "baseline");
    let effect_control = credit(&control, "proposed") - credit(&control, "baseline");
    assert_eq!(effect_v1, -7_609_851);
    assert_eq!(effect_control - effect_v1, 0);

    // Load candidate bytes as an explicit overlay; never reseal a historical
    // record/manifest as if these bytes were deployed at the retained slot.
    let candidate = ProgramVersion::from_file(
        "constructed-deposit-only-fixture",
        root.join("artifacts/fixture_stake_pool_v2.so"),
    )
    .unwrap();
    assert_eq!(
        hash_bytes(&candidate.bytes),
        "3193eabd9fe2e479109ef3b2dd7301fffd06774325133ff8f88916ed482db099"
    );
    let upgrade = ChangeSpec::program_upgrade(&input.record.program_id, &candidate.bytes);
    let dependencies =
        load_dependencies(std::slice::from_ref(&input.record), &bundle.dependencies()).unwrap();
    let action = input.record.execute(&candidate, &dependencies).unwrap();
    assert!(action.success, "{:?}", action.logs);
    let reconciled = s::reconcile(&input.record, &action).unwrap();

    let config = &v1["simulated_config_instruction"];
    let accounts: Vec<NamedAccount> =
        serde_json::from_value(config["pre_accounts"].clone()).unwrap();
    let watch: Vec<String> = serde_json::from_value(config["watch"].clone()).unwrap();
    let manager = config["manager_assumption"]["address"].as_str().unwrap();
    let payer = config["fee_payer"]["address"].as_str().unwrap();
    let pool = accounts.iter().find(|a| a.label == "stake-pool").unwrap();
    let mut programs = input
        .programs
        .iter()
        .map(|p| LoadedProgram {
            program_id: p.program_id.parse().unwrap(),
            loader: p.loader.parse().unwrap(),
            bytes: if p.program_id == input.record.program_id {
                candidate.bytes.clone()
            } else {
                p.elf.clone()
            },
        })
        .collect::<Vec<_>>();
    let c = &input.record.clock;
    let clock = Clock {
        slot: c.slot,
        epoch: c.epoch,
        epoch_start_timestamp: c.epoch_start_timestamp,
        leader_schedule_epoch: c.leader_schedule_epoch,
        unix_timestamp: c.unix_timestamp,
    };
    let official = spl_stake_pool::instruction::set_fee(
        &spl_stake_pool::id(),
        &pool.address.parse().unwrap(),
        &manager.parse().unwrap(),
        spl_stake_pool::state::FeeType::SolDeposit(spl_stake_pool::state::Fee {
            numerator: 1,
            denominator: 100,
        }),
    );
    let instruction = Instruction {
        program_id: input.record.program_id.parse().unwrap(),
        accounts: vec![
            AccountMeta::new(pool.address.parse().unwrap(), false),
            AccountMeta::new_readonly(manager.parse().unwrap(), true),
        ],
        data: official.data,
    };
    let message = Message::new(&[instruction], Some(&payer.parse().unwrap()));
    assert_eq!(json!(ProbeMessage::from(&message)), config["message"]);
    let run_config = |loaded: &[LoadedProgram]| {
        execute_probe_message(&accounts, &watch, clock.clone(), loaded, message.clone()).unwrap()
    };
    let rejected = run_config(&programs);
    assert!(!rejected.success);
    assert!(rejected
        .logs
        .iter()
        .any(|l| l.contains("supports DepositSol only")));
    assert_eq!(rejected.transaction_fee_lamports, 10_000);
    assert!(rejected.inner_instructions.is_empty());
    for a in &accounts {
        let mut expected = a.account.clone();
        if a.address == payer {
            expected.lamports -= rejected.transaction_fee_lamports;
        }
        assert_eq!(rejected.post_accounts[&a.address], expected);
    }
    // Same explicit executable-loading seam with V1 must reproduce K1 exactly.
    programs
        .iter_mut()
        .find(|p| p.program_id.to_string() == input.record.program_id)
        .unwrap()
        .bytes = input
        .programs
        .iter()
        .find(|p| p.program_id == input.record.program_id)
        .unwrap()
        .elf
        .clone();
    assert_eq!(json!(run_config(&programs)), config["execution"]);
    assert_eq!(input, original_input);
    let evidence = json!({
        "schema":"eplyx-upgrade-parameter-feasibility-probe-v1",
        "claim":"code/configuration interaction under retained state",
        "retained_input":input,
        "upgrade_change_spec_id":upgrade.id().unwrap(),
        "parameter_change_spec_id":spec.id().unwrap(),
        "candidate":{"origin":"constructed test fixture; not upstream release",
            "sha256":hash_bytes(&candidate.bytes),"len":candidate.bytes.len()},
        "v1_report":v1,"same_code_report":control,
        "same_code_recipient_effect_raw":effect_control.to_string(),
        "same_code_interaction_raw":(effect_control-effect_v1).to_string(),
        "distinct_candidate_config":{"execution":rejected,"rollback_verified":true,
            "assumption":"V1 envelope used only to probe rejection; candidate manager boundary unqualified"},
        "distinct_candidate_c0":{"execution":action,"reconciliation":reconciled},
        "distinct_candidate_c1":{"status":"unavailable: SetFee rejected; no verified K2 pool"},
        "distinct_candidate_effect_raw":null,"distinct_candidate_interaction_raw":null
    });
    if let Some(dir) = std::env::var_os("EPLYX_INTERACTION_PROBE_OUT") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            std::path::PathBuf::from(dir).join("probe.json"),
            eplyx_engine::canonical::document(&evidence).unwrap(),
        )
        .unwrap();
    }
    println!("same-code effect={effect_control}; interaction=0; distinct K2 rejected, R11 unavailable; R10={reconciled}");
}
