//! Token-2022's account layouts and TLV extensions.
//!
//! Token-2022 accounts begin with the legacy SPL Token layout — so the base
//! fields are read by [`super::spl_token`], not re-derived here — then carry an
//! account-type byte and a TLV list of extensions.
//!
//! Before Phase U1 this lived entirely inside the Token-2022 protocol adapter.
//! That was not yet duplication; it was *pre-duplication*. Any protocol whose
//! accounts are Token-2022 mints — which is most new protocols — would have had
//! to walk the same TLV list to notice a withheld fee, and an adapter that
//! reimplements extension semantics to spot a fee is an adapter that will get
//! one wrong.
//!
//! ## Unknown extensions stay visible
//!
//! An extension this build does not model is reported as
//! [`Extension::Unrecognized`], carrying its type number and its raw bytes. It
//! is never dropped. A mint that gained a pause authority this build has never
//! heard of must not read as a mint with nothing unusual about it.

use super::{
    spl_token::{self, Mint, TokenAccount},
    u16_at, u64_at, u8_at, Decoded, MalformedReason, SchemaProvenance,
};

pub const PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

/// Where the account-type discriminant sits: immediately after the base account
/// layout, whose length both structures are padded to.
pub const ACCOUNT_TYPE_OFFSET: usize = spl_token::ACCOUNT_LEN;
/// TLV entries begin after the discriminant.
pub const TLV_START: usize = ACCOUNT_TYPE_OFFSET + 1;

const ACCOUNT_TYPE_MINT: u8 = 1;
const ACCOUNT_TYPE_ACCOUNT: u8 = 2;

// Extension type numbers, as `spl_token_2022::extension::ExtensionType` declares
// them.
pub const TRANSFER_FEE_CONFIG: u16 = 1;
pub const TRANSFER_FEE_AMOUNT: u16 = 2;
pub const DEFAULT_ACCOUNT_STATE: u16 = 6;
pub const PERMANENT_DELEGATE: u16 = 12;
pub const TRANSFER_HOOK: u16 = 14;
pub const SCALED_UI_AMOUNT: u16 = 25;
pub const PAUSABLE: u16 = 26;

/// Which of the two structures a buffer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Mint,
    Account,
}

/// Classify a buffer.
///
/// Exactly the rule the Token-2022 adapter has always applied: the two base
/// lengths are unambiguous, and anything longer than the base account layout is
/// tagged by its account-type byte. A length in between belongs to neither.
pub fn layout_of(data: &[u8]) -> Option<Layout> {
    match data.len() {
        spl_token::MINT_LEN => Some(Layout::Mint),
        spl_token::ACCOUNT_LEN => Some(Layout::Account),
        length if length > ACCOUNT_TYPE_OFFSET => match data.get(ACCOUNT_TYPE_OFFSET) {
            Some(&ACCOUNT_TYPE_MINT) => Some(Layout::Mint),
            Some(&ACCOUNT_TYPE_ACCOUNT) => Some(Layout::Account),
            _ => None,
        },
        _ => None,
    }
}

/// One TLV entry, decoded as far as this build models it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Extension {
    /// A mint's fee schedule. Two schedules are carried, `older` and `newer`;
    /// which applies depends on the epoch, which this layer does not resolve —
    /// that is a runtime question and the replay answers it by executing.
    TransferFeeConfig {
        withheld_amount: u64,
        older: TransferFee,
        newer: TransferFee,
    },
    /// Fees withheld inside one token account. Spendable value parked there.
    TransferFeeAmount {
        withheld_amount: u64,
    },
    TransferHook {
        program_id: Option<String>,
    },
    PermanentDelegate {
        delegate: Option<String>,
    },
    /// Whether transfers of this mint are currently halted.
    Pausable {
        paused: bool,
    },
    DefaultAccountState {
        state: u8,
    },
    ScaledUiAmount {
        multiplier_bytes: Vec<u8>,
    },
    /// Present, named, and not modelled by this build. Carries its bytes so a
    /// reader can see there is something here.
    Unrecognized {
        kind: u16,
        bytes: Vec<u8>,
    },
}

/// A single fee record: `epoch`, `maximum_fee`, `basis_points`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferFee {
    pub epoch: u64,
    pub maximum_fee: u64,
    pub basis_points: u16,
}

// TransferFeeConfig field offsets, within the extension's value bytes.
// Two `OptionalNonZeroPubkey` authorities of 32 bytes each, the withheld total,
// then the older and newer fee records of 18 bytes each.
//
// Public so an adapter presenting these fields names them rather than pasting
// the numbers. Two copies of `106` in two files is how one of them gets fixed.
pub const FEE_CONFIG_WITHHELD: usize = 64;
pub const FEE_CONFIG_OLDER: usize = 72;
pub const FEE_CONFIG_NEWER: usize = 90;
pub const FEE_RECORD_MAXIMUM: usize = 8;
pub const FEE_RECORD_BASIS_POINTS: usize = 16;

const _: () = assert!(FEE_CONFIG_NEWER == FEE_CONFIG_OLDER + 18);

fn transfer_fee_at(value: &[u8], offset: usize) -> Option<TransferFee> {
    Some(TransferFee {
        epoch: u64_at(value, offset)?,
        maximum_fee: u64_at(value, offset + FEE_RECORD_MAXIMUM)?,
        basis_points: u16_at(value, offset + FEE_RECORD_BASIS_POINTS)?,
    })
}

/// The name of an extension type.
///
/// Stable public vocabulary: these strings reach the `extensions` field of a
/// decoded account and from there a report a team reads. `"unrecognized"` is a
/// real answer, not a failure — a future extension is a thing that exists.
pub fn extension_name(kind: u16) -> &'static str {
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

/// The result of walking one account's TLV list.
///
/// `truncated_at` is what the previous inline implementation discarded. The walk
/// still yields everything it read before the problem — that is deliberate, and
/// it is why the economic verdict never rests on extension parsing — but a
/// caller can now *see* that the list ended badly instead of receiving a short
/// list indistinguishable from a complete one.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExtensionList {
    pub entries: Vec<(u16, Vec<u8>)>,
    pub truncated_at: Option<MalformedReason>,
}

impl ExtensionList {
    pub fn names(&self) -> Vec<&'static str> {
        self.entries
            .iter()
            .map(|(kind, _)| extension_name(*kind))
            .collect()
    }

    pub fn value(&self, kind: u16) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .map(|(_, bytes)| bytes.as_slice())
    }

    /// Every entry this build does not model, by type number.
    pub fn unrecognized(&self) -> Vec<u16> {
        self.entries
            .iter()
            .map(|(kind, _)| *kind)
            .filter(|kind| extension_name(*kind) == "unrecognized")
            .collect()
    }
}

/// Walk the TLV extension list.
///
/// A malformed or truncated list yields what was parsed up to that point and
/// records why it stopped. This is the same tolerance the adapter has always
/// had, and the reason it is safe is unchanged: extension parsing informs the
/// report, while the economic verdict rests on the base fields and on proved
/// balances.
pub fn extensions(data: &[u8]) -> ExtensionList {
    let mut list = ExtensionList::default();
    let mut offset = TLV_START;
    while offset + 4 <= data.len() {
        let (Some(kind), Some(length)) = (u16_at(data, offset), u16_at(data, offset + 2)) else {
            list.truncated_at = Some(MalformedReason::Truncated {
                needed: offset + 4,
                found: data.len(),
            });
            break;
        };
        // A zero type with a zero length is the end-of-list marker, not an
        // error: Token-2022 zero-pads the tail of a reallocated account.
        if kind == 0 && length == 0 {
            break;
        }
        let start = offset + 4;
        let end = start + usize::from(length);
        if end > data.len() {
            list.truncated_at = Some(MalformedReason::ExtensionOverrun {
                at: offset,
                declared: usize::from(length),
            });
            break;
        }
        list.entries.push((kind, data[start..end].to_vec()));
        offset = end;
    }
    list
}

/// Decode one TLV entry into the richest form this build models.
pub fn decode_extension(kind: u16, value: &[u8]) -> Extension {
    match kind {
        TRANSFER_FEE_CONFIG => {
            match (
                u64_at(value, FEE_CONFIG_WITHHELD),
                transfer_fee_at(value, FEE_CONFIG_OLDER),
                transfer_fee_at(value, FEE_CONFIG_NEWER),
            ) {
                (Some(withheld_amount), Some(older), Some(newer)) => Extension::TransferFeeConfig {
                    withheld_amount,
                    older,
                    newer,
                },
                // Short value bytes: keep it visible rather than reporting a
                // zero fee, which would read as "this mint charges nothing".
                _ => Extension::Unrecognized {
                    kind,
                    bytes: value.to_vec(),
                },
            }
        }
        TRANSFER_FEE_AMOUNT => match u64_at(value, 0) {
            Some(withheld_amount) => Extension::TransferFeeAmount { withheld_amount },
            None => Extension::Unrecognized {
                kind,
                bytes: value.to_vec(),
            },
        },
        TRANSFER_HOOK => Extension::TransferHook {
            program_id: optional_pubkey(value, 32),
        },
        PERMANENT_DELEGATE => Extension::PermanentDelegate {
            delegate: optional_pubkey(value, 0),
        },
        PAUSABLE => match u8_at(value, 32) {
            Some(byte) => Extension::Pausable { paused: byte == 1 },
            None => Extension::Unrecognized {
                kind,
                bytes: value.to_vec(),
            },
        },
        DEFAULT_ACCOUNT_STATE => match u8_at(value, 0) {
            Some(state) => Extension::DefaultAccountState { state },
            None => Extension::Unrecognized {
                kind,
                bytes: value.to_vec(),
            },
        },
        SCALED_UI_AMOUNT => Extension::ScaledUiAmount {
            multiplier_bytes: value.to_vec(),
        },
        _ => Extension::Unrecognized {
            kind,
            bytes: value.to_vec(),
        },
    }
}

/// An `OptionalNonZeroPubkey`: 32 bytes, all-zero meaning absent.
fn optional_pubkey(value: &[u8], offset: usize) -> Option<String> {
    let bytes = value.get(offset..offset + 32)?;
    if bytes.iter().all(|byte| *byte == 0) {
        return None;
    }
    Some(bs58::encode(bytes).into_string())
}

/// A Token-2022 token account: the base layout plus its extensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtendedAccount {
    pub base: TokenAccount,
    pub extensions: ExtensionList,
}

/// A Token-2022 mint: the base layout plus its extensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtendedMint {
    pub base: Mint,
    pub extensions: ExtensionList,
}

pub fn decode_account(data: &[u8]) -> Decoded<ExtendedAccount> {
    match layout_of(data) {
        Some(Layout::Account) => match spl_token::decode_account_base(data) {
            Decoded::Decoded(base) => Decoded::Decoded(ExtendedAccount {
                base,
                extensions: extensions(data),
            }),
            Decoded::Malformed(reason) => Decoded::Malformed(reason),
            Decoded::NotApplicable => Decoded::NotApplicable,
            Decoded::Unsupported(what) => Decoded::Unsupported(what),
        },
        Some(Layout::Mint) => Decoded::NotApplicable,
        None => Decoded::NotApplicable,
    }
}

pub fn decode_mint(data: &[u8]) -> Decoded<ExtendedMint> {
    match layout_of(data) {
        Some(Layout::Mint) => match spl_token::decode_mint_base(data) {
            Decoded::Decoded(base) => Decoded::Decoded(ExtendedMint {
                base,
                extensions: extensions(data),
            }),
            Decoded::Malformed(reason) => Decoded::Malformed(reason),
            Decoded::NotApplicable => Decoded::NotApplicable,
            Decoded::Unsupported(what) => Decoded::Unsupported(what),
        },
        Some(Layout::Account) => Decoded::NotApplicable,
        None => Decoded::NotApplicable,
    }
}

/// The fee schedule currently recorded as `newer`.
///
/// The epoch-dependent choice between the two schedules is deliberately not
/// modelled here: it feeds ranking, never an economic verdict, which rests on
/// executing the real program.
pub fn newer_transfer_fee(mint_data: &[u8]) -> Option<(u16, u64)> {
    let list = extensions(mint_data);
    let value = list.value(TRANSFER_FEE_CONFIG)?;
    let newer = transfer_fee_at(value, FEE_CONFIG_NEWER)?;
    Some((newer.basis_points, newer.maximum_fee))
}

pub fn provenance() -> SchemaProvenance {
    SchemaProvenance::StandardProgram
}

// ---------------------------------------------------------------------------
// Narrow accessors, matching `spl_token`'s but accepting extended buffers
// ---------------------------------------------------------------------------

pub fn account_amount(data: &[u8]) -> Option<u64> {
    (layout_of(data)? == Layout::Account)
        .then(|| u64_at(data, spl_token::ACCOUNT_AMOUNT))
        .flatten()
}

pub fn account_mint(data: &[u8]) -> Option<String> {
    (layout_of(data)? == Layout::Account)
        .then(|| super::address_at(data, spl_token::ACCOUNT_MINT))
        .flatten()
}

pub fn mint_decimals(data: &[u8]) -> Option<u8> {
    (layout_of(data)? == Layout::Mint)
        .then(|| u8_at(data, spl_token::MINT_DECIMALS))
        .flatten()
}

pub fn mint_supply(data: &[u8]) -> Option<u64> {
    (layout_of(data)? == Layout::Mint)
        .then(|| u64_at(data, spl_token::MINT_SUPPLY))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_account(amount: u64) -> Vec<u8> {
        let mut data = vec![0_u8; spl_token::ACCOUNT_LEN];
        data[spl_token::ACCOUNT_MINT..spl_token::ACCOUNT_MINT + 32].copy_from_slice(&[1_u8; 32]);
        data[spl_token::ACCOUNT_OWNER..spl_token::ACCOUNT_OWNER + 32].copy_from_slice(&[2_u8; 32]);
        data[spl_token::ACCOUNT_AMOUNT..spl_token::ACCOUNT_AMOUNT + 8]
            .copy_from_slice(&amount.to_le_bytes());
        data[spl_token::ACCOUNT_STATE] = 1;
        data
    }

    fn base_mint(decimals: u8) -> Vec<u8> {
        let mut data = vec![0_u8; spl_token::MINT_LEN];
        data[spl_token::MINT_DECIMALS] = decimals;
        data[spl_token::MINT_IS_INITIALIZED] = 1;
        data
    }

    /// Pad a base structure to the extended form and append TLV entries.
    fn extended(base: Vec<u8>, account_type: u8, entries: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut data = base;
        data.resize(ACCOUNT_TYPE_OFFSET, 0);
        data.push(account_type);
        for (kind, value) in entries {
            data.extend_from_slice(&kind.to_le_bytes());
            data.extend_from_slice(&(value.len() as u16).to_le_bytes());
            data.extend_from_slice(value);
        }
        data
    }

    fn fee_config(withheld: u64, older_bps: u16, newer_bps: u16, newer_max: u64) -> Vec<u8> {
        let mut value = vec![0_u8; 108];
        value[FEE_CONFIG_WITHHELD..FEE_CONFIG_WITHHELD + 8]
            .copy_from_slice(&withheld.to_le_bytes());
        value[FEE_CONFIG_OLDER + FEE_RECORD_BASIS_POINTS
            ..FEE_CONFIG_OLDER + FEE_RECORD_BASIS_POINTS + 2]
            .copy_from_slice(&older_bps.to_le_bytes());
        value[FEE_CONFIG_NEWER..FEE_CONFIG_NEWER + 8].copy_from_slice(&42_u64.to_le_bytes());
        value[FEE_CONFIG_NEWER + FEE_RECORD_MAXIMUM..FEE_CONFIG_NEWER + FEE_RECORD_MAXIMUM + 8]
            .copy_from_slice(&newer_max.to_le_bytes());
        value[FEE_CONFIG_NEWER + FEE_RECORD_BASIS_POINTS
            ..FEE_CONFIG_NEWER + FEE_RECORD_BASIS_POINTS + 2]
            .copy_from_slice(&newer_bps.to_le_bytes());
        value
    }

    #[test]
    fn the_two_base_lengths_and_the_type_byte_classify_every_buffer() {
        assert_eq!(layout_of(&base_mint(6)), Some(Layout::Mint));
        assert_eq!(layout_of(&base_account(0)), Some(Layout::Account));
        assert_eq!(
            layout_of(&extended(base_mint(6), ACCOUNT_TYPE_MINT, &[])),
            Some(Layout::Mint)
        );
        assert_eq!(
            layout_of(&extended(base_account(0), ACCOUNT_TYPE_ACCOUNT, &[])),
            Some(Layout::Account)
        );
        // Neither length, and no type byte to disambiguate.
        assert_eq!(layout_of(&[0_u8; 100]), None);
        assert_eq!(layout_of(&[]), None);
        // A type byte naming nothing.
        let mut odd = extended(base_account(0), ACCOUNT_TYPE_ACCOUNT, &[]);
        odd[ACCOUNT_TYPE_OFFSET] = 9;
        assert_eq!(layout_of(&odd), None);
    }

    #[test]
    fn a_base_account_decodes_with_an_empty_extension_list() {
        let decoded = decode_account(&base_account(500)).ok().expect("decodes");
        assert_eq!(decoded.base.amount, 500);
        assert!(decoded.extensions.entries.is_empty());
        assert_eq!(decoded.extensions.truncated_at, None);
    }

    #[test]
    fn a_transfer_fee_config_decodes_both_schedules() {
        let mint = extended(
            base_mint(6),
            ACCOUNT_TYPE_MINT,
            &[(TRANSFER_FEE_CONFIG, fee_config(9_000, 50, 150, 5_000))],
        );
        let decoded = decode_mint(&mint).ok().expect("decodes");
        let value = decoded
            .extensions
            .value(TRANSFER_FEE_CONFIG)
            .expect("entry");
        match decode_extension(TRANSFER_FEE_CONFIG, value) {
            Extension::TransferFeeConfig {
                withheld_amount,
                older,
                newer,
            } => {
                assert_eq!(withheld_amount, 9_000);
                assert_eq!(older.basis_points, 50);
                assert_eq!(newer.basis_points, 150);
                assert_eq!(newer.maximum_fee, 5_000);
                assert_eq!(newer.epoch, 42);
            }
            other => panic!("expected a fee config, got {other:?}"),
        }
        assert_eq!(newer_transfer_fee(&mint), Some((150, 5_000)));
    }

    #[test]
    fn a_withheld_fee_on_an_account_decodes() {
        let account = extended(
            base_account(100),
            ACCOUNT_TYPE_ACCOUNT,
            &[(TRANSFER_FEE_AMOUNT, 7_777_u64.to_le_bytes().to_vec())],
        );
        let decoded = decode_account(&account).ok().expect("decodes");
        let value = decoded
            .extensions
            .value(TRANSFER_FEE_AMOUNT)
            .expect("entry");
        assert_eq!(
            decode_extension(TRANSFER_FEE_AMOUNT, value),
            Extension::TransferFeeAmount {
                withheld_amount: 7_777
            }
        );
    }

    #[test]
    fn a_transfer_hook_reports_its_program_and_its_absence() {
        let mut value = vec![0_u8; 64];
        value[32..64].copy_from_slice(&[5_u8; 32]);
        assert_eq!(
            decode_extension(TRANSFER_HOOK, &value),
            Extension::TransferHook {
                program_id: Some(bs58::encode([5_u8; 32]).into_string())
            }
        );
        // An all-zero OptionalNonZeroPubkey is absent, not an address of zeros.
        assert_eq!(
            decode_extension(TRANSFER_HOOK, &[0_u8; 64]),
            Extension::TransferHook { program_id: None }
        );
    }

    #[test]
    fn a_permanent_delegate_decodes() {
        let value = [6_u8; 32].to_vec();
        assert_eq!(
            decode_extension(PERMANENT_DELEGATE, &value),
            Extension::PermanentDelegate {
                delegate: Some(bs58::encode([6_u8; 32]).into_string())
            }
        );
    }

    #[test]
    fn pause_state_decodes_in_both_directions() {
        let mut value = vec![0_u8; 33];
        assert_eq!(
            decode_extension(PAUSABLE, &value),
            Extension::Pausable { paused: false }
        );
        value[32] = 1;
        assert_eq!(
            decode_extension(PAUSABLE, &value),
            Extension::Pausable { paused: true }
        );
    }

    #[test]
    fn a_default_account_state_of_frozen_decodes() {
        assert_eq!(
            decode_extension(DEFAULT_ACCOUNT_STATE, &[2_u8]),
            Extension::DefaultAccountState { state: 2 }
        );
    }

    /// The rule that matters most in this module: an extension this build does
    /// not model must stay visible, with its bytes.
    #[test]
    fn an_unknown_extension_is_reported_not_dropped() {
        let account = extended(
            base_account(1),
            ACCOUNT_TYPE_ACCOUNT,
            &[(999, vec![1, 2, 3, 4])],
        );
        let list = extensions(&account);
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.names(), vec!["unrecognized"]);
        assert_eq!(list.unrecognized(), vec![999]);
        assert_eq!(
            decode_extension(999, &[1, 2, 3, 4]),
            Extension::Unrecognized {
                kind: 999,
                bytes: vec![1, 2, 3, 4]
            }
        );
        // And the account still decodes: an unknown extension does not make the
        // base layout unreadable.
        assert_eq!(
            decode_account(&account).ok().expect("decodes").base.amount,
            1
        );
    }

    #[test]
    fn a_malformed_extension_layout_records_why_the_walk_stopped() {
        let mut account = extended(base_account(1), ACCOUNT_TYPE_ACCOUNT, &[]);
        // A TLV header declaring 64 bytes of value with none following.
        account.extend_from_slice(&TRANSFER_FEE_AMOUNT.to_le_bytes());
        account.extend_from_slice(&64_u16.to_le_bytes());
        let list = extensions(&account);
        assert!(list.entries.is_empty());
        assert_eq!(
            list.truncated_at,
            Some(MalformedReason::ExtensionOverrun {
                at: TLV_START,
                declared: 64
            })
        );
    }

    /// A short fee-config value must not read as a mint that charges nothing.
    #[test]
    fn a_truncated_fee_config_is_unrecognized_rather_than_zero() {
        let short = vec![0_u8; 80];
        assert!(matches!(
            decode_extension(TRANSFER_FEE_CONFIG, &short),
            Extension::Unrecognized { .. }
        ));
    }

    #[test]
    fn extension_order_is_preserved_as_written() {
        let account = extended(
            base_account(1),
            ACCOUNT_TYPE_ACCOUNT,
            &[
                (TRANSFER_FEE_AMOUNT, 1_u64.to_le_bytes().to_vec()),
                (7, Vec::new()),
                (999, vec![9]),
            ],
        );
        assert_eq!(
            extensions(&account).names(),
            vec!["transfer-fee-amount", "immutable-owner", "unrecognized"]
        );
    }

    #[test]
    fn a_zero_terminator_ends_the_list_without_being_an_error() {
        let mut account = extended(
            base_account(1),
            ACCOUNT_TYPE_ACCOUNT,
            &[(TRANSFER_FEE_AMOUNT, 5_u64.to_le_bytes().to_vec())],
        );
        account.extend_from_slice(&[0_u8; 16]);
        let list = extensions(&account);
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.truncated_at, None, "padding is not a malformation");
    }

    #[test]
    fn a_mint_does_not_decode_as_an_account_or_the_reverse() {
        assert_eq!(decode_account(&base_mint(6)), Decoded::NotApplicable);
        assert_eq!(decode_mint(&base_account(0)), Decoded::NotApplicable);
    }

    #[test]
    fn extended_accessors_accept_what_the_legacy_ones_refuse() {
        let account = extended(
            base_account(4_242),
            ACCOUNT_TYPE_ACCOUNT,
            &[(TRANSFER_FEE_AMOUNT, 1_u64.to_le_bytes().to_vec())],
        );
        assert_eq!(account_amount(&account), Some(4_242));
        // The legacy decoder refuses it, on purpose: it is not a legacy account.
        assert_eq!(spl_token::account_amount(&account), None);
    }
}
