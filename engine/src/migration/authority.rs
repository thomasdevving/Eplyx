//! Holder authority paths: who must sign to migrate one exact source account.
//!
//! Eplyx never assumes it can migrate every holder. For each account it resolves
//! the actual token-account authority from captured state (owner wallet, SPL
//! multisig threshold, approved delegate, the mint's permanent delegate) against
//! what the specification allows. A path that is not available is classified with
//! a machine-readable reason; it is never replaced by a fabricated executable path.
//! Signatures are assumed locally in the VM; key possession always stays unknown.
use super::spec::{HolderAuthorization, OwnerAuthorityClass, TokenMigrationV1};
use crate::{
    evidence::authority::{classify_authority, EntityType},
    standard_programs::token::{MintConfig, TokenAccountState},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OwnerClass {
    /// On-curve, existing, empty System-owned account: compatible with a wallet.
    Wallet,
    /// An initialized SPL multisig account of the source token program.
    Multisig,
    /// Owned by a non-System program; only that program could authorize.
    ProgramControlled,
    /// Absent, executable, off-curve System account or not inspected.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum HolderAuthority {
    OwnerWallet {
        owner: String,
    },
    OwnerMultisig {
        owner: String,
        threshold: u8,
        signers: Vec<String>,
    },
    Delegate {
        delegate: String,
        delegated_amount_raw: String,
    },
    PermanentDelegate {
        delegate: String,
    },
}

impl HolderAuthority {
    /// The account passed as the token-program authority.
    pub fn authority(&self) -> &str {
        match self {
            Self::OwnerWallet { owner } | Self::OwnerMultisig { owner, .. } => owner,
            Self::Delegate { delegate, .. } | Self::PermanentDelegate { delegate } => delegate,
        }
    }
    pub fn kind(&self) -> RequiredAuthority {
        match self {
            Self::OwnerWallet { .. } => RequiredAuthority::HolderSignature,
            Self::OwnerMultisig { .. } => RequiredAuthority::MultisigThreshold,
            Self::Delegate { .. } => RequiredAuthority::DelegateSignature,
            Self::PermanentDelegate { .. } => RequiredAuthority::IssuerPermanentDelegate,
        }
    }
    /// Signers the transaction needs, in the order the adapter passes them.
    pub fn signers(&self) -> Vec<RequiredSigner> {
        match self {
            Self::OwnerWallet { owner } => {
                vec![RequiredSigner::new(SignerRole::HolderOwner, owner)]
            }
            Self::OwnerMultisig {
                threshold, signers, ..
            } => signers
                .iter()
                .take(usize::from(*threshold))
                .map(|s| RequiredSigner::new(SignerRole::MultisigMember, s))
                .collect(),
            Self::Delegate { delegate, .. } => {
                vec![RequiredSigner::new(SignerRole::Delegate, delegate)]
            }
            Self::PermanentDelegate { delegate } => {
                vec![RequiredSigner::new(SignerRole::PermanentDelegate, delegate)]
            }
        }
    }
}

/// The authority axis of impact classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RequiredAuthority {
    HolderSignature,
    MultisigThreshold,
    DelegateSignature,
    IssuerPermanentDelegate,
    OwningProgramInvocation,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SignerRole {
    HolderOwner,
    MultisigMember,
    Delegate,
    PermanentDelegate,
    ExternalMigrationAuthority,
    Relayer,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredSigner {
    pub role: SignerRole,
    pub address: String,
    /// Always "Unknown": Eplyx never holds or verifies private keys.
    pub key_possession: String,
}

impl RequiredSigner {
    pub fn new(role: SignerRole, address: &str) -> Self {
        Self {
            role,
            address: address.into(),
            key_possession: "Unknown".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", deny_unknown_fields)]
pub enum AuthorityPath {
    Available {
        owner_class: OwnerClass,
        authority: HolderAuthority,
        /// Other paths the captured state and specification would also permit.
        alternatives: Vec<HolderAuthority>,
    },
    Unavailable {
        owner_class: OwnerClass,
        required: RequiredAuthority,
        code: String,
        reason: String,
    },
}

impl AuthorityPath {
    pub fn required(&self) -> RequiredAuthority {
        match self {
            Self::Available { authority, .. } => authority.kind(),
            Self::Unavailable { required, .. } => *required,
        }
    }
    pub fn owner_class(&self) -> OwnerClass {
        match self {
            Self::Available { owner_class, .. } | Self::Unavailable { owner_class, .. } => {
                *owner_class
            }
        }
    }
}

fn permanent_delegate(mint: &MintConfig) -> Option<String> {
    mint.extensions
        .iter()
        .find(|e| e.extension_type == "PermanentDelegate")
        .and_then(|e| e.config["delegate"].as_str().map(str::to_string))
        .filter(|d| d != "11111111111111111111111111111111")
}

/// Resolve the authority path for migrating `amount` from one source account.
///
/// `owner_raw` is the captured owner account in RPC JSON shape (null when absent),
/// or `None` when the owner was never inspected.
pub fn resolve(
    spec: &TokenMigrationV1,
    source_program: &str,
    source_mint: &MintConfig,
    account: &TokenAccountState,
    owner_raw: Option<&Value>,
    amount: u64,
) -> Result<AuthorityPath> {
    let (owner_class, multisig) = match owner_raw {
        None => (OwnerClass::Unknown, None),
        Some(raw) => {
            let (entity, observation, _) = classify_authority(&account.owner, raw)?;
            match entity {
                EntityType::WalletCompatible => (OwnerClass::Wallet, None),
                EntityType::TokenMultisig
                    if observation.runtime_owner.as_deref() == Some(source_program) =>
                {
                    (OwnerClass::Multisig, observation.multisig)
                }
                // A multisig of the other token program cannot sign for this account.
                EntityType::TokenMultisig | EntityType::ProgramOwnedAuthority => {
                    (OwnerClass::ProgramControlled, None)
                }
                EntityType::Unknown => (OwnerClass::Unknown, None),
            }
        }
    };
    let mut paths = Vec::new();
    if spec.allows(HolderAuthorization::Owner) {
        match owner_class {
            OwnerClass::Wallet if spec.allows_owner_class(OwnerAuthorityClass::Wallet) => paths
                .push(HolderAuthority::OwnerWallet {
                    owner: account.owner.clone(),
                }),
            OwnerClass::Multisig if spec.allows_owner_class(OwnerAuthorityClass::Multisig) => {
                if let Some(m) = &multisig {
                    let threshold = m["required_signers"].as_u64().unwrap_or(0) as u8;
                    let signers: Vec<String> = m["signers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect();
                    if threshold > 0 && usize::from(threshold) <= signers.len() {
                        paths.push(HolderAuthority::OwnerMultisig {
                            owner: account.owner.clone(),
                            threshold,
                            signers,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    let delegated: u64 = account.delegated_amount.parse().unwrap_or(0);
    if spec.allows(HolderAuthorization::Delegate)
        && account.has_active_delegate
        && delegated >= amount
    {
        if let Some(delegate) = &account.delegate {
            paths.push(HolderAuthority::Delegate {
                delegate: delegate.clone(),
                delegated_amount_raw: account.delegated_amount.clone(),
            });
        }
    }
    if spec.allows(HolderAuthorization::PermanentDelegate) {
        if let Some(delegate) = permanent_delegate(source_mint) {
            paths.push(HolderAuthority::PermanentDelegate { delegate });
        }
    }
    if !paths.is_empty() {
        let authority = paths.remove(0);
        return Ok(AuthorityPath::Available {
            owner_class,
            authority,
            alternatives: paths,
        });
    }
    let partial_delegate = account.has_active_delegate && delegated < amount;
    let (required, code, reason) = match owner_class {
        OwnerClass::ProgramControlled => (
            RequiredAuthority::OwningProgramInvocation,
            "PROGRAM_CONTROLLED_OWNER",
            "The owner is controlled by a program; only that program could authorize this migration and the v1 mechanism has no path through it.".to_string(),
        ),
        OwnerClass::Multisig if !spec.allows_owner_class(OwnerAuthorityClass::Multisig) => (
            RequiredAuthority::MultisigThreshold,
            "OWNER_CLASS_NOT_ELIGIBLE",
            "The owner is an SPL multisig, which the specification does not include.".to_string(),
        ),
        OwnerClass::Wallet if !spec.allows_owner_class(OwnerAuthorityClass::Wallet) => (
            RequiredAuthority::HolderSignature,
            "OWNER_CLASS_NOT_ELIGIBLE",
            "The owner is a wallet, which the specification does not include.".to_string(),
        ),
        OwnerClass::Unknown => (
            RequiredAuthority::Unknown,
            "OWNER_UNVERIFIABLE",
            if owner_raw.is_none() {
                "The owner account was not inspected; control is unknown.".to_string()
            } else {
                "The owner has no verifiable wallet or multisig account; control is unproven.".to_string()
            },
        ),
        _ if partial_delegate => (
            RequiredAuthority::DelegateSignature,
            "DELEGATED_AMOUNT_INSUFFICIENT",
            format!("The approved delegate may move {delegated} raw, less than the full balance {amount}."),
        ),
        _ => (
            RequiredAuthority::HolderSignature,
            "AUTHORITY_PATH_UNAVAILABLE",
            "No authorization path the specification allows exists for this account.".to_string(),
        ),
    };
    Ok(AuthorityPath::Unavailable {
        owner_class,
        required,
        code: code.into(),
        reason,
    })
}
