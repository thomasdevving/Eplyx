//! The SPL Token program's account layouts.
//!
//! One implementation, shared. Before Phase U1 the 165-byte token account was
//! parsed in `protocol/stake_pool.rs` and again in `protocol/token2022.rs`, each
//! with its own constants and its own idea of which lengths were acceptable.
//!
//! Layout provenance is [`SchemaProvenance::StandardProgram`]: these offsets are
//! `spl_token::state::Account` and `spl_token::state::Mint`, whose binary is the
//! hash-pinned dependency a bundle carries. That is the strongest claim this
//! layer makes, and it is still a claim about a *layout*, not a proof that the
//! deployed bytecode implements it.

use super::{
    address_at, coption_address_at, u64_at, u8_at, Decoded, MalformedReason, SchemaProvenance,
};

pub const PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// `spl_token::state::Account::LEN`. A legacy token account is exactly this
/// long — never shorter, never longer.
pub const ACCOUNT_LEN: usize = 165;
/// `spl_token::state::Mint::LEN`.
pub const MINT_LEN: usize = 82;

// Field offsets, stated once. Each is `spl_token`'s own declaration order.
pub const ACCOUNT_MINT: usize = 0;
pub const ACCOUNT_OWNER: usize = 32;
pub const ACCOUNT_AMOUNT: usize = 64;
pub const ACCOUNT_DELEGATE: usize = 72;
pub const ACCOUNT_STATE: usize = 108;
pub const ACCOUNT_IS_NATIVE: usize = 109;
pub const ACCOUNT_DELEGATED_AMOUNT: usize = 121;
pub const ACCOUNT_CLOSE_AUTHORITY: usize = 129;

pub const MINT_MINT_AUTHORITY: usize = 0;
pub const MINT_SUPPLY: usize = 36;
pub const MINT_DECIMALS: usize = 44;
pub const MINT_IS_INITIALIZED: usize = 45;
pub const MINT_FREEZE_AUTHORITY: usize = 46;

/// `spl_token::state::AccountState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountState {
    Uninitialized,
    Initialized,
    Frozen,
}

impl AccountState {
    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0 => Self::Uninitialized,
            1 => Self::Initialized,
            2 => Self::Frozen,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uninitialized => "uninitialized",
            Self::Initialized => "initialized",
            Self::Frozen => "frozen",
        }
    }

    /// The raw byte, which is what a decoded report has always shown.
    pub fn as_byte(self) -> u8 {
        match self {
            Self::Uninitialized => 0,
            Self::Initialized => 1,
            Self::Frozen => 2,
        }
    }
}

/// A decoded SPL Token account. Every field the layout defines, none inferred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenAccount {
    pub mint: String,
    pub owner: String,
    pub amount: u64,
    pub delegate: Option<String>,
    pub state: AccountState,
    /// `Some(rent_exempt_reserve)` for a wrapped-SOL account.
    pub native_reserve: Option<u64>,
    pub delegated_amount: u64,
    pub close_authority: Option<String>,
}

/// A decoded SPL Token mint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mint {
    pub mint_authority: Option<String>,
    pub supply: u64,
    pub decimals: u8,
    pub is_initialized: bool,
    pub freeze_authority: Option<String>,
}

/// Decode the base account layout out of a buffer of at least [`ACCOUNT_LEN`].
///
/// Separated from [`decode_account`] so Token-2022, whose accounts carry the
/// same base layout followed by a TLV extension list, reads the base exactly
/// the way the legacy program does rather than re-deriving it.
pub fn decode_account_base(data: &[u8]) -> Decoded<TokenAccount> {
    if data.len() < ACCOUNT_LEN {
        return Decoded::Malformed(MalformedReason::Truncated {
            needed: ACCOUNT_LEN,
            found: data.len(),
        });
    }
    let (Some(mint), Some(owner), Some(amount)) = (
        address_at(data, ACCOUNT_MINT),
        address_at(data, ACCOUNT_OWNER),
        u64_at(data, ACCOUNT_AMOUNT),
    ) else {
        return Decoded::Malformed(MalformedReason::Truncated {
            needed: ACCOUNT_LEN,
            found: data.len(),
        });
    };
    let delegate = match coption_address_at(data, ACCOUNT_DELEGATE) {
        Ok(delegate) => delegate,
        Err(reason) => return Decoded::Malformed(reason),
    };
    let Some(state) = u8_at(data, ACCOUNT_STATE).and_then(AccountState::from_byte) else {
        return Decoded::Malformed(MalformedReason::InvalidDiscriminant {
            at: ACCOUNT_STATE,
            value: u32::from(u8_at(data, ACCOUNT_STATE).unwrap_or(0)),
        });
    };
    // `is_native` is a COption<u64>: the tag, then the rent-exempt reserve.
    let native_reserve = match super::u32_at(data, ACCOUNT_IS_NATIVE) {
        Some(0) => None,
        Some(1) => u64_at(data, ACCOUNT_IS_NATIVE + 4),
        Some(value) => {
            return Decoded::Malformed(MalformedReason::InvalidDiscriminant {
                at: ACCOUNT_IS_NATIVE,
                value,
            })
        }
        None => {
            return Decoded::Malformed(MalformedReason::Truncated {
                needed: ACCOUNT_IS_NATIVE + 4,
                found: data.len(),
            })
        }
    };
    let Some(delegated_amount) = u64_at(data, ACCOUNT_DELEGATED_AMOUNT) else {
        return Decoded::Malformed(MalformedReason::Truncated {
            needed: ACCOUNT_DELEGATED_AMOUNT + 8,
            found: data.len(),
        });
    };
    let close_authority = match coption_address_at(data, ACCOUNT_CLOSE_AUTHORITY) {
        Ok(authority) => authority,
        Err(reason) => return Decoded::Malformed(reason),
    };
    Decoded::Decoded(TokenAccount {
        mint,
        owner,
        amount,
        delegate,
        state,
        native_reserve,
        delegated_amount,
        close_authority,
    })
}

/// Decode a legacy SPL Token account.
///
/// Strict on length: a legacy token account is exactly [`ACCOUNT_LEN`] bytes.
/// A longer buffer is a Token-2022 account and belongs to
/// [`super::token2022::decode_account`] — decoding it here would let one
/// program's extended state be read under another program's narrower claim.
pub fn decode_account(data: &[u8]) -> Decoded<TokenAccount> {
    if data.len() != ACCOUNT_LEN {
        return Decoded::NotApplicable;
    }
    decode_account_base(data)
}

pub fn decode_mint_base(data: &[u8]) -> Decoded<Mint> {
    if data.len() < MINT_LEN {
        return Decoded::Malformed(MalformedReason::Truncated {
            needed: MINT_LEN,
            found: data.len(),
        });
    }
    let mint_authority = match coption_address_at(data, MINT_MINT_AUTHORITY) {
        Ok(authority) => authority,
        Err(reason) => return Decoded::Malformed(reason),
    };
    let (Some(supply), Some(decimals), Some(initialized)) = (
        u64_at(data, MINT_SUPPLY),
        u8_at(data, MINT_DECIMALS),
        u8_at(data, MINT_IS_INITIALIZED),
    ) else {
        return Decoded::Malformed(MalformedReason::Truncated {
            needed: MINT_LEN,
            found: data.len(),
        });
    };
    if initialized > 1 {
        return Decoded::Malformed(MalformedReason::InvalidDiscriminant {
            at: MINT_IS_INITIALIZED,
            value: u32::from(initialized),
        });
    }
    let freeze_authority = match coption_address_at(data, MINT_FREEZE_AUTHORITY) {
        Ok(authority) => authority,
        Err(reason) => return Decoded::Malformed(reason),
    };
    Decoded::Decoded(Mint {
        mint_authority,
        supply,
        decimals,
        is_initialized: initialized == 1,
        freeze_authority,
    })
}

/// Decode a legacy SPL Token mint. Strict on length, for the same reason
/// [`decode_account`] is.
pub fn decode_mint(data: &[u8]) -> Decoded<Mint> {
    if data.len() != MINT_LEN {
        return Decoded::NotApplicable;
    }
    decode_mint_base(data)
}

/// The layout claim this module makes.
pub fn provenance() -> SchemaProvenance {
    SchemaProvenance::StandardProgram
}

// ---------------------------------------------------------------------------
// Narrow accessors
// ---------------------------------------------------------------------------
//
// The adapters read one field far more often than they read a whole account,
// and threading a full decode through every call site would change a great deal
// of code to no purpose. These keep the old shape — `Option<T>` — while routing
// every byte through the decoder above.

pub fn account_amount(data: &[u8]) -> Option<u64> {
    decode_account(data).ok().map(|account| account.amount)
}

pub fn account_mint(data: &[u8]) -> Option<String> {
    decode_account(data).ok().map(|account| account.mint)
}

pub fn mint_decimals(data: &[u8]) -> Option<u8> {
    decode_mint(data).ok().map(|mint| mint.decimals)
}

pub fn mint_supply(data: &[u8]) -> Option<u64> {
    decode_mint(data).ok().map(|mint| mint.supply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(byte: u8) -> String {
        bs58::encode([byte; 32]).into_string()
    }

    pub(super) fn token_account_bytes(amount: u64) -> Vec<u8> {
        let mut data = vec![0_u8; ACCOUNT_LEN];
        data[ACCOUNT_MINT..ACCOUNT_MINT + 32].copy_from_slice(&[1_u8; 32]);
        data[ACCOUNT_OWNER..ACCOUNT_OWNER + 32].copy_from_slice(&[2_u8; 32]);
        data[ACCOUNT_AMOUNT..ACCOUNT_AMOUNT + 8].copy_from_slice(&amount.to_le_bytes());
        data[ACCOUNT_STATE] = 1;
        data
    }

    fn mint_bytes(supply: u64, decimals: u8) -> Vec<u8> {
        let mut data = vec![0_u8; MINT_LEN];
        data[MINT_SUPPLY..MINT_SUPPLY + 8].copy_from_slice(&supply.to_le_bytes());
        data[MINT_DECIMALS] = decimals;
        data[MINT_IS_INITIALIZED] = 1;
        data
    }

    #[test]
    fn a_valid_account_decodes_every_declared_field() {
        let account = decode_account(&token_account_bytes(5_000_000))
            .ok()
            .expect("valid account");
        assert_eq!(account.mint, address(1));
        assert_eq!(account.owner, address(2));
        assert_eq!(account.amount, 5_000_000);
        assert_eq!(account.delegate, None);
        assert_eq!(account.state, AccountState::Initialized);
        assert_eq!(account.native_reserve, None);
        assert_eq!(account.delegated_amount, 0);
        assert_eq!(account.close_authority, None);
    }

    #[test]
    fn a_valid_mint_decodes_every_declared_field() {
        let mint = decode_mint(&mint_bytes(1_000, 6)).ok().expect("valid mint");
        assert_eq!(mint.supply, 1_000);
        assert_eq!(mint.decimals, 6);
        assert!(mint.is_initialized);
        assert_eq!(mint.mint_authority, None);
        assert_eq!(mint.freeze_authority, None);
    }

    #[test]
    fn an_uninitialized_account_is_decoded_not_refused() {
        let mut data = token_account_bytes(0);
        data[ACCOUNT_STATE] = 0;
        let account = decode_account(&data).ok().expect("decodes");
        assert_eq!(account.state, AccountState::Uninitialized);
        // Uninitialized is a real state the program writes. It is not malformed,
        // and it is not the same as the account being absent.
        assert_eq!(account.amount, 0);
    }

    #[test]
    fn a_frozen_account_reports_its_state() {
        let mut data = token_account_bytes(7);
        data[ACCOUNT_STATE] = 2;
        assert_eq!(
            decode_account(&data).ok().expect("decodes").state,
            AccountState::Frozen
        );
    }

    #[test]
    fn an_account_state_byte_outside_the_enum_is_malformed() {
        let mut data = token_account_bytes(7);
        data[ACCOUNT_STATE] = 9;
        assert!(decode_account(&data).is_malformed());
    }

    #[test]
    fn a_delegate_and_its_delegated_amount_decode_together() {
        let mut data = token_account_bytes(500);
        data[ACCOUNT_DELEGATE..ACCOUNT_DELEGATE + 4].copy_from_slice(&1_u32.to_le_bytes());
        data[ACCOUNT_DELEGATE + 4..ACCOUNT_DELEGATE + 36].copy_from_slice(&[3_u8; 32]);
        data[ACCOUNT_DELEGATED_AMOUNT..ACCOUNT_DELEGATED_AMOUNT + 8]
            .copy_from_slice(&250_u64.to_le_bytes());
        let account = decode_account(&data).ok().expect("decodes");
        assert_eq!(account.delegate, Some(address(3)));
        assert_eq!(account.delegated_amount, 250);
    }

    #[test]
    fn a_close_authority_decodes() {
        let mut data = token_account_bytes(1);
        data[ACCOUNT_CLOSE_AUTHORITY..ACCOUNT_CLOSE_AUTHORITY + 4]
            .copy_from_slice(&1_u32.to_le_bytes());
        data[ACCOUNT_CLOSE_AUTHORITY + 4..ACCOUNT_CLOSE_AUTHORITY + 36]
            .copy_from_slice(&[4_u8; 32]);
        assert_eq!(
            decode_account(&data).ok().expect("decodes").close_authority,
            Some(address(4))
        );
    }

    #[test]
    fn a_wrapped_sol_account_reports_its_rent_exempt_reserve() {
        let mut data = token_account_bytes(2_000);
        data[ACCOUNT_IS_NATIVE..ACCOUNT_IS_NATIVE + 4].copy_from_slice(&1_u32.to_le_bytes());
        data[ACCOUNT_IS_NATIVE + 4..ACCOUNT_IS_NATIVE + 12]
            .copy_from_slice(&2_039_280_u64.to_le_bytes());
        assert_eq!(
            decode_account(&data).ok().expect("decodes").native_reserve,
            Some(2_039_280)
        );
    }

    /// The brief's rule: a malformed account must be visible, never silently
    /// accepted and never indistinguishable from an absent one.
    #[test]
    fn a_wrong_length_buffer_is_not_applicable_rather_than_zero() {
        for length in [0, 1, 82, 164, 166, 611] {
            let data = vec![0_u8; length];
            assert_eq!(
                decode_account(&data),
                Decoded::NotApplicable,
                "length {length}"
            );
            assert_eq!(account_amount(&data), None, "length {length}");
        }
    }

    /// The specific historical defect this length check exists for: a real
    /// 611-byte stake-pool account must never decode as a token account.
    #[test]
    fn a_stake_pool_account_does_not_decode_as_a_token_account() {
        let mut data = vec![0_u8; 611];
        data[0] = 1; // AccountType::StakePool
        assert_eq!(decode_account(&data), Decoded::NotApplicable);
        assert_eq!(decode_mint(&data), Decoded::NotApplicable);
    }

    #[test]
    fn a_truncated_base_layout_is_malformed_rather_than_absent() {
        // Reached through the base decoder, which is what Token-2022 uses when
        // a buffer claims to be an extended account but is too short.
        let data = vec![0_u8; 100];
        assert_eq!(
            decode_account_base(&data),
            Decoded::Malformed(MalformedReason::Truncated {
                needed: ACCOUNT_LEN,
                found: 100
            })
        );
    }

    #[test]
    fn zero_and_maximum_amounts_both_round_trip() {
        for amount in [0_u64, 1, u64::MAX] {
            assert_eq!(account_amount(&token_account_bytes(amount)), Some(amount));
        }
    }

    #[test]
    fn maximum_supply_round_trips() {
        assert_eq!(mint_supply(&mint_bytes(u64::MAX, 9)), Some(u64::MAX));
        assert_eq!(mint_decimals(&mint_bytes(0, 255)), Some(255));
    }

    #[test]
    fn an_uninitialized_mint_still_decodes_and_says_so() {
        let mut data = mint_bytes(0, 0);
        data[MINT_IS_INITIALIZED] = 0;
        let mint = decode_mint(&data).ok().expect("decodes");
        assert!(!mint.is_initialized);
    }

    #[test]
    fn an_initialized_flag_outside_its_range_is_malformed() {
        let mut data = mint_bytes(1, 6);
        data[MINT_IS_INITIALIZED] = 7;
        assert!(decode_mint(&data).is_malformed());
    }

    #[test]
    fn the_narrow_accessors_agree_with_the_full_decode() {
        let data = token_account_bytes(123_456);
        assert_eq!(
            account_amount(&data),
            decode_account(&data).ok().map(|a| a.amount)
        );
        assert_eq!(
            account_mint(&data),
            decode_account(&data).ok().map(|a| a.mint)
        );
        let mint = mint_bytes(999, 8);
        assert_eq!(
            mint_supply(&mint),
            decode_mint(&mint).ok().map(|m| m.supply)
        );
        assert_eq!(
            mint_decimals(&mint),
            decode_mint(&mint).ok().map(|m| m.decimals)
        );
    }
}
