//! Phase 7: stateful production-protocol replay.
//!
//! Two kinds of coverage. The synthetic archive exercises the acquisition path
//! and every rejection it is supposed to make, deterministically and offline.
//! The committed record is the real mainnet artefact the phase was measured on,
//! so a change that would have made that acquisition impossible fails here
//! rather than only on a network run.

use anyhow::Result;
use base64::Engine as _;
use eplyx_engine::{
    executor::ExecutionResult,
    historical::{HistoricalStateProvider, ProtocolArchiveProvider},
    ingest::rpc::RpcProvider,
    protocol::{adapter_for, token2022::PROGRAM_ID as TOKEN_2022},
    replay::{ReplayRecord, ReplayStateSource},
    types::AccountSnapshot,
    versions::{self, ProgramLoader},
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const UPGRADEABLE_LOADER: &str = "BPFLoaderUpgradeab1e11111111111111111111111";
const SLOT: u64 = 200;
const DECIMALS: u8 = 6;
const TRANSFER: u64 = 10_000_000;
const SOURCE_BEFORE: u64 = 25_000_000;
const DESTINATION_BEFORE: u64 = 4_000_000;

fn address(byte: u8) -> String {
    solana_address::Address::new_from_array([byte; 32]).to_string()
}

fn signature() -> String {
    bs58::encode([9_u8; 64]).into_string()
}

fn base58_bytes(address: &str) -> Vec<u8> {
    bs58::decode(address).into_vec().expect("valid address")
}

/// A base SPL token account: mint, owner, amount, then the initialized flag.
fn token_account(mint: &str, owner: &str, amount: u64) -> Vec<u8> {
    let mut data = vec![0_u8; 165];
    data[..32].copy_from_slice(&base58_bytes(mint));
    data[32..64].copy_from_slice(&base58_bytes(owner));
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1;
    data
}

fn mint_account(decimals: u8) -> Vec<u8> {
    let mut data = vec![0_u8; 82];
    data[44] = decimals;
    data[45] = 1;
    data
}

fn encode(data: &[u8]) -> String {
    base64::prelude::BASE64_STANDARD.encode(data)
}

fn authority() -> String {
    address(1)
}
fn source() -> String {
    address(2)
}
fn destination() -> String {
    address(3)
}
fn mint() -> String {
    address(4)
}

/// A direct `TransferChecked`: authority signs, source and destination are
/// writable, mint and program are read-only.
fn raw_transaction() -> Value {
    let mut data = vec![12_u8];
    data.extend_from_slice(&TRANSFER.to_le_bytes());
    data.push(DECIMALS);
    json!({
        "slot": SLOT,
        "blockTime": 1_700_000_000,
        "version": "legacy",
        "transaction": {
            "signatures": [signature()],
            "message": {
                "header": {
                    "numRequiredSignatures": 1,
                    "numReadonlySignedAccounts": 0,
                    "numReadonlyUnsignedAccounts": 2
                },
                "accountKeys": [authority(), source(), destination(), mint(), TOKEN_2022],
                "recentBlockhash": solana_hash::Hash::default().to_string(),
                "instructions": [
                    {"programIdIndex": 4, "accounts": [1, 3, 2, 0],
                     "data": bs58::encode(&data).into_string()}
                ]
            }
        },
        "meta": {
            "err": null,
            "fee": 5_000,
            "computeUnitsConsumed": 3_800,
            "innerInstructions": [],
            "logMessages": [],
            "preBalances": [1_000_000, 2_039_280, 2_039_280, 1_461_600, 1],
            "postBalances": [995_000, 2_039_280, 2_039_280, 1_461_600, 1],
            "preTokenBalances": [
                {"accountIndex": 1, "mint": mint(), "programId": TOKEN_2022,
                 "uiTokenAmount": {"amount": SOURCE_BEFORE.to_string(), "decimals": DECIMALS}},
                {"accountIndex": 2, "mint": mint(), "programId": TOKEN_2022,
                 "uiTokenAmount": {"amount": DESTINATION_BEFORE.to_string(), "decimals": DECIMALS}}
            ],
            "postTokenBalances": [
                {"accountIndex": 1, "mint": mint(), "programId": TOKEN_2022,
                 "uiTokenAmount": {"amount": (SOURCE_BEFORE - TRANSFER).to_string(), "decimals": DECIMALS}},
                {"accountIndex": 2, "mint": mint(), "programId": TOKEN_2022,
                 "uiTokenAmount": {"amount": (DESTINATION_BEFORE + TRANSFER).to_string(), "decimals": DECIMALS}}
            ]
        }
    })
}

fn snapshot_json(lamports: u64, owner: &str, data: &[u8], executable: bool) -> Value {
    json!({
        "data": [encode(data), "base64"],
        "owner": owner,
        "lamports": lamports,
        "executable": executable,
        "rentEpoch": 0,
        "space": data.len(),
    })
}

/// Deterministic stand-in for a slot-addressable archive plus transaction RPC.
struct Archive {
    transaction: Value,
    /// (address, slot) -> account JSON. A missing entry answers null, which is
    /// how a key that holds no account is represented.
    accounts: BTreeMap<(String, u64), Value>,
}

impl Archive {
    fn new() -> Self {
        let programdata = address(200);
        let mut elf = b"\x7fELF".to_vec();
        elf.extend_from_slice(&[0_u8; 128]);
        let mut programdata_data = vec![0_u8; 45];
        programdata_data[..4].copy_from_slice(&3_u32.to_le_bytes());
        programdata_data[4..12].copy_from_slice(&150_u64.to_le_bytes());
        programdata_data[12] = 0;
        programdata_data.extend_from_slice(&elf);

        let mut program_data = 2_u32.to_le_bytes().to_vec();
        program_data.extend_from_slice(&base58_bytes(&programdata));

        let mut accounts = BTreeMap::new();
        for slot in [SLOT - 1, SLOT] {
            accounts.insert(
                (TOKEN_2022.to_string(), slot),
                snapshot_json(1, UPGRADEABLE_LOADER, &program_data, true),
            );
            accounts.insert(
                (programdata.clone(), slot),
                snapshot_json(1, UPGRADEABLE_LOADER, &programdata_data, false),
            );
            accounts.insert(
                (mint(), slot),
                snapshot_json(1_461_600, TOKEN_2022, &mint_account(DECIMALS), false),
            );
        }
        accounts.insert(
            (authority(), SLOT - 1),
            snapshot_json(1_000_000, "11111111111111111111111111111111", &[], false),
        );
        accounts.insert(
            (authority(), SLOT),
            snapshot_json(995_000, "11111111111111111111111111111111", &[], false),
        );
        for (slot, source_amount, destination_amount) in [
            (SLOT - 1, SOURCE_BEFORE, DESTINATION_BEFORE),
            (
                SLOT,
                SOURCE_BEFORE - TRANSFER,
                DESTINATION_BEFORE + TRANSFER,
            ),
        ] {
            accounts.insert(
                (source(), slot),
                snapshot_json(
                    2_039_280,
                    TOKEN_2022,
                    &token_account(&mint(), &authority(), source_amount),
                    false,
                ),
            );
            accounts.insert(
                (destination(), slot),
                snapshot_json(
                    2_039_280,
                    TOKEN_2022,
                    &token_account(&mint(), &address(5), destination_amount),
                    false,
                ),
            );
        }
        Self {
            transaction: raw_transaction(),
            accounts,
        }
    }
}

impl RpcProvider for Archive {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "getGenesisHash" => Ok(json!("test-genesis")),
            "getTransaction" => Ok(self.transaction.clone()),
            "getAccountInfo" => {
                let address = params[0].as_str().expect("address").to_string();
                let slot = params[1]["slot"].as_u64().expect("slot selector");
                let Some(account) = self.accounts.get(&(address, slot)) else {
                    return Ok(json!({"context": {"slot": slot}, "value": null}));
                };
                let mut account = account.clone();
                // Honour dataSlice the way the archive does, so chunked reads
                // are exercised rather than bypassed.
                if let Some(slice) = params[1].get("dataSlice") {
                    let offset = slice["offset"].as_u64().unwrap_or(0) as usize;
                    let length = slice["length"].as_u64().unwrap_or(0) as usize;
                    let full = base64::prelude::BASE64_STANDARD
                        .decode(account["data"][0].as_str().expect("data"))?;
                    let end = offset.saturating_add(length).min(full.len());
                    let part = full.get(offset..end).unwrap_or_default();
                    account["data"] = json!([encode(part), "base64"]);
                }
                Ok(json!({"context": {"slot": slot}, "value": account}))
            }
            other => anyhow::bail!("unexpected RPC {other}"),
        }
    }
}

fn acquire(archive: &Archive) -> Result<eplyx_engine::historical::HistoricalAcquisition> {
    ProtocolArchiveProvider {
        block_rpc: None,
        transaction_rpc: archive,
        account_archive_rpc: archive,
        program_id: TOKEN_2022,
    }
    .acquire_exact(&signature())
}

#[test]
fn archive_acquisition_builds_a_valid_stateful_record() {
    let acquired = acquire(&Archive::new()).expect("acquisition");
    let record = &acquired.record;
    record.validate().expect("record validates");
    assert_eq!(record.program_id, TOKEN_2022);
    assert_eq!(record.state_source, ReplayStateSource::HistoricalArchive);
    assert_eq!(record.clock.slot, SLOT);
    assert!(acquired.v1_program.starts_with(b"\x7fELF"));

    // Labels come from the transfer's account roles, not from addresses.
    let labels: Vec<_> = record.accounts.iter().map(|a| a.label.as_str()).collect();
    assert!(labels.contains(&"source"), "{labels:?}");
    assert!(labels.contains(&"destination"), "{labels:?}");
    assert!(labels.contains(&"mint"), "{labels:?}");

    // The program under test is never seeded as state.
    assert!(record.accounts.iter().all(|a| a.address != TOKEN_2022));
}

#[test]
fn acquired_snapshots_decode_to_the_validator_observed_amounts() {
    let acquired = acquire(&Archive::new()).expect("acquisition");
    let adapter = adapter_for(TOKEN_2022).expect("adapter");
    let source = acquired
        .record
        .accounts
        .iter()
        .find(|a| a.label == "source")
        .expect("source snapshot");
    let decoded = adapter.decode(&source.account).expect("decodes");
    assert_eq!(decoded.kind, "token-account");
    assert_eq!(
        decoded
            .field("amount")
            .and_then(|f| f.value.as_quantity())
            .map(|q| q.base_units),
        Some(SOURCE_BEFORE)
    );
}

/// The proof exists to reject a snapshot taken on the wrong side of a same-slot
/// write. This is the failure that rejected several real candidates.
#[test]
fn a_snapshot_contradicting_validator_metadata_is_refused() {
    let mut archive = Archive::new();
    archive.accounts.insert(
        (source(), SLOT - 1),
        snapshot_json(
            2_039_280,
            TOKEN_2022,
            &token_account(&mint(), &authority(), SOURCE_BEFORE + 1),
            false,
        ),
    );
    let error = acquire(&archive).expect_err("must refuse").to_string();
    assert!(error.contains("differs from validator-observed"), "{error}");
}

#[test]
fn a_lamport_boundary_mismatch_names_the_side_it_failed_on() {
    let mut archive = Archive::new();
    archive.accounts.insert(
        (authority(), SLOT),
        snapshot_json(994_000, "11111111111111111111111111111111", &[], false),
    );
    let error = acquire(&archive).expect_err("must refuse").to_string();
    assert!(
        error.contains("slot-end state is not this transaction's post-state"),
        "{error}"
    );
}

#[test]
fn a_read_only_account_that_changed_across_the_boundary_is_refused() {
    let mut archive = Archive::new();
    archive.accounts.insert(
        (mint(), SLOT),
        snapshot_json(1_461_600, TOKEN_2022, &mint_account(DECIMALS + 1), false),
    );
    let error = acquire(&archive).expect_err("must refuse").to_string();
    assert!(error.contains("read-only account"), "{error}");
}

#[test]
fn the_record_states_what_its_proof_rests_on() {
    let acquired = acquire(&Archive::new()).expect("acquisition");
    let assumptions = acquired.record.assumptions.join("\n");
    for expected in [
        "match the validator-observed pre/post token balances",
        "read-only accounts, including the mint",
        "V1 is the slot-150 deployment live at slot 200",
        "feature set",
    ] {
        assert!(
            assumptions.contains(expected),
            "missing {expected:?} in:\n{assumptions}"
        );
    }
}

/// One unsupported shape: a name, a mutation applied to the raw transaction,
/// and the fragment its rejection must mention.
struct UnsupportedShape {
    name: &'static str,
    mutate: fn(&mut Value),
    expected: &'static str,
}

/// Rejections the adapter must make before any archive request is spent.
#[test]
fn unsupported_transaction_shapes_are_refused() {
    let cases = [
        UnsupportedShape {
            // Well-formed v0 with nothing actually loaded, so normalization
            // succeeds and the adapter is what refuses it. Address lookup
            // tables are normalized but never executed.
            name: "versioned message",
            mutate: |raw| {
                raw["version"] = json!(0);
                raw["transaction"]["message"]["addressTableLookups"] = json!([]);
                raw["meta"]["loadedAddresses"] = json!({"writable": [], "readonly": []});
            },
            expected: "legacy",
        },
        UnsupportedShape {
            name: "cross-program invocation",
            mutate: |raw| {
                raw["meta"]["innerInstructions"] = json!([
                    {"index": 0, "instructions": [
                        {"programIdIndex": 4, "accounts": [], "data": ""}]}
                ]);
            },
            expected: "CPI",
        },
        UnsupportedShape {
            name: "unsupported instruction",
            mutate: |raw| {
                let mut data = vec![3_u8];
                data.extend_from_slice(&TRANSFER.to_le_bytes());
                raw["transaction"]["message"]["instructions"][0]["data"] =
                    json!(bs58::encode(&data).into_string());
            },
            expected: "TransferChecked only",
        },
        UnsupportedShape {
            name: "extra accounts implying a hook or multisig",
            mutate: |raw| {
                raw["transaction"]["message"]["instructions"][0]["accounts"] =
                    json!([1, 3, 2, 0, 0])
            },
            expected: "supported shape",
        },
        UnsupportedShape {
            name: "an unrelated program in the message",
            mutate: |raw| {
                raw["transaction"]["message"]["accountKeys"][3] =
                    json!("11111111111111111111111111111111");
                raw["transaction"]["message"]["instructions"][0]["programIdIndex"] = json!(3);
            },
            expected: "unsupported program",
        },
        UnsupportedShape {
            name: "an originally failed transaction",
            mutate: |raw| raw["meta"]["err"] = json!({"InstructionError": [0, "Custom"]}),
            expected: "successfully captured",
        },
    ];
    for case in cases {
        let mut archive = Archive::new();
        (case.mutate)(&mut archive.transaction);
        let error = acquire(&archive)
            .expect_err(&format!("{} must be refused", case.name))
            .to_string();
        assert!(
            error.contains(case.expected),
            "{}: expected {:?}, got {error}",
            case.name,
            case.expected
        );
    }
}

#[test]
fn version_resolution_reads_the_deployment_live_at_the_slot() {
    let archive = Archive::new();
    let resolved = versions::resolve_at(&archive, TOKEN_2022, SLOT - 1).expect("resolves");
    assert_eq!(resolved.loader, ProgramLoader::Upgradeable);
    assert_eq!(resolved.deploy_slot, Some(150));
    // A `None` authority is a finalised, immutable deployment.
    assert_eq!(resolved.upgrade_authority, None);
    assert!(resolved.elf.starts_with(b"\x7fELF"));
    assert_eq!(resolved.observed_slot, SLOT - 1);
}

#[test]
fn economic_interpretation_reports_the_token_delta_with_mint_decimals() {
    let acquired = acquire(&Archive::new()).expect("acquisition");
    let adapter = adapter_for(TOKEN_2022).expect("adapter");
    let result = |destination_amount: u64| ExecutionResult {
        version: "test".into(),
        success: true,
        error: None,
        compute_units: Some(3_800),
        fee: 5_000,
        logs: vec![],
        cpi_calls: vec![],
        accounts: BTreeMap::from([(
            "destination".to_string(),
            AccountSnapshot {
                lamports: 2_039_280,
                owner: TOKEN_2022.into(),
                data: token_account(&mint(), &address(5), destination_amount),
                executable: false,
                rent_epoch: 0,
            },
        )]),
    };
    let expected = DESTINATION_BEFORE + TRANSFER;
    let changes = adapter.interpret(
        &acquired.record.accounts,
        &result(expected),
        &result(expected - 1_000),
    );
    let change = changes
        .iter()
        .find(|change| change.field == "amount")
        .expect("amount difference");
    assert_eq!(change.account_label, "destination");
    assert_eq!(change.v1, "14.000000");
    assert_eq!(change.v2, "13.999000");
    assert_eq!(
        change.delta.map(|delta| delta.to_string()),
        Some("-0.001000".to_string())
    );

    // Identical executions must produce no economic finding at all: that is the
    // result a behaviour-preserving upgrade has to be able to reach.
    assert!(adapter
        .interpret(
            &acquired.record.accounts,
            &result(expected),
            &result(expected)
        )
        .is_empty());
}

/// The real mainnet artefact the phase was measured on.
#[test]
fn committed_mainnet_record_is_exact_ready_and_self_consistent() {
    let record: ReplayRecord = serde_json::from_str(include_str!(
        "../../docs/examples/mainnet-token2022-record.json"
    ))
    .expect("committed record parses");
    record.validate().expect("committed record validates");

    assert_eq!(record.program_id, TOKEN_2022);
    assert_eq!(record.state_source, ReplayStateSource::HistoricalArchive);
    assert_eq!(record.transaction.slot, 427_146_982);
    // The V1 artefact is the Token-2022 deployment made at slot 395047597.
    assert_eq!(
        record.current_program_sha256,
        "b2a7ce1ea6dfbcbc5ccb0e7f48f7c61dced1a86582d1c7d2e059ac54ed612da4"
    );

    let adapter = adapter_for(TOKEN_2022).expect("adapter");
    adapter
        .accept(&record.transaction)
        .expect("committed transaction is inside the supported contract");

    // Every snapshot the validator recorded a token balance for must decode to
    // exactly that amount; this is the proof that made the record ExactReady.
    let balances = record
        .transaction
        .pre_token_balances
        .as_ref()
        .expect("token balance evidence");
    assert_eq!(balances.len(), 2);
    for balance in balances {
        let address = &record.transaction.account_keys[balance.account_index].address;
        let snapshot = record
            .accounts
            .iter()
            .find(|account| &account.address == address)
            .expect("snapshot for every proved balance");
        assert_eq!(
            eplyx_engine::protocol::token2022::token_account_amount(&snapshot.account.data),
            Some(balance.amount),
            "{} decodes to the validator-observed amount",
            snapshot.label
        );
    }

    // 10.000000 PYUSD moved, and the mint carries real extension state.
    let transferred: Vec<u64> = balances.iter().map(|b| b.amount).collect();
    assert!(transferred.contains(&10_000_000), "{transferred:?}");
    let mint = record
        .accounts
        .iter()
        .find(|account| account.label == "mint")
        .expect("mint snapshot");
    assert!(
        mint.account.data.len() > 165,
        "PYUSD's mint carries extensions past the base layout"
    );
    assert_eq!(
        eplyx_engine::protocol::token2022::mint_decimals(&mint.account.data),
        Some(6)
    );
}
