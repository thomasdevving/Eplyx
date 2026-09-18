//! `kamino_fraction_v1` — the named deterministic evaluator.
//!
//! Kamino KLend stores every internal quantity that needs sub-unit precision as
//! a **scaled fraction**: a `u128` interpreted as `U68F60`, meaning 68 integer
//! bits and 60 fractional bits. Fields carrying one are suffixed `Sf` in the
//! IDL — `borrowedAmountSf`, `depositedValueSf`, `marketPriceSf`.
//!
//! This is the boundary the phase brief draws, made concrete. A scaled fraction
//! is **readable declaratively and meaningless declaratively**: the raw `u128`
//! delta on a borrow is a number in units of 2^-60 tokens, and reporting it as
//! though it were a token amount would be off by eighteen orders of magnitude.
//! Converting it is protocol math, so it lives in a named, versioned, tested
//! evaluator rather than in the universal evidence layer.
//!
//! ## What it computes, exactly
//!
//! [`to_base_units`] is `value >> 60` — an exact integer floor of the fraction.
//! Nothing is rounded, nothing is scaled by a mint's decimals, and no floating
//! point is involved at any point. The result is token base units, which still
//! have to be interpreted against the reserve's `mintDecimals` for display.
//!
//! ## What it does not compute
//!
//! Not a health factor. Not a liquidation threshold. Not an oracle-adjusted
//! value. Not an interest projection. Those need a price and a borrow-rate
//! model, and this phase deliberately implements neither — see
//! `docs/phase-u2-kamino.md`.

/// Fractional bits in Kamino's `Fraction` (`U68F60`).
///
/// Stated once. Every conversion in the adapter goes through this module, so a
/// wrong scale is one edit and one test away rather than scattered shifts.
pub const FRACTION_BITS: u32 = 60;

/// The evaluator's stable identity, recorded wherever its output is reported.
pub const EVALUATOR_ID: &str = "kamino_fraction_v1";

/// Integer part of a scaled fraction, in token base units.
///
/// Floor, not rounding: `>> 60` discards the fractional bits. That matches how
/// a holder's position reads — a debt of `1.9999` base units is one whole base
/// unit plus a fraction the protocol still tracks — and it is why the raw
/// scaled value is reported alongside, never replaced by, this one.
pub fn to_base_units(value: u128) -> u128 {
    value >> FRACTION_BITS
}

/// The fractional remainder, in units of `2^-60`.
///
/// Exposed so a caller can show that a conversion discarded something. A debt
/// that grew by less than one base unit converts to a delta of zero, and
/// reporting that as "no change" without the remainder would hide a real
/// movement.
pub fn fractional_remainder(value: u128) -> u128 {
    value & ((1_u128 << FRACTION_BITS) - 1)
}

/// Signed difference of two scaled fractions, in base units.
///
/// Converts **after** subtracting, not before. Converting each side first and
/// then subtracting loses up to one base unit whenever the two sides straddle a
/// whole-unit boundary, which on a small borrow is the entire quantity.
pub fn delta_base_units(before: u128, after: u128) -> i128 {
    if after >= before {
        // The difference is at most `u128::MAX`, and its floor fits an i128 for
        // every value the protocol can hold: 128 bits minus 60 fractional bits
        // leaves 68, well inside i128.
        to_base_units(after - before) as i128
    } else {
        -(to_base_units(before - after) as i128)
    }
}

/// Exact signed difference of two scaled fractions, still scaled.
///
/// The raw evidence, kept beside the converted quantity so a reader can check
/// the conversion rather than trust it.
pub fn delta_scaled(before: u128, after: u128) -> i128 {
    if after >= before {
        i128::try_from(after - before).unwrap_or(i128::MAX)
    } else {
        -i128::try_from(before - after).unwrap_or(i128::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One whole base unit is exactly `2^60`.
    #[test]
    fn a_whole_base_unit_is_two_to_the_sixty() {
        assert_eq!(to_base_units(1_u128 << FRACTION_BITS), 1);
        assert_eq!(to_base_units(1_000_u128 << FRACTION_BITS), 1_000);
        assert_eq!(fractional_remainder(1_u128 << FRACTION_BITS), 0);
    }

    #[test]
    fn a_pure_fraction_floors_to_zero_and_keeps_its_remainder() {
        let half = 1_u128 << (FRACTION_BITS - 1);
        assert_eq!(to_base_units(half), 0);
        assert_eq!(fractional_remainder(half), half);
    }

    /// The reason the remainder is exposed: a sub-unit debt increase converts
    /// to zero, and calling that "no change" would be false.
    #[test]
    fn a_sub_unit_increase_is_visible_even_though_it_floors_to_zero() {
        let before = 5_u128 << FRACTION_BITS;
        let after = before + (1_u128 << (FRACTION_BITS - 4));
        assert_eq!(delta_base_units(before, after), 0);
        assert_ne!(
            delta_scaled(before, after),
            0,
            "the raw evidence still moved"
        );
    }

    /// Converting before subtracting is the defect this ordering prevents.
    #[test]
    fn the_delta_is_converted_after_subtracting_not_before() {
        // 1.75 -> 2.25 base units. Both floor to 1 and 2, so a naive
        // difference-of-floors gives 1; the true floored difference is 0.
        let before = (7_u128 << FRACTION_BITS) / 4;
        let after = (9_u128 << FRACTION_BITS) / 4;
        assert_eq!(to_base_units(before), 1);
        assert_eq!(to_base_units(after), 2);
        let naive = to_base_units(after) as i128 - to_base_units(before) as i128;
        assert_eq!(naive, 1);
        assert_eq!(
            delta_base_units(before, after),
            0,
            "0.5 of a base unit is not a whole base unit"
        );
    }

    #[test]
    fn a_decrease_is_negative_and_exact() {
        let before = 100_u128 << FRACTION_BITS;
        let after = 40_u128 << FRACTION_BITS;
        assert_eq!(delta_base_units(before, after), -60);
        assert_eq!(delta_scaled(before, after), -(60_i128 << FRACTION_BITS));
    }

    #[test]
    fn an_unchanged_value_has_a_zero_delta_in_both_forms() {
        let value = 12_345_u128 << FRACTION_BITS;
        assert_eq!(delta_base_units(value, value), 0);
        assert_eq!(delta_scaled(value, value), 0);
    }

    /// No floating point anywhere: the largest representable fraction still
    /// converts exactly.
    #[test]
    fn the_full_range_converts_without_overflow_or_rounding() {
        assert_eq!(to_base_units(u128::MAX), (1_u128 << 68) - 1);
        assert_eq!(fractional_remainder(u128::MAX), (1_u128 << 60) - 1);
        // And a full-range delta stays representable as a signed base-unit count.
        assert_eq!(delta_base_units(0, u128::MAX), ((1_u128 << 68) - 1) as i128);
        assert_eq!(
            delta_base_units(u128::MAX, 0),
            -(((1_u128 << 68) - 1) as i128)
        );
    }

    /// A plausible USDC debt: 1,234.567891 tokens at 6 decimals.
    #[test]
    fn a_realistic_debt_round_trips_through_the_scale() {
        let base_units: u128 = 1_234_567_891;
        let scaled = base_units << FRACTION_BITS;
        assert_eq!(to_base_units(scaled), base_units);
        // And a borrow of 500.000000 on top of it.
        let after = scaled + (500_000_000_u128 << FRACTION_BITS);
        assert_eq!(delta_base_units(scaled, after), 500_000_000);
    }
}
