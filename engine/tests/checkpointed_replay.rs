use std::path::PathBuf;

use eplyx_engine::universal::{
    evidence::{EvidenceKind, EvidenceRef, EvidenceStore},
    model::{FidelityProfile, ReplayObservationV2},
};

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/examples/phase-u11-checkpointed-corpus")
}

fn fixture() -> (ReplayObservationV2, EvidenceStore) {
    let root = corpus_root();
    let records = eplyx_engine::corpus_store::CorpusStore::open(&root)
        .unwrap()
        .load_v2()
        .unwrap();
    assert_eq!(
        records.len(),
        1,
        "U11 corpus must contain exactly one observation"
    );
    (records[0].clone(), EvidenceStore::at(root.join("evidence")))
}

fn reidentify(record: &mut ReplayObservationV2) {
    record.id = record.identity().unwrap();
}

fn assert_rejected(
    name: &str,
    record: &ReplayObservationV2,
    store: &EvidenceStore,
    expected: &str,
) {
    let error = match record.resolve(store) {
        Ok(_) => panic!("{name}: mutation unexpectedly resolved"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains(expected),
        "{name}: expected {expected:?}, got {error:?}"
    );
}

#[test]
fn checkpointed_profile_mutations_fail_closed() {
    let (control, store) = fixture();
    let resolved = control.resolve(&store).expect("control proof resolves");
    let (_, fidelity) = eplyx_engine::universal::pipeline::baseline(&control, &resolved)
        .expect("control replay matches");
    assert!(fidelity.matched(), "control replay must match");

    let mut record = control.clone();
    let proof = record.checkpointed_execution.as_mut().unwrap();
    proof.start_checkpoint = proof.terminal_checkpoint.clone();
    reidentify(&mut record);
    assert_rejected(
        "mutated_start_checkpoint_is_rejected",
        &record,
        &store,
        "checkpoint uses the wrong slot",
    );

    let mut record = control.clone();
    let proof = record.checkpointed_execution.as_mut().unwrap();
    proof.terminal_checkpoint = proof.start_checkpoint.clone();
    reidentify(&mut record);
    assert_rejected(
        "mutated_terminal_checkpoint_is_rejected",
        &record,
        &store,
        "checkpoint uses the wrong slot",
    );

    let mut record = control.clone();
    record
        .runtime
        .historical_evidence
        .as_mut()
        .unwrap()
        .environment_blockhash = "mutated-environment-blockhash".into();
    reidentify(&mut record);
    assert_rejected(
        "mutated_runtime_environment_is_rejected",
        &record,
        &store,
        "historical runtime evidence differs from CAS object",
    );

    let mut record = control.clone();
    let execution_reference = record
        .checkpointed_execution
        .as_ref()
        .unwrap()
        .deterministic_execution
        .clone();
    record
        .checkpointed_execution
        .as_mut()
        .unwrap()
        .closure_proof = execution_reference;
    reidentify(&mut record);
    assert_rejected(
        "mutated_closure_proof_is_rejected",
        &record,
        &store,
        "closure proof reference has wrong evidence category",
    );

    let mut record = control.clone();
    let eplyx_engine::universal::model::ExecutionInput::V0 { lookup_tables, .. } =
        &mut record.execution
    else {
        panic!("fixture must be native v0")
    };
    lookup_tables[0] = lookup_tables[1].clone();
    reidentify(&mut record);
    assert_rejected(
        "mutated_lut_evidence_is_rejected",
        &record,
        &store,
        "account address or historical boundary differs",
    );

    let mut record = control.clone();
    record.binaries[0].elf = record.binaries[1].elf.clone();
    reidentify(&mut record);
    assert_rejected(
        "mutated_historical_program_binary_is_rejected",
        &record,
        &store,
        "historical binary dependency identity differs",
    );

    let mut record = control.clone();
    record
        .checkpointed_execution
        .as_mut()
        .unwrap()
        .validation_outputs
        .pop();
    reidentify(&mut record);
    assert_rejected(
        "mutated_validation_output_set_is_rejected",
        &record,
        &store,
        "validation-output set differs from watched accounts",
    );

    let mut bypass = serde_json::to_value(&control).unwrap();
    bypass["matched"] = serde_json::Value::Bool(true);
    bypass["admitted"] = serde_json::Value::Bool(true);
    bypass["checkpoint_reconciled"] = serde_json::Value::Bool(true);
    bypass["checkpointed_execution"]["deterministic_execution"]["sha256"] =
        serde_json::Value::String("0".repeat(64));
    let mut record: ReplayObservationV2 = serde_json::from_value(bypass).unwrap();
    reidentify(&mut record);
    assert_rejected(
        "stored_success_booleans_cannot_bypass_revalidation",
        &record,
        &store,
        "missing evidence object",
    );

    let mut record = control.clone();
    record.fidelity_profile = FidelityProfile::CompleteExecutionV2;
    reidentify(&mut record);
    assert_rejected(
        "checkpointed_profile_cannot_be_replaced_with_complete_execution_v2",
        &record,
        &store,
        "complete execution fidelity cannot claim checkpoint-derived provenance",
    );

    let mut record = control;
    record
        .checkpointed_execution
        .as_mut()
        .unwrap()
        .deterministic_execution = EvidenceRef {
        kind: EvidenceKind::Execution,
        sha256: "f".repeat(64),
    };
    reidentify(&mut record);
    assert_rejected(
        "missing_checkpointed_evidence_object_is_rejected",
        &record,
        &store,
        "missing evidence object",
    );
}

#[test]
fn checkpointed_corpus_rejects_same_id_with_different_bytes() {
    let root = corpus_root();
    let corpus = eplyx_engine::corpus_store::CorpusStore::open(&root).unwrap();
    let record = corpus.load_v2().unwrap().pop().unwrap();
    let path = corpus.record_path(&record.id);
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        corpus.insert_v2(&record).unwrap(),
        eplyx_engine::corpus_store::Insert::AlreadyPresent
    );
    let mut conflicting = record;
    conflicting.protocol.push_str("-different-bytes");
    assert!(
        corpus.insert_v2(&conflicting).is_err(),
        "same observation ID with different bytes must be rejected"
    );
    assert_eq!(
        before,
        std::fs::read(path).unwrap(),
        "immutable insertion must leave the stored record untouched"
    );
}
