//! Wire format for the fixture lending protocol.
//!
//! This crate is deliberately free of any Solana or feature-flag dependency. It is
//! compiled *once* and linked by both the on-chain program and the host-side
//! differential engine. That is what makes the "identical public interface" claim
//! structural rather than aspirational: V1 and V2 cannot drift in instruction
//! encoding or account layout, because neither owns the definition.
//!
//! Units used throughout:
//!   * collateral  - lamports (1 SOL = 1_000_000_000)
//!   * debt        - micro-USD (1 USD = 1_000_000)
//!   * price       - micro-USD per SOL
//!   * bps         - basis points (10_000 = 100%)
//!   * health      - fixed point, HEALTH_SCALE = 1.0

#![deny(unsafe_code)]

use borsh::{BorshDeserialize, BorshSerialize};

/// 32-byte account address. Kept as a plain byte array so this crate never has to
/// agree with anyone about which `solana-*` crate version owns `Pubkey`.
pub type Address = [u8; 32];

pub const LAMPORTS_PER_SOL: u128 = 1_000_000_000;
pub const USD_SCALE: u128 = 1_000_000;
pub const BPS_DENOMINATOR: u128 = 10_000;

/// Fixed-point scale for the health factor. `HEALTH_SCALE` == 1.0.
pub const HEALTH_SCALE: u64 = 1_000_000;

/// Health factor reported for a position carrying no debt.
pub const HEALTH_INFINITE: u64 = u64::MAX;

/// Decimal exponent of the collateral asset (SOL: 1e9 lamports per SOL).
pub const COLLATERAL_DECIMALS: u8 = 9;

/// Decimal exponent of the debt asset (USD-denominated, 1e6 per whole unit).
pub const DEBT_DECIMALS: u8 = 6;

/// The fixture protocol's debt asset is USD-pegged: one whole unit is $1.00,
/// expressed in micro-USD. Held as a constant rather than per-position state
/// because it is a property of the asset, not of any individual position.
pub const DEBT_PRICE_MICRO_USD: u64 = 1_000_000;

pub const ACCOUNT_TAG_MARKET: u8 = 1;
pub const ACCOUNT_TAG_POSITION: u8 = 2;

pub const MARKET_LEN: usize = 86;
pub const POSITION_LEN: usize = 110;

/// Seed for the market's collateral vault PDA.
pub const VAULT_SEED: &[u8] = b"vault";

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Market {
    pub tag: u8,
    pub version: u8,
    pub authority: Address,
    pub vault: Address,
    /// micro-USD per SOL.
    pub collateral_price: u64,
    /// Above this loan-to-value the position may be liquidated.
    pub liquidation_threshold_bps: u16,
    /// Borrow/withdraw ceiling. Stricter than the liquidation threshold.
    pub max_ltv_bps: u16,
    pub position_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Position {
    pub tag: u8,
    pub version: u8,
    pub owner: Address,
    pub market: Address,
    /// Lamports of SOL collateral held for this position by the market vault.
    pub collateral_amount: u64,
    /// micro-USD of outstanding debt.
    pub debt_amount: u64,
    /// Price snapshot the cached `health_factor` was computed at.
    pub collateral_price: u64,
    pub liquidation_threshold_bps: u16,
    pub max_ltv_bps: u16,
    /// Cached risk metric, `HEALTH_SCALE`-fixed-point. Recomputed by every
    /// state-mutating instruction. This is the field the differential engine
    /// watches most closely.
    pub health_factor: u64,
    pub last_update_slot: u64,
}

/// The full public instruction set. Identical for V1 and V2 by construction.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum LendingInstruction {
    /// `[writable] market, [signer] authority`
    InitializeMarket {
        collateral_price: u64,
        liquidation_threshold_bps: u16,
        max_ltv_bps: u16,
    },
    /// `[writable] position, [] market, [signer] owner`
    CreatePosition,
    /// `[writable] position, [] market, [writable] vault, [signer, writable] owner, [] system_program`
    DepositCollateral { amount: u64 },
    /// `[writable] position, [] market, [signer] owner`
    Borrow { amount: u64 },
    /// `[writable] position, [] market, [signer] owner`
    Repay { amount: u64 },
    /// `[writable] position, [] market, [writable] vault, [signer, writable] owner`
    WithdrawCollateral { amount: u64 },
    /// `[writable] position, [] market, [writable] vault, [signer, writable] liquidator`
    Liquidate { repay_amount: u64 },
    /// Permissionless re-pricing of a single position. `[writable] position, [] market`
    RefreshPosition,
    /// `[writable] market, [signer] authority`
    SetPrice { price: u64 },
}

/// Reference (ground-truth) risk math.
///
/// This is the host-side oracle the engine grades on-chain behaviour against. It
/// mirrors V1's intended semantics: widen to `u128`, multiply before dividing, and
/// never let an intermediate truncate. It is intentionally *not* shared with the
/// on-chain program — if the program imported it, V2 could not diverge and the
/// differential test would be vacuous.
pub mod reference {
    use super::*;

    /// Normalise a token amount into micro-USD.
    ///
    /// `amount` is in the asset's smallest unit, `decimals` is that asset's
    /// decimal exponent, and `price_micro_usd` is the price of one *whole*
    /// unit, itself in micro-USD. Multiplying before dividing keeps the full
    /// precision of the product through the scale conversion - the same
    /// discipline whose absence is the seeded V2 regression.
    ///
    /// Integer-only by construction: there is no floating point anywhere in
    /// this path.
    pub fn value_micro_usd(amount: u64, decimals: u8, price_micro_usd: u64) -> u128 {
        // A u64*u64 product is below 10^39. Higher decimal exponents
        // therefore round to zero without constructing an overflowing scale.
        let Some(scale) = 10u128.checked_pow(decimals as u32) else {
            return 0;
        };
        (amount as u128) * (price_micro_usd as u128) / scale
    }

    /// Collateral value in micro-USD.
    pub fn collateral_value(collateral_lamports: u64, price: u64) -> u128 {
        value_micro_usd(collateral_lamports, COLLATERAL_DECIMALS, price)
    }

    /// Debt value in micro-USD. The debt asset is USD-pegged, so this is an
    /// identity today; it is routed through the same normalisation so that a
    /// non-pegged debt asset would need no new code path.
    pub fn debt_value(debt_amount: u64) -> u128 {
        value_micro_usd(debt_amount, DEBT_DECIMALS, DEBT_PRICE_MICRO_USD)
    }

    /// Risk-adjusted collateral in micro-USD.
    pub fn adjusted_collateral(collateral_lamports: u64, price: u64, bps: u16) -> u128 {
        collateral_value(collateral_lamports, price) * (bps as u128) / BPS_DENOMINATOR
    }

    /// Health factor, `HEALTH_SCALE`-fixed-point.
    pub fn health_factor(
        collateral_lamports: u64,
        debt: u64,
        price: u64,
        liquidation_threshold_bps: u16,
    ) -> u64 {
        if debt == 0 {
            return HEALTH_INFINITE;
        }
        let adjusted = adjusted_collateral(collateral_lamports, price, liquidation_threshold_bps);
        u64::try_from(adjusted * (HEALTH_SCALE as u128) / (debt as u128)).unwrap_or(HEALTH_INFINITE)
    }

    pub fn is_liquidatable(health: u64) -> bool {
        health < HEALTH_SCALE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_sizes_are_stable() {
        let market = Market {
            tag: ACCOUNT_TAG_MARKET,
            version: 1,
            authority: [7u8; 32],
            vault: [9u8; 32],
            collateral_price: 100_000_000,
            liquidation_threshold_bps: 8_000,
            max_ltv_bps: 7_500,
            position_count: 3,
        };
        let position = Position {
            tag: ACCOUNT_TAG_POSITION,
            version: 1,
            owner: [1u8; 32],
            market: [2u8; 32],
            collateral_amount: 99_500_000_000,
            debt_amount: 7_945_700_000,
            collateral_price: 100_000_000,
            liquidation_threshold_bps: 8_000,
            max_ltv_bps: 7_500,
            health_factor: 1_001_799,
            last_update_slot: 0,
        };
        assert_eq!(borsh::to_vec(&market).unwrap().len(), MARKET_LEN);
        assert_eq!(borsh::to_vec(&position).unwrap().len(), POSITION_LEN);
    }

    #[test]
    fn reference_math_matches_worked_example() {
        // 99.5 SOL at $100.00, 80% liquidation threshold, $7,945.70 debt.
        let health = reference::health_factor(99_500_000_000, 7_945_700_000, 100_000_000, 8_000);
        assert_eq!(health, 1_001_799);
        assert!(!reference::is_liquidatable(health));
    }

    #[test]
    fn value_normalisation_handles_arbitrary_decimals() {
        // 9 decimals: 99.5 SOL at $100.00 is $9,950.00.
        assert_eq!(
            reference::value_micro_usd(99_500_000_000, 9, 100_000_000),
            9_950_000_000
        );
        // 6 decimals: a USD-pegged unit values to itself.
        assert_eq!(
            reference::value_micro_usd(7_930_000_000, 6, 1_000_000),
            7_930_000_000
        );
        // 8 decimals: 1.5 units at $60,000.00 is $90,000.00.
        assert_eq!(
            reference::value_micro_usd(150_000_000, 8, 60_000_000_000),
            90_000_000_000
        );
        // 0 decimals: 3 units at $2.50 is $7.50.
        assert_eq!(reference::value_micro_usd(3, 0, 2_500_000), 7_500_000);
    }

    #[test]
    fn debt_valuation_matches_the_pegged_amount() {
        assert_eq!(reference::debt_value(7_930_000_000), 7_930_000_000);
        assert_eq!(reference::debt_value(0), 0);
    }

    #[test]
    fn zero_debt_is_infinitely_healthy() {
        let health = reference::health_factor(1_000_000_000, 0, 100_000_000, 8_000);
        assert_eq!(health, HEALTH_INFINITE);
        assert!(!reference::is_liquidatable(health));
    }
}

/// Program error codes. Defined here (not in the program crate) so the host-side
/// engine can turn a raw `Custom(n)` back into a readable name, and so V1 and V2
/// are guaranteed to report identical codes for identical conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum LendingError {
    MathOverflow = 0,
    InvalidAccountData = 1,
    InvalidAccountOwner = 2,
    MissingSignature = 3,
    AlreadyInitialized = 4,
    UninitializedAccount = 5,
    MarketMismatch = 6,
    OwnerMismatch = 7,
    InsufficientCollateral = 8,
    LtvExceeded = 9,
    PositionHealthy = 10,
    InsufficientVaultBalance = 11,
    RepayExceedsDebt = 12,
    InvalidVault = 13,
    InvalidPrice = 14,
}

impl LendingError {
    pub fn name(self) -> &'static str {
        match self {
            Self::MathOverflow => "MathOverflow",
            Self::InvalidAccountData => "InvalidAccountData",
            Self::InvalidAccountOwner => "InvalidAccountOwner",
            Self::MissingSignature => "MissingSignature",
            Self::AlreadyInitialized => "AlreadyInitialized",
            Self::UninitializedAccount => "UninitializedAccount",
            Self::MarketMismatch => "MarketMismatch",
            Self::OwnerMismatch => "OwnerMismatch",
            Self::InsufficientCollateral => "InsufficientCollateral",
            Self::LtvExceeded => "LtvExceeded",
            Self::PositionHealthy => "PositionHealthy",
            Self::InsufficientVaultBalance => "InsufficientVaultBalance",
            Self::RepayExceedsDebt => "RepayExceedsDebt",
            Self::InvalidVault => "InvalidVault",
            Self::InvalidPrice => "InvalidPrice",
        }
    }

    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => Self::MathOverflow,
            1 => Self::InvalidAccountData,
            2 => Self::InvalidAccountOwner,
            3 => Self::MissingSignature,
            4 => Self::AlreadyInitialized,
            5 => Self::UninitializedAccount,
            6 => Self::MarketMismatch,
            7 => Self::OwnerMismatch,
            8 => Self::InsufficientCollateral,
            9 => Self::LtvExceeded,
            10 => Self::PositionHealthy,
            11 => Self::InsufficientVaultBalance,
            12 => Self::RepayExceedsDebt,
            13 => Self::InvalidVault,
            14 => Self::InvalidPrice,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod audit_tests {
    #[test]
    fn valuation_handles_all_decimal_exponents() {
        assert_eq!(
            super::reference::value_micro_usd(u64::MAX, 0, u64::MAX),
            u64::MAX as u128 * u64::MAX as u128
        );
        for decimals in 39..=255 {
            assert_eq!(
                super::reference::value_micro_usd(u64::MAX, decimals, u64::MAX),
                0
            );
        }
    }
}
