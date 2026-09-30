//! Step 9B's retained-evidence gate. Never substitute a candidate executable.
use borsh::BorshDeserialize;
use eplyx_engine::{
    bundle::CiBundle,
    executor::{execute_probe_message, ProgramVersion},
    replay::{hash_bytes, load_dependencies, ReplayFidelity},
    types::{AccountSnapshot, NamedAccount},
};
use solana_clock::Clock;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_message::Message;
use spl_stake_pool::{
    instruction::StakePoolInstruction,
    state::{Fee, FeeType, StakePool},
};

#[test]
fn retained_deposit_baseline_gate() {
    let bundle = CiBundle::open("../deploy/bundle").unwrap();
    let record = bundle
        .records()
        .iter()
        .find(|r| r.id == "mainnet-spl-stake-pool-151010f709e113e7")
        .unwrap();
    let program = ProgramVersion::from_file("historical-baseline", bundle.baseline()).unwrap();
    assert_eq!(hash_bytes(&program.bytes), record.current_program_sha256);
    let dependencies =
        load_dependencies(std::slice::from_ref(record), &bundle.dependencies()).unwrap();
    let result = record.execute(&program, &dependencies).unwrap();
    println!(
        "success={} fee={} error={:?} logs={:?}",
        result.success, result.fee, result.error, result.logs
    );
    println!(
        "fidelity failures: {:?}",
        record.fidelity_failures(&result).unwrap()
    );
    // Historical archive records use Matched; Exact is controlled snapshots.
    assert_eq!(record.fidelity(&result).unwrap(), ReplayFidelity::Matched);

    let pool = record
        .accounts
        .iter()
        .find(|a| a.label == "stake-pool")
        .unwrap();
    let mut remaining = pool.account.data.as_slice();
    let before = StakePool::deserialize(&mut remaining).unwrap();
    let encoded_len = pool.account.data.len() - remaining.len();
    assert_eq!(
        borsh::to_vec(&before).unwrap(),
        pool.account.data[..encoded_len]
    );
    let manager = before.manager.to_string();
    assert!(!record.accounts.iter().any(|a| a.address == manager));
    let payer = bs58::encode([77; 32]).into_string();
    let envelope = |lamports| AccountSnapshot {
        lamports,
        owner: "11111111111111111111111111111111".into(),
        data: vec![],
        executable: false,
        rent_epoch: 0,
    };
    let accounts = vec![
        pool.clone(),
        NamedAccount {
            label: "assumed-manager".into(),
            address: manager.clone(),
            account: envelope(1_000_000),
        },
        NamedAccount {
            label: "simulation-payer".into(),
            address: payer.clone(),
            account: envelope(10_000_000),
        },
    ];
    let proposed_fee = Fee {
        denominator: 100,
        numerator: 1,
    };
    let official = spl_stake_pool::instruction::set_fee(
        &spl_stake_pool::id(),
        &pool.address.parse().unwrap(),
        &before.manager,
        FeeType::SolDeposit(proposed_fee),
    );
    assert_eq!(
        official.data,
        borsh::to_vec(&StakePoolInstruction::SetFee {
            fee: FeeType::SolDeposit(proposed_fee)
        })
        .unwrap()
    );
    let ix = Instruction {
        program_id: record.program_id.parse().unwrap(),
        accounts: vec![
            AccountMeta::new(pool.address.parse().unwrap(), false),
            AccountMeta::new_readonly(manager.parse().unwrap(), true),
        ],
        data: official.data,
    };
    let message = Message::new(&[ix], Some(&payer.parse().unwrap()));
    let watch = accounts
        .iter()
        .map(|a| a.address.clone())
        .collect::<Vec<_>>();
    let clock = Clock {
        slot: record.clock.slot,
        epoch: record.clock.epoch,
        epoch_start_timestamp: record.clock.epoch_start_timestamp,
        leader_schedule_epoch: record.clock.leader_schedule_epoch,
        unix_timestamp: record.clock.unix_timestamp,
    };
    let mut config_programs = dependencies.programs().to_vec();
    config_programs.push(eplyx_engine::executor::LoadedProgram {
        program_id: record.program_id.parse().unwrap(),
        loader: eplyx_engine::versions::UPGRADEABLE_LOADER_ID
            .parse()
            .unwrap(),
        bytes: program.bytes.clone(),
    });
    let config =
        execute_probe_message(&accounts, &watch, clock, &config_programs, message).unwrap();
    println!(
        "SetFee success={} CU={} fee={}",
        config.success, config.compute_units, config.transaction_fee_lamports
    );
    assert!(
        config.success,
        "config execution rejected: {:?}",
        config.error
    );
    let after_account = &config.post_accounts[&pool.address];
    let mut bytes = after_account.data.as_slice();
    let after = StakePool::deserialize(&mut bytes).unwrap();
    let mut expected = before.clone();
    expected.sol_deposit_fee = proposed_fee;
    assert_eq!(after, expected);
    assert_eq!(bytes, remaining);
    let mut envelope_restored = after_account.clone();
    envelope_restored.data = pool.account.data.clone();
    assert_eq!(envelope_restored, pool.account);
    assert_eq!(config.post_accounts[&manager], accounts[1].account);
    assert_eq!(
        config.post_accounts[&payer].lamports,
        accounts[2].account.lamports - config.transaction_fee_lamports
    );
    let mut proposed = record.clone();
    proposed
        .accounts
        .iter_mut()
        .find(|a| a.address == pool.address)
        .unwrap()
        .account = after_account.clone();
    proposed.pre_state_hash = eplyx_engine::replay::state_hash(&proposed.accounts).unwrap();
    let paired = proposed.execute(&program, &dependencies).unwrap();
    println!("paired success={} error={:?}", paired.success, paired.error);
    assert!(paired.success, "{:?}", paired.logs);
}
