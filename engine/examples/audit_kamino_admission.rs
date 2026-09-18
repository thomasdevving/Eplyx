//! Read-only admission audit of captured RPC transactions against this engine.
//! This neither acquires historical state nor executes a replay.
use anyhow::{Context, Result};
use eplyx_engine::{
    discovery::replay_eligibility,
    ingest::transactions::normalize,
    protocol::{adapter_for, kamino},
    replay::hash_bytes,
};
use serde_json::{json, Value};

fn audit(path: &str) -> Result<Value> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    let envelope: Value = serde_json::from_slice(&bytes)?;
    let raw = envelope.get("result").unwrap_or(&envelope);
    let transaction = normalize(raw).with_context(|| format!("normalizing {path}"))?;
    let adapter = adapter_for(kamino::PROGRAM_ID).context("Kamino adapter missing")?;
    let admission = adapter.accept(&transaction);
    let companions: Vec<_> = transaction
        .instructions
        .iter()
        .enumerate()
        .filter(|(_, instruction)| instruction.program != kamino::PROGRAM_ID)
        .map(|(index, instruction)| {
            json!({
                "outer_index": index,
                "program": instruction.program,
                "data_hex": eplyx_engine::hexfmt::encode(&instruction.data),
                "accounts": instruction.accounts,
            })
        })
        .collect();
    Ok(json!({
        "signature": transaction.signature,
        "slot": transaction.slot,
        "action": adapter.action_id(&transaction),
        "message_version": transaction.version,
        "uses_lookup_tables": transaction.loaded_address_count != 0,
        "lookup_table_count": raw["transaction"]["message"]["addressTableLookups"]
            .as_array().map_or(0, Vec::len),
        "resolved_address_count": transaction.loaded_address_count,
        "standard_program_companions": companions,
        "current_rejection_code": replay_eligibility(&transaction, kamino::PROGRAM_ID, None),
        "current_rejection_detail": admission.as_ref().err().map(|error| format!("{error:#}")),
        "limitation_class": if admission.is_err() { "B: eligibility/admission" } else { "historical state not acquired" },
        "capture_sha256": hash_bytes(&bytes),
        "admitted": admission.is_ok(),
        "replay_eligible_production_evidence": false,
    }))
}

fn main() -> Result<()> {
    let paths: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        !paths.is_empty(),
        "supply captured getTransaction JSON paths"
    );
    let mut rows = paths
        .iter()
        .map(|path| audit(path))
        .collect::<Result<Vec<_>>>()?;
    rows.sort_by_key(|row| {
        (
            row["slot"].as_u64(),
            row["signature"].as_str().map(str::to_owned),
        )
    });
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}
