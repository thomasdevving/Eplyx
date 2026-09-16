//! Phase 8: CPI-aware mainnet replay.
//!
//! Two harnesses, because two different things need proving and they need
//! different evidence.
//!
//! The acquisition tests drive a deterministic stand-in for a slot-addressable
//! archive, a transaction RPC and a block source. They prove what the pipeline
//! refuses: a dependency resolved at the wrong slot, an account whose boundary
//! another transaction in the same slot made ambiguous, a manifest that does not
//! cover something that executes, a binary whose bytes do not match the hash the
//! record pins.
//!
//! The execution tests run real SBF bytecode through a real cross-program
//! invocation into the real SPL Token program, using two builds of one source:
//! one with the share-calculation defect and one without. They prove that the
//! difference survives all the way to an economic finding and a blocked gate.
//! They do not prove anything about the real stake pool - the binary mainnet
//! actually deployed is what the Phase 8 demo runs, and the committed record
//! here is checked for self-consistency rather than executed, because a 1 MB
//! mainnet binary does not belong in a repository.

use anyhow::Result;
use base64::Engine as _;
use eplyx_engine::{
    dependencies::{
        self, DependencyDiscovery, DependencyManifest, ProgramDependency, ProgramSource,
    },
    executor::ProgramVersion,
    historical::{HistoricalStateProvider, ProtocolArchiveProvider},
    ingest::{rpc::RpcProvider, transactions::CpiFrame},
    protocol,
    replay::*,
    screening,
    types::{AccountMetaSpec, AccountSnapshot, InstructionSpec, NamedAccount},
    versions::ProgramLoader,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const STAKE_POOL: &str = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";
const SYSTEM: &str = "11111111111111111111111111111111";
const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const STAKE: &str = "Stake11111111111111111111111111111111111111";
const NATIVE_LOADER: &str = "NativeLoader1111111111111111111111111111111";
const UPGRADEABLE_LOADER: &str = "BPFLoaderUpgradeab1e11111111111111111111111";
const COMMITTED_TOKEN_SHA256: &str =
    "8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697";

const SLOT: u64 = 429_878_778;
const DEPOSIT: u64 = 423_000_000;
const POOL_TOTAL_BEFORE: u64 = 311_850_055_457_947;
const POOL_SUPPLY_BEFORE: u64 = 291_875_877_314_830;
const MINTED: u64 = 395_906_603;
const DESTINATION_BEFORE: u64 = 500_000_000;
const RESERVE_BEFORE: u64 = 50_000_000_000;
const DEPOSITOR_BEFORE: u64 = 5_000_000_000;
const FEE: u64 = 5_000;
const DECIMALS: u8 = 9;

// ---------------------------------------------------------------------------
// Shared builders
// ---------------------------------------------------------------------------

fn address(byte: u8) -> String {
    bs58::encode([byte; 32]).into_string()
}

fn base58_bytes(text: &str) -> Vec<u8> {
    bs58::decode(text).into_vec().expect("valid base58")
}

fn signature() -> String {
    bs58::encode([9_u8; 64]).into_string()
}

fn depositor() -> String {
    address(1)
}
fn destination() -> String {
    address(2)
}
fn manager_fee() -> String {
    address(3)
}
fn pool() -> String {
    address(4)
}
fn reserve() -> String {
    address(5)
}
fn mint() -> String {
    address(6)
}
fn withdraw_authority() -> String {
    address(7)
}

/// The fixed-offset prefix of `StakePool`, followed by the variable-length tail
/// the layout declares. Built field by field rather than pasted as a blob so the
/// test says what it is asserting about the layout.
fn pool_account(total_lamports: u64, pool_token_supply: u64) -> Vec<u8> {
    let mut data = vec![1_u8]; // AccountType::StakePool
    for _ in 0..3 {
        data.extend_from_slice(&[0_u8; 32]); // manager, staker, stake deposit authority
    }
    data.push(255); // stake withdraw bump seed
    data.extend_from_slice(&[8_u8; 32]); // validator list
    data.extend_from_slice(&base58_bytes(&reserve()));
    data.extend_from_slice(&base58_bytes(&mint()));
    data.extend_from_slice(&base58_bytes(&manager_fee()));
    data.extend_from_slice(&base58_bytes(TOKEN));
    data.extend_from_slice(&total_lamports.to_le_bytes());
    data.extend_from_slice(&pool_token_supply.to_le_bytes());
    data.extend_from_slice(&1035_u64.to_le_bytes()); // last update epoch
    data.extend_from_slice(&[0_u8; 48]); // lockup
    data.extend_from_slice(&100_u64.to_le_bytes()); // epoch fee denominator
    data.extend_from_slice(&4_u64.to_le_bytes()); // epoch fee numerator
    data.push(0); // next epoch fee: None
    data.push(0); // preferred deposit validator: None
    data.push(0); // preferred withdraw validator: None
    data.extend_from_slice(&[0_u8; 16]); // stake deposit fee
    data.extend_from_slice(&1000_u64.to_le_bytes()); // stake withdrawal denominator
    data.extend_from_slice(&1_u64.to_le_bytes()); // stake withdrawal numerator
    data.push(0); // next stake withdrawal fee: None
    data.push(0); // stake referral fee
    data.push(0); // sol deposit authority: None
    data.extend_from_slice(&[0_u8; 16]); // sol deposit fee: no fee
    data.push(0); // sol referral fee
    data.push(0); // sol withdraw authority: None
    data.extend_from_slice(&1000_u64.to_le_bytes()); // sol withdrawal denominator
    data.extend_from_slice(&1_u64.to_le_bytes()); // sol withdrawal numerator
    data.push(0); // next sol withdrawal fee: None
    data.extend_from_slice(&pool_token_supply.to_le_bytes()); // last epoch supply
    data.extend_from_slice(&total_lamports.to_le_bytes()); // last epoch lamports
    data.extend_from_slice(&[0_u8; 176]); // stale bytes, as a real pool carries
    data
}

fn token_account(owner: &str, amount: u64) -> Vec<u8> {
    let mut data = vec![0_u8; 165];
    data[..32].copy_from_slice(&base58_bytes(&mint()));
    data[32..64].copy_from_slice(&base58_bytes(owner));
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1; // initialized
    data
}

fn mint_account(authority: &str, supply: u64) -> Vec<u8> {
    let mut data = vec![0_u8; 82];
    data[..4].copy_from_slice(&1_u32.to_le_bytes()); // mint authority: Some
    data[4..36].copy_from_slice(&base58_bytes(authority));
    data[36..44].copy_from_slice(&supply.to_le_bytes());
    data[44] = DECIMALS;
    data[45] = 1; // initialized
    data
}

/// `StakeStateV2`. Only its length and owner matter to this path: a SOL deposit
/// credits the reserve's lamports and never invokes the stake program.
fn stake_account() -> Vec<u8> {
    vec![0_u8; 200]
}

fn snapshot(lamports: u64, owner: &str, data: Vec<u8>) -> AccountSnapshot {
    AccountSnapshot {
        lamports,
        owner: owner.into(),
        data,
        executable: false,
        rent_epoch: 0,
    }
}

fn meta(pubkey: &str, signer: bool, writable: bool) -> AccountMetaSpec {
    AccountMetaSpec {
        address: pubkey.into(),
        is_signer: signer,
        is_writable: writable,
    }
}

/// Message keys, in the privilege order a legacy message requires.
fn message_keys() -> Vec<AccountMetaSpec> {
    vec![
        meta(&depositor(), true, true),
        meta(&destination(), false, true),
        meta(&manager_fee(), false, true),
        meta(&pool(), false, true),
        meta(&reserve(), false, true),
        meta(&mint(), false, true),
        meta(&withdraw_authority(), false, false),
        meta(SYSTEM, false, false),
        meta(TOKEN, false, false),
        meta(STAKE_POOL, false, false),
    ]
}

fn deposit_instruction() -> InstructionSpec {
    let keys = message_keys();
    let pick = |index: usize| keys[index].clone();
    InstructionSpec {
        program: STAKE_POOL.into(),
        accounts: vec![
            pick(3), // stake pool
            pick(6), // withdraw authority
            pick(4), // reserve
            pick(0), // depositor
            pick(1), // destination pool token
            pick(2), // manager fee
            pick(1), // referral fee, the depositor's own account
            pick(5), // pool mint
            pick(7), // system program
            pick(8), // token program
        ],
        data: std::iter::once(14_u8)
            .chain(DEPOSIT.to_le_bytes())
            .collect(),
    }
}

fn inner_instructions() -> Vec<InstructionSpec> {
    let keys = message_keys();
    vec![
        InstructionSpec {
            program: SYSTEM.into(),
            accounts: vec![keys[0].clone(), keys[4].clone()],
            data: 2_u32
                .to_le_bytes()
                .iter()
                .copied()
                .chain(DEPOSIT.to_le_bytes())
                .collect(),
        },
        InstructionSpec {
            program: TOKEN.into(),
            accounts: vec![keys[5].clone(), keys[1].clone(), keys[6].clone()],
            data: std::iter::once(7_u8).chain(MINTED.to_le_bytes()).collect(),
        },
    ]
}

fn cpi_frames() -> Vec<CpiFrame> {
    inner_instructions()
        .iter()
        .map(|instruction| {
            CpiFrame::new(
                0,
                2,
                instruction.program.clone(),
                instruction.accounts.len(),
                &instruction.data,
            )
        })
        .collect()
}

fn pre_balances() -> Vec<u64> {
    vec![
        DEPOSITOR_BEFORE,
        2_039_280,
        2_039_280,
        5_143_440,
        RESERVE_BEFORE,
        1_461_600,
        0,
        1,
        1,
        153_141_440,
    ]
}

fn post_balances() -> Vec<u64> {
    let mut balances = pre_balances();
    balances[0] = DEPOSITOR_BEFORE - DEPOSIT - FEE;
    balances[4] = RESERVE_BEFORE + DEPOSIT;
    balances
}

fn token_balance(index: usize, amount: u64) -> Value {
    json!({
        "accountIndex": index,
        "mint": mint(),
        "programId": TOKEN,
        "uiTokenAmount": {"amount": amount.to_string(), "decimals": DECIMALS},
    })
}

fn raw_transaction() -> Value {
    let keys = message_keys();
    let index = |address: &str| keys.iter().position(|k| k.address == address).unwrap();
    let compiled = |instruction: &InstructionSpec| {
        json!({
            "programIdIndex": index(&instruction.program),
            "accounts": instruction.accounts.iter().map(|a| index(&a.address)).collect::<Vec<_>>(),
            "data": bs58::encode(&instruction.data).into_string(),
        })
    };
    let deposit = deposit_instruction();
    let inner: Vec<Value> = inner_instructions()
        .iter()
        .map(|instruction| {
            let mut value = compiled(instruction);
            value["stackHeight"] = json!(2);
            value
        })
        .collect();
    json!({
        "slot": SLOT,
        "blockTime": 1_782_800_000_i64,
        "transaction": {
            "signatures": [signature()],
            "message": {
                "header": {
                    "numRequiredSignatures": 1,
                    "numReadonlySignedAccounts": 0,
                    "numReadonlyUnsignedAccounts": 4,
                },
                "accountKeys": keys.iter().map(|k| k.address.clone()).collect::<Vec<_>>(),
                "recentBlockhash": bs58::encode([3_u8; 32]).into_string(),
                "instructions": [compiled(&deposit)],
            },
        },
        "meta": {
            "err": null,
            "fee": FEE,
            "computeUnitsConsumed": 12_368,
            "preBalances": pre_balances(),
            "postBalances": post_balances(),
            "preTokenBalances": [token_balance(1, DESTINATION_BEFORE), token_balance(2, 0)],
            "postTokenBalances": [
                token_balance(1, DESTINATION_BEFORE + MINTED),
                token_balance(2, 0),
            ],
            "innerInstructions": [{"index": 0, "instructions": inner}],
            "logMessages": [
                format!("Program {STAKE_POOL} invoke [1]"),
                "Program log: Instruction: DepositSol".to_string(),
                format!("Program {SYSTEM} invoke [2]"),
                format!("Program {TOKEN} invoke [2]"),
                format!("Program {STAKE_POOL} success"),
            ],
        },
    })
}

fn transaction() -> eplyx_engine::ingest::transactions::HistoricalTransaction {
    eplyx_engine::ingest::transactions::normalize(&raw_transaction()).expect("normalizes")
}

// ---------------------------------------------------------------------------
// Archive harness
// ---------------------------------------------------------------------------

fn encode(data: &[u8]) -> String {
    base64::prelude::BASE64_STANDARD.encode(data)
}

fn account_json(lamports: u64, owner: &str, data: &[u8], executable: bool) -> Value {
    json!({
        "data": [encode(data), "base64"],
        "owner": owner,
        "lamports": lamports,
        "executable": executable,
        "rentEpoch": 0,
        "space": data.len(),
    })
}

/// A program deployed under the upgradeable loader, as two accounts.
fn upgradeable_program(programdata: &str, deploy_slot: u64, elf: &[u8]) -> (Value, Value) {
    let mut program = 2_u32.to_le_bytes().to_vec();
    program.extend_from_slice(&base58_bytes(programdata));
    let mut data = vec![0_u8; 45];
    data[..4].copy_from_slice(&3_u32.to_le_bytes());
    data[4..12].copy_from_slice(&deploy_slot.to_le_bytes());
    data.extend_from_slice(elf);
    (
        account_json(1, UPGRADEABLE_LOADER, &program, true),
        account_json(1, UPGRADEABLE_LOADER, &data, false),
    )
}

fn elf(tag: u8) -> Vec<u8> {
    let mut bytes = b"\x7fELF".to_vec();
    bytes.extend_from_slice(&[tag; 512]);
    bytes
}

struct Archive {
    accounts: BTreeMap<(String, u64), Value>,
    block: Value,
}

/// `getBlock` in the compact `transactionDetails=accounts` form the screen uses.
fn block(extra_writer_of: Option<&str>) -> Value {
    let entry = |signature: &str, writable: &[String]| {
        json!({
            "transaction": {
                "signatures": [signature],
                "accountKeys": writable
                    .iter()
                    .map(|key| json!({"pubkey": key, "writable": true, "signer": false}))
                    .collect::<Vec<_>>(),
            },
            "meta": {"err": null},
        })
    };
    let mut transactions = vec![entry("unrelated-before", &[address(200)])];
    if let Some(account) = extra_writer_of {
        transactions.push(entry("same-slot-writer", &[account.to_string()]));
    }
    transactions.push(entry(
        &signature(),
        &message_keys()
            .iter()
            .filter(|key| key.is_writable)
            .map(|key| key.address.clone())
            .collect::<Vec<_>>(),
    ));
    transactions.push(entry("unrelated-after", &[address(201)]));
    json!({ "transactions": transactions })
}

impl Archive {
    /// `token_deploy_slot` is what makes the version-selection test meaningful:
    /// the archive answers differently at `SLOT - 1` than it does later.
    fn new(token_deploy_slot: u64, extra_writer_of: Option<&str>) -> Self {
        let mut accounts = BTreeMap::new();
        let mut put = |address: &str, slot: u64, value: Value| {
            accounts.insert((address.to_string(), slot), value);
        };
        for slot in [SLOT - 1, SLOT] {
            put(SYSTEM, slot, account_json(1, NATIVE_LOADER, &[0; 21], true));
            let (program, programdata) =
                upgradeable_program(&address(100), 370_300_186, &elf(0xAA));
            put(STAKE_POOL, slot, program);
            put(&address(100), slot, programdata);
            let (program, programdata) =
                upgradeable_program(&address(101), token_deploy_slot, &elf(0xBB));
            put(TOKEN, slot, program);
            put(&address(101), slot, programdata);
            put(
                &mint(),
                slot,
                account_json(
                    1_461_600,
                    TOKEN,
                    &mint_account(
                        &withdraw_authority(),
                        if slot == SLOT - 1 {
                            POOL_SUPPLY_BEFORE
                        } else {
                            POOL_SUPPLY_BEFORE + MINTED
                        },
                    ),
                    false,
                ),
            );
            put(
                &manager_fee(),
                slot,
                account_json(2_039_280, TOKEN, &token_account(&address(9), 0), false),
            );
        }
        let before = SLOT - 1;
        put(
            &depositor(),
            before,
            account_json(DEPOSITOR_BEFORE, SYSTEM, &[], false),
        );
        put(
            &depositor(),
            SLOT,
            account_json(DEPOSITOR_BEFORE - DEPOSIT - FEE, SYSTEM, &[], false),
        );
        put(
            &destination(),
            before,
            account_json(
                2_039_280,
                TOKEN,
                &token_account(&depositor(), DESTINATION_BEFORE),
                false,
            ),
        );
        put(
            &destination(),
            SLOT,
            account_json(
                2_039_280,
                TOKEN,
                &token_account(&depositor(), DESTINATION_BEFORE + MINTED),
                false,
            ),
        );
        put(
            &pool(),
            before,
            account_json(
                5_143_440,
                STAKE_POOL,
                &pool_account(POOL_TOTAL_BEFORE, POOL_SUPPLY_BEFORE),
                false,
            ),
        );
        put(
            &pool(),
            SLOT,
            account_json(
                5_143_440,
                STAKE_POOL,
                &pool_account(POOL_TOTAL_BEFORE + DEPOSIT, POOL_SUPPLY_BEFORE + MINTED),
                false,
            ),
        );
        put(
            &reserve(),
            before,
            account_json(RESERVE_BEFORE, STAKE, &stake_account(), false),
        );
        put(
            &reserve(),
            SLOT,
            account_json(RESERVE_BEFORE + DEPOSIT, STAKE, &stake_account(), false),
        );
        Self {
            accounts,
            block: block(extra_writer_of),
        }
    }
}

impl RpcProvider for Archive {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "getGenesisHash" => Ok(json!("test-genesis")),
            "getTransaction" => Ok(raw_transaction()),
            "getBlock" => Ok(self.block.clone()),
            "getAccountInfo" => {
                let address = params[0].as_str().expect("address").to_string();
                let slot = params[1]["slot"].as_u64().expect("exact slot selector");
                let Some(account) = self.accounts.get(&(address, slot)) else {
                    return Ok(json!({"context": {"slot": slot}, "value": null}));
                };
                let mut account = account.clone();
                if let Some(slice) = params[1].get("dataSlice") {
                    let offset = slice["offset"].as_u64().unwrap_or(0) as usize;
                    let length = slice["length"].as_u64().unwrap_or(0) as usize;
                    let full = base64::prelude::BASE64_STANDARD
                        .decode(account["data"][0].as_str().expect("data"))?;
                    let end = offset.saturating_add(length).min(full.len());
                    account["data"] =
                        json!([encode(full.get(offset..end).unwrap_or_default()), "base64"]);
                }
                Ok(json!({"context": {"slot": slot}, "value": account}))
            }
            other => anyhow::bail!("unexpected RPC {other}"),
        }
    }
}

fn acquire(archive: &Archive) -> Result<eplyx_engine::historical::HistoricalAcquisition> {
    ProtocolArchiveProvider {
        transaction_rpc: archive,
        account_archive_rpc: archive,
        block_rpc: Some(archive),
        program_id: STAKE_POOL,
    }
    .acquire_exact(&signature())
}

// ---------------------------------------------------------------------------
// 1. CPI dependency extraction
// ---------------------------------------------------------------------------

/// The SPL Token program appears in no top-level instruction of this
/// transaction. Top-level metas alone would load a replay environment missing
/// the program that mints the depositor's shares.
#[test]
fn dependency_extraction_finds_programs_reached_only_through_cpi() {
    let transaction = transaction();
    let top_level: BTreeSet<&str> = transaction
        .instructions
        .iter()
        .map(|instruction| instruction.program.as_str())
        .collect();
    assert!(!top_level.contains(TOKEN), "premise: {top_level:?}");

    let adapter = protocol::adapter_for(STAKE_POOL).expect("adapter");
    let found = dependencies::discover(&transaction, Some(adapter), STAKE_POOL);
    let token = found
        .iter()
        .find(|(id, _)| id == TOKEN)
        .expect("token program discovered");
    assert!(token.1.contains(&DependencyDiscovery::InnerInstruction));
    assert!(token.1.contains(&DependencyDiscovery::ExecutionLog));
    assert!(token.1.contains(&DependencyDiscovery::AdapterDeclared));
}

/// Normalization has to keep the depth and the owning instruction of every CPI
/// frame. The flattened list Phase 4 recorded cannot express either, and the
/// fidelity gate compares against what is kept here.
#[test]
fn cpi_frames_survive_transaction_normalization() {
    let transaction = transaction();
    assert_eq!(transaction.inner_instruction_frames, cpi_frames());
    assert_eq!(
        transaction.inner_instruction_frames.len(),
        transaction.inner_instructions.len()
    );
    assert_eq!(
        transaction.inner_instruction_frames[1].discriminant,
        Some(7)
    );
    assert_eq!(transaction.inner_instruction_frames[1].account_count, 3);
}

// ---------------------------------------------------------------------------
// 2. Dependency binary resolution
// ---------------------------------------------------------------------------

#[test]
fn every_executing_program_is_pinned_to_a_binary_or_to_the_runtime() {
    let acquired = acquire(&Archive::new(419_472_000, None)).expect("acquisition");
    let manifest = &acquired.record.dependencies;
    assert!(manifest.unsupported().is_empty());

    let system = manifest.get(SYSTEM).expect("system pinned");
    assert_eq!(system.source, ProgramSource::Builtin);
    assert!(
        system.binary_sha256.is_none(),
        "nothing is substituted for a runtime program"
    );

    let token = manifest.get(TOKEN).expect("token pinned");
    assert_eq!(token.source, ProgramSource::HistoricalMainnet);
    assert_eq!(token.deployed_slot, Some(419_472_000));
    assert_eq!(token.loader, Some(ProgramLoader::Upgradeable));
    assert_eq!(token.observed_slot, Some(SLOT - 1));
    assert_eq!(
        token.binary_sha256.as_deref(),
        Some(hash_bytes(&elf(0xBB)).as_str())
    );

    // The bytes come back beside the record, keyed by program, and the program
    // under test is not among them: it is the artefact the comparison varies.
    assert_eq!(acquired.dependency_binaries.get(TOKEN), Some(&elf(0xBB)));
    assert!(!acquired.dependency_binaries.contains_key(STAKE_POOL));
    assert_eq!(
        manifest.get(STAKE_POOL).unwrap().deployed_slot,
        Some(370_300_186)
    );
}

// ---------------------------------------------------------------------------
// 3. Historical version selection
// ---------------------------------------------------------------------------

/// A dependency is read at the slot the transaction ran at, not at the present.
/// Two archives that differ only in when the token program was deployed have to
/// produce two different records.
#[test]
fn dependency_versions_are_selected_at_the_transactions_slot() {
    let older = acquire(&Archive::new(400_000_000, None)).expect("acquisition");
    let newer = acquire(&Archive::new(419_472_000, None)).expect("acquisition");
    assert_eq!(
        older.record.dependencies.get(TOKEN).unwrap().deployed_slot,
        Some(400_000_000)
    );
    assert_eq!(
        newer.record.dependencies.get(TOKEN).unwrap().deployed_slot,
        Some(419_472_000)
    );
    assert!(older
        .record
        .assumptions
        .iter()
        .any(|line| line.contains("not from today's")));
}

// ---------------------------------------------------------------------------
// 4. CPI account-state discovery
// ---------------------------------------------------------------------------

#[test]
fn account_discovery_records_how_each_account_was_found() {
    let acquired = acquire(&Archive::new(419_472_000, None)).expect("acquisition");
    let record = &acquired.record;
    let by_address: BTreeMap<&str, &AccountAcquisition> = record
        .acquisitions
        .iter()
        .map(|entry| (entry.address.as_str(), entry))
        .collect();

    for address in [pool(), reserve(), destination(), manager_fee(), mint()] {
        let entry = by_address
            .get(address.as_str())
            .unwrap_or_else(|| panic!("{address} acquired"));
        assert_eq!(entry.source, AccountStateSource::HistoricalArchive);
        assert_eq!(entry.context_slot, SLOT - 1);
        assert!(entry.method.contains("exact slot"));
        assert!(
            entry
                .discovered_by
                .contains(&AccountDiscovery::AdapterDependency),
            "{address}: {:?}",
            entry.discovered_by
        );
    }
    // The reserve and the mint are also named by the inner instructions, which
    // is the route a top-level-only view would not have.
    for address in [reserve(), mint()] {
        assert!(by_address[address.as_str()]
            .discovered_by
            .contains(&AccountDiscovery::InnerInstruction));
    }
    // The withdraw authority holds nothing at either boundary. Recording it as
    // absent is more faithful than inventing a snapshot for it.
    let authority = by_address[withdraw_authority().as_str()];
    assert_eq!(authority.source, AccountStateSource::AbsentAtBothBoundaries);
    assert!(record
        .accounts
        .iter()
        .all(|named| named.address != withdraw_authority()));
}

// ---------------------------------------------------------------------------
// 5. Same-slot conflict rejection
// ---------------------------------------------------------------------------

/// The rejection has to name the account and the conflicting transaction. A
/// boundary failure that only says "the numbers disagree" is not actionable.
#[test]
fn a_same_slot_writer_of_a_cpi_account_rejects_the_candidate() {
    let error = acquire(&Archive::new(419_472_000, Some(&reserve())))
        .expect_err("acquisition refused")
        .to_string();
    assert!(error.contains(&reserve()), "{error}");
    assert!(
        error.contains("same-slot") || error.contains("same slot"),
        "{error}"
    );
    assert!(error.contains("earlier in the slot"), "{error}");
}

#[test]
fn the_pool_state_account_is_screened_like_any_other() {
    let error = acquire(&Archive::new(419_472_000, Some(&pool())))
        .expect_err("acquisition refused")
        .to_string();
    assert!(error.contains(&pool()), "{error}");
}

/// A clean screen is recorded as evidence, not merely passed through.
#[test]
fn a_clean_screen_is_recorded_on_the_record() {
    let acquired = acquire(&Archive::new(419_472_000, None)).expect("acquisition");
    let screening = acquired
        .record
        .slot_screening
        .as_ref()
        .expect("screening recorded");
    assert!(screening.is_clean());
    assert_eq!(screening.slot, SLOT);
    assert_eq!(screening.target_index, 1);
    assert_eq!(screening.transactions_in_slot, 3);
    // Screening covers exactly the accounts whose boundary rests on the
    // archive. An account reconstructed from this transaction's own balance
    // metadata is deliberately excluded: its boundary is already exact for this
    // transaction, so another writer in the same slot cannot spoil it.
    use eplyx_engine::replay::AccountStateSource;
    let archive_backed = acquired
        .record
        .acquisitions
        .iter()
        .filter(|a| a.source == AccountStateSource::HistoricalArchive)
        .count();
    let reconstructed: Vec<&str> = acquired
        .record
        .acquisitions
        .iter()
        .filter(|a| a.source == AccountStateSource::TransactionBalanceMetadata)
        .map(|a| a.address.as_str())
        .collect();
    assert_eq!(screening.required_accounts.len(), archive_backed);
    assert_eq!(
        archive_backed + reconstructed.len(),
        acquired.record.accounts.len(),
        "every acquired account is either archive-backed or metadata-reconstructed"
    );
    for address in reconstructed {
        assert!(
            !screening.required_accounts.iter().any(|a| a == address),
            "{address} is reconstructed and must not be screened"
        );
    }
}

/// A CPI-admitting record without screening evidence does not validate: the
/// weaker boundary proof that sufficed before CPI is not enough here.
#[test]
fn a_cpi_record_without_screening_is_refused() {
    let mut record = acquire(&Archive::new(419_472_000, None))
        .expect("acquisition")
        .record;
    record.slot_screening = None;
    let error = record.validate().expect_err("refused").to_string();
    assert!(error.contains("screening"), "{error}");
}

// ---------------------------------------------------------------------------
// 6. CPI-aware state hashing
// ---------------------------------------------------------------------------

#[test]
fn the_pre_state_hash_covers_every_dependent_account_in_canonical_order() {
    let record = acquire(&Archive::new(419_472_000, None))
        .expect("acquisition")
        .record;
    assert_eq!(state_hash(&record.accounts).unwrap(), record.pre_state_hash);

    // Order of presentation must not move the hash; content must.
    let mut shuffled = record.accounts.clone();
    shuffled.reverse();
    assert_eq!(state_hash(&shuffled).unwrap(), record.pre_state_hash);

    for label in [
        "stake-pool",
        "reserve-stake",
        "destination-pool-token",
        "pool-mint",
    ] {
        let mut altered = record.accounts.clone();
        let account = altered
            .iter_mut()
            .find(|named| named.label == label)
            .unwrap_or_else(|| panic!("{label} is watched"));
        account.account.lamports += 1;
        assert_ne!(
            state_hash(&altered).unwrap(),
            record.pre_state_hash,
            "{label} does not reach the hash"
        );
    }
}

// ---------------------------------------------------------------------------
// Local CPI execution harness
// ---------------------------------------------------------------------------

fn artifact(name: &str) -> ProgramVersion {
    let path = eplyx_engine::repo_root().join("artifacts").join(name);
    ProgramVersion::from_file(name, &path)
        .unwrap_or_else(|e| panic!("{e:#}\nrun `./scripts/build-stake-pool-candidate.sh` first"))
}

fn committed_dependencies() -> std::path::PathBuf {
    eplyx_engine::repo_root().join("fixtures/dependencies")
}

fn committed_token_bundle(record: &ReplayRecord) -> DependencyBundle {
    load_dependencies(std::slice::from_ref(record), &committed_dependencies())
        .expect("committed SPL Token binary loads and matches its pinned hash")
}

/// A record whose accounts and message are the local scenario, and whose
/// dependency manifest pins the real SPL Token binary committed to this
/// repository.
///
/// The pool's withdraw authority is a real program address derived from the
/// pool, because the mint authority has to be a key the program can sign for.
fn local_record(v1: &ProgramVersion) -> ReplayRecord {
    use solana_address::Address;
    let program: Address = STAKE_POOL.parse().unwrap();
    let pool_key: Address = pool().parse().unwrap();
    let (authority, bump) =
        Address::find_program_address(&[pool_key.as_ref(), b"withdraw"], &program);
    let authority = authority.to_string();

    let mut pool_data = pool_account(POOL_TOTAL_BEFORE, POOL_SUPPLY_BEFORE);
    pool_data[97] = bump;

    let keys: Vec<AccountMetaSpec> = message_keys()
        .into_iter()
        .map(|mut key| {
            if key.address == withdraw_authority() {
                key.address = authority.clone();
            }
            key
        })
        .collect();
    let retarget = |instruction: InstructionSpec| InstructionSpec {
        accounts: instruction
            .accounts
            .into_iter()
            .map(|mut account| {
                if account.address == withdraw_authority() {
                    account.address = authority.clone();
                }
                account
            })
            .collect(),
        ..instruction
    };

    let accounts = vec![
        NamedAccount {
            label: "depositor".into(),
            address: depositor(),
            account: snapshot(DEPOSITOR_BEFORE, SYSTEM, vec![]),
        },
        NamedAccount {
            label: "destination-pool-token".into(),
            address: destination(),
            account: snapshot(
                2_039_280,
                TOKEN,
                token_account(&depositor(), DESTINATION_BEFORE),
            ),
        },
        NamedAccount {
            label: "manager-fee".into(),
            address: manager_fee(),
            account: snapshot(2_039_280, TOKEN, token_account(&address(9), 0)),
        },
        NamedAccount {
            label: "stake-pool".into(),
            address: pool(),
            account: snapshot(5_143_440, STAKE_POOL, pool_data),
        },
        NamedAccount {
            label: "reserve-stake".into(),
            address: reserve(),
            account: snapshot(RESERVE_BEFORE, STAKE, stake_account()),
        },
        NamedAccount {
            label: "pool-mint".into(),
            address: mint(),
            account: snapshot(
                1_461_600,
                TOKEN,
                mint_account(&authority, POOL_SUPPLY_BEFORE),
            ),
        },
    ];

    let mut transaction = transaction();
    transaction.account_keys = keys;
    transaction.instructions = vec![retarget(deposit_instruction())];
    transaction.inner_instructions = inner_instructions().into_iter().map(retarget).collect();
    transaction.payer = depositor();

    let manifest = DependencyManifest {
        programs: vec![
            ProgramDependency {
                program_id: SYSTEM.into(),
                source: ProgramSource::Builtin,
                loader: None,
                deployed_slot: None,
                binary_sha256: None,
                binary_len: None,
                observed_slot: Some(SLOT - 1),
                discovered_by: vec![DependencyDiscovery::InnerInstruction],
                note: Some("implemented by the runtime".into()),
            },
            ProgramDependency {
                program_id: STAKE_POOL.into(),
                source: ProgramSource::HistoricalMainnet,
                loader: Some(ProgramLoader::Upgradeable),
                deployed_slot: Some(370_300_186),
                binary_sha256: Some(hash_bytes(&v1.bytes)),
                binary_len: Some(v1.bytes.len() as u64),
                observed_slot: Some(SLOT - 1),
                discovered_by: vec![DependencyDiscovery::ProgramUnderTest],
                note: None,
            },
            ProgramDependency {
                program_id: TOKEN.into(),
                source: ProgramSource::HistoricalMainnet,
                loader: Some(ProgramLoader::Upgradeable),
                deployed_slot: Some(419_472_000),
                binary_sha256: Some(COMMITTED_TOKEN_SHA256.into()),
                binary_len: Some(108_600),
                observed_slot: Some(SLOT - 1),
                discovered_by: vec![DependencyDiscovery::InnerInstruction],
                note: None,
            },
        ],
    };

    let screening = screening::SlotScreening {
        slot: SLOT,
        target_signature: transaction.signature.clone(),
        target_index: 1,
        transactions_in_slot: 3,
        required_accounts: accounts.iter().map(|named| named.address.clone()).collect(),
        conflicts: vec![],
    };

    ReplayRecord {
        schema_version: REPLAY_SCHEMA,
        id: "local-stake-pool-deposit".into(),
        program_id: STAKE_POOL.into(),
        genesis_hash: "offline-test-chain".into(),
        pre_state_hash: state_hash(&accounts).unwrap(),
        accounts,
        transaction,
        clock: ReplayClock {
            slot: SLOT,
            epoch_start_timestamp: 0,
            epoch: 0,
            leader_schedule_epoch: 0,
            unix_timestamp: 1_782_800_000,
        },
        state_source: ReplayStateSource::ControlledSnapshot,
        original: None,
        current_program_sha256: hash_bytes(&v1.bytes),
        dependencies: manifest,
        acquisitions: vec![],
        slot_screening: Some(screening),
        assumptions: vec!["locally constructed CPI scenario".into()],
    }
}

/// The pair the local execution tests run: a record, the reference build, the
/// regressed build, and the committed SPL Token dependency.
fn local_scenario() -> (
    ReplayRecord,
    ProgramVersion,
    ProgramVersion,
    DependencyBundle,
) {
    let v1 = artifact("fixture_stake_pool_reference.so");
    let v2 = artifact("fixture_stake_pool_v2.so");
    let mut record = local_record(&v1);
    let bundle = committed_token_bundle(&record);
    let original = record.execute(&v1, &bundle).expect("reference executes");
    assert!(
        original.success,
        "reference build failed: {:?}\n{:#?}",
        original.error, original.logs
    );
    record.original = Some(OriginalExecution {
        post_accounts: Vec::new(),
        success: original.success,
        fee: original.fee,
        post_state_hash: record.post_hash(&original).unwrap(),
        cpi_invocations: cpi_graph(&original.cpi_calls),
    });
    (record, v1, v2, bundle)
}

// ---------------------------------------------------------------------------
// 7. V1 fidelity across CPI
// ---------------------------------------------------------------------------

/// The gate compares the invocation graph as well as the post-state. A replay
/// can land on the right bytes by luck far more easily than it can make the
/// same calls, to the same programs, at the same depths, from the same
/// instruction.
#[test]
fn fidelity_requires_the_invocation_graph_as_well_as_the_post_state() {
    let (mut record, v1, _, bundle) = local_scenario();
    let original = record.execute(&v1, &bundle).unwrap();
    assert_eq!(record.fidelity(&original).unwrap(), ReplayFidelity::Exact);
    assert!(record.fidelity_failures(&original).unwrap().is_empty());

    let graph = cpi_graph(&original.cpi_calls);
    assert_eq!(graph.len(), 2, "{graph:?}");
    assert_eq!(graph[0].program, SYSTEM);
    assert_eq!(graph[1].program, TOKEN);
    assert!(graph.iter().all(|frame| frame.stack_height == 2));

    // Drop one recorded invocation: the post-state still matches, so this is
    // exactly the failure a state-only gate would miss.
    record
        .original
        .as_mut()
        .unwrap()
        .cpi_invocations
        .truncate(1);
    assert_eq!(
        record.fidelity(&original).unwrap(),
        ReplayFidelity::Mismatch
    );
    let failures = record.fidelity_failures(&original).unwrap();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("invocation graph"), "{failures:?}");
}

#[test]
fn a_fidelity_failure_withholds_the_candidate() {
    let (mut record, v1, v2, bundle) = local_scenario();
    // The recorded evidence and the transaction metadata have to agree with
    // each other before the gate compares either against a replay, so both move.
    record.transaction.fee += 1;
    record.original.as_mut().unwrap().fee += 1;
    let error = compare_with_dependencies(&[record], &v1, &v2, &bundle)
        .expect_err("comparison refused")
        .to_string();
    assert!(error.contains("candidate execution withheld"), "{error}");
    assert!(error.contains("fee"), "{error}");
}

/// A dependency binary that is not the one the record pins is an error, not a
/// different replay.
#[test]
fn a_substituted_dependency_binary_is_refused() {
    let v1 = artifact("fixture_stake_pool_reference.so");
    let mut record = local_record(&v1);
    record
        .dependencies
        .programs
        .iter_mut()
        .find(|program| program.program_id == TOKEN)
        .unwrap()
        .binary_sha256 = Some("0".repeat(64));
    let error = load_dependencies(std::slice::from_ref(&record), &committed_dependencies())
        .expect_err("refused")
        .to_string();
    assert!(error.contains("hashes to"), "{error}");
}

#[test]
fn a_program_that_executes_but_is_not_pinned_is_refused() {
    let v1 = artifact("fixture_stake_pool_reference.so");
    let mut record = local_record(&v1);
    record
        .dependencies
        .programs
        .retain(|program| program.program_id != TOKEN);
    let error = record.validate().expect_err("refused").to_string();
    assert!(error.contains(TOKEN), "{error}");
    assert!(error.contains("does not pin it"), "{error}");
}

#[test]
fn executing_without_the_dependency_bundle_is_refused() {
    let (record, v1, _, _) = local_scenario();
    let error = record
        .execute(&v1, &DependencyBundle::empty())
        .expect_err("refused")
        .to_string();
    assert!(error.contains(TOKEN), "{error}");
}

// ---------------------------------------------------------------------------
// 8-9. Economic decoding and share comparison
// ---------------------------------------------------------------------------

#[test]
fn the_adapter_decodes_pool_state_reserve_and_token_balances() {
    let adapter = protocol::adapter_for(STAKE_POOL).expect("adapter");
    let pool = adapter
        .decode(&snapshot(
            5_143_440,
            STAKE_POOL,
            pool_account(POOL_TOTAL_BEFORE, POOL_SUPPLY_BEFORE),
        ))
        .expect("pool decodes");
    assert_eq!(pool.kind, "stake-pool");
    assert_eq!(
        pool.field("total_lamports").unwrap().value.render(),
        "311850.055457947"
    );
    assert!(pool.field("pool_token_supply").unwrap().economic);
    assert!(!pool.field("last_update_epoch").unwrap().economic);

    let reserve = adapter
        .decode(&snapshot(RESERVE_BEFORE, STAKE, stake_account()))
        .expect("reserve decodes");
    assert_eq!(reserve.kind, "stake-account");
    assert_eq!(
        reserve.field("lamports").unwrap().value.render(),
        "50.000000000"
    );

    let account = adapter
        .decode(&snapshot(
            2_039_280,
            TOKEN,
            token_account(&depositor(), DESTINATION_BEFORE),
        ))
        .expect("token account decodes");
    assert_eq!(account.kind, "token-account");
    assert!(account.field("amount").unwrap().economic);

    let mint = adapter
        .decode(&snapshot(
            1_461_600,
            TOKEN,
            mint_account(&withdraw_authority(), POOL_SUPPLY_BEFORE),
        ))
        .expect("mint decodes");
    assert_eq!(mint.kind, "mint");
    assert_eq!(mint.field("decimals").unwrap().value.render(), "9");
}

#[test]
fn the_share_result_is_reported_for_both_builds_even_when_it_is_unchanged() {
    let (record, v1, _, bundle) = local_scenario();
    let report = compare_with_dependencies(&[record], &v1, &v1, &bundle).expect("comparison");
    let summary = &report.observations[0].economic_summary;
    let field = |name: &str| {
        summary
            .iter()
            .find(|row| row.field == name)
            .unwrap_or_else(|| panic!("{name} summarized"))
    };
    assert_eq!(field("sol_deposited").v1, "0.423000000");
    assert_eq!(field("pool_tokens_received").v1, "0.395906603");
    assert_eq!(field("pool_tokens_received").v2, "0.395906603");
    assert!(field("pool_tokens_received").delta.unwrap().is_zero());
    assert_eq!(field("manager_fee_pool_tokens").v1, "0.000000000");
    assert_eq!(field("pool_token_supply_after").v1, "291876.273221433");
    assert_eq!(report.economic_findings, 0);
}

// ---------------------------------------------------------------------------
// 10. Deliberate candidate regression detection
// ---------------------------------------------------------------------------

/// Both builds succeed, both make the same two calls, and the depositor ends up
/// with fewer shares. That is the shape a byte diff cannot explain on its own.
#[test]
fn the_regressed_candidate_changes_the_economic_result_while_still_succeeding() {
    let (record, v1, v2, bundle) = local_scenario();
    let report = compare_with_dependencies(&[record], &v1, &v2, &bundle).expect("comparison");
    let observation = &report.observations[0];
    assert!(matches!(
        observation.fidelity,
        ReplayFidelity::Exact | ReplayFidelity::Matched
    ));
    assert_eq!(report.economic_findings, 1);
    assert_ne!(
        observation.post_v1_state_hash,
        observation.post_v2_state_hash
    );
    assert!(
        !observation.cpi_graph_changed,
        "the invocation graph is unchanged; only the amount moved"
    );

    let received = observation
        .economic_summary
        .iter()
        .find(|row| row.field == "pool_tokens_received")
        .expect("summarized");
    assert_eq!(received.v1, "0.395906603");
    assert_eq!(received.v2, "0.395885700");
    let delta = received.delta.expect("same mint");
    assert!(delta.base_units < 0, "{delta}");

    let change = observation
        .economic_changes
        .iter()
        .find(|change| change.account_label == "destination-pool-token")
        .expect("the depositor's balance is reported");
    assert_eq!(change.field, "amount");
    assert!(change.delta.unwrap().base_units < 0);
}

// ---------------------------------------------------------------------------
// 11-12. Two channels, and the gate
// ---------------------------------------------------------------------------

/// The protocol-agnostic classifier sees a token account's bytes change and can
/// only call that a raw-data difference. The adapter is what knows the bytes
/// were a balance. Collapsing the two would put protocol semantics in `diff`.
#[test]
fn the_generic_diff_and_the_economic_finding_stay_separate() {
    let (record, v1, v2, bundle) = local_scenario();
    let report = compare_with_dependencies(&[record], &v1, &v2, &bundle).expect("comparison");
    let diff = report.analysis.diffs.first().expect("one structural diff");
    assert!(
        diff.differences.iter().any(|difference| matches!(
            difference,
            eplyx_engine::Difference::RawDataChanged { .. }
        )),
        "{:?}",
        diff.differences
    );
    assert_eq!(report.analysis.summary.critical, 0);
    assert!(report.economic_findings > 0);
}

#[test]
fn the_ci_gate_trips_on_an_economic_finding_the_generic_classifier_rates_lower() {
    let (record, v1, v2, bundle) = local_scenario();
    let report = compare_with_dependencies(&[record], &v1, &v2, &bundle).expect("comparison");
    // This is the condition `compare --fail-on-critical` applies.
    let blocked = report.analysis.summary.critical > 0 || report.economic_findings > 0;
    assert!(blocked);
    assert_eq!(
        report.analysis.summary.critical, 0,
        "the gate must not depend on the generic classifier here"
    );
}

// ---------------------------------------------------------------------------
// 13. Deterministic repeated offline replay
// ---------------------------------------------------------------------------

#[test]
fn repeated_offline_replay_produces_an_identical_report() {
    let (record, v1, v2, bundle) = local_scenario();
    let first =
        compare_with_dependencies(std::slice::from_ref(&record), &v1, &v2, &bundle).unwrap();
    let second = compare_with_dependencies(&[record], &v1, &v2, &bundle).unwrap();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
}

// ---------------------------------------------------------------------------
// 14. Backward compatibility
// ---------------------------------------------------------------------------

/// Records written before any of this existed carry no manifest, no provenance
/// and no screening, and must still load and validate unchanged.
#[test]
fn records_from_earlier_phases_remain_valid() {
    for relative in [
        "docs/examples/replay-record.json",
        "docs/examples/mainnet-replay-record.json",
        "docs/examples/mainnet-token2022-record.json",
    ] {
        let path = eplyx_engine::repo_root().join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        assert!(
            !text.contains("dependencies") && !text.contains("slot_screening"),
            "{relative} predates the CPI fields"
        );
        let record: ReplayRecord = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{relative} no longer deserializes: {e}"));
        record
            .validate()
            .unwrap_or_else(|e| panic!("{relative} no longer validates: {e:#}"));
        assert!(record.dependencies.programs.is_empty());
        assert!(record.slot_screening.is_none());
        assert!(record.acquisitions.is_empty());
    }
}

// ---------------------------------------------------------------------------
// The committed mainnet record
// ---------------------------------------------------------------------------

/// Checked for self-consistency rather than executed: the binaries it pins are
/// 1 MB of mainnet bytecode, which the demo fetches and this repository does not
/// carry.
#[test]
fn the_committed_mainnet_record_is_internally_consistent() {
    let text = include_str!("../../docs/examples/mainnet-stake-pool-record.json");
    let record: ReplayRecord = serde_json::from_str(text).expect("record parses");
    record.validate().expect("record validates");

    assert_eq!(record.program_id, STAKE_POOL);
    assert_eq!(record.state_source, ReplayStateSource::HistoricalArchive);
    assert!(record.slot_screening.as_ref().unwrap().is_clean());

    // V1 is the deployment that was live, which is earlier than the upgrade
    // this phase compares against.
    let pool = record.dependencies.get(STAKE_POOL).expect("pinned");
    assert_eq!(pool.deployed_slot, Some(370_300_186));
    assert!(pool.deployed_slot.unwrap() < record.transaction.slot);
    assert_eq!(
        pool.binary_sha256.as_deref(),
        Some(record.current_program_sha256.as_str())
    );

    let token = record.dependencies.get(TOKEN).expect("pinned");
    assert_eq!(token.deployed_slot, Some(419_472_000));
    assert!(token
        .discovered_by
        .contains(&DependencyDiscovery::InnerInstruction));
    assert_eq!(token.binary_sha256.as_deref(), Some(COMMITTED_TOKEN_SHA256));

    assert_eq!(
        record.dependencies.get(SYSTEM).unwrap().source,
        ProgramSource::Builtin
    );
    let original = record.original.as_ref().expect("evidence");
    assert_eq!(original.cpi_invocations.len(), 2);
    assert_eq!(original.cpi_invocations[0].program, SYSTEM);
    assert_eq!(original.cpi_invocations[1].program, TOKEN);
    assert!(original
        .cpi_invocations
        .iter()
        .all(|frame| frame.stack_height == 2));
}
