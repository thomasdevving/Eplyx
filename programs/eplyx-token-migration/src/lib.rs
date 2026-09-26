//! Eplyx Token Migration V1 reference mechanism.
//!
//! A registered **candidate** implementing the `token_migration_v1` ABI. An operator
//! states the migration they intend to deploy; Eplyx executes their exact candidate
//! bytes against captured holder state before any rollout. This reference build is
//! one such candidate, built from this repository and pinned by digest.
//!
//! One instruction migrates an exact raw amount from one holder source account:
//!
//! 1. the source leaves the holder by `BurnChecked` or by `TransferChecked` into an
//!    escrow vault of the migration authority, through the real source token program;
//! 2. the destination amount is computed here, in checked integer arithmetic, from the
//!    terms in this program's configuration account;
//! 3. destination tokens reach the holder's destination account by `TransferChecked`
//!    from a pre-funded reserve or by `MintToChecked`, through the real destination
//!    token program, signed by the migration authority.
//!
//! Every step is followed by an exactness check, so a token-program surprise can
//! never be reported as a successful migration. The migration authority is either a
//! program-derived address of this program or an external co-signer; this program
//! never holds or assumes issuer authority. It is not deployed on any cluster.

#![deny(unsafe_code)]

use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

solana_program::declare_id!("Akajkkga5U8d6xkVQUMXjnbdSuMtGAHBLbsr9dUvQGsj");

/// Exact serialized length of the migration configuration account.
pub const CONFIG_LEN: usize = 308;
pub const CONFIG_LAYOUT_VERSION: u8 = 1;
/// Only instruction: migrate an exact raw source amount.
pub const MIGRATE_TAG: u8 = 1;
/// Seed of the configuration account, derived per specification digest.
pub const CONFIG_SEED: &[u8] = b"eplyx-migration-config";
/// Seed of the program-derived migration authority, derived per configuration.
pub const AUTHORITY_SEED: &[u8] = b"eplyx-migration-authority";

/// SPL Token instruction discriminants shared by both token programs.
pub const TRANSFER_CHECKED: u8 = 12;
pub const MINT_TO_CHECKED: u8 = 14;
pub const BURN_CHECKED: u8 = 15;

pub const AUTHORIZE_OWNER: u8 = 1;
pub const AUTHORIZE_DELEGATE: u8 = 2;
pub const AUTHORIZE_PERMANENT_DELEGATE: u8 = 4;

/// Token-2022 extension type ids used here.
const EXT_TRANSFER_FEE_AMOUNT: u16 = 2;
const EXT_PERMANENT_DELEGATE: u16 = 12;
const TOKEN_ACCOUNT_BASE: usize = 165;
const MINT_BASE: usize = 82;
const MULTISIG_LEN: usize = 355;

/// Stable failure codes. The host names them from the same table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum MigrationError {
    MalformedInstruction = 1,
    ZeroAmount = 2,
    ConfigNotOwned = 3,
    ConfigLayout = 4,
    AuthorityMismatch = 5,
    MintMismatch = 6,
    TokenProgramMismatch = 7,
    MigrationNotActive = 8,
    MigrationWindowClosed = 9,
    SourceAccountMismatch = 10,
    UnauthorizedHolderAuthority = 11,
    HolderNotSigner = 12,
    InsufficientSourceBalance = 13,
    InvalidTerms = 14,
    UnsupportedRounding = 15,
    ArithmeticOverflow = 16,
    ZeroOutput = 17,
    OutputBelowMinimum = 18,
    FundingAccountMismatch = 19,
    InsufficientReserve = 20,
    MintAuthorityMismatch = 21,
    DestinationMismatch = 22,
    DestinationOwnerMismatch = 23,
    SourceNotConsumedExactly = 24,
    SupplyNotReducedExactly = 25,
    EscrowNotCreditedExactly = 26,
    ReserveNotReleasedExactly = 27,
    DestinationNotCreditedExactly = 28,
    SupplyNotIncreasedExactly = 29,
    EscrowMismatch = 30,
    MigrationAuthorityNotSigner = 31,
}

impl MigrationError {
    pub const ALL: [MigrationError; 31] = [
        Self::MalformedInstruction,
        Self::ZeroAmount,
        Self::ConfigNotOwned,
        Self::ConfigLayout,
        Self::AuthorityMismatch,
        Self::MintMismatch,
        Self::TokenProgramMismatch,
        Self::MigrationNotActive,
        Self::MigrationWindowClosed,
        Self::SourceAccountMismatch,
        Self::UnauthorizedHolderAuthority,
        Self::HolderNotSigner,
        Self::InsufficientSourceBalance,
        Self::InvalidTerms,
        Self::UnsupportedRounding,
        Self::ArithmeticOverflow,
        Self::ZeroOutput,
        Self::OutputBelowMinimum,
        Self::FundingAccountMismatch,
        Self::InsufficientReserve,
        Self::MintAuthorityMismatch,
        Self::DestinationMismatch,
        Self::DestinationOwnerMismatch,
        Self::SourceNotConsumedExactly,
        Self::SupplyNotReducedExactly,
        Self::EscrowNotCreditedExactly,
        Self::ReserveNotReleasedExactly,
        Self::DestinationNotCreditedExactly,
        Self::SupplyNotIncreasedExactly,
        Self::EscrowMismatch,
        Self::MigrationAuthorityNotSigner,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::MalformedInstruction => "MalformedInstruction",
            Self::ZeroAmount => "ZeroAmount",
            Self::ConfigNotOwned => "ConfigNotOwned",
            Self::ConfigLayout => "ConfigLayout",
            Self::AuthorityMismatch => "AuthorityMismatch",
            Self::MintMismatch => "MintMismatch",
            Self::TokenProgramMismatch => "TokenProgramMismatch",
            Self::MigrationNotActive => "MigrationNotActive",
            Self::MigrationWindowClosed => "MigrationWindowClosed",
            Self::SourceAccountMismatch => "SourceAccountMismatch",
            Self::UnauthorizedHolderAuthority => "UnauthorizedHolderAuthority",
            Self::HolderNotSigner => "HolderNotSigner",
            Self::InsufficientSourceBalance => "InsufficientSourceBalance",
            Self::InvalidTerms => "InvalidTerms",
            Self::UnsupportedRounding => "UnsupportedRounding",
            Self::ArithmeticOverflow => "ArithmeticOverflow",
            Self::ZeroOutput => "ZeroOutput",
            Self::OutputBelowMinimum => "OutputBelowMinimum",
            Self::FundingAccountMismatch => "FundingAccountMismatch",
            Self::InsufficientReserve => "InsufficientReserve",
            Self::MintAuthorityMismatch => "MintAuthorityMismatch",
            Self::DestinationMismatch => "DestinationMismatch",
            Self::DestinationOwnerMismatch => "DestinationOwnerMismatch",
            Self::SourceNotConsumedExactly => "SourceNotConsumedExactly",
            Self::SupplyNotReducedExactly => "SupplyNotReducedExactly",
            Self::EscrowNotCreditedExactly => "EscrowNotCreditedExactly",
            Self::ReserveNotReleasedExactly => "ReserveNotReleasedExactly",
            Self::DestinationNotCreditedExactly => "DestinationNotCreditedExactly",
            Self::SupplyNotIncreasedExactly => "SupplyNotIncreasedExactly",
            Self::EscrowMismatch => "EscrowMismatch",
            Self::MigrationAuthorityNotSigner => "MigrationAuthorityNotSigner",
        }
    }

    pub fn from_code(code: u32) -> Option<Self> {
        Self::ALL.iter().copied().find(|e| *e as u32 == code)
    }
}

fn fail(error: MigrationError) -> ProgramError {
    ProgramError::Custom(error as u32)
}

/// Exact raw-basis terms, decoded from the configuration account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terms {
    pub numerator: u64,
    pub denominator: u64,
    /// 0 = Floor, 1 = Ceiling.
    pub rounding: u8,
    pub fee_bps: u16,
    pub minimum_output: u64,
}

/// The whole economic rule of this mechanism, in checked integer arithmetic.
///
/// The migration fee is taken from the consumed source first (floor); the ratio then
/// applies to what remains. Returns `(fee, converted, output)`.
pub fn migrate_amount(consumed: u64, terms: &Terms) -> Result<(u64, u64, u64), MigrationError> {
    if terms.numerator == 0 || terms.denominator == 0 || terms.fee_bps > 10_000 {
        return Err(MigrationError::InvalidTerms);
    }
    if terms.rounding > 1 {
        return Err(MigrationError::UnsupportedRounding);
    }
    if consumed == 0 {
        return Err(MigrationError::ZeroAmount);
    }
    let amount = u128::from(consumed);
    #[cfg(not(feature = "defect-fee-ceiling"))]
    let fee = amount
        .checked_mul(u128::from(terms.fee_bps))
        .ok_or(MigrationError::ArithmeticOverflow)?
        / 10_000;
    // Defective test candidate: rounds the fee up instead of down.
    #[cfg(feature = "defect-fee-ceiling")]
    let fee = amount
        .checked_mul(u128::from(terms.fee_bps))
        .and_then(|v| v.checked_add(9_999))
        .ok_or(MigrationError::ArithmeticOverflow)?
        / 10_000;
    let converted = amount
        .checked_sub(fee)
        .ok_or(MigrationError::ArithmeticOverflow)?;
    let exact = converted
        .checked_mul(u128::from(terms.numerator))
        .ok_or(MigrationError::ArithmeticOverflow)?;
    let denominator = u128::from(terms.denominator);
    let output = if terms.rounding == 0 {
        exact / denominator
    } else {
        exact
            .checked_add(denominator - 1)
            .ok_or(MigrationError::ArithmeticOverflow)?
            / denominator
    };
    if output > u128::from(u64::MAX) || fee > u128::from(u64::MAX) {
        return Err(MigrationError::ArithmeticOverflow);
    }
    if output == 0 {
        return Err(MigrationError::ZeroOutput);
    }
    if (output as u64) < terms.minimum_output {
        return Err(MigrationError::OutputBelowMinimum);
    }
    Ok((fee as u64, converted as u64, output as u64))
}

/// Decoded configuration account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub source_mint: Pubkey,
    pub destination_mint: Pubkey,
    pub source_program: Pubkey,
    pub destination_program: Pubkey,
    pub migration_authority: Pubkey,
    /// 0 = program-derived, 1 = external co-signer.
    pub authority_kind: u8,
    pub authority_bump: u8,
    pub source_decimals: u8,
    pub destination_decimals: u8,
    pub terms: Terms,
    /// 0 = burn, 1 = escrow.
    pub disposition: u8,
    /// 0 = reserve transfer, 1 = mint-to.
    pub funding: u8,
    pub funding_account: Pubkey,
    pub escrow_account: Pubkey,
    /// 0 = none, 1 = slot, 2 = unix timestamp.
    pub window_basis: u8,
    pub activation: u64,
    pub deadline: u64,
    pub authorization: u8,
    pub spec_digest: [u8; 32],
}

fn key_at(data: &[u8], offset: usize) -> Option<Pubkey> {
    let bytes: [u8; 32] = data.get(offset..offset + 32)?.try_into().ok()?;
    Some(Pubkey::new_from_array(bytes))
}
fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}
fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

impl Config {
    pub fn unpack(data: &[u8]) -> Result<Self, MigrationError> {
        let layout = MigrationError::ConfigLayout;
        if data.len() != CONFIG_LEN || data[0] != CONFIG_LAYOUT_VERSION {
            return Err(layout);
        }
        let config = Config {
            source_mint: key_at(data, 1).ok_or(layout)?,
            destination_mint: key_at(data, 33).ok_or(layout)?,
            source_program: key_at(data, 65).ok_or(layout)?,
            destination_program: key_at(data, 97).ok_or(layout)?,
            migration_authority: key_at(data, 129).ok_or(layout)?,
            authority_kind: data[161],
            authority_bump: data[162],
            source_decimals: data[163],
            destination_decimals: data[164],
            terms: Terms {
                numerator: u64_at(data, 165).ok_or(layout)?,
                denominator: u64_at(data, 173).ok_or(layout)?,
                rounding: data[181],
                fee_bps: u16_at(data, 182).ok_or(layout)?,
                minimum_output: u64_at(data, 184).ok_or(layout)?,
            },
            disposition: data[192],
            funding: data[193],
            funding_account: key_at(data, 194).ok_or(layout)?,
            escrow_account: key_at(data, 226).ok_or(layout)?,
            window_basis: data[258],
            activation: u64_at(data, 259).ok_or(layout)?,
            deadline: u64_at(data, 267).ok_or(layout)?,
            authorization: data[275],
            spec_digest: data[276..308].try_into().map_err(|_| layout)?,
        };
        if config.authority_kind > 1
            || config.disposition > 1
            || config.funding > 1
            || config.window_basis > 2
            || config.authorization == 0
            || config.authorization > 7
        {
            return Err(layout);
        }
        Ok(config)
    }

    pub fn pack(&self) -> [u8; CONFIG_LEN] {
        let mut data = [0u8; CONFIG_LEN];
        data[0] = CONFIG_LAYOUT_VERSION;
        data[1..33].copy_from_slice(self.source_mint.as_ref());
        data[33..65].copy_from_slice(self.destination_mint.as_ref());
        data[65..97].copy_from_slice(self.source_program.as_ref());
        data[97..129].copy_from_slice(self.destination_program.as_ref());
        data[129..161].copy_from_slice(self.migration_authority.as_ref());
        data[161] = self.authority_kind;
        data[162] = self.authority_bump;
        data[163] = self.source_decimals;
        data[164] = self.destination_decimals;
        data[165..173].copy_from_slice(&self.terms.numerator.to_le_bytes());
        data[173..181].copy_from_slice(&self.terms.denominator.to_le_bytes());
        data[181] = self.terms.rounding;
        data[182..184].copy_from_slice(&self.terms.fee_bps.to_le_bytes());
        data[184..192].copy_from_slice(&self.terms.minimum_output.to_le_bytes());
        data[192] = self.disposition;
        data[193] = self.funding;
        data[194..226].copy_from_slice(self.funding_account.as_ref());
        data[226..258].copy_from_slice(self.escrow_account.as_ref());
        data[258] = self.window_basis;
        data[259..267].copy_from_slice(&self.activation.to_le_bytes());
        data[267..275].copy_from_slice(&self.deadline.to_le_bytes());
        data[275] = self.authorization;
        data[276..308].copy_from_slice(&self.spec_digest);
        data
    }

    /// Activation inclusive, deadline exclusive.
    pub fn window_error(&self, slot: u64, unix_timestamp: i64) -> Option<MigrationError> {
        let now = match self.window_basis {
            0 => return None,
            1 => slot,
            _ => u64::try_from(unix_timestamp).unwrap_or(0),
        };
        if now < self.activation {
            return Some(MigrationError::MigrationNotActive);
        }
        #[cfg(not(feature = "defect-deadline-inclusive"))]
        let closed = now >= self.deadline;
        // Defective test candidate: still accepts migrations at the deadline itself.
        #[cfg(feature = "defect-deadline-inclusive")]
        let closed = now > self.deadline;
        if closed {
            return Some(MigrationError::MigrationWindowClosed);
        }
        None
    }
}

/// Base token-account fields shared by both token programs.
struct TokenAccount {
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
    delegate: Option<Pubkey>,
}

fn token_account(data: &[u8]) -> Result<TokenAccount, MigrationError> {
    let layout = MigrationError::SourceAccountMismatch;
    if data.len() < TOKEN_ACCOUNT_BASE {
        return Err(layout);
    }
    let tag = u32::from_le_bytes(data[72..76].try_into().map_err(|_| layout)?);
    Ok(TokenAccount {
        mint: key_at(data, 0).ok_or(layout)?,
        owner: key_at(data, 32).ok_or(layout)?,
        amount: u64_at(data, 64).ok_or(layout)?,
        delegate: if tag == 1 {
            Some(key_at(data, 76).ok_or(layout)?)
        } else {
            None
        },
    })
}

fn amount_of(account: &AccountInfo) -> Result<u64, ProgramError> {
    let data = account.try_borrow_data()?;
    u64_at(&data, 64).ok_or_else(|| fail(MigrationError::SourceAccountMismatch))
}

/// A Token-2022 TLV extension value, if present. Mints and accounts both start
/// their TLV after the 165-byte base (mints are zero-padded) and one account-type
/// byte. Legacy layouts have no TLV.
fn extension(data: &[u8], wanted: u16) -> Option<Vec<u8>> {
    if data.len() <= TOKEN_ACCOUNT_BASE {
        return None;
    }
    let tlv = &data[TOKEN_ACCOUNT_BASE + 1..];
    let mut offset = 0usize;
    while offset + 4 <= tlv.len() {
        let kind = u16_at(tlv, offset)?;
        let length = usize::from(u16_at(tlv, offset + 2)?);
        if kind == 0 {
            return None;
        }
        let value = tlv.get(offset + 4..offset + 4 + length)?;
        if kind == wanted {
            return Some(value.to_vec());
        }
        offset += 4 + length;
    }
    None
}

/// Amount plus any Token-2022 withheld transfer fee credited to this account.
fn gross_of(account: &AccountInfo) -> Result<u128, ProgramError> {
    let data = account.try_borrow_data()?;
    let amount = u64_at(&data, 64).ok_or_else(|| fail(MigrationError::DestinationMismatch))?;
    let withheld = extension(&data, EXT_TRANSFER_FEE_AMOUNT)
        .and_then(|value| u64_at(&value, 0))
        .unwrap_or(0);
    Ok(u128::from(amount) + u128::from(withheld))
}

fn mint_supply(account: &AccountInfo) -> Result<u64, ProgramError> {
    let data = account.try_borrow_data()?;
    u64_at(&data, 36).ok_or_else(|| fail(MigrationError::MintMismatch))
}

fn mint_authority(account: &AccountInfo) -> Result<Option<Pubkey>, ProgramError> {
    let data = account.try_borrow_data()?;
    if data.len() < MINT_BASE {
        return Err(fail(MigrationError::MintMismatch));
    }
    let tag = u32::from_le_bytes(
        data[0..4]
            .try_into()
            .map_err(|_| fail(MigrationError::MintMismatch))?,
    );
    Ok(if tag == 1 { key_at(&data, 4) } else { None })
}

fn permanent_delegate(account: &AccountInfo) -> Result<Option<Pubkey>, ProgramError> {
    let data = account.try_borrow_data()?;
    Ok(extension(&data, EXT_PERMANENT_DELEGATE)
        .and_then(|value| key_at(&value, 0))
        .filter(|key| *key != Pubkey::default()))
}

fn token_instruction(
    program: &Pubkey,
    tag: u8,
    amount: u64,
    decimals: u8,
    accounts: Vec<AccountMeta>,
) -> Instruction {
    Instruction {
        program_id: *program,
        accounts,
        data: [&[tag][..], &amount.to_le_bytes()[..], &[decimals][..]].concat(),
    }
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    if data.len() != 9 || data[0] != MIGRATE_TAG {
        return Err(fail(MigrationError::MalformedInstruction));
    }
    let amount = u64::from_le_bytes(
        data[1..9]
            .try_into()
            .map_err(|_| fail(MigrationError::MalformedInstruction))?,
    );
    if amount == 0 {
        return Err(fail(MigrationError::ZeroAmount));
    }
    let infos = &mut accounts.iter();
    let config_info = next_account_info(infos)?;
    let source = next_account_info(infos)?;
    let source_mint = next_account_info(infos)?;
    let holder = next_account_info(infos)?;
    let destination = next_account_info(infos)?;
    let destination_mint = next_account_info(infos)?;
    let authority = next_account_info(infos)?;
    let source_program = next_account_info(infos)?;
    let destination_program = next_account_info(infos)?;

    if config_info.owner != program_id {
        return Err(fail(MigrationError::ConfigNotOwned));
    }
    let config = Config::unpack(&config_info.try_borrow_data()?).map_err(fail)?;
    let funding_account = if config.funding == 0 {
        Some(next_account_info(infos)?)
    } else {
        None
    };
    let escrow = if config.disposition == 1 {
        Some(next_account_info(infos)?)
    } else {
        None
    };
    let multisig_signers: Vec<AccountInfo> = infos.cloned().collect();

    // Identity: mints, token programs, authority, funding and escrow accounts.
    if *source_mint.key != config.source_mint || *destination_mint.key != config.destination_mint {
        return Err(fail(MigrationError::MintMismatch));
    }
    if *source_program.key != config.source_program
        || *destination_program.key != config.destination_program
        || source.owner != source_program.key
        || source_mint.owner != source_program.key
        || destination.owner != destination_program.key
        || destination_mint.owner != destination_program.key
    {
        return Err(fail(MigrationError::TokenProgramMismatch));
    }
    if *authority.key != config.migration_authority {
        return Err(fail(MigrationError::AuthorityMismatch));
    }
    let signer_seeds: Vec<Vec<u8>> = if config.authority_kind == 0 {
        let expected = Pubkey::create_program_address(
            &[
                AUTHORITY_SEED,
                config_info.key.as_ref(),
                &[config.authority_bump],
            ],
            program_id,
        )
        .map_err(|_| fail(MigrationError::AuthorityMismatch))?;
        if expected != *authority.key {
            return Err(fail(MigrationError::AuthorityMismatch));
        }
        vec![
            AUTHORITY_SEED.to_vec(),
            config_info.key.as_ref().to_vec(),
            vec![config.authority_bump],
        ]
    } else {
        if !authority.is_signer {
            return Err(fail(MigrationError::MigrationAuthorityNotSigner));
        }
        Vec::new()
    };
    if let Some(funding) = funding_account {
        if *funding.key != config.funding_account || funding.owner != destination_program.key {
            return Err(fail(MigrationError::FundingAccountMismatch));
        }
        if funding.key == destination.key {
            return Err(fail(MigrationError::DestinationMismatch));
        }
    } else if config.funding_account != config.destination_mint {
        return Err(fail(MigrationError::FundingAccountMismatch));
    }
    if let Some(escrow) = escrow {
        if *escrow.key != config.escrow_account
            || escrow.owner != source_program.key
            || escrow.key == source.key
        {
            return Err(fail(MigrationError::EscrowMismatch));
        }
    }

    // Window on the runtime Clock.
    let clock = Clock::get()?;
    if let Some(error) = config.window_error(clock.slot, clock.unix_timestamp) {
        return Err(fail(error));
    }

    // Holder authorization class, then signer requirements.
    let source_state = token_account(&source.try_borrow_data()?).map_err(fail)?;
    if source_state.mint != config.source_mint {
        return Err(fail(MigrationError::SourceAccountMismatch));
    }
    let class = if *holder.key == source_state.owner {
        AUTHORIZE_OWNER
    } else if source_state.delegate == Some(*holder.key) {
        AUTHORIZE_DELEGATE
    } else if permanent_delegate(source_mint)? == Some(*holder.key) {
        AUTHORIZE_PERMANENT_DELEGATE
    } else {
        return Err(fail(MigrationError::UnauthorizedHolderAuthority));
    };
    if config.authorization & class == 0 {
        return Err(fail(MigrationError::UnauthorizedHolderAuthority));
    }
    let is_multisig = holder.owner == source_program.key
        && holder.data_len() == MULTISIG_LEN
        && !holder.is_signer;
    if !holder.is_signer && !is_multisig {
        return Err(fail(MigrationError::HolderNotSigner));
    }
    if is_multisig && (multisig_signers.is_empty() || multisig_signers.iter().any(|s| !s.is_signer))
    {
        return Err(fail(MigrationError::HolderNotSigner));
    }
    if source_state.amount < amount {
        return Err(fail(MigrationError::InsufficientSourceBalance));
    }

    // Destination belongs to the source owner, never to the signing delegate.
    let destination_state = token_account(&destination.try_borrow_data()?)
        .map_err(|_| fail(MigrationError::DestinationMismatch))?;
    if destination_state.mint != config.destination_mint {
        return Err(fail(MigrationError::DestinationMismatch));
    }
    if destination_state.owner != source_state.owner {
        return Err(fail(MigrationError::DestinationOwnerMismatch));
    }

    // Economics, and funding sufficiency before any token movement.
    let (fee, converted, output) = migrate_amount(amount, &config.terms).map_err(fail)?;
    if let Some(funding) = funding_account {
        let reserve = token_account(&funding.try_borrow_data()?)
            .map_err(|_| fail(MigrationError::FundingAccountMismatch))?;
        if reserve.mint != config.destination_mint || reserve.owner != config.migration_authority {
            return Err(fail(MigrationError::FundingAccountMismatch));
        }
        if reserve.amount < output {
            return Err(fail(MigrationError::InsufficientReserve));
        }
    } else if mint_authority(destination_mint)? != Some(config.migration_authority) {
        return Err(fail(MigrationError::MintAuthorityMismatch));
    }

    // 1. Source disposition through the real source token program.
    let source_before = source_state.amount;
    let mut holder_metas = vec![AccountMeta::new_readonly(*holder.key, holder.is_signer)];
    holder_metas.extend(
        multisig_signers
            .iter()
            .map(|s| AccountMeta::new_readonly(*s.key, true)),
    );
    let mut holder_infos = vec![holder.clone()];
    holder_infos.extend(multisig_signers.iter().cloned());
    let supply_before = mint_supply(source_mint)?;
    let escrow_before = match escrow {
        Some(escrow) => gross_of(escrow)?,
        None => 0,
    };
    if let Some(escrow) = escrow {
        let mut metas = vec![
            AccountMeta::new(*source.key, false),
            AccountMeta::new_readonly(*source_mint.key, false),
            AccountMeta::new(*escrow.key, false),
        ];
        metas.extend(holder_metas);
        let mut infos = vec![
            source.clone(),
            source_mint.clone(),
            escrow.clone(),
            source_program.clone(),
        ];
        infos.extend(holder_infos);
        invoke(
            &token_instruction(
                source_program.key,
                TRANSFER_CHECKED,
                amount,
                config.source_decimals,
                metas,
            ),
            &infos,
        )?;
    } else {
        let mut metas = vec![
            AccountMeta::new(*source.key, false),
            AccountMeta::new(*source_mint.key, false),
        ];
        metas.extend(holder_metas);
        let mut infos = vec![source.clone(), source_mint.clone(), source_program.clone()];
        infos.extend(holder_infos);
        invoke(
            &token_instruction(
                source_program.key,
                BURN_CHECKED,
                amount,
                config.source_decimals,
                metas,
            ),
            &infos,
        )?;
    }
    if source_before.checked_sub(amount_of(source)?) != Some(amount) {
        return Err(fail(MigrationError::SourceNotConsumedExactly));
    }
    match escrow {
        Some(escrow) => {
            if gross_of(escrow)?.checked_sub(escrow_before) != Some(u128::from(amount)) {
                return Err(fail(MigrationError::EscrowNotCreditedExactly));
            }
        }
        None => {
            if supply_before.checked_sub(mint_supply(source_mint)?) != Some(amount) {
                return Err(fail(MigrationError::SupplyNotReducedExactly));
            }
        }
    }

    // 2. Destination funding through the real destination token program.
    let seeds: Vec<&[u8]> = signer_seeds.iter().map(Vec::as_slice).collect();
    let signer_group = [seeds.as_slice()];
    let signers: &[&[&[u8]]] = if seeds.is_empty() { &[] } else { &signer_group };
    let destination_before = gross_of(destination)?;
    match funding_account {
        Some(reserve) => {
            let reserve_before = amount_of(reserve)?;
            invoke_signed(
                &token_instruction(
                    destination_program.key,
                    TRANSFER_CHECKED,
                    output,
                    config.destination_decimals,
                    vec![
                        AccountMeta::new(*reserve.key, false),
                        AccountMeta::new_readonly(*destination_mint.key, false),
                        AccountMeta::new(*destination.key, false),
                        AccountMeta::new_readonly(*authority.key, true),
                    ],
                ),
                &[
                    reserve.clone(),
                    destination_mint.clone(),
                    destination.clone(),
                    authority.clone(),
                    destination_program.clone(),
                ],
                signers,
            )?;
            if reserve_before.checked_sub(amount_of(reserve)?) != Some(output) {
                return Err(fail(MigrationError::ReserveNotReleasedExactly));
            }
        }
        None => {
            let minted_before = mint_supply(destination_mint)?;
            invoke_signed(
                &token_instruction(
                    destination_program.key,
                    MINT_TO_CHECKED,
                    output,
                    config.destination_decimals,
                    vec![
                        AccountMeta::new(*destination_mint.key, false),
                        AccountMeta::new(*destination.key, false),
                        AccountMeta::new_readonly(*authority.key, true),
                    ],
                ),
                &[
                    destination_mint.clone(),
                    destination.clone(),
                    authority.clone(),
                    destination_program.clone(),
                ],
                signers,
            )?;
            if mint_supply(destination_mint)?.checked_sub(minted_before) != Some(output) {
                return Err(fail(MigrationError::SupplyNotIncreasedExactly));
            }
        }
    }
    if gross_of(destination)?.checked_sub(destination_before) != Some(u128::from(output)) {
        return Err(fail(MigrationError::DestinationNotCreditedExactly));
    }
    msg!(
        "EPLYX_TOKEN_MIGRATION v1 consumed={} fee={} converted={} output={} disposition={} funding={}",
        amount,
        fee,
        converted,
        output,
        if config.disposition == 0 { "burn" } else { "escrow" },
        if config.funding == 0 { "reserve" } else { "mint" }
    );
    Ok(())
}

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(entry);

#[cfg(not(feature = "no-entrypoint"))]
fn entry(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    process_instruction(program_id, accounts, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine and this program share one golden-vector file.
    #[cfg(not(feature = "defect-fee-ceiling"))]
    const VECTORS: &str = include_str!("../../../fixtures/migration/economics-golden-vectors.csv");

    #[test]
    #[cfg(not(feature = "defect-fee-ceiling"))]
    fn golden_vectors_match_the_program() {
        let mut checked = 0;
        for line in VECTORS.lines().skip(1).filter(|l| !l.trim().is_empty()) {
            let c: Vec<&str> = line.split(',').collect();
            assert_eq!(c.len(), 12, "{line}");
            let n = |i: usize| c[i].parse::<u64>().unwrap();
            let terms = Terms {
                numerator: n(2),
                denominator: n(3),
                rounding: if c[4] == "floor" { 0 } else { 1 },
                fee_bps: n(5) as u16,
                minimum_output: n(6),
            };
            let actual = migrate_amount(n(1), &terms);
            if c[11].is_empty() {
                assert_eq!(actual, Ok((n(7), n(8), n(9))), "{}", c[0]);
            } else {
                assert_eq!(actual.map_err(MigrationError::name), Err(c[11]), "{}", c[0]);
            }
            checked += 1;
        }
        assert!(checked >= 20);
    }

    #[test]
    fn config_round_trips_and_rejects_bad_layouts() {
        let config = Config {
            source_mint: Pubkey::new_from_array([1; 32]),
            destination_mint: Pubkey::new_from_array([2; 32]),
            source_program: Pubkey::new_from_array([3; 32]),
            destination_program: Pubkey::new_from_array([4; 32]),
            migration_authority: Pubkey::new_from_array([5; 32]),
            authority_kind: 0,
            authority_bump: 254,
            source_decimals: 9,
            destination_decimals: 6,
            terms: Terms {
                numerator: 3,
                denominator: 7,
                rounding: 1,
                fee_bps: 25,
                minimum_output: 1,
            },
            disposition: 1,
            funding: 0,
            funding_account: Pubkey::new_from_array([6; 32]),
            escrow_account: Pubkey::new_from_array([7; 32]),
            window_basis: 1,
            activation: 10,
            deadline: 20,
            authorization: AUTHORIZE_OWNER | AUTHORIZE_DELEGATE,
            spec_digest: [9; 32],
        };
        let packed = config.pack();
        assert_eq!(Config::unpack(&packed), Ok(config));
        let mut bad = packed;
        bad[0] = 2;
        assert_eq!(Config::unpack(&bad), Err(MigrationError::ConfigLayout));
        let mut bad = packed;
        bad[275] = 0;
        assert_eq!(Config::unpack(&bad), Err(MigrationError::ConfigLayout));
        assert_eq!(
            Config::unpack(&packed[..CONFIG_LEN - 1]),
            Err(MigrationError::ConfigLayout)
        );
    }

    #[test]
    #[cfg(not(feature = "defect-deadline-inclusive"))]
    fn window_is_activation_inclusive_and_deadline_exclusive() {
        let mut data = [0u8; CONFIG_LEN];
        data[0] = 1;
        data[258] = 1;
        data[259..267].copy_from_slice(&10u64.to_le_bytes());
        data[267..275].copy_from_slice(&20u64.to_le_bytes());
        data[275] = 1;
        let config = Config::unpack(&data).unwrap();
        assert_eq!(
            config.window_error(9, 0),
            Some(MigrationError::MigrationNotActive)
        );
        assert_eq!(config.window_error(10, 0), None);
        assert_eq!(config.window_error(19, 0), None);
        assert_eq!(
            config.window_error(20, 0),
            Some(MigrationError::MigrationWindowClosed)
        );
    }

    #[test]
    fn error_codes_are_stable_and_named() {
        for (index, error) in MigrationError::ALL.iter().enumerate() {
            assert_eq!(*error as u32, index as u32 + 1);
            assert_eq!(MigrationError::from_code(*error as u32), Some(*error));
        }
        assert_eq!(MigrationError::InsufficientReserve as u32, 20);
    }
}
