//! Bounded experimental dependency capture through the existing resolver and
//! same-slot screen. Transport is supplied externally; no endpoint is read here.
use anyhow::{Context, Result};
use eplyx_engine::{
    dependencies,
    ingest::{rpc::RpcProvider, transactions::HistoricalTransaction},
    protocol::kamino::PROGRAM_ID,
    screening, versions,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{self, BufRead, Write},
    path::PathBuf,
};
struct Bridge;
impl RpcProvider for Bridge {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        println!(
            "{}",
            json!({"kind":"rpc_request","method":method,"params":params})
        );
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        let response: Value = serde_json::from_str(&line)?;
        anyhow::ensure!(
            response.get("error").is_none(),
            "retained transport failure"
        );
        Ok(response["result"].clone())
    }
}
fn main() -> Result<()> {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let input: Value = serde_json::from_str(&line)?;
    let tx: HistoricalTransaction = serde_json::from_value(input["transaction"].clone())?;
    let output = PathBuf::from(
        input["output"]
            .as_str()
            .context("missing output directory")?,
    );
    let pre_slot = tx.slot.checked_sub(1).context("slot has no predecessor")?;
    let rpc = Bridge;
    let genesis = rpc.call("getGenesisHash", json!([]))?;
    anyhow::ensure!(
        genesis.as_str() == Some("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"),
        "archive genesis differs"
    );
    let mut programs = Vec::new();
    let mut failed = None;
    for (program_id, discovered_by) in dependencies::discover(&tx, None, PROGRAM_ID) {
        match versions::resolve_program_at(&rpc,&program_id,pre_slot){
   Ok(versions::ProgramResolution::Native)=>programs.push(json!({"program_id":program_id,"source":"builtin","observed_slot":pre_slot,"discovered_by":discovered_by})),
   Ok(versions::ProgramResolution::Deployed(deployed))=>{
    std::fs::create_dir_all(output.join("binaries"))?;
    let file=format!("binaries/{program_id}.so");std::fs::write(output.join(&file),&deployed.elf)?;
    programs.push(json!({"program_id":program_id,"source":"historical_mainnet","provenance":deployed,"binary_file":file,"binary_bytes":deployed.elf.len(),"discovered_by":discovered_by}));
   },
   Err(e)=>{failed=Some(json!({"code":"dependency_binary_unavailable","program_id":program_id,"requested_slot":pre_slot,"detail":format!("{e:#}")}));break;}
  }
    }
    let mut result = json!({"kind":"experimental_envelope_dependency_capture","signature":tx.signature,"execution_slot":tx.slot,"pre_slot":pre_slot,"programs":programs,"C4_binaries_identified":failed.is_none(),"failure":failed,"slot_screening":null,"runtime_executed":false});
    if failed.is_none() {
        // Include every original account and upgradeable ProgramData key. The
        // latter detects same-slot binary upgrades outside message account metas.
        let mut required: BTreeSet<String> = tx
            .account_keys
            .iter()
            .map(|a| a.address.clone())
            .filter(|a| !a.starts_with("Sysvar"))
            .collect();
        for program in result["programs"].as_array().unwrap() {
            if let Some(address) = program["provenance"]["programdata_address"].as_str() {
                required.insert(address.into());
            }
        }
        match screening::screen(&rpc, tx.slot, &tx.signature, &required) {
            Ok(screen) => {
                if !screen.is_clean() {
                    result["failure"] = json!({"code":"dependency_state_unavailable","detail":"same-slot interference invalidates pre-transaction/post-transaction archive boundaries"});
                }
                result["slot_screening"] = serde_json::to_value(screen)?;
            }
            Err(e) => {
                result["failure"] = json!({"code":"dependency_state_unavailable","detail":format!("same-slot screening unavailable: {e:#}")})
            }
        }
    }
    // A clean screen is a prerequisite, not an account-state acquisition result.
    // This bounded experiment stops at binary capture and screening.
    result["historical_state_acquisition"] = json!("not_attempted");
    result["executable_transaction_constructed"] = json!(false);
    println!("{}", json!({"kind":"capture_result","result":result}));
    Ok(())
}
