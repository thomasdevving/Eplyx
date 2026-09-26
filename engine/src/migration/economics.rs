//! Exact integer migration economics: the specification's semantic truth.
//!
//! The planner, reconciliation, search and reports all use this module. A candidate
//! migration program implements the same rule independently; the VM result is
//! checked against this calculation and the shared golden vectors
//! (`fixtures/migration/economics-golden-vectors.csv`) are executed by the engine,
//! by the reference program's host tests and by the reference program in the VM, so
//! the two implementations cannot silently diverge. No floating point is used.
//!
//! Rule: `fee = floor(consumed × feeBps / 10 000)`, `converted = consumed − fee`,
//! `exact = converted × numerator`, `output = floor(exact / denominator)` or
//! `ceil(exact / denominator)`. `output == 0` and `output < minimumOutput` are
//! explicit failures, as is any value outside the u64 raw-amount range.
use super::spec::{Conversion, RatioBasis, Rounding};
use serde::{Deserialize, Serialize};
use std::fmt;

/// 10^38 is the largest power of ten representable in u128.
pub const MAX_UI_DECIMALS: u8 = 38;
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Raw-basis terms after exact decimal scaling and reduction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveTerms {
    #[serde(with = "crate::numfmt::u64_string")]
    pub numerator: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub denominator: u64,
    pub rounding: Rounding,
    pub fee_bps: u16,
    #[serde(with = "crate::numfmt::u64_string")]
    pub minimum_output: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TermsError {
    NonCanonicalInteger,
    ZeroRatio,
    FeeOutOfRange,
    ZeroMinimumOutput,
    DecimalsOutOfRange,
    RatioNotRepresentable,
}

impl fmt::Display for TermsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NonCanonicalInteger => "ratio and minimum output must be canonical u64 strings",
            Self::ZeroRatio => "the ratio numerator and denominator must be positive",
            Self::FeeOutOfRange => "the source fee must be within 0-10000 bps",
            Self::ZeroMinimumOutput => "minimumOutputRaw must be at least 1",
            Self::DecimalsOutOfRange => "a ui-basis ratio needs decimals of at most 38",
            Self::RatioNotRepresentable => {
                "the decimal-scaled ratio does not reduce to u64 numerator and denominator"
            }
        })
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn canonical(input: &str) -> Result<u64, TermsError> {
    let value: u64 = input.parse().map_err(|_| TermsError::NonCanonicalInteger)?;
    if value.to_string() != input {
        return Err(TermsError::NonCanonicalInteger);
    }
    Ok(value)
}

/// Resolve declared conversion terms into exact raw-basis terms.
///
/// A ui-basis ratio `n/d` between whole tokens becomes the raw ratio
/// `n × 10^destinationDecimals / (d × 10^sourceDecimals)`, reduced exactly.
pub fn effective_terms(
    conversion: &Conversion,
    source_decimals: u8,
    destination_decimals: u8,
) -> Result<EffectiveTerms, TermsError> {
    let numerator = canonical(&conversion.numerator)?;
    let denominator = canonical(&conversion.denominator)?;
    let minimum_output = canonical(&conversion.minimum_output_raw)?;
    if numerator == 0 || denominator == 0 {
        return Err(TermsError::ZeroRatio);
    }
    if u64::from(conversion.fee.bps()) > BPS_DENOMINATOR {
        return Err(TermsError::FeeOutOfRange);
    }
    if minimum_output == 0 {
        return Err(TermsError::ZeroMinimumOutput);
    }
    let (mut n, mut d) = (u128::from(numerator), u128::from(denominator));
    if conversion.ratio_basis == RatioBasis::Ui {
        if source_decimals > MAX_UI_DECIMALS || destination_decimals > MAX_UI_DECIMALS {
            return Err(TermsError::DecimalsOutOfRange);
        }
        let scale = |decimals: u8| 10u128.checked_pow(u32::from(decimals));
        let dest = scale(destination_decimals).ok_or(TermsError::DecimalsOutOfRange)?;
        let source = scale(source_decimals).ok_or(TermsError::DecimalsOutOfRange)?;
        n = n
            .checked_mul(dest)
            .ok_or(TermsError::RatioNotRepresentable)?;
        d = d
            .checked_mul(source)
            .ok_or(TermsError::RatioNotRepresentable)?;
    }
    let divisor = gcd(n, d);
    let (n, d) = (n / divisor, d / divisor);
    Ok(EffectiveTerms {
        numerator: u64::try_from(n).map_err(|_| TermsError::RatioNotRepresentable)?,
        denominator: u64::try_from(d).map_err(|_| TermsError::RatioNotRepresentable)?,
        rounding: conversion.rounding,
        fee_bps: conversion.fee.bps(),
        minimum_output,
    })
}

/// The exact expected arithmetic of one migration of `consumed` raw source units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quote {
    pub consumed: u64,
    pub fee: u64,
    pub converted: u64,
    pub output: u64,
    /// `converted × numerator`, before division.
    pub exact_numerator: u128,
    /// `exact_numerator mod denominator`.
    pub remainder: u64,
    pub denominator: u64,
}

impl Quote {
    /// Delivered minus exact output, in units of `1/denominator` destination raw:
    /// zero or negative under floor (dust kept back), zero or positive under ceiling.
    pub fn rounding_delta_numerator(&self) -> i128 {
        let delivered = i128::try_from(u128::from(self.output) * u128::from(self.denominator))
            .unwrap_or(i128::MAX);
        let exact = i128::try_from(self.exact_numerator).unwrap_or(i128::MAX);
        delivered - exact
    }

    pub fn record(&self) -> QuoteRecord {
        QuoteRecord {
            consumed_raw: self.consumed.to_string(),
            fee_raw: self.fee.to_string(),
            converted_raw: self.converted.to_string(),
            output_raw: self.output.to_string(),
            exact_output_numerator: self.exact_numerator.to_string(),
            denominator: self.denominator.to_string(),
            remainder: self.remainder.to_string(),
            rounding_delta_numerator: self.rounding_delta_numerator().to_string(),
        }
    }
}

/// Serializable quote. Integers are exact decimal strings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteRecord {
    pub consumed_raw: String,
    pub fee_raw: String,
    pub converted_raw: String,
    pub output_raw: String,
    pub exact_output_numerator: String,
    pub denominator: String,
    pub remainder: String,
    pub rounding_delta_numerator: String,
}

/// Explicit arithmetic failures. Names match the reference program's error names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error")]
pub enum QuoteError {
    ZeroAmount,
    ArithmeticOverflow,
    ZeroOutput,
    OutputBelowMinimum { output: u64, minimum: u64 },
}

impl QuoteError {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ZeroAmount => "ZeroAmount",
            Self::ArithmeticOverflow => "ArithmeticOverflow",
            Self::ZeroOutput => "ZeroOutput",
            Self::OutputBelowMinimum { .. } => "OutputBelowMinimum",
        }
    }
}

impl fmt::Display for QuoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutputBelowMinimum { output, minimum } => {
                write!(f, "OutputBelowMinimum: output {output} < minimum {minimum}")
            }
            other => f.write_str(other.name()),
        }
    }
}

/// The exact expected migration of `consumed` raw source units.
pub fn quote(consumed: u64, terms: &EffectiveTerms) -> Result<Quote, QuoteError> {
    if consumed == 0 {
        return Err(QuoteError::ZeroAmount);
    }
    let amount = u128::from(consumed);
    let fee = amount
        .checked_mul(u128::from(terms.fee_bps))
        .ok_or(QuoteError::ArithmeticOverflow)?
        / u128::from(BPS_DENOMINATOR);
    let converted = amount
        .checked_sub(fee)
        .ok_or(QuoteError::ArithmeticOverflow)?;
    let exact = converted
        .checked_mul(u128::from(terms.numerator))
        .ok_or(QuoteError::ArithmeticOverflow)?;
    let denominator = u128::from(terms.denominator);
    let remainder = exact % denominator;
    let output = match terms.rounding {
        Rounding::Floor => exact / denominator,
        Rounding::Ceiling => exact
            .checked_add(denominator - 1)
            .ok_or(QuoteError::ArithmeticOverflow)?
            .checked_div(denominator)
            .ok_or(QuoteError::ArithmeticOverflow)?,
    };
    let output = u64::try_from(output).map_err(|_| QuoteError::ArithmeticOverflow)?;
    if output == 0 {
        return Err(QuoteError::ZeroOutput);
    }
    if output < terms.minimum_output {
        return Err(QuoteError::OutputBelowMinimum {
            output,
            minimum: terms.minimum_output,
        });
    }
    Ok(Quote {
        consumed,
        fee: u64::try_from(fee).map_err(|_| QuoteError::ArithmeticOverflow)?,
        converted: u64::try_from(converted).map_err(|_| QuoteError::ArithmeticOverflow)?,
        output,
        exact_numerator: exact,
        remainder: u64::try_from(remainder).map_err(|_| QuoteError::ArithmeticOverflow)?,
        denominator: terms.denominator,
    })
}

/// Output before minimum-output enforcement, for boundary search. `None` for
/// zero input or overflow.
pub fn raw_output(consumed: u64, terms: &EffectiveTerms) -> Option<u64> {
    let relaxed = EffectiveTerms {
        minimum_output: 0,
        ..*terms
    };
    match quote(consumed, &relaxed) {
        Ok(quote) => Some(quote.output),
        Err(QuoteError::ZeroOutput) => Some(0),
        Err(_) => None,
    }
}

/// The smallest amount in `lo..=hi` satisfying a monotone predicate
/// (false … false true … true). Output is nondecreasing in the consumed amount
/// because the floor-rounded fee grows by at most one unit per unit consumed.
pub fn first_where(lo: u64, hi: u64, predicate: impl Fn(u64) -> bool) -> Option<u64> {
    if lo > hi || !predicate(hi) {
        return None;
    }
    let (mut lo, mut hi) = (lo, hi);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if predicate(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Some(lo)
}

/// The smallest source amount whose migration succeeds under the terms. Output is
/// monotone only below the u64 overflow point, so the search stops there.
pub fn minimum_migratable_amount(terms: &EffectiveTerms) -> Option<u64> {
    let hi = maximum_migratable_amount(terms)?;
    first_where(1, hi, |amount| {
        raw_output(amount, terms).is_some_and(|output| output >= terms.minimum_output.max(1))
    })
}

/// The largest source amount whose output still fits in u64.
pub fn maximum_migratable_amount(terms: &EffectiveTerms) -> Option<u64> {
    let fits = |amount: u64| raw_output(amount, terms).is_some();
    if fits(u64::MAX) {
        return Some(u64::MAX);
    }
    first_where(1, u64::MAX, |amount| !fits(amount)).and_then(|first| first.checked_sub(1))
}

/// One shared golden vector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoldenVector {
    pub name: String,
    pub amount: u64,
    pub terms: EffectiveTerms,
    pub expected: Result<(u64, u64, u64, u64), String>,
}

pub const GOLDEN_VECTORS: &str =
    include_str!("../../../fixtures/migration/economics-golden-vectors.csv");

/// Parse the shared CSV: `name,amount,numerator,denominator,rounding,fee_bps,
/// minimum_output,fee,converted,output,remainder,error`.
pub fn golden_vectors() -> anyhow::Result<Vec<GoldenVector>> {
    use anyhow::{bail, Context};
    let mut vectors = Vec::new();
    for (line_number, line) in GOLDEN_VECTORS.lines().enumerate().skip(1) {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        if cells.len() != 12 {
            bail!(
                "golden vector line {} has {} cells",
                line_number + 1,
                cells.len()
            );
        }
        let number = |index: usize| -> anyhow::Result<u64> {
            cells[index]
                .parse()
                .with_context(|| format!("golden vector line {} cell {index}", line_number + 1))
        };
        let rounding = match cells[4] {
            "floor" => Rounding::Floor,
            "ceiling" => Rounding::Ceiling,
            other => bail!("unknown rounding {other}"),
        };
        let expected = if cells[11].is_empty() {
            Ok((number(7)?, number(8)?, number(9)?, number(10)?))
        } else {
            Err(cells[11].to_string())
        };
        vectors.push(GoldenVector {
            name: cells[0].into(),
            amount: number(1)?,
            terms: EffectiveTerms {
                numerator: number(2)?,
                denominator: number(3)?,
                rounding,
                fee_bps: u16::try_from(number(5)?)?,
                minimum_output: number(6)?,
            },
            expected,
        });
    }
    Ok(vectors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::spec::Fee;

    fn terms(n: u64, d: u64, rounding: Rounding, bps: u16, minimum: u64) -> EffectiveTerms {
        EffectiveTerms {
            numerator: n,
            denominator: d,
            rounding,
            fee_bps: bps,
            minimum_output: minimum,
        }
    }

    #[test]
    fn golden_vectors_match_the_planner() {
        let vectors = golden_vectors().unwrap();
        assert!(vectors.len() >= 20, "golden vector file lost entries");
        for vector in vectors {
            let actual = quote(vector.amount, &vector.terms)
                .map(|q| (q.fee, q.converted, q.output, q.remainder))
                .map_err(|e| e.name().to_string());
            assert_eq!(actual, vector.expected, "{}", vector.name);
        }
    }

    #[test]
    fn rounding_delta_is_signed_dust() {
        let floor = quote(7, &terms(1, 2, Rounding::Floor, 0, 1)).unwrap();
        assert_eq!((floor.output, floor.rounding_delta_numerator()), (3, -1));
        let ceiling = quote(7, &terms(1, 2, Rounding::Ceiling, 0, 1)).unwrap();
        assert_eq!((ceiling.output, ceiling.rounding_delta_numerator()), (4, 1));
        let exact = quote(8, &terms(1, 2, Rounding::Ceiling, 0, 1)).unwrap();
        assert_eq!(exact.rounding_delta_numerator(), 0);
    }

    #[test]
    fn ui_basis_scales_decimals_exactly() {
        let conversion = |n: &str, d: &str| Conversion {
            ratio_basis: RatioBasis::Ui,
            numerator: n.into(),
            denominator: d.into(),
            rounding: Rounding::Floor,
            fee: Fee::None,
            minimum_output_raw: "1".into(),
        };
        // 1 whole source (9 decimals) = 2 whole destination (6 decimals).
        let t = effective_terms(&conversion("2", "1"), 9, 6).unwrap();
        assert_eq!((t.numerator, t.denominator), (1, 500));
        assert_eq!(quote(1_000_000_000, &t).unwrap().output, 2_000_000);
        // Destination with more decimals.
        let t = effective_terms(&conversion("1", "1"), 2, 8).unwrap();
        assert_eq!((t.numerator, t.denominator), (1_000_000, 1));
        assert_eq!(
            effective_terms(&conversion("1", "1"), 0, 39),
            Err(TermsError::DecimalsOutOfRange)
        );
        assert_eq!(
            effective_terms(&conversion("18446744073709551615", "1"), 0, 38),
            Err(TermsError::RatioNotRepresentable)
        );
        let raw = Conversion {
            ratio_basis: RatioBasis::Raw,
            ..conversion("0", "1")
        };
        assert_eq!(effective_terms(&raw, 0, 0), Err(TermsError::ZeroRatio));
        let fee = Conversion {
            fee: Fee::SourceBps { bps: 10_001 },
            ..conversion("1", "1")
        };
        assert_eq!(effective_terms(&fee, 0, 0), Err(TermsError::FeeOutOfRange));
    }

    #[test]
    fn output_is_monotone_so_boundaries_are_exact() {
        let t = terms(3, 7, Rounding::Floor, 250, 5);
        let minimum = minimum_migratable_amount(&t).unwrap();
        assert!(quote(minimum, &t).is_ok());
        assert!(quote(minimum - 1, &t).is_err());
        for amount in 1..5_000u64 {
            assert!(raw_output(amount, &t) <= raw_output(amount + 1, &t));
        }
        let wide = terms(1_000_000, 1, Rounding::Floor, 0, 1);
        let maximum = maximum_migratable_amount(&wide).unwrap();
        assert_eq!(maximum, u64::MAX / 1_000_000);
        assert!(quote(maximum, &wide).is_ok());
        assert_eq!(
            quote(maximum + 1, &wide),
            Err(QuoteError::ArithmeticOverflow)
        );
        assert_eq!(
            maximum_migratable_amount(&terms(1, 1, Rounding::Floor, 0, 1)),
            Some(u64::MAX)
        );
        // Regression: a ratio above 1 overflows at the top of the u64 range; the
        // minimum must still be found below the overflow point.
        assert_eq!(minimum_migratable_amount(&wide), Some(1));
        let dusty = terms(1_000, 1, Rounding::Floor, 0, 5_000);
        assert_eq!(minimum_migratable_amount(&dusty), Some(5));
    }
}
