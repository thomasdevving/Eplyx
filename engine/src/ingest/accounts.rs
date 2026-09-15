use crate::types::AccountSnapshot;
use anyhow::{Context, Result};
use base64::Engine;
use serde_json::Value;

pub fn normalize(value: &Value) -> Result<AccountSnapshot> {
    anyhow::ensure!(!value.is_null(), "required account missing");
    anyhow::ensure!(
        value["data"][1] == "base64",
        "account encoding must be base64"
    );
    let owner = value["owner"]
        .as_str()
        .context("missing account owner")?
        .to_string();
    owner
        .parse::<solana_address::Address>()
        .context("invalid owner")?;
    Ok(AccountSnapshot {
        owner,
        lamports: value["lamports"].as_u64().context("missing lamports")?,
        data: base64::prelude::BASE64_STANDARD
            .decode(value["data"][0].as_str().context("missing account data")?)?,
        executable: value["executable"]
            .as_bool()
            .context("missing executable")?,
        rent_epoch: value["rentEpoch"].as_u64().context("missing rent epoch")?,
    })
}
