//! Fixed-point USD arithmetic.
//!
//! Every valuation in the impact layer flows through these types, and they are
//! integer-only by construction. There is deliberately no `From<f64>`, no
//! `as f64`, and no arithmetic that leaves the integer domain: at corpus scale a
//! double stops being able to represent every micro-USD once totals pass 2^53,
//! and "close enough" is not a property an impact report should have.
//!
//! Representation is micro-USD - six decimal places - which matches the debt
//! asset's own precision, so a USD-pegged debt amount converts with no scaling
//! loss at all.
//!
//! JSON encoding is a decimal *string* (`"12841220.000000"`) rather than a
//! number. A JSON number would be parsed as a double by most consumers,
//! reintroducing exactly the precision loss these types exist to avoid.

use std::fmt;
use std::iter::Sum;
use std::ops::Add;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Micro-USD per whole USD.
pub const USD_SCALE: u128 = 1_000_000;
/// Decimal places carried by [`Usd`] and [`SignedUsd`].
pub const USD_DECIMALS: u32 = 6;

fn group_thousands(value: u128) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Split a decimal string such as `"-12.5"` into (negative, whole, fraction)
/// scaled to [`USD_DECIMALS`].
fn parse_decimal(text: &str) -> Result<(bool, u128), String> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if digits.is_empty() || !digits.bytes().any(|b| b.is_ascii_digit()) {
        return Err("empty amount".to_string());
    }
    let (whole, fraction) = match digits.split_once('.') {
        Some((w, f)) => (w, f),
        None => (digits, ""),
    };
    if fraction.len() > USD_DECIMALS as usize {
        return Err(format!(
            "amount {text:?} has more than {USD_DECIMALS} decimal places"
        ));
    }
    let parse = |part: &str| -> Result<u128, String> {
        if part.is_empty() {
            return Ok(0);
        }
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("invalid amount {text:?}"));
        }
        part.parse::<u128>()
            .map_err(|e| format!("invalid amount {text:?}: {e}"))
    };
    let whole = parse(whole)?;
    let fraction_digits = parse(fraction)?;
    let padding = 10u128.pow(USD_DECIMALS - fraction.len() as u32);
    let micro = whole
        .checked_mul(USD_SCALE)
        .and_then(|v| v.checked_add(fraction_digits * padding))
        .ok_or_else(|| format!("amount {text:?} overflows"))?;
    Ok((negative, micro))
}

fn render_decimal(micro: u128) -> String {
    format!("{}.{:06}", micro / USD_SCALE, micro % USD_SCALE)
}

/// Round micro-USD to cents, half up, without leaving the integer domain.
fn to_cents(micro: u128) -> u128 {
    micro / 10_000 + u128::from(micro % 10_000 >= 5_000)
}

/// A non-negative USD amount, in micro-USD.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Usd(u128);

impl Usd {
    pub const ZERO: Usd = Usd(0);

    pub const fn from_micro(micro: u128) -> Self {
        Usd(micro)
    }

    pub const fn from_whole(dollars: u64) -> Self {
        Usd(dollars as u128 * USD_SCALE)
    }

    pub const fn micro(self) -> u128 {
        self.0
    }

    /// Saturating because an impact report should degrade to a clamped total
    /// rather than panic; the corpus is many orders of magnitude below u128.
    pub fn saturating_add(self, other: Usd) -> Usd {
        Usd(self.0.saturating_add(other.0))
    }

    pub fn checked_sub(self, other: Usd) -> Option<Usd> {
        self.0.checked_sub(other.0).map(Usd)
    }

    /// Checked signed difference, including values above i128::MAX.
    pub fn checked_signed_sub(self, other: Usd) -> Option<SignedUsd> {
        if self.0 >= other.0 {
            i128::try_from(self.0 - other.0).ok().map(SignedUsd)
        } else {
            let magnitude = other.0 - self.0;
            if magnitude == i128::MIN.unsigned_abs() {
                Some(SignedUsd(i128::MIN))
            } else {
                i128::try_from(magnitude).ok().map(|v| SignedUsd(-v))
            }
        }
    }

    /// Fails explicitly if the difference cannot be represented; never wraps.
    pub fn signed_sub(self, other: Usd) -> SignedUsd {
        self.checked_signed_sub(other)
            .expect("USD signed difference overflows")
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Full-precision decimal string, as used in JSON.
    pub fn to_plain_string(self) -> String {
        render_decimal(self.0)
    }

    /// Terminal rendering: `$12,841,220.00`.
    pub fn format_dollars(self) -> String {
        let cents = to_cents(self.0);
        format!("${}.{:02}", group_thousands(cents / 100), cents % 100)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let (negative, micro) = parse_decimal(text)?;
        if negative && micro != 0 {
            return Err(format!("{text:?} is negative; use SignedUsd"));
        }
        Ok(Usd(micro))
    }
}

impl Add for Usd {
    type Output = Usd;
    fn add(self, other: Usd) -> Usd {
        self.saturating_add(other)
    }
}

impl Sum for Usd {
    fn sum<I: Iterator<Item = Usd>>(iter: I) -> Usd {
        iter.fold(Usd::ZERO, Usd::saturating_add)
    }
}

impl fmt::Display for Usd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_dollars())
    }
}

impl Serialize for Usd {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_plain_string())
    }
}

impl<'de> Deserialize<'de> for Usd {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Usd::parse(&text).map_err(D::Error::custom)
    }
}

/// A USD amount that may be negative - used for net value, where debt can
/// exceed collateral.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignedUsd(i128);

impl SignedUsd {
    pub const ZERO: SignedUsd = SignedUsd(0);

    pub const fn from_micro(micro: i128) -> Self {
        SignedUsd(micro)
    }

    pub const fn micro(self) -> i128 {
        self.0
    }

    pub fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub fn saturating_add(self, other: SignedUsd) -> SignedUsd {
        SignedUsd(self.0.saturating_add(other.0))
    }

    pub fn to_plain_string(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        format!("{sign}{}", render_decimal(self.0.unsigned_abs()))
    }

    pub fn format_dollars(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        format!("{sign}{}", Usd(self.0.unsigned_abs()).format_dollars())
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let (negative, micro) = parse_decimal(text)?;
        if negative && micro == i128::MIN.unsigned_abs() {
            return Ok(SignedUsd(i128::MIN));
        }
        let magnitude = i128::try_from(micro).map_err(|_| format!("{text:?} overflows"))?;
        Ok(SignedUsd(if negative { -magnitude } else { magnitude }))
    }
}

impl Add for SignedUsd {
    type Output = SignedUsd;
    fn add(self, other: SignedUsd) -> SignedUsd {
        self.saturating_add(other)
    }
}

impl Sum for SignedUsd {
    fn sum<I: Iterator<Item = SignedUsd>>(iter: I) -> SignedUsd {
        iter.fold(SignedUsd::ZERO, SignedUsd::saturating_add)
    }
}

impl fmt::Display for SignedUsd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_dollars())
    }
}

impl Serialize for SignedUsd {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_plain_string())
    }
}

impl<'de> Deserialize<'de> for SignedUsd {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        SignedUsd::parse(&text).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_boundaries_and_malformed_decimals() {
        let max = Usd::from_micro(u128::MAX);
        assert_eq!(Usd::parse(&max.to_plain_string()).unwrap(), max);
        assert!(!max.format_dollars().is_empty());
        assert_eq!(max.signed_sub(Usd::from_micro(u128::MAX - 1)).micro(), 1);
        assert!(max.checked_signed_sub(Usd::ZERO).is_none());
        let min = SignedUsd::from_micro(i128::MIN);
        assert_eq!(SignedUsd::parse(&min.to_plain_string()).unwrap(), min);
        for invalid in [".", "-.", "1.+2", "++1", "1.-2"] {
            assert!(Usd::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn renders_full_precision_for_json() {
        assert_eq!(
            Usd::from_micro(9_950_000_000).to_plain_string(),
            "9950.000000"
        );
        assert_eq!(Usd::from_micro(1).to_plain_string(), "0.000001");
        assert_eq!(Usd::ZERO.to_plain_string(), "0.000000");
    }

    #[test]
    fn renders_grouped_dollars_for_terminals() {
        assert_eq!(
            Usd::from_micro(12_841_220_000_000).format_dollars(),
            "$12,841,220.00"
        );
        assert_eq!(Usd::from_micro(9_950_000_000).format_dollars(), "$9,950.00");
        assert_eq!(Usd::from_micro(999_999).format_dollars(), "$1.00");
        assert_eq!(Usd::from_micro(504_999).format_dollars(), "$0.50");
        assert_eq!(Usd::from_micro(505_000).format_dollars(), "$0.51");
    }

    #[test]
    fn round_trips_through_json() {
        for micro in [0u128, 1, 999_999, 7_930_000_000, 12_841_220_000_000] {
            let value = Usd::from_micro(micro);
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(serde_json::from_str::<Usd>(&json).unwrap(), value);
        }
    }

    #[test]
    fn signed_values_round_trip_and_render() {
        let negative = Usd::from_micro(100).signed_sub(Usd::from_micro(2_500_000));
        assert_eq!(negative.to_plain_string(), "-2.499900");
        assert_eq!(negative.format_dollars(), "-$2.50");
        assert!(negative.is_negative());
        let json = serde_json::to_string(&negative).unwrap();
        assert_eq!(serde_json::from_str::<SignedUsd>(&json).unwrap(), negative);
    }

    /// The reason these types exist. 2^53 + 1 micro-USD is the first integer a
    /// double cannot represent; a report that silently rounded here would be
    /// wrong in a way nobody would notice.
    #[test]
    fn exact_at_magnitudes_where_f64_would_lose_precision() {
        let micro: u128 = (1u128 << 53) + 1;
        let value = Usd::from_micro(micro);
        assert_eq!(value.micro(), micro);
        assert_eq!(value.to_plain_string(), "9007199254.740993");
        // Demonstrate the loss this representation avoids.
        assert_ne!((micro as f64) as u128, micro);
    }

    #[test]
    fn summation_is_exact_and_order_independent() {
        let values: Vec<Usd> = (1..=1_000)
            .map(|n| Usd::from_micro(n as u128 * 7))
            .collect();
        let forward: Usd = values.iter().copied().sum();
        let backward: Usd = values.iter().rev().copied().sum();
        assert_eq!(forward, backward);
        assert_eq!(
            forward.micro(),
            (1..=1_000u128).map(|n| n * 7).sum::<u128>()
        );
    }

    #[test]
    fn rejects_malformed_and_over_precise_input() {
        assert!(Usd::parse("").is_err());
        assert!(Usd::parse("abc").is_err());
        assert!(
            Usd::parse("1.1234567").is_err(),
            "more precision than micro-USD"
        );
        assert!(
            Usd::parse("-5.00").is_err(),
            "negative rejected by the unsigned type"
        );
        assert_eq!(Usd::parse("12.5").unwrap(), Usd::from_micro(12_500_000));
        assert_eq!(
            SignedUsd::parse("-12.5").unwrap(),
            SignedUsd::from_micro(-12_500_000)
        );
    }
}
