//! Read-only native-v0 proof derivation. Reads one JSON payload from stdin,
//! never imports RPC transport or environment configuration.
use anyhow::{Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use eplyx_engine::{
    message::{self, ArchiveProvenance, FrozenV0, HistoricalAccountEvidence},
    protocol::{kamino::KaminoKlendAdapter, ProtocolAdapter},
};
use serde_json::{json, Value};
use std::io::{self, Read};
fn main() -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let rows: Vec<Value> = serde_json::from_str(&input)?;
    let mut output = Vec::new();
    for row in rows {
        let frozen = FrozenV0::from_rpc(
            &row["result"],
            row["genesis"].as_str().context("missing genesis")?,
        )?;
        let mut evidence = Vec::new();
        let mut acquisition_failure = None;
        let mut slot_hashes = None;
        for raw in row["evidence"]
            .as_array()
            .context("missing evidence membership")?
        {
            let provider: ArchiveProvenance = serde_json::from_value(raw["provider"].clone())?;
            let body = BASE64_STANDARD.decode(
                raw["raw_response_base64"]
                    .as_str()
                    .context("missing raw historical envelope")?,
            )?;
            match HistoricalAccountEvidence::from_response(
                raw["pubkey"].as_str().context("missing LUT key")?,
                frozen.execution_slot(),
                provider,
                &body,
            ) {
                Ok(e) if e.pubkey == message::SLOT_HASHES_ID => slot_hashes = Some(e),
                Ok(e) => evidence.push(e),
                Err(e) => {
                    acquisition_failure = Some(message::LutProofFailure {
                        stage: 1,
                        detail: format!("historical response invalid: {e:#}"),
                    })
                }
            }
        }
        let result = if let Some(e) = acquisition_failure {
            Err(e)
        } else {
            message::reconstruct(&frozen, &evidence, slot_hashes.as_ref())
        };
        match result {
            Ok(proven) => {
                // Also enforce serialized-proof roundtrip, without trusting flags.
                let proof=serde_json::from_value(serde_json::to_value(proven.proof())?)?;
                let verified=message::validate_proof(&frozen,&evidence,slot_hashes.as_ref(),&proof).map_err(|e|anyhow::anyhow!("proof roundtrip failed: {}",e.detail))?;
                let next=KaminoKlendAdapter.accept_reconstructed_message(&verified).err().map(|e|e.to_string());
                output.push(json!({"signature":row["result"]["transaction"]["signatures"][0],"proof":proof,"failure":null,"passed_previous_lut_gate":true,"next_old_engine_rejection":next,"production_replay_eligible":false,"envelope":eplyx_engine::protocol::kamino::envelope::analyse(&verified),"normalized_transaction":verified.transaction()}));
            },
            Err(e)=>output.push(json!({"signature":row["result"]["transaction"]["signatures"][0],"proof":null,"failure":e,"passed_previous_lut_gate":false,"next_old_engine_rejection":null,"production_replay_eligible":false})),
        }
    }
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
