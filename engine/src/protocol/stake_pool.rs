//! SPL Stake Pool adapter: `DepositSol`.
//!
//! Token-2022 transfers gave Phase 7 real user balances and a real upgrade, but
//! the execution graph stopped at one program. A stake-pool deposit is the first
//! target where that is no longer true: one instruction moves lamports into a
//! stake account through the System program and mints pool tokens through the
//! SPL Token program, and the number of pool tokens is *computed* from pool
//! state rather than named in the instruction. That is what makes it worth
//! replaying - a share calculation is exactly the kind of thing an upgrade can
//! change by a rounding step, in a way that no byte diff explains and no fee
//! schedule announces.
//!
//! The supported contract is one `DepositSol` against a pool with no SOL deposit
//! authority, alongside compute-budget instructions, plain System transfers, and
//! an idempotent associated-token-account instruction that provably did not
//! create anything. Everything else is rejected rather than approximated:
//! `DepositStake`, `WithdrawStake`, slippage variants, pools gated by a deposit
//! authority, and the versioned lookup-table transactions most aggregators send.

use super::{
    BoundaryDistance, EconomicChange, EntityId, FieldValue, ProtocolAdapter, SemanticAccount,
    SemanticAction, SemanticField, StateFeature, TokenQuantity,
};
use crate::{
    executor::ExecutionResult,
    ingest::transactions::{HistoricalTransaction, TokenBalance},
    types::{AccountSnapshot, InstructionSpec, NamedAccount},
};
use anyhow::{Context, Result};

pub const PROGRAM_ID: &str = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";
pub const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";
pub const COMPUTE_BUDGET_PROGRAM_ID: &str = "ComputeBudget111111111111111111111111111111";
pub const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const ASSOCIATED_TOKEN_PROGRAM_ID: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub const STAKE_PROGRAM_ID: &str = "Stake11111111111111111111111111111111111111";

/// `StakePoolInstruction::DepositSol`, borsh-tagged by variant index.
const DEPOSIT_SOL: u8 = 14;
/// Discriminant plus the `u64` lamport amount.
const DEPOSIT_SOL_LEN: usize = 9;
/// `DepositSol` without a SOL deposit authority takes exactly these accounts.
const DEPOSIT_SOL_ACCOUNTS: usize = 10;
/// `SystemInstruction::Transfer`, followed by a `u64`.
const SYSTEM_TRANSFER: u32 = 2;
const SYSTEM_TRANSFER_LEN: usize = 12;
/// `AssociatedTokenAccountInstruction::CreateIdempotent`.
const CREATE_IDEMPOTENT: u8 = 1;
/// SPL Token `Approve`: the delegation a withdrawing client grants so the pool
/// can burn its pool tokens. Discriminant, then a u64 amount.
const TOKEN_APPROVE: u8 = 4;
const TOKEN_APPROVE_LEN: usize = 9;
const TOKEN_APPROVE_ACCOUNTS: usize = 3;

/// `AccountType::StakePool`.
const ACCOUNT_TYPE_STAKE_POOL: u8 = 1;
/// Base SPL Token layouts, which the pool mint and every pool token account use.
const TOKEN_ACCOUNT_LEN: usize = 165;
const MINT_LEN: usize = 82;
/// `StakeStateV2`, which is what the reserve stake account holds.
const STAKE_STATE_LEN: usize = 200;

/// Roles, in the order `DepositSol` declares them.
/// `WithdrawSol`: burn pool tokens, pay the manager fee, withdraw lamports from
/// the reserve. Twelve accounts; a pool gated by a SOL withdraw authority passes
/// that authority as a thirteenth and is not supported.
const WITHDRAW_SOL: u8 = 16;
const WITHDRAW_SOL_LEN: usize = 9;
const WITHDRAW_SOL_ACCOUNTS: usize = 12;
const CLOCK_SYSVAR_ID: &str = "SysvarC1ock11111111111111111111111111111111";
const STAKE_HISTORY_SYSVAR_ID: &str = "SysvarStakeHistory1111111111111111111111111";

const WITHDRAW_SOL_ROLES: [&str; WITHDRAW_SOL_ACCOUNTS] = [
    "stake-pool",
    "withdraw-authority",
    "user-transfer-authority",
    "source-pool-token",
    "reserve-stake",
    "destination-lamports",
    "manager-fee",
    "pool-mint",
    "clock-sysvar",
    "stake-history-sysvar",
    "stake-program",
    "token-program",
];

/// The stake-pool instructions this adapter replays exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoolOp {
    DepositSol,
    WithdrawSol,
}

impl PoolOp {
    fn from_discriminant(discriminant: u8) -> Option<Self> {
        match discriminant {
            DEPOSIT_SOL => Some(Self::DepositSol),
            WITHDRAW_SOL => Some(Self::WithdrawSol),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::DepositSol => "DepositSol",
            Self::WithdrawSol => "WithdrawSol",
        }
    }

    fn roles(self) -> &'static [&'static str] {
        match self {
            Self::DepositSol => &DEPOSIT_SOL_ROLES,
            Self::WithdrawSol => &WITHDRAW_SOL_ROLES,
        }
    }

    fn data_len(self) -> usize {
        match self {
            Self::DepositSol => DEPOSIT_SOL_LEN,
            Self::WithdrawSol => WITHDRAW_SOL_LEN,
        }
    }

    /// Position of the account that must sign: the lamport source for a
    /// deposit, the pool-token transfer authority for a withdrawal.
    fn authority_position(self) -> usize {
        match self {
            Self::DepositSol => 3,
            Self::WithdrawSol => 2,
        }
    }

    /// Position of the pool-token account whose holding changes hands. That is
    /// the economic entity: the wallet would conflate people sharing a fee
    /// payer, and the pool would collapse every user into one.
    fn entity_position(self) -> usize {
        match self {
            Self::DepositSol => 4,  // destination-pool-token
            Self::WithdrawSol => 3, // source-pool-token
        }
    }

    /// Programs the instruction names in fixed declared positions.
    fn declared_programs(self) -> &'static [(usize, &'static str)] {
        match self {
            Self::DepositSol => &[(8, SYSTEM_PROGRAM_ID), (9, TOKEN_PROGRAM_ID)],
            Self::WithdrawSol => &[
                (8, CLOCK_SYSVAR_ID),
                (9, STAKE_HISTORY_SYSVAR_ID),
                (10, STAKE_PROGRAM_ID),
                (11, TOKEN_PROGRAM_ID),
            ],
        }
    }

    /// Programs this operation's execution is defined to reach by CPI.
    fn cpi_programs(self) -> &'static [&'static str] {
        match self {
            Self::DepositSol => &[SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID],
            Self::WithdrawSol => &[TOKEN_PROGRAM_ID, STAKE_PROGRAM_ID],
        }
    }

    fn semantic_action(self) -> SemanticAction {
        match self {
            Self::DepositSol => SemanticAction::Deposit,
            Self::WithdrawSol => SemanticAction::Withdraw,
        }
    }

    /// The precise action in the finding vocabulary.
    ///
    /// `deposit` groups DepositSol with a future DepositStake, which is what
    /// the corpus selector wants. An expectation must not: `deposit_sol` names
    /// one instruction and keeps naming one instruction.
    fn action_id(self) -> &'static str {
        match self {
            Self::DepositSol => "deposit_sol",
            Self::WithdrawSol => "withdraw_sol",
        }
    }
}

/// Quantities promoted to the public expectation surface.
///
/// Deliberately a short list. `summarize` derives more than this, but a field
/// there is a reporting detail while a subject here is a compatibility
/// commitment: the moment a team names one in their TOML, it has to keep
/// meaning the same thing. Promoted only where the quantity is economically
/// meaningful to a user, decoded from where value actually landed rather than
/// from the instruction's stated amount, and produced identically from either
/// build so a V1/V2 comparison is well defined.
///
/// Each entry pairs the `summarize` field name with the account whose presence
/// makes it measurable at all.
/// `(operation, subject, the account that makes it measurable)`.
///
/// The operation is part of the key, not decoration. `pool-mint` is present on
/// a deposit as well as a withdrawal, so keying on the account alone would have
/// a deposit claim it can measure `pool_tokens_burned` - and an expectation
/// about burning would then be judged against an observation that only mints.
const PROMOTED_SUBJECTS: [(PoolOp, &str, &str); 4] = [
    (
        PoolOp::DepositSol,
        "pool_tokens_received",
        "destination-pool-token",
    ),
    // The holder's debit, which on a withdrawal includes the manager fee.
    (
        PoolOp::WithdrawSol,
        "pool_tokens_debited",
        "source-pool-token",
    ),
    // The burn proper: the mint's supply decrease.
    (PoolOp::WithdrawSol, "pool_tokens_burned", "pool-mint"),
    (
        PoolOp::WithdrawSol,
        "sol_received_by_user",
        "destination-lamports",
    ),
];

const DEPOSIT_SOL_ROLES: [&str; DEPOSIT_SOL_ACCOUNTS] = [
    "stake-pool",
    "withdraw-authority",
    "reserve-stake",
    "depositor",
    "destination-pool-token",
    "manager-fee",
    "referral-fee",
    "pool-mint",
    "system-program",
    "token-program",
];

/// Fields denominated in pool tokens rather than in lamports.
///
/// A pool token's decimal count belongs to the pool mint, which an individual
/// account does not carry, so [`ProtocolAdapter::decode`] leaves these in raw
/// base units and the interpretation layer rescales once the mint is known.
/// Lamport fields are decoded at their real precision and must not be rescaled,
/// which is why the distinction is an explicit list rather than a guess at the
/// decimal count.
const POOL_TOKEN_FIELDS: [&str; 5] = [
    "pool_token_supply",
    "last_epoch_pool_token_supply",
    "supply",
    "amount",
    "delegated_amount",
];

/// Lamports carry nine decimals. SOL is not a protocol asset being valued here;
/// this is the native unit's own precision.
const LAMPORT_DECIMALS: u8 = 9;

pub struct StakePoolAdapter;

/// Sequential borsh reader.
///
/// The `StakePool` layout is not fixed-offset: three `Option<Pubkey>` fields and
/// three `FutureEpoch<Fee>` fields each change length with their contents, so a
/// table of constants would silently mis-read a pool configured differently from
/// the one it was written against.
struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.offset..self.offset + length)?;
        self.offset += length;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn address(&mut self) -> Option<String> {
        Some(bs58::encode(self.take(32)?).into_string())
    }

    /// `Fee { denominator, numerator }`, in that declaration order.
    fn fee(&mut self) -> Option<Fee> {
        let denominator = self.u64()?;
        let numerator = self.u64()?;
        Some(Fee {
            numerator,
            denominator,
        })
    }

    /// `FutureEpoch<Fee>`: a tag, then the fee for `One` and `Two`.
    fn future_fee(&mut self) -> Option<Option<Fee>> {
        match self.u8()? {
            0 => Some(None),
            1 | 2 => Some(Some(self.fee()?)),
            _ => None,
        }
    }

    fn option_address(&mut self) -> Option<Option<String>> {
        match self.u8()? {
            0 => Some(None),
            1 => Some(Some(self.address()?)),
            _ => None,
        }
    }
}

/// A rational fee, applied by truncating multiplication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fee {
    pub numerator: u64,
    pub denominator: u64,
}

impl Fee {
    /// The program's own `Fee::apply`: a zero denominator means no fee.
    pub fn apply(self, amount: u64) -> Option<u128> {
        if self.denominator == 0 {
            return Some(0);
        }
        u128::from(amount)
            .checked_mul(u128::from(self.numerator))?
            .checked_div(u128::from(self.denominator))
    }
}

/// The fields of `StakePool` this adapter reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StakePool {
    pub reserve_stake: String,
    pub pool_mint: String,
    pub manager_fee_account: String,
    pub token_program_id: String,
    pub total_lamports: u64,
    pub pool_token_supply: u64,
    pub last_update_epoch: u64,
    pub epoch_fee: Fee,
    pub sol_deposit_authority: Option<String>,
    pub sol_deposit_fee: Fee,
    pub sol_referral_fee: u8,
    /// Set when the pool gates SOL withdrawals behind an authority. Such a pool
    /// passes that authority as a thirteenth account, which is outside the
    /// supported `WithdrawSol` shape.
    pub sol_withdraw_authority: Option<String>,
    pub sol_withdrawal_fee: Fee,
    pub last_epoch_pool_token_supply: u64,
    pub last_epoch_total_lamports: u64,
}

impl StakePool {
    /// Decode the pool state. `None` when the bytes are not an initialized pool.
    ///
    /// Trailing bytes are expected and ignored: the program serializes over a
    /// fixed-size account, so a pool that once had a longer encoding leaves
    /// stale bytes past the current one. The program reads the same way.
    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut reader = Reader::new(data);
        if reader.u8()? != ACCOUNT_TYPE_STAKE_POOL {
            return None;
        }
        let _manager = reader.address()?;
        let _staker = reader.address()?;
        let _stake_deposit_authority = reader.address()?;
        let _stake_withdraw_bump_seed = reader.u8()?;
        let _validator_list = reader.address()?;
        let reserve_stake = reader.address()?;
        let pool_mint = reader.address()?;
        let manager_fee_account = reader.address()?;
        let token_program_id = reader.address()?;
        let total_lamports = reader.u64()?;
        let pool_token_supply = reader.u64()?;
        let last_update_epoch = reader.u64()?;
        // Lockup: unix_timestamp, epoch, custodian.
        let _lockup = reader.take(48)?;
        let epoch_fee = reader.fee()?;
        let _next_epoch_fee = reader.future_fee()?;
        let _preferred_deposit_validator = reader.option_address()?;
        let _preferred_withdraw_validator = reader.option_address()?;
        let _stake_deposit_fee = reader.fee()?;
        let _stake_withdrawal_fee = reader.fee()?;
        let _next_stake_withdrawal_fee = reader.future_fee()?;
        let _stake_referral_fee = reader.u8()?;
        let sol_deposit_authority = reader.option_address()?;
        let sol_deposit_fee = reader.fee()?;
        let sol_referral_fee = reader.u8()?;
        let sol_withdraw_authority = reader.option_address()?;
        let sol_withdrawal_fee = reader.fee()?;
        let _next_sol_withdrawal_fee = reader.future_fee()?;
        let last_epoch_pool_token_supply = reader.u64()?;
        let last_epoch_total_lamports = reader.u64()?;
        Some(Self {
            reserve_stake,
            pool_mint,
            manager_fee_account,
            token_program_id,
            total_lamports,
            pool_token_supply,
            last_update_epoch,
            epoch_fee,
            sol_deposit_authority,
            sol_deposit_fee,
            sol_referral_fee,
            sol_withdraw_authority,
            sol_withdrawal_fee,
            last_epoch_pool_token_supply,
            last_epoch_total_lamports,
        })
    }

    /// The program's `calc_lamports_withdraw_amount`.
    ///
    /// The mirror of the deposit path and, like it, reproduced only as the
    /// reference the adapter reasons with - the reported numbers come from
    /// running the real bytecode. Multiplication precedes division in u128,
    /// which is the property a truncating candidate breaks.
    pub fn lamports_for_withdrawal(&self, pool_tokens: u64) -> Option<u64> {
        if self.pool_token_supply == 0 {
            return Some(0);
        }
        u64::try_from(
            u128::from(pool_tokens)
                .checked_mul(u128::from(self.total_lamports))?
                .checked_div(u128::from(self.pool_token_supply))?,
        )
        .ok()
    }

    /// The program's `calc_pool_tokens_for_deposit`.
    ///
    /// Reproduced here as the reference the adapter reasons with, never as
    /// something the replay executes: the numbers reported come from running the
    /// real bytecode. Multiplication precedes division, in u128, which is the
    /// property a truncating candidate breaks.
    pub fn pool_tokens_for_deposit(&self, lamports: u64) -> Option<u64> {
        if self.total_lamports == 0 || self.pool_token_supply == 0 {
            return Some(lamports);
        }
        u64::try_from(
            u128::from(lamports)
                .checked_mul(u128::from(self.pool_token_supply))?
                .checked_div(u128::from(self.total_lamports))?,
        )
        .ok()
    }
}

fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn address_at(data: &[u8], offset: usize) -> Option<String> {
    Some(bs58::encode(data.get(offset..offset + 32)?).into_string())
}

/// Base-layout SPL Token account balance.
pub fn token_account_amount(data: &[u8]) -> Option<u64> {
    (data.len() == TOKEN_ACCOUNT_LEN).then(|| u64_at(data, 64))?
}

pub fn token_account_mint(data: &[u8]) -> Option<String> {
    (data.len() == TOKEN_ACCOUNT_LEN).then(|| address_at(data, 0))?
}

pub fn mint_decimals(data: &[u8]) -> Option<u8> {
    (data.len() == MINT_LEN)
        .then(|| data.get(44).copied())
        .flatten()
}

pub fn mint_supply(data: &[u8]) -> Option<u64> {
    (data.len() == MINT_LEN).then(|| u64_at(data, 36))?
}

impl StakePoolAdapter {
    /// The single supported pool instruction this record exists to replay,
    /// with the operation it encodes.
    fn operation<'a>(
        &self,
        transaction: &'a HistoricalTransaction,
    ) -> Result<(PoolOp, &'a InstructionSpec)> {
        let instruction = self.deposit(transaction)?;
        let discriminant = *instruction
            .data
            .first()
            .context("empty SPL Stake Pool instruction data")?;
        let op = PoolOp::from_discriminant(discriminant).with_context(|| {
            format!(
                "stake-pool replay supports DepositSol and WithdrawSol; \
                 found instruction variant {discriminant}"
            )
        })?;
        Ok((op, instruction))
    }

    /// Base-unit amount the operation names: lamports in, pool tokens out.
    fn operation_amount(&self, transaction: &HistoricalTransaction) -> Option<u64> {
        let (op, instruction) = self.operation(transaction).ok()?;
        (instruction.data.len() == op.data_len())
            .then(|| u64_at(&instruction.data, 1))
            .flatten()
    }

    /// The single pool instruction this record exists to replay.
    fn deposit<'a>(&self, transaction: &'a HistoricalTransaction) -> Result<&'a InstructionSpec> {
        let mut found = transaction
            .instructions
            .iter()
            .filter(|instruction| instruction.program == PROGRAM_ID);
        let instruction = found
            .next()
            .context("transaction contains no SPL Stake Pool instruction")?;
        anyhow::ensure!(
            found.next().is_none(),
            "stake-pool replay supports one pool instruction per transaction"
        );
        Ok(instruction)
    }

    /// Lamports the deposit instruction names.
    pub fn deposit_lamports(&self, transaction: &HistoricalTransaction) -> Option<u64> {
        let instruction = self.deposit(transaction).ok()?;
        (instruction.data.len() == DEPOSIT_SOL_LEN && instruction.data[0] == DEPOSIT_SOL)
            .then(|| u64_at(&instruction.data, 1))?
    }

    fn labels(&self, transaction: &HistoricalTransaction) -> Vec<String> {
        let mut labels: Vec<String> = (0..transaction.account_keys.len())
            .map(|index| format!("key-{index}"))
            .collect();
        if let Ok((op, instruction)) = self.operation(transaction) {
            for (position, role) in op.roles().iter().copied().enumerate() {
                let Some(meta) = instruction.accounts.get(position) else {
                    continue;
                };
                let Some(index) = transaction
                    .account_keys
                    .iter()
                    .position(|key| key.address == meta.address)
                else {
                    continue;
                };
                // One account can hold two roles - a depositor who takes the
                // referral position, most commonly - and the first role named is
                // the one that explains what it is doing.
                if labels[index].starts_with("key-") {
                    labels[index] = role.to_string();
                }
            }
        }
        if let Some(label) = labels.first_mut() {
            if label.starts_with("key-") {
                *label = "payer".into();
            }
        }
        labels
    }

    fn balance_at<'a>(
        &self,
        balances: Option<&'a Vec<TokenBalance>>,
        index: usize,
    ) -> Option<&'a TokenBalance> {
        balances?
            .iter()
            .find(|balance| balance.account_index == index)
    }

    /// Pool mint decimals, from whichever snapshot carries the mint.
    fn pool_decimals(&self, accounts: &[NamedAccount]) -> u8 {
        accounts
            .iter()
            .find(|named| named.label == "pool-mint")
            .and_then(|named| mint_decimals(&named.account.data))
            .or_else(|| {
                accounts
                    .iter()
                    .find_map(|named| mint_decimals(&named.account.data))
            })
            .unwrap_or(0)
    }

    /// Rescale a decoded field into the units a reader expects.
    fn rescale(&self, field: &str, quantity: TokenQuantity, decimals: u8) -> TokenQuantity {
        if POOL_TOKEN_FIELDS.contains(&field) {
            TokenQuantity::new(quantity.base_units, decimals)
        } else {
            quantity
        }
    }

    /// Post-execution snapshot of one labelled account.
    fn after<'a>(&self, result: &'a ExecutionResult, label: &str) -> Option<&'a AccountSnapshot> {
        result.accounts.get(label)
    }
}

impl ProtocolAdapter for StakePoolAdapter {
    fn name(&self) -> &'static str {
        "spl-stake-pool"
    }

    fn program_id(&self) -> &'static str {
        PROGRAM_ID
    }

    fn supports_cpi(&self) -> bool {
        true
    }

    fn dependency_programs(&self) -> &'static [&'static str] {
        // `DepositSol` moves lamports through the System program and mints
        // through the token program named in pool state. Declaring them means
        // they are resolved at the historical slot and recorded even if a
        // validator ever truncated the inner-instruction metadata.
        &[SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID]
    }

    /// Widened in Phase 9 from `DepositSol` alone to `DepositSol` and
    /// `WithdrawSol`, and given semantic classification.
    fn adapter_version(&self) -> u32 {
        // 3: `pool_tokens_burned` is the mint's supply decrease. Under 2 it was
        // the source account's debit, which included transferred fees.
        3
    }

    fn semantic_action(&self, transaction: &HistoricalTransaction) -> SemanticAction {
        self.operation(transaction)
            .map(|(op, _)| op.semantic_action())
            .unwrap_or(SemanticAction::Unknown)
    }

    /// The depositor's pool-token account: the position that gains the shares.
    ///
    /// The depositing wallet would conflate two people who happen to share a
    /// fee payer, and the pool itself would collapse every depositor into one
    /// entity. The destination pool-token account is the holding that actually
    /// changes hands.
    fn economic_entity_id(
        &self,
        transaction: &HistoricalTransaction,
        _accounts: &[NamedAccount],
    ) -> Option<EntityId> {
        let (op, instruction) = self.operation(transaction).ok()?;
        Some(EntityId::new(
            "pool-token-account",
            instruction
                .accounts
                .get(op.entity_position())?
                .address
                .clone(),
        ))
    }

    fn state_features(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
    ) -> Vec<StateFeature> {
        let action = self.semantic_action(transaction);
        let mut features = vec![StateFeature::text("semantic_action", action.as_str())];
        if let Some(amount) = self.operation_amount(transaction) {
            // Lamports paid in for a deposit; pool tokens burned for a
            // withdrawal. Named for what it is rather than a shared label.
            let name = match action {
                SemanticAction::Withdraw => "pool_tokens_burned",
                _ => "deposit_lamports",
            };
            features.push(StateFeature::integer(name, amount as u128));
        }
        if let Ok((_, instruction)) = self.operation(transaction) {
            features.push(StateFeature::text(
                "pool",
                instruction.accounts[0].address.clone(),
            ));
        }
        // Pool-wide quantities come from the decoded stake-pool account, so they
        // are whatever the adapter already proves it can read.
        for named in accounts {
            let Some(decoded) = self.decode(&named.account) else {
                continue;
            };
            if decoded.kind != "stake-pool" {
                continue;
            }
            for field in [
                "total_lamports",
                "pool_token_supply",
                "sol_deposit_fee_basis_points",
            ] {
                if let Some(value) = decoded
                    .field(field)
                    .and_then(|f| f.value.as_quantity())
                    .map(|q| q.base_units)
                {
                    features.push(StateFeature::integer(field, value as u128));
                }
            }
        }
        features
    }

    /// A deposit's share price is `total_lamports / pool_token_supply`, and the
    /// interesting boundary is where the deposit is large enough relative to the
    /// pool to move it. Nothing else in this contract has a threshold with a
    /// branch behind it, so nothing else is claimed.
    fn boundaries(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
    ) -> Vec<BoundaryDistance> {
        let Some(amount) = self.operation_amount(transaction) else {
            return Vec::new();
        };
        let action = self.semantic_action(transaction);
        for named in accounts {
            let Some(decoded) = self.decode(&named.account) else {
                continue;
            };
            if decoded.kind != "stake-pool" {
                continue;
            }
            if let Some(total) = decoded
                .field("total_lamports")
                .and_then(|f| f.value.as_quantity())
                .map(|q| q.base_units)
            {
                // A deposit is denominated in lamports and compares directly
                // against pool size. A withdrawal is denominated in pool tokens,
                // so it is compared against the token supply instead; mixing the
                // two would be an arithmetic category error.
                let (reference, label) = match action {
                    SemanticAction::Withdraw => (
                        decoded
                            .field("pool_token_supply")
                            .and_then(|f| f.value.as_quantity())
                            .map(|q| q.base_units)
                            .unwrap_or(0),
                        "withdrawal_against_pool_supply",
                    ),
                    _ => (total, "deposit_against_pool_size"),
                };
                return vec![BoundaryDistance::from_quantities(
                    label,
                    "interaction measured against the pool it moves; a small distance is an \
                     interaction large enough to move the share price",
                    u128::from(amount),
                    u128::from(reference),
                )];
            }
        }
        Vec::new()
    }

    fn accept(&self, transaction: &HistoricalTransaction) -> Result<()> {
        super::require_executable_message(transaction)?;
        anyhow::ensure!(
            transaction.success && transaction.error.is_none(),
            "replay selects successfully captured original transactions"
        );
        let (op, instruction) = self.operation(transaction)?;
        anyhow::ensure!(
            instruction.data.len() == op.data_len(),
            "{} data must be the discriminant and a u64 amount",
            op.name()
        );
        anyhow::ensure!(
            instruction.accounts.len() == op.roles().len(),
            "{} with {} accounts is outside the supported shape; a pool gated by a SOL \
             deposit or withdraw authority passes that authority as an extra account and is \
             not supported",
            op.name(),
            instruction.accounts.len()
        );
        anyhow::ensure!(
            instruction.accounts[op.authority_position()].is_signer,
            "the {} must be a direct signer",
            op.roles()[op.authority_position()]
        );
        for (position, program) in op.declared_programs() {
            anyhow::ensure!(
                instruction.accounts[*position].address == *program,
                "{} must name {} at declared position {position}",
                op.name(),
                op.roles()[*position]
            );
        }
        anyhow::ensure!(
            self.operation_amount(transaction).is_some_and(|a| a > 0),
            "{} must name a non-zero amount",
            op.name()
        );

        for (index, companion) in transaction.instructions.iter().enumerate() {
            match companion.program.as_str() {
                PROGRAM_ID | COMPUTE_BUDGET_PROGRAM_ID => {}
                SYSTEM_PROGRAM_ID => {
                    anyhow::ensure!(
                        op == PoolOp::DepositSol,
                        "a top-level System instruction accompanies a deposit, not a {}",
                        op.name()
                    );
                    // Plain transfers only. Account creation and allocation are
                    // outside the contract, and both are System instructions.
                    anyhow::ensure!(
                        companion.data.len() == SYSTEM_TRANSFER_LEN
                            && u32::from_le_bytes(
                                companion.data[..4].try_into().expect("length checked")
                            ) == SYSTEM_TRANSFER
                            && companion.accounts.len() == 2,
                        "top-level System instruction {index} is not a plain transfer; \
                         account creation and allocation are not supported"
                    );
                }
                TOKEN_PROGRAM_ID => {
                    // A withdrawing client delegates its pool tokens to the
                    // authority the pool will burn with. That delegation is part
                    // of the withdrawal, and it is admitted only when it
                    // provably is: same account, same delegate, same amount.
                    // Anything else is an unrelated approval and is refused.
                    anyhow::ensure!(
                        op == PoolOp::WithdrawSol,
                        "a top-level SPL Token instruction accompanies a withdrawal, not a {}",
                        op.name()
                    );
                    anyhow::ensure!(
                        companion.data.first() == Some(&TOKEN_APPROVE)
                            && companion.data.len() == TOKEN_APPROVE_LEN,
                        "the SPL Token program is supported for Approve only; top-level \
                         instruction {index} is a different variant"
                    );
                    anyhow::ensure!(
                        companion.accounts.len() == TOKEN_APPROVE_ACCOUNTS,
                        "Approve with {} accounts is outside the supported shape; a multisig \
                         owner passes additional signers",
                        companion.accounts.len()
                    );
                    anyhow::ensure!(
                        companion.accounts[0].address == instruction.accounts[3].address,
                        "the Approve delegates {}, not the account this withdrawal burns from",
                        companion.accounts[0].address
                    );
                    anyhow::ensure!(
                        companion.accounts[1].address == instruction.accounts[2].address,
                        "the Approve names a delegate other than the authority the withdrawal \
                         burns with"
                    );
                    anyhow::ensure!(
                        companion.accounts[2].is_signer,
                        "the Approve owner must be a direct signer"
                    );
                    anyhow::ensure!(
                        u64_at(&companion.data, 1) == u64_at(&instruction.data, 1),
                        "the Approve delegates a different amount than the withdrawal burns"
                    );
                }
                ASSOCIATED_TOKEN_PROGRAM_ID => {
                    anyhow::ensure!(
                        op == PoolOp::DepositSol,
                        "idempotent token-account creation accompanies a deposit, not a {}",
                        op.name()
                    );
                    anyhow::ensure!(
                        companion.data == [CREATE_IDEMPOTENT],
                        "the associated-token-account program is supported for \
                         CreateIdempotent only"
                    );
                    // Idempotent creation is only in scope when it provably had
                    // nothing to do: the validator recorded a token balance for
                    // the account on both sides, so it already existed.
                    let target = companion
                        .accounts
                        .get(1)
                        .context("CreateIdempotent names no account to create")?;
                    let index_of = transaction
                        .account_keys
                        .iter()
                        .position(|key| key.address == target.address)
                        .context("CreateIdempotent target is not a message key")?;
                    anyhow::ensure!(
                        self.balance_at(transaction.pre_token_balances.as_ref(), index_of)
                            .is_some()
                            && self
                                .balance_at(transaction.post_token_balances.as_ref(), index_of)
                                .is_some(),
                        "CreateIdempotent targets {} which the validator did not record as an \
                         existing token account on both sides; account creation is outside the \
                         supported contract",
                        target.address
                    );
                }
                other => anyhow::bail!("unsupported program {other} in a stake-pool replay"),
            }
        }

        // CPI is supported, but only into the programs this operation is defined
        // to reach. Anything else means a different execution graph. A deposit
        // reaches System and Token; a withdrawal reaches Token and Stake.
        for frame in &transaction.inner_instruction_frames {
            anyhow::ensure!(
                op.cpi_programs().contains(&frame.program.as_str()),
                "unsupported cross-program invocation into {} during a {} replay",
                frame.program,
                op.name()
            );
            anyhow::ensure!(
                frame.stack_height == 2,
                "stake-pool replay supports one level of cross-program invocation; \
                 observed depth {}",
                frame.stack_height
            );
        }
        anyhow::ensure!(
            transaction.inner_instructions.len() == transaction.inner_instruction_frames.len(),
            "inner-instruction metadata is incomplete; the invocation graph cannot be checked"
        );
        anyhow::ensure!(
            transaction.pre_token_balances.is_some() && transaction.post_token_balances.is_some(),
            "stake-pool replay requires validator-observed token balances as boundary evidence"
        );
        Ok(())
    }

    fn label(&self, transaction: &HistoricalTransaction, index: usize) -> String {
        self.labels(transaction)
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("key-{index}"))
    }

    fn required_accounts(&self, transaction: &HistoricalTransaction) -> Vec<String> {
        // Every account the operation declares, minus programs and sysvars:
        // those are the accounts the pool actually reads and writes.
        let Ok((_, instruction)) = self.operation(transaction) else {
            return Vec::new();
        };
        instruction
            .accounts
            .iter()
            .map(|meta| meta.address.clone())
            .filter(|address| {
                ![
                    SYSTEM_PROGRAM_ID,
                    TOKEN_PROGRAM_ID,
                    STAKE_PROGRAM_ID,
                    CLOCK_SYSVAR_ID,
                    STAKE_HISTORY_SYSVAR_ID,
                ]
                .contains(&address.as_str())
            })
            .collect()
    }

    fn decode(&self, account: &AccountSnapshot) -> Option<SemanticAccount> {
        match account.owner.as_str() {
            PROGRAM_ID => {
                let pool = StakePool::decode(&account.data)?;
                Some(SemanticAccount {
                    kind: "stake-pool".into(),
                    fields: vec![
                        SemanticField {
                            name: "total_lamports".into(),
                            value: FieldValue::quantity(pool.total_lamports, LAMPORT_DECIMALS),
                            economic: true,
                        },
                        SemanticField {
                            name: "pool_token_supply".into(),
                            value: FieldValue::quantity(pool.pool_token_supply, 0),
                            economic: true,
                        },
                        SemanticField {
                            name: "last_update_epoch".into(),
                            value: FieldValue::Count(pool.last_update_epoch),
                            economic: false,
                        },
                        SemanticField {
                            name: "epoch_fee_basis_points".into(),
                            value: FieldValue::Count(basis_points(pool.epoch_fee)),
                            economic: true,
                        },
                        SemanticField {
                            name: "sol_deposit_fee_basis_points".into(),
                            value: FieldValue::Count(basis_points(pool.sol_deposit_fee)),
                            economic: true,
                        },
                        SemanticField {
                            name: "sol_referral_fee_percent".into(),
                            value: FieldValue::Count(u64::from(pool.sol_referral_fee)),
                            economic: true,
                        },
                        SemanticField {
                            name: "reserve_stake".into(),
                            value: FieldValue::Address(pool.reserve_stake.clone()),
                            economic: false,
                        },
                        SemanticField {
                            name: "pool_mint".into(),
                            value: FieldValue::Address(pool.pool_mint.clone()),
                            economic: false,
                        },
                        SemanticField {
                            name: "last_epoch_pool_token_supply".into(),
                            value: FieldValue::quantity(pool.last_epoch_pool_token_supply, 0),
                            economic: false,
                        },
                        SemanticField {
                            name: "last_epoch_total_lamports".into(),
                            value: FieldValue::quantity(
                                pool.last_epoch_total_lamports,
                                LAMPORT_DECIMALS,
                            ),
                            economic: false,
                        },
                    ],
                })
            }
            TOKEN_PROGRAM_ID if account.data.len() == TOKEN_ACCOUNT_LEN => Some(SemanticAccount {
                kind: "token-account".into(),
                fields: vec![
                    SemanticField {
                        name: "mint".into(),
                        value: FieldValue::Address(address_at(&account.data, 0)?),
                        economic: false,
                    },
                    SemanticField {
                        name: "owner".into(),
                        value: FieldValue::Address(address_at(&account.data, 32)?),
                        economic: false,
                    },
                    SemanticField {
                        name: "amount".into(),
                        value: FieldValue::quantity(u64_at(&account.data, 64)?, 0),
                        economic: true,
                    },
                    SemanticField {
                        name: "state".into(),
                        value: FieldValue::Count(u64::from(*account.data.get(108)?)),
                        economic: true,
                    },
                    SemanticField {
                        name: "delegated_amount".into(),
                        value: FieldValue::quantity(u64_at(&account.data, 121)?, 0),
                        economic: true,
                    },
                ],
            }),
            TOKEN_PROGRAM_ID if account.data.len() == MINT_LEN => Some(SemanticAccount {
                kind: "mint".into(),
                fields: vec![
                    SemanticField {
                        name: "supply".into(),
                        value: FieldValue::quantity(mint_supply(&account.data)?, 0),
                        economic: true,
                    },
                    SemanticField {
                        name: "decimals".into(),
                        value: FieldValue::Count(u64::from(mint_decimals(&account.data)?)),
                        economic: false,
                    },
                ],
            }),
            STAKE_PROGRAM_ID if account.data.len() == STAKE_STATE_LEN => {
                // A stake account's economically load-bearing quantity for a SOL
                // deposit is its lamport balance: that is what the reserve
                // receives. Its delegation state is untouched by this path.
                Some(SemanticAccount {
                    kind: "stake-account".into(),
                    fields: vec![SemanticField {
                        name: "lamports".into(),
                        value: FieldValue::quantity(account.lamports, LAMPORT_DECIMALS),
                        economic: true,
                    }],
                })
            }
            _ => None,
        }
    }

    fn prove_boundaries(
        &self,
        transaction: &HistoricalTransaction,
        pre: &[NamedAccount],
        post: &[NamedAccount],
    ) -> Result<Vec<String>> {
        let pre_balances = transaction
            .pre_balances
            .as_ref()
            .context("transaction metadata omitted pre-balances")?;
        let post_balances = transaction
            .post_balances
            .as_ref()
            .context("transaction metadata omitted post-balances")?;
        let index_of = |address: &str| -> Result<usize> {
            transaction
                .account_keys
                .iter()
                .position(|key| key.address == address)
                .with_context(|| format!("snapshot account {address} is not in the message"))
        };
        let mut proved_token_accounts = 0_usize;

        for (side, snapshots, lamports, token_balances) in [
            (
                "pre",
                pre,
                pre_balances,
                transaction.pre_token_balances.as_ref(),
            ),
            (
                "post",
                post,
                post_balances,
                transaction.post_token_balances.as_ref(),
            ),
        ] {
            for named in snapshots {
                let index = index_of(&named.address)?;
                anyhow::ensure!(
                    lamports.get(index) == Some(&named.account.lamports),
                    "{} for {}: archive reports {} lamports at the {} boundary, validator \
                     metadata records {}. Another transaction in slot {} wrote this account \
                     {} this one.",
                    if side == "pre" {
                        "slot-before state is not this transaction's pre-state"
                    } else {
                        "slot-end state is not this transaction's post-state"
                    },
                    named.address,
                    named.account.lamports,
                    if side == "pre" { "S-1" } else { "S" },
                    lamports
                        .get(index)
                        .map(u64::to_string)
                        .unwrap_or_else(|| "nothing".into()),
                    transaction.slot,
                    if side == "pre" { "before" } else { "after" }
                );
                let Some(balance) = self.balance_at(token_balances, index) else {
                    continue;
                };
                let decoded = token_account_amount(&named.account.data).with_context(|| {
                    format!(
                        "account {} has a token balance but does not decode as a base-layout \
                         token account",
                        named.address
                    )
                })?;
                anyhow::ensure!(
                    decoded == balance.amount,
                    "{side}-state archive amount {decoded} differs from validator-observed {} \
                     for {}",
                    balance.amount,
                    named.address
                );
                anyhow::ensure!(
                    balance.program_id == TOKEN_PROGRAM_ID,
                    "account {} is owned by token program {}, not the SPL Token program",
                    named.address,
                    balance.program_id
                );
                anyhow::ensure!(
                    token_account_mint(&named.account.data).as_deref() == Some(&balance.mint),
                    "account {} decodes to a different mint than the validator recorded",
                    named.address
                );
                if side == "pre" {
                    proved_token_accounts += 1;
                }
            }
        }

        for named in pre {
            let index = index_of(&named.address)?;
            if transaction.account_keys[index].is_writable {
                continue;
            }
            // Sysvars are read-only to the transaction but rewritten by the
            // runtime every slot, so byte-identity across S-1 and S is not a
            // property they have and cannot be evidence of anything. Only the
            // two this contract's instructions actually name are exempted; any
            // other sysvar appearing here is still treated as a real change,
            // because the contract does not admit an instruction that reads one.
            if named.address == CLOCK_SYSVAR_ID || named.address == STAKE_HISTORY_SYSVAR_ID {
                continue;
            }
            let after = post
                .iter()
                .find(|other| other.address == named.address)
                .context("read-only account missing from post-state")?;
            anyhow::ensure!(
                named.account.data == after.account.data
                    && named.account.owner == after.account.owner,
                "read-only account {} changed across the transaction boundary",
                named.address
            );
        }

        // Validator metadata says nothing about a pool account's bytes, so the
        // pool would otherwise rest on the archive alone. It does not have to:
        // the two fields the deposit moves are each equal to something the
        // validator did observe, and checking that is what turns the pool
        // snapshot from trusted into corroborated.
        let find = |snapshots: &[NamedAccount], label: &str| {
            snapshots.iter().find(|named| named.label == label).cloned()
        };
        let mut corroboration = Vec::new();
        if let (Some(before), Some(after)) = (find(pre, "stake-pool"), find(post, "stake-pool")) {
            let opening = StakePool::decode(&before.account.data)
                .context("pre-state stake pool does not decode")?;
            let closing = StakePool::decode(&after.account.data)
                .context("post-state stake pool does not decode")?;
            // A pool that gates this direction behind an authority passes that
            // authority as an extra account, so the shape we accepted could not
            // have been produced by one. Checking the pool's own bytes is what
            // turns that inference into evidence.
            match self.operation(transaction)?.0 {
                PoolOp::DepositSol => anyhow::ensure!(
                    opening.sol_deposit_authority.is_none(),
                    "this pool gates SOL deposits behind an authority, which the supported \
                     DepositSol shape does not carry"
                ),
                PoolOp::WithdrawSol => anyhow::ensure!(
                    opening.sol_withdraw_authority.is_none(),
                    "this pool gates SOL withdrawals behind an authority, which the supported \
                     WithdrawSol shape does not carry"
                ),
            }

            let reserve_index = index_of(&opening.reserve_stake)?;
            let observed_lamports =
                i128::from(post_balances[reserve_index]) - i128::from(pre_balances[reserve_index]);
            let recorded_lamports =
                i128::from(closing.total_lamports) - i128::from(opening.total_lamports);
            anyhow::ensure!(
                observed_lamports == recorded_lamports,
                "the pool records a {recorded_lamports} lamport change while the validator \
                 observed {observed_lamports} moving into the reserve; the archived pool \
                 state is not this transaction's boundary"
            );
            corroboration.push(
                "the pool's total_lamports change equals the validator-observed lamport change \
                 of the reserve stake account"
                    .to_string(),
            );

            let minted = |balances: Option<&Vec<TokenBalance>>| -> i128 {
                balances
                    .map(|entries| {
                        entries
                            .iter()
                            .filter(|balance| balance.mint == opening.pool_mint)
                            .map(|balance| i128::from(balance.amount))
                            .sum()
                    })
                    .unwrap_or(0)
            };
            let observed_minted = minted(transaction.post_token_balances.as_ref())
                - minted(transaction.pre_token_balances.as_ref());
            let recorded_minted =
                i128::from(closing.pool_token_supply) - i128::from(opening.pool_token_supply);
            anyhow::ensure!(
                observed_minted == recorded_minted,
                "the pool records {recorded_minted} pool tokens minted while the validator \
                 observed holdings of the pool mint change by {observed_minted}; the archived \
                 pool state is not this transaction's boundary"
            );
            corroboration.push(
                "the pool's pool_token_supply change equals the validator-observed change in \
                 holdings of the pool mint"
                    .to_string(),
            );
        }

        anyhow::ensure!(
            proved_token_accounts > 0,
            "no token account balance could be proved against validator metadata"
        );

        let mut assumptions = vec![
            format!(
                "{proved_token_accounts} token account balance(s) at S-1 and S match the \
                 validator-observed pre/post token balances"
            ),
            "every snapshot's lamports match validator-observed pre/post balances".into(),
        ];
        assumptions.extend(corroboration);
        assumptions.push(
            "read-only accounts are byte-identical across the boundary; validator metadata \
             records no account data, so their bytes rest on the archive and on V1 reproducing \
             the original outcome"
                .into(),
        );
        if transaction
            .account_keys
            .iter()
            .any(|key| key.address == CLOCK_SYSVAR_ID || key.address == STAKE_HISTORY_SYSVAR_ID)
        {
            assumptions.push(
                "the Clock and StakeHistory sysvars are exempt from that byte-identity check: \
                 the runtime rewrites them every slot, so they carry no boundary evidence and \
                 are supplied to the replay at their acquired S-1 values"
                    .into(),
            );
        }
        match self.operation(transaction)?.0 {
            PoolOp::DepositSol => {
                assumptions.push(
                    "the reserve stake account's delegation state is carried through \
                     unchanged: a SOL deposit credits its lamports and does not invoke the \
                     stake program"
                        .into(),
                );
                assumptions.push(
                    "supported contract is one DepositSol into a pool with no SOL deposit \
                     authority, with cross-program invocation into the System and SPL Token \
                     programs only"
                        .into(),
                );
            }
            PoolOp::WithdrawSol => {
                assumptions.push(
                    "the reserve stake account is debited through the Stake program's \
                     Withdraw, which the runtime implements natively; its delegation state \
                     and the rent-exempt floor it must retain are enforced by that program, \
                     not by this adapter"
                        .into(),
                );
                assumptions.push(
                    "supported contract is one WithdrawSol from a pool with no SOL withdraw \
                     authority, with cross-program invocation into the SPL Token and Stake \
                     programs only"
                        .into(),
                );
                assumptions.push(
                    "the Clock and StakeHistory sysvars the withdrawal reads are acquired at \
                     the historical boundary like any other account; the Stake program's \
                     lockup and deactivation checks therefore see the slot's real values"
                        .into(),
                );
            }
        }
        assumptions.push(
            "the pool's own share arithmetic reads no Clock; the pinned replay clock reproduces \
             the transaction's slot and block time"
                .into(),
        );
        Ok(assumptions)
    }

    fn interpret(
        &self,
        accounts: &[NamedAccount],
        v1: &ExecutionResult,
        v2: &ExecutionResult,
    ) -> Vec<EconomicChange> {
        let decimals = self.pool_decimals(accounts);
        let mut changes = Vec::new();
        for named in accounts {
            let (Some(after_v1), Some(after_v2)) =
                (self.after(v1, &named.label), self.after(v2, &named.label))
            else {
                continue;
            };
            let (Some(decoded_v1), Some(decoded_v2)) =
                (self.decode(after_v1), self.decode(after_v2))
            else {
                continue;
            };
            for field in &decoded_v1.fields {
                if !field.economic {
                    continue;
                }
                let Some(other) = decoded_v2.field(&field.name) else {
                    continue;
                };
                if other.value == field.value {
                    continue;
                }
                let render = |value: &FieldValue| match value.as_quantity() {
                    Some(quantity) => self.rescale(&field.name, quantity, decimals).to_string(),
                    None => value.render(),
                };
                let delta = match (field.value.as_quantity(), other.value.as_quantity()) {
                    (Some(before), Some(after)) => self
                        .rescale(&field.name, after, decimals)
                        .delta(self.rescale(&field.name, before, decimals)),
                    _ => None,
                };
                changes.push(EconomicChange {
                    account_label: named.label.clone(),
                    account_kind: decoded_v1.kind.clone(),
                    field: field.name.clone(),
                    v1: render(&field.value),
                    v2: render(&other.value),
                    delta,
                });
            }
        }
        changes
    }

    fn action_id(&self, transaction: &HistoricalTransaction) -> Option<crate::semantics::ActionId> {
        let (op, _) = self.operation(transaction).ok()?;
        crate::semantics::ActionId::new(op.action_id()).ok()
    }

    fn evaluable_subjects(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
    ) -> Vec<crate::semantics::EvaluableSubject> {
        use crate::semantics::{EvaluableSubject, FindingDomain, SemanticSubject};

        let (Some(protocol), Some(action)) = (self.protocol_id(), self.action_id(transaction))
        else {
            return Vec::new();
        };
        let subject = |domain, name: &str| {
            Some(EvaluableSubject {
                protocol: protocol.clone(),
                action: action.clone(),
                domain,
                subject: SemanticSubject::new(name).ok()?,
            })
        };

        // Whether the transaction runs at all is measurable for every accepted
        // observation: the replay produces a result either way.
        let mut subjects: Vec<EvaluableSubject> = subject(FindingDomain::Execution, "transaction")
            .into_iter()
            .collect();

        // A quantity is measurable when the account it is read from is part of
        // this observation. This is a property of the observation, not of
        // whether anything about it changed.
        let (Ok((op, _)), _) = (self.operation(transaction), ()) else {
            return subjects;
        };
        for (operation, name, required) in PROMOTED_SUBJECTS {
            if operation == op && accounts.iter().any(|named| named.label == required) {
                subjects.extend(subject(FindingDomain::Economic, name));
            }
        }
        subjects
    }

    fn decoded_source_of(&self, subject: &str) -> Option<(&'static str, &'static str)> {
        match subject {
            "pool_tokens_received" => Some(("destination-pool-token", "amount")),
            "pool_tokens_debited" => Some(("source-pool-token", "amount")),
            "pool_tokens_burned" => Some(("pool-mint", "supply")),
            "sol_received_by_user" => Some(("destination-lamports", "lamports")),
            _ => None,
        }
    }

    fn decoded_byte_ranges(&self, account_label: &str) -> &'static [std::ops::Range<usize>] {
        // Verified against the committed mainnet pool account: the u64 at 266
        // equals the mint's supply at 36, which is what fixes both offsets.
        const TOTAL_LAMPORTS: usize = 258;
        const POOL_TOKEN_SUPPLY: usize = 266;
        const _: () = assert!(POOL_TOKEN_SUPPLY == TOTAL_LAMPORTS + 8);
        // Exactly the fields this adapter reads, and nothing else. Everything
        // outside them is state it does not interpret, so a change there cannot
        // be explained away by an economic finding.
        const POOL: [std::ops::Range<usize>; 2] = [258..266, 266..274];
        const TOKEN_AMOUNT: [std::ops::Range<usize>; 1] = [std::ops::Range { start: 64, end: 72 }];
        const MINT_SUPPLY: [std::ops::Range<usize>; 1] = [std::ops::Range { start: 36, end: 44 }];
        match account_label {
            "stake-pool" => &POOL,
            "destination-pool-token" | "source-pool-token" | "manager-fee" => &TOKEN_AMOUNT,
            "pool-mint" => &MINT_SUPPLY,
            _ => &[],
        }
    }

    fn named_findings(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
        v1: &ExecutionResult,
        v2: &ExecutionResult,
    ) -> Vec<crate::semantics::NamedFinding> {
        use crate::semantics::{
            ChangeKind, FindingDomain, FindingFingerprint, NamedFinding, SemanticSubject,
            SemanticValue,
        };

        let (Some(protocol), Some(action)) = (self.protocol_id(), self.action_id(transaction))
        else {
            return Vec::new();
        };
        let print = |domain, name: &str, change| {
            Some(FindingFingerprint {
                protocol: protocol.clone(),
                action: action.clone(),
                domain,
                subject: SemanticSubject::new(name).ok()?,
                change,
            })
        };

        // Whether it ran at all comes first, and when it differs it is the
        // whole story. A candidate that rejects the instruction produced no
        // quantities, so reporting the user's shares as having "decreased to
        // zero" beside it would double-count one change as two and invite a
        // team to declare it twice.
        if v1.success != v2.success {
            let change = if v1.success {
                ChangeKind::NowReverts
            } else {
                ChangeKind::NowSucceeds
            };
            return print(FindingDomain::Execution, "transaction", change)
                .map(|fingerprint| NamedFinding {
                    fingerprint,
                    baseline: None,
                    candidate: None,
                    relative_delta_bps: None,
                    // Either direction is critical, matching how the generic
                    // layer already rates a changed outcome: a withdrawal that
                    // starts failing strands users, one that starts succeeding
                    // bypasses a guard.
                    severity: crate::diff::Severity::Critical,
                })
                .into_iter()
                .collect();
        }

        let before = self.summarize(accounts, v1);
        let after = self.summarize(accounts, v2);
        let Ok((op, _)) = self.operation(transaction) else {
            return Vec::new();
        };
        let mut findings = Vec::new();
        for (operation, name, _) in PROMOTED_SUBJECTS {
            if operation != op {
                continue;
            }
            let find = |fields: &[SemanticField]| {
                fields
                    .iter()
                    .find(|field| field.name == name)
                    .and_then(|field| field.value.as_quantity())
            };
            let (Some(baseline), Some(candidate)) = (find(&before), find(&after)) else {
                continue;
            };
            let Some(delta) = candidate.delta(baseline) else {
                continue;
            };
            if delta.base_units == 0 {
                continue;
            }
            let Some(fingerprint) = print(
                FindingDomain::Economic,
                name,
                ChangeKind::from_delta(delta.base_units),
            ) else {
                continue;
            };
            findings.push(NamedFinding {
                fingerprint,
                baseline: Some(SemanticValue::quantity(
                    baseline.base_units,
                    baseline.decimals,
                )),
                candidate: Some(SemanticValue::quantity(
                    candidate.base_units,
                    candidate.decimals,
                )),
                relative_delta_bps: None,
                // A quantity a user receives is a balance, and the generic
                // layer already rates a changed balance as High. Rating it
                // lower here because these bytes happen to sit in a token
                // account rather than in lamports would be the same
                // under-reading `--fail-on-critical` exists to correct.
                severity: crate::diff::Severity::High,
            });
        }
        findings
    }

    fn summarize(&self, accounts: &[NamedAccount], result: &ExecutionResult) -> Vec<SemanticField> {
        let decimals = self.pool_decimals(accounts);
        let before = |label: &str| accounts.iter().find(|named| named.label == label);
        let mut fields = Vec::new();

        // Every quantity below is read from where value actually landed rather
        // than from the instruction's stated amount: what the reserve and the
        // user's token account did is the transfer that happened, and a
        // candidate that computes a different number cannot hide behind the
        // argument it was handed.
        let lamports_moved = |label: &str| -> Option<(u64, u64)> {
            let opening = before(label)?;
            let closing = self.after(result, label)?;
            Some((opening.account.lamports, closing.lamports))
        };
        let tokens_moved = |label: &str| -> Option<(u64, u64)> {
            let opening = before(label)?;
            let closing = self.after(result, label)?;
            Some((
                token_account_amount(&opening.account.data).unwrap_or(0),
                token_account_amount(&closing.data).unwrap_or(0),
            ))
        };

        if let Some((opening, closing)) = lamports_moved("reserve-stake") {
            // The reserve gains on a deposit and loses on a withdrawal, so the
            // direction names the field rather than being asserted.
            let (name, amount) = if closing >= opening {
                ("sol_deposited", closing - opening)
            } else {
                ("sol_withdrawn", opening - closing)
            };
            fields.push(SemanticField {
                name: name.into(),
                value: FieldValue::quantity(amount, LAMPORT_DECIMALS),
                economic: true,
            });
        }
        // What the withdrawing user actually received, which is the reserve's
        // debit minus nothing: the destination is credited directly.
        if let Some((opening, closing)) = lamports_moved("destination-lamports") {
            fields.push(SemanticField {
                name: "sol_received_by_user".into(),
                value: FieldValue::quantity(closing.saturating_sub(opening), LAMPORT_DECIMALS),
                economic: true,
            });
        }
        if let Some((opening, closing)) = tokens_moved("destination-pool-token") {
            fields.push(SemanticField {
                name: "pool_tokens_received".into(),
                value: FieldValue::quantity(closing.saturating_sub(opening), decimals),
                economic: true,
            });
        }
        // What the holder's account actually lost. On a withdrawal this is the
        // burn *plus* any manager fee transferred out of the same account, so
        // it is not the burn and must not be named as one.
        if let Some((opening, closing)) = tokens_moved("source-pool-token") {
            fields.push(SemanticField {
                name: "pool_tokens_debited".into(),
                value: FieldValue::quantity(opening.saturating_sub(closing), decimals),
                economic: true,
            });
        }
        // The burn itself is the mint's supply decrease. Reporting the source
        // debit under this name counted transferred fees as burned tokens, and
        // an expectation written against supply semantics would have been
        // evaluated against a different quantity.
        if let Some(opening) = before("pool-mint").and_then(|m| mint_supply(&m.account.data)) {
            if let Some(closing) = self
                .after(result, "pool-mint")
                .and_then(|m| mint_supply(&m.data))
            {
                // Emitted whenever this is a withdrawal shape, including when
                // the burn is zero. Omitting a zero made the field absent, and
                // a comparison that skips an absent side reported "a candidate
                // that stopped burning entirely" as no change at all.
                if before("source-pool-token").is_some() {
                    fields.push(SemanticField {
                        name: "pool_tokens_burned".into(),
                        value: FieldValue::quantity(opening.saturating_sub(closing), decimals),
                        economic: true,
                    });
                }
            }
        }
        // Supply is the pool-wide counterpart: a withdrawal must destroy the
        // tokens it debited, less whatever became fee.
        if let (Some(opening), Some(closing)) =
            (before("pool-mint"), self.after(result, "pool-mint"))
        {
            let opened = mint_supply(&opening.account.data).unwrap_or(0);
            let closed = mint_supply(&closing.data).unwrap_or(0);
            let (name, amount) = if closed >= opened {
                ("pool_token_supply_minted", closed - opened)
            } else {
                ("pool_token_supply_burned", opened - closed)
            };
            fields.push(SemanticField {
                name: name.into(),
                value: FieldValue::quantity(amount, decimals),
                economic: true,
            });
        }
        if let (Some(opening), Some(closing)) =
            (before("manager-fee"), self.after(result, "manager-fee"))
        {
            let fee = token_account_amount(&closing.data)
                .unwrap_or(0)
                .saturating_sub(token_account_amount(&opening.account.data).unwrap_or(0));
            fields.push(SemanticField {
                name: "manager_fee_pool_tokens".into(),
                value: FieldValue::quantity(fee, decimals),
                economic: true,
            });
        }
        if let (Some(opening), Some(closing)) =
            (before("stake-pool"), self.after(result, "stake-pool"))
        {
            if let (Some(open), Some(close)) = (
                StakePool::decode(&opening.account.data),
                StakePool::decode(&closing.data),
            ) {
                fields.push(SemanticField {
                    name: "pool_tokens_minted".into(),
                    value: FieldValue::quantity(
                        close
                            .pool_token_supply
                            .saturating_sub(open.pool_token_supply),
                        decimals,
                    ),
                    economic: true,
                });
                fields.push(SemanticField {
                    name: "pool_token_supply_after".into(),
                    value: FieldValue::quantity(close.pool_token_supply, decimals),
                    economic: true,
                });
                fields.push(SemanticField {
                    name: "pool_total_lamports_after".into(),
                    value: FieldValue::quantity(close.total_lamports, LAMPORT_DECIMALS),
                    economic: true,
                });
                // Integer throughout: lamports per pool token, scaled by 10^9 so
                // the ratio renders at nine decimals without floating point.
                if close.pool_token_supply > 0 {
                    let scaled = u128::from(close.total_lamports).saturating_mul(1_000_000_000)
                        / u128::from(close.pool_token_supply);
                    fields.push(SemanticField {
                        name: "sol_per_pool_token".into(),
                        value: FieldValue::quantity(
                            u64::try_from(scaled).unwrap_or(u64::MAX),
                            LAMPORT_DECIMALS,
                        ),
                        economic: true,
                    });
                }
            }
        }
        fields
    }
}

fn basis_points(fee: Fee) -> u64 {
    if fee.denominator == 0 {
        return 0;
    }
    u64::try_from(
        u128::from(fee.numerator)
            .saturating_mul(10_000)
            .checked_div(u128::from(fee.denominator))
            .unwrap_or(0),
    )
    .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes of the JitoSOL pool account at slot 429,880,687, the predecessor of
    /// the slot this phase replays. Encoded as the fields the layout declares
    /// rather than pasted as a blob, so the test states what it is asserting.
    fn bs58_of(bytes: &[u8; 32]) -> String {
        solana_address::Address::new_from_array(*bytes).to_string()
    }

    fn pool_bytes(total_lamports: u64, pool_token_supply: u64) -> Vec<u8> {
        let mut data = vec![ACCOUNT_TYPE_STAKE_POOL];
        for _ in 0..3 {
            data.extend_from_slice(&[7_u8; 32]);
        }
        data.push(253);
        data.extend_from_slice(&[1_u8; 32]); // validator list
        data.extend_from_slice(&[2_u8; 32]); // reserve stake
        data.extend_from_slice(&[3_u8; 32]); // pool mint
        data.extend_from_slice(&[4_u8; 32]); // manager fee account
        data.extend_from_slice(&[5_u8; 32]); // token program
        data.extend_from_slice(&total_lamports.to_le_bytes());
        data.extend_from_slice(&pool_token_supply.to_le_bytes());
        data.extend_from_slice(&1035_u64.to_le_bytes()); // last update epoch
        data.extend_from_slice(&[0_u8; 48]); // lockup
        data.extend_from_slice(&100_u64.to_le_bytes()); // epoch fee denominator
        data.extend_from_slice(&4_u64.to_le_bytes()); // epoch fee numerator
        data.push(0); // next epoch fee: None
        data.push(0); // preferred deposit validator: None
        data.push(0); // preferred withdraw validator: None
        data.extend_from_slice(&[0_u8; 16]); // stake deposit fee
        data.extend_from_slice(&1000_u64.to_le_bytes()); // stake withdrawal denominator
        data.extend_from_slice(&1_u64.to_le_bytes()); // stake withdrawal numerator
        data.push(0); // next stake withdrawal fee: None
        data.push(0); // stake referral fee
        data.push(0); // sol deposit authority: None
        data.extend_from_slice(&[0_u8; 16]); // sol deposit fee
        data.push(0); // sol referral fee
        data.push(0); // sol withdraw authority: None
        data.extend_from_slice(&1000_u64.to_le_bytes()); // sol withdrawal denominator
        data.extend_from_slice(&1_u64.to_le_bytes()); // sol withdrawal numerator
        data.push(0); // next sol withdrawal fee: None
        data.extend_from_slice(&7_902_165_672_995_279_u64.to_le_bytes());
        data.extend_from_slice(&10_281_588_458_642_360_u64.to_le_bytes());
        // Stale bytes past the current encoding, as a real pool account carries.
        data.extend_from_slice(&[0_u8; 176]);
        data
    }

    // ---- Phase 9: WithdrawSol -------------------------------------------

    #[test]
    fn the_pool_layout_surfaces_the_withdrawal_side() {
        let pool = StakePool::decode(&pool_bytes(10_281_588_458_642_360, 7_902_165_672_995_279))
            .expect("pool decodes");
        // The supported shape requires an ungated withdrawal path, and the
        // pool's own bytes are what establish that rather than the account count.
        assert_eq!(pool.sol_withdraw_authority, None);
        assert_eq!(pool.sol_withdrawal_fee.numerator, 1);
        assert_eq!(pool.sol_withdrawal_fee.denominator, 1_000);
    }

    #[test]
    fn a_pool_gating_withdrawals_is_decoded_as_gated() {
        let mut data = pool_bytes(1_000, 1_000);
        // Flip `sol_withdraw_authority` from None to Some and shift the rest.
        let offset =
            1 + 32 * 3 + 1 + 32 * 5 + 8 * 3 + 48 + 16 + 1 + 1 + 1 + 16 + 16 + 1 + 1 + 1 + 16 + 1;
        assert_eq!(
            data[offset], 0,
            "sol_withdraw_authority should start as None"
        );
        data[offset] = 1;
        data.splice(offset + 1..offset + 1, [9_u8; 32]);
        let pool = StakePool::decode(&data).expect("pool decodes");
        assert_eq!(pool.sol_withdraw_authority, Some(bs58_of(&[9_u8; 32])));
    }

    #[test]
    fn withdrawal_reference_math_multiplies_before_dividing() {
        // A pool worth more than one lamport per token: 3 tokens against a
        // 10/7 exchange rate truncates differently if the divide comes first.
        let pool = StakePool::decode(&pool_bytes(10, 7)).expect("pool decodes");
        // 3 * 10 / 7 == 4, whereas (3 / 7) * 10 == 0.
        assert_eq!(pool.lamports_for_withdrawal(3), Some(4));
        // The round trip is the deposit path's inverse at the same rate.
        assert_eq!(pool.pool_tokens_for_deposit(4), Some(2));
    }

    #[test]
    fn an_empty_pool_withdraws_nothing() {
        let pool = StakePool::decode(&pool_bytes(0, 0)).expect("pool decodes");
        assert_eq!(pool.lamports_for_withdrawal(5), Some(0));
    }

    #[test]
    fn withdrawal_reference_math_survives_mainnet_scale() {
        // Real Jito-scale numbers: the product overflows u64 and must be taken
        // in u128, which is exactly what a narrowing candidate would break.
        let pool = StakePool::decode(&pool_bytes(10_281_588_458_642_360, 7_902_165_672_995_279))
            .expect("pool decodes");
        let lamports = pool
            .lamports_for_withdrawal(9_006_733_966)
            .expect("no overflow");
        let expected = (9_006_733_966_u128 * 10_281_588_458_642_360) / 7_902_165_672_995_279;
        assert_eq!(u128::from(lamports), expected);
        assert!(
            lamports > 9_006_733_966,
            "a staked pool is worth more than par"
        );
    }

    #[test]
    fn the_two_operations_have_distinct_shapes_and_meanings() {
        assert_eq!(
            PoolOp::from_discriminant(DEPOSIT_SOL),
            Some(PoolOp::DepositSol)
        );
        assert_eq!(
            PoolOp::from_discriminant(WITHDRAW_SOL),
            Some(PoolOp::WithdrawSol)
        );
        assert_eq!(
            PoolOp::from_discriminant(9),
            None,
            "DepositStake is out of contract"
        );

        let deposit = PoolOp::DepositSol;
        let withdraw = PoolOp::WithdrawSol;
        assert_eq!(deposit.roles().len(), DEPOSIT_SOL_ACCOUNTS);
        assert_eq!(withdraw.roles().len(), WITHDRAW_SOL_ACCOUNTS);
        assert_eq!(deposit.semantic_action(), SemanticAction::Deposit);
        assert_eq!(withdraw.semantic_action(), SemanticAction::Withdraw);

        // The signer differs: a depositor signs as the lamport source, while a
        // withdrawer signs as the pool-token transfer authority.
        assert_eq!(deposit.roles()[deposit.authority_position()], "depositor");
        assert_eq!(
            withdraw.roles()[withdraw.authority_position()],
            "user-transfer-authority"
        );
        // The economic entity is the pool-token holding either way.
        assert_eq!(
            deposit.roles()[deposit.entity_position()],
            "destination-pool-token"
        );
        assert_eq!(
            withdraw.roles()[withdraw.entity_position()],
            "source-pool-token"
        );

        // Each operation reaches a different pair of programs.
        assert_eq!(
            deposit.cpi_programs(),
            &[SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID]
        );
        assert_eq!(
            withdraw.cpi_programs(),
            &[TOKEN_PROGRAM_ID, STAKE_PROGRAM_ID]
        );
        // A withdrawal names the sysvars its Stake CPI reads.
        let declared: Vec<&str> = withdraw
            .declared_programs()
            .iter()
            .map(|(_, p)| *p)
            .collect();
        assert!(declared.contains(&CLOCK_SYSVAR_ID));
        assert!(declared.contains(&STAKE_HISTORY_SYSVAR_ID));
    }

    #[test]
    fn required_accounts_exclude_programs_and_sysvars() {
        // Programs and sysvars are not protocol state: another transaction
        // naming one writable changes nothing this record depends on, and
        // counting them would reject clean candidates.
        for op in [PoolOp::DepositSol, PoolOp::WithdrawSol] {
            for (_, program) in op.declared_programs() {
                assert!(
                    [
                        SYSTEM_PROGRAM_ID,
                        TOKEN_PROGRAM_ID,
                        STAKE_PROGRAM_ID,
                        CLOCK_SYSVAR_ID,
                        STAKE_HISTORY_SYSVAR_ID
                    ]
                    .contains(program),
                    "{program} is declared but not in the excluded set"
                );
            }
        }
    }

    #[test]
    fn the_pool_layout_decodes_past_its_variable_length_fields() {
        let pool = StakePool::decode(&pool_bytes(10_281_588_458_642_360, 7_902_165_672_995_279))
            .expect("pool decodes");
        assert_eq!(pool.total_lamports, 10_281_588_458_642_360);
        assert_eq!(pool.pool_token_supply, 7_902_165_672_995_279);
        assert_eq!(pool.last_update_epoch, 1035);
        assert_eq!(pool.epoch_fee.numerator, 4);
        assert_eq!(pool.epoch_fee.denominator, 100);
        assert_eq!(pool.sol_deposit_authority, None);
        assert_eq!(pool.sol_deposit_fee.denominator, 0);
        assert_eq!(pool.sol_referral_fee, 0);
    }

    /// Trailing bytes are ordinary: the program writes a shorter encoding over a
    /// fixed-size account and leaves whatever was there before.
    #[test]
    fn stale_trailing_bytes_do_not_break_decoding() {
        let mut data = pool_bytes(1, 1);
        data.extend_from_slice(&[0xAB; 64]);
        assert!(StakePool::decode(&data).is_some());
    }

    #[test]
    fn a_truncated_pool_does_not_decode() {
        let data = pool_bytes(1, 1);
        assert!(StakePool::decode(&data[..200]).is_none());
        assert!(StakePool::decode(&[]).is_none());
        assert!(StakePool::decode(&[0; 611]).is_none());
    }

    /// The arithmetic the deposit actually performed on mainnet, at the state
    /// this phase replays: 0.1 SOL into the JitoSOL pool.
    #[test]
    fn the_replayed_deposit_reproduces_the_observed_mint() {
        let pool = StakePool::decode(&pool_bytes(10_301_216_130_206_486, 7_922_046_141_432_616))
            .expect("pool decodes");
        // Multiplication before division, in u128.
        let expected = u128::from(100_000_000_u64) * u128::from(pool.pool_token_supply)
            / u128::from(pool.total_lamports);
        assert_eq!(
            pool.pool_tokens_for_deposit(100_000_000),
            Some(expected as u64)
        );
    }

    /// An empty pool mints one-for-one rather than dividing by zero.
    #[test]
    fn an_empty_pool_mints_one_for_one() {
        let pool = StakePool::decode(&pool_bytes(0, 0)).expect("pool decodes");
        assert_eq!(pool.pool_tokens_for_deposit(5_000), Some(5_000));
    }

    #[test]
    fn a_zero_denominator_fee_is_no_fee() {
        assert_eq!(
            Fee {
                numerator: 5,
                denominator: 0
            }
            .apply(1_000_000),
            Some(0)
        );
        assert_eq!(
            Fee {
                numerator: 1,
                denominator: 1000
            }
            .apply(1_000_000),
            Some(1_000)
        );
        assert_eq!(
            basis_points(Fee {
                numerator: 4,
                denominator: 100
            }),
            400
        );
        assert_eq!(
            basis_points(Fee {
                numerator: 0,
                denominator: 0
            }),
            0
        );
    }

    #[test]
    fn a_pool_account_decodes_into_economic_fields() {
        let account = AccountSnapshot {
            lamports: 2_060_816_388,
            owner: PROGRAM_ID.into(),
            data: pool_bytes(10_301_216_130_206_486, 7_922_046_141_432_616),
            executable: false,
            rent_epoch: 0,
        };
        let decoded = StakePoolAdapter.decode(&account).expect("decodes");
        assert_eq!(decoded.kind, "stake-pool");
        let supply = decoded.field("pool_token_supply").expect("supply present");
        assert!(supply.economic);
        assert_eq!(
            supply.value.as_quantity().unwrap().base_units,
            7_922_046_141_432_616
        );
        assert!(!decoded.field("last_update_epoch").unwrap().economic);
        assert_eq!(
            decoded
                .field("epoch_fee_basis_points")
                .unwrap()
                .value
                .render(),
            "400"
        );
    }

    /// An account this adapter does not own decodes to nothing rather than to a
    /// guess: a wrong decode would put an invented number in a report.
    #[test]
    fn a_foreign_account_does_not_decode() {
        let account = AccountSnapshot {
            lamports: 1,
            owner: "11111111111111111111111111111111".into(),
            data: vec![],
            executable: false,
            rent_epoch: 0,
        };
        assert!(StakePoolAdapter.decode(&account).is_none());
    }

    /// Lamport fields keep their own precision; pool-token fields are rescaled
    /// by the mint. Conflating the two would misreport both.
    #[test]
    fn only_pool_token_fields_are_rescaled() {
        let adapter = StakePoolAdapter;
        let lamports = TokenQuantity::new(100_000_000, LAMPORT_DECIMALS);
        assert_eq!(
            adapter.rescale("total_lamports", lamports, 6).decimals,
            LAMPORT_DECIMALS
        );
        assert_eq!(
            adapter
                .rescale("pool_token_supply", TokenQuantity::new(5, 0), 9)
                .decimals,
            9
        );
    }
}

/// Tests for the expectation surface: what an observation can measure, and
/// what a V1/V2 pair is named as having changed.
#[cfg(test)]
mod semantic_surface {
    use super::*;
    use crate::replay::ReplayRecord;
    use crate::semantics::{ChangeKind, FindingDomain};
    use std::collections::BTreeMap;

    const ADAPTER: StakePoolAdapter = StakePoolAdapter;

    fn deposit() -> ReplayRecord {
        serde_json::from_str(include_str!(
            "../../../docs/examples/mainnet-stake-pool-record.json"
        ))
        .expect("committed deposit record")
    }

    fn withdraw() -> ReplayRecord {
        serde_json::from_str(include_str!(
            "../../../docs/examples/mainnet-stake-pool-withdraw-record.json"
        ))
        .expect("committed withdraw record")
    }

    fn subjects(record: &ReplayRecord) -> Vec<String> {
        ADAPTER
            .evaluable_subjects(&record.transaction, &record.accounts)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// An execution result that leaves every account exactly as it started,
    /// then applies one edit. The baseline is the record's own pre-state, so
    /// the quantities under test are the ones the adapter really derives.
    fn result(
        record: &ReplayRecord,
        success: bool,
        edit: impl Fn(&str, &mut Vec<u8>),
    ) -> ExecutionResult {
        let mut accounts = BTreeMap::new();
        for named in &record.accounts {
            let mut data = named.account.data.clone();
            edit(&named.label, &mut data);
            accounts.insert(
                named.label.clone(),
                AccountSnapshot {
                    lamports: named.account.lamports,
                    owner: named.account.owner.clone(),
                    executable: named.account.executable,
                    rent_epoch: named.account.rent_epoch,
                    data,
                },
            );
        }
        ExecutionResult {
            version: "test".into(),
            success,
            error: (!success).then(|| "InstructionError(3, InvalidInstructionData)".to_string()),
            compute_units: Some(1),
            fee: 0,
            logs: Vec::new(),
            cpi_calls: Vec::new(),
            accounts,
        }
    }

    fn set_token_amount(data: &mut [u8], amount: u64) {
        if data.len() == TOKEN_ACCOUNT_LEN {
            data[64..72].copy_from_slice(&amount.to_le_bytes());
        }
    }

    fn findings(record: &ReplayRecord, v1: &ExecutionResult, v2: &ExecutionResult) -> Vec<String> {
        ADAPTER
            .named_findings(&record.transaction, &record.accounts, v1, v2)
            .into_iter()
            .map(|f| f.fingerprint.to_string())
            .collect()
    }

    #[test]
    fn a_deposit_can_measure_the_shares_it_produces() {
        let record = deposit();
        let subjects = subjects(&record);
        assert!(subjects.contains(&"spl-stake-pool/deposit_sol/execution/transaction".to_string()));
        assert!(subjects
            .contains(&"spl-stake-pool/deposit_sol/economic/pool_tokens_received".to_string()));
        // A deposit burns nothing and credits no withdrawal destination, so it
        // cannot speak about those at all.
        assert!(!subjects.iter().any(|s| s.contains("pool_tokens_burned")));
        assert!(!subjects.iter().any(|s| s.contains("sol_received_by_user")));
    }

    #[test]
    fn a_withdrawal_can_measure_what_it_burns_and_pays_out() {
        let record = withdraw();
        let subjects = subjects(&record);
        for expected in [
            "spl-stake-pool/withdraw_sol/execution/transaction",
            "spl-stake-pool/withdraw_sol/economic/pool_tokens_burned",
            "spl-stake-pool/withdraw_sol/economic/sol_received_by_user",
        ] {
            assert!(
                subjects.contains(&expected.to_string()),
                "missing {expected}"
            );
        }
        assert!(!subjects.iter().any(|s| s.contains("pool_tokens_received")));
    }

    /// The whole basis of the stale-versus-unevaluable split: capability comes
    /// from the observation's shape, so it is the same whether or not the two
    /// builds happened to differ.
    #[test]
    fn capability_does_not_depend_on_anything_changing() {
        let record = deposit();
        let unchanged = subjects(&record);
        assert!(!unchanged.is_empty());
        // The signature takes no execution results at all, which is what makes
        // this structural rather than a convention to be remembered.
        assert_eq!(unchanged, subjects(&deposit()));
    }

    /// No action id, no subjects. An observation the adapter cannot decode
    /// must not claim it can measure anything about it.
    #[test]
    fn an_action_outside_the_contract_names_no_subjects() {
        let mut record = deposit();
        for instruction in &mut record.transaction.instructions {
            if instruction.program == ADAPTER.program_id() {
                instruction.data = vec![0];
            }
        }
        assert!(ADAPTER.action_id(&record.transaction).is_none());
        assert!(subjects(&record).is_empty());
    }

    #[test]
    fn fewer_shares_for_the_same_deposit_is_named() {
        let record = deposit();
        let baseline = record
            .accounts
            .iter()
            .find(|a| a.label == "destination-pool-token")
            .and_then(|a| token_account_amount(&a.account.data))
            .expect("a pool token account");

        let v1 = result(&record, true, |label, data| {
            if label == "destination-pool-token" {
                set_token_amount(data, baseline + 1_000_000);
            }
        });
        let v2 = result(&record, true, |label, data| {
            if label == "destination-pool-token" {
                set_token_amount(data, baseline + 999_000);
            }
        });

        let named = ADAPTER.named_findings(&record.transaction, &record.accounts, &v1, &v2);
        assert_eq!(named.len(), 1, "{named:#?}");
        assert_eq!(
            named[0].fingerprint.to_string(),
            "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased"
        );
        assert_eq!(named[0].fingerprint.domain, FindingDomain::Economic);
        assert_eq!(named[0].fingerprint.change, ChangeKind::Decreased);
        assert!(named[0].baseline.is_some() && named[0].candidate.is_some());
        assert_eq!(named[0].severity, crate::diff::Severity::High);
    }

    #[test]
    fn an_unchanged_outcome_is_named_as_nothing() {
        let record = deposit();
        let v1 = result(&record, true, |_, _| {});
        let v2 = result(&record, true, |_, _| {});
        assert!(findings(&record, &v1, &v2).is_empty());
    }

    /// A candidate that rejects the instruction produced no quantities.
    /// Reporting the user's payout as having collapsed to zero beside the
    /// revert would count one change twice and invite two declarations for it.
    #[test]
    fn a_rejected_instruction_is_one_finding_not_several() {
        let record = withdraw();
        let v1 = result(&record, true, |_, _| {});
        let v2 = result(&record, false, |label, data| {
            // The candidate did nothing, so the burn never happened - exactly
            // the state that would look like a large economic change.
            if label == "source-pool-token" {
                set_token_amount(data, u64::MAX / 2);
            }
        });
        assert_eq!(
            findings(&record, &v1, &v2),
            ["spl-stake-pool/withdraw_sol/execution/transaction/now_reverts"]
        );
    }

    #[test]
    fn an_instruction_that_starts_succeeding_is_also_named() {
        let record = withdraw();
        let v1 = result(&record, false, |_, _| {});
        let v2 = result(&record, true, |_, _| {});
        assert_eq!(
            findings(&record, &v1, &v2),
            ["spl-stake-pool/withdraw_sol/execution/transaction/now_succeeds"]
        );
    }

    /// Every finding the adapter emits must be measurable by the observation
    /// that produced it, or the review engine would call a real finding
    /// unevaluable.
    #[test]
    fn every_emitted_finding_is_covered_by_a_declared_capability() {
        let record = deposit();
        let baseline = record
            .accounts
            .iter()
            .find(|a| a.label == "destination-pool-token")
            .and_then(|a| token_account_amount(&a.account.data))
            .expect("a pool token account");
        let v1 = result(&record, true, |label, data| {
            if label == "destination-pool-token" {
                set_token_amount(data, baseline + 10);
            }
        });
        let v2 = result(&record, true, |label, data| {
            if label == "destination-pool-token" {
                set_token_amount(data, baseline + 5);
            }
        });

        let capability: Vec<String> = subjects(&record);
        for finding in ADAPTER.named_findings(&record.transaction, &record.accounts, &v1, &v2) {
            let subject = finding.fingerprint.evaluable_subject().to_string();
            assert!(
                capability.contains(&subject),
                "emitted {subject} but the observation does not declare it measurable"
            );
        }
    }
}
