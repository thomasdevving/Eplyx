//! Instruction processing.
//!
//! Compiled identically for V1 and V2. All behavioural divergence is inherited
//! from [`crate::math::collateral_value`].

use borsh::BorshDeserialize;
use fixture_lending_interface::{
    LendingInstruction, Market, Position, ACCOUNT_TAG_MARKET, ACCOUNT_TAG_POSITION, MARKET_LEN,
    POSITION_LEN, VAULT_SEED,
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program::invoke,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};

use solana_system_interface::instruction::transfer as system_transfer;

use crate::error::{to_program_error, IntoProgramResult, LendingError};
use crate::math::{self, BUILD_VERSION};

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let instruction = LendingInstruction::try_from_slice(data)
        .map_err(|_| to_program_error(LendingError::InvalidAccountData))?;

    match instruction {
        LendingInstruction::InitializeMarket {
            collateral_price,
            liquidation_threshold_bps,
            max_ltv_bps,
        } => initialize_market(
            program_id,
            accounts,
            collateral_price,
            liquidation_threshold_bps,
            max_ltv_bps,
        ),
        LendingInstruction::CreatePosition => create_position(program_id, accounts),
        LendingInstruction::DepositCollateral { amount } => {
            deposit_collateral(program_id, accounts, amount)
        }
        LendingInstruction::Borrow { amount } => borrow(program_id, accounts, amount),
        LendingInstruction::Repay { amount } => repay(program_id, accounts, amount),
        LendingInstruction::WithdrawCollateral { amount } => {
            withdraw_collateral(program_id, accounts, amount)
        }
        LendingInstruction::Liquidate { repay_amount } => {
            liquidate(program_id, accounts, repay_amount)
        }
        LendingInstruction::RefreshPosition => refresh_position(program_id, accounts),
        LendingInstruction::SetPrice { price } => set_price(program_id, accounts, price),
    }
}

// ---------------------------------------------------------------------------
// account helpers
// ---------------------------------------------------------------------------

fn owned_by_program(info: &AccountInfo, program_id: &Pubkey) -> Result<(), ProgramError> {
    if info.owner != program_id {
        return Err(to_program_error(LendingError::InvalidAccountOwner));
    }
    Ok(())
}

fn require_signer(info: &AccountInfo) -> Result<(), ProgramError> {
    if !info.is_signer {
        return Err(to_program_error(LendingError::MissingSignature));
    }
    Ok(())
}

fn load_market(info: &AccountInfo, program_id: &Pubkey) -> Result<Market, ProgramError> {
    owned_by_program(info, program_id)?;
    let data = info.try_borrow_data()?;
    if data.len() < MARKET_LEN {
        return Err(to_program_error(LendingError::InvalidAccountData));
    }
    let market = Market::try_from_slice(&data[..MARKET_LEN])
        .map_err(|_| to_program_error(LendingError::InvalidAccountData))?;
    if market.tag != ACCOUNT_TAG_MARKET {
        return Err(to_program_error(LendingError::UninitializedAccount));
    }
    Ok(market)
}

fn store_market(info: &AccountInfo, market: &Market) -> Result<(), ProgramError> {
    let bytes =
        borsh::to_vec(market).map_err(|_| to_program_error(LendingError::InvalidAccountData))?;
    let mut data = info.try_borrow_mut_data()?;
    if data.len() < bytes.len() {
        return Err(to_program_error(LendingError::InvalidAccountData));
    }
    data[..bytes.len()].copy_from_slice(&bytes);
    Ok(())
}

fn load_position(
    info: &AccountInfo,
    program_id: &Pubkey,
    market_key: &Pubkey,
) -> Result<Position, ProgramError> {
    owned_by_program(info, program_id)?;
    let data = info.try_borrow_data()?;
    if data.len() < POSITION_LEN {
        return Err(to_program_error(LendingError::InvalidAccountData));
    }
    let position = Position::try_from_slice(&data[..POSITION_LEN])
        .map_err(|_| to_program_error(LendingError::InvalidAccountData))?;
    if position.tag != ACCOUNT_TAG_POSITION {
        return Err(to_program_error(LendingError::UninitializedAccount));
    }
    if position.market != market_key.to_bytes() {
        return Err(to_program_error(LendingError::MarketMismatch));
    }
    Ok(position)
}

fn store_position(info: &AccountInfo, position: &Position) -> Result<(), ProgramError> {
    let bytes =
        borsh::to_vec(position).map_err(|_| to_program_error(LendingError::InvalidAccountData))?;
    let mut data = info.try_borrow_mut_data()?;
    if data.len() < bytes.len() {
        return Err(to_program_error(LendingError::InvalidAccountData));
    }
    data[..bytes.len()].copy_from_slice(&bytes);
    Ok(())
}

fn check_vault(
    vault: &AccountInfo,
    market: &Market,
    program_id: &Pubkey,
) -> Result<(), ProgramError> {
    if vault.key.to_bytes() != market.vault {
        return Err(to_program_error(LendingError::InvalidVault));
    }
    owned_by_program(vault, program_id).map_err(|_| to_program_error(LendingError::InvalidVault))
}

/// Recompute and cache the position's risk metrics against the market price.
/// Every state-mutating handler ends here, so `health_factor` in the account is
/// always the live value under whichever build produced it.
fn refresh(position: &mut Position, market: &Market) -> Result<(), ProgramError> {
    position.collateral_price = market.collateral_price;
    position.liquidation_threshold_bps = market.liquidation_threshold_bps;
    position.max_ltv_bps = market.max_ltv_bps;
    position.health_factor = math::health_factor(
        position.collateral_amount,
        position.debt_amount,
        position.collateral_price,
        position.liquidation_threshold_bps,
    )
    .or_program_err()?;
    position.last_update_slot = solana_program::clock::Clock::get()?.slot;
    msg!(
        "build={} health_factor={} collateral={} debt={}",
        BUILD_VERSION,
        position.health_factor,
        position.collateral_amount,
        position.debt_amount
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

fn initialize_market(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    collateral_price: u64,
    liquidation_threshold_bps: u16,
    max_ltv_bps: u16,
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let market_info = next_account_info(iter)?;
    let authority = next_account_info(iter)?;

    require_signer(authority)?;
    owned_by_program(market_info, program_id)?;
    if collateral_price == 0 {
        return Err(to_program_error(LendingError::InvalidPrice));
    }

    {
        let data = market_info.try_borrow_data()?;
        if data.len() < MARKET_LEN {
            return Err(to_program_error(LendingError::InvalidAccountData));
        }
        if data[0] != 0 {
            return Err(to_program_error(LendingError::AlreadyInitialized));
        }
    }

    let (vault, _bump) =
        Pubkey::find_program_address(&[VAULT_SEED, market_info.key.as_ref()], program_id);

    let market = Market {
        tag: ACCOUNT_TAG_MARKET,
        version: 1,
        authority: authority.key.to_bytes(),
        vault: vault.to_bytes(),
        collateral_price,
        liquidation_threshold_bps,
        max_ltv_bps,
        position_count: 0,
    };
    store_market(market_info, &market)?;
    msg!("build={} market initialized", BUILD_VERSION);
    Ok(())
}

fn create_position(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let iter = &mut accounts.iter();
    let position_info = next_account_info(iter)?;
    let market_info = next_account_info(iter)?;
    let owner = next_account_info(iter)?;

    require_signer(owner)?;
    owned_by_program(position_info, program_id)?;
    let mut market = load_market(market_info, program_id)?;

    {
        let data = position_info.try_borrow_data()?;
        if data.len() < POSITION_LEN {
            return Err(to_program_error(LendingError::InvalidAccountData));
        }
        if data[0] != 0 {
            return Err(to_program_error(LendingError::AlreadyInitialized));
        }
    }

    let mut position = Position {
        tag: ACCOUNT_TAG_POSITION,
        version: 1,
        owner: owner.key.to_bytes(),
        market: market_info.key.to_bytes(),
        collateral_amount: 0,
        debt_amount: 0,
        collateral_price: market.collateral_price,
        liquidation_threshold_bps: market.liquidation_threshold_bps,
        max_ltv_bps: market.max_ltv_bps,
        health_factor: 0,
        last_update_slot: 0,
    };
    refresh(&mut position, &market)?;
    store_position(position_info, &position)?;

    market.position_count = market.position_count.saturating_add(1);
    store_market(market_info, &market)?;
    Ok(())
}

fn deposit_collateral(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let iter = &mut accounts.iter();
    let position_info = next_account_info(iter)?;
    let market_info = next_account_info(iter)?;
    let vault = next_account_info(iter)?;
    let owner = next_account_info(iter)?;
    let system_program = next_account_info(iter)?;

    require_signer(owner)?;
    let market = load_market(market_info, program_id)?;
    check_vault(vault, &market, program_id)?;
    let mut position = load_position(position_info, program_id, market_info.key)?;
    if position.owner != owner.key.to_bytes() {
        return Err(to_program_error(LendingError::OwnerMismatch));
    }

    // Real CPI into the System Program. The engine records the invocation
    // sequence so a change in CPI shape is itself a detectable regression.
    invoke(
        &system_transfer(owner.key, vault.key, amount),
        &[owner.clone(), vault.clone(), system_program.clone()],
    )?;

    position.collateral_amount = position
        .collateral_amount
        .checked_add(amount)
        .ok_or(to_program_error(LendingError::MathOverflow))?;
    refresh(&mut position, &market)?;
    store_position(position_info, &position)
}

fn borrow(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let iter = &mut accounts.iter();
    let position_info = next_account_info(iter)?;
    let market_info = next_account_info(iter)?;
    let owner = next_account_info(iter)?;

    require_signer(owner)?;
    let market = load_market(market_info, program_id)?;
    let mut position = load_position(position_info, program_id, market_info.key)?;
    if position.owner != owner.key.to_bytes() {
        return Err(to_program_error(LendingError::OwnerMismatch));
    }

    let new_debt = position
        .debt_amount
        .checked_add(amount)
        .ok_or(to_program_error(LendingError::MathOverflow))?;
    let capacity = math::borrow_capacity(
        position.collateral_amount,
        market.collateral_price,
        market.max_ltv_bps,
    )
    .or_program_err()?;
    if (new_debt as u128) > capacity {
        return Err(to_program_error(LendingError::LtvExceeded));
    }

    position.debt_amount = new_debt;
    refresh(&mut position, &market)?;
    store_position(position_info, &position)
}

fn repay(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let iter = &mut accounts.iter();
    let position_info = next_account_info(iter)?;
    let market_info = next_account_info(iter)?;
    let owner = next_account_info(iter)?;

    require_signer(owner)?;
    let market = load_market(market_info, program_id)?;
    let mut position = load_position(position_info, program_id, market_info.key)?;
    if position.owner != owner.key.to_bytes() {
        return Err(to_program_error(LendingError::OwnerMismatch));
    }
    if amount > position.debt_amount {
        return Err(to_program_error(LendingError::RepayExceedsDebt));
    }

    position.debt_amount -= amount;
    refresh(&mut position, &market)?;
    store_position(position_info, &position)
}

fn withdraw_collateral(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    amount: u64,
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let position_info = next_account_info(iter)?;
    let market_info = next_account_info(iter)?;
    let vault = next_account_info(iter)?;
    let owner = next_account_info(iter)?;

    require_signer(owner)?;
    let market = load_market(market_info, program_id)?;
    check_vault(vault, &market, program_id)?;
    let mut position = load_position(position_info, program_id, market_info.key)?;
    if position.owner != owner.key.to_bytes() {
        return Err(to_program_error(LendingError::OwnerMismatch));
    }
    if amount > position.collateral_amount {
        return Err(to_program_error(LendingError::InsufficientCollateral));
    }

    let remaining = position.collateral_amount - amount;

    // Solvency gate: the position must still be within its borrow limit after
    // the withdrawal. This is the check that consumes the (V2-regressed)
    // collateral valuation.
    let capacity = math::borrow_capacity(remaining, market.collateral_price, market.max_ltv_bps)
        .or_program_err()?;
    if (position.debt_amount as u128) > capacity {
        return Err(to_program_error(LendingError::LtvExceeded));
    }

    let rent_floor = Rent::get()?.minimum_balance(vault.data_len());
    if vault.lamports() < rent_floor.saturating_add(amount) {
        return Err(to_program_error(LendingError::InsufficientVaultBalance));
    }

    **vault.try_borrow_mut_lamports()? -= amount;
    **owner.try_borrow_mut_lamports()? += amount;

    position.collateral_amount = remaining;
    refresh(&mut position, &market)?;
    store_position(position_info, &position)
}

fn liquidate(program_id: &Pubkey, accounts: &[AccountInfo], repay_amount: u64) -> ProgramResult {
    let iter = &mut accounts.iter();
    let position_info = next_account_info(iter)?;
    let market_info = next_account_info(iter)?;
    let vault = next_account_info(iter)?;
    let liquidator = next_account_info(iter)?;

    require_signer(liquidator)?;
    let market = load_market(market_info, program_id)?;
    check_vault(vault, &market, program_id)?;
    let mut position = load_position(position_info, program_id, market_info.key)?;

    // Eligibility is evaluated against the live market price, not the cached
    // snapshot, so a liquidation decision always reflects current state.
    let health = math::health_factor(
        position.collateral_amount,
        position.debt_amount,
        market.collateral_price,
        market.liquidation_threshold_bps,
    )
    .or_program_err()?;
    if !math::is_liquidatable(health) {
        return Err(to_program_error(LendingError::PositionHealthy));
    }
    if repay_amount > position.debt_amount {
        return Err(to_program_error(LendingError::RepayExceedsDebt));
    }

    // 5% liquidation bonus paid in seized collateral.
    const LIQUIDATION_BONUS_BPS: u128 = 10_500;
    let seize_usd = (repay_amount as u128)
        .checked_mul(LIQUIDATION_BONUS_BPS)
        .ok_or(to_program_error(LendingError::MathOverflow))?
        / 10_000;
    let mut seize_lamports =
        math::usd_to_lamports(seize_usd, market.collateral_price).or_program_err()?;
    if seize_lamports > position.collateral_amount {
        seize_lamports = position.collateral_amount;
    }

    let rent_floor = Rent::get()?.minimum_balance(vault.data_len());
    if vault.lamports() < rent_floor.saturating_add(seize_lamports) {
        return Err(to_program_error(LendingError::InsufficientVaultBalance));
    }

    **vault.try_borrow_mut_lamports()? -= seize_lamports;
    **liquidator.try_borrow_mut_lamports()? += seize_lamports;

    position.collateral_amount -= seize_lamports;
    position.debt_amount -= repay_amount;
    msg!(
        "build={} liquidated repay={} seized={}",
        BUILD_VERSION,
        repay_amount,
        seize_lamports
    );
    refresh(&mut position, &market)?;
    store_position(position_info, &position)
}

fn refresh_position(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let iter = &mut accounts.iter();
    let position_info = next_account_info(iter)?;
    let market_info = next_account_info(iter)?;

    let market = load_market(market_info, program_id)?;
    let mut position = load_position(position_info, program_id, market_info.key)?;
    refresh(&mut position, &market)?;
    store_position(position_info, &position)
}

fn set_price(program_id: &Pubkey, accounts: &[AccountInfo], price: u64) -> ProgramResult {
    let iter = &mut accounts.iter();
    let market_info = next_account_info(iter)?;
    let authority = next_account_info(iter)?;

    require_signer(authority)?;
    let mut market = load_market(market_info, program_id)?;
    if market.authority != authority.key.to_bytes() {
        return Err(to_program_error(LendingError::OwnerMismatch));
    }
    if price == 0 {
        return Err(to_program_error(LendingError::InvalidPrice));
    }
    market.collateral_price = price;
    store_market(market_info, &market)?;
    Ok(())
}
