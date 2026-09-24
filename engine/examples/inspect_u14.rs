use anyhow::Result;
use eplyx_engine::{
    corpus_store::CorpusStore,
    universal::{evidence::EvidenceStore, pipeline},
};
use serde_json::json;
use std::{fs, path::PathBuf};

fn main() -> Result<()> {
    let root = PathBuf::from("docs/examples/phase-u13-3-sequence-corpus");
    let record = CorpusStore::open(&root)?.load_v2()?.pop().unwrap();
    let resolved = record.resolve(&EvidenceStore::at(root.join("evidence")))?;
    let (baseline, _) = pipeline::baseline(&record, &resolved)?;
    let mut out = serde_json::Map::new();
    for (label, address) in [
        ("user", "JE9m89yHHiCGzzL2FAeeZgHKAFwjkW4Qp1GfjegWnojR"),
        (
            "perp_market",
            "7QAtMC3AaAc91W4XuwYXM1Mtffq9h9Z8dTxcJrKRHu1z",
        ),
        (
            "spot_market",
            "6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3",
        ),
    ] {
        let pre = resolved.seeds.get(address).unwrap();
        let post = baseline
            .post_accounts
            .get(address)
            .unwrap()
            .as_ref()
            .unwrap();
        out.insert(
            label.into(),
            json!({"address":address,"pre":pre,"post":post}),
        );
    }
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/u14-accounts.json".into());
    fs::write(&output, serde_json::to_vec_pretty(&out)?)?;
    println!("wrote {output}");
    Ok(())
}
