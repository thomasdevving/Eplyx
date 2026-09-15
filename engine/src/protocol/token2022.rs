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
    EconomicChange, FieldValue, ProtocolAdapter, SemanticAccount, SemanticField, TokenQuantity,
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
const TRANSFER_CHECKED: u8 = 12;

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
        let transfers = self.transfer_instructions(transaction);
        let multiple = transfers.len() > 1;
        for (ordinal, instruction) in transfers.iter().enumerate() {
            for (position, role) in ["source", "mint", "destination", "authority"]
                .into_iter()
                .enumerate()
            {
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
                // A key already named by an earlier transfer keeps that name, so
                // the label stays stable no matter how many transfers touch it.
                if !labels[index].starts_with("key-") {
                    continue;
                }
                labels[index] = if multiple {
                    format!("{role}-{ordinal}")
                } else {
                    role.to_string()
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

    fn accept(&self, transaction: &HistoricalTransaction) -> Result<()> {
        anyhow::ensure!(
            transaction.version == "legacy",
            "Token-2022 replay supports legacy messages only; \
             address lookup tables are normalized but not executed"
        );
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
        let transfers = self.transfer_instructions(transaction);
        anyhow::ensure!(
            !transfers.is_empty(),
            "transaction contains no Token-2022 instruction"
        );
        for instruction in &transfers {
            let discriminant = *instruction
                .data
                .first()
                .context("empty Token-2022 instruction data")?;
            anyhow::ensure!(
                discriminant == TRANSFER_CHECKED,
                "Token-2022 replay supports TransferChecked only, found discriminant {discriminant}"
            );
            anyhow::ensure!(
                instruction.data.len() == 10,
                "TransferChecked data must be discriminant, u64 amount and decimals"
            );
            // Exactly four accounts. More would mean a multisig authority or
            // transfer-hook extra accounts, both of which change what executes.
            anyhow::ensure!(
                instruction.accounts.len() == 4,
                "TransferChecked with {} accounts is outside the supported shape; \
                 multisig authorities and transfer-hook extra accounts are not supported",
                instruction.accounts.len()
            );
            anyhow::ensure!(
                instruction.accounts[3].is_signer,
                "TransferChecked authority must be a direct signer"
            );
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
            "supported contract is a direct TransferChecked with a signer authority, optionally \
             preceded by compute-budget instructions, and no CPI"
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
