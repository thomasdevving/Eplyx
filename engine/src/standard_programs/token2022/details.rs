//! Complete Token-2022 extension fields for current-state policy consumers.
//!
//! Layouts are pinned to spl-token-2022-interface 3.1.1; tests construct the
//! official POD/Borsh layouts. Decoding describes bytes, never eligibility.
//! Unknown entries retain their exact bytes; callers decide support policy.
use super::{optional_pubkey, transfer_fee_at, Layout, TransferFee};
use crate::standard_programs::{address_at, u16_at, u64_at, Decoded, MalformedReason};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Extension {
    TransferFeeConfig {
        config_authority: Option<String>,
        withdraw_authority: Option<String>,
        #[serde(with = "crate::numfmt::u64_string")]
        withheld_amount: u64,
        older: TransferFee,
        newer: TransferFee,
    },
    TransferFeeAmount {
        #[serde(with = "crate::numfmt::u64_string")]
        withheld_amount: u64,
    },
    MintCloseAuthority {
        close_authority: Option<String>,
    },
    ConfidentialTransferMint {
        authority: Option<String>,
        auto_approve_new_accounts: bool,
        auditor_elgamal_pubkey: Option<String>,
    },
    ConfidentialTransferAccount {
        approved: bool,
        elgamal_pubkey: String,
        pending_balance_lo: String,
        pending_balance_hi: String,
        available_balance: String,
        decryptable_available_balance: String,
        allow_confidential_credits: bool,
        allow_non_confidential_credits: bool,
        #[serde(with = "crate::numfmt::u64_string")]
        pending_balance_credit_counter: u64,
        #[serde(with = "crate::numfmt::u64_string")]
        maximum_pending_balance_credit_counter: u64,
        #[serde(with = "crate::numfmt::u64_string")]
        expected_pending_balance_credit_counter: u64,
        #[serde(with = "crate::numfmt::u64_string")]
        actual_pending_balance_credit_counter: u64,
    },
    DefaultAccountState {
        state: u8,
    },
    ImmutableOwner,
    MemoTransfer {
        require_incoming_transfer_memos: bool,
    },
    NonTransferable,
    InterestBearingConfig {
        rate_authority: Option<String>,
        #[serde(with = "crate::numfmt::i64_string")]
        initialization_timestamp: i64,
        pre_update_average_rate: i16,
        #[serde(with = "crate::numfmt::i64_string")]
        last_update_timestamp: i64,
        current_rate: i16,
    },
    CpiGuard {
        lock_cpi: bool,
    },
    PermanentDelegate {
        delegate: Option<String>,
    },
    NonTransferableAccount,
    TransferHook {
        authority: Option<String>,
        program_id: Option<String>,
    },
    TransferHookAccount {
        transferring: bool,
    },
    ConfidentialTransferFeeConfig {
        authority: Option<String>,
        withdraw_withheld_authority_elgamal_pubkey: String,
        harvest_to_mint_enabled: bool,
        withheld_amount: String,
    },
    ConfidentialTransferFeeAmount {
        withheld_amount: String,
    },
    MetadataPointer {
        authority: Option<String>,
        metadata_address: Option<String>,
    },
    GroupPointer {
        authority: Option<String>,
        group_address: Option<String>,
    },
    TokenGroup {
        update_authority: Option<String>,
        mint: String,
        #[serde(with = "crate::numfmt::u64_string")]
        size: u64,
        #[serde(with = "crate::numfmt::u64_string")]
        max_size: u64,
    },
    GroupMemberPointer {
        authority: Option<String>,
        member_address: Option<String>,
    },
    TokenGroupMember {
        mint: String,
        group: String,
        #[serde(with = "crate::numfmt::u64_string")]
        member_number: u64,
    },
    ConfidentialMintBurn {
        confidential_supply: String,
        decryptable_supply: String,
        supply_elgamal_pubkey: String,
        pending_burn: String,
    },
    ScaledUiAmount {
        authority: Option<String>,
        #[serde(with = "crate::numfmt::u64_string")]
        multiplier_bits: u64,
        #[serde(with = "crate::numfmt::i64_string")]
        new_multiplier_effective_timestamp: i64,
        #[serde(with = "crate::numfmt::u64_string")]
        new_multiplier_bits: u64,
    },
    Pausable {
        authority: Option<String>,
        paused: bool,
    },
    PausableAccount,
    PermissionedBurn {
        authority: Option<String>,
    },
    TokenMetadata {
        update_authority: Option<String>,
        mint: String,
        name: String,
        symbol: String,
        uri: String,
        additional_metadata: Vec<(String, String)>,
    },
    Unrecognized {
        kind: u16,
        #[serde(with = "crate::hexfmt")]
        bytes: Vec<u8>,
    },
}

/// The base layout a known extension belongs to. Unknown types have no invented
/// layout and remain visible to the policy that decides whether to execute.
pub fn extension_layout(kind: u16) -> Option<Layout> {
    match kind {
        1 | 3 | 4 | 6 | 9 | 10 | 12 | 14 | 16 | 18 | 20 | 21 | 22 | 23 | 24 | 25 | 26 | 28 | 19 => {
            Some(Layout::Mint)
        }
        2 | 5 | 7 | 8 | 11 | 13 | 15 | 17 | 27 => Some(Layout::Account),
        _ => None,
    }
}

/// Exact known lengths and field checks distinguish malformed from unknown.
/// The tolerant legacy view maps malformed entries to raw `Unrecognized`.
pub fn decode_checked_extension(kind: u16, value: &[u8]) -> Decoded<Extension> {
    match decode(kind, value) {
        Ok(extension) => Decoded::Decoded(extension),
        Err(reason) => Decoded::Malformed(reason),
    }
}

fn decode(kind: u16, v: &[u8]) -> Result<Extension, MalformedReason> {
    let expected = match kind {
        1 => 108,
        2 => 8,
        3 => 32,
        4 => 65,
        5 => 295,
        6 => 1,
        7 => 0,
        8 => 1,
        9 => 0,
        10 => 52,
        11 => 1,
        12 => 32,
        13 => 0,
        14 => 64,
        15 => 1,
        16 => 129,
        17 => 64,
        18 => 64,
        20 => 64,
        21 => 80,
        22 => 64,
        23 => 72,
        24 => 196,
        25 => 56,
        26 => 33,
        27 => 0,
        28 => 32,
        19 => return metadata(v),
        _ => {
            return Ok(Extension::Unrecognized {
                kind,
                bytes: v.to_vec(),
            })
        }
    };
    if v.len() != expected {
        return Err(MalformedReason::UnexpectedLength { found: v.len() });
    }
    Ok(match kind {
        1 => Extension::TransferFeeConfig {
            config_authority: optional_pubkey(v, 0),
            withdraw_authority: optional_pubkey(v, 32),
            withheld_amount: u64_at(v, 64).expect("checked layout"),
            older: transfer_fee_at(v, 72).expect("checked layout"),
            newer: transfer_fee_at(v, 90).expect("checked layout"),
        },
        2 => Extension::TransferFeeAmount {
            withheld_amount: u64_at(v, 0).expect("checked layout"),
        },
        3 => Extension::MintCloseAuthority {
            close_authority: optional_pubkey(v, 0),
        },
        4 => Extension::ConfidentialTransferMint {
            authority: optional_pubkey(v, 0),
            auto_approve_new_accounts: boolean(v, 32),
            auditor_elgamal_pubkey: optional_cipher(&v[33..65]),
        },
        5 => Extension::ConfidentialTransferAccount {
            approved: boolean(v, 0),
            elgamal_pubkey: STANDARD.encode(&v[1..33]),
            pending_balance_lo: STANDARD.encode(&v[33..97]),
            pending_balance_hi: STANDARD.encode(&v[97..161]),
            available_balance: STANDARD.encode(&v[161..225]),
            decryptable_available_balance: STANDARD.encode(&v[225..261]),
            allow_confidential_credits: boolean(v, 261),
            allow_non_confidential_credits: boolean(v, 262),
            pending_balance_credit_counter: u64_at(v, 263).expect("checked layout"),
            maximum_pending_balance_credit_counter: u64_at(v, 271).expect("checked layout"),
            expected_pending_balance_credit_counter: u64_at(v, 279).expect("checked layout"),
            actual_pending_balance_credit_counter: u64_at(v, 287).expect("checked layout"),
        },
        6 => Extension::DefaultAccountState {
            state: state(v[0])?,
        },
        7 => Extension::ImmutableOwner,
        8 => Extension::MemoTransfer {
            require_incoming_transfer_memos: boolean(v, 0),
        },
        9 => Extension::NonTransferable,
        10 => Extension::InterestBearingConfig {
            rate_authority: optional_pubkey(v, 0),
            initialization_timestamp: i64::from_le_bytes(
                v[32..40].try_into().expect("checked layout"),
            ),
            pre_update_average_rate: i16::from_le_bytes(
                v[40..42].try_into().expect("checked layout"),
            ),
            last_update_timestamp: i64::from_le_bytes(
                v[42..50].try_into().expect("checked layout"),
            ),
            current_rate: i16::from_le_bytes(v[50..52].try_into().expect("checked layout")),
        },
        11 => Extension::CpiGuard {
            lock_cpi: boolean(v, 0),
        },
        12 => Extension::PermanentDelegate {
            delegate: optional_pubkey(v, 0),
        },
        13 => Extension::NonTransferableAccount,
        14 => Extension::TransferHook {
            authority: optional_pubkey(v, 0),
            program_id: optional_pubkey(v, 32),
        },
        15 => Extension::TransferHookAccount {
            transferring: boolean(v, 0),
        },
        16 => Extension::ConfidentialTransferFeeConfig {
            authority: optional_pubkey(v, 0),
            withdraw_withheld_authority_elgamal_pubkey: STANDARD.encode(&v[32..64]),
            harvest_to_mint_enabled: boolean(v, 64),
            withheld_amount: STANDARD.encode(&v[65..129]),
        },
        17 => Extension::ConfidentialTransferFeeAmount {
            withheld_amount: STANDARD.encode(&v[0..64]),
        },
        18 => Extension::MetadataPointer {
            authority: optional_pubkey(v, 0),
            metadata_address: optional_pubkey(v, 32),
        },
        20 => Extension::GroupPointer {
            authority: optional_pubkey(v, 0),
            group_address: optional_pubkey(v, 32),
        },
        21 => Extension::TokenGroup {
            update_authority: optional_pubkey(v, 0),
            mint: address_at(v, 32).expect("checked layout"),
            size: u64_at(v, 64).expect("checked layout"),
            max_size: u64_at(v, 72).expect("checked layout"),
        },
        22 => Extension::GroupMemberPointer {
            authority: optional_pubkey(v, 0),
            member_address: optional_pubkey(v, 32),
        },
        23 => Extension::TokenGroupMember {
            mint: address_at(v, 0).expect("checked layout"),
            group: address_at(v, 32).expect("checked layout"),
            member_number: u64_at(v, 64).expect("checked layout"),
        },
        24 => Extension::ConfidentialMintBurn {
            confidential_supply: STANDARD.encode(&v[0..64]),
            decryptable_supply: STANDARD.encode(&v[64..100]),
            supply_elgamal_pubkey: STANDARD.encode(&v[100..132]),
            pending_burn: STANDARD.encode(&v[132..196]),
        },
        25 => Extension::ScaledUiAmount {
            authority: optional_pubkey(v, 0),
            multiplier_bits: finite_bits(v, 32)?,
            new_multiplier_effective_timestamp: i64::from_le_bytes(
                v[40..48].try_into().expect("checked layout"),
            ),
            new_multiplier_bits: finite_bits(v, 48)?,
        },
        26 => Extension::Pausable {
            authority: optional_pubkey(v, 0),
            paused: boolean(v, 32),
        },
        27 => Extension::PausableAccount,
        28 => Extension::PermissionedBurn {
            authority: optional_pubkey(v, 0),
        },
        _ => unreachable!("known lengths only"),
    })
}

// SPL's Bool is an unaligned byte: every nonzero value means true.
fn boolean(v: &[u8], at: usize) -> bool {
    v[at] != 0
}
fn state(value: u8) -> Result<u8, MalformedReason> {
    if value <= 2 {
        Ok(value)
    } else {
        Err(MalformedReason::InvalidDiscriminant {
            at: 0,
            value: value.into(),
        })
    }
}
fn optional_cipher(v: &[u8]) -> Option<String> {
    v.iter().any(|b| *b != 0).then(|| STANDARD.encode(v))
}
/// IEEE-754 exponent bits identify infinity/NaN without floating-point arithmetic.
fn finite_bits(v: &[u8], at: usize) -> Result<u64, MalformedReason> {
    let bits = u64_at(v, at).expect("checked layout");
    if (bits >> 52) & 0x7ff == 0x7ff {
        Err(MalformedReason::InvalidExtension { kind: 25 })
    } else {
        Ok(bits)
    }
}

#[derive(borsh::BorshDeserialize)]
struct Metadata {
    update_authority: [u8; 32],
    mint: [u8; 32],
    name: String,
    symbol: String,
    uri: String,
    additional_metadata: Vec<(String, String)>,
}
fn metadata(v: &[u8]) -> Result<Extension, MalformedReason> {
    let m: Metadata =
        borsh::from_slice(v).map_err(|_| MalformedReason::InvalidExtension { kind: 19 })?;
    Ok(Extension::TokenMetadata {
        update_authority: optional_pubkey(&m.update_authority, 0),
        mint: bs58::encode(m.mint).into_string(),
        name: m.name,
        symbol: m.symbol,
        uri: m.uri,
        additional_metadata: m.additional_metadata,
    })
}

/// Validate TLV structure and every known entry, while retaining unknown bytes.
/// This supplies facts to a support matrix; it never authorizes execution.
/// Base fields are still decoded by `spl_token` / `token2022::decode_*`.
pub fn checked_extensions(data: &[u8], layout: Layout) -> Decoded<Vec<Extension>> {
    match check_list(data, layout) {
        Ok(entries) => Decoded::Decoded(entries),
        Err(reason) => Decoded::Malformed(reason),
    }
}
fn check_list(data: &[u8], layout: Layout) -> Result<Vec<Extension>, MalformedReason> {
    if data.len() == super::spl_token::MULTISIG_LEN {
        return Err(MalformedReason::UnexpectedLength { found: data.len() });
    }
    let base_len = match layout {
        Layout::Mint => super::spl_token::MINT_LEN,
        Layout::Account => super::spl_token::ACCOUNT_LEN,
    };
    if data.len() == base_len {
        return Ok(vec![]);
    }
    if data.len() < super::TLV_START {
        return Err(MalformedReason::UnexpectedLength { found: data.len() });
    }
    if super::layout_of(data) != Some(layout) {
        return Err(MalformedReason::InvalidDiscriminant {
            at: super::ACCOUNT_TYPE_OFFSET,
            value: data[super::ACCOUNT_TYPE_OFFSET].into(),
        });
    }
    // A mint's base is padded to the account length; nonzero padding is not TLV.
    if layout == Layout::Mint
        && data[base_len..super::ACCOUNT_TYPE_OFFSET]
            .iter()
            .any(|b| *b != 0)
    {
        return Err(MalformedReason::InvalidExtension { kind: 0 });
    }
    let mut result = vec![];
    let mut seen = std::collections::BTreeSet::new();
    let mut at = super::TLV_START;
    while at < data.len() {
        if data[at..].iter().all(|b| *b == 0) {
            break;
        }
        let kind = u16_at(data, at).ok_or(MalformedReason::Truncated {
            needed: at + 4,
            found: data.len(),
        })?;
        let len = u16_at(data, at + 2).ok_or(MalformedReason::Truncated {
            needed: at + 4,
            found: data.len(),
        })? as usize;
        if kind == 0
            || !seen.insert(kind)
            || extension_layout(kind).is_some_and(|expected| expected != layout)
        {
            return Err(MalformedReason::InvalidExtension { kind });
        }
        let end = at + 4 + len;
        let v = data
            .get(at + 4..end)
            .ok_or(MalformedReason::ExtensionOverrun { at, declared: len })?;
        result.push(decode(kind, v)?);
        at = end;
    }
    Ok(result)
}
