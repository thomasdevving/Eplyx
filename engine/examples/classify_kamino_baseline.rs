//! U3A read-only tooling: call the unchanged U2 engine on stored RPC evidence.
//! No historical account acquisition, message construction or execution.
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
    let raw = &envelope["result"];
    let mut row = json!({
        "signature": raw["transaction"]["signatures"][0],
        "slot": raw["slot"],
        "capture_sha256": hash_bytes(&bytes),
        "state_acquisition_attempted": false,
        "execution_attempted": false,
        "production_replay_validated": false,
    });
    match normalize(raw) {
        Err(error) => {
            row["normalization"] = json!("rejected");
            row["action_id"] = Value::Null;
            row["admission"] = json!("not_reached");
            row["replay_eligibility"] = json!("not_reached");
            row["first_blocker"] = json!({
                "class": "A", "stage": "normalization", "code": "normalization_error",
                "detail": format!("{error:#}")
            });
        }
        Ok(transaction) => {
            let adapter = adapter_for(kamino::PROGRAM_ID).context("Kamino adapter missing")?;
            let admission = adapter.accept(&transaction);
            row["normalization"] = json!("accepted");
            row["action_id"] = json!(adapter.action_id(&transaction));
            row["admission"] = json!(if admission.is_ok() {
                "accepted"
            } else {
                "rejected"
            });
            let eligibility = replay_eligibility(&transaction, kamino::PROGRAM_ID, None);
            row["replay_eligibility"] = json!(eligibility);
            row["first_blocker"] = match admission {
                Err(error) => json!({
                    "class": "B", "stage": "admission", "code": eligibility,
                    "detail": format!("{error:#}")
                }),
                Ok(()) => Value::Null,
            };
            row["normalized_top_level_instructions"] = json!(transaction.instructions);
        }
    }
    Ok(row)
}

fn main() -> Result<()> {
    let paths: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        !paths.is_empty(),
        "supply captured getTransaction JSON paths"
    );
    let rows = paths
        .iter()
        .map(|path| audit(path))
        .collect::<Result<Vec<_>>>()?;
    println!("{}", serde_json::to_string(&rows)?);
    Ok(())
}
