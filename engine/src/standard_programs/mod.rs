//! Decoders for programs the runtime or a pinned dependency binary defines.
//!
//! A *standard program* is one whose layout is not a protocol's private
//! knowledge: System, SPL Token, Token-2022. Their account layouts are
//! published, stable across deployments, and needed by every protocol that
//! touches a token — which is nearly all of them. Before Phase U1 each adapter
//! carried its own copy of the 165-byte token account layout, and the third
//! would have carried a third.
//!
//! This is deliberately *not* the protocol adapter seam. An adapter says what a
//! balance change **means**; a standard-program decoder says only what the
//! bytes **are**. Nothing in this module may name a deposit, a withdrawal, a
//! fee or a receipt.
//!
//! ## What a decoder here may claim
//!
//! It may claim that a byte range holds a field of a given type under a named
//! layout. It may not claim that the layout is what the deployed program
//! actually implements: that would require a reproducible build matching the
//! deployed hash, which this layer does not perform. See
//! [`SchemaProvenance`] — the claim is recorded, never inferred.

pub mod spl_token;
pub mod system;
pub mod token2022;

use serde::{Deserialize, Serialize};

/// Why a layout is believed to describe a program's accounts.
///
/// Recorded rather than assumed. The research phase's finding stands: an
/// on-chain IDL is published by the upgrade authority, which is precisely the
/// party whose change Eplyx exists to measure, so "an interface says so" is
/// never the same claim as "the deployed bytes do so".
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaProvenance {
    /// The layout is part of a program's published, stable interface and the
    /// binary that implements it is the runtime's own or a hash-pinned
    /// dependency. This is the strongest rung this layer can reach, and it
    /// still does not prove source-to-bytecode equivalence.
    StandardProgram,
    /// A layout hand-written against program source, reviewed by a person. What
    /// every protocol adapter in this repository uses today.
    ManualInterface,
}

/// The outcome of attempting to read a structure out of account bytes.
///
/// Four states rather than `Option`, because the brief's hard rule is that a
/// decode failure must never be indistinguishable from an absence. A caller
/// that sees [`Decoded::Malformed`] knows bytes were present and wrong;
/// [`Decoded::NotApplicable`] means they were never this kind of account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decoded<T> {
    /// The bytes parsed under the named layout.
    Decoded(T),
    /// These bytes are not this structure at all — a mint asked for as a token
    /// account, an account owned by another program. Not an error.
    NotApplicable,
    /// The bytes claim to be this structure and are not readable as one: a
    /// truncated buffer, an out-of-range discriminant, a length that no version
    /// of the layout produces. Always an explicit result, never a zero.
    Malformed(MalformedReason),
    /// Recognised as this structure, carrying something this build does not
    /// model — an unknown Token-2022 extension, most concretely. Visible rather
    /// than silently dropped.
    Unsupported(&'static str),
}

impl<T> Decoded<T> {
    pub fn ok(self) -> Option<T> {
        match self {
            Self::Decoded(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_ref(&self) -> Decoded<&T> {
        match self {
            Self::Decoded(value) => Decoded::Decoded(value),
            Self::NotApplicable => Decoded::NotApplicable,
            Self::Malformed(reason) => Decoded::Malformed(*reason),
            Self::Unsupported(what) => Decoded::Unsupported(what),
        }
    }

    pub fn is_malformed(&self) -> bool {
        matches!(self, Self::Malformed(_))
    }
}

/// Why bytes that claimed to be a structure could not be read as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MalformedReason {
    /// The buffer is shorter than the layout requires.
    Truncated { needed: usize, found: usize },
    /// A length no version of this layout produces.
    UnexpectedLength { found: usize },
    /// A tagged union's tag is outside its declared range — a `COption`
    /// discriminant that is neither 0 nor 1, an account type byte that names
    /// nothing.
    InvalidDiscriminant { at: usize, value: u32 },
    /// A TLV entry declares a length that runs past the end of the buffer.
    ExtensionOverrun { at: usize, declared: usize },
}

impl std::fmt::Display for MalformedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, found } => {
                write!(f, "truncated: needs {needed} bytes, found {found}")
            }
            Self::UnexpectedLength { found } => write!(f, "unexpected length {found}"),
            Self::InvalidDiscriminant { at, value } => {
                write!(f, "invalid discriminant {value} at offset {at}")
            }
            Self::ExtensionOverrun { at, declared } => {
                write!(
                    f,
                    "extension at {at} declares {declared} bytes past the buffer"
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Primitive readers
// ---------------------------------------------------------------------------
//
// Defined once here rather than three times across adapters. Every one is
// bounds-checked and returns `None` past the end — a truncated buffer must
// never read as a zero, which is the failure mode that makes a malformed
// account look like an empty one.

pub fn u8_at(data: &[u8], offset: usize) -> Option<u8> {
    data.get(offset).copied()
}

pub fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

pub fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

pub fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

pub fn address_at(data: &[u8], offset: usize) -> Option<String> {
    Some(bs58::encode(data.get(offset..offset + 32)?).into_string())
}

/// A `COption<Pubkey>`: a 4-byte little-endian tag, then the key.
///
/// `Ok(None)` is a present-and-absent option; `Err` is a tag that is neither 0
/// nor 1, which means the buffer is not this layout.
pub fn coption_address_at(data: &[u8], offset: usize) -> Result<Option<String>, MalformedReason> {
    let tag = u32_at(data, offset).ok_or(MalformedReason::Truncated {
        needed: offset + 4,
        found: data.len(),
    })?;
    match tag {
        0 => Ok(None),
        1 => address_at(data, offset + 4)
            .map(Some)
            .ok_or(MalformedReason::Truncated {
                needed: offset + 36,
                found: data.len(),
            }),
        value => Err(MalformedReason::InvalidDiscriminant { at: offset, value }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_refuse_to_read_past_the_end() {
        let data = [1_u8, 2, 3];
        assert_eq!(u64_at(&data, 0), None);
        assert_eq!(u32_at(&data, 0), None);
        assert_eq!(u16_at(&data, 2), None);
        assert_eq!(address_at(&data, 0), None);
        assert_eq!(u8_at(&data, 3), None);
        assert_eq!(u8_at(&data, 2), Some(3));
    }

    /// The failure this guards: a short buffer reading as zero would make a
    /// truncated token account indistinguishable from an empty one.
    #[test]
    fn a_truncated_buffer_does_not_read_as_zero() {
        let data = [0_u8; 7];
        assert_eq!(
            u64_at(&data, 0),
            None,
            "seven zero bytes are not a zero u64"
        );
    }

    #[test]
    fn a_coption_tag_outside_its_range_is_malformed_not_absent() {
        let mut data = vec![0_u8; 36];
        data[..4].copy_from_slice(&7_u32.to_le_bytes());
        assert_eq!(
            coption_address_at(&data, 0),
            Err(MalformedReason::InvalidDiscriminant { at: 0, value: 7 })
        );
        data[..4].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(coption_address_at(&data, 0), Ok(None));
        data[..4].copy_from_slice(&1_u32.to_le_bytes());
        assert!(coption_address_at(&data, 0).unwrap().is_some());
    }

    #[test]
    fn decoded_states_are_distinguishable() {
        let malformed: Decoded<u8> =
            Decoded::Malformed(MalformedReason::UnexpectedLength { found: 3 });
        assert!(malformed.is_malformed());
        assert_eq!(malformed.clone().ok(), None);
        assert_eq!(Decoded::<u8>::NotApplicable.ok(), None);
        assert!(!Decoded::<u8>::NotApplicable.is_malformed());
        assert_eq!(Decoded::Decoded(7_u8).ok(), Some(7));
    }
}
