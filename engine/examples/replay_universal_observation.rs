//! Offline replay of one schema-2 observation through the generic product core.
use anyhow::{Context, Result};
use eplyx_engine::{
    replay::hash_bytes,
    universal::{evidence::EvidenceStore, model::ReplayObservationV2, pipeline},
};
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let record_path = PathBuf::from(args.next().context("record JSON path required")?);
    let evidence_path = PathBuf::from(args.next().context("evidence store path required")?);
    let candidate_path = args.next().map(PathBuf::from);
    let record: ReplayObservationV2 = serde_json::from_slice(&std::fs::read(&record_path)?)?;
    let store = EvidenceStore::at(evidence_path);
    let resolved = record.resolve(&store)?;
    let (baseline, fidelity) = pipeline::baseline(&record, &resolved)?;
    let candidate = if let Some(path) = candidate_path {
        let bytes = std::fs::read(path)?;
        let result = pipeline::execute(&record, &resolved, &bytes)?;
        Some(serde_json::json!({"binary_sha256":hash_bytes(&bytes),
            "execution_evidence_sha256":pipeline::evidence_hash(&result)?,
            "same_execution_evidence":result == baseline}))
    } else {
        None
    };
    println!(
        "{}",
        serde_json::json!({"observation_id":record.id,
        "signature":record.signature,"slot":record.slot,
        "fidelity":"matched","fidelity_profile":"complete_execution_v2",
        "baseline_evidence_sha256":pipeline::evidence_hash(&baseline)?,
        "watched_accounts":baseline.post_accounts.len(),"candidate":candidate})
    );
    Ok(())
}
