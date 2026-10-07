//! ProgramData preparation for the rollout rehearsal (Step 16B): the declared
//! preparation plan, the read-only capacity preflight, and the loader-v3
//! `ExtendProgram` message.
//!
//! Eplyx diagnoses a missing prerequisite; the declared plan decides whether
//! the rollout includes it. A computed minimum is presentation only and never
//! turns `none` into `extend_program`.
//!
//! Loader semantics are those of the pinned `solana-bpf-loader-program 4.2.2`
//! (`common_extend_program`, `check_authority = false`) under LiteSVM 0.16's
//! mainnet feature set:
//! - accounts: ProgramData (writable), Program (writable), System program,
//!   payer (writable, signer). No upgrade-authority signature is read.
//! - `additional_bytes` is a `u32`; zero is rejected; the new account length
//!   may not exceed `MAX_PERMITTED_DATA_LENGTH` (10 MiB); under SIMD-0431
//!   (`loader_v3_minimum_extend_program_size`, active) an extension must be at
//!   least 10,240 bytes unless it exactly fills the remaining headroom.
//! - the same-slot guard ("Program was extended in this block already")
//!   compares ProgramData's slot with Clock.slot; an immutable program is refused.
//! - rent: `max(1, Rent::minimum_balance(new_len)) - current lamports`, when
//!   positive, is transferred from the payer through a System CPI.
//! - the account is resized (new bytes zero), the existing bytes are redeployed
//!   and ProgramData's slot becomes Clock.slot, authority unchanged.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;
use solana_loader_v3_interface::{
    instruction::MINIMUM_EXTEND_PROGRAM_BYTES, state::UpgradeableLoaderState,
};
use solana_message::Message;

use crate::{change::ExecutableArtifact, standard_programs::upgradeable_loader as loader};

pub const PLAN_SCHEMA: u32 = 1;
/// `solana_system_interface::MAX_PERMITTED_DATA_LENGTH`, the bound the loader
/// checks the extended ProgramData length against.
pub const MAX_PERMITTED_DATA_LENGTH: u64 = solana_system_interface::MAX_PERMITTED_DATA_LENGTH;
/// Lamports of the assumed extension payer in S0.
pub const EXTENSION_PAYER_LAMPORTS: u64 = 10_000_000_000;

/// Assumed fee payer and rent funder of ExtendProgram.
pub fn extension_payer() -> String {
    super::simulated(84)
}

mod canonical_u64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<u64, D::Error> {
        use serde::de::Error;
        let text = String::deserialize(d)?;
        let value = text.parse::<u64>().map_err(D::Error::custom)?;
        if text != value.to_string() {
            return Err(D::Error::custom("canonical u64 decimal string required"));
        }
        Ok(value)
    }
}

/// The declared ProgramData preparation. Exactly one optional step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Preparation {
    None,
    ExtendProgram {
        #[serde(with = "canonical_u64")]
        additional_bytes: u64,
    },
}

impl Preparation {
    pub fn additional_bytes(&self) -> Option<u64> {
        match self {
            Self::None => None,
            Self::ExtendProgram { additional_bytes } => Some(*additional_bytes),
        }
    }
}

/// The small versioned plan document `eplyx rollout analyse --plan` reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema_version: u32,
    pub programdata_preparation: Preparation,
}

impl Plan {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let plan: Self = serde_json::from_slice(bytes)
            .map_err(|e| anyhow::anyhow!("invalid_preparation_plan: {e}"))?;
        // An internally tagged unit variant tolerates extra keys; the plan does
        // not. Exactly the declared kind's keys are accepted.
        let raw: Value = serde_json::from_slice(bytes)?;
        let mut keys: Vec<&str> = raw["programdata_preparation"]
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        keys.sort_unstable();
        let expected: &[&str] = match plan.programdata_preparation {
            Preparation::None => &["kind"],
            Preparation::ExtendProgram { .. } => &["additional_bytes", "kind"],
        };
        ensure!(
            keys == expected,
            "invalid_preparation_plan: programdata_preparation carries fields outside its kind"
        );
        ensure!(
            plan.schema_version == PLAN_SCHEMA,
            "invalid_preparation_plan: schema_version {} is not supported; this build reads {PLAN_SCHEMA}",
            plan.schema_version
        );
        Ok(plan)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightStatus {
    ReadyWithoutExtension,
    ExtensionRequired,
    DeclaredExtensionSufficient,
    DeclaredExtensionInsufficient,
    ExtensionUnsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Funding {
    #[serde(with = "crate::numfmt::u64_string")]
    pub programdata_lamports_before: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub minimum_balance_after: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub required_funding_lamports: u64,
    pub payer: String,
    pub payer_origin: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub payer_lamports: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub transaction_fee_lamports: u64,
    pub payer_covers_funding_and_fee: bool,
}

/// Facts derived from the retained world and the candidate alone. Setup
/// evidence: it never states that an extension executed or that a rollout is
/// acceptable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramDataPreflight {
    pub origin: String,
    pub loader: String,
    pub program_id: String,
    pub program_account_sha256: String,
    pub programdata_address: String,
    pub programdata_account_sha256: String,
    /// Account data length: header plus executable capacity.
    pub programdata_account_len: u64,
    /// `UpgradeableLoaderState::size_of_programdata_metadata()`.
    pub programdata_metadata_len: u64,
    /// Bytes after the header: what an Upgrade can write and the VM loads.
    pub executable_capacity_bytes: u64,
    /// The installed executable region (retained V1, padding included).
    pub current_executable: ExecutableArtifact,
    pub candidate: ExecutableArtifact,
    pub candidate_fits: bool,
    pub minimum_additional_bytes_required: u64,
    pub declared: Preparation,
    pub status: PreflightStatus,
    pub reason: Option<String>,
    pub extended_account_len: Option<u64>,
    pub extended_executable_capacity_bytes: Option<u64>,
    pub surplus_capacity_bytes: Option<u64>,
    pub shortfall_bytes: Option<u64>,
    pub funding: Option<Funding>,
    pub loader_rules: Value,
}

pub fn loader_rules() -> Value {
    json!({
        "implementation": "solana-bpf-loader-program 4.2.2 common_extend_program (check_authority = false)",
        "instruction": "UpgradeableLoaderInstruction::ExtendProgram { additional_bytes: u32 }",
        "accounts": ["ProgramData (writable)", "Program (writable)", "System program", "payer (writable, signer)"],
        "upgrade_authority_signature_required": false,
        "payer_signer_required_when_funding_needed": true,
        "minimum_extend_bytes": MINIMUM_EXTEND_PROGRAM_BYTES,
        "minimum_rule": "SIMD-0431 (active): additional_bytes >= minimum unless it equals the remaining headroom to the maximum",
        "max_permitted_data_length": MAX_PERMITTED_DATA_LENGTH,
        "zero_additional_bytes": "rejected (InvalidInstructionData)",
        "funding": "max(1, Rent::minimum_balance(new_len)) - current lamports, transferred from the payer by System CPI when positive",
        "post_state": "account resized with zero bytes, existing executable redeployed, ProgramData slot set to Clock.slot, authority unchanged",
        "same_slot": "refused when ProgramData's slot equals Clock.slot",
    })
}

pub fn rent_identity() -> Value {
    let rent = solana_rent::Rent::default();
    json!({
        "source": "solana-rent 4.3.0 Rent::default; equal to the LiteSVM 0.16 mainnet Rent sysvar",
        "lamports_per_byte": rent.lamports_per_byte,
        "account_storage_overhead": solana_rent::ACCOUNT_STORAGE_OVERHEAD,
        "minimum_balance": "max(1, (account_storage_overhead + data_len) * lamports_per_byte)",
    })
}

pub fn minimum_balance(len: u64) -> Option<u64> {
    solana_rent::Rent::default()
        .try_minimum_balance(usize::try_from(len).ok()?)
        .map(|b| b.max(1))
}

/// The capacity arithmetic, from the loader's own account format.
#[allow(clippy::too_many_arguments)]
pub fn preflight(
    program_id: &str,
    program_account_sha256: &str,
    programdata_address: &str,
    programdata: &crate::types::AccountSnapshot,
    candidate: &[u8],
    declared: &Preparation,
    payer_lamports: u64,
) -> Result<ProgramDataPreflight> {
    ensure!(
        programdata.owner == loader::id().to_string() && !programdata.executable,
        "ProgramData envelope is not loader-owned data"
    );
    let metadata = UpgradeableLoaderState::size_of_programdata_metadata() as u64;
    let decoded = loader::decode_programdata(&programdata.data)?;
    let account_len = programdata.data.len() as u64;
    let capacity = account_len
        .checked_sub(metadata)
        .context("ProgramData shorter than its header")?;
    let required = candidate.len() as u64;
    let missing = required.saturating_sub(capacity);
    let mut out = ProgramDataPreflight {
        origin: "derived_from_retained_world_and_candidate".into(),
        loader: loader::id().to_string(),
        program_id: program_id.into(),
        program_account_sha256: program_account_sha256.into(),
        programdata_address: programdata_address.into(),
        programdata_account_sha256: super::world::account_id(programdata)?,
        programdata_account_len: account_len,
        programdata_metadata_len: metadata,
        executable_capacity_bytes: capacity,
        current_executable: ExecutableArtifact::of(&decoded.bytes),
        candidate: ExecutableArtifact::of(candidate),
        candidate_fits: missing == 0,
        minimum_additional_bytes_required: missing,
        declared: declared.clone(),
        status: PreflightStatus::ReadyWithoutExtension,
        reason: None,
        extended_account_len: None,
        extended_executable_capacity_bytes: None,
        surplus_capacity_bytes: None,
        shortfall_bytes: None,
        funding: None,
        loader_rules: loader_rules(),
    };
    let unsupported = |out: &mut ProgramDataPreflight, reason: &str| {
        out.status = PreflightStatus::ExtensionUnsupported;
        out.reason = Some(reason.into());
    };
    match declared {
        Preparation::None => {
            if missing > 0 {
                out.status = PreflightStatus::ExtensionRequired;
                out.reason = Some("programdata_extension_required".into());
            }
        }
        Preparation::ExtendProgram { additional_bytes } => {
            let n = *additional_bytes;
            let headroom = MAX_PERMITTED_DATA_LENGTH.saturating_sub(account_len);
            let new_len = account_len.checked_add(n);
            if n == 0 {
                unsupported(&mut out, "zero_extension");
            } else if n > u64::from(u32::MAX)
                || new_len.is_none_or(|len| len > MAX_PERMITTED_DATA_LENGTH)
            {
                unsupported(&mut out, "programdata_size_overflow");
            } else if n < u64::from(MINIMUM_EXTEND_PROGRAM_BYTES) && n != headroom {
                unsupported(&mut out, "below_minimum_extension");
            } else {
                let new_len = account_len + n;
                let new_capacity = capacity + n;
                out.extended_account_len = Some(new_len);
                out.extended_executable_capacity_bytes = Some(new_capacity);
                if new_capacity >= required {
                    out.status = PreflightStatus::DeclaredExtensionSufficient;
                    out.surplus_capacity_bytes = Some(new_capacity - required);
                    if missing == 0 {
                        out.reason = Some("extension_not_required".into());
                    }
                } else {
                    out.status = PreflightStatus::DeclaredExtensionInsufficient;
                    out.reason = Some("declared_extension_insufficient".into());
                    out.shortfall_bytes = Some(required - new_capacity);
                }
                let min_after = minimum_balance(new_len).context("rent minimum")?;
                let funding = min_after.saturating_sub(programdata.lamports);
                let fee = 5_000;
                out.funding = Some(Funding {
                    programdata_lamports_before: programdata.lamports,
                    minimum_balance_after: min_after,
                    required_funding_lamports: funding,
                    payer: extension_payer(),
                    payer_origin: "assumed_simulation_only".into(),
                    payer_lamports,
                    transaction_fee_lamports: fee,
                    payer_covers_funding_and_fee: payer_lamports
                        >= funding.checked_add(fee).context("funding overflow")?,
                });
            }
        }
    }
    // The program's own deployment slot is not needed for capacity, but the
    // ProgramData must decode: an undecodable header is not a capacity fact.
    let _ = decoded.deploy_slot;
    Ok(out)
}

/// The official interface builder: ProgramData, Program, System program,
/// payer (signer). `None` when the declared amount is not a `u32`.
pub fn extend_message(program_id: &str, additional_bytes: u64) -> Result<Option<Message>> {
    let Ok(n) = u32::try_from(additional_bytes) else {
        return Ok(None);
    };
    let program: Address = program_id.parse()?;
    let payer: Address = extension_payer().parse()?;
    let instruction =
        solana_loader_v3_interface::instruction::extend_program(&program, Some(&payer), n);
    Ok(Some(Message::new(&[instruction], Some(&payer))))
}
