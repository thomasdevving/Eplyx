//! SPL-Stake-Pool-compatible upgrade candidate with one intentional regression.
//!
//! This is a locally constructed counterexample. It is not a proposed upstream
//! release, not a claim about any real deployment, and not code anyone should
//! deploy. It exists because the real historical upgrade this phase measures
//! turns out to preserve behaviour, and a preserved outcome cannot by itself
//! show that a changed outcome would have been caught.
//!
//! The candidate implements `DepositSol` with the validation the deployed
//! program applies to this path - pool ownership and initialization, the
//! withdraw-authority program address, the reserve, mint and manager-fee
//! accounts the pool names, the token program the pool names, and a signing
//! lamport source - and performs the same two cross-program invocations in the
//! same order: a System transfer into the reserve, then a `MintTo` signed by the
//! withdraw authority. It differs in exactly one place.
//!
//! The defect is in the share calculation. The deployed program computes
//! `lamports * pool_token_supply / total_lamports` in `u128`, multiplying before
//! dividing. This candidate precomputes the exchange rate once at four decimal
//! places and multiplies by that instead - the shape a "cache the rate" refactor
//! takes. Every deposit then truncates to the nearest basis point of the rate,
//! so the depositor receives slightly fewer pool tokens and the difference stays
//! in the pool. The transaction still succeeds, the invocation graph is
//! unchanged, and the only evidence is the number of tokens minted.

#![deny(unsafe_code)]

use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
};

solana_program::declare_id!("SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy");

/// `StakePoolInstruction::DepositSol`.
const DEPOSIT_SOL: u8 = 14;
/// `SystemInstruction::Transfer`.
const SYSTEM_TRANSFER: u32 = 2;
/// `TokenInstruction::MintTo`.
const TOKEN_MINT_TO: u8 = 7;
/// PDA seed the pool signs its mints with.
const AUTHORITY_WITHDRAW: &[u8] = b"withdraw";

/// `AccountType::StakePool`.
const ACCOUNT_TYPE_STAKE_POOL: u8 = 1;

// Fixed-offset prefix of the `StakePool` layout. Everything past
// `last_update_epoch` is variable-length and this program neither reads nor
// rewrites it, which is what keeps it byte-compatible with a real pool account.
const WITHDRAW_BUMP_OFFSET: usize = 97;
const RESERVE_STAKE_OFFSET: usize = 130;
const POOL_MINT_OFFSET: usize = 162;
const MANAGER_FEE_OFFSET: usize = 194;
const TOKEN_PROGRAM_OFFSET: usize = 226;
const TOTAL_LAMPORTS_OFFSET: usize = 258;
const POOL_TOKEN_SUPPLY_OFFSET: usize = 266;
const LAST_UPDATE_EPOCH_OFFSET: usize = 274;

/// Precision of the cached exchange rate: pool tokens per lamport, to four
/// decimal places. The deployed program caches nothing and divides last.
const RATE_SCALE: u128 = 10_000;

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(entry);

#[cfg(not(feature = "no-entrypoint"))]
fn entry(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    process_instruction(program_id, accounts, data)
}

/// The deployed program's `calc_pool_tokens_for_deposit`.
pub fn reference_pool_tokens(lamports: u64, supply: u64, total: u64) -> u64 {
    if total == 0 || supply == 0 {
        return lamports;
    }
    ((lamports as u128 * supply as u128) / total as u128) as u64
}

/// What this candidate mints instead.
///
/// The rate is computed once and truncated, then applied. The loss is bounded
/// by one part in [`RATE_SCALE`] of the rate, which is why it reads as a
/// rounding change rather than as a transfer of value.
pub fn candidate_pool_tokens(lamports: u64, supply: u64, total: u64) -> u64 {
    if total == 0 || supply == 0 {
        return lamports;
    }
    let rate = (supply as u128 * RATE_SCALE) / total as u128;
    ((lamports as u128 * rate) / RATE_SCALE) as u64
}

/// What this build actually mints.
///
/// Under `reference` the defect is absent, which is what lets the same source
/// produce both sides of a local differential run over a real CPI path. The
/// two builds are otherwise identical, so a difference between them is the
/// share calculation and nothing else.
pub fn minted_pool_tokens(lamports: u64, supply: u64, total: u64) -> u64 {
    #[cfg(feature = "reference")]
    {
        reference_pool_tokens(lamports, supply, total)
    }
    #[cfg(not(feature = "reference"))]
    {
        candidate_pool_tokens(lamports, supply, total)
    }
}

fn u64_at(data: &[u8], offset: usize) -> Result<u64, ProgramError> {
    data.get(offset..offset + 8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(ProgramError::InvalidAccountData)
}

fn write_u64(data: &mut [u8], offset: usize, value: u64) -> Result<(), ProgramError> {
    data.get_mut(offset..offset + 8)
        .ok_or(ProgramError::InvalidAccountData)?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn key_at(data: &[u8], offset: usize) -> Result<Pubkey, ProgramError> {
    let bytes: [u8; 32] = data
        .get(offset..offset + 32)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(ProgramError::InvalidAccountData)?;
    Ok(Pubkey::from(bytes))
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let (&discriminant, rest) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    if discriminant != DEPOSIT_SOL {
        msg!("candidate supports DepositSol only");
        return Err(ProgramError::InvalidInstructionData);
    }
    let deposit_lamports = rest
        .get(..8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(ProgramError::InvalidInstructionData)?;

    let [pool, withdraw_authority, reserve, from, destination, manager_fee, _referral, mint, system_program, token_program, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    msg!("Instruction: DepositSol");

    if pool.owner != program_id {
        return Err(ProgramError::IncorrectProgramId);
    }
    if !from.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let (bump, supply, total) = {
        let data = pool.try_borrow_data()?;
        if data.first().copied() != Some(ACCOUNT_TYPE_STAKE_POOL) {
            msg!("stake pool is uninitialized");
            return Err(ProgramError::InvalidAccountData);
        }
        // Every account the instruction names must be the one pool state names.
        for (offset, supplied) in [
            (RESERVE_STAKE_OFFSET, reserve.key),
            (POOL_MINT_OFFSET, mint.key),
            (MANAGER_FEE_OFFSET, manager_fee.key),
            (TOKEN_PROGRAM_OFFSET, token_program.key),
        ] {
            if key_at(&data, offset)? != *supplied {
                msg!("account does not match the one the pool names");
                return Err(ProgramError::InvalidArgument);
            }
        }
        // Reading it keeps the layout walk honest about where the totals are.
        let _last_update_epoch = u64_at(&data, LAST_UPDATE_EPOCH_OFFSET)?;
        (
            *data
                .get(WITHDRAW_BUMP_OFFSET)
                .ok_or(ProgramError::InvalidAccountData)?,
            u64_at(&data, POOL_TOKEN_SUPPLY_OFFSET)?,
            u64_at(&data, TOTAL_LAMPORTS_OFFSET)?,
        )
    };

    let seeds = [pool.key.as_ref(), AUTHORITY_WITHDRAW, &[bump]];
    let expected_authority = Pubkey::create_program_address(&seeds, program_id)
        .map_err(|_| ProgramError::InvalidSeeds)?;
    if expected_authority != *withdraw_authority.key {
        msg!("withdraw authority is not the pool's program address");
        return Err(ProgramError::InvalidSeeds);
    }

    let new_pool_tokens = minted_pool_tokens(deposit_lamports, supply, total);
    if new_pool_tokens == 0 {
        return Err(ProgramError::InsufficientFunds);
    }

    invoke(
        &Instruction {
            program_id: *system_program.key,
            accounts: vec![
                AccountMeta::new(*from.key, true),
                AccountMeta::new(*reserve.key, false),
            ],
            data: SYSTEM_TRANSFER
                .to_le_bytes()
                .iter()
                .copied()
                .chain(deposit_lamports.to_le_bytes())
                .collect(),
        },
        &[from.clone(), reserve.clone(), system_program.clone()],
    )?;

    invoke_signed(
        &Instruction {
            program_id: *token_program.key,
            accounts: vec![
                AccountMeta::new(*mint.key, false),
                AccountMeta::new(*destination.key, false),
                AccountMeta::new_readonly(*withdraw_authority.key, true),
            ],
            data: std::iter::once(TOKEN_MINT_TO)
                .chain(new_pool_tokens.to_le_bytes())
                .collect(),
        },
        &[
            mint.clone(),
            destination.clone(),
            withdraw_authority.clone(),
            token_program.clone(),
        ],
        &[&seeds],
    )?;

    let mut data = pool.try_borrow_mut_data()?;
    write_u64(
        &mut data,
        TOTAL_LAMPORTS_OFFSET,
        total
            .checked_add(deposit_lamports)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    )?;
    write_u64(
        &mut data,
        POOL_TOKEN_SUPPLY_OFFSET,
        supply
            .checked_add(new_pool_tokens)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pool state the replayed mainnet deposit actually executed against.
    const SUPPLY: u64 = 291_875_877_314_830;
    const TOTAL: u64 = 311_850_055_457_947;
    const DEPOSIT: u64 = 423_000_000;

    /// The mint amount mainnet recorded for this transaction.
    #[test]
    fn the_reference_reproduces_the_observed_mint() {
        assert_eq!(reference_pool_tokens(DEPOSIT, SUPPLY, TOTAL), 395_906_603);
    }

    #[test]
    fn the_candidate_mints_less_for_the_same_deposit() {
        let reference = reference_pool_tokens(DEPOSIT, SUPPLY, TOTAL);
        let candidate = candidate_pool_tokens(DEPOSIT, SUPPLY, TOTAL);
        assert!(candidate < reference, "{candidate} vs {reference}");
        assert_eq!(reference - candidate, 20_903);
    }

    /// The loss is a truncation of the rate, so it is bounded by one part in
    /// `RATE_SCALE` of the deposit rather than being a fixed skim.
    #[test]
    fn the_loss_is_bounded_by_the_rate_precision() {
        for lamports in [1_000_000, 423_000_000, 10_000_000_000, 250_000_000_000] {
            let reference = reference_pool_tokens(lamports, SUPPLY, TOTAL);
            let candidate = candidate_pool_tokens(lamports, SUPPLY, TOTAL);
            assert!(candidate <= reference);
            assert!(
                u128::from(reference - candidate) * RATE_SCALE <= u128::from(reference) * 2,
                "loss on {lamports} exceeds the rate precision"
            );
        }
    }

    /// An empty pool mints one for one under both, so the defect only shows on
    /// a pool that has a rate at all.
    /// Which arithmetic this build ships. The differential run is vacuous if
    /// both sides compute the same thing, so the wiring is asserted directly.
    #[test]
    fn the_build_ships_the_arithmetic_its_feature_selects() {
        let minted = minted_pool_tokens(DEPOSIT, SUPPLY, TOTAL);
        if cfg!(feature = "reference") {
            assert_eq!(minted, reference_pool_tokens(DEPOSIT, SUPPLY, TOTAL));
        } else {
            assert_eq!(minted, candidate_pool_tokens(DEPOSIT, SUPPLY, TOTAL));
            assert_ne!(minted, reference_pool_tokens(DEPOSIT, SUPPLY, TOTAL));
        }
    }

    #[test]
    fn an_empty_pool_is_unaffected() {
        assert_eq!(candidate_pool_tokens(5_000, 0, 0), 5_000);
        assert_eq!(reference_pool_tokens(5_000, 0, 0), 5_000);
    }

    #[test]
    fn a_non_deposit_discriminant_is_rejected() {
        assert_eq!(
            process_instruction(&id(), &[], &[9, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(ProgramError::InvalidInstructionData)
        );
    }

    #[test]
    fn empty_instruction_data_is_rejected() {
        assert_eq!(
            process_instruction(&id(), &[], &[]),
            Err(ProgramError::InvalidInstructionData)
        );
    }
}
