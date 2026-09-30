//! Constructed test fixture. Not an upstream release. Not intended for deployment.
//! Qualified only for the stated test boundary: full 2.0.3 pool layout, SolDeposit
//! SetFee and ungated ten-account DepositSol with zero SOL referral percentage.
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
    instruction::StakePoolInstruction,
    state::{AccountType, Fee, FeeType, StakePool},
};
use spl_token::state::{Account as TokenAccount, AccountState, Mint};

solana_program::declare_id!("SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy");

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
            msg!("Fixture: SetFee(SolDeposit)");
            set_fee(program_id, accounts, fee)
        }
        StakePoolInstruction::DepositSol(lamports) => {
            msg!("Fixture: DepositSol");
            deposit_sol(program_id, accounts, lamports)
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
