use eplyx_engine::{
    corpus_store::{CorpusStore, Insert},
    replay::ReplayRecord,
    universal::evidence::{EvidenceKind, EvidenceStore},
};
use std::sync::{Arc, Barrier};

#[test]
fn concurrent_conflicting_observations_have_exactly_one_winner() {
    let scratch = tempfile::tempdir().unwrap();
    let store = Arc::new(CorpusStore::open(scratch.path()).unwrap());
    let barrier = Arc::new(Barrier::new(8));
    let record: ReplayRecord = serde_json::from_str(include_str!(
        "../../docs/examples/mainnet-stake-pool-record.json"
    ))
    .unwrap();
    let threads: Vec<_> = (0..8)
        .map(|index| {
            let store = store.clone();
            let barrier = barrier.clone();
            let mut record = record.clone();
            record.transaction.slot += index;
            std::thread::spawn(move || {
                barrier.wait();
                (record.transaction.slot, store.insert(&record))
            })
        })
        .collect();
    let winners: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .filter_map(|(slot, result)| result.ok().map(|inserted| (slot, inserted)))
        .collect();
    assert_eq!(winners.len(), 1, "conflicting evidence was overwritten");
    assert_eq!(winners[0].1, Insert::Added);
    assert_eq!(store.load().unwrap()[0].transaction.slot, winners[0].0);
}

#[test]
fn concurrent_identical_evidence_is_idempotent_and_fully_published() {
    let scratch = tempfile::tempdir().unwrap();
    let store = Arc::new(EvidenceStore::at(scratch.path()));
    let barrier = Arc::new(Barrier::new(8));
    let bytes = Arc::new(vec![42; 8 * 1024 * 1024]);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            let barrier = barrier.clone();
            let bytes = bytes.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let reference = store.put(EvidenceKind::ProgramBinary, &bytes).unwrap();
                assert_eq!(store.get(&reference).unwrap(), *bytes);
                reference
            })
        })
        .collect();
    let references: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(references
        .iter()
        .all(|reference| reference == &references[0]));
}
