//! Official LUT decoder used only to plan conditional SlotHashes acquisition.
use anyhow::{Context, Result};
use eplyx_engine::ingest::accounts;
use solana_address_lookup_table_interface::{program, state::AddressLookupTable};
use std::io::{self, Read};
fn main() -> Result<()> {
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes)?;
    let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
    let account = accounts::normalize(&raw["result"]["value"])?;
    anyhow::ensure!(
        account.owner == program::id().to_string(),
        "wrong LUT owner"
    );
    let table = AddressLookupTable::deserialize(&account.data)
        .ok()
        .context("official LUT decode failed")?;
    println!(
        "{}",
        serde_json::json!({"metadata":table.meta,"address_count":table.addresses.len()})
    );
    Ok(())
}
