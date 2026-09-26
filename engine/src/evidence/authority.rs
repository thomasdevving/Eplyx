//! Classification from captured account bytes, never signing possession.
use crate::standard_programs::token::{self as decode, LEGACY_PROGRAM, TOKEN_2022_PROGRAM};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;
const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
/// JSON Pointer into an evidence result, carrying the actual observation context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub rpc_id: usize,
    pub pointer: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub slot: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityType {
    /// On-curve, existing non-executable System-owned account with empty data.
    /// This proves compatibility with a wallet authority, not a human or signer identity.
    WalletCompatible,
    /// Authority address has an account owned by a non-System runtime program.
    /// This does NOT identify the program that can sign for a PDA.
    ProgramOwnedAuthority,
    TokenMultisig,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthorityObservation {
    pub is_on_curve: bool,
    pub account_exists: bool,
    pub runtime_owner: Option<String>,
    pub executable: Option<bool>,
    pub multisig: Option<Value>,
}

pub fn classify_authority(
    owner: &str,
    raw: &Value,
) -> Result<(EntityType, AuthorityObservation, String)> {
    classify(owner, raw)
}

fn classify(owner: &str, raw: &Value) -> Result<(EntityType, AuthorityObservation, String)> {
    let address: Address = owner.parse().context("invalid owner authority address")?;
    let on_curve = address.is_on_curve();
    let mut observation = AuthorityObservation {
        is_on_curve: on_curve,
        account_exists: !raw.is_null(),
        runtime_owner: None,
        executable: None,
        multisig: None,
    };
    if raw.is_null() {
        return Ok((
            EntityType::Unknown,
            observation,
            "Authority account absent; address alone does not prove holder identity or control"
                .into(),
        ));
    }
    let program = raw["owner"]
        .as_str()
        .context("authority runtime owner missing")?;
    let _: Address = program.parse().context("invalid authority runtime owner")?;
    let bytes = decode::raw_account_bytes(raw).context("invalid authority account evidence")?;
    observation.runtime_owner = Some(program.into());
    observation.executable = raw["executable"].as_bool();
    ensure!(
        observation.executable.is_some(),
        "authority executable flag missing"
    );
    if observation.executable == Some(true) {
        return Ok((
            EntityType::Unknown,
            observation,
            "Authority address is an executable account; token signing control is unproven".into(),
        ));
    }
    if [LEGACY_PROGRAM, TOKEN_2022_PROGRAM].contains(&program)
        && bytes.len() == crate::standard_programs::spl_token::MULTISIG_LEN
    {
        let m = crate::standard_programs::spl_token::decode_multisig(&bytes)
            .ok()
            .context("malformed SPL multisig authority")?;
        ensure!(
            m.required_signers > 0 && m.required_signers <= m.signer_count && m.signer_count <= 11,
            "invalid multisig threshold"
        );
        observation.multisig = Some(
            json!({"required_signers":m.required_signers,"signer_count":m.signer_count,"signers":m.signers}),
        );
        return Ok((
            EntityType::TokenMultisig,
            observation,
            "Authority is an initialized SPL multisig account".into(),
        ));
    }
    if program == SYSTEM_PROGRAM && on_curve && bytes.is_empty() {
        return Ok((EntityType::WalletCompatible, observation, "On-curve authority with an existing empty System-owned account; no human identity inferred".into()));
    }
    if program != SYSTEM_PROGRAM {
        return Ok((EntityType::ProgramOwnedAuthority, observation, "Authority account has a non-System runtime owner; signing program and protocol role remain unknown".into()));
    }
    Ok((
        EntityType::Unknown,
        observation,
        "System-owned authority is off-curve or carries account data; control is unproven".into(),
    ))
}
