//! Normalize legacy and v0 RPC JSON; reject missing resolution rather than guess.
use crate::types::{AccountMetaSpec, InstructionSpec};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalTransaction {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub version: String,
    pub recent_blockhash: String,
    pub payer: String,
    pub account_keys: Vec<AccountMetaSpec>,
    pub instructions: Vec<InstructionSpec>,
    pub inner_instructions: Vec<InstructionSpec>,
    pub success: bool,
    pub error: Option<Value>,
    pub fee: u64,
    pub compute_units: Option<u64>,
    pub logs: Vec<String>,
}
fn number(value: &Value) -> Result<usize> {
    usize::try_from(
        value
            .as_u64()
            .context("missing unsigned transaction field")?,
    )
    .context("index too large")
}
fn address(value: &Value) -> Result<String> {
    let text = value
        .as_str()
        .context("account key must be base58 string (use encoding=json)")?;
    text.parse::<solana_address::Address>()
        .context("invalid account key")?;
    Ok(text.into())
}
fn instructions(value: &Value, keys: &[AccountMetaSpec]) -> Result<Vec<InstructionSpec>> {
    value
        .as_array()
        .context("missing instructions")?
        .iter()
        .map(|ix| {
            let program = keys
                .get(number(&ix["programIdIndex"])?)
                .context("program index out of range")?
                .address
                .clone();
            let accounts = ix["accounts"]
                .as_array()
                .context("missing account indices")?
                .iter()
                .map(|i| {
                    keys.get(number(i)?)
                        .cloned()
                        .context("account index out of range")
                })
                .collect::<Result<_>>()?;
            let data = bs58::decode(ix["data"].as_str().context("missing instruction data")?)
                .into_vec()
                .context("invalid base58 data")?;
            Ok(InstructionSpec {
                program,
                accounts,
                data,
            })
        })
        .collect()
}
pub fn normalize(value: &Value) -> Result<HistoricalTransaction> {
    anyhow::ensure!(
        !value.is_null(),
        "transaction unavailable: archive RPC may be required"
    );
    let message = &value["transaction"]["message"];
    let meta = &value["meta"];
    anyhow::ensure!(!meta.is_null(), "transaction metadata unavailable");
    anyhow::ensure!(
        meta.get("err").is_some(),
        "transaction metadata missing outcome"
    );
    let version = match &value["version"] {
        Value::Null => "legacy",
        Value::String(s) if s == "legacy" => "legacy",
        Value::Number(n) if n.as_u64() == Some(0) => "v0",
        _ => anyhow::bail!("unsupported transaction version"),
    };
    let static_keys = message["accountKeys"]
        .as_array()
        .context("missing account keys")?;
    let signed = number(&message["header"]["numRequiredSignatures"])?;
    let ro_signed = number(&message["header"]["numReadonlySignedAccounts"])?;
    let ro_unsigned = number(&message["header"]["numReadonlyUnsignedAccounts"])?;
    anyhow::ensure!(
        signed > 0
            && signed <= static_keys.len()
            && ro_signed < signed
            && ro_unsigned <= static_keys.len() - signed,
        "invalid message header"
    );
    let mut keys = Vec::new();
    for (i, key) in static_keys.iter().enumerate() {
        keys.push(AccountMetaSpec {
            address: address(key)?,
            is_signer: i < signed,
            is_writable: if i < signed {
                i < signed - ro_signed
            } else {
                i < static_keys.len() - ro_unsigned
            },
        });
    }
    if version == "v0" {
        let lookups = message["addressTableLookups"]
            .as_array()
            .context("missing address table lookups")?;
        for (field, writable, index_field) in [
            ("writable", true, "writableIndexes"),
            ("readonly", false, "readonlyIndexes"),
        ] {
            let expected = lookups
                .iter()
                .map(|l| {
                    l[index_field]
                        .as_array()
                        .map(Vec::len)
                        .context("missing lookup indices")
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .sum::<usize>();
            let loaded = meta["loadedAddresses"][field]
                .as_array()
                .context("v0 requires resolved loadedAddresses")?;
            anyhow::ensure!(
                loaded.len() == expected,
                "incomplete lookup table resolution"
            );
            for key in loaded {
                keys.push(AccountMetaSpec {
                    address: address(key)?,
                    is_signer: false,
                    is_writable: writable,
                });
            }
        }
    }
    let outer = instructions(&message["instructions"], &keys)?;
    let mut inner = Vec::new();
    if let Some(groups) = meta["innerInstructions"].as_array() {
        for group in groups {
            inner.extend(instructions(&group["instructions"], &keys)?);
        }
    }
    let signatures = value["transaction"]["signatures"]
        .as_array()
        .context("missing signatures")?;
    anyhow::ensure!(
        signatures.len() == signed,
        "signature count differs from header"
    );
    let signature = signatures[0]
        .as_str()
        .context("missing signature")?
        .to_string();
    anyhow::ensure!(
        bs58::decode(&signature).into_vec()?.len() == 64,
        "invalid signature length"
    );
    Ok(HistoricalTransaction {
        signature,
        slot: value["slot"].as_u64().context("missing slot")?,
        block_time: value["blockTime"].as_i64(),
        version: version.into(),
        recent_blockhash: message["recentBlockhash"]
            .as_str()
            .context("missing blockhash")?
            .into(),
        payer: keys[0].address.clone(),
        account_keys: keys,
        instructions: outer,
        inner_instructions: inner,
        success: meta["err"].is_null(),
        error: if meta["err"].is_null() {
            None
        } else {
            Some(meta["err"].clone())
        },
        fee: meta["fee"].as_u64().context("missing fee")?,
        compute_units: meta["computeUnitsConsumed"].as_u64(),
        logs: serde_json::from_value(meta["logMessages"].clone()).unwrap_or_default(),
    })
}
