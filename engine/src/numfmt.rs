//! Integers that are too large for a JSON number.
//!
//! A `u64` reaches 1.8e19 and a lamport balance genuinely gets there: a pool
//! holding 15 million SOL is 1.5e16 lamports, past the 2^53 where most JSON
//! parsers — every JavaScript one — silently start rounding. A report that a
//! frontend cannot read back exactly is not a report.
//!
//! So these fields serialize as decimal strings. Deserialization accepts a
//! number as well, because records written before this change are evidence and
//! must keep parsing; only the writing side moved.
//!
//! This is separate from [`crate::money`] and from token quantities, which were
//! already strings for the same reason. What is left as a plain number is what
//! cannot plausibly approach 2^53: counts, basis points, slots, byte offsets.

use serde::{Deserialize, Deserializer, Serializer};

pub mod u64_string {
    use super::*;

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        use serde::de::Error as _;
        match serde_json::Value::deserialize(deserializer)? {
            serde_json::Value::String(text) => text.parse().map_err(D::Error::custom),
            serde_json::Value::Number(number) => number
                .as_u64()
                .ok_or_else(|| D::Error::custom(format!("{number} is not a u64"))),
            other => Err(D::Error::custom(format!("expected a u64, found {other}"))),
        }
    }
}

pub mod i64_string {
    use super::*;

    pub fn serialize<S: Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        use serde::de::Error as _;
        match serde_json::Value::deserialize(deserializer)? {
            serde_json::Value::String(text) => text.parse().map_err(D::Error::custom),
            serde_json::Value::Number(number) => number
                .as_i64()
                .ok_or_else(|| D::Error::custom(format!("{number} is not an i64"))),
            other => Err(D::Error::custom(format!("expected an i64, found {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(with = "super::u64_string")]
        lamports: u64,
        #[serde(with = "super::i64_string")]
        delta: i64,
    }

    /// The whole point: a value past 2^53 survives a round trip exactly.
    #[test]
    fn large_values_round_trip_without_loss() {
        let holder = Holder {
            // 15 million SOL, comfortably past 2^53.
            lamports: 15_000_000_000_000_000,
            delta: -9_007_199_254_740_993,
        };
        let json = serde_json::to_string(&holder).unwrap();
        assert!(json.contains(r#""15000000000000000""#), "{json}");
        assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), holder);

        let extreme = Holder {
            lamports: u64::MAX,
            delta: i64::MIN,
        };
        let json = serde_json::to_string(&extreme).unwrap();
        assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), extreme);
    }

    /// Records written before this change are evidence. They must keep parsing.
    #[test]
    fn a_number_is_still_accepted_on_the_way_in() {
        let holder: Holder = serde_json::from_str(r#"{"lamports":42,"delta":-7}"#).unwrap();
        assert_eq!(holder.lamports, 42);
        assert_eq!(holder.delta, -7);
    }

    #[test]
    fn nonsense_is_rejected_rather_than_defaulted() {
        assert!(serde_json::from_str::<Holder>(r#"{"lamports":"x","delta":"0"}"#).is_err());
        assert!(serde_json::from_str::<Holder>(r#"{"lamports":-1,"delta":"0"}"#).is_err());
        assert!(serde_json::from_str::<Holder>(r#"{"lamports":null,"delta":"0"}"#).is_err());
    }
}
