//! Offline experimental state checks through existing decoders and screening.
//! This does not admit replay records or execute transactions.
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use eplyx_engine::{
    ingest::rpc::RpcProvider,
    protocol::kamino::state,
    screening,
    standard_programs::{spl_token, token2022, Decoded},
};
use serde_json::{json, Value};
use std::{collections::BTreeSet, fmt::Debug, io::Read};

struct RetainedBlock(Value);
impl RpcProvider for RetainedBlock {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        anyhow::ensure!(
            method == "getBlock" && params == self.0["params"],
            "uncaptured request"
        );
        Ok(self.0["result"].clone())
    }
}
fn decoded<T: Debug>(value: Decoded<T>) -> Result<String> {
    match value {
        Decoded::Decoded(value) => Ok(format!("{value:?}")),
        other => anyhow::bail!("unproved layout: {other:?}"),
    }
}
fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let input: Value = serde_json::from_str(&input)?;
    if input["mode"] == "screen" {
        let required: BTreeSet<String> =
            serde_json::from_value(input["required_accounts"].clone())?;
        let screen = screening::screen(
            &RetainedBlock(input["block"].clone()),
            input["slot"].as_u64().context("slot")?,
            input["signature"].as_str().context("signature")?,
            &required,
        )?;
        screen.ensure_unambiguous()?;
        println!("{}", serde_json::to_string(&screen)?);
    } else {
        let mut rows = Vec::new();
        for item in input["accounts"].as_array().context("accounts")? {
            let data =
                STANDARD.decode(item["account"]["data"][0].as_str().context("base64 data")?)?;
            let kind = item["type"].as_str().context("type")?;
            let fields = match kind {
                "Reserve" => decoded(state::decode_reserve(&data))?,
                "Obligation" => decoded(state::decode_obligation(&data))?,
                "LendingMarket" => {
                    anyhow::ensure!(state::is_lending_market(&data), "invalid lending market");
                    "existing lending-market discriminator check; full raw bytes retained".into()
                }
                "TokenAccount" if item["account"]["owner"] == token2022::PROGRAM_ID => {
                    let value = token2022::decode_account(&data)
                        .ok()
                        .context("invalid Token-2022 account")?;
                    anyhow::ensure!(
                        value.extensions.truncated_at.is_none(),
                        "malformed Token-2022 extensions"
                    );
                    format!("{value:?}")
                }
                "Mint" if item["account"]["owner"] == token2022::PROGRAM_ID => {
                    let value = token2022::decode_mint(&data)
                        .ok()
                        .context("invalid Token-2022 mint")?;
                    anyhow::ensure!(
                        value.extensions.truncated_at.is_none(),
                        "malformed Token-2022 extensions"
                    );
                    format!("{value:?}")
                }
                "TokenAccount" => decoded(spl_token::decode_account(&data))?,
                "Mint" => decoded(spl_token::decode_mint(&data))?,
                _ => anyhow::bail!("unsupported account type {kind}"),
            };
            rows.push(json!({"address":item["address"],"boundary":item["boundary"],"type":kind,"fields":fields}));
        }
        println!(
            "{}",
            json!({"kind":"existing_decoder_validation","accounts":rows,"runtime_executed":false})
        );
    }
    Ok(())
}
