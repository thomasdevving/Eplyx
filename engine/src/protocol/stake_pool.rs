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
    EconomicChange, FieldValue, ProtocolAdapter, SemanticAccount, SemanticField, TokenQuantity,
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

/// `AccountType::StakePool`.
const ACCOUNT_TYPE_STAKE_POOL: u8 = 1;
/// Base SPL Token layouts, which the pool mint and every pool token account use.
const TOKEN_ACCOUNT_LEN: usize = 165;
const MINT_LEN: usize = 82;
/// `StakeStateV2`, which is what the reserve stake account holds.
const STAKE_STATE_LEN: usize = 200;

/// Roles, in the order `DepositSol` declares them.
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
        let _sol_withdraw_authority = reader.option_address()?;
        let _sol_withdrawal_fee = reader.fee()?;
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
            last_epoch_pool_token_supply,
            last_epoch_total_lamports,
        })
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
    /// The single `DepositSol` this record exists to replay.
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
        if let Ok(instruction) = self.deposit(transaction) {
            for (position, role) in DEPOSIT_SOL_ROLES.into_iter().enumerate() {
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

    fn accept(&self, transaction: &HistoricalTransaction) -> Result<()> {
        anyhow::ensure!(
            transaction.version == "legacy",
            "stake-pool replay supports legacy messages only; address lookup tables are \
             normalized but not executed"
        );
        anyhow::ensure!(
            transaction.success && transaction.error.is_none(),
            "replay selects successfully captured original transactions"
        );
        let deposit = self.deposit(transaction)?;
        let discriminant = *deposit
            .data
            .first()
            .context("empty SPL Stake Pool instruction data")?;
        anyhow::ensure!(
            discriminant == DEPOSIT_SOL,
            "stake-pool replay supports DepositSol only, found instruction variant {discriminant}"
        );
        anyhow::ensure!(
            deposit.data.len() == DEPOSIT_SOL_LEN,
            "DepositSol data must be the discriminant and a u64 lamport amount"
        );
        anyhow::ensure!(
            deposit.accounts.len() == DEPOSIT_SOL_ACCOUNTS,
            "DepositSol with {} accounts is outside the supported shape; a pool gated by a \
             SOL deposit authority passes that authority as an eleventh account and is not \
             supported",
            deposit.accounts.len()
        );
        anyhow::ensure!(
            deposit.accounts[3].is_signer,
            "the depositing lamport source must be a direct signer"
        );
        anyhow::ensure!(
            deposit.accounts[8].address == SYSTEM_PROGRAM_ID
                && deposit.accounts[9].address == TOKEN_PROGRAM_ID,
            "DepositSol must name the System program and the SPL Token program in their \
             declared positions"
        );
        anyhow::ensure!(
            self.deposit_lamports(transaction).is_some_and(|l| l > 0),
            "DepositSol must deposit a non-zero amount"
        );

        for (index, instruction) in transaction.instructions.iter().enumerate() {
            match instruction.program.as_str() {
                PROGRAM_ID | COMPUTE_BUDGET_PROGRAM_ID => {}
                SYSTEM_PROGRAM_ID => {
                    // Plain transfers only. Account creation and allocation are
                    // outside the contract, and both are System instructions.
                    anyhow::ensure!(
                        instruction.data.len() == SYSTEM_TRANSFER_LEN
                            && u32::from_le_bytes(
                                instruction.data[..4].try_into().expect("length checked")
                            ) == SYSTEM_TRANSFER
                            && instruction.accounts.len() == 2,
                        "top-level System instruction {index} is not a plain transfer; \
                         account creation and allocation are not supported"
                    );
                }
                ASSOCIATED_TOKEN_PROGRAM_ID => {
                    anyhow::ensure!(
                        instruction.data == [CREATE_IDEMPOTENT],
                        "the associated-token-account program is supported for \
                         CreateIdempotent only"
                    );
                    // Idempotent creation is only in scope when it provably had
                    // nothing to do: the validator recorded a token balance for
                    // the account on both sides, so it already existed.
                    let target = instruction
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

        // CPI is supported, but only into the two programs the deposit path is
        // defined to reach. Anything else means a different execution graph.
        for frame in &transaction.inner_instruction_frames {
            anyhow::ensure!(
                frame.program == SYSTEM_PROGRAM_ID || frame.program == TOKEN_PROGRAM_ID,
                "unsupported cross-program invocation into {} during a stake-pool replay",
                frame.program
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
        // Every account `DepositSol` declares, programs excluded: those are the
        // accounts the pool actually reads and writes.
        let Ok(deposit) = self.deposit(transaction) else {
            return Vec::new();
        };
        deposit
            .accounts
            .iter()
            .map(|meta| meta.address.clone())
            .filter(|address| address != SYSTEM_PROGRAM_ID && address != TOKEN_PROGRAM_ID)
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
            anyhow::ensure!(
                opening.sol_deposit_authority.is_none(),
                "this pool gates SOL deposits behind an authority, which the supported \
                 DepositSol shape does not carry"
            );

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
        assumptions.push(
            "the reserve stake account's delegation state is carried through unchanged: a SOL \
             deposit credits its lamports and does not invoke the stake program"
                .into(),
        );
        assumptions.push(
            "supported contract is one DepositSol into a pool with no SOL deposit authority, \
             with cross-program invocation into the System and SPL Token programs only"
                .into(),
        );
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

    fn summarize(&self, accounts: &[NamedAccount], result: &ExecutionResult) -> Vec<SemanticField> {
        let decimals = self.pool_decimals(accounts);
        let before = |label: &str| accounts.iter().find(|named| named.label == label);
        let mut fields = Vec::new();

        // What the depositor paid, read from where it landed rather than from
        // the instruction: the reserve's credit is the transfer that happened.
        if let (Some(opening), Some(closing)) =
            (before("reserve-stake"), self.after(result, "reserve-stake"))
        {
            fields.push(SemanticField {
                name: "sol_deposited".into(),
                value: FieldValue::quantity(
                    closing.lamports.saturating_sub(opening.account.lamports),
                    LAMPORT_DECIMALS,
                ),
                economic: true,
            });
        }
        if let (Some(opening), Some(closing)) = (
            before("destination-pool-token"),
            self.after(result, "destination-pool-token"),
        ) {
            let received = token_account_amount(&closing.data)
                .unwrap_or(0)
                .saturating_sub(token_account_amount(&opening.account.data).unwrap_or(0));
            fields.push(SemanticField {
                name: "pool_tokens_received".into(),
                value: FieldValue::quantity(received, decimals),
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
