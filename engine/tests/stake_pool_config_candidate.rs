//! Fixture-only qualification. No production evaluator, admission route or API.
use anyhow::{ensure, Result};
use borsh::BorshDeserialize;
use eplyx_engine::{
    bundle::CiBundle,
    canonical,
    change::ChangeSpec,
    executor::{
        execute_probe_message, ExecutionResult, LoadedProgram, ProbeTransactionExecution,
        ProgramVersion,
    },
    parameter_change::stake_pool as s,
    path::ProbeMessage,
    replay::{self, DependencyBundle, ReplayRecord},
    types::{AccountSnapshot, NamedAccount},
};
use serde_json::{json, Value};
use solana_clock::Clock;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_message::Message;
use spl_stake_pool::state::{AccountType, Fee, FeeType, FutureEpoch, StakePool};

const CANDIDATE_SHA: &str = "a664f74b73dedc713f16934829b25f9a0c0c3a06c6ce21f03fdc7869ae5b555d";
const ORIGINAL_SHA: &str = "3193eabd9fe2e479109ef3b2dd7301fffd06774325133ff8f88916ed482db099";
const C1: Fee = Fee {
    numerator: 1,
    denominator: 100,
};

struct Harness {
    input: s::Input,
    spec: ChangeSpec,
    baseline: Value,
    candidate: ProgramVersion,
    dependencies: DependencyBundle,
    build: Value,
}
impl Harness {
    fn new() -> Self {
        let root = eplyx_engine::repo_root();
        let bundle = CiBundle::open(root.join("deploy/bundle")).unwrap();
        let input =
            s::Input::from_bundle(&bundle, "mainnet-spl-stake-pool-151010f709e113e7").unwrap();
        let spec = ChangeSpec::parse(
            &std::fs::read(root.join("docs/examples/stake-pool-parameter-change.json")).unwrap(),
        )
        .unwrap();
        let baseline = s::analyze(&spec, &input).unwrap();
        s::verify(&spec, &baseline).unwrap();
        let candidate = ProgramVersion::from_file(
            "constructed-config-fixture",
            root.join("artifacts/fixture_stake_pool_config_v2.so"),
        )
        .unwrap();
        assert_eq!(replay::hash_bytes(&candidate.bytes), CANDIDATE_SHA);
        assert_eq!(candidate.bytes.len(), 133_992);
        assert_ne!(CANDIDATE_SHA, input.record.current_program_sha256);
        assert_eq!(
            replay::hash_bytes(
                &std::fs::read(root.join("artifacts/fixture_stake_pool_v2.so")).unwrap()
            ),
            ORIGINAL_SHA
        );
        let build: Value = serde_json::from_slice(
            &std::fs::read(root.join("artifacts/fixture_stake_pool_config_v2.build.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(build["elf"]["sha256"], CANDIDATE_SHA);
        for (file, expected) in build["files"].as_object().unwrap() {
            let bytes = std::fs::read(
                root.join("programs/fixture-stake-pool-config-candidate")
                    .join(file),
            )
            .unwrap();
            assert_eq!(expected["sha256"], replay::hash_bytes(&bytes));
            assert_eq!(expected["len"], bytes.len());
        }
        let dependencies =
            replay::load_dependencies(std::slice::from_ref(&input.record), &bundle.dependencies())
                .unwrap();
        Self {
            input,
            spec,
            baseline,
            candidate,
            dependencies,
            build,
        }
    }
    fn programs(&self) -> Vec<LoadedProgram> {
        self.input
            .programs
            .iter()
            .map(|p| LoadedProgram {
                program_id: p.program_id.parse().unwrap(),
                loader: p.loader.parse().unwrap(),
                bytes: if p.program_id == self.input.record.program_id {
                    self.candidate.bytes.clone()
                } else {
                    p.elf.clone()
                },
            })
            .collect()
    }
    fn clock(&self) -> Clock {
        let c = &self.input.record.clock;
        Clock {
            slot: c.slot,
            epoch: c.epoch,
            epoch_start_timestamp: c.epoch_start_timestamp,
            leader_schedule_epoch: c.leader_schedule_epoch,
            unix_timestamp: c.unix_timestamp,
        }
    }
    fn config(&self, fee: Fee, mode: &str) -> Value {
        let template = &self.baseline["simulated_config_instruction"];
        let mut pre: Vec<NamedAccount> =
            serde_json::from_value(template["pre_accounts"].clone()).unwrap();
        let payer = template["fee_payer"]["address"].as_str().unwrap();
        let pool_index = pre.iter().position(|a| a.label == "stake-pool").unwrap();
        let address = pre[pool_index].address.clone();
        let (state, _) = decode(&pre[pool_index].account).unwrap();
        let mut manager = state.manager.to_string();
        let manager_index = pre.iter().position(|a| a.address == manager).unwrap();
        let mut signer = true;
        match mode {
            "wrong_manager" => {
                manager = bs58::encode([91; 32]).into_string();
                pre[manager_index].address = manager.clone();
            }
            "missing_signer" => signer = false,
            "manager_owner" => {
                pre[manager_index].account.owner = bs58::encode([92; 32]).into_string()
            }
            "manager_data" => {
                pre[manager_index].account.data = vec![17; 8];
                pre[manager_index].account.lamports = 2_000_000;
            }
            "manager_balance" => pre[manager_index].account.lamports = 3_000_000,
            "variable_layout" => {
                let mut state = state.clone();
                state.preferred_deposit_validator_vote_address = Some(state.manager);
                state.preferred_withdraw_validator_vote_address = Some(state.staker);
                state.sol_deposit_authority = Some(state.manager);
                state.sol_withdraw_authority = Some(state.staker);
                state.next_epoch_fee = FutureEpoch::One(Fee {
                    numerator: 7,
                    denominator: 123,
                });
                state.next_stake_withdrawal_fee = FutureEpoch::Two(Fee {
                    numerator: 8,
                    denominator: 456,
                });
                state.next_sol_withdrawal_fee = FutureEpoch::One(Fee {
                    numerator: 9,
                    denominator: 789,
                });
                pre[pool_index].account.data = borsh::to_vec(&state).unwrap();
                pre[pool_index].account.data.extend_from_slice(&[0xAB; 64]);
            }
            "malformed_layout" => pre[pool_index].account.data = vec![1; 200],
            "uninitialized" => pre[pool_index].account.data[0] = 0,
            "incorrect_owner" => {
                pre[pool_index].account.owner = "11111111111111111111111111111111".into()
            }
            _ => {}
        }
        let official = spl_stake_pool::instruction::set_fee(
            &spl_stake_pool::id(),
            &address.parse().unwrap(),
            &manager.parse().unwrap(),
            FeeType::SolDeposit(fee),
        );
        let mut ix = Instruction {
            program_id: self.input.record.program_id.parse().unwrap(),
            accounts: vec![
                AccountMeta::new(address.parse().unwrap(), false),
                AccountMeta::new_readonly(manager.parse().unwrap(), signer),
            ],
            data: official.data,
        };
        if mode == "other_fee" {
            ix.data = borsh::to_vec(&spl_stake_pool::instruction::StakePoolInstruction::SetFee {
                fee: FeeType::Epoch(fee),
            })
            .unwrap();
        }
        if mode == "trailing_instruction" {
            ix.data.push(0);
        }
        if mode == "extra_config_account" {
            ix.accounts
                .push(AccountMeta::new_readonly(payer.parse().unwrap(), false));
        }
        let message = Message::new(&[ix], Some(&payer.parse().unwrap()));
        let watch = pre.iter().map(|a| a.address.clone()).collect::<Vec<_>>();
        let execution = execute_probe_message(
            &pre,
            &watch,
            self.clock(),
            &self.programs(),
            message.clone(),
        )
        .unwrap();
        let mut case = json!({"origin":"labelled_fixture_qualification_control","mode":mode,
            "program_sha256":CANDIDATE_SHA,"pre_accounts":pre,"watch":watch,"clock":self.input.record.clock,
            "message":ProbeMessage::from(&message),"pool_address":address,"payer":payer,
            "manager_assumption":{"address":manager,"signer":signer,"origin":"assumed_simulation_only","key_possession_established":false},
            "fee_payer":{"address":payer,"origin":"assumed_simulation_only","propagated_to_user_action":false},
            "fee":s::RationalFee::from(fee),"execution":execution});
        case["config_input_id"] = canonical::digest(&(
            "eplyx-fixture-config-input-v1",
            CANDIDATE_SHA,
            &pre,
            ProbeMessage::from(&message),
            &self.input.record.clock,
            &self.input.record.dependencies,
        ))
        .unwrap()
        .into();
        case["execution_sha256"] = canonical::digest(&execution).unwrap().into();
        verify_config(&case).unwrap();
        case
    }
    fn action(&self, record: &ReplayRecord) -> Value {
        let execution = record.execute(&self.candidate, &self.dependencies).unwrap();
        let (reconciliation, reconciliation_error) = match s::reconcile(record, &execution) {
            Ok(value) => (value, None),
            Err(error) => (Value::Null, Some(error.to_string())),
        };
        json!({"pre_record":record,"program_sha256":CANDIDATE_SHA,"execution":execution,
            "action_input_id":canonical::digest(&("eplyx-fixture-action-input-v1",CANDIDATE_SHA,record,s::runtime())).unwrap(),
            "execution_sha256":canonical::digest(&execution).unwrap(),"reconciliation":reconciliation,
            "reconciliation_error":reconciliation_error})
    }
}

fn decode(a: &AccountSnapshot) -> Result<(StakePool, Vec<u8>)> {
    ensure!(
        a.owner == spl_stake_pool::id().to_string() && !a.executable,
        "pool envelope"
    );
    let mut bytes = a.data.as_slice();
    let state = StakePool::deserialize(&mut bytes)?;
    ensure!(state.account_type == AccountType::StakePool, "pool type");
    ensure!(
        borsh::to_vec(&state)? == a.data[..a.data.len() - bytes.len()],
        "typed roundtrip"
    );
    Ok((state, bytes.to_vec()))
}

// The same complete typed/envelope/trailing-byte and non-pool preservation
// invariants as the private production checker, applied only inside this test.
fn verify_config(case: &Value) -> Result<()> {
    let pre: Vec<NamedAccount> = serde_json::from_value(case["pre_accounts"].clone())?;
    let execution: ProbeTransactionExecution = serde_json::from_value(case["execution"].clone())?;
    let message: ProbeMessage = serde_json::from_value(case["message"].clone())?;
    ensure!(
        execution.transaction_fee_lamports == u64::from(message.required_signatures) * 5000,
        "config fee"
    );
    ensure!(
        execution.inner_instructions.is_empty() && execution.post_accounts.len() == pre.len(),
        "config closure/CPI"
    );
    for a in &pre {
        let mut expected = a.account.clone();
        if execution.success && a.address == case["pool_address"].as_str().unwrap() {
            let (mut state, trailing) = decode(&a.account)?;
            let fee: s::RationalFee = serde_json::from_value(case["fee"].clone())?;
            state.sol_deposit_fee = Fee::from(&fee);
            let mut data = borsh::to_vec(&state)?;
            data.extend_from_slice(&trailing);
            expected.data = data;
        }
        if a.address == case["payer"].as_str().unwrap() {
            expected.lamports = expected
                .lamports
                .checked_sub(execution.transaction_fee_lamports)
                .unwrap();
        }
        ensure!(
            execution.post_accounts.get(&a.address) == Some(&expected),
            "config preservation/rollback {}",
            a.address
        );
    }
    ensure!(
        execution.success == execution.error.is_none(),
        "config status/error"
    );
    Ok(())
}
fn proposed(record: &ReplayRecord, next: AccountSnapshot) -> ReplayRecord {
    let mut r = record.clone();
    r.accounts
        .iter_mut()
        .find(|a| a.label == "stake-pool")
        .unwrap()
        .account = next;
    r.pre_state_hash = replay::state_hash(&r.accounts).unwrap();
    let mut restored = r.clone();
    restored.accounts = record.accounts.clone();
    restored.pre_state_hash = record.pre_state_hash.clone();
    assert_eq!(restored, *record, "only pool overlay permitted");
    r
}
fn config_controls(h: &Harness) -> Vec<Value> {
    let mut cases = Vec::new();
    for (mode, fee, success) in [
        ("correct", C1, true),
        ("wrong_manager", C1, false),
        ("missing_signer", C1, false),
        ("manager_owner", C1, true),
        ("manager_data", C1, true),
        ("manager_balance", C1, true),
        (
            "zero",
            Fee {
                numerator: 0,
                denominator: 0,
            },
            true,
        ),
        (
            "equal",
            Fee {
                numerator: 0,
                denominator: 1000,
            },
            true,
        ),
        (
            "invalid_zero",
            Fee {
                numerator: 1,
                denominator: 0,
            },
            false,
        ),
        (
            "invalid_high",
            Fee {
                numerator: 2,
                denominator: 1,
            },
            false,
        ),
        (
            "tiny",
            Fee {
                numerator: 1,
                denominator: u64::MAX,
            },
            true,
        ),
        ("variable_layout", C1, true),
        ("malformed_layout", C1, false),
        ("uninitialized", C1, false),
        ("incorrect_owner", C1, false),
        ("other_fee", C1, false),
        ("trailing_instruction", C1, false),
        ("extra_config_account", C1, false),
    ] {
        let case = h.config(fee, mode);
        assert_eq!(
            case["execution"]["success"], success,
            "{mode}: {}",
            case["execution"]
        );
        if mode == "wrong_manager" || mode == "missing_signer" {
            assert!(case["execution"]["logs"]
                .as_array()
                .unwrap()
                .iter()
                .any(
                    |l| l.as_str().unwrap().contains(if mode == "wrong_manager" {
                        "Incorrect manager"
                    } else {
                        "manager signature missing"
                    })
                ));
        }
        cases.push(case);
    }
    cases
}
fn mutate_pool(record: &mut ReplayRecord, change: impl FnOnce(&mut StakePool)) {
    let a = record
        .accounts
        .iter_mut()
        .find(|a| a.label == "stake-pool")
        .unwrap();
    let (mut state, trailing) = decode(&a.account).unwrap();
    change(&mut state);
    a.account.data = borsh::to_vec(&state).unwrap();
    a.account.data.extend(trailing);
    record.pre_state_hash = replay::state_hash(&record.accounts).unwrap();
}
fn action_controls(h: &Harness) -> Vec<Value> {
    let mut cases = Vec::new();
    for mode in [
        "withdraw_pda",
        "reserve_relationship",
        "mint_relationship",
        "manager_relationship",
        "token_relationship",
        "gated",
        "referral_nonzero",
        "bad_token_mint",
        "bad_token_owner",
        "bad_mint_authority",
        "bad_pool_owner",
        "bad_reserve_owner",
        "frozen_role",
        "stale_epoch",
        "insufficient_funding",
        "corrupt_manager_balance",
        "full_fee_deposit",
        "all_fee_roles_alias",
        "tiny_round_up",
        "zero_fee",
        "equal_fee",
    ] {
        let mut record = h.input.record.clone();
        match mode {
            "withdraw_pda" => mutate_pool(&mut record, |s| {
                s.stake_withdraw_bump_seed = s.stake_withdraw_bump_seed.wrapping_add(1)
            }),
            "reserve_relationship" => mutate_pool(&mut record, |s| s.reserve_stake = s.manager),
            "mint_relationship" => mutate_pool(&mut record, |s| s.pool_mint = s.manager),
            "manager_relationship" => {
                mutate_pool(&mut record, |s| s.manager_fee_account = s.manager)
            }
            "token_relationship" => mutate_pool(&mut record, |s| s.token_program_id = s.manager),
            "gated" => mutate_pool(&mut record, |s| s.sol_deposit_authority = Some(s.manager)),
            "referral_nonzero" => mutate_pool(&mut record, |s| s.sol_referral_fee = 1),
            "stale_epoch" => {
                record.clock.epoch = 1037;
            }
            "bad_pool_owner" => {
                record
                    .accounts
                    .iter_mut()
                    .find(|a| a.label == "stake-pool")
                    .unwrap()
                    .account
                    .owner = "11111111111111111111111111111111".into()
            }
            "bad_reserve_owner" => {
                record
                    .accounts
                    .iter_mut()
                    .find(|a| a.label == "reserve-stake")
                    .unwrap()
                    .account
                    .owner = "11111111111111111111111111111111".into();
            }
            "frozen_role" => {
                record
                    .accounts
                    .iter_mut()
                    .find(|a| a.label == "destination-pool-token")
                    .unwrap()
                    .account
                    .data[108] = 2;
            }
            "bad_token_mint" => record
                .accounts
                .iter_mut()
                .find(|a| a.label == "destination-pool-token")
                .unwrap()
                .account
                .data[..32]
                .fill(7),
            "bad_token_owner" => {
                record
                    .accounts
                    .iter_mut()
                    .find(|a| a.label == "destination-pool-token")
                    .unwrap()
                    .account
                    .owner = "11111111111111111111111111111111".into()
            }
            "bad_mint_authority" => record
                .accounts
                .iter_mut()
                .find(|a| a.label == "pool-mint")
                .unwrap()
                .account
                .data[4..36]
                .fill(7),
            "insufficient_funding" => {
                record
                    .accounts
                    .iter_mut()
                    .find(|a| a.address == record.transaction.payer)
                    .unwrap()
                    .account
                    .lamports = 1_000_000
            }
            _ => {
                let fee = if mode == "full_fee_deposit" {
                    Fee {
                        numerator: 1,
                        denominator: 1,
                    }
                } else if matches!(mode, "all_fee_roles_alias" | "corrupt_manager_balance") {
                    C1
                } else if mode == "tiny_round_up" {
                    Fee {
                        numerator: 1,
                        denominator: u64::MAX,
                    }
                } else if mode == "zero_fee" {
                    Fee {
                        numerator: 0,
                        denominator: 0,
                    }
                } else {
                    Fee {
                        numerator: 0,
                        denominator: 1000,
                    }
                };
                let k = h.config(fee, "correct");
                record = proposed(
                    &record,
                    serde_json::from_value::<ProbeTransactionExecution>(k["execution"].clone())
                        .unwrap()
                        .post_accounts[&k["pool_address"].as_str().unwrap().to_string()]
                        .clone(),
                );
                if mode == "corrupt_manager_balance" {
                    record
                        .accounts
                        .iter_mut()
                        .find(|a| a.label == "manager-fee")
                        .unwrap()
                        .account
                        .data[64..72]
                        .copy_from_slice(&u64::MAX.to_le_bytes());
                }
                if mode == "all_fee_roles_alias" {
                    let recipient = record
                        .accounts
                        .iter()
                        .find(|a| a.label == "destination-pool-token")
                        .unwrap()
                        .address
                        .clone();
                    mutate_pool(&mut record, |s| {
                        s.manager_fee_account = recipient.parse().unwrap()
                    });
                    record
                        .transaction
                        .instructions
                        .iter_mut()
                        .find(|ix| ix.program == record.program_id)
                        .unwrap()
                        .accounts[5]
                        .address = recipient;
                }
            }
        }
        record.pre_state_hash = replay::state_hash(&record.accounts).unwrap();
        let outcome = h.action(&record);
        assert_eq!(
            outcome["execution"]["success"],
            matches!(
                mode,
                "tiny_round_up"
                    | "zero_fee"
                    | "equal_fee"
                    | "all_fee_roles_alias"
                    | "corrupt_manager_balance"
            ),
            "{mode}: {}",
            outcome["execution"]
        );
        if mode == "tiny_round_up" {
            assert_eq!(
                outcome["reconciliation"]["manager_fee_account_credit_raw"],
                "1"
            );
        }
        if mode == "corrupt_manager_balance" {
            // The pinned Token CPI's observed behavior on this deliberately
            // inconsistent ledger is retained, never converted to a zero.
            assert_eq!(outcome["reconciliation"], Value::Null);
            assert_eq!(
                outcome["reconciliation_error"],
                "negative minted account credit"
            );
            assert_eq!(credit(&outcome), None);
        }
        if mode == "all_fee_roles_alias" {
            assert_eq!(
                outcome["reconciliation"]["manager_fee_account_credit_raw"],
                Value::Null
            );
            assert_eq!(
                outcome["reconciliation"]["unique_token_account_credits"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                outcome["reconciliation"]["recipient_account_credit_raw"],
                outcome["reconciliation"]["mint_supply_delta_raw"]
            );
        }
        if outcome["execution"]["success"] == false {
            assert_eq!(outcome["reconciliation"]["rollback_verified"], true);
        }
        cases.push(
            json!({"origin":"labelled_synthetic_action_control","mode":mode,"outcome":outcome}),
        );
    }
    cases
}

fn message_controls(h: &Harness) -> Vec<Value> {
    let mut cases = Vec::new();
    for mode in [
        "missing_funding_signer",
        "extra_deposit_account",
        "missing_deposit_account",
        "trailing_deposit_instruction",
        "unsupported_instruction",
        "wrong_target_program",
        "readonly_manager_fee",
    ] {
        let mut accounts = h.input.record.accounts.clone();
        if mode == "readonly_manager_fee" {
            let config = h.config(C1, "correct");
            let execution: ProbeTransactionExecution =
                serde_json::from_value(config["execution"].clone()).unwrap();
            accounts
                .iter_mut()
                .find(|a| a.label == "stake-pool")
                .unwrap()
                .account =
                execution.post_accounts[config["pool_address"].as_str().unwrap()].clone();
        }
        let deposit = h
            .input
            .record
            .transaction
            .instructions
            .iter()
            .find(|ix| ix.program == h.input.record.program_id)
            .unwrap();
        let funding = &deposit.accounts[3].address;
        let manager_fee = &deposit.accounts[5].address;
        let payer = &h.input.record.transaction.payer;
        let mut instructions = h
            .input
            .record
            .transaction
            .instructions
            .iter()
            .map(|ix| Instruction {
                program_id: ix.program.parse().unwrap(),
                accounts: ix
                    .accounts
                    .iter()
                    .map(|a| AccountMeta {
                        pubkey: a.address.parse().unwrap(),
                        is_signer: a.is_signer
                            && !(mode == "missing_funding_signer" && a.address == *funding),
                        is_writable: a.is_writable
                            && !(mode == "readonly_manager_fee" && a.address == *manager_fee),
                    })
                    .collect(),
                data: ix.data.clone(),
            })
            .collect::<Vec<_>>();
        let last = instructions.last_mut().unwrap();
        let mut programs = h.programs();
        match mode {
            "extra_deposit_account" => last
                .accounts
                .push(AccountMeta::new_readonly(payer.parse().unwrap(), false)),
            "missing_deposit_account" => {
                last.accounts.remove(6);
            }
            "trailing_deposit_instruction" => last.data.push(0),
            "unsupported_instruction" => last.data = vec![9],
            "wrong_target_program" => {
                let key = bs58::encode([90; 32]).into_string();
                last.program_id = key.parse().unwrap();
                programs
                    .iter_mut()
                    .find(|p| p.program_id.to_string() == h.input.record.program_id)
                    .unwrap()
                    .program_id = last.program_id;
            }
            _ => {}
        }
        let message = Message::new(&instructions, Some(&payer.parse().unwrap()));
        let watch = accounts
            .iter()
            .map(|a| a.address.clone())
            .collect::<Vec<_>>();
        let x = execute_probe_message(&accounts, &watch, h.clock(), &programs, message.clone())
            .unwrap();
        assert!(!x.success, "{mode}: {x:?}");
        for a in &accounts {
            let mut expected = a.account.clone();
            if a.address == *payer {
                expected.lamports -= x.transaction_fee_lamports;
            }
            assert_eq!(
                x.post_accounts[&a.address], expected,
                "{mode}: rollback {}",
                a.address
            );
        }
        if mode == "missing_funding_signer" {
            assert!(x
                .error
                .as_ref()
                .unwrap()
                .contains("MissingRequiredSignature"));
        }
        if mode == "wrong_target_program" {
            assert!(x.error.as_ref().unwrap().contains("IncorrectProgramId"));
        }
        cases.push(json!({"origin":"labelled_synthetic_message_control","mode":mode,"pre_accounts":accounts,
            "clock":h.input.record.clock,"message":ProbeMessage::from(&message),"execution":x,"rollback_verified":true}));
    }
    cases
}
fn credit(cell: &Value) -> Option<i128> {
    if cell["reconciliation"]["reconciled"] != true || cell["execution"]["success"] != true {
        return None;
    }
    cell["reconciliation"]["recipient_account_credit_raw"]
        .as_str()?
        .parse()
        .ok()
}
fn effects(cells: &Value) -> Value {
    let pair = |a: &str, b: &str| Some(credit(&cells[b])? - credit(&cells[a])?);
    let v1 = pair("R00", "R01");
    let v2 = pair("R10", "R11");
    json!({"v1_recipient_raw":v1.map(|x|x.to_string()),"v2_recipient_raw":v2.map(|x|x.to_string()),
        "interaction_recipient_raw":v1.zip(v2).map(|(a,b)|(b-a).to_string())})
}
fn qualify(h: &Harness) -> Value {
    let unchanged = canonical::document(&h.input).unwrap();
    let baseline = s::analyze(&h.spec, &h.input).unwrap();
    let same_code = s::analyze(&h.spec, &h.input).unwrap();
    assert_eq!(baseline, same_code);
    s::reproduce(&h.spec, &same_code).unwrap();
    let k2 = h.config(C1, "correct");
    let config: ProbeTransactionExecution =
        serde_json::from_value(k2["execution"].clone()).unwrap();
    let next = config.post_accounts[k2["pool_address"].as_str().unwrap()].clone();
    let r11 = proposed(&h.input.record, next.clone());
    // Only K2's own checked output supplies the candidate action pre-state.
    assert_eq!(
        r11.accounts
            .iter()
            .find(|a| a.label == "stake-pool")
            .unwrap()
            .account,
        next
    );
    let r10 = h.action(&h.input.record);
    let r11 = h.action(&r11);
    let mut cells =
        json!({"R00":baseline["baseline"],"R01":baseline["proposed"],"R10":r10,"R11":r11});
    for (name, commitment) in [
        ("R00", "baseline_execution_commitment"),
        ("R01", "proposed_execution_commitment"),
    ] {
        cells[name]["pre_execution_commitment"] = baseline[commitment].clone();
        cells[name]["program_sha256"] = h.input.record.current_program_sha256.clone().into();
        cells[name]["action_input_id"] =
            canonical::digest(&("eplyx-fixture-action-input-v1", &baseline[commitment]))
                .unwrap()
                .into();
    }
    let mut k1 = baseline["simulated_config_instruction"].clone();
    k1["config_input_id"] = canonical::digest(&(
        "eplyx-fixture-config-input-v1",
        &k1["programs"],
        &k1["pre_accounts"],
        &k1["message"],
        &k1["clock"],
    ))
    .unwrap()
    .into();
    for name in ["R00", "R01", "R10", "R11"] {
        assert_eq!(cells[name]["execution"]["success"], true, "{name}");
        assert_eq!(cells[name]["reconciliation"]["reconciled"], true);
        assert_eq!(
            cells[name]["reconciliation"]["config_transaction_fee_included"],
            false
        );
        assert_eq!(
            cells[name]["reconciliation"]["referral_account_credit_raw"],
            Value::Null
        );
        assert_eq!(
            cells[name]["reconciliation"]["unique_token_account_credits"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    assert_eq!(baseline["baseline_fidelity"]["status"], "matched");
    assert_eq!(credit(&cells["R00"]), Some(760_985_008));
    assert_eq!(credit(&cells["R01"]), Some(753_375_157));
    // VM-produced V1 deltas and independent reconciliation are the oracle.
    for side in [("R00", "R10"), ("R01", "R11")] {
        assert_eq!(
            cells[side.0]["reconciliation"],
            cells[side.1]["reconciliation"]
        );
    }
    let effect = effects(&cells);
    assert_eq!(effect["interaction_recipient_raw"], "0");
    let mut missing = cells.clone();
    missing["R11"] = json!({"status":"unavailable"});
    assert_eq!(effects(&missing)["interaction_recipient_raw"], Value::Null);
    assert_eq!(effects(&missing)["v2_recipient_raw"], Value::Null);
    let configuration_controls = config_controls(h);
    let deposit_controls = action_controls(h);
    let message_controls = message_controls(h);
    assert_eq!(canonical::document(&h.input).unwrap(), unchanged);
    let upgrade = ChangeSpec::program_upgrade(&h.input.record.program_id, &h.candidate.bytes);
    let mut receipt = json!({"schema":"eplyx-stake-pool-config-candidate-qualification-v1",
        "origin":"constructed test fixture; not upstream release; not intended for deployment",
        "scope":{"layout":"full spl-stake-pool 2.0.3 Borsh typed-prefix roundtrip, exact trailing bytes",
            "instructions":["SetFee(SolDeposit)","ungated ten-account DepositSol"],"sol_referral_percent":0,
            "manager_boundary":"key and signer enforced; tested owner/data/balance variations irrelevant",
            "production_candidate_admission":false,"historical_fidelity_cell":"R00"},
        "build":h.build,"runtime":s::runtime(),"retained_input":h.input,
        "candidate":{"sha256":CANDIDATE_SHA,"len":h.candidate.bytes.len(),"elf_hex":eplyx_engine::hexfmt::encode(&h.candidate.bytes)},
        "harness_sha256":replay::hash_bytes(include_bytes!("stake_pool_config_candidate.rs")),
        "upgrade_change_spec":upgrade,"upgrade_change_spec_id":upgrade.id().unwrap(),
        "parameter_change_spec":h.spec,"parameter_change_spec_id":h.spec.id().unwrap(),
        "K1":k1,"K2":k2,
        "verified_K1_pool":baseline["simulated_proposed_pre_state"],"verified_K2_pool":next,
        "cells":cells,"effects":effect,"same_code_control":{"report_sha256":same_code["report_sha256"],
            "independent_config_and_actions":true,"effect_raw":"-7609851","interaction_raw":"0"},
        "config_controls":configuration_controls,"action_controls":deposit_controls,"message_controls":message_controls,
        "limitations":["Constructed code only; no arbitrary candidate or upstream release qualification.",
            "Zero referral, ungated action only; no authority possession or live signability.",
            "Retained schema-1 Clock epoch 0 and runtime; no exact validator equivalence.",
            "Fresh VMs and pool-only handoffs; no installed upgrade, deployment or rollout ordering.",
            "Aliased token roles measured once; no independent referral split."]});
    receipt["qualification_sha256"] = canonical::digest(&receipt).unwrap().into();
    receipt
}
// Fixture-specific offline check: seal, supplied ELF identity, preservation,
// independent ledgers, and exact repeated VM evidence. Never a production API.
fn verify_receipt(h: &Harness, receipt: &Value, candidate: &[u8], repeated: &Value) -> Result<()> {
    ensure!(
        replay::hash_bytes(candidate) == receipt["candidate"]["sha256"].as_str().unwrap(),
        "candidate identity"
    );
    let mut unsealed = receipt.clone();
    let seal = unsealed
        .as_object_mut()
        .unwrap()
        .remove("qualification_sha256")
        .unwrap();
    ensure!(seal == canonical::digest(&unsealed)?, "qualification seal");
    verify_config(&receipt["K2"])?;
    for case in receipt["config_controls"].as_array().unwrap() {
        verify_config(case)?;
    }
    for name in ["R10", "R11"] {
        let record: ReplayRecord =
            serde_json::from_value(receipt["cells"][name]["pre_record"].clone())?;
        let x: ExecutionResult =
            serde_json::from_value(receipt["cells"][name]["execution"].clone())?;
        ensure!(
            s::reconcile(&record, &x)? == receipt["cells"][name]["reconciliation"],
            "action evidence"
        );
    }
    ensure!(
        effects(&receipt["cells"]) == receipt["effects"],
        "effect evidence"
    );
    ensure!(
        receipt["retained_input"] == json!(h.input),
        "historical evidence"
    );
    ensure!(receipt == repeated, "offline repeated VM evidence differs");
    Ok(())
}

fn compact_receipt(receipt: &Value, full: &str) -> Value {
    let cells = receipt["cells"].as_object().unwrap().iter().map(|(name, cell)| {
        (name.clone(), json!({"action_input_id":cell["action_input_id"],"program_sha256":cell["program_sha256"],
            "execution_sha256":cell["execution_sha256"],"success":cell["execution"]["success"],
            "compute_units":cell["execution"]["compute_units"],"reconciliation":cell["reconciliation"]}))
    }).collect::<serde_json::Map<_, _>>();
    let configs = ["K1", "K2"].into_iter().map(|name| {
        let c = &receipt[name];
        let x: ProbeTransactionExecution = serde_json::from_value(c["execution"].clone()).unwrap();
        let pool = x.post_accounts.values().find(|a| a.owner == spl_stake_pool::id().to_string() && a.data.first() == Some(&1)).unwrap();
        (name.to_string(), json!({"config_input_id":c["config_input_id"],"execution_sha256":c["execution_sha256"],
            "success":x.success,"compute_units":x.compute_units.to_string(),"transaction_fee_lamports":x.transaction_fee_lamports.to_string(),
            "post_pool_data_sha256":replay::hash_bytes(&pool.data),"full_preservation_verified":true,
            "manager_assumption":c["manager_assumption"],"fee_payer":c["fee_payer"]}))
    }).collect::<serde_json::Map<_, _>>();
    let controls = ["config_controls", "action_controls", "message_controls"].into_iter().map(|group| {
        let values = receipt[group].as_array().unwrap().iter().map(|c| {
            let outcome = if group == "action_controls" { &c["outcome"] } else { c };
            json!({"mode":c["mode"],"success":outcome["execution"]["success"],"error":outcome["execution"]["error"],
                "reconciliation_error":outcome["reconciliation_error"],"evidence_sha256":canonical::digest(c).unwrap()})
        }).collect::<Vec<_>>();
        (group.to_string(), json!(values))
    }).collect::<serde_json::Map<_, _>>();
    json!({"schema":"eplyx-stake-pool-config-candidate-qualification-summary-v1",
        "qualification_sha256":receipt["qualification_sha256"],"origin":receipt["origin"],"scope":receipt["scope"],
        "build":receipt["build"],"runtime":receipt["runtime"],"harness_sha256":receipt["harness_sha256"],
        "candidate":{"sha256":receipt["candidate"]["sha256"],"len":receipt["candidate"]["len"]},
        "historical_record":{"id":receipt["retained_input"]["record"]["id"],"record_sha256":receipt["retained_input"]["record_sha256"],
            "pre_state_hash":receipt["retained_input"]["record"]["pre_state_hash"],"source_bundle_sha256":receipt["retained_input"]["source_bundle_sha256"],
            "clock":receipt["retained_input"]["record"]["clock"],"programs":receipt["retained_input"]["record"]["dependencies"]},
        "upgrade_change_spec_id":receipt["upgrade_change_spec_id"],"parameter_change_spec_id":receipt["parameter_change_spec_id"],
        "configurations":configs,"cells":cells,"effects":receipt["effects"],"same_code_control":receipt["same_code_control"],
        "controls":controls,"limitations":receipt["limitations"],
        "full_evidence":{"sha256":replay::hash_bytes(full.as_bytes()),"len":full.len(),"regenerate":"EPLYX_STAKE_CONFIG_QUALIFICATION_OUT=/private/tmp/eplyx-step10b-qualification cargo test -p eplyx-engine --test stake_pool_config_candidate --offline -- --nocapture"}})
}

#[test]
fn constructed_candidate_four_cells_controls_and_offline_receipt() {
    let h = Harness::new();
    let first = qualify(&h);
    let second = qualify(&h);
    verify_receipt(&h, &first, &h.candidate.bytes, &second).unwrap();
    let mut changed_candidate = h.candidate.bytes.clone();
    changed_candidate[100] ^= 1;
    assert!(verify_receipt(&h, &first, &changed_candidate, &second).is_err());
    for mode in ["config_output", "verified_pool", "action_account", "effect"] {
        let mut tampered = first.clone();
        let field = match mode {
            "config_output" => {
                let address = first["K2"]["pool_address"].as_str().unwrap();
                &mut tampered["K2"]["execution"]["post_accounts"][address]["lamports"]
            }
            "verified_pool" => &mut tampered["verified_K2_pool"]["lamports"],
            "action_account" => {
                &mut tampered["cells"]["R11"]["execution"]["accounts"]["destination-pool-token"]
                    ["lamports"]
            }
            _ => &mut tampered["effects"]["interaction_recipient_raw"],
        };
        *field = if mode == "effect" {
            json!("1")
        } else {
            json!((field.as_str().unwrap().parse::<u64>().unwrap() + 1).to_string())
        };
        assert!(
            verify_receipt(&h, &tampered, &h.candidate.bytes, &second).is_err(),
            "{mode}"
        );
        tampered
            .as_object_mut()
            .unwrap()
            .remove("qualification_sha256");
        tampered["qualification_sha256"] = canonical::digest(&tampered).unwrap().into();
        assert!(
            verify_receipt(&h, &tampered, &h.candidate.bytes, &second).is_err(),
            "resealed {mode}"
        );
    }
    if let Some(out) = std::env::var_os("EPLYX_STAKE_CONFIG_QUALIFICATION_OUT") {
        std::fs::create_dir_all(&out).unwrap();
        let full = canonical::document(&first).unwrap();
        std::fs::write(
            std::path::PathBuf::from(&out).join("qualification.json"),
            &full,
        )
        .unwrap();
        std::fs::write(
            std::path::PathBuf::from(out).join("qualification-summary.json"),
            canonical::document(&compact_receipt(&first, &full)).unwrap(),
        )
        .unwrap();
    }
    println!(
        "four-cell effects={}; receipt={}",
        first["effects"], first["qualification_sha256"]
    );
}
