//! Typed field measurement, where a layout is known.
//!
//! Where a layout is known, a field delta says what moved. Where it is not, a
//! byte-range delta says only that something did — which is less, and is the
//! honest amount.
//!
//! ## What this does not become
//!
//! Not an IDL system. Not a schema registry. Not a place a protocol declares
//! its accounts. Phase U1 builds the *boundary* so that a later phase can add
//! sources behind it; it adds no source beyond the two that already existed.
//! [`FieldSchema`] has exactly the inhabitants [`crate::diff::FieldDecoder`]
//! had, because adding one here without an implementation behind it would be a
//! claim with no test.
//!
//! ## Provenance is required, not optional
//!
//! A raw byte offset is not a semantic contract. Every [`TypedFieldDelta`]
//! carries the schema that produced it and that schema's provenance, so a
//! reader can tell a field read under a standard program's published layout
//! from one read under a hand-written interface. Without that distinction,
//! "offset 258 changed" reads as authoritative when it is an assertion.

use super::{DecoderIdentity, Provenance};
use crate::standard_programs::SchemaProvenance;

/// Which layout a field was read under.
///
/// Deliberately closed and deliberately small. The generic diff may decode the
/// synthetic fixture layout or nothing, and that is still true after Phase U1 —
/// what changed is that the choice now names a *schema* with provenance rather
/// than being an untyped decoder switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FieldSchema {
    /// No layout is claimed. Bytes are compared as bytes.
    Opaque,
    /// The synthetic fixture-lending layout, defined in this repository by the
    /// `interface` crate that both the program and the engine link.
    FixtureLending,
}

impl FieldSchema {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Opaque => "opaque",
            Self::FixtureLending => "fixture-lending",
        }
    }

    /// What the layout claim rests on.
    ///
    /// `FixtureLending` is [`SchemaProvenance::ManualInterface`] and not
    /// anything stronger: the layout is hand-written, reviewed, and shared by
    /// construction with the program under test — which makes it trustworthy
    /// *here* and says nothing about an arbitrary deployed binary.
    pub fn provenance(self) -> SchemaProvenance {
        match self {
            Self::Opaque | Self::FixtureLending => SchemaProvenance::ManualInterface,
        }
    }

    pub fn decoder(self) -> DecoderIdentity {
        DecoderIdentity::manual(
            match self {
                Self::Opaque => "opaque",
                Self::FixtureLending => "fixture-lending",
            },
            1,
        )
    }
}

/// A value read out of account bytes at a known position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Integer(u128),
    SignedInteger(i128),
    Address(String),
    Flag(bool),
    Bytes(Vec<u8>),
}

impl FieldValue {
    pub fn render(&self) -> String {
        match self {
            Self::Integer(value) => value.to_string(),
            Self::SignedInteger(value) => value.to_string(),
            Self::Address(value) => value.clone(),
            Self::Flag(value) => value.to_string(),
            Self::Bytes(bytes) => crate::hexfmt::encode(bytes),
        }
    }
}

/// One field's value across a boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedFieldDelta {
    pub provenance: Provenance,
    pub schema: FieldSchema,
    /// A dotted path within the layout: `market.total_collateral`.
    pub field_path: String,
    /// Byte range the value was read from, so a reader can check it.
    pub byte_range: std::ops::Range<usize>,
    pub before: FieldValue,
    pub after: FieldValue,
}

impl TypedFieldDelta {
    pub fn changed(&self) -> bool {
        self.before != self.after
    }

    /// Signed difference where both sides are integers, `None` otherwise.
    ///
    /// `None` for an address or a flag is not a failure: those do not subtract,
    /// and returning a zero would say they were equal.
    pub fn numeric_delta(&self) -> Option<i128> {
        match (&self.before, &self.after) {
            (FieldValue::Integer(before), FieldValue::Integer(after)) => {
                Some(i128::try_from(*after).ok()? - i128::try_from(*before).ok()?)
            }
            (FieldValue::SignedInteger(before), FieldValue::SignedInteger(after)) => {
                Some(after - before)
            }
            _ => None,
        }
    }
}

/// A byte range that differs, where no layout explains it.
///
/// The weaker sibling of [`TypedFieldDelta`], and the one that must exist: an
/// account with no known layout still has state, and reporting nothing about it
/// because nothing could be named would be a gap that looks like coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteRangeDelta {
    pub provenance: Provenance,
    pub range: std::ops::Range<usize>,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

/// Every contiguous run of differing bytes between two buffers.
///
/// Runs rather than individual offsets, because a changed `u64` is eight
/// adjacent differing bytes and reporting eight findings for one field would be
/// noise. Buffers of different lengths report the common prefix's differences
/// plus one range covering the tail, so a truncation is never silently dropped.
pub fn byte_ranges(provenance: &Provenance, before: &[u8], after: &[u8]) -> Vec<ByteRangeDelta> {
    let mut ranges = Vec::new();
    let common = before.len().min(after.len());
    let mut start: Option<usize> = None;
    for offset in 0..common {
        match (before[offset] == after[offset], start) {
            (false, None) => start = Some(offset),
            (true, Some(from)) => {
                ranges.push(ByteRangeDelta {
                    provenance: provenance.clone(),
                    range: from..offset,
                    before: before[from..offset].to_vec(),
                    after: after[from..offset].to_vec(),
                });
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        ranges.push(ByteRangeDelta {
            provenance: provenance.clone(),
            range: from..common,
            before: before[from..common].to_vec(),
            after: after[from..common].to_vec(),
        });
    }
    if before.len() != after.len() {
        ranges.push(ByteRangeDelta {
            provenance: provenance.clone(),
            range: common..before.len().max(after.len()),
            before: before.get(common..).unwrap_or_default().to_vec(),
            after: after.get(common..).unwrap_or_default().to_vec(),
        });
    }
    ranges
}

/// Read a big-endian-free little-endian `u64` as a typed field.
///
/// Bounds-checked: a read past the end is `None`, never a zero. This is the
/// primitive every "the field at offset N" measurement goes through, so the
/// bounds check happens once.
pub fn u64_field(
    provenance: &Provenance,
    schema: FieldSchema,
    field_path: &str,
    offset: usize,
    before: &[u8],
    after: &[u8],
) -> Option<TypedFieldDelta> {
    let read = |data: &[u8]| -> Option<u128> {
        Some(u128::from(u64::from_le_bytes(
            data.get(offset..offset + 8)?.try_into().ok()?,
        )))
    };
    let mut provenance = provenance.clone();
    provenance.decoder = schema.decoder();
    Some(TypedFieldDelta {
        provenance,
        schema,
        field_path: field_path.to_string(),
        byte_range: offset..offset + 8,
        before: FieldValue::Integer(read(before)?),
        after: FieldValue::Integer(read(after)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> Provenance {
        Provenance {
            record: "r".into(),
            account_label: "market".into(),
            address: Some("addr".into()),
            decoder: DecoderIdentity::manual("test", 1),
            origin: None,
        }
    }

    #[test]
    fn a_schema_carries_its_provenance_rather_than_implying_one() {
        assert_eq!(
            FieldSchema::FixtureLending.provenance(),
            SchemaProvenance::ManualInterface
        );
        assert_eq!(FieldSchema::FixtureLending.as_str(), "fixture-lending");
        assert_eq!(FieldSchema::Opaque.as_str(), "opaque");
    }

    #[test]
    fn a_typed_field_reports_its_range_and_its_delta() {
        let mut before = vec![0_u8; 32];
        let mut after = vec![0_u8; 32];
        before[8..16].copy_from_slice(&1_000_u64.to_le_bytes());
        after[8..16].copy_from_slice(&1_500_u64.to_le_bytes());
        let delta = u64_field(
            &provenance(),
            FieldSchema::FixtureLending,
            "market.total",
            8,
            &before,
            &after,
        )
        .expect("in range");
        assert!(delta.changed());
        assert_eq!(delta.byte_range, 8..16);
        assert_eq!(delta.numeric_delta(), Some(500));
        assert_eq!(delta.provenance.decoder.name, "fixture-lending");
    }

    #[test]
    fn a_field_read_past_the_end_is_absent_not_zero() {
        let short = vec![0_u8; 4];
        assert!(u64_field(
            &provenance(),
            FieldSchema::FixtureLending,
            "f",
            0,
            &short,
            &short
        )
        .is_none());
    }

    #[test]
    fn a_non_numeric_field_has_no_numeric_delta() {
        let delta = TypedFieldDelta {
            provenance: provenance(),
            schema: FieldSchema::FixtureLending,
            field_path: "market.authority".into(),
            byte_range: 0..32,
            before: FieldValue::Address("a".into()),
            after: FieldValue::Address("b".into()),
        };
        assert!(delta.changed());
        assert_eq!(
            delta.numeric_delta(),
            None,
            "an address does not subtract, and must not report zero"
        );
    }

    #[test]
    fn contiguous_changes_are_one_range_not_eight_findings() {
        let before = vec![0_u8; 16];
        let mut after = before.clone();
        after[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
        let ranges = byte_ranges(&provenance(), &before, &after);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].range, 4..12);
    }

    #[test]
    fn separated_changes_are_separate_ranges() {
        let before = vec![0_u8; 16];
        let mut after = before.clone();
        after[2] = 1;
        after[10] = 1;
        let ranges = byte_ranges(&provenance(), &before, &after);
        assert_eq!(
            ranges.iter().map(|r| r.range.clone()).collect::<Vec<_>>(),
            vec![2..3, 10..11]
        );
    }

    #[test]
    fn identical_buffers_produce_no_ranges() {
        let data = vec![7_u8; 16];
        assert!(byte_ranges(&provenance(), &data, &data).is_empty());
    }

    /// A truncation must never be reported as "the common prefix matched".
    #[test]
    fn a_length_change_is_always_reported() {
        let before = vec![1_u8; 16];
        let after = vec![1_u8; 8];
        let ranges = byte_ranges(&provenance(), &before, &after);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].range, 8..16);
        assert_eq!(ranges[0].after, Vec::<u8>::new());
    }

    #[test]
    fn a_change_running_to_the_end_is_closed_off() {
        let before = vec![0_u8; 8];
        let after = vec![9_u8; 8];
        let ranges = byte_ranges(&provenance(), &before, &after);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].range, 0..8);
    }
}
