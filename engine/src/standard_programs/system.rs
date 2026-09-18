//! The System program's instruction encoding.
//!
//! Only the variants the engine has a reason to recognise are modelled, and the
//! rest are [`SystemInstruction::Other`] carrying their discriminant. An
//! unmodelled System instruction is a real thing that happened, and reporting
//! it as "not a transfer" is honest where reporting it as nothing would not be.
//!
//! This exists because "is this a plain lamport transfer, or is it account
//! creation?" is a question every protocol that touches the System program
//! asks, and the answer is the System program's, not the protocol's.

use super::{u32_at, u64_at, Decoded, MalformedReason, SchemaProvenance};

pub const PROGRAM_ID: &str = "11111111111111111111111111111111";

/// `SystemInstruction` variant indices, borsh-tagged as a `u32`.
const CREATE_ACCOUNT: u32 = 0;
const ASSIGN: u32 = 1;
const TRANSFER: u32 = 2;
const CREATE_ACCOUNT_WITH_SEED: u32 = 3;
const ALLOCATE: u32 = 8;

/// `Transfer` is the discriminant plus a `u64`.
pub const TRANSFER_LEN: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemInstruction {
    /// A plain lamport transfer: `from` and `to`, nothing allocated.
    Transfer {
        lamports: u64,
    },
    /// Allocates an account. Outside every replay contract in this build, and
    /// named so that "this was not a transfer" carries the reason.
    CreateAccount,
    CreateAccountWithSeed,
    Allocate,
    Assign,
    /// A System instruction this build does not model.
    Other {
        discriminant: u32,
    },
}

impl SystemInstruction {
    /// Whether this instruction changes an account's existence or ownership
    /// rather than only its lamports.
    pub fn is_lifecycle(&self) -> bool {
        matches!(
            self,
            Self::CreateAccount | Self::CreateAccountWithSeed | Self::Allocate | Self::Assign
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Transfer { .. } => "Transfer",
            Self::CreateAccount => "CreateAccount",
            Self::CreateAccountWithSeed => "CreateAccountWithSeed",
            Self::Allocate => "Allocate",
            Self::Assign => "Assign",
            Self::Other { .. } => "unmodelled System instruction",
        }
    }
}

/// Decode System instruction data.
///
/// A `Transfer` must carry exactly [`TRANSFER_LEN`] bytes: a longer buffer with
/// a transfer discriminant is not a transfer this layer will vouch for.
pub fn decode_instruction(data: &[u8]) -> Decoded<SystemInstruction> {
    let Some(discriminant) = u32_at(data, 0) else {
        return Decoded::Malformed(MalformedReason::Truncated {
            needed: 4,
            found: data.len(),
        });
    };
    Decoded::Decoded(match discriminant {
        TRANSFER => {
            if data.len() != TRANSFER_LEN {
                return Decoded::Malformed(MalformedReason::UnexpectedLength { found: data.len() });
            }
            match u64_at(data, 4) {
                Some(lamports) => SystemInstruction::Transfer { lamports },
                None => {
                    return Decoded::Malformed(MalformedReason::Truncated {
                        needed: TRANSFER_LEN,
                        found: data.len(),
                    })
                }
            }
        }
        CREATE_ACCOUNT => SystemInstruction::CreateAccount,
        CREATE_ACCOUNT_WITH_SEED => SystemInstruction::CreateAccountWithSeed,
        ALLOCATE => SystemInstruction::Allocate,
        ASSIGN => SystemInstruction::Assign,
        discriminant => SystemInstruction::Other { discriminant },
    })
}

pub fn provenance() -> SchemaProvenance {
    SchemaProvenance::StandardProgram
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transfer(lamports: u64) -> Vec<u8> {
        let mut data = TRANSFER.to_le_bytes().to_vec();
        data.extend_from_slice(&lamports.to_le_bytes());
        data
    }

    #[test]
    fn a_plain_transfer_decodes_with_its_amount() {
        assert_eq!(
            decode_instruction(&transfer(1_000_000)).ok(),
            Some(SystemInstruction::Transfer {
                lamports: 1_000_000
            })
        );
    }

    #[test]
    fn account_creation_is_named_rather_than_reported_as_not_a_transfer() {
        let created = decode_instruction(&CREATE_ACCOUNT.to_le_bytes())
            .ok()
            .expect("decodes");
        assert_eq!(created, SystemInstruction::CreateAccount);
        assert!(created.is_lifecycle());
        assert!(!SystemInstruction::Transfer { lamports: 1 }.is_lifecycle());
    }

    #[test]
    fn a_transfer_discriminant_with_the_wrong_length_is_malformed() {
        let mut data = transfer(5);
        data.push(0);
        assert!(decode_instruction(&data).is_malformed());
        assert!(decode_instruction(&data[..8]).is_malformed());
    }

    #[test]
    fn an_unmodelled_variant_keeps_its_discriminant() {
        assert_eq!(
            decode_instruction(&99_u32.to_le_bytes()).ok(),
            Some(SystemInstruction::Other { discriminant: 99 })
        );
    }

    #[test]
    fn empty_data_is_malformed_not_a_transfer_of_zero() {
        assert!(decode_instruction(&[]).is_malformed());
    }
}
