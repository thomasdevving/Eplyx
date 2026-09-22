use std::{
    fs,
    path::{Path, PathBuf},
};

use eplyx_engine::{
    corpus_store::CorpusStore,
    types::AccountSnapshot,
    universal::{
        checkpoint::{
            AccountDerivationV2, DerivedAccountEvidenceV2, ObservedCheckpointV1,
            TransactionClosureProofV1,
        },
        evidence::{AccountBoundary, AccountObservation, EvidenceKind, EvidenceRef, EvidenceStore},
        model::ReplayObservationV2,
    },
};
use serde_json::{json, Value};

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../docs/examples/phase-u11-2-checkpointed-corpus")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn fixture() -> (ReplayObservationV2, tempfile::TempDir, EvidenceStore) {
    let root = corpus_root();
    let record = CorpusStore::open(&root)
        .unwrap()
        .load_v2()
        .unwrap()
        .pop()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    copy_tree(&root.join("evidence"), &temp.path().join("evidence"));
    let store = EvidenceStore::at(temp.path().join("evidence"));
    (record, temp, store)
}

fn reject(name: &str, record: &mut ReplayObservationV2, store: &EvidenceStore, expected: &str) {
    record.id = record.identity().unwrap();
    let error = record
        .resolve(store)
        .err()
        .unwrap_or_else(|| panic!("{name}: accepted mutation"))
        .to_string();
    assert!(
        error.contains(expected),
        "{name}: expected {expected:?}, got {error:?}"
    );
}

fn derived(
    record: &ReplayObservationV2,
    store: &EvidenceStore,
    address: &str,
) -> DerivedAccountEvidenceV2 {
    let reference = &record
        .runtime
        .sysvars
        .iter()
        .find(|seed| seed.address == address)
        .unwrap()
        .observation;
    serde_json::from_slice(&store.get(reference).unwrap()).unwrap()
}

fn replace_derived(
    record: &mut ReplayObservationV2,
    store: &EvidenceStore,
    evidence: &DerivedAccountEvidenceV2,
) {
    let reference = evidence.store(store).unwrap();
    record
        .runtime
        .sysvars
        .iter_mut()
        .find(|seed| seed.address == evidence.address)
        .unwrap()
        .observation = reference;
}

fn closure(record: &ReplayObservationV2, store: &EvidenceStore) -> TransactionClosureProofV1 {
    serde_json::from_slice(
        &store
            .get(
                &record
                    .checkpointed_execution
                    .as_ref()
                    .unwrap()
                    .closure_proof,
            )
            .unwrap(),
    )
    .unwrap()
}

fn replace_closure(
    record: &mut ReplayObservationV2,
    store: &EvidenceStore,
    proof: &TransactionClosureProofV1,
) {
    record
        .checkpointed_execution
        .as_mut()
        .unwrap()
        .closure_proof = proof.store(store).unwrap();
}

#[test]
fn reconstructive_derived_account_mutations_fail_closed() {
    let (control, _temp, store) = fixture();
    control
        .resolve(&store)
        .expect("reconstructive control resolves");
    let recent = "SysvarRecentB1ockHashes11111111111111111111";
    let slot_hashes = "SysvarS1otHashes111111111111111111111111111";

    let mut record = control.clone();
    let mut evidence = derived(&record, &store, recent);
    let mut content: AccountSnapshot =
        serde_json::from_slice(&store.get(&evidence.content).unwrap()).unwrap();
    content.data[100] ^= 1;
    evidence.content = store
        .put(
            EvidenceKind::AccountContent,
            &serde_json::to_vec(&content).unwrap(),
        )
        .unwrap();
    replace_derived(&mut record, &store, &evidence);
    reject(
        "recent_content_rehashed_but_sources_unchanged",
        &mut record,
        &store,
        "derived account content differs from reconstructed sources",
    );

    let mut record = control.clone();
    let mut evidence = derived(&record, &store, recent);
    if let AccountDerivationV2::RecentBlockhashesPreTargetV1 {
        tail_block,
        acquisition_receipt,
        ..
    } = &mut evidence.derivation
    {
        let mut block: Value = serde_json::from_slice(&store.get(tail_block).unwrap()).unwrap();
        block["result"]["previousBlockhash"] = block["result"]["blockhash"].clone();
        *tail_block = store
            .put(
                EvidenceKind::Validator,
                &serde_json::to_vec(&block).unwrap(),
            )
            .unwrap();
        let mut receipt: Value =
            serde_json::from_slice(&store.get(acquisition_receipt).unwrap()).unwrap();
        receipt["recent_blockhash_tail_anchor"]["previous_blockhash"] =
            block["result"]["previousBlockhash"].clone();
        for row in receipt["receipts"].as_array_mut().unwrap() {
            if row["method"] == "getBlock" && row["params"][0] == 448_760_809 {
                row["response_sha256"] = json!(tail_block.sha256);
            }
        }
        *acquisition_receipt = store
            .put(
                EvidenceKind::Validator,
                &serde_json::to_vec(&receipt).unwrap(),
            )
            .unwrap();
    } else {
        panic!("recent derivation type differs");
    }
    replace_derived(&mut record, &store, &evidence);
    reject(
        "tail_previous_blockhash_rehashed",
        &mut record,
        &store,
        "derived account content differs from reconstructed sources",
    );

    let mut record = control.clone();
    let mut evidence = derived(&record, &store, recent);
    if let AccountDerivationV2::RecentBlockhashesPreTargetV1 { event, .. } =
        &mut evidence.derivation
    {
        let mut frame = store.get(event).unwrap();
        frame[250] ^= 1;
        *event = store.put(EvidenceKind::Validator, &frame).unwrap();
    }
    replace_derived(&mut record, &store, &evidence);
    reject(
        "yellowstone_event_rehashed",
        &mut record,
        &store,
        "derived account content differs from reconstructed sources",
    );

    let mut record = control.clone();
    let mut evidence = derived(&record, &store, recent);
    if let AccountDerivationV2::RecentBlockhashesPreTargetV1 { tail_block, .. } =
        &mut evidence.derivation
    {
        tail_block.sha256 = "f".repeat(64);
    }
    replace_derived(&mut record, &store, &evidence);
    reject(
        "tail_block_source_removed",
        &mut record,
        &store,
        "missing evidence object",
    );

    let mut record = control.clone();
    let mut evidence = derived(&record, &store, recent);
    let event = match &evidence.derivation {
        AccountDerivationV2::RecentBlockhashesPreTargetV1 { event, .. } => event.clone(),
        _ => unreachable!(),
    };
    evidence.derivation = AccountDerivationV2::YellowstoneAccountImageV1 { event };
    replace_derived(&mut record, &store, &evidence);
    reject(
        "incompatible_derived_account_type",
        &mut record,
        &store,
        "direct-event derivation has unsupported runtime address",
    );

    let mut record = control.clone();
    let mut evidence = derived(&record, &store, slot_hashes);
    let mut content: AccountSnapshot =
        serde_json::from_slice(&store.get(&evidence.content).unwrap()).unwrap();
    content.data[100] ^= 1;
    evidence.content = store
        .put(
            EvidenceKind::AccountContent,
            &serde_json::to_vec(&content).unwrap(),
        )
        .unwrap();
    replace_derived(&mut record, &store, &evidence);
    reject(
        "slot_hashes_content_rehashed_but_source_unchanged",
        &mut record,
        &store,
        "derived account content differs from reconstructed sources",
    );

    let checkpoint: ObservedCheckpointV1 = serde_json::from_slice(
        &store
            .get(
                &control
                    .checkpointed_execution
                    .as_ref()
                    .unwrap()
                    .start_checkpoint,
            )
            .unwrap(),
    )
    .unwrap();
    let archived = checkpoint
        .accounts
        .iter()
        .find(|row| row.observation.kind == EvidenceKind::AccountObservation)
        .unwrap();
    let observation: Value =
        serde_json::from_slice(&store.get(&archived.observation).unwrap()).unwrap();
    let provider = serde_json::from_value(observation["provider"].clone()).unwrap();
    let error = AccountObservation::capture(
        &store,
        &archived.address,
        control.slot,
        AccountBoundary::BeforeTargetExecution,
        provider,
        b"{}",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("archived account cannot claim before-target execution"),
        "direct_archived_capture_before_target_is_rejected: {error}"
    );
    let error = AccountObservation::resolve(
        &store,
        &archived.observation,
        &archived.address,
        control.slot,
        AccountBoundary::BeforeTargetExecution,
        &control.genesis_hash,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("archived account cannot claim before-target execution"),
        "direct_archived_before_target_is_rejected: {error}"
    );
}

#[test]
fn reconstructive_closure_mutations_fail_closed() {
    let (control, _temp, store) = fixture();
    let mut record = control.clone();
    let mut proof = closure(&record, &store);
    proof.failed_overlaps.pop();
    replace_closure(&mut record, &store, &proof);
    reject(
        "failed_overlap_omitted",
        &mut record,
        &store,
        "failed output overlap lacks rollback evidence",
    );

    let mut record = control.clone();
    let mut proof = closure(&record, &store);
    let mut fake = proof.failed_overlaps[0].clone();
    fake.transaction_index = 0;
    proof.failed_overlaps.push(fake);
    replace_closure(&mut record, &store, &proof);
    reject(
        "fake_failed_overlap_added",
        &mut record,
        &store,
        "failed-overlap annotations do not match the block",
    );

    let mut record = control.clone();
    let mut proof = closure(&record, &store);
    let false_nonce = proof.validation_outputs[0].clone();
    proof.failed_overlaps[0].durable_nonce_accounts = vec![false_nonce.clone()];
    let mut census: Value =
        serde_json::from_slice(&store.get(&proof.conflict_census_evidence).unwrap()).unwrap();
    let index = proof.failed_overlaps[0].transaction_index as u64;
    let row = census["overlap_transactions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["transaction_index"] == index)
        .unwrap();
    row["durable_nonce_accounts"] = json!([false_nonce]);
    proof.conflict_census_evidence = store
        .put(
            EvidenceKind::Validator,
            &serde_json::to_vec(&census).unwrap(),
        )
        .unwrap();
    replace_closure(&mut record, &store, &proof);
    reject(
        "false_nonce_classification_added_to_census_and_annotation",
        &mut record,
        &store,
        "failed overlap nonce classification differs from transaction",
    );

    let mut record = control.clone();
    let mut proof = closure(&record, &store);
    let mut block: Value = serde_json::from_slice(
        &store
            .get(proof.full_block_evidence.as_ref().unwrap())
            .unwrap(),
    )
    .unwrap();
    let tx = &mut block["result"]["transactions"][proof.failed_overlaps[0].transaction_index];
    let keys = tx["transaction"]["message"]["accountKeys"]
        .as_array()
        .unwrap();
    let system_index = keys
        .iter()
        .position(|key| key == "11111111111111111111111111111111")
        .unwrap();
    tx["transaction"]["message"]["instructions"][0]["programIdIndex"] = json!(system_index);
    tx["transaction"]["message"]["instructions"][0]["data"] =
        json!(bs58::encode([4u8, 0, 0, 0]).into_string());
    tx["transaction"]["message"]["instructions"][0]["accounts"] = json!([1, 0, 0]);
    proof.full_block_evidence = Some(
        store
            .put(
                EvidenceKind::Validator,
                &serde_json::to_vec(&block).unwrap(),
            )
            .unwrap(),
    );
    replace_closure(&mut record, &store, &proof);
    reject(
        "real_nonce_omitted_from_census_synthetic_witness",
        &mut record,
        &store,
        "failed overlap nonce classification differs from transaction",
    );

    let mut record = control.clone();
    let mut proof = closure(&record, &store);
    proof.failed_overlaps[0].fee_payer = proof.validation_outputs[0].clone();
    let mut census: Value =
        serde_json::from_slice(&store.get(&proof.conflict_census_evidence).unwrap()).unwrap();
    let index = proof.failed_overlaps[0].transaction_index as u64;
    let row = census["overlap_transactions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["transaction_index"] == index)
        .unwrap();
    row["fee_payer"] = json!(proof.failed_overlaps[0].fee_payer);
    proof.conflict_census_evidence = store
        .put(
            EvidenceKind::Validator,
            &serde_json::to_vec(&census).unwrap(),
        )
        .unwrap();
    replace_closure(&mut record, &store, &proof);
    reject(
        "false_fee_payer_classification",
        &mut record,
        &store,
        "failed overlap transaction identity differs",
    );

    let mut record = control.clone();
    let mut proof = closure(&record, &store);
    proof.rollback_provenance = None;
    replace_closure(&mut record, &store, &proof);
    reject(
        "rollback_provenance_missing",
        &mut record,
        &store,
        "rollback semantics provenance missing",
    );

    let mut record = control.clone();
    let mut proof = closure(&record, &store);
    proof.rollback_provenance.as_mut().unwrap().upstream_commit = "changed".into();
    replace_closure(&mut record, &store, &proof);
    reject(
        "rollback_provenance_changed",
        &mut record,
        &store,
        "unsupported rollback semantics provenance",
    );

    let mut record = control.clone();
    let mut proof = closure(&record, &store);
    proof.full_block_evidence = Some(EvidenceRef {
        kind: EvidenceKind::Validator,
        sha256: "f".repeat(64),
    });
    replace_closure(&mut record, &store, &proof);
    reject(
        "full_transaction_evidence_missing",
        &mut record,
        &store,
        "missing evidence object",
    );
}
