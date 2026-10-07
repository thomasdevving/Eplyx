//! Constructed oversized rollout counterexample. Not an upstream release. Not a
//! production candidate. Not intended for deployment. Qualified only for the
//! stated test boundary: full 2.0.3 pool layout, SolDeposit SetFee and ungated
//! ten-account DepositSol with zero SOL referral percentage.
//!
//! Identical in behaviour to the constructed rollout counterexample
//! (fixture-stake-pool-rollout-candidate): a newly configured SOL deposit fee
//! above 1/200 is rejected, and DepositSol applies whatever fee the pool holds.
//! The only addition is `BALLAST`, deterministic read-only program data that
//! makes the ELF larger than the retained Stake Pool ProgramData capacity, so
//! the rollout rehearsal can exercise an explicit loader ExtendProgram. It is
//! referenced only on the rejected-instruction path, so the qualified SetFee
//! and DepositSol paths never read it.
//!
//! Layout, instructions and checked arithmetic reuse Apache-2.0 SPL Stake Pool
//! 2.0.3 (Solana Labs Maintainers). The bounded CPI/allocation sequence follows
//! its processor::process_deposit_sol. See NOTICE for attribution and scope.
#![deny(unsafe_code)]

use borsh::BorshDeserialize;
use solana_program::{
    account_info::AccountInfo,
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    program_option::COption,
    program_pack::Pack,
    pubkey::Pubkey,
    sysvar::Sysvar,
};
use spl_stake_pool::{
    error::StakePoolError,
    instruction::StakePoolInstruction,
    state::{AccountType, Fee, FeeType, StakePool},
};
use spl_token::state::{Account as TokenAccount, AccountState, Mint};

solana_program::declare_id!("SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy");

/// Bytes of deterministic read-only program data. Chosen so the ELF exceeds the
/// retained ProgramData executable capacity (1,080,464 bytes) by more than the
/// loader's 10 KiB minimum extension, and stays far below its 10 MiB maximum.
pub const BALLAST_LEN: usize = 1_012_000;

/// Non-zero, so it is emitted into the ELF's read-only data rather than
/// zero-initialized memory. Its contents carry no meaning.
pub static BALLAST: [u8; BALLAST_LEN] = [0xA5; BALLAST_LEN];

/// The stricter V2 maximum for a newly configured SOL deposit fee, as an exact
/// fraction: numerator / denominator must not exceed 1/200.
pub const MAX_NEW_SOL_DEPOSIT_FEE: (u64, u64) = (1, 200);

/// Exact rational comparison; no basis-point normalization. `0/0` is a zero fee.
pub fn exceeds_rollout_maximum(fee: &Fee) -> bool {
    let (max_numerator, max_denominator) = MAX_NEW_SOL_DEPOSIT_FEE;
    u128::from(fee.numerator) * u128::from(max_denominator)
        > u128::from(fee.denominator) * u128::from(max_numerator)
}

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process_instruction);

/// Decode the complete official type and require an exact prefix roundtrip.
/// Account padding is retained byte for byte; it is never decoded or normalized.
fn read_pool(pool: &AccountInfo, program_id: &Pubkey) -> Result<(StakePool, usize), ProgramError> {
    if pool.owner != program_id || pool.executable {
        return Err(ProgramError::IncorrectProgramId);
    }
    let data = pool.try_borrow_data()?;
    let mut remaining = &data[..];
    let state =
        StakePool::deserialize(&mut remaining).map_err(|_| ProgramError::InvalidAccountData)?;
    let len = data.len() - remaining.len();
    if state.account_type != AccountType::StakePool
        || borsh::to_vec(&state).map_err(|_| ProgramError::InvalidAccountData)? != data[..len]
    {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok((state, len))
}

fn write_pool(pool: &AccountInfo, state: &StakePool, len: usize) -> ProgramResult {
    let encoded = borsh::to_vec(state).map_err(|_| ProgramError::InvalidAccountData)?;
    if encoded.len() != len {
        return Err(ProgramError::InvalidAccountData);
    }
    pool.try_borrow_mut_data()?[..len].copy_from_slice(&encoded);
    Ok(())
}

fn set_fee(program_id: &Pubkey, accounts: &[AccountInfo], fee: Fee) -> ProgramResult {
    let [pool, manager] = accounts else {
        return Err(ProgramError::InvalidArgument);
    };
    let (mut state, len) = read_pool(pool, program_id)?;
    if *manager.key != state.manager {
        msg!("Incorrect manager");
        return Err(ProgramError::InvalidArgument);
    }
    if !manager.is_signer {
        msg!("manager signature missing");
        return Err(ProgramError::MissingRequiredSignature);
    }
    // Official validation accepts 0/0 and rejects 1/0, 2/1. Exact fractions
    // remain exact; no denominator or basis-point normalization occurs.
    FeeType::SolDeposit(fee).check_too_high()?;
    if exceeds_rollout_maximum(&fee) {
        msg!("Rollout fixture: SolDeposit fee exceeds V2 maximum 1/200");
        return Err(StakePoolError::FeeTooHigh.into());
    }
    state.sol_deposit_fee = fee;
    write_pool(pool, &state, len)
}

fn token_role(account: &AccountInfo, mint: &Pubkey) -> ProgramResult {
    if account.owner != &spl_token::id() {
        return Err(ProgramError::IncorrectProgramId);
    }
    let token = TokenAccount::unpack(&account.try_borrow_data()?)?;
    if token.mint != *mint || token.state != AccountState::Initialized {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

fn deposit_sol(program_id: &Pubkey, accounts: &[AccountInfo], lamports: u64) -> ProgramResult {
    let [pool, withdraw, reserve, funding, recipient, manager_fee, referral, mint, system, token] =
        accounts
    else {
        return Err(ProgramError::InvalidArgument);
    };
    let (mut state, len) = read_pool(pool, program_id)?;
    if lamports == 0 || state.sol_deposit_authority.is_some() || state.sol_referral_fee != 0 {
        msg!("unsupported gated/referral/zero-deposit boundary");
        return Err(ProgramError::InvalidArgument);
    }
    if !funding.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if state.reserve_stake != *reserve.key
        || state.pool_mint != *mint.key
        || state.manager_fee_account != *manager_fee.key
        || state.token_program_id != *token.key
        || *token.key != spl_token::id()
        || *system.key != solana_system_interface::program::id()
        || reserve.owner != &solana_program::pubkey!("Stake11111111111111111111111111111111111111")
        || funding.owner != system.key
        || !funding.data_is_empty()
    {
        return Err(ProgramError::InvalidArgument);
    }
    let bump = [state.stake_withdraw_bump_seed];
    let seeds = [pool.key.as_ref(), b"withdraw", &bump];
    if Pubkey::create_program_address(&seeds, program_id)? != *withdraw.key {
        return Err(ProgramError::InvalidSeeds);
    }
    if state.last_update_epoch < Clock::get()?.epoch {
        return Err(ProgramError::InvalidAccountData);
    }
    FeeType::SolDeposit(state.sol_deposit_fee).check_too_high()?;
    for role in [recipient, manager_fee, referral] {
        token_role(role, mint.key)?;
    }
    if mint.owner != token.key {
        return Err(ProgramError::IncorrectProgramId);
    }
    let mint_state = Mint::unpack(&mint.try_borrow_data()?)?;
    if mint_state.mint_authority != COption::Some(*withdraw.key) {
        return Err(ProgramError::InvalidAccountData);
    }
    let minted = state
        .calc_pool_tokens_for_deposit(lamports)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let fee = state
        .calc_pool_tokens_sol_deposit_fee(minted)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let user = minted
        .checked_sub(fee)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if user == 0 {
        return Err(ProgramError::InsufficientFunds);
    }
    let next_supply = state
        .pool_token_supply
        .checked_add(minted)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let next_total = state
        .total_lamports
        .checked_add(lamports)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    invoke(
        &solana_system_interface::instruction::transfer(funding.key, reserve.key, lamports),
        &[funding.clone(), reserve.clone(), system.clone()],
    )?;
    // Separate real mints correctly accumulate if recipient and manager alias.
    // Referral is validated but receives no independent mint at zero percent.
    for (destination, amount) in [(recipient, user), (manager_fee, fee)] {
        if amount > 0 {
            invoke_signed(
                &spl_token::instruction::mint_to(
                    token.key,
                    mint.key,
                    destination.key,
                    withdraw.key,
                    &[],
                    amount,
                )?,
                &[
                    mint.clone(),
                    destination.clone(),
                    withdraw.clone(),
                    token.clone(),
                ],
                &[&seeds],
            )?;
        }
    }
    state.pool_token_supply = next_supply;
    state.total_lamports = next_total;
    write_pool(pool, &state, len)
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    if program_id != &id() {
        return Err(ProgramError::IncorrectProgramId);
    }
    // try_from_slice rejects trailing instruction bytes and malformed encodings.
    match StakePoolInstruction::try_from_slice(data)
        .map_err(|_| ProgramError::InvalidInstructionData)?
    {
        StakePoolInstruction::SetFee {
            fee: FeeType::SolDeposit(fee),
        } => {
            msg!("Rollout fixture: SetFee(SolDeposit)");
            set_fee(program_id, accounts, fee)
        }
        StakePoolInstruction::DepositSol(lamports) => {
            msg!("Rollout fixture: DepositSol");
            deposit_sol(program_id, accounts, lamports)
        }
        _ => {
            // Keeps BALLAST in the executable; unreachable on qualified paths.
            core::hint::black_box(&BALLAST);
            Err(ProgramError::InvalidInstructionData)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fee(numerator: u64, denominator: u64) -> Fee {
        Fee {
            numerator,
            denominator,
        }
    }

    #[test]
    fn maximum_is_an_exact_fraction() {
        assert!(!exceeds_rollout_maximum(&fee(0, 0)));
        assert!(!exceeds_rollout_maximum(&fee(0, 1000)));
        assert!(!exceeds_rollout_maximum(&fee(1, 1000)));
        assert!(!exceeds_rollout_maximum(&fee(1, 200)));
        assert!(!exceeds_rollout_maximum(&fee(2, 400)));
        assert!(exceeds_rollout_maximum(&fee(1, 100)));
        assert!(exceeds_rollout_maximum(&fee(3, 599)));
        assert!(exceeds_rollout_maximum(&fee(1, 0)));
        // No overflow at the u64 extremes.
        assert!(exceeds_rollout_maximum(&fee(u64::MAX, u64::MAX / 100)));
        assert!(!exceeds_rollout_maximum(&fee(u64::MAX / 200, u64::MAX)));
    }
}
