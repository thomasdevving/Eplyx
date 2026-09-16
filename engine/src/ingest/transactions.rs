//! Normalize legacy and v0 RPC JSON; reject missing resolution rather than guess.
use crate::types::{AccountMetaSpec, InstructionSpec};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One validator-observed SPL token balance, in message-key order.
///
/// This is the stateful analogue of `preBalances`/`postBalances`: for accounts
/// owned by a token program the validator records the exact base-unit amount on
/// both sides of the transaction, which is what lets an archived snapshot of a
/// token account be proved rather than merely trusted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBalance {
    pub account_index: usize,
    pub mint: String,
    pub program_id: String,
    /// Base units. Kept as a string exactly as the RPC reports it, because a
    /// u64 token amount does not survive a JSON double.
    pub amount: u64,
    pub decimals: u8,
}

/// One validator-observed cross-program invocation.
///
/// [`HistoricalTransaction::inner_instructions`] flattens every CPI group into
/// one list, which is enough to know *that* a transaction used CPI and enough
/// to discover which programs it needs. It is not enough to check that a replay
/// reproduced the same invocation graph: flattening drops the depth each call
/// ran at and which top-level instruction it belonged to. Those are kept here,
/// alongside the shape of the invoked instruction, so the original graph can be
/// compared against the replayed one as part of the fidelity gate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpiFrame {
    /// Index of the top-level instruction this call descends from.
    pub outer_index: u8,
    /// Invocation depth. A top-level instruction is 1, so CPI starts at 2.
    pub stack_height: u8,
    pub program: String,
    pub account_count: u8,
    pub data_len: u32,
    /// First instruction byte, which is the discriminant for every program in
    /// the supported contract. Absent for empty instruction data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discriminant: Option<u8>,
}

impl CpiFrame {
    pub fn new(
        outer_index: u8,
        stack_height: u8,
        program: String,
        accounts: usize,
        data: &[u8],
    ) -> Self {
        Self {
            outer_index,
            stack_height,
            program,
            account_count: u8::try_from(accounts).unwrap_or(u8::MAX),
            data_len: u32::try_from(data.len()).unwrap_or(u32::MAX),
            discriminant: data.first().copied(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalTransaction {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub version: String,
    pub recent_blockhash: String,
    pub payer: String,
    pub account_keys: Vec<AccountMetaSpec>,
    /// Addresses appended from address lookup tables during normalization.
    ///
    /// Zero means the message resolved no tables, so `account_keys` is exactly
    /// the static key list and the transaction executes identically to a legacy
    /// message. Defaults to zero so records written before this field existed -
    /// all of which are legacy - stay loadable and correct.
    #[serde(default)]
    pub loaded_address_count: usize,
    pub instructions: Vec<InstructionSpec>,
    pub inner_instructions: Vec<InstructionSpec>,
    /// Structured CPI frames for the same calls. Kept beside the flattened list
    /// rather than replacing it so records written before CPI replay existed
    /// stay loadable unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inner_instruction_frames: Vec<CpiFrame>,
    pub success: bool,
    pub error: Option<Value>,
    pub fee: u64,
    pub compute_units: Option<u64>,
    /// Validator-observed balances in message-key order. These are retained as
    /// independent evidence for historical account-provider qualification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_balances: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_balances: Option<Vec<u64>>,
    /// Validator-observed token balances. Absent for transactions that touch no
    /// token accounts; an empty vector and `None` therefore mean different
    /// things and are kept distinct.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_token_balances: Option<Vec<TokenBalance>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_token_balances: Option<Vec<TokenBalance>>,
    /// Half the sum of absolute native balance changes. This is a transaction
    /// activity signal, not protocol value or TVL. It is absent when an RPC
    /// response omits complete pre/post native balances.
    #[serde(default)]
    pub native_value_lamports: Option<u64>,
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
fn token_balances(value: &Value) -> Result<Option<Vec<TokenBalance>>> {
    let Some(entries) = value.as_array() else {
        return Ok(None);
    };
    entries
        .iter()
        .map(|entry| {
            Ok(TokenBalance {
                account_index: number(&entry["accountIndex"])?,
                mint: address(&entry["mint"])?,
                program_id: address(&entry["programId"])?,
                // The RPC reports the base-unit amount as a decimal string
                // precisely so it survives JSON; parse it rather than reading
                // the lossy `uiAmount` double alongside it.
                amount: entry["uiTokenAmount"]["amount"]
                    .as_str()
                    .context("missing token amount")?
                    .parse()
                    .context("invalid token amount")?,
                decimals: u8::try_from(number(&entry["uiTokenAmount"]["decimals"])?)
                    .context("invalid token decimals")?,
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
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
    let keys_before_lookups = keys.len();
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
    let mut frames = Vec::new();
    let mut complete = true;
    if let Some(groups) = meta["innerInstructions"].as_array() {
        for group in groups {
            let outer_index =
                u8::try_from(number(&group["index"])?).context("outer index too large")?;
            let decoded = instructions(&group["instructions"], &keys)?;
            let raw = group["instructions"]
                .as_array()
                .context("missing inner instructions")?;
            for (instruction, entry) in decoded.iter().zip(raw) {
                // A validator that omits stackHeight cannot be distinguished
                // from one reporting depth 0, so the absence is not defaulted:
                // the whole frame list is dropped, leaving the flattened view
                // intact for discovery and leaving any replay contract that
                // needs the graph to reject the transaction for the real reason.
                let Some(stack_height) = entry["stackHeight"]
                    .as_u64()
                    .and_then(|height| u8::try_from(height).ok())
                    .filter(|height| *height >= 2)
                else {
                    complete = false;
                    continue;
                };
                frames.push(CpiFrame::new(
                    outer_index,
                    stack_height,
                    instruction.program.clone(),
                    instruction.accounts.len(),
                    &instruction.data,
                ));
            }
            inner.extend(decoded);
        }
    }
    if !complete {
        frames.clear();
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
    let pre_balances = meta["preBalances"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| value.as_u64().context("invalid preBalance"))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    let post_balances = meta["postBalances"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| value.as_u64().context("invalid postBalance"))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    let native_value_lamports = match (&pre_balances, &post_balances) {
        (Some(pre), Some(post)) if pre.len() == keys.len() && post.len() == keys.len() => {
            let movement = pre.iter().zip(post).fold(0_u128, |sum, (before, after)| {
                sum + (*before as u128).abs_diff(*after as u128)
            });
            // Every transfer normally appears once as a debit and once as a
            // credit. Fees and account creation make this only a structural
            // magnitude signal, so no economic semantics are attached to it.
            Some(u64::try_from(movement / 2).context("native balance movement overflow")?)
        }
        _ => None,
    };
    Ok(HistoricalTransaction {
        signature,
        slot: value["slot"].as_u64().context("missing slot")?,
        block_time: value["blockTime"].as_i64(),
        version: version.into(),
        loaded_address_count: keys.len() - keys_before_lookups,
        recent_blockhash: message["recentBlockhash"]
            .as_str()
            .context("missing blockhash")?
            .into(),
        payer: keys[0].address.clone(),
        account_keys: keys,
        instructions: outer,
        inner_instructions: inner,
        inner_instruction_frames: frames,
        success: meta["err"].is_null(),
        error: if meta["err"].is_null() {
            None
        } else {
            Some(meta["err"].clone())
        },
        fee: meta["fee"].as_u64().context("missing fee")?,
        compute_units: meta["computeUnitsConsumed"].as_u64(),
        pre_balances,
        post_balances,
        pre_token_balances: token_balances(&meta["preTokenBalances"])?,
        post_token_balances: token_balances(&meta["postTokenBalances"])?,
        native_value_lamports,
        logs: serde_json::from_value(meta["logMessages"].clone()).unwrap_or_default(),
    })
}
