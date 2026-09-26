//! Shared captured account, loader and clock checks for offline paths.
pub use crate::standard_programs::token::{transfer_fee, CLOCK, UPGRADEABLE_LOADER};
use crate::{standard_programs::token as decode, types::NamedAccount};
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use solana_address::Address;
use solana_clock::Clock;
use solana_keypair::Keypair;
use solana_signer::Signer;
pub fn payer() -> Address {
    Keypair::new_from_array([198; 32]).pubkey()
}
pub(crate) fn slot(v: &Value) -> Result<u64> {
    v["context"]["slot"]
        .as_u64()
        .context("missing finalized RPC context")
}
pub(crate) fn programdata_address(raw: &Value) -> Result<Option<String>> {
    ensure!(
        raw["executable"] == true,
        "required program is not executable"
    );
    let owner = raw["owner"].as_str().context("missing program loader")?;
    let bytes = decode::raw_account_bytes(raw)?;
    if owner == UPGRADEABLE_LOADER {
        ensure!(
            crate::standard_programs::upgradeable_loader::decode_program(&bytes).is_ok(),
            "invalid upgradeable Program header"
        );
        Ok(Some(
            crate::standard_programs::upgradeable_loader::decode_program(&bytes)?.to_string(),
        ))
    } else {
        ensure!(
            [
                "BPFLoader2111111111111111111111111111111111",
                "BPFLoader1111111111111111111111111111111111"
            ]
            .contains(&owner),
            "unsupported executable loader"
        );
        ensure!(bytes.starts_with(b"\x7fELF"), "legacy program lacks ELF");
        Ok(None)
    }
}
pub(crate) fn clock(raw: &Value) -> Result<Clock> {
    ensure!(
        raw["owner"] == "Sysvar1111111111111111111111111111111111111" && raw["executable"] == false,
        "wrong Clock owner or executable flag"
    );
    Ok(
        crate::standard_programs::clock::CapturedClock::from_bytes(&decode::raw_account_bytes(
            raw,
        )?)?
        .clock(),
    )
}
pub(crate) fn captured_account(address: &str, raw: &Value) -> Result<NamedAccount> {
    let account = crate::ingest::accounts::normalize(raw)?;
    ensure!(
        account.data == decode::raw_account_bytes(raw)?,
        "captured account byte/space mismatch"
    );
    Ok(NamedAccount {
        label: address.into(),
        address: address.into(),
        account,
    })
}
/// Project measured byte ranges into the portable report record.
pub(crate) fn data_changes(
    address: &str,
    before: &[u8],
    after: &[u8],
) -> Vec<crate::path::ByteRangeDelta> {
    use crate::evidence::{
        field::{byte_ranges, FieldSchema},
        Provenance,
    };
    let provenance = Provenance {
        record: "captured-path-execution".into(),
        account_label: address.into(),
        address: Some(address.into()),
        decoder: FieldSchema::Opaque.decoder(),
        origin: None,
    };
    let mut records: Vec<crate::path::ByteRangeDelta> = Vec::new();
    for delta in byte_ranges(&provenance, before, after) {
        // STA displayed a changed common-prefix tail and an adjacent resize as
        // one range. This lossless coalescing keeps that recorded presentation.
        if let Some(last) = records.last_mut().filter(|last| {
            last.offset + last.before.len().max(last.after.len()) == delta.range.start
        }) {
            last.before.extend(delta.before);
            last.after.extend(delta.after);
        } else {
            records.push(crate::path::ByteRangeDelta {
                offset: delta.range.start,
                before: delta.before,
                after: delta.after,
            });
        }
    }
    records
}
