use anyhow::Result;
use base64::Engine;
use eplyx_engine::{
    discovery::{replay_eligibility, ReplayEligibility},
    historical::{HistoricalStateProvider, SlotAccountArchiveProvider},
    ingest::rpc::RpcProvider,
    replay::{ReplayStateSource, MEMO_PROGRAM_ID, SYSTEM_PROGRAM_ID},
};
use serde_json::{json, Value};

fn address(byte: u8) -> String {
    solana_address::Address::new_from_array([byte; 32]).to_string()
}

fn signature() -> String {
    bs58::encode([7_u8; 64]).into_string()
}

fn raw_transaction() -> Value {
    let transfer_data = [
        2_u32.to_le_bytes().as_slice(),
        100_u64.to_le_bytes().as_slice(),
    ]
    .concat();
    json!({
        "slot": 200,
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
                "accountKeys": [address(1), address(2), SYSTEM_PROGRAM_ID, MEMO_PROGRAM_ID],
                "recentBlockhash": solana_hash::Hash::default().to_string(),
                "instructions": [
                    {"programIdIndex":2,"accounts":[0,1],"data":bs58::encode(transfer_data).into_string()},
                    {"programIdIndex":3,"accounts":[],"data":bs58::encode(b"historical memo").into_string()}
                ]
            }
        },
        "meta": {
            "err": null,
            "fee": 5_000,
            "computeUnitsConsumed": 2_700,
            "innerInstructions": [],
            "logMessages": [],
            "preBalances": [1_000_000, 890_880, 1, 523_015_135],
            "postBalances": [994_900, 890_980, 1, 523_015_135]
        }
    })
}

struct TransactionRpc;
impl RpcProvider for TransactionRpc {
    fn call(&self, method: &str, _: Value) -> Result<Value> {
        match method {
            "getGenesisHash" => Ok(json!("mainnet-genesis")),
            "getTransaction" => Ok(raw_transaction()),
            _ => anyhow::bail!("unexpected transaction RPC call {method}"),
        }
    }
}

struct ArchiveRpc {
    corrupt_pre: bool,
    wrong_context: bool,
}

fn snapshot(lamports: u64, owner: &str, data: &[u8], executable: bool) -> Value {
    json!({
        "lamports": lamports,
        "owner": owner,
        "data": [base64::prelude::BASE64_STANDARD.encode(data), "base64"],
        "executable": executable,
        "rentEpoch": u64::MAX
    })
}

impl RpcProvider for ArchiveRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        if method == "getGenesisHash" {
            return Ok(json!("mainnet-genesis"));
        }
        anyhow::ensure!(method == "getAccountInfo", "unexpected archive call");
        let requested = params[1]["slot"].as_u64().unwrap();
        let context = if self.wrong_context {
            requested + 1
        } else {
            requested
        };
        let key = params[0].as_str().unwrap();
        let value = if key == MEMO_PROGRAM_ID {
            snapshot(
                523_015_135,
                "BPFLoader2111111111111111111111111111111111",
                b"\x7fELF-test",
                true,
            )
        } else if key == address(1) {
            let lamports = if requested == 199 {
                1_000_000 + u64::from(self.corrupt_pre)
            } else {
                994_900
            };
            snapshot(lamports, SYSTEM_PROGRAM_ID, b"", false)
        } else if key == address(2) {
            snapshot(
                if requested == 199 { 890_880 } else { 890_980 },
                SYSTEM_PROGRAM_ID,
                b"",
                false,
            )
        } else {
            anyhow::bail!("unexpected account {key}")
        };
        Ok(json!({"context":{"slot":context},"value":value}))
    }
}

#[test]
fn exact_archive_builds_a_valid_mainnet_record_and_v1_artifact() {
    let transactions = TransactionRpc;
    let archive = ArchiveRpc {
        corrupt_pre: false,
        wrong_context: false,
    };
    let acquired = SlotAccountArchiveProvider {
        transaction_rpc: &transactions,
        account_archive_rpc: &archive,
    }
    .acquire_exact(&signature())
    .unwrap();
    acquired.record.validate().unwrap();
    assert_eq!(
        acquired.record.state_source,
        ReplayStateSource::HistoricalArchive
    );
    assert_eq!(acquired.record.native_transfer_lamports(), Some(100));
    assert_eq!(acquired.record.accounts[0].account.lamports, 1_000_000);
    assert!(acquired.v1_program.starts_with(b"\x7fELF"));
}

#[test]
fn archive_must_match_transaction_boundary_evidence_exactly() {
    let transactions = TransactionRpc;
    for archive in [
        ArchiveRpc {
            corrupt_pre: true,
            wrong_context: false,
        },
        ArchiveRpc {
            corrupt_pre: false,
            wrong_context: true,
        },
    ] {
        let error = SlotAccountArchiveProvider {
            transaction_rpc: &transactions,
            account_archive_rpc: &archive,
        }
        .acquire_exact(&signature())
        .unwrap_err()
        .to_string();
        assert!(error.contains("does not equal") || error.contains("did not honor"));
    }
}

#[test]
fn extra_instruction_or_cpi_is_refused_before_state_is_claimed() {
    struct UnsupportedRpc;
    impl RpcProvider for UnsupportedRpc {
        fn call(&self, method: &str, _: Value) -> Result<Value> {
            match method {
                "getGenesisHash" => Ok(json!("mainnet-genesis")),
                "getTransaction" => {
                    let mut raw = raw_transaction();
                    raw["transaction"]["message"]["instructions"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"programIdIndex":3,"accounts":[],"data":""}));
                    Ok(raw)
                }
                _ => anyhow::bail!("archive must not be queried for unsupported transaction"),
            }
        }
    }
    let rpc = UnsupportedRpc;
    assert!(SlotAccountArchiveProvider {
        transaction_rpc: &rpc,
        account_archive_rpc: &rpc,
    }
    .acquire_exact(&signature())
    .unwrap_err()
    .to_string()
    .contains("outside the bounded"));
}

#[test]
fn committed_mainnet_record_is_historical_state_ready_and_self_consistent() {
    let record: eplyx_engine::replay::ReplayRecord = serde_json::from_str(include_str!(
        "../../docs/examples/mainnet-replay-record.json"
    ))
    .unwrap();
    record.validate().unwrap();
    assert_eq!(record.state_source, ReplayStateSource::HistoricalArchive);
    assert_eq!(record.native_transfer_lamports(), Some(19_661));
    assert_eq!(
        replay_eligibility(
            &record.transaction,
            MEMO_PROGRAM_ID,
            Some(&record.state_source)
        ),
        ReplayEligibility::HistoricalStateReady
    );
}
