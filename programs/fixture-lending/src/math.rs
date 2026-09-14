//! Risk math for the fixture lending protocol.
//!
//! # This module is the *only* difference between V1 and V2
//!
//! Everything else in the repository - the instruction set, the account layouts,
//! the error codes, the program ID, the account ordering, the signer set - is
//! compiled from identical source for both builds. The entire behavioural delta
//! that the differential engine is asked to find lives in [`collateral_value`],
//! and amounts to swapping the order of one multiply and one divide.
//!
//! See the README section "How V1 and V2 differ" for the economic consequence.

use crate::error::LendingError;
use fixture_lending_interface::{BPS_DENOMINATOR, HEALTH_INFINITE, HEALTH_SCALE, LAMPORTS_PER_SOL};

#[cfg(all(feature = "v1", feature = "v2"))]
compile_error!("features `v1` and `v2` are mutually exclusive; build with exactly one");

#[cfg(not(any(feature = "v1", feature = "v2")))]
compile_error!("exactly one of the `v1` / `v2` features must be enabled");

#[cfg(feature = "v1")]
pub const BUILD_VERSION: &str = "v1";
#[cfg(feature = "v2")]
pub const BUILD_VERSION: &str = "v2";

/// Value of a collateral balance, in micro-USD.
///
/// `collateral_lamports` is in lamports (1e9 per SOL); `price` is micro-USD per
/// whole SOL. The result must therefore be divided by `LAMPORTS_PER_SOL` at some
/// point. *Where* that division happens is the whole story.
#[cfg(feature = "v1")]
pub fn collateral_value(collateral_lamports: u64, price: u64) -> Result<u128, LendingError> {
    // Widen to u128 first, multiply, and divide last. The full-precision product
    // is carried through the scale conversion, so fractional-SOL collateral keeps
    // its value. Do not reorder these two operations.
    (collateral_lamports as u128)
        .checked_mul(price as u128)
        .map(|scaled| scaled / LAMPORTS_PER_SOL)
        .ok_or(LendingError::MathOverflow)
}

/// Value of a collateral balance, in micro-USD.
#[cfg(feature = "v2")]
pub fn collateral_value(collateral_lamports: u64, price: u64) -> Result<u128, LendingError> {
    // Normalise lamports to whole SOL up front, so the multiply runs on a small
    // number and the overflow check is trivially satisfied.
    //
    // ==================== INTENTIONAL REGRESSION ====================
    // This is the seeded bug. Dividing before multiplying discards the
    // *fractional* SOL of every position: 99.5 SOL is valued as 99 SOL, a
    // silent ~$50 haircut at a $100 SOL price.
    //
    // Why it is hard to catch by conventional means:
    //   * the public instruction set is byte-identical to V1
    //   * the account layout is byte-identical to V1
    //   * authorities, program ID and CPI set are unchanged
    //   * every position holding a whole number of SOL is completely unaffected
    //   * of the affected positions, only those already close to a threshold
    //     change their *outcome*; the rest merely shift a few decimal places
    //
    // It only becomes visible by executing against specific account states.
    // ================================================================
    let whole_sol = (collateral_lamports as u128) / LAMPORTS_PER_SOL;
    whole_sol
        .checked_mul(price as u128)
        .ok_or(LendingError::MathOverflow)
}

/// Collateral value scaled by a risk parameter (in bps), in micro-USD.
///
/// Shared by both builds - the divergence is confined to [`collateral_value`].
pub fn adjusted_collateral(
    collateral_lamports: u64,
    price: u64,
    bps: u16,
) -> Result<u128, LendingError> {
    collateral_value(collateral_lamports, price)?
        .checked_mul(bps as u128)
        .map(|scaled| scaled / BPS_DENOMINATOR)
        .ok_or(LendingError::MathOverflow)
}

/// Health factor in `HEALTH_SCALE` fixed point. Below `HEALTH_SCALE` the position
/// is liquidatable.
pub fn health_factor(
    collateral_lamports: u64,
    debt: u64,
    price: u64,
    liquidation_threshold_bps: u16,
) -> Result<u64, LendingError> {
    if debt == 0 {
        return Ok(HEALTH_INFINITE);
    }
    let adjusted = adjusted_collateral(collateral_lamports, price, liquidation_threshold_bps)?;
    let scaled = adjusted
        .checked_mul(HEALTH_SCALE as u128)
        .ok_or(LendingError::MathOverflow)?;
    Ok(u64::try_from(scaled / debt as u128).unwrap_or(HEALTH_INFINITE))
}

/// Maximum debt this collateral may support, in micro-USD.
pub fn borrow_capacity(
    collateral_lamports: u64,
    price: u64,
    max_ltv_bps: u16,
) -> Result<u128, LendingError> {
    adjusted_collateral(collateral_lamports, price, max_ltv_bps)
}

/// Convert a micro-USD amount into lamports at `price`. Not feature-gated: the
/// seeded regression is deliberately confined to one function.
pub fn usd_to_lamports(usd: u128, price: u64) -> Result<u64, LendingError> {
    if price == 0 {
        return Err(LendingError::InvalidPrice);
    }
    let lamports = usd
        .checked_mul(LAMPORTS_PER_SOL)
        .ok_or(LendingError::MathOverflow)?
        / price as u128;
    u64::try_from(lamports).map_err(|_| LendingError::MathOverflow)
}

pub fn is_liquidatable(health: u64) -> bool {
    health < HEALTH_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixture_lending_interface::reference;

    const PRICE: u64 = 100_000_000; // $100.00 per SOL
    const WHOLE: u64 = 100_000_000_000; // 100 SOL
    const FRACTIONAL: u64 = 99_500_000_000; // 99.5 SOL

    /// Both builds must agree on whole-SOL collateral. This is the property that
    /// makes the regression state-dependent rather than universal.
    #[test]
    fn whole_sol_collateral_is_valued_identically_by_both_builds() {
        assert_eq!(collateral_value(WHOLE, PRICE).unwrap(), 10_000_000_000);
        assert_eq!(
            collateral_value(WHOLE, PRICE).unwrap(),
            reference::collateral_value(WHOLE, PRICE)
        );
    }

    #[test]
    fn zero_debt_is_infinitely_healthy() {
        assert_eq!(
            health_factor(WHOLE, 0, PRICE, 8_000).unwrap(),
            fixture_lending_interface::HEALTH_INFINITE
        );
    }

    /// Widening to u128 before multiplying removes the overflow surface
    /// entirely for u64 inputs: `u64::MAX * u64::MAX` still fits. The
    /// `checked_mul` calls are therefore defensive rather than load-bearing.
    /// Asserting that keeps the claim honest - if an intermediate type is ever
    /// narrowed, this test fails and the checked path becomes reachable again.
    #[test]
    fn widening_removes_the_overflow_surface_for_u64_inputs() {
        assert!(collateral_value(u64::MAX, u64::MAX).is_ok());
        // An absurd position saturates to "infinitely healthy" rather than
        // wrapping to a small number, which would read as liquidatable.
        let extreme = health_factor(u64::MAX, 1, u64::MAX, 10_000).unwrap();
        assert_eq!(extreme, fixture_lending_interface::HEALTH_INFINITE);
        assert!(!is_liquidatable(extreme));
    }

    #[cfg(feature = "v1")]
    mod v1 {
        use super::*;

        #[test]
        fn fractional_collateral_keeps_its_value() {
            // 99.5 SOL at $100 is $9,950.00 exactly.
            assert_eq!(collateral_value(FRACTIONAL, PRICE).unwrap(), 9_950_000_000);
        }

        #[test]
        fn agrees_with_the_reference_implementation() {
            for lamports in [1u64, 999_999_999, FRACTIONAL, WHOLE, 7_000_000_000_000] {
                assert_eq!(
                    collateral_value(lamports, PRICE).unwrap(),
                    reference::collateral_value(lamports, PRICE),
                    "mismatch at {lamports} lamports"
                );
            }
        }

        #[test]
        fn the_flagship_position_is_healthy() {
            let health = health_factor(FRACTIONAL, 7_930_000_000, PRICE, 8_000).unwrap();
            assert_eq!(health, 1_003_783);
            assert!(!is_liquidatable(health));
        }
    }

    #[cfg(feature = "v2")]
    mod v2 {
        use super::*;

        #[test]
        fn fractional_collateral_is_silently_truncated() {
            // The seeded regression: 99.5 SOL is valued as 99 SOL, a $50 haircut.
            assert_eq!(collateral_value(FRACTIONAL, PRICE).unwrap(), 9_900_000_000);
            assert_ne!(
                collateral_value(FRACTIONAL, PRICE).unwrap(),
                reference::collateral_value(FRACTIONAL, PRICE)
            );
        }

        #[test]
        fn sub_one_sol_collateral_is_valued_at_zero() {
            assert_eq!(collateral_value(999_999_999, PRICE).unwrap(), 0);
        }

        #[test]
        fn the_flagship_position_is_liquidatable() {
            let health = health_factor(FRACTIONAL, 7_930_000_000, PRICE, 8_000).unwrap();
            assert_eq!(health, 998_738);
            assert!(is_liquidatable(health));
        }
    }
}
