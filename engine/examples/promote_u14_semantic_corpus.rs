//! Publish a separate semantic view of the frozen U13.3 replay observation.
//! The old replay-only corpus and its evidence are never edited.
use anyhow::{ensure, Result};
use eplyx_engine::{
    corpus_store::CorpusStore,
    protocol::{drift::DriftSettlePnlAdapter, ProtocolAdapter},
};
use std::{fs, path::Path};

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let out = std::env::args()
        .nth(1)
        .expect("output corpus path required");
    let out = Path::new(&out);
    ensure!(!out.exists(), "semantic corpus output must not exist");
    let source = Path::new("docs/examples/phase-u13-3-sequence-corpus");
    copy_tree(&source.join("evidence"), &out.join("evidence"))?;
    let mut records = CorpusStore::open(source)?.load_v2()?;
    ensure!(records.len() == 1, "expected one frozen U13.3 witness");
    let mut record = records.pop().unwrap();
    ensure!(
        record.protocol == "unknown",
        "source corpus was already adapted"
    );
    record.protocol = DriftSettlePnlAdapter.name().into();
    record.id = record.identity()?;
    let store = CorpusStore::open(out)?;
    store.insert_v2(&record)?;
    let manifest = store.publish_v2()?;
    println!(
        "observation {} corpus {}",
        record.id, manifest.canonical_hash
    );
    Ok(())
}
