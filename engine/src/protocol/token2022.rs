//! Token-2022 adapter.
//!
//! Token-2022 is a real production program holding real user balances, it is
//! upgraded in place under the upgradeable loader, and its direct transfers are
//! replayable without executing any other program. That combination is what
//! makes it a workable first target: the economics are genuine while the
//! execution graph stays provable.
//!
//! The supported contract is deliberately one transaction class - a direct
//! `TransferChecked`, optionally preceded by compute-budget instructions - and
//! anything else is rejected rather than approximated. Most Token-2022 mainnet
//! traffic arrives through aggregators as versioned transactions with address
//! lookup tables and deep CPI, which this adapter does not claim to replay.

use super::{
    BoundaryDistance, EconomicChange, EntityId, FieldValue, ProtocolAdapter, SemanticAccount,
    SemanticAction, SemanticField, StateFeature, TokenQuantity,
};
use crate::{
    executor::ExecutionResult,
    ingest::transactions::HistoricalTransaction,
    types::{AccountSnapshot, NamedAccount},
};
use anyhow::{Context, Result};

pub const PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const COMPUTE_BUDGET_PROGRAM_ID: &str = "ComputeBudget111111111111111111111111111111";

/// `TransferChecked`. The unchecked `Transfer` is deliberately not supported:
/// it carries no mint, so a replay could not confirm the decimals the original
/// execution validated against.
const TRANSFER: u8 = 3;
const APPROVE: u8 = 4;
const REVOKE: u8 = 5;
const MINT_TO: u8 = 7;
const BURN: u8 = 8;
const TRANSFER_CHECKED: u8 = 12;
const APPROVE_CHECKED: u8 = 13;
const MINT_TO_CHECKED: u8 = 14;
const BURN_CHECKED: u8 = 15;

/// Offset of the `newer_transfer_fee` record inside a TransferFeeConfig
/// extension: two optional authorities (32 each), the withheld amount (8), then
/// the older fee record (8 + 8 + 2).
const NEWER_TRANSFER_FEE_OFFSET: usize = 32 + 32 + 8 + 18;
const TRANSFER_FEE_CONFIG: u16 = 1;

/// The Token-2022 instructions this adapter replays exactly.
///
/// Each variant is admitted on its own evidence: a fixed data length, a fixed
/// account count, a direct signer authority, and validator-observed token
/// balances that pin the boundary. Widening this list is additive - it never
/// relaxes the bar for a shape already supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenOp {
    Transfer,
    TransferChecked,
    MintTo,
    MintToChecked,
    Burn,
    BurnChecked,
    Approve,
    ApproveChecked,
    Revoke,
}

impl TokenOp {
    fn from_discriminant(discriminant: u8) -> Option<Self> {
        Some(match discriminant {
            TRANSFER => Self::Transfer,
            TRANSFER_CHECKED => Self::TransferChecked,
            MINT_TO => Self::MintTo,
            MINT_TO_CHECKED => Self::MintToChecked,
            BURN => Self::Burn,
            BURN_CHECKED => Self::BurnChecked,
            APPROVE => Self::Approve,
            APPROVE_CHECKED => Self::ApproveChecked,
            REVOKE => Self::Revoke,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Transfer => "Transfer",
            Self::TransferChecked => "TransferChecked",
            Self::MintTo => "MintTo",
            Self::MintToChecked => "MintToChecked",
            Self::Burn => "Burn",
            Self::BurnChecked => "BurnChecked",
            Self::Approve => "Approve",
            Self::ApproveChecked => "ApproveChecked",
            Self::Revoke => "Revoke",
        }
    }

    /// Account roles in declaration order. The authority is always last.
    fn roles(self) -> &'static [&'static str] {
        match self {
            Self::Transfer => &["source", "destination", "authority"],
            Self::TransferChecked => &["source", "mint", "destination", "authority"],
            Self::MintTo | Self::MintToChecked => &["mint", "destination", "authority"],
            Self::Burn | Self::BurnChecked => &["source", "mint", "authority"],
            Self::Approve => &["source", "delegate", "authority"],
            Self::ApproveChecked => &["source", "mint", "delegate", "authority"],
            Self::Revoke => &["source", "authority"],
        }
    }

    /// Exact instruction data length: discriminant, optional u64 amount, and
    /// for the `*Checked` variants the caller-asserted decimals.
    fn data_len(self) -> usize {
        match self {
            Self::Revoke => 1,
            Self::Transfer | Self::MintTo | Self::Burn | Self::Approve => 9,
            Self::TransferChecked
            | Self::MintToChecked
            | Self::BurnChecked
            | Self::ApproveChecked => 10,
        }
    }

    fn has_amount(self) -> bool {
        self != Self::Revoke
    }

    fn semantic_action(self) -> SemanticAction {
        match self {
            Self::Transfer | Self::TransferChecked => SemanticAction::Transfer,
            Self::MintTo | Self::MintToChecked => SemanticAction::Mint,
            Self::Burn | Self::BurnChecked => SemanticAction::Burn,
            Self::Approve | Self::ApproveChecked => SemanticAction::Approve,
            Self::Revoke => SemanticAction::Revoke,
        }
    }
}

/// Length of the base account and mint structures, before any extensions.
const ACCOUNT_LEN: usize = 165;
const MINT_LEN: usize = 82;
/// Extended accounts carry a discriminant here, then TLV extension entries.
const ACCOUNT_TYPE_OFFSET: usize = 165;
const TLV_START: usize = 166;
const ACCOUNT_TYPE_MINT: u8 = 1;
const ACCOUNT_TYPE_ACCOUNT: u8 = 2;

pub struct Token2022Adapter;

fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn address_at(data: &[u8], offset: usize) -> Option<String> {
    Some(bs58::encode(data.get(offset..offset + 32)?).into_string())
}

/// A `COption<Pubkey>`: a 4-byte discriminant followed by the key.
fn coption_address_at(data: &[u8], offset: usize) -> Option<Option<String>> {
    match u32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?) {
        0 => Some(None),
        1 => Some(address_at(data, offset + 4)),
        _ => None,
    }
}

fn extension_name(kind: u16) -> &'static str {
    match kind {
        1 => "transfer-fee-config",
        2 => "transfer-fee-amount",
        3 => "mint-close-authority",
        4 => "confidential-transfer-mint",
        5 => "confidential-transfer-account",
        6 => "default-account-state",
        7 => "immutable-owner",
        8 => "memo-transfer",
        9 => "non-transferable",
        10 => "interest-bearing-config",
        11 => "cpi-guard",
        12 => "permanent-delegate",
        13 => "non-transferable-account",
        14 => "transfer-hook",
        15 => "transfer-hook-account",
        16 => "confidential-transfer-fee-config",
        17 => "confidential-transfer-fee-amount",
        18 => "metadata-pointer",
        19 => "token-metadata",
        20 => "group-pointer",
        21 => "token-group",
        22 => "group-member-pointer",
        23 => "token-group-member",
        24 => "confidential-mint-burn",
        25 => "scaled-ui-amount",
        26 => "pausable",
        27 => "pausable-account",
        _ => "unrecognized",
    }
}

/// Walk the TLV extension list, returning `(type, value)` pairs.
///
/// A malformed or truncated list yields what was parsed up to that point rather
/// than an error: extension parsing informs the report, while the economic
/// verdict rests on the base fields and the proved balances.
fn extensions(data: &[u8]) -> Vec<(u16, &[u8])> {
    let mut found = Vec::new();
    let mut offset = TLV_START;
    while offset + 4 <= data.len() {
        let Some(kind) = u16_at(data, offset) else {
            break;
        };
        let Some(length) = u16_at(data, offset + 2) else {
            break;
        };
        if kind == 0 && length == 0 {
            break;
        }
        let start = offset + 4;
        let end = start + usize::from(length);
        if end > data.len() {
            break;
        }
        found.push((kind, &data[start..end]));
        offset = end;
    }
    found
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Mint,
    Account,
}

fn layout_of(data: &[u8]) -> Option<Layout> {
    match data.len() {
        MINT_LEN => Some(Layout::Mint),
        ACCOUNT_LEN => Some(Layout::Account),
        length if length > ACCOUNT_TYPE_OFFSET => match data.get(ACCOUNT_TYPE_OFFSET) {
            Some(&ACCOUNT_TYPE_MINT) => Some(Layout::Mint),
            Some(&ACCOUNT_TYPE_ACCOUNT) => Some(Layout::Account),
            _ => None,
        },
        _ => None,
    }
}

/// The decoded balance of a token account, with the decimals it is denominated
/// in. Decimals come from the account's own mint, so the caller must supply it.
pub fn token_account_amount(data: &[u8]) -> Option<u64> {
    (layout_of(data)? == Layout::Account).then(|| u64_at(data, 64))?
}

pub fn token_account_mint(data: &[u8]) -> Option<String> {
    (layout_of(data)? == Layout::Account).then(|| address_at(data, 0))?
}

pub fn mint_decimals(data: &[u8]) -> Option<u8> {
    (layout_of(data)? == Layout::Mint)
        .then(|| data.get(44).copied())
        .flatten()
}

impl Token2022Adapter {
    /// Token-2022 instructions in this transaction, with their message indices.
    fn transfer_instructions<'a>(
        &self,
        transaction: &'a HistoricalTransaction,
    ) -> Vec<&'a crate::types::InstructionSpec> {
        transaction
            .instructions
            .iter()
            .filter(|ix| ix.program == PROGRAM_ID)
            .collect()
    }

    /// Unique semantic labels for every message key.
    ///
    /// Roles are taken from the transfer's account positions; anything with no
    /// role keeps a positional label. Uniqueness is enforced by construction so
    /// that labels stay usable as the diff and report key.
    fn labels(&self, transaction: &HistoricalTransaction) -> Vec<String> {
        let mut labels: Vec<String> = (0..transaction.account_keys.len())
            .map(|index| format!("key-{index}"))
            .collect();
        let ops = self.ops(transaction);
        let multiple = ops.len() > 1;
        for (ordinal, (op, instruction)) in ops.iter().enumerate() {
            for (position, role) in op.roles().iter().enumerate() {
                let Some(meta) = instruction.accounts.get(position) else {
                    continue;
                };
                let Some(index) = transaction
                    .account_keys
                    .iter()
                    .position(|key| key.address == meta.address)
                else {
                    continue;
                };
                // A key already named by an earlier instruction keeps that name,
                // so the label stays stable however many instructions touch it.
                if !labels[index].starts_with("key-") {
                    continue;
                }
                labels[index] = if multiple {
                    format!("{role}-{ordinal}")
                } else {
                    (*role).to_string()
                };
            }
        }
        // The fee payer is always message key 0. Name it only if no transfer
        // role already claimed it, so an authority that also pays keeps the
        // role that explains what it is doing.
        if let Some(label) = labels.first_mut() {
            if label.starts_with("key-") {
                *label = "payer".into();
            }
        }
        labels
    }

    /// Token-2022 instructions paired with the operation they encode.
    ///
    /// Only reachable after `accept`, which is what guarantees every
    /// discriminant here is one this adapter proved it can replay.
    fn ops<'a>(
        &self,
        transaction: &'a HistoricalTransaction,
    ) -> Vec<(TokenOp, &'a crate::types::InstructionSpec)> {
        self.transfer_instructions(transaction)
            .into_iter()
            .filter_map(|instruction| {
                let discriminant = *instruction.data.first()?;
                Some((TokenOp::from_discriminant(discriminant)?, instruction))
            })
            .collect()
    }

    /// Address playing `role` in the first operation that declares it.
    fn role_address(&self, transaction: &HistoricalTransaction, role: &str) -> Option<String> {
        for (op, instruction) in self.ops(transaction) {
            let position = op.roles().iter().position(|candidate| *candidate == role)?;
            if let Some(meta) = instruction.accounts.get(position) {
                return Some(meta.address.clone());
            }
        }
        None
    }

    /// Base-unit amount carried by the first operation that has one.
    fn primary_amount(&self, transaction: &HistoricalTransaction) -> Option<u64> {
        self.ops(transaction)
            .into_iter()
            .find_map(|(op, ix)| op.has_amount().then(|| u64_at(&ix.data, 1)).flatten())
    }

    fn account_data<'a>(&self, accounts: &'a [NamedAccount], address: &str) -> Option<&'a [u8]> {
        accounts
            .iter()
            .find(|named| named.address == address)
            .map(|named| named.account.data.as_slice())
    }

    /// The mint's transfer-fee schedule currently in force, if it has one.
    ///
    /// Returns `(basis_points, maximum_fee)` from the `newer_transfer_fee`
    /// record. The epoch-dependent choice between the older and newer schedule
    /// is not modelled: this feeds boundary *ranking*, never the economic
    /// verdict, which rests on executing the real program.
    fn transfer_fee_schedule(&self, mint_data: &[u8]) -> Option<(u16, u64)> {
        let (_, value) = extensions(mint_data)
            .into_iter()
            .find(|(kind, _)| *kind == TRANSFER_FEE_CONFIG)?;
        let maximum_fee = u64_at(value, NEWER_TRANSFER_FEE_OFFSET + 8)?;
        let basis_points = u16_at(value, NEWER_TRANSFER_FEE_OFFSET + 16)?;
        Some((basis_points, maximum_fee))
    }

    fn balance_at<'a>(
        &self,
        balances: Option<&'a Vec<crate::ingest::transactions::TokenBalance>>,
        index: usize,
    ) -> Option<&'a crate::ingest::transactions::TokenBalance> {
        balances?
            .iter()
            .find(|balance| balance.account_index == index)
    }
}

impl ProtocolAdapter for Token2022Adapter {
    fn name(&self) -> &'static str {
        "token-2022"
    }

    fn program_id(&self) -> &'static str {
        PROGRAM_ID
    }

    /// Widened in Phase 9 from TransferChecked alone to the nine balance and
    /// delegation instructions, and given semantic classification. A corpus
    /// built under version 1 describes a different interpretation.
    fn adapter_version(&self) -> u32 {
        2
    }

    /// Classified from the first Token-2022 instruction in the message.
    ///
    /// A transaction carrying several instructions of different families is
    /// named by its first; `token_instruction_count` records that there were
    /// more, so the simplification is visible rather than hidden.
    fn semantic_action(&self, transaction: &HistoricalTransaction) -> SemanticAction {
        self.ops(transaction)
            .first()
            .map(|(op, _)| op.semantic_action())
            .unwrap_or(SemanticAction::Unknown)
    }

    /// The token account whose balance or delegation the interaction moves.
    ///
    /// For every operation but minting that is the source; a mint has no
    /// source, so the destination is the account that gains the position. This
    /// is what stops a hundred interactions with one account from being counted
    /// as a hundred independent exposures.
    fn economic_entity_id(
        &self,
        transaction: &HistoricalTransaction,
        _accounts: &[NamedAccount],
    ) -> Option<EntityId> {
        let address = self
            .role_address(transaction, "source")
            .or_else(|| self.role_address(transaction, "destination"))?;
        Some(EntityId::new("token-account", address))
    }

    fn state_features(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
    ) -> Vec<StateFeature> {
        let ops = self.ops(transaction);
        let mut features = vec![
            StateFeature::integer("token_instruction_count", ops.len() as u128),
            StateFeature::text(
                "semantic_action",
                self.semantic_action(transaction).as_str(),
            ),
        ];

        let source = self.role_address(transaction, "source");
        let destination = self.role_address(transaction, "destination");
        let amount = self.primary_amount(transaction);

        if let Some(amount) = amount {
            features.push(StateFeature::integer("amount", amount as u128));
        }

        // The mint is named directly by the checked variants; for the unchecked
        // ones it has to come from the token account's own bytes.
        let mint_address = self.role_address(transaction, "mint").or_else(|| {
            source
                .as_ref()
                .or(destination.as_ref())
                .and_then(|address| self.account_data(accounts, address))
                .and_then(token_account_mint)
        });

        if let Some(data) = mint_address
            .as_deref()
            .and_then(|address| self.account_data(accounts, address))
        {
            if let Some(decimals) = mint_decimals(data) {
                features.push(StateFeature::integer("mint_decimals", decimals as u128));
            }
            if let Some(supply) = u64_at(data, 36) {
                features.push(StateFeature::integer("mint_supply", supply as u128));
            }
            features.push(StateFeature::integer(
                "mint_extension_count",
                extensions(data).len() as u128,
            ));
            if let Some((basis_points, maximum_fee)) = self.transfer_fee_schedule(data) {
                features.push(StateFeature::integer(
                    "transfer_fee_basis_points",
                    basis_points as u128,
                ));
                features.push(StateFeature::integer(
                    "transfer_fee_maximum",
                    maximum_fee as u128,
                ));
            }
        }

        let balance_of = |address: &Option<String>| -> Option<u64> {
            address
                .as_deref()
                .and_then(|address| self.account_data(accounts, address))
                .and_then(token_account_amount)
        };
        let source_balance = balance_of(&source);
        if let Some(balance) = source_balance {
            features.push(StateFeature::integer("source_balance", balance as u128));
        }
        if let Some(balance) = balance_of(&destination) {
            features.push(StateFeature::integer(
                "destination_balance",
                balance as u128,
            ));
        }

        // How much of the holder's position the interaction moves. This is the
        // feature that separates a routine payment from an account being
        // emptied, and it is what the drain boundary is computed against.
        if let (Some(amount), Some(balance)) = (amount, source_balance) {
            let ratio = if balance == 0 {
                0
            } else {
                u128::from(amount).saturating_mul(10_000) / u128::from(balance)
            };
            features.push(StateFeature::integer("amount_to_source_balance_bps", ratio));
        }

        features
    }

    /// Thresholds Token-2022 itself defines, so a distance means something.
    ///
    /// Two are modelled. Draining a position to exactly zero is where balance
    /// arithmetic and rounding stop having slack. A transfer fee reaching its
    /// configured cap is where the fee formula switches from proportional to
    /// clamped, which is a real branch in the program.
    fn boundaries(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
    ) -> Vec<BoundaryDistance> {
        let Some(amount) = self.primary_amount(transaction) else {
            return Vec::new();
        };
        let mut boundaries = Vec::new();

        let source = self.role_address(transaction, "source");
        if let Some(balance) = source
            .as_deref()
            .and_then(|address| self.account_data(accounts, address))
            .and_then(token_account_amount)
        {
            boundaries.push(BoundaryDistance::from_quantities(
                "source_drain",
                "amount moved against the source balance; zero distance empties the position",
                u128::from(amount),
                u128::from(balance),
            ));
        }

        let mint_address = self.role_address(transaction, "mint").or_else(|| {
            source
                .as_ref()
                .and_then(|address| self.account_data(accounts, address))
                .and_then(token_account_mint)
        });
        if let Some((basis_points, maximum_fee)) = mint_address
            .as_deref()
            .and_then(|address| self.account_data(accounts, address))
            .and_then(|data| self.transfer_fee_schedule(data))
        {
            if basis_points > 0 && maximum_fee > 0 {
                // Token-2022 rounds the proportional fee up before clamping it.
                let uncapped = u128::from(amount)
                    .saturating_mul(u128::from(basis_points))
                    .saturating_add(9_999)
                    / 10_000;
                boundaries.push(BoundaryDistance::from_quantities(
                    "transfer_fee_cap",
                    "proportional fee against the configured maximum; zero distance is where \
                     the fee stops scaling with the amount",
                    uncapped,
                    u128::from(maximum_fee),
                ));
            }
        }

        boundaries
    }

    fn accept(&self, transaction: &HistoricalTransaction) -> Result<()> {
        super::require_executable_message(transaction)?;
        anyhow::ensure!(
            transaction.success && transaction.error.is_none(),
            "replay selects successfully captured original transactions"
        );
        anyhow::ensure!(
            transaction.inner_instructions.is_empty(),
            "Token-2022 replay excludes transactions containing CPI"
        );
        for instruction in &transaction.instructions {
            anyhow::ensure!(
                instruction.program == PROGRAM_ID
                    || instruction.program == COMPUTE_BUDGET_PROGRAM_ID,
                "unsupported program {} in a Token-2022 replay",
                instruction.program
            );
        }
        let instructions = self.transfer_instructions(transaction);
        anyhow::ensure!(
            !instructions.is_empty(),
            "transaction contains no Token-2022 instruction"
        );
        for instruction in &instructions {
            let discriminant = *instruction
                .data
                .first()
                .context("empty Token-2022 instruction data")?;
            let op = TokenOp::from_discriminant(discriminant).with_context(|| {
                format!(
                    "Token-2022 replay supports Transfer, TransferChecked, MintTo, \
                     MintToChecked, Burn, BurnChecked, Approve, ApproveChecked and Revoke; \
                     found instruction variant {discriminant}"
                )
            })?;
            anyhow::ensure!(
                instruction.data.len() == op.data_len(),
                "{} data must be {} bytes, found {}",
                op.name(),
                op.data_len(),
                instruction.data.len()
            );
            // An exact account count is what excludes multisig authorities and
            // transfer-hook extra accounts, both of which change what executes.
            anyhow::ensure!(
                instruction.accounts.len() == op.roles().len(),
                "{} with {} accounts is outside the supported shape; multisig authorities \
                 and transfer-hook extra accounts are not supported",
                op.name(),
                instruction.accounts.len()
            );
            anyhow::ensure!(
                instruction
                    .accounts
                    .last()
                    .is_some_and(|meta| meta.is_signer),
                "{} authority must be a direct signer",
                op.name()
            );
            if op.has_amount() {
                anyhow::ensure!(
                    u64_at(&instruction.data, 1).is_some_and(|amount| amount > 0),
                    "{} must carry a non-zero amount",
                    op.name()
                );
            }
        }
        anyhow::ensure!(
            transaction.pre_token_balances.is_some() && transaction.post_token_balances.is_some(),
            "Token-2022 replay requires validator-observed token balances as boundary evidence"
        );
        Ok(())
    }

    fn label(&self, transaction: &HistoricalTransaction, index: usize) -> String {
        self.labels(transaction)
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("key-{index}"))
    }

    fn decode(&self, account: &AccountSnapshot) -> Option<SemanticAccount> {
        if account.owner != PROGRAM_ID {
            return None;
        }
        let data = &account.data;
        let present = extensions(data)
            .into_iter()
            .map(|(kind, _)| extension_name(kind))
            .collect::<Vec<_>>();
        match layout_of(data)? {
            Layout::Account => {
                let amount = u64_at(data, 64)?;
                let mut fields = vec![
                    SemanticField {
                        name: "mint".into(),
                        value: FieldValue::Address(address_at(data, 0)?),
                        economic: false,
                    },
                    SemanticField {
                        name: "owner".into(),
                        value: FieldValue::Address(address_at(data, 32)?),
                        economic: false,
                    },
                    SemanticField {
                        name: "amount".into(),
                        // Decimals are a property of the mint, which this
                        // account does not carry. The interpretation layer
                        // rescales once the mint is known; zero here keeps the
                        // raw base units visible and unrounded.
                        value: FieldValue::quantity(amount, 0),
                        economic: true,
                    },
                    SemanticField {
                        name: "state".into(),
                        value: FieldValue::Count(u64::from(*data.get(108)?)),
                        economic: true,
                    },
                    SemanticField {
                        name: "delegated_amount".into(),
                        value: FieldValue::quantity(u64_at(data, 121)?, 0),
                        economic: true,
                    },
                ];
                if let Some(delegate) = coption_address_at(data, 72)? {
                    fields.push(SemanticField {
                        name: "delegate".into(),
                        value: FieldValue::Address(delegate),
                        economic: true,
                    });
                }
                // Withheld transfer fees are spendable value parked in the
                // account, so a change in them is an economic change.
                for (kind, value) in extensions(data) {
                    if kind == 2 {
                        if let Some(withheld) = u64_at(value, 0) {
                            fields.push(SemanticField {
                                name: "withheld_transfer_fee".into(),
                                value: FieldValue::quantity(withheld, 0),
                                economic: true,
                            });
                        }
                    }
                }
                if !present.is_empty() {
                    fields.push(SemanticField {
                        name: "extensions".into(),
                        value: FieldValue::Text(present.join(", ")),
                        economic: false,
                    });
                }
                Some(SemanticAccount {
                    kind: "token-account".into(),
                    fields,
                })
            }
            Layout::Mint => {
                let decimals = *data.get(44)?;
                let mut fields = vec![
                    SemanticField {
                        name: "supply".into(),
                        value: FieldValue::quantity(u64_at(data, 36)?, decimals),
                        economic: true,
                    },
                    SemanticField {
                        name: "decimals".into(),
                        value: FieldValue::Count(u64::from(decimals)),
                        economic: false,
                    },
                    SemanticField {
                        name: "is_initialized".into(),
                        value: FieldValue::Flag(*data.get(45)? == 1),
                        economic: false,
                    },
                ];
                for (kind, value) in extensions(data) {
                    if kind == 1 {
                        // TransferFeeConfig: two authorities, the withheld
                        // total, then the older and newer fee schedules.
                        if let (Some(withheld), Some(older_bps), Some(newer_bps)) =
                            (u64_at(value, 64), u16_at(value, 88), u16_at(value, 106))
                        {
                            fields.push(SemanticField {
                                name: "withheld_transfer_fee".into(),
                                value: FieldValue::quantity(withheld, decimals),
                                economic: true,
                            });
                            fields.push(SemanticField {
                                name: "transfer_fee_basis_points_older".into(),
                                value: FieldValue::Count(u64::from(older_bps)),
                                economic: true,
                            });
                            fields.push(SemanticField {
                                name: "transfer_fee_basis_points_newer".into(),
                                value: FieldValue::Count(u64::from(newer_bps)),
                                economic: true,
                            });
                        }
                    }
                }
                if !present.is_empty() {
                    fields.push(SemanticField {
                        name: "extensions".into(),
                        value: FieldValue::Text(present.join(", ")),
                        economic: false,
                    });
                }
                Some(SemanticAccount {
                    kind: "mint".into(),
                    fields,
                })
            }
        }
    }

    fn prove_boundaries(
        &self,
        transaction: &HistoricalTransaction,
        pre: &[NamedAccount],
        post: &[NamedAccount],
    ) -> Result<Vec<String>> {
        let pre_balances = transaction
            .pre_balances
            .as_ref()
            .context("transaction metadata omitted pre-balances")?;
        let post_balances = transaction
            .post_balances
            .as_ref()
            .context("transaction metadata omitted post-balances")?;
        let mut proved_token_accounts = 0_usize;

        for (side, snapshots, lamports, token_balances) in [
            (
                "pre",
                pre,
                pre_balances,
                transaction.pre_token_balances.as_ref(),
            ),
            (
                "post",
                post,
                post_balances,
                transaction.post_token_balances.as_ref(),
            ),
        ] {
            for named in snapshots {
                let index = transaction
                    .account_keys
                    .iter()
                    .position(|key| key.address == named.address)
                    .with_context(|| {
                        format!("snapshot account {} is not in the message", named.address)
                    })?;
                // A mismatch here is almost always same-slot interference
                // rather than a bad archive: the archive answers with the state
                // at the *end* of slot S, so another transaction touching the
                // same account in that slot moves it away from this
                // transaction's boundary. Pre-state and post-state failures
                // mean different things, so they are reported differently.
                anyhow::ensure!(
                    lamports.get(index) == Some(&named.account.lamports),
                    "{} for {}: archive reports {} lamports at the {} boundary, validator \
                     metadata records {}. {}",
                    if side == "pre" {
                        "slot-before state is not this transaction's pre-state"
                    } else {
                        "slot-end state is not this transaction's post-state"
                    },
                    named.address,
                    named.account.lamports,
                    if side == "pre" { "S-1" } else { "S" },
                    lamports
                        .get(index)
                        .map(u64::to_string)
                        .unwrap_or_else(|| "nothing".into()),
                    if side == "pre" {
                        "Another transaction in slot S wrote this account before this one."
                    } else {
                        "Another transaction in slot S wrote this account after this one, so \
                         the archived end-of-slot state cannot serve as the fidelity reference. \
                         Select a transaction whose accounts are untouched elsewhere in its slot."
                    }
                );
                let Some(balance) = self.balance_at(token_balances, index) else {
                    continue;
                };
                // The validator recorded an exact base-unit amount for this
                // account. Anything the archive returned has to match it, which
                // is what rejects a snapshot taken on the wrong side of a
                // same-slot write.
                let decoded = token_account_amount(&named.account.data).with_context(|| {
                    format!(
                        "account {} has a token balance but does not decode as a token account",
                        named.address
                    )
                })?;
                anyhow::ensure!(
                    decoded == balance.amount,
                    "{side}-state archive amount {decoded} differs from validator-observed \
                     {} for {}",
                    balance.amount,
                    named.address
                );
                anyhow::ensure!(
                    balance.program_id == PROGRAM_ID,
                    "account {} is owned by token program {}, not Token-2022",
                    named.address,
                    balance.program_id
                );
                anyhow::ensure!(
                    token_account_mint(&named.account.data).as_deref() == Some(&balance.mint),
                    "account {} decodes to a different mint than the validator recorded",
                    named.address
                );
                if side == "pre" {
                    proved_token_accounts += 1;
                }
            }
        }

        // Read-only accounts must be byte-identical across the boundary. For the
        // mint this is the only available proof - metadata records no mint data -
        // so it is stated as an assumption rather than claimed as independent.
        for named in pre {
            let index = transaction
                .account_keys
                .iter()
                .position(|key| key.address == named.address)
                .expect("checked above");
            if transaction.account_keys[index].is_writable {
                continue;
            }
            let after = post
                .iter()
                .find(|other| other.address == named.address)
                .context("read-only account missing from post-state")?;
            anyhow::ensure!(
                named.account.data == after.account.data
                    && named.account.owner == after.account.owner,
                "read-only account {} changed across the transaction boundary",
                named.address
            );
        }

        anyhow::ensure!(
            proved_token_accounts > 0,
            "no token account balance could be proved against validator metadata"
        );

        Ok(vec![
            format!(
                "{proved_token_accounts} token account balance(s) at S-1 and S match the \
                 validator-observed pre/post token balances"
            ),
            "every snapshot's lamports match validator-observed pre/post balances".into(),
            "read-only accounts, including the mint, are byte-identical across the boundary; \
             validator metadata records no mint data, so mint bytes rest on the archive and on \
             V1 reproducing the original outcome"
                .into(),
            "supported contract is a direct Token-2022 balance or delegation instruction \
             (Transfer, TransferChecked, MintTo, MintToChecked, Burn, BurnChecked, Approve, \
             ApproveChecked, Revoke) with a signer authority, optionally preceded by \
             compute-budget instructions, and no CPI"
                .into(),
            "Token-2022 reads no Clock in this path; remaining runtime state uses pinned \
             LiteSVM defaults"
                .into(),
        ])
    }

    fn interpret(
        &self,
        accounts: &[NamedAccount],
        v1: &ExecutionResult,
        v2: &ExecutionResult,
    ) -> Vec<EconomicChange> {
        // Decimals come from whichever mint the run touched, so amounts render
        // in human terms instead of raw base units.
        let decimals = accounts
            .iter()
            .find_map(|named| mint_decimals(&named.account.data))
            .unwrap_or(0);
        let mut changes = Vec::new();
        for named in accounts {
            let (Some(after_v1), Some(after_v2)) =
                (v1.accounts.get(&named.label), v2.accounts.get(&named.label))
            else {
                continue;
            };
            let (Some(decoded_v1), Some(decoded_v2)) =
                (self.decode(after_v1), self.decode(after_v2))
            else {
                continue;
            };
            for field in &decoded_v1.fields {
                if !field.economic {
                    continue;
                }
                let Some(other) = decoded_v2.field(&field.name) else {
                    continue;
                };
                if other.value == field.value {
                    continue;
                }
                let delta = match (field.value.as_quantity(), other.value.as_quantity()) {
                    (Some(before), Some(after)) => TokenQuantity::new(after.base_units, decimals)
                        .delta(TokenQuantity::new(before.base_units, decimals)),
                    _ => None,
                };
                let render = |value: &FieldValue| match value.as_quantity() {
                    Some(quantity) => TokenQuantity::new(quantity.base_units, decimals).to_string(),
                    None => value.render(),
                };
                changes.push(EconomicChange {
                    account_label: named.label.clone(),
                    account_kind: decoded_v1.kind.clone(),
                    field: field.name.clone(),
                    v1: render(&field.value),
                    v2: render(&other.value),
                    delta,
                });
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_discriminant_maps_to_one_shape_and_action() {
        let cases = [
            (TRANSFER, TokenOp::Transfer, 9, 3, SemanticAction::Transfer),
            (
                TRANSFER_CHECKED,
                TokenOp::TransferChecked,
                10,
                4,
                SemanticAction::Transfer,
            ),
            (MINT_TO, TokenOp::MintTo, 9, 3, SemanticAction::Mint),
            (
                MINT_TO_CHECKED,
                TokenOp::MintToChecked,
                10,
                3,
                SemanticAction::Mint,
            ),
            (BURN, TokenOp::Burn, 9, 3, SemanticAction::Burn),
            (
                BURN_CHECKED,
                TokenOp::BurnChecked,
                10,
                3,
                SemanticAction::Burn,
            ),
            (APPROVE, TokenOp::Approve, 9, 3, SemanticAction::Approve),
            (
                APPROVE_CHECKED,
                TokenOp::ApproveChecked,
                10,
                4,
                SemanticAction::Approve,
            ),
            (REVOKE, TokenOp::Revoke, 1, 2, SemanticAction::Revoke),
        ];
        for (discriminant, op, data_len, accounts, action) in cases {
            assert_eq!(TokenOp::from_discriminant(discriminant), Some(op));
            assert_eq!(op.data_len(), data_len, "{}", op.name());
            assert_eq!(op.roles().len(), accounts, "{}", op.name());
            assert_eq!(op.semantic_action(), action, "{}", op.name());
            // The authority is always the last declared account, which is what
            // the signer check in `accept` relies on.
            assert_eq!(*op.roles().last().unwrap(), "authority", "{}", op.name());
        }
    }

    #[test]
    fn instruction_families_outside_the_contract_stay_unmapped() {
        // InitializeAccount, SetAuthority, CloseAccount, FreezeAccount,
        // ThawAccount and SyncNative are all deliberately absent.
        for discriminant in [1_u8, 6, 9, 10, 11, 17, 200] {
            assert_eq!(TokenOp::from_discriminant(discriminant), None);
        }
    }

    #[test]
    fn only_revoke_carries_no_amount() {
        for discriminant in [TRANSFER, TRANSFER_CHECKED, MINT_TO, BURN, APPROVE] {
            assert!(TokenOp::from_discriminant(discriminant)
                .unwrap()
                .has_amount());
        }
        assert!(!TokenOp::Revoke.has_amount());
    }

    #[test]
    fn boundary_distance_is_integer_basis_points_of_the_reference() {
        // Exactly on the boundary.
        let exact = BoundaryDistance::from_quantities("d", "", 1_000, 1_000);
        assert_eq!(exact.distance_bps, 0);
        // Half the reference away.
        let half = BoundaryDistance::from_quantities("d", "", 500, 1_000);
        assert_eq!(half.distance_bps, 5_000);
        // One percent away, in either direction.
        assert_eq!(
            BoundaryDistance::from_quantities("d", "", 990, 1_000).distance_bps,
            100
        );
        assert_eq!(
            BoundaryDistance::from_quantities("d", "", 1_010, 1_000).distance_bps,
            100
        );
        // Nothing can be near a boundary with no magnitude.
        assert_eq!(
            BoundaryDistance::from_quantities("d", "", 5, 0).distance_bps,
            10_000
        );
    }

    #[test]
    fn transfer_fee_schedule_reads_the_newer_record() {
        // A TransferFeeConfig whose newer schedule is 150 bps capped at 5_000.
        let mut value = vec![0_u8; 108];
        value[NEWER_TRANSFER_FEE_OFFSET..NEWER_TRANSFER_FEE_OFFSET + 8]
            .copy_from_slice(&42_u64.to_le_bytes()); // epoch
        value[NEWER_TRANSFER_FEE_OFFSET + 8..NEWER_TRANSFER_FEE_OFFSET + 16]
            .copy_from_slice(&5_000_u64.to_le_bytes()); // maximum fee
        value[NEWER_TRANSFER_FEE_OFFSET + 16..NEWER_TRANSFER_FEE_OFFSET + 18]
            .copy_from_slice(&150_u16.to_le_bytes()); // basis points

        let mut mint = vec![0_u8; MINT_LEN];
        mint[44] = 6; // decimals, so the base layout is a plausible mint
        mint.resize(ACCOUNT_TYPE_OFFSET, 0);
        mint.push(ACCOUNT_TYPE_MINT);
        mint.extend_from_slice(&TRANSFER_FEE_CONFIG.to_le_bytes());
        mint.extend_from_slice(&(value.len() as u16).to_le_bytes());
        mint.extend_from_slice(&value);

        let schedule = Token2022Adapter.transfer_fee_schedule(&mint);
        assert_eq!(schedule, Some((150, 5_000)));
    }

    #[test]
    fn a_mint_without_the_extension_has_no_schedule() {
        let mut mint = vec![0_u8; MINT_LEN];
        mint[44] = 9;
        assert_eq!(Token2022Adapter.transfer_fee_schedule(&mint), None);
    }
}
