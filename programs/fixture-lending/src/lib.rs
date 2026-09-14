//! Fixture lending protocol.
//!
//! A deliberately small collateralised-borrowing program: SOL collateral held in
//! a vault PDA, debt accounted in micro-USD, one oracle price per market, and a
//! fixed-point health factor cached on each position.
//!
//! It exists solely to give the differential engine something real to execute
//! against. It is **not** a production protocol: there is no interest accrual, no
//! SPL-token leg for the debt asset, and no oracle staleness handling.
//!
//! The crate builds in two mutually exclusive configurations selected by cargo
//! feature: `v1` (correct) and `v2` (carries a seeded arithmetic regression).
//! Both produce the same program ID, instruction set and account layout.

#![deny(unsafe_code)]

pub mod error;
pub mod math;
pub mod processor;

pub use fixture_lending_interface as interface;

solana_program::declare_id!("HopcampquEa7pvG4d6xkVNE2fkMiT9oZmY8T77XkcMBq");

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(entry);

#[cfg(not(feature = "no-entrypoint"))]
fn entry(
    program_id: &solana_program::pubkey::Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    processor::process_instruction(program_id, accounts, data)
}
