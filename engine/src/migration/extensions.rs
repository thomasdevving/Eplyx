//! Token-2022 extension inventory and the deliberate migration support matrix.
//!
//! Every extension on a relevant mint or token account is inventoried and placed in
//! exactly one support class for the operation the migration performs on it (burn
//! or escrow on the source side, reserve transfer or mint-to on the destination
//! side). Nothing is silently executed with legacy semantics: an extension that
//! changes transfer, burn or mint behavior is either modeled explicitly or kept out
//! of executed evidence. The classifications are checked against the pinned
//! Token-2022 program in the VM test suite (`engine/tests/migration_extensions.rs`).
use super::spec::{DestinationFunding, RatioBasis, SourceDisposition, TokenMigrationV1};
use crate::standard_programs::token::{MintConfig, TokenAccountState, TokenExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MATRIX_VERSION: &str = "eplyx-token-2022-migration-matrix/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Support {
    /// The extension does not change the migration's token semantics.
    Supported,
    /// Executable, with semantics the report models explicitly (for example a
    /// transfer fee withheld at the recipient).
    SupportedWithSpecialSemantics,
    /// The v1 mechanism cannot perform the operation under this extension/state.
    Unsupported,
    /// Executable only with protocol- or issuer-specific handling v1 does not have
    /// (extra hook accounts, a memo instruction, an extra burn authority, a thaw).
    RequiresProtocolSpecificHandling,
    /// The relevant state cannot be verified from public data (encrypted balances).
    Unverifiable,
}

impl Support {
    /// Whether an account/state with this finding may enter executed evidence.
    pub fn executable(self) -> bool {
        matches!(self, Self::Supported | Self::SupportedWithSpecialSemantics)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Side {
    Source,
    Destination,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    Mint,
    Account,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionFinding {
    pub side: Side,
    pub scope: Scope,
    pub extension: String,
    pub type_id: u16,
    pub operation: String,
    pub support: Support,
    /// Stable machine-readable code, e.g. `CPI_GUARD_ENABLED`.
    pub code: String,
    pub semantics: String,
}

fn operation(spec: &TokenMigrationV1, side: Side) -> &'static str {
    match side {
        Side::Source => match spec.source_disposition {
            SourceDisposition::Burn => "burn",
            SourceDisposition::Escrow => "escrow-transfer",
        },
        Side::Destination => match spec.destination_funding {
            DestinationFunding::ReserveTransfer { .. } => "reserve-transfer",
            DestinationFunding::MintTo => "mint-to",
        },
    }
}

fn truthy(config: &Value, field: &str) -> bool {
    config[field] == true
}

fn finding(
    side: Side,
    scope: Scope,
    extension: &TokenExtension,
    operation: &str,
    support: Support,
    code: &str,
    semantics: impl Into<String>,
) -> ExtensionFinding {
    ExtensionFinding {
        side,
        scope,
        extension: extension.extension_type.clone(),
        type_id: extension.type_id,
        operation: operation.into(),
        support,
        code: code.into(),
        semantics: semantics.into(),
    }
}

/// Classify every extension of a mint for the operation performed on that side.
pub fn classify_mint(
    spec: &TokenMigrationV1,
    side: Side,
    mint: &MintConfig,
) -> Vec<ExtensionFinding> {
    let op = operation(spec, side);
    let transfer = matches!(op, "escrow-transfer" | "reserve-transfer");
    let mint_to = op == "mint-to";
    let burn = op == "burn";
    mint.extensions
        .iter()
        .map(|e| {
            let f = |support, code: &str, semantics: String| {
                finding(side, Scope::Mint, e, op, support, code, semantics)
            };
            use Support::*;
            match e.extension_type.as_str() {
                "TransferFeeConfig" if transfer => f(
                    SupportedWithSpecialSemantics,
                    "TRANSFER_FEE_WITHHELD",
                    "The recipient is credited net of the Token-2022 transfer fee; the fee is withheld in the recipient account and reported separately from the migration fee.".into(),
                ),
                "TransferFeeConfig" => f(Supported, "TRANSFER_FEE_NOT_APPLICABLE", format!("A {op} is not subject to Token-2022 transfer fees.")),
                "MintCloseAuthority" => f(Supported, "MINT_CLOSE_AUTHORITY", "The close authority is reported as an authority; it does not change migration semantics.".into()),
                "ConfidentialTransferMint" | "ConfidentialTransferFeeConfig" => f(
                    SupportedWithSpecialSemantics,
                    "PUBLIC_BALANCES_ONLY",
                    "Only public balances migrate. Confidential balances are encrypted, are not migrated and remain unverifiable.".into(),
                ),
                "ConfidentialMintBurn" => f(
                    if side == Side::Source || mint_to { Unsupported } else { SupportedWithSpecialSemantics },
                    "CONFIDENTIAL_SUPPLY",
                    "Confidential mint/burn makes supply partly encrypted; supply reconciliation cannot be verified for burns or mints.".into(),
                ),
                "DefaultAccountState" => {
                    let frozen = e.config["state"] == "Frozen";
                    match (side, frozen) {
                        (Side::Destination, true) => f(
                            SupportedWithSpecialSemantics,
                            "DEFAULT_FROZEN_DESTINATION",
                            "New destination accounts are created frozen; existing unfrozen destinations can be credited, missing ones need a freeze-authority thaw before they can receive tokens.".into(),
                        ),
                        (Side::Source, true) if op == "escrow-transfer" => f(
                            RequiresProtocolSpecificHandling,
                            "DEFAULT_FROZEN_ESCROW",
                            "A newly created escrow vault would be frozen; the freeze authority must thaw it before rollout.".into(),
                        ),
                        _ => f(Supported, "DEFAULT_ACCOUNT_STATE", "The default state only affects newly created accounts.".into()),
                    }
                }
                "NonTransferable" if transfer => f(
                    Unsupported,
                    "NON_TRANSFERABLE",
                    format!("Non-transferable tokens cannot be moved by a {op}."),
                ),
                "NonTransferable" if burn => f(Supported, "NON_TRANSFERABLE_BURN", "Burning non-transferable tokens is permitted.".into()),
                "NonTransferable" => f(SupportedWithSpecialSemantics, "NON_TRANSFERABLE_DESTINATION", "Holders receive non-transferable destination tokens.".into()),
                "InterestBearingConfig" | "ScaledUiAmount" => {
                    if spec.conversion.ratio_basis == RatioBasis::Ui {
                        f(Unsupported, "SCALED_UI_RATIO_AMBIGUOUS", "A ui-basis ratio is ambiguous when displayed amounts are scaled over time; use a raw-basis ratio.".into())
                    } else {
                        f(SupportedWithSpecialSemantics, "RAW_AMOUNTS_MIGRATE", "Raw amounts migrate exactly; displayed UI amounts are scaled and differ from raw amounts.".into())
                    }
                }
                "PermanentDelegate" => f(
                    SupportedWithSpecialSemantics,
                    "PERMANENT_DELEGATE_AUTHORITY",
                    if side == Side::Source {
                        "The permanent delegate can burn or move any holder balance. It is reported as an authority and authorizes migrations only if the specification allows permanentDelegate.".into()
                    } else {
                        "The destination permanent delegate can move migrated balances; reported as an authority risk.".into()
                    },
                ),
                "TransferHook" => {
                    let active = !e.config["programId"].is_null();
                    if transfer && active {
                        f(RequiresProtocolSpecificHandling, "TRANSFER_HOOK_ACCOUNTS_REQUIRED", format!("A {op} invokes the transfer-hook program and needs its extra account metas; the v1 ABI forwards none."))
                    } else {
                        f(Supported, "TRANSFER_HOOK_NOT_INVOKED", format!("A {op} does not invoke the transfer hook{}.", if active { "" } else { " (no hook program is set)" }))
                    }
                }
                "MetadataPointer" | "TokenMetadata" | "GroupPointer" | "TokenGroup"
                | "GroupMemberPointer" | "TokenGroupMember" => {
                    f(Supported, "METADATA_ONLY", "Metadata does not change migration semantics.".into())
                }
                "Pausable" => {
                    if truthy(&e.config, "paused") {
                        f(Unsupported, "MINT_PAUSED", format!("The mint is paused; a {op} fails until the pause authority resumes it."))
                    } else {
                        f(SupportedWithSpecialSemantics, "PAUSE_AUTHORITY", "The pause authority can halt the migration at any time; reported as an authority.".into())
                    }
                }
                "PermissionedBurn" if burn => f(
                    RequiresProtocolSpecificHandling,
                    "PERMISSIONED_BURN",
                    "Burns require the permissioned-burn authority as an additional signer; the v1 ABI does not forward it.".into(),
                ),
                "PermissionedBurn" => f(Supported, "PERMISSIONED_BURN_NOT_INVOKED", format!("A {op} does not burn.")),
                other => f(Unverifiable, "UNKNOWN_EXTENSION", format!("Extension {other} has no migration classification.")),
            }
        })
        .collect()
}

/// Classify every extension of a token account. The source account is debited;
/// the destination account is credited.
pub fn classify_account(
    spec: &TokenMigrationV1,
    side: Side,
    account: &TokenAccountState,
    holder_is_owner: bool,
) -> Vec<ExtensionFinding> {
    let op = operation(spec, side);
    account
        .extensions
        .iter()
        .map(|e| {
            let f = |support, code: &str, semantics: String| {
                finding(side, Scope::Account, e, op, support, code, semantics)
            };
            use Support::*;
            match (side, e.extension_type.as_str()) {
                (_, "TransferFeeAmount") => {
                    let withheld = e.config["withheldAmount"].as_str().unwrap_or("0").to_string();
                    if side == Side::Source && withheld != "0" {
                        f(SupportedWithSpecialSemantics, "WITHHELD_FEES_NOT_MIGRATED", format!("{withheld} raw withheld transfer fees stay in the source account; they are not holder balance and are not migrated."))
                    } else {
                        f(Supported, "WITHHELD_FEE_AMOUNT", "Withheld-fee bookkeeping only.".into())
                    }
                }
                (_, "ImmutableOwner" | "TransferHookAccount" | "PausableAccount") => {
                    f(Supported, "ACCOUNT_MARKER", "Account marker; no migration effect.".into())
                }
                (Side::Source, "MemoTransfer") => f(Supported, "MEMO_OUTGOING_NOT_REQUIRED", "Required memos apply to incoming transfers only.".into()),
                (Side::Destination, "MemoTransfer") => {
                    if truthy(&e.config, "requireIncomingTransferMemos") && op == "reserve-transfer" {
                        f(RequiresProtocolSpecificHandling, "MEMO_REQUIRED", "The destination requires a memo on incoming transfers; the v1 ABI emits none.".into())
                    } else {
                        f(Supported, "MEMO_NOT_REQUIRED", format!("A {op} into this account needs no memo."))
                    }
                }
                (Side::Source, "CpiGuard") => {
                    if truthy(&e.config, "lockCpi") && holder_is_owner {
                        f(Unsupported, "CPI_GUARD_ENABLED", "CPI Guard blocks owner-authorized burns and transfers made through a program; the holder must disable it or approve a delegate.".into())
                    } else {
                        f(Supported, "CPI_GUARD_INACTIVE", "CPI Guard does not restrict this authorization path.".into())
                    }
                }
                (Side::Destination, "CpiGuard") => f(Supported, "CPI_GUARD_RECEIVE", "CPI Guard does not restrict receiving tokens.".into()),
                (Side::Source, "NonTransferableAccount") if op == "escrow-transfer" => f(Unsupported, "NON_TRANSFERABLE", "Non-transferable balances cannot be moved into escrow.".into()),
                (_, "NonTransferableAccount") => f(Supported, "NON_TRANSFERABLE_ACCOUNT", "Non-transferable account marker.".into()),
                (Side::Source, "ConfidentialTransferAccount" | "ConfidentialTransferFeeAmount") => f(
                    Unverifiable,
                    "CONFIDENTIAL_BALANCE",
                    "The account carries encrypted balance state; its full holder balance cannot be verified or migrated publicly.".into(),
                ),
                (Side::Destination, "ConfidentialTransferAccount") => {
                    if e.config["allowNonConfidentialCredits"] == false {
                        f(Unsupported, "CONFIDENTIAL_ONLY_DESTINATION", "The destination refuses non-confidential credits.".into())
                    } else {
                        f(SupportedWithSpecialSemantics, "PUBLIC_CREDIT", "The destination receives a public credit.".into())
                    }
                }
                (_, other) => f(Unverifiable, "UNKNOWN_EXTENSION", format!("Extension {other} has no migration classification.")),
            }
        })
        .collect()
}

/// The matrix as documentation: every known extension and operation.
pub fn matrix() -> Vec<(&'static str, &'static str)> {
    vec![
        ("TransferFeeConfig", "burn/mint-to: Supported; escrow/reserve transfer: SupportedWithSpecialSemantics (recipient credited net, fee withheld)"),
        ("TransferFeeAmount", "Supported; nonzero source withheld fees are not migrated (SupportedWithSpecialSemantics)"),
        ("MintCloseAuthority", "Supported (reported as an authority)"),
        ("ConfidentialTransferMint / ConfidentialTransferFeeConfig", "SupportedWithSpecialSemantics (public balances only)"),
        ("ConfidentialTransferAccount / ConfidentialTransferFeeAmount (source)", "Unverifiable"),
        ("ConfidentialTransferAccount (destination)", "SupportedWithSpecialSemantics; Unsupported if non-confidential credits are refused"),
        ("ConfidentialMintBurn", "burn/mint-to: Unsupported; reserve transfer: SupportedWithSpecialSemantics"),
        ("DefaultAccountState", "Supported; Frozen on destination: SupportedWithSpecialSemantics (new destinations frozen); Frozen with escrow: RequiresProtocolSpecificHandling"),
        ("NonTransferable / NonTransferableAccount", "burn: Supported; transfer: Unsupported; mint-to: SupportedWithSpecialSemantics"),
        ("InterestBearingConfig / ScaledUiAmount", "raw-basis ratio: SupportedWithSpecialSemantics; ui-basis ratio: Unsupported"),
        ("PermanentDelegate", "SupportedWithSpecialSemantics (authority; optional issuer-authorized path)"),
        ("TransferHook", "burn/mint-to or no hook program: Supported; transfer with hook program: RequiresProtocolSpecificHandling"),
        ("MetadataPointer / TokenMetadata / Group* ", "Supported"),
        ("Pausable", "paused: Unsupported; not paused: SupportedWithSpecialSemantics (pause authority)"),
        ("PermissionedBurn", "burn: RequiresProtocolSpecificHandling; otherwise Supported"),
        ("MemoTransfer", "source: Supported; destination with required memos: reserve transfer RequiresProtocolSpecificHandling, mint-to Supported"),
        ("CpiGuard", "source with lock and owner authorization: Unsupported; otherwise Supported"),
        ("ImmutableOwner / TransferHookAccount / PausableAccount", "Supported"),
        ("any other extension", "Unverifiable"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mint(extensions: Vec<(&str, Value)>) -> MintConfig {
        MintConfig {
            token_program: crate::standard_programs::token::TOKEN_2022_PROGRAM.into(),
            is_token_2022: true,
            decimals: 6,
            raw_supply: "0".into(),
            decimal_supply: "0".into(),
            mint_authority: None,
            freeze_authority: None,
            is_initialized: true,
            extensions: extensions
                .into_iter()
                .enumerate()
                .map(|(i, (name, config))| TokenExtension {
                    extension_type: name.into(),
                    type_id: i as u16 + 1,
                    config,
                })
                .collect(),
        }
    }

    #[test]
    fn classification_depends_on_the_operation() {
        let mut spec = crate::migration::spec::tests::example();
        let fee = mint(vec![
            ("TransferFeeConfig", json!({})),
            ("NonTransferable", json!({})),
        ]);
        let burn = classify_mint(&spec, Side::Source, &fee);
        assert_eq!(burn[0].support, Support::Supported);
        assert_eq!(burn[1].support, Support::Supported);
        spec.source_disposition = SourceDisposition::Escrow;
        let escrow = classify_mint(&spec, Side::Source, &fee);
        assert_eq!(escrow[0].support, Support::SupportedWithSpecialSemantics);
        assert_eq!(escrow[1].support, Support::Unsupported);
        let reserve = classify_mint(&spec, Side::Destination, &fee);
        assert_eq!(reserve[1].code, "NON_TRANSFERABLE");
        spec.destination_funding = DestinationFunding::MintTo;
        let minted = classify_mint(&spec, Side::Destination, &fee);
        assert_eq!(minted[0].support, Support::Supported);
        assert_eq!(minted[1].support, Support::SupportedWithSpecialSemantics);
    }

    #[test]
    fn hooks_pauses_and_unknown_extensions_never_execute_silently() {
        let spec = crate::migration::spec::tests::example();
        let state = mint(vec![
            (
                "TransferHook",
                json!({"programId": "11111111111111111111111111111111"}),
            ),
            ("Pausable", json!({"paused": true})),
            ("SomethingNew", json!({})),
        ]);
        let findings = classify_mint(&spec, Side::Destination, &state);
        assert_eq!(
            findings[0].support,
            Support::RequiresProtocolSpecificHandling
        );
        assert_eq!(findings[1].code, "MINT_PAUSED");
        assert_eq!(findings[2].support, Support::Unverifiable);
        assert!(findings.iter().all(|f| !f.support.executable()));
    }
}
