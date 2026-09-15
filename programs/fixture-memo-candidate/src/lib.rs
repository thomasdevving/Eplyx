//! Memo-compatible upgrade candidate with one intentional regression.
//!
//! The deployed Memo program accepts a much larger UTF-8 payload. This candidate
//! introduces a 64-byte ceiling so the fixed historical payment becomes a
//! concrete counterexample: its 88-byte memo succeeded on mainnet and fails here.

#![deny(unsafe_code)]

use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, msg, program_error::ProgramError,
    pubkey::Pubkey,
};

solana_program::declare_id!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(entry);

#[cfg(not(feature = "no-entrypoint"))]
fn entry(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    process_instruction(program_id, accounts, data)
}

pub fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    for account in accounts {
        if !account.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }
    }
    let memo = std::str::from_utf8(data).map_err(|_| ProgramError::InvalidInstructionData)?;
    if data.len() > 64 {
        msg!("candidate memo exceeds 64-byte limit");
        return Err(ProgramError::InvalidInstructionData);
    }
    msg!("Memo (len {}): {:?}", data.len(), memo);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_short_utf8() {
        assert_eq!(process_instruction(&id(), &[], b"hello"), Ok(()));
    }

    #[test]
    fn rejects_historical_counterexample_length() {
        assert_eq!(
            process_instruction(&id(), &[], &[b'x'; 88]),
            Err(ProgramError::InvalidInstructionData)
        );
    }
}
