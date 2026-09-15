//! Offline contract tests. The separate demo validates against an actual Agave
//! validator, not an expected result manufactured by this test's VM.
use anyhow::Result;
use eplyx_engine::{
    self as engine, executor,
    ingest::{self, rpc::RpcProvider, transactions::normalize},
    replay::*,
};
use serde_json::{json, Value};
use std::{
    cell::Cell,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

fn fixture() -> engine::Fixture {
    engine::corpus::generate(&engine::fixture_program_id())
        .into_iter()
        .find(|f| f.id == "boundary-position-017")
        .unwrap()
}
fn raw_transaction() -> Value {
    let fixture = fixture();
    let tx = executor::fixture_transaction(&fixture, solana_hash::Hash::default()).unwrap();
    json!({"slot":executor::FIXED_SLOT,"blockTime":executor::FIXED_UNIX_TIMESTAMP,"version":"legacy",
        "transaction":{"signatures":tx.signatures.iter().map(ToString::to_string).collect::<Vec<_>>(),"message":{
            "header":{"numRequiredSignatures":tx.message.header.num_required_signatures,"numReadonlySignedAccounts":tx.message.header.num_readonly_signed_accounts,"numReadonlyUnsignedAccounts":tx.message.header.num_readonly_unsigned_accounts},
            "accountKeys":tx.message.account_keys.iter().map(ToString::to_string).collect::<Vec<_>>(),"recentBlockhash":tx.message.recent_blockhash.to_string(),
            "instructions":tx.message.instructions.iter().map(|i|json!({"programIdIndex":i.program_id_index,"accounts":i.accounts,"data":bs58::encode(&i.data).into_string()})).collect::<Vec<_>>()
        }},"meta":{"err":null,"fee":5000,"computeUnitsConsumed":100,"innerInstructions":[],"logMessages":[]}})
}
fn versions() -> (engine::ProgramVersion, engine::ProgramVersion) {
    engine::load_versions(
        &engine::default_artifact("v1"),
        &engine::default_artifact("v2"),
    )
    .expect("build SBF artifacts first")
}
fn record() -> ReplayRecord {
    let transaction = normalize(&raw_transaction()).unwrap();
    let mut fixture = fixture();
    fixture.accounts.retain(|a| {
        transaction
            .account_keys
            .iter()
            .any(|k| k.address == a.address)
    });
    fixture.watch = fixture.accounts.iter().map(|a| a.label.clone()).collect();
    let (v1, _) = versions();
    let original = executor::execute(&fixture, &engine::fixture_program_id(), &v1).unwrap();
    let mut record = ReplayRecord {
        schema_version: 1,
        id: fixture.id.clone(),
        program_id: engine::fixture_program_id().to_string(),
        genesis_hash: "offline-test-chain".into(),
        transaction,
        pre_state_hash: state_hash(&fixture.accounts).unwrap(),
        accounts: fixture.accounts,
        state_source: ReplayStateSource::ControlledSnapshot,
        clock: ReplayClock {
            slot: executor::FIXED_SLOT,
            epoch_start_timestamp: executor::FIXED_UNIX_TIMESTAMP,
            epoch: executor::FIXED_EPOCH,
            leader_schedule_epoch: executor::FIXED_EPOCH + 1,
            unix_timestamp: executor::FIXED_UNIX_TIMESTAMP,
        },
        original: None,
        current_program_sha256: hash_bytes(&v1.bytes),
        assumptions: vec!["offline test".into()],
    };
    record.original = Some(OriginalExecution {
        success: original.success,
        fee: original.fee,
        post_state_hash: record.post_hash(&original).unwrap(),
    });
    record
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "replay-tests-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn normalization_preserves_order_privileges_and_instruction_bytes() {
    let raw = raw_transaction();
    let tx = normalize(&raw).unwrap();
    let fixture = fixture();
    assert_eq!(tx.instructions, vec![fixture.instruction]);
    assert!(tx.account_keys[0].is_signer && tx.account_keys[0].is_writable);
    assert_eq!(tx.version, "legacy");
    assert_eq!(tx.slot, executor::FIXED_SLOT);
    assert_eq!(tx.instructions[0].data, vec![7]);
}
#[test]
fn v0_resolves_loaded_accounts_and_preserves_duplicate_metas() {
    let mut raw = raw_transaction();
    raw["version"] = json!(0);
    let loaded = solana_address::Address::new_from_array([8; 32]).to_string();
    let ix = raw["transaction"]["message"]["accountKeys"]
        .as_array()
        .unwrap()
        .len();
    raw["transaction"]["message"]["addressTableLookups"] = json!([{"accountKey":solana_address::Address::new_from_array([9;32]).to_string(),"writableIndexes":[3],"readonlyIndexes":[]}]);
    raw["meta"]["loadedAddresses"] = json!({"writable":[loaded],"readonly":[]});
    raw["transaction"]["message"]["instructions"][0]["accounts"] = json!([ix, ix]);
    let tx = normalize(&raw).unwrap();
    assert_eq!(tx.account_keys.last().unwrap().address, loaded);
    assert!(tx.account_keys.last().unwrap().is_writable);
    assert!(!tx.account_keys.last().unwrap().is_signer);
    assert_eq!(tx.instructions[0].accounts.len(), 2);
    assert_eq!(
        tx.instructions[0].accounts[0],
        tx.instructions[0].accounts[1]
    );
    raw["meta"]["loadedAddresses"]["writable"] = json!([]);
    assert!(normalize(&raw).is_err());
}
#[test]
fn malformed_transactions_fail_instead_of_partial_normalization() {
    assert!(normalize(&Value::Null).is_err());
    let mut raw = raw_transaction();
    raw["transaction"]["message"]["instructions"][0]["programIdIndex"] = json!(250);
    assert!(normalize(&raw).is_err());
    let mut raw = raw_transaction();
    raw["transaction"]["message"]["header"]["numRequiredSignatures"] = json!(200);
    assert!(normalize(&raw).is_err());
    let mut raw = raw_transaction();
    raw["version"] = json!(1);
    assert!(normalize(&raw).is_err());
}
#[test]
fn account_normalization_preserves_all_fields() {
    let raw = json!({"lamports":123,"owner":"11111111111111111111111111111111","executable":true,"rentEpoch":u64::MAX,"data":["AAECAw==","base64"]});
    let a = ingest::accounts::normalize(&raw).unwrap();
    assert_eq!(a.data, vec![0, 1, 2, 3]);
    assert_eq!(a.lamports, 123);
    assert!(a.executable);
    assert_eq!(a.rent_epoch, u64::MAX);
    assert!(ingest::accounts::normalize(&Value::Null).is_err());
}
#[test]
fn canonical_hash_ignores_order_and_labels_but_covers_every_account_field() {
    let accounts = fixture().accounts;
    let hash = state_hash(&accounts).unwrap();
    let mut reverse = accounts.clone();
    reverse.reverse();
    reverse[0].label = "other label".into();
    assert_eq!(state_hash(&reverse).unwrap(), hash);
    for field in 0..6 {
        let mut changed = accounts.clone();
        match field {
            0 => changed[0].account.lamports += 1,
            1 => changed[0].account.data.push(8),
            2 => changed[0].account.owner = "11111111111111111111111111111111".into(),
            3 => changed[0].account.executable = true,
            4 => changed[0].account.rent_epoch += 1,
            _ => changed[0].address = solana_address::Address::new_from_array([55; 32]).to_string(),
        }
        assert_ne!(state_hash(&changed).unwrap(), hash);
    }
    let mut duplicate = accounts.clone();
    duplicate.push(accounts[0].clone());
    assert!(state_hash(&duplicate).is_err());
}
struct CountingRpc {
    calls: Cell<usize>,
}
impl RpcProvider for CountingRpc {
    fn call(&self, _: &str, params: Value) -> Result<Value> {
        self.calls.set(self.calls.get() + 1);
        Ok(params)
    }
}
#[test]
fn cache_paths_are_deterministic_and_roundtrip_without_rpc() {
    let temp = Temp::new();
    let rpc = CountingRpc {
        calls: Cell::new(0),
    };
    let cache = ingest::CachedRpc {
        provider: &rpc,
        root: temp.0.clone(),
    };
    let params = json!(["key",{"encoding":"json"}]);
    let a = ingest::cache_path(&temp.0, "getTransaction", &params);
    assert_eq!(a, ingest::cache_path(&temp.0, "getTransaction", &params));
    assert_ne!(a, ingest::cache_path(&temp.0, "other", &params));
    assert_eq!(
        cache.call("getTransaction", params.clone()).unwrap(),
        params
    );
    assert_eq!(
        cache.call("getTransaction", params.clone()).unwrap(),
        params
    );
    assert_eq!(rpc.calls.get(), 1);
    std::fs::write(a, b"bad json").unwrap();
    assert!(cache.call("getTransaction", params).is_err());
    assert_eq!(rpc.calls.get(), 1);
}
#[test]
fn exact_offline_replay_uses_fresh_state_and_detects_candidate_regression() {
    let record = record();
    let (v1, v2) = versions();
    let serialized = serde_json::to_vec(&record).unwrap();
    let restored: ReplayRecord = serde_json::from_slice(&serialized).unwrap();
    assert_eq!(restored, record);
    assert!(!String::from_utf8(serialized).unwrap().contains("seed"));
    let first = record.execute(&v1).unwrap();
    let candidate = record.execute(&v2).unwrap();
    let second = record.execute(&v1).unwrap();
    assert_eq!(first, second);
    assert_eq!(record.fidelity(&first).unwrap(), ReplayFidelity::Exact);
    assert_ne!(
        record.post_hash(&first).unwrap(),
        record.post_hash(&candidate).unwrap()
    );
    let report = compare(std::slice::from_ref(&record), &v1, &v2).unwrap();
    assert_eq!(report.analysis.summary.critical, 1);
    assert_eq!(report.analysis.economics.newly_liquidatable.positions, 1);
    let repeat = compare(&[record], &v1, &v2).unwrap();
    assert_eq!(
        serde_json::to_vec(&report).unwrap(),
        serde_json::to_vec(&repeat).unwrap()
    );
}
#[test]
fn fidelity_mismatch_blocks_candidate_and_unknown_is_not_exact() {
    let mut record = record();
    let (v1, _) = versions();
    let original = record.execute(&v1).unwrap();
    record.original.as_mut().unwrap().post_state_hash = "wrong".into();
    assert_eq!(
        record.fidelity(&original).unwrap(),
        ReplayFidelity::Mismatch
    );
    let invalid_candidate = engine::ProgramVersion {
        label: "must not execute".into(),
        bytes: vec![],
    };
    let error = compare(std::slice::from_ref(&record), &v1, &invalid_candidate)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Mismatch") && error.contains("withheld"));
    record.original = None;
    assert_eq!(record.fidelity(&original).unwrap(), ReplayFidelity::Unknown);
    record.state_source = ReplayStateSource::CurrentApproximation;
    assert_eq!(
        record.fidelity(&original).unwrap(),
        ReplayFidelity::Approximate
    );
}
#[test]
fn incomplete_or_tampered_prestate_and_privileges_are_rejected() {
    let record = record();
    let mut bad = record.clone();
    bad.accounts[0].account.lamports += 1;
    assert!(bad.validate().is_err());
    let mut bad = record.clone();
    bad.accounts.pop();
    bad.pre_state_hash = state_hash(&bad.accounts).unwrap();
    assert!(bad.validate().is_err());
    let mut bad = record.clone();
    bad.transaction.instructions[0].accounts[0].is_signer = true;
    assert!(bad.validate().is_err());
    let mut bad = record.clone();
    bad.clock.slot += 1;
    assert!(bad.validate().is_err());
    let mut bad = record;
    bad.schema_version = 42;
    assert!(bad.validate().is_err());
}
#[test]
fn corpus_build_matches_capture_to_discovery_and_loads_offline() {
    let temp = Temp::new();
    let record = record();
    let manifest = ingest::IngestManifest {
        schema_version: 1,
        program_id: record.program_id.clone(),
        genesis_hash: record.genesis_hash.clone(),
        start_slot: record.transaction.slot,
        end_slot: record.transaction.slot,
        transactions: vec![record.transaction.clone()],
    };
    ingest::write_json(&temp.0.join("manifest.json"), &manifest).unwrap();
    let snapshots = temp.0.join("snapshots");
    let out = temp.0.join("corpus.json");
    assert!(ingest::build_corpus(&temp.0, &snapshots, &out, None).is_err());
    ingest::write_json(
        &snapshots.join(format!("{}.json", record.transaction.signature)),
        &record,
    )
    .unwrap();
    assert_eq!(
        ingest::build_corpus(&temp.0, &snapshots, &out, None).unwrap(),
        1
    );
    assert_eq!(load_corpus(&out).unwrap(), vec![record.clone()]);
    let mut corrupt = record;
    corrupt.genesis_hash = "other chain".into();
    ingest::write_json(
        &snapshots.join(format!("{}.json", corrupt.transaction.signature)),
        &corrupt,
    )
    .unwrap();
    assert!(ingest::build_corpus(&temp.0, &snapshots, &out, None).is_err());
}

struct WindowRpc {
    pages: Cell<usize>,
    raw: Value,
}
impl RpcProvider for WindowRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "getGenesisHash" => Ok(json!("test-genesis")),
            "getTransaction" => Ok(self.raw.clone()),
            "getSignaturesForAddress" => {
                let page = self.pages.get();
                self.pages.set(page + 1);
                if page == 0 {
                    assert!(params[1].get("before").is_none());
                    Ok(
                        json!([{"signature":self.raw["transaction"]["signatures"][0],"slot":executor::FIXED_SLOT}]),
                    )
                } else {
                    assert_eq!(
                        params[1]["before"],
                        self.raw["transaction"]["signatures"][0]
                    );
                    Ok(json!([]))
                }
            }
            _ => anyhow::bail!("unexpected RPC method"),
        }
    }
}
#[test]
fn discovery_paginates_inclusive_window_and_filters_program_interaction() {
    let rpc = WindowRpc {
        pages: Cell::new(0),
        raw: raw_transaction(),
    };
    let manifest = ingest::discover(
        &rpc,
        &engine::fixture_program_id().to_string(),
        executor::FIXED_SLOT,
        executor::FIXED_SLOT,
    )
    .unwrap();
    assert_eq!(manifest.transactions.len(), 1);
    assert_eq!(rpc.pages.get(), 2);
    let rpc = WindowRpc {
        pages: Cell::new(0),
        raw: raw_transaction(),
    };
    let manifest = ingest::discover(
        &rpc,
        &engine::fixture_program_id().to_string(),
        0,
        executor::FIXED_SLOT - 1,
    )
    .unwrap();
    assert!(manifest.transactions.is_empty());
}

#[test]
fn actual_validator_snapshot_matches_original_offline() {
    // Original post-state hash was captured independently from Agave JSON-RPC,
    // never generated by LiteSVM. The artifact hash pins the captured V1 build.
    let record: ReplayRecord =
        serde_json::from_str(include_str!("../../docs/examples/replay-record.json")).unwrap();
    let (v1, v2) = versions();
    let report = compare(&[record], &v1, &v2).unwrap();
    assert_eq!(report.observations[0].fidelity, ReplayFidelity::Exact);
    assert_eq!(report.analysis.economics.newly_liquidatable.positions, 1);
}
