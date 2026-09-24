use std::{fs, path::PathBuf};

use anyhow::{ensure, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use eplyx_engine::universal::{
    evidence::{EvidenceKind, EvidenceStore},
    execution::{InnerGroup, InnerInstruction, ReturnData},
    model::{ExecutionInput, ExpectedHistoricalOutcome},
    resolver::verify_validator_envelope,
};
use serde_json::{json, Value};

const SIGNATURE: &str =
    "rR1tBf4kb4xocTPpLZXRPqFFtvXQHn2GwAEgHbU8d83URqJNWnACGcz6cJfq27kGYhuWaTjrce7ksBNAF5NZVQC";
const SLOT: u64 = 450026714;
const INDEX: u64 = 95;

fn fixture() -> (
    tempfile::TempDir,
    EvidenceStore,
    Value,
    ExecutionInput,
    ExpectedHistoricalOutcome,
) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u17-phoenix-qualification");
    let response: Value =
        serde_json::from_slice(&fs::read(root.join("transaction.json")).unwrap()).unwrap();
    let raw = response["result"].clone();
    let temp = tempfile::tempdir().unwrap();
    let store = EvidenceStore::at(temp.path().join("evidence"));
    let input = ExecutionInput::capture_legacy_v2(&store, &raw).unwrap();
    let meta = &raw["meta"];
    let inner_instructions = meta["innerInstructions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| InnerGroup {
            outer_index: group["index"].as_u64().unwrap() as usize,
            instructions: group["instructions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|ix| InnerInstruction {
                    program_id_index: ix["programIdIndex"].as_u64().unwrap() as u8,
                    accounts: ix["accounts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_u64().unwrap() as u8)
                        .collect(),
                    data: bs58::decode(ix["data"].as_str().unwrap())
                        .into_vec()
                        .unwrap(),
                    stack_height: ix["stackHeight"].as_u64().unwrap() as u8,
                })
                .collect(),
        })
        .collect();
    let expected = ExpectedHistoricalOutcome {
        success: meta["err"].is_null(),
        error: None,
        fee: meta["fee"].as_u64().unwrap(),
        compute_units: meta["computeUnitsConsumed"].as_u64(),
        logs: meta["logMessages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect(),
        inner_instructions,
        return_data: Some(ReturnData {
            program: meta["returnData"]["programId"].as_str().unwrap().into(),
            data: BASE64_STANDARD
                .decode(meta["returnData"]["data"][0].as_str().unwrap())
                .unwrap(),
        }),
        watched_accounts: vec![],
    };
    (temp, store, raw, input, expected)
}

fn reject(input: &ExecutionInput, store: &EvidenceStore) {
    assert!(
        input.resolve(store, "").is_err(),
        "mutated message or evidence resolved"
    );
}

fn replace_evidence(input: &mut ExecutionInput, store: &EvidenceStore, raw: &Value) {
    let ExecutionInput::LegacyV2 {
        frozen_transaction, ..
    } = input
    else {
        panic!()
    };
    *frozen_transaction = store
        .put(EvidenceKind::Transaction, &serde_json::to_vec(raw).unwrap())
        .unwrap();
}

fn verify_production_position(
    input: &ExecutionInput,
    store: &EvidenceStore,
    analysis: &Value,
) -> Result<()> {
    let transaction = input.resolve(store, "")?.transaction;
    let raw: Value =
        serde_json::from_slice(&store.get(input.validator_transaction_ref().unwrap())?)?;
    ensure!(
        transaction.signature == analysis["signature"].as_str().unwrap(),
        "signature differs from block census"
    );
    ensure!(
        transaction.slot == analysis["slot"].as_u64().unwrap(),
        "slot differs from block census"
    );
    ensure!(
        raw["transactionIndex"] == analysis["index"],
        "index differs from block census"
    );
    Ok(())
}

#[test]
fn phoenix_production_legacy_validator_envelope_qualifies_without_state_or_replay() {
    let (_temp, store, raw, input, expected) = fixture();
    let resolved = input.resolve(&store, "").unwrap();
    verify_validator_envelope(&input, &expected, &store).unwrap();
    assert_eq!(resolved.transaction.version, "legacy");
    assert_eq!(resolved.transaction.signature, SIGNATURE);
    assert_eq!(resolved.transaction.slot, SLOT);
    assert_eq!(raw["transactionIndex"], INDEX);
    assert_eq!(resolved.transaction.instructions.len(), 6);
    assert_eq!(resolved.transaction.account_keys.len(), 18);
    assert_eq!(
        resolved
            .transaction
            .pre_token_balances
            .as_ref()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        resolved
            .transaction
            .post_token_balances
            .as_ref()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(expected.inner_instructions.len(), 1);
    assert_eq!(expected.inner_instructions[0].instructions.len(), 2);
    assert!(expected.return_data.is_some());
    let serialized = serde_json::to_value(&input).unwrap();
    assert_eq!(serialized["format"], "legacy_v2");
    let restored: ExecutionInput = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored, input);
    assert!(input.validator_transaction_ref().is_some());

    // The account-mode block independently locates this signature at index 95.
    // Its account-mode rows cannot reconstruct the compiled message.
    let analysis: Value = serde_json::from_slice(
        &fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../docs/examples/phase-u17-phoenix-qualification/analysis.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(analysis["signature"], SIGNATURE);
    assert_eq!(analysis["slot"], SLOT);
    assert_eq!(analysis["index"], INDEX);
    assert_eq!(analysis["target_version"], "legacy");
    assert_eq!(analysis["historical_replay_attempted"], false);
    assert_eq!(
        analysis["historical_target_account_receipts_acquired"],
        false
    );
    verify_production_position(&input, &store, &analysis).unwrap();
}

#[test]
fn production_block_index_mismatch_fails_with_rebuilt_transaction_evidence() {
    let (_temp, store, mut raw, mut input, _expected) = fixture();
    let analysis: Value = serde_json::from_slice(
        &fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../docs/examples/phase-u17-phoenix-qualification/analysis.json"),
        )
        .unwrap(),
    )
    .unwrap();
    raw["transactionIndex"] = json!(INDEX + 1);
    replace_evidence(&mut input, &store, &raw);
    assert!(
        input.resolve(&store, "").is_ok(),
        "index is a block position, not a message field"
    );
    assert!(verify_production_position(&input, &store, &analysis).is_err());
}

#[test]
fn legacy_message_and_identity_mutations_fail_after_rebuilding_input_identity() {
    let (_temp, store, _raw, input, _expected) = fixture();
    for mutation in 0..10 {
        let mut changed = input.clone();
        let ExecutionInput::LegacyV2 {
            message,
            transaction,
            signatures,
            ..
        } = &mut changed
        else {
            panic!()
        };
        match mutation {
            0 => signatures[0] = bs58::encode([9u8; 64]).into_string(),
            1 => message.recent_blockhash = solana_hash::Hash::new_from_array([9; 32]),
            2 => message.account_keys.swap(1, 2),
            3 => message.header.num_readonly_unsigned_accounts -= 1,
            4 => message.header.num_required_signatures += 1,
            5 => message.instructions.truncate(5),
            6 => message.instructions[5].program_id_index = 0,
            7 => message.instructions[5].accounts.swap(2, 3),
            8 => message.instructions[5].data[0] ^= 1,
            9 => transaction.slot += 1,
            _ => unreachable!(),
        }
        reject(&changed, &store);
    }
    let mut changed = input.clone();
    let ExecutionInput::LegacyV2 { transaction, .. } = &mut changed else {
        panic!()
    };
    transaction.signature = bs58::encode([8u8; 64]).into_string();
    reject(&changed, &store);
}

#[test]
fn swapped_validator_evidence_and_balance_metadata_fail_with_new_content_hashes() {
    let (_temp, store, raw, input, _expected) = fixture();
    for mutation in 0..5 {
        let mut changed_raw = raw.clone();
        match mutation {
            0 => {
                changed_raw["transaction"]["signatures"][0] =
                    json!(bs58::encode([7u8; 64]).into_string())
            }
            1 => changed_raw["meta"]["preBalances"][0] = json!(1),
            2 => changed_raw["meta"]["postBalances"][0] = json!(1),
            3 => changed_raw["meta"]["preTokenBalances"][0]["uiTokenAmount"]["amount"] = json!("1"),
            4 => {
                changed_raw["meta"]["loadedAddresses"]["writable"] =
                    json!(["11111111111111111111111111111111"])
            }
            _ => unreachable!(),
        }
        let mut changed = input.clone();
        replace_evidence(&mut changed, &store, &changed_raw);
        reject(&changed, &store);
    }
}

#[test]
fn validator_outcome_mutations_fail_against_retained_envelope() {
    let (_temp, store, _raw, input, expected) = fixture();
    for mutation in 0..6 {
        let mut changed = expected.clone();
        match mutation {
            0 => {
                changed.success = false;
                changed.error = Some("forged".into());
            }
            1 => changed.fee += 1,
            2 => changed.compute_units = Some(changed.compute_units.unwrap() + 1),
            3 => changed.logs.push("forged".into()),
            4 => changed.inner_instructions[0].instructions[0].data[0] ^= 1,
            5 => changed.return_data.as_mut().unwrap().data[0] ^= 1,
            _ => unreachable!(),
        }
        assert!(verify_validator_envelope(&input, &changed, &store).is_err());
    }
}

#[test]
fn old_legacy_serialization_keeps_its_old_tag_and_no_validator_reference() {
    let (_temp, store, _raw, input, _expected) = fixture();
    let ExecutionInput::LegacyV2 {
        message,
        transaction,
        ..
    } = input
    else {
        panic!()
    };
    let old = ExecutionInput::Legacy {
        message: message.clone(),
        transaction: transaction.clone(),
    };
    let old_value = serde_json::to_value(&old).unwrap();
    assert_eq!(old_value["format"], "legacy");
    assert!(old_value.get("frozen_transaction").is_none());
    let restored: ExecutionInput = serde_json::from_value(old_value).unwrap();
    assert_eq!(restored, old);
    assert!(restored.validator_transaction_ref().is_none());
    assert!(restored.resolve(&store, "").is_ok());
    let compat = ExecutionInput::LegacyV1Compatibility {
        message,
        transaction,
    };
    assert_eq!(
        serde_json::to_value(&compat).unwrap()["format"],
        "legacy_v1_compatibility"
    );
    assert!(compat.validator_transaction_ref().is_none());
}
