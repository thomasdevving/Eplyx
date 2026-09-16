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
    path::PathBuf,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
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
        dependencies: Default::default(),
        acquisitions: vec![],
        slot_screening: None,
        assumptions: vec!["offline test".into()],
    };
    record.original = Some(OriginalExecution {
        post_accounts: Vec::new(),
        success: original.success,
        fee: original.fee,
        post_state_hash: record.post_hash(&original).unwrap(),
        cpi_invocations: cpi_graph(&original.cpi_calls),
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
fn normalization_derives_native_movement_without_calling_it_economic_value() {
    let mut raw = raw_transaction();
    let count = raw["transaction"]["message"]["accountKeys"]
        .as_array()
        .unwrap()
        .len();
    let pre = vec![100_u64; count];
    let mut post = pre.clone();
    post[0] = 90;
    post[1] = 110;
    raw["meta"]["preBalances"] = json!(pre);
    raw["meta"]["postBalances"] = json!(post);
    assert_eq!(normalize(&raw).unwrap().native_value_lamports, Some(10));
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
    calls: AtomicUsize,
}
impl RpcProvider for CountingRpc {
    fn call(&self, _: &str, params: Value) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(params)
    }
}
#[test]
fn cache_paths_are_deterministic_and_roundtrip_without_rpc() {
    let temp = Temp::new();
    let rpc = CountingRpc {
        calls: AtomicUsize::new(0),
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
    assert_eq!(rpc.calls.load(Ordering::Relaxed), 1);
    std::fs::write(a, b"bad json").unwrap();
    assert!(cache.call("getTransaction", params).is_err());
    assert_eq!(rpc.calls.load(Ordering::Relaxed), 1);
}
#[test]
fn exact_offline_replay_uses_fresh_state_and_detects_candidate_regression() {
    let record = record();
    let (v1, v2) = versions();
    let serialized = serde_json::to_vec(&record).unwrap();
    let restored: ReplayRecord = serde_json::from_slice(&serialized).unwrap();
    assert_eq!(restored, record);
    assert!(!String::from_utf8(serialized).unwrap().contains("seed"));
    let first = record.execute(&v1, &DependencyBundle::empty()).unwrap();
    let candidate = record.execute(&v2, &DependencyBundle::empty()).unwrap();
    let second = record.execute(&v1, &DependencyBundle::empty()).unwrap();
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
    let original = record.execute(&v1, &DependencyBundle::empty()).unwrap();
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
    pages: AtomicUsize,
    raw: Value,
}

struct ParallelRpc {
    pages: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    raw: Value,
    signatures: Vec<String>,
}
impl RpcProvider for ParallelRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "getGenesisHash" => Ok(json!("test-genesis")),
            "getSignaturesForAddress" => {
                if self.pages.fetch_add(1, Ordering::Relaxed) == 0 {
                    Ok(Value::Array(
                        self.signatures
                            .iter()
                            .map(|signature| {
                                json!({"signature":signature,"slot":executor::FIXED_SLOT})
                            })
                            .collect(),
                    ))
                } else {
                    Ok(json!([]))
                }
            }
            "getTransaction" => {
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(active, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(15));
                let mut raw = self.raw.clone();
                raw["transaction"]["signatures"][0] = params[0].clone();
                self.active.fetch_sub(1, Ordering::SeqCst);
                Ok(raw)
            }
            _ => anyhow::bail!("unexpected RPC method"),
        }
    }
}

#[test]
fn discovery_respects_configured_parallel_transaction_fetches() {
    let signatures: Vec<_> = (1..=4)
        .map(|byte| bs58::encode([byte; 64]).into_string())
        .collect();
    let rpc = ParallelRpc {
        pages: AtomicUsize::new(0),
        active: AtomicUsize::new(0),
        max_active: AtomicUsize::new(0),
        raw: raw_transaction(),
        signatures,
    };
    let manifest = ingest::discover_bounded_with_concurrency(
        &rpc,
        &engine::fixture_program_id().to_string(),
        executor::FIXED_SLOT,
        executor::FIXED_SLOT,
        Some(4),
        4,
    )
    .unwrap();
    assert_eq!(manifest.transactions.len(), 4);
    assert!(rpc.max_active.load(Ordering::SeqCst) > 1);
    assert!(rpc.max_active.load(Ordering::SeqCst) <= 4);
}
impl RpcProvider for WindowRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "getGenesisHash" => Ok(json!("test-genesis")),
            "getTransaction" => Ok(self.raw.clone()),
            "getSignaturesForAddress" => {
                let page = self.pages.fetch_add(1, Ordering::Relaxed);
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
        pages: AtomicUsize::new(0),
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
    assert_eq!(rpc.pages.load(Ordering::Relaxed), 2);
    let rpc = WindowRpc {
        pages: AtomicUsize::new(0),
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

/// Phase 9: the known runtime-semantics divergence is classified as itself.
///
/// Solana refuses to credit an account that stays below its rent-exempt
/// minimum. Mainnet accepted transactions that do so - the Jito tip accounts
/// that surfaced this - and the replay runtime does not, so the record is
/// rejected up front under its own name rather than as an unexplained
/// post-state mismatch several stages later.
#[test]
fn crediting_a_rent_paying_account_is_classified_not_left_as_a_mismatch() {
    let mut record = record();
    let target = record.accounts[0].clone();
    let minimum = 890_880_u64; // rent-exempt minimum for a zero-data account

    // Below the minimum before, credited, still below it after.
    record.accounts[0].account.lamports = 5;
    record.accounts[0].account.data = Vec::new();
    record.pre_state_hash = eplyx_engine::replay::state_hash(&record.accounts).unwrap();
    let original = record.original.as_mut().unwrap();
    original.post_accounts = vec![eplyx_engine::replay::PostAccountDigest {
        label: target.label.clone(),
        address: target.address.clone(),
        owner: record.accounts[0].account.owner.clone(),
        lamports: 105,
        data_len: 0,
        data_sha256: eplyx_engine::replay::hash_bytes(&[]),
    }];

    let credits = record.rent_paying_credits();
    assert_eq!(credits.len(), 1, "the credit must be identified");
    let credit = &credits[0];
    assert_eq!(credit.pre_lamports, 5);
    assert_eq!(credit.post_lamports, 105);
    assert_eq!(credit.credited, 100);
    assert_eq!(credit.rent_exempt_minimum, minimum);
    assert_eq!(credit.address, target.address);

    let error = record.validate().expect_err("must be refused").to_string();
    assert!(error.contains("unsupported_runtime_semantics: rent_paying_account_credited"));
    assert!(error.contains(&target.address), "the account must be named");
    assert!(
        error.contains("890880"),
        "the rent-exempt minimum must be reported"
    );
}

/// The classification must not fire on ordinary accounts. A rent-exempt
/// account may be credited freely, and a rent-paying account may be debited.
#[test]
fn ordinary_credits_and_debits_are_not_classified_as_rent_state_failures() {
    let cases: [(u64, u64, &str); 3] = [
        (1_000_000, 1_000_100, "rent-exempt account credited"),
        (5, 4, "rent-paying account debited"),
        (5, 5, "rent-paying account unchanged"),
    ];
    for (pre, post, name) in cases {
        let mut record = record();
        let target = record.accounts[0].clone();
        record.accounts[0].account.lamports = pre;
        record.accounts[0].account.data = Vec::new();
        record.pre_state_hash = eplyx_engine::replay::state_hash(&record.accounts).unwrap();
        record.original.as_mut().unwrap().post_accounts =
            vec![eplyx_engine::replay::PostAccountDigest {
                label: target.label.clone(),
                address: target.address.clone(),
                owner: record.accounts[0].account.owner.clone(),
                lamports: post,
                data_len: 0,
                data_sha256: eplyx_engine::replay::hash_bytes(&[]),
            }];
        assert!(
            record.rent_paying_credits().is_empty(),
            "{name} must not be classified as a rent-state failure"
        );
    }
}
