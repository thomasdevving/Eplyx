//! Deterministic state-shape classification and per-entity conversion eligibility.
//!
//! A state shape groups accounts by execution-relevant characteristics so that
//! tests can be prioritized and untested classes can be named. It is explicitly
//! not a risk score and explicitly not a proof equivalence class: the shape key
//! carries no balance, no address and no outcome, and nothing in this module ever
//! reads an execution result.
use super::current_select::Eligibility;
use super::population_types::{AuthorityResolution, StressEntity};
use crate::{
    canonical::digest, evidence::authority::EntityType, standard_programs::token::MintConfig,
};
fn type_key(t: &EntityType) -> String {
    format!("{t:?}")
}
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const CLASSIFIER_VERSION: &str = "eplyx-conversion-stress-shape/v1";

/// Execution-relevant characteristics of one account, plus the mint-level
/// conditions that govern whether a candidate burn and release can run at all.
///
/// Raw balance, balance bucket, account address and owner address are deliberately
/// absent: balance diversity is a separate selection dimension, and identity must
/// never enter a grouping key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeDimensions {
    pub authority_model: String,
    pub authority_resolved: bool,
    pub authority_on_curve: bool,
    pub authority_account_exists: bool,
    pub authority_runtime_owner: Option<String>,
    pub authority_executable: Option<bool>,
    pub account_initialized: bool,
    pub account_frozen: bool,
    pub delegate_present: bool,
    pub active_delegation: bool,
    pub close_authority_present: bool,
    pub account_extension_types: Vec<String>,
    pub mint_token_program: String,
    pub mint_paused: bool,
    pub mint_transfer_hook_active: bool,
    pub mint_transfer_fee_configured: bool,
    pub mint_default_account_state: Option<String>,
    pub mint_permanent_delegate: bool,
    pub mint_confidential_transfer: bool,
    pub mint_confidential_mint_burn: bool,
    pub mint_non_transferable: bool,
    pub mint_undecodable_extension: bool,
    /// Zero versus positive only. Exact amounts are never part of a shape.
    pub balance_positive: bool,
}

fn mint_extension<'a>(
    mint: &'a MintConfig,
    name: &str,
) -> Option<&'a crate::standard_programs::token::TokenExtension> {
    mint.extensions.iter().find(|e| e.extension_type == name)
}

pub fn dimensions(entity: &StressEntity, mint: &MintConfig) -> Result<ShapeDimensions> {
    let mut account_extension_types: Vec<String> = entity
        .state
        .extensions
        .iter()
        .map(|e| e.extension_type.clone())
        .collect();
    account_extension_types.sort();
    Ok(ShapeDimensions {
        authority_model: type_key(&entity.authority_model),
        authority_resolved: entity.authority_resolution == AuthorityResolution::Resolved,
        authority_on_curve: entity.authority_observation.is_on_curve,
        authority_account_exists: entity.authority_observation.account_exists,
        authority_runtime_owner: entity.authority_observation.runtime_owner.clone(),
        authority_executable: entity.authority_observation.executable,
        account_initialized: entity.state.is_initialized,
        account_frozen: entity.state.is_frozen,
        delegate_present: entity.state.delegate.is_some(),
        active_delegation: entity.state.has_active_delegate,
        close_authority_present: entity.state.close_authority.is_some(),
        account_extension_types,
        mint_token_program: mint.token_program.clone(),
        mint_paused: mint_extension(mint, "Pausable").is_some_and(|e| e.config["paused"] == true),
        // Presence of the extension is not activity: a hook is active only when a
        // program id is actually configured.
        mint_transfer_hook_active: mint_extension(mint, "TransferHook")
            .is_some_and(|e| !e.config["programId"].is_null()),
        mint_transfer_fee_configured: mint_extension(mint, "TransferFeeConfig").is_some(),
        mint_default_account_state: mint_extension(mint, "DefaultAccountState")
            .and_then(|e| e.config["state"].as_str().map(str::to_string)),
        mint_permanent_delegate: mint_extension(mint, "PermanentDelegate").is_some(),
        mint_confidential_transfer: mint_extension(mint, "ConfidentialTransferMint").is_some(),
        mint_confidential_mint_burn: mint_extension(mint, "ConfidentialMintBurn").is_some(),
        mint_non_transferable: mint_extension(mint, "NonTransferable").is_some(),
        mint_undecodable_extension: mint
            .extensions
            .iter()
            .any(|e| matches!(e.extension_type.as_str(), "Unrecognized" | "Malformed")),
        balance_positive: entity.state.raw_balance != "0",
    })
}

pub fn shape_key(d: &ShapeDimensions) -> Result<String> {
    digest(&(CLASSIFIER_VERSION, d))
}

/// A short human label derived from the same dimensions. Purely presentational;
/// it never introduces a judgment word that the dimensions do not support.
pub fn shape_label(d: &ShapeDimensions) -> String {
    let authority = match d.authority_model.as_str() {
        "WalletCompatible" => "Direct wallet authority",
        "ProgramOwnedAuthority" => "Program-controlled authority",
        "TokenMultisig" => "SPL multisig authority",
        _ if !d.authority_resolved => "Unresolved authority model",
        _ => "Unknown authority",
    };
    let mut parts = vec![authority.to_string()];
    parts.push(
        if !d.account_initialized {
            "uninitialized"
        } else if d.account_frozen {
            "frozen"
        } else {
            "initialized"
        }
        .to_string(),
    );
    if d.active_delegation {
        parts.push("active delegation".into());
    } else if d.delegate_present {
        parts.push("delegate set".into());
    }
    if d.close_authority_present {
        parts.push("close authority set".into());
    }
    if !d.account_extension_types.is_empty() {
        parts.push(format!(
            "account extensions: {}",
            d.account_extension_types.join(", ")
        ));
    }
    if d.mint_paused {
        parts.push("mint paused".into());
    }
    if d.mint_transfer_hook_active {
        parts.push("transfer hook active".into());
    }
    parts.push(if d.balance_positive {
        "positive balance".into()
    } else {
        "zero balance".to_string()
    });
    parts.join(" · ")
}

/// Whether the registered candidate conversion adapter can even attempt this
/// entity. These are executor and evidence boundaries, never claims that an
/// account cannot convert under its real mechanism.
pub fn eligibility(d: &ShapeDimensions) -> (Eligibility, String) {
    if d.mint_undecodable_extension
        || d.account_extension_types
            .iter()
            .any(|e| e == "Unrecognized" || e == "Malformed")
    {
        return (
            Eligibility::Unsupported,
            "Unknown or malformed extension; state cannot be evaluated.".into(),
        );
    }
    if !d.balance_positive {
        return (
            Eligibility::Invalid,
            "Zero observed public balance. A zero balance is not conversion exposure and is never selected; it is also not evidence that an encrypted balance is zero.".into(),
        );
    }
    if !d.account_initialized {
        return (
            Eligibility::Unsupported,
            "Uninitialized source token account. The candidate mechanism has no supported precondition for this state.".into(),
        );
    }
    if d.account_frozen {
        return (
            Eligibility::Unsupported,
            "Frozen source token account. The candidate burn cannot be attempted while the account is frozen, and no thaw authority is assumed.".into(),
        );
    }
    if d.account_extension_types
        .iter()
        .any(|e| e.contains("Confidential"))
    {
        return (
            Eligibility::Unsupported,
            "Confidential account state. Encrypted balances are outside this decoder and executor; the balance is unknown, not zero.".into(),
        );
    }
    if d.mint_paused {
        return (
            Eligibility::Unsupported,
            "The source mint is paused. No candidate burn can be attempted against a paused mint."
                .into(),
        );
    }
    if d.mint_confidential_mint_burn {
        return (
            Eligibility::Unsupported,
            "The source mint uses confidential mint/burn. The candidate burn path is unsupported for this configuration.".into(),
        );
    }
    if !d.authority_resolved {
        return (
            Eligibility::CaptureRequired,
            "This account's recorded authority was inside the population but outside the declared authority-lookup budget. Its authority model is unknown; resolving it requires further capture, and no signer is assumed meanwhile.".into(),
        );
    }
    match d.authority_model.as_str() {
        "WalletCompatible" => (
            Eligibility::ExecutableCandidate,
            "On-curve, existing, non-executable System-owned authority with empty data. The candidate conversion can be attempted under locally assumed holder signing; key possession remains unknown.".into(),
        ),
        "ProgramOwnedAuthority" => (
            Eligibility::Unsupported,
            "The recorded authority is owned by a non-System runtime program. The controlling program's signing path is not implemented, and no wallet signer is substituted for it.".into(),
        ),
        "TokenMultisig" => (
            Eligibility::Unsupported,
            "The recorded authority is an initialized SPL multisig. Threshold signing is not implemented by this executor.".into(),
        ),
        _ => (
            Eligibility::Unsupported,
            "The recorded authority is off-curve, absent, executable or carries account data. Control is unproven, so no signer is assumed.".into(),
        ),
    }
}

/// The one place that decides whether an entity may receive the locally assumed
/// holder signer. Keeping it here means a new case type cannot quietly grant it.
pub fn assumed_local_signer(model: &EntityType, resolution: AuthorityResolution) -> bool {
    resolution == AuthorityResolution::Resolved && *model == EntityType::WalletCompatible
}
