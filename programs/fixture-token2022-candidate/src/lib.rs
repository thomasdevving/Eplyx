//! Token-2022-compatible upgrade candidate with one intentional regression.
//!
//! This is a locally constructed counterexample, not a proposed upstream
//! Token-2022 release and not a claim about any real deployment. It exists to
//! show that the replay path detects an economic change when one is present:
//! the real historical upgrade this phase measures turns out to preserve
//! behaviour, so a preserved outcome alone would not demonstrate that a changed
//! outcome would have been caught.
//!
//! The candidate implements `TransferChecked` with the same validation the
//! deployed program applies to this path - mint agreement, declared decimals,
//! a signing authority, an initialized and unfrozen source, and a sufficient
//! balance - and then introduces a single defect: the destination is credited
//! one basis point less than the source is debited, as an unintended truncation
//! would do. Extension bytes past the base account are left untouched, which is
//! what keeps the candidate byte-compatible with real mainnet accounts.

#![deny(unsafe_code)]

use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, msg, program_error::ProgramError,
    pubkey::Pubkey,
};

solana_program::declare_id!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Base SPL token account and mint layouts. Extensions live past these.
const ACCOUNT_LEN: usize = 165;
const MINT_LEN: usize = 82;
const AMOUNT_OFFSET: usize = 64;
const STATE_OFFSET: usize = 108;
const MINT_DECIMALS_OFFSET: usize = 44;

const STATE_INITIALIZED: u8 = 1;
const TRANSFER_CHECKED: u8 = 12;

/// The defect: one basis point of every transfer fails to arrive.
const REGRESSION_BASIS_POINTS: u64 = 1;

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(entry);

#[cfg(not(feature = "no-entrypoint"))]
fn entry(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    process_instruction(program_id, accounts, data)
}

fn read_amount(data: &[u8]) -> Result<u64, ProgramError> {
    data.get(AMOUNT_OFFSET..AMOUNT_OFFSET + 8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(ProgramError::InvalidAccountData)
}

fn write_amount(data: &mut [u8], amount: u64) -> Result<(), ProgramError> {
    data.get_mut(AMOUNT_OFFSET..AMOUNT_OFFSET + 8)
        .ok_or(ProgramError::InvalidAccountData)?
        .copy_from_slice(&amount.to_le_bytes());
    Ok(())
}

/// What the destination actually receives. The deployed program credits the
/// full amount; this candidate truncates a basis point off it.
pub fn credited_amount(amount: u64) -> u64 {
    amount.saturating_sub(amount / 10_000 * REGRESSION_BASIS_POINTS)
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let (&discriminant, rest) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    if discriminant != TRANSFER_CHECKED {
        msg!("candidate supports TransferChecked only");
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = rest
        .get(..8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(ProgramError::InvalidInstructionData)?;
    let declared_decimals = *rest.get(8).ok_or(ProgramError::InvalidInstructionData)?;

    let [source, mint, destination, authority, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    for account in [source, mint, destination] {
        if account.owner != program_id {
            return Err(ProgramError::IncorrectProgramId);
        }
    }

    let mint_data = mint.try_borrow_data()?;
    if mint_data.len() < MINT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    if mint_data[MINT_DECIMALS_OFFSET] != declared_decimals {
        msg!("declared decimals do not match the mint");
        return Err(ProgramError::InvalidInstructionData);
    }
    drop(mint_data);

    let mut source_data = source.try_borrow_mut_data()?;
    let mut destination_data = destination.try_borrow_mut_data()?;
    if source_data.len() < ACCOUNT_LEN || destination_data.len() < ACCOUNT_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    if source_data[STATE_OFFSET] != STATE_INITIALIZED
        || destination_data[STATE_OFFSET] != STATE_INITIALIZED
    {
        msg!("token account is uninitialized or frozen");
        return Err(ProgramError::InvalidAccountData);
    }
    // Both accounts must belong to the mint the instruction names, and the
    // authority must own the source.
    if source_data[..32] != mint.key.to_bytes() || destination_data[..32] != mint.key.to_bytes() {
        msg!("token account does not belong to the named mint");
        return Err(ProgramError::InvalidAccountData);
    }
    if source_data[32..64] != authority.key.to_bytes() {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let source_amount = read_amount(&source_data)?;
    let destination_amount = read_amount(&destination_data)?;
    let debited = source_amount
        .checked_sub(amount)
        .ok_or(ProgramError::InsufficientFunds)?;
    let credited = destination_amount
        .checked_add(credited_amount(amount))
        .ok_or(ProgramError::ArithmeticOverflow)?;

    write_amount(&mut source_data, debited)?;
    write_amount(&mut destination_data, credited)?;
    msg!("Instruction: TransferChecked");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The historical PYUSD payment this phase replays: 10.000000 PYUSD.
    #[test]
    fn the_replayed_payment_loses_one_basis_point() {
        assert_eq!(credited_amount(10_000_000), 9_999_000);
    }

    /// The defect is a truncation, so amounts under the rounding step pass
    /// through untouched. That is what makes it the kind of change a byte diff
    /// shows plainly but a spot check can easily miss.
    #[test]
    fn small_amounts_are_unaffected() {
        assert_eq!(credited_amount(9_999), 9_999);
        assert_eq!(credited_amount(0), 0);
    }

    #[test]
    fn large_amounts_lose_proportionally() {
        assert_eq!(credited_amount(1_000_000_000), 999_900_000);
    }

    #[test]
    fn a_non_transfer_discriminant_is_rejected() {
        assert_eq!(
            process_instruction(&id(), &[], &[3, 0, 0, 0, 0, 0, 0, 0, 0, 6]),
            Err(ProgramError::InvalidInstructionData)
        );
    }
}
