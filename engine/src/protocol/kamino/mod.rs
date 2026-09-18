//! Kamino KLend adapter — a narrow vertical slice.
//!
//! Adapter number three, and the reason it exists is architectural: Phase U1
//! extracted the protocol-independent half of the first two adapters, and this
//! is the test of whether a new protocol can now consume that evidence without
//! rebuilding it. What this adapter does **not** contain is the measurement:
//! there is no SPL Token layout here, no pre/post pairing, no boundary-proof
//! machinery and no lamport arithmetic. See `docs/phase-u2-kamino.md`.
//!
//! ## The slice
//!
//! Two action families, chosen because they stress different things:
//!
//! - **deposit** (`depositReserveLiquidityAndObligationCollateral`, and its
//!   `V2` form) — almost entirely flow-observable. Liquidity leaves the user's
//!   token account, arrives in the reserve's supply vault, collateral is minted,
//!   and the obligation records it.
//! - **borrow** (`borrowObligationLiquidity`, and its `V2` form) — *not*
//!   flow-observable. Liquidity reaching the borrower is an ordinary token
//!   transfer, but the debt it creates is a scaled fraction inside a
//!   program-owned account, and two candidates can credit a borrower
//!   identically while recording different debts. That is the case the
//!   universal flow primitives cannot settle, and it is why this slice was
//!   chosen.
//!
//! Everything else KLend does is unsupported and says so.
//!
//! ## Semantics are not contained in one instruction
//!
//! Both supported actions require the reserve and the obligation to have been
//! refreshed **in the same transaction**: KLend rejects a stale reserve, and a
//! borrow against an unrefreshed obligation is a different computation. So
//! [`ProtocolAdapter::accept`] *requires* the prerequisites rather than merely
//! tolerating them. This is the multi-instruction shape the architecture study
//! predicted and the first two adapters never exercised — Stake Pool's action
//! is one instruction whose meaning is complete on its own.
//!
//! ## What is deliberately absent
//!
//! No health factor, no liquidation threshold, no oracle price normalisation,
//! no interest projection. The supported subjects are honest without them, and
//! an evaluable subject this adapter cannot compute would be a claim with no
//! implementation behind it.

pub mod fraction;
pub mod state;

use super::{
    BoundaryDistance, EconomicChange, EntityId, FieldValue, ProtocolAdapter, SemanticAccount,
    SemanticAction, SemanticField, StateFeature, TokenQuantity,
};
use crate::{
    evidence::{boundary, labels, pairing, token::TokenProgram},
    executor::ExecutionResult,
    ingest::transactions::HistoricalTransaction,
    types::{AccountSnapshot, InstructionSpec, NamedAccount},
};
use anyhow::{Context, Result};

pub const PROGRAM_ID: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";
pub const COMPUTE_BUDGET_PROGRAM_ID: &str = "ComputeBudget111111111111111111111111111111";
pub const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";
/// Kamino Farms. Reached by the `V2` forms when a reserve has farms attached.
pub const FARMS_PROGRAM_ID: &str = "FarmsPZpWu9i7Kky8tPN37rs2TpmMrAZrC7S7vJa91Hr";

// ---------------------------------------------------------------------------
// A. Interface and instruction decoding
// ---------------------------------------------------------------------------
//
// Anchor discriminators: the first eight bytes of sha256("global:<snake_name>").
// Recognition is by discriminator and account arity, never by a human-readable
// name — a name is a property of an interface document, and the interface is
// published by the party whose change this tool measures.

const DEPOSIT_V1: [u8; 8] = [129, 199, 4, 2, 222, 39, 26, 46];
const DEPOSIT_V2: [u8; 8] = [216, 224, 191, 27, 204, 151, 102, 175];
const BORROW_V1: [u8; 8] = [121, 127, 18, 204, 73, 245, 225, 65];
const BORROW_V2: [u8; 8] = [161, 128, 143, 245, 171, 199, 194, 6];
const REFRESH_RESERVE: [u8; 8] = [2, 218, 138, 235, 79, 201, 25, 102];
const REFRESH_OBLIGATION: [u8; 8] = [33, 132, 147, 228, 151, 192, 72, 89];
const REFRESH_OBLIGATION_FARMS: [u8; 8] = [140, 144, 253, 21, 10, 74, 248, 3];

/// Discriminator plus one `u64 liquidityAmount`.
const ACTION_DATA_LEN: usize = 16;
const REFRESH_RESERVE_ACCOUNTS: usize = 6;
/// `refreshObligation` declares two accounts in the IDL and carries **more** in
/// production: Anchor appends the obligation's reserves as `remaining_accounts`,
/// so a real one names three or more. Measured against mainnet — every sampled
/// production refresh carried three. The two declared positions are checked;
/// the rest are the obligation's own reserves and are not this adapter's to
/// constrain.
const REFRESH_OBLIGATION_MIN_ACCOUNTS: usize = 2;

/// Account roles for `depositReserveLiquidityAndObligationCollateral`.
///
/// KLend's own names, kebab-cased. Local to this adapter and never added to a
/// shared enum: `reserve-destination-deposit-collateral` is meaningful here and
/// meaningless for a stake pool.
const DEPOSIT_ROLES: [&str; 14] = [
    "owner",
    "obligation",
    "lending-market",
    "lending-market-authority",
    "reserve",
    "reserve-liquidity-mint",
    "reserve-liquidity-supply",
    "reserve-collateral-mint",
    "reserve-destination-deposit-collateral",
    "user-source-liquidity",
    "placeholder-user-destination-collateral",
    "collateral-token-program",
    "liquidity-token-program",
    "instruction-sysvar",
];

/// Account roles for `borrowObligationLiquidity`.
const BORROW_ROLES: [&str; 12] = [
    "owner",
    "obligation",
    "lending-market",
    "lending-market-authority",
    "borrow-reserve",
    "borrow-reserve-liquidity-mint",
    "reserve-source-liquidity",
    "borrow-reserve-liquidity-fee-receiver",
    "user-destination-liquidity",
    "referrer-token-state",
    "token-program",
    "instruction-sysvar",
];

/// The three accounts a `V2` form appends. Same economics, extra bookkeeping.
const FARMS_ROLES: [&str; 3] = [
    "obligation-farm-user-state",
    "reserve-farm-state",
    "farms-program",
];

/// The supported KLend instructions.
///
/// `V1` and `V2` of one family share an [`KlendOp::action_id`] because they are
/// the same economic action: `V2`'s account list is `V1`'s with three farms
/// accounts appended, and every economic account sits at the same position. A
/// team declaring an expectation about borrowing means both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KlendOp {
    DepositV1,
    DepositV2,
    BorrowV1,
    BorrowV2,
}

impl KlendOp {
    fn from_discriminator(data: &[u8]) -> Option<Self> {
        let tag: [u8; 8] = data.get(..8)?.try_into().ok()?;
        Some(match tag {
            DEPOSIT_V1 => Self::DepositV1,
            DEPOSIT_V2 => Self::DepositV2,
            BORROW_V1 => Self::BorrowV1,
            BORROW_V2 => Self::BorrowV2,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::DepositV1 => "depositReserveLiquidityAndObligationCollateral",
            Self::DepositV2 => "depositReserveLiquidityAndObligationCollateralV2",
            Self::BorrowV1 => "borrowObligationLiquidity",
            Self::BorrowV2 => "borrowObligationLiquidityV2",
        }
    }

    fn is_deposit(self) -> bool {
        matches!(self, Self::DepositV1 | Self::DepositV2)
    }

    fn has_farms(self) -> bool {
        matches!(self, Self::DepositV2 | Self::BorrowV2)
    }

    /// Exact account count. An instruction with any other arity is a shape this
    /// adapter has not proved it can replay.
    fn account_count(self) -> usize {
        let base = if self.is_deposit() {
            DEPOSIT_ROLES.len()
        } else {
            BORROW_ROLES.len()
        };
        base + if self.has_farms() {
            FARMS_ROLES.len()
        } else {
            0
        }
    }

    fn role(self, position: usize) -> Option<&'static str> {
        let base: &[&str] = if self.is_deposit() {
            &DEPOSIT_ROLES
        } else {
            &BORROW_ROLES
        };
        base.get(position).copied().or_else(|| {
            self.has_farms()
                .then(|| FARMS_ROLES.get(position - base.len()).copied())
                .flatten()
        })
    }

    fn roles(self) -> Vec<&'static str> {
        (0..self.account_count())
            .filter_map(|position| self.role(position))
            .collect()
    }

    /// The exact action in the finding vocabulary.
    fn action_id(self) -> &'static str {
        if self.is_deposit() {
            "deposit_reserve_liquidity_and_obligation_collateral"
        } else {
            "borrow_obligation_liquidity"
        }
    }

    /// Coarse classification, for corpus stratification only.
    ///
    /// A deposit is a deposit: an asset goes in and a position is credited.
    ///
    /// A **borrow is not in the vocabulary**, and is reported as
    /// [`SemanticAction::Unknown`] rather than squeezed into `Withdraw`. Value
    /// leaves the protocol toward the user, which looks like a withdrawal and
    /// is the opposite economically: the user's net position falls rather than
    /// rising, and a corpus selector stratifying the two together would sample
    /// them as though they were one thing. The exact identity stays in
    /// [`KlendOp::action_id`], which is what expectations key on. This pressure
    /// is recorded rather than resolved — adding an enum variant with no second
    /// protocol behind it is a claim with no test.
    fn semantic_action(self) -> SemanticAction {
        if self.is_deposit() {
            SemanticAction::Deposit
        } else {
            SemanticAction::Unknown
        }
    }

    /// Programs this operation's execution is defined to reach by CPI.
    fn cpi_programs(self) -> &'static [&'static str] {
        // Both move tokens; a deposit also mints collateral. The `V2` forms
        // additionally call Farms. Token-2022 appears because a reserve names
        // its own token program and some KLend markets use it.
        if self.has_farms() {
            &[
                SPL_TOKEN_PROGRAM_ID,
                TOKEN_2022_PROGRAM_ID,
                FARMS_PROGRAM_ID,
                SYSTEM_PROGRAM_ID,
            ]
        } else {
            &[
                SPL_TOKEN_PROGRAM_ID,
                TOKEN_2022_PROGRAM_ID,
                SYSTEM_PROGRAM_ID,
            ]
        }
    }
}

// ---------------------------------------------------------------------------
// The promoted semantic surface
// ---------------------------------------------------------------------------

/// How a subject is measured. Kept explicit because the categories must not
/// blur: a quantity read straight from a universal primitive and one computed
/// by protocol math carry different amounts of trust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measurement {
    /// Read from a universal evidence primitive — a token balance delta, a mint
    /// supply delta. No protocol math involved.
    DirectUniversal,
    /// Read from a typed field in a protocol-owned account, at a known offset,
    /// with no arithmetic beyond the read.
    DirectTypedField,
    /// Computed by a named Kamino evaluator from protocol state. See
    /// [`fraction`].
    DerivedKamino,
}

/// `(deposit?, subject, the account that makes it measurable, how)`.
///
/// Promoted subjects are a compatibility commitment: the moment a team names
/// one in their TOML it has to keep meaning the same thing. Short on purpose.
const PROMOTED: [(bool, &str, &str, Measurement); 7] = [
    (
        true,
        "liquidity_deposited",
        "user-source-liquidity",
        Measurement::DirectUniversal,
    ),
    (
        true,
        "reserve_liquidity_received",
        "reserve-liquidity-supply",
        Measurement::DirectUniversal,
    ),
    (
        true,
        "obligation_collateral_deposited",
        "obligation",
        Measurement::DirectTypedField,
    ),
    (
        false,
        "liquidity_borrowed",
        "user-destination-liquidity",
        Measurement::DirectUniversal,
    ),
    (
        false,
        "reserve_liquidity_drawn",
        "reserve-source-liquidity",
        Measurement::DirectUniversal,
    ),
    (
        false,
        "origination_fee",
        "borrow-reserve-liquidity-fee-receiver",
        Measurement::DirectUniversal,
    ),
    (
        false,
        "debt_increased",
        "obligation",
        Measurement::DerivedKamino,
    ),
];

/// Decoded obligation fields a debt subject may read.
///
/// One per borrow slot, because the array position the action touches is a
/// property of the observation. See [`ProtocolAdapter::decoded_sources_of`].
const OBLIGATION_BORROW_FIELDS: [(&str, &str); state::BORROW_SLOTS] = [
    ("obligation", "borrow_0_amount_sf"),
    ("obligation", "borrow_1_amount_sf"),
    ("obligation", "borrow_2_amount_sf"),
    ("obligation", "borrow_3_amount_sf"),
    ("obligation", "borrow_4_amount_sf"),
];

const OBLIGATION_DEPOSIT_FIELDS: [(&str, &str); state::DEPOSIT_SLOTS] = [
    ("obligation", "deposit_0_amount"),
    ("obligation", "deposit_1_amount"),
    ("obligation", "deposit_2_amount"),
    ("obligation", "deposit_3_amount"),
    ("obligation", "deposit_4_amount"),
    ("obligation", "deposit_5_amount"),
    ("obligation", "deposit_6_amount"),
    ("obligation", "deposit_7_amount"),
];

/// Whether instruction data names one of the two supported action families.
///
/// Exposed so a test can pin the discriminators to Anchor's own derivation
/// rather than to the constants above, which would be a tautology.
pub fn recognises(data: &[u8]) -> bool {
    KlendOp::from_discriminator(data).is_some()
}

pub struct KaminoKlendAdapter;

// ---------------------------------------------------------------------------
// B. Account-role binding
// ---------------------------------------------------------------------------

impl KaminoKlendAdapter {
    /// The single supported KLend action instruction in this transaction.
    ///
    /// Refresh instructions are prerequisites, not actions: a transaction whose
    /// only KLend instructions are refreshes has no action to interpret.
    fn operation<'a>(
        &self,
        transaction: &'a HistoricalTransaction,
    ) -> Result<(KlendOp, &'a InstructionSpec)> {
        let mut found = transaction
            .instructions
            .iter()
            .filter(|instruction| instruction.program == PROGRAM_ID)
            .filter_map(|instruction| {
                KlendOp::from_discriminator(&instruction.data).map(|op| (op, instruction))
            });
        let first = found
            .next()
            .context("transaction contains no supported Kamino KLend action")?;
        anyhow::ensure!(
            found.next().is_none(),
            "Kamino replay supports one KLend action per transaction"
        );
        Ok(first)
    }

    /// Address bound to `role` in the supported action.
    fn role_address(&self, transaction: &HistoricalTransaction, role: &str) -> Option<String> {
        let (op, instruction) = self.operation(transaction).ok()?;
        let position = op.roles().iter().position(|candidate| *candidate == role)?;
        Some(instruction.accounts.get(position)?.address.clone())
    }

    /// Base-unit amount the instruction names. Not the amount that moved —
    /// that is read from where value actually landed.
    fn requested_amount(&self, transaction: &HistoricalTransaction) -> Option<u64> {
        let (_, instruction) = self.operation(transaction).ok()?;
        crate::standard_programs::u64_at(&instruction.data, 8)
    }

    /// KLend instructions in the message, by discriminator, in order.
    fn klend_discriminators<'a>(
        &self,
        transaction: &'a HistoricalTransaction,
    ) -> Vec<(&'a InstructionSpec, Option<[u8; 8]>)> {
        transaction
            .instructions
            .iter()
            .filter(|instruction| instruction.program == PROGRAM_ID)
            .map(|instruction| {
                let tag = instruction.data.get(..8).and_then(|b| b.try_into().ok());
                (instruction, tag)
            })
            .collect()
    }

    fn account<'a>(&self, accounts: &'a [NamedAccount], label: &str) -> Option<&'a NamedAccount> {
        accounts.iter().find(|named| named.label == label)
    }

    /// The reserve this action names, decoded from its own bytes.
    fn reserve_of(&self, accounts: &[NamedAccount], label: &str) -> Option<state::Reserve> {
        state::decode_reserve(&self.account(accounts, label)?.account.data).ok()
    }

    fn obligation_of(&self, accounts: &[NamedAccount]) -> Option<state::Obligation> {
        state::decode_obligation(&self.account(accounts, "obligation")?.account.data).ok()
    }

    /// Which reserve label this op's action reads.
    fn reserve_label(op: KlendOp) -> &'static str {
        if op.is_deposit() {
            "reserve"
        } else {
            "borrow-reserve"
        }
    }

    // -----------------------------------------------------------------------
    // C. Universal evidence plumbing
    // -----------------------------------------------------------------------
    //
    // Every token quantity below is read through `evidence::token::TokenProgram`,
    // which dispatches to the shared SPL Token or Token-2022 decoder. This
    // adapter contains no token layout of its own, by construction: a KLend
    // reserve names its token program in its own state, and both appear in
    // production.

    /// Token balance of a labelled account, under whichever token program owns
    /// it. `None` when the account is not a token account at all.
    fn token_balance(&self, snapshot: &AccountSnapshot) -> Option<u64> {
        TokenProgram::of(&snapshot.owner)?.account_amount(&snapshot.data)
    }

    fn mint_supply(&self, snapshot: &AccountSnapshot) -> Option<u64> {
        TokenProgram::of(&snapshot.owner)?.mint_supply(&snapshot.data)
    }

    /// Decimals of the liquidity mint this action moves, for presentation only.
    fn liquidity_decimals(&self, accounts: &[NamedAccount], op: KlendOp) -> u8 {
        self.reserve_of(accounts, Self::reserve_label(op))
            .map(|reserve| reserve.mint_decimals)
            .unwrap_or(0)
    }

    /// Token movement of a labelled account across one execution.
    fn token_moved(
        &self,
        accounts: &[NamedAccount],
        result: &ExecutionResult,
        label: &str,
    ) -> Option<(u64, u64)> {
        let opening = self.account(accounts, label)?;
        let closing = result.accounts.get(label)?;
        Some((
            self.token_balance(&opening.account)?,
            self.token_balance(closing)?,
        ))
    }
}

// ---------------------------------------------------------------------------
// D/E/F. Invariants, protocol math, semantics
// ---------------------------------------------------------------------------

impl ProtocolAdapter for KaminoKlendAdapter {
    fn name(&self) -> &'static str {
        "kamino-klend"
    }

    fn program_id(&self) -> &'static str {
        PROGRAM_ID
    }

    fn adapter_version(&self) -> u32 {
        1
    }

    fn supports_cpi(&self) -> bool {
        true
    }

    fn dependency_programs(&self) -> &'static [&'static str] {
        // A reserve names its own token program, and KLend markets use both.
        // Declaring them means they are resolved at the historical slot and
        // recorded even if a validator truncated inner-instruction metadata.
        &[
            SPL_TOKEN_PROGRAM_ID,
            TOKEN_2022_PROGRAM_ID,
            SYSTEM_PROGRAM_ID,
        ]
    }

    fn semantic_action(&self, transaction: &HistoricalTransaction) -> SemanticAction {
        self.operation(transaction)
            .map(|(op, _)| op.semantic_action())
            .unwrap_or(SemanticAction::Unknown)
    }

    /// The obligation: one borrower's position in one market.
    ///
    /// Not the owner wallet, which conflates a person's several obligations,
    /// and not the reserve, which collapses every borrower into one entity.
    fn economic_entity_id(
        &self,
        transaction: &HistoricalTransaction,
        _accounts: &[NamedAccount],
    ) -> Option<EntityId> {
        Some(EntityId::new(
            "kamino-obligation",
            self.role_address(transaction, "obligation")?,
        ))
    }

    fn label(&self, transaction: &HistoricalTransaction, index: usize) -> String {
        labels::at(&self.labels(transaction), index)
    }

    /// Reject anything outside the contract this adapter replays exactly.
    ///
    /// The prerequisite refreshes are **required**, not tolerated. KLend refuses
    /// a stale reserve outright, and a borrow computed against an unrefreshed
    /// obligation is a different computation from the one that ran. Admitting a
    /// record without them would put an unproved interpretation under an
    /// exactness claim.
    fn accept_instruction_contract(&self, transaction: &HistoricalTransaction) -> Result<()> {
        anyhow::ensure!(
            transaction.success && transaction.error.is_none(),
            "replay selects successfully captured original transactions"
        );

        let (op, instruction) = self.operation(transaction)?;
        anyhow::ensure!(
            instruction.data.len() == ACTION_DATA_LEN,
            "{} data must be an 8-byte discriminator and a u64 amount, found {} bytes",
            op.name(),
            instruction.data.len()
        );
        anyhow::ensure!(
            instruction.accounts.len() == op.account_count(),
            "{} with {} accounts is outside the supported shape; {} are declared",
            op.name(),
            instruction.accounts.len(),
            op.account_count()
        );
        anyhow::ensure!(
            instruction.accounts[0].is_signer,
            "the obligation owner must be a direct signer"
        );
        anyhow::ensure!(
            self.requested_amount(transaction).is_some_and(|a| a > 0),
            "{} must name a non-zero amount",
            op.name()
        );

        // The prerequisite contract. Both are required, and both must name the
        // accounts this action names - a refresh of some other reserve leaves
        // this one stale.
        let mut refreshed_reserve = false;
        let mut refreshed_obligation = false;
        // Both families declare their accounts in the same order for the first
        // five positions, which is what lets one prerequisite check serve both.
        const OBLIGATION: usize = 1;
        const MARKET: usize = 2;
        const RESERVE: usize = 4;
        const _: () = assert!(RESERVE == 4);
        let action_reserve = &instruction.accounts[RESERVE].address;
        let action_obligation = &instruction.accounts[OBLIGATION].address;
        let action_market = &instruction.accounts[MARKET].address;

        for (companion, tag) in self.klend_discriminators(transaction) {
            match tag {
                Some(REFRESH_RESERVE) => {
                    anyhow::ensure!(
                        companion.accounts.len() == REFRESH_RESERVE_ACCOUNTS,
                        "refreshReserve with {} accounts is outside the supported shape",
                        companion.accounts.len()
                    );
                    if &companion.accounts[0].address == action_reserve {
                        anyhow::ensure!(
                            &companion.accounts[1].address == action_market,
                            "refreshReserve names a different lending market than the action"
                        );
                        refreshed_reserve = true;
                    }
                }
                Some(REFRESH_OBLIGATION) => {
                    anyhow::ensure!(
                        companion.accounts.len() >= REFRESH_OBLIGATION_MIN_ACCOUNTS,
                        "refreshObligation with {} accounts names neither a market nor an \
                         obligation",
                        companion.accounts.len()
                    );
                    if &companion.accounts[1].address == action_obligation {
                        anyhow::ensure!(
                            &companion.accounts[0].address == action_market,
                            "refreshObligation names a different lending market than the action"
                        );
                        refreshed_obligation = true;
                    }
                }
                // Farm refreshes are bookkeeping and carry no economics here.
                Some(REFRESH_OBLIGATION_FARMS) => {}
                Some(tag) if Some(op) == KlendOp::from_discriminator(&tag) => {}
                _ => anyhow::bail!(
                    "unsupported KLend instruction alongside {}; the supported contract is one \
                     action preceded by refreshReserve and refreshObligation",
                    op.name()
                ),
            }
        }
        anyhow::ensure!(
            refreshed_reserve,
            "{} requires refreshReserve for {action_reserve} in the same transaction; KLend \
             refuses a stale reserve and the replayed computation would not be the one that ran",
            op.name()
        );
        anyhow::ensure!(
            refreshed_obligation,
            "{} requires refreshObligation for {action_obligation} in the same transaction",
            op.name()
        );

        // Companion programs outside KLend.
        for companion in &transaction.instructions {
            match companion.program.as_str() {
                PROGRAM_ID | COMPUTE_BUDGET_PROGRAM_ID => {}
                other => anyhow::bail!(
                    "unsupported program {other} in a Kamino KLend replay; the supported \
                     contract admits KLend and compute-budget instructions only"
                ),
            }
        }

        // CPI is supported, into the programs this operation is defined to
        // reach and one level deep.
        for frame in &transaction.inner_instruction_frames {
            anyhow::ensure!(
                op.cpi_programs().contains(&frame.program.as_str()),
                "unsupported cross-program invocation into {} during a {} replay",
                frame.program,
                op.name()
            );
            anyhow::ensure!(
                frame.stack_height == 2,
                "Kamino replay supports one level of cross-program invocation; observed depth {}",
                frame.stack_height
            );
        }
        anyhow::ensure!(
            transaction.inner_instructions.len() == transaction.inner_instruction_frames.len(),
            "inner-instruction metadata is incomplete; the invocation graph cannot be checked"
        );
        anyhow::ensure!(
            transaction.pre_token_balances.is_some() && transaction.post_token_balances.is_some(),
            "Kamino replay requires validator-observed token balances as boundary evidence"
        );
        Ok(())
    }

    /// Accounts the protocol knows this transaction depends on.
    ///
    /// Includes every account the prerequisite refreshes name — the oracle
    /// accounts among them. A borrow's numbers depend on the price
    /// `refreshReserve` wrote, so the oracle state at `S-1` is part of the
    /// record. Nothing is fetched live at replay time; these are acquired at the
    /// historical boundary like any other account.
    fn required_accounts(&self, transaction: &HistoricalTransaction) -> Vec<String> {
        let mut required: Vec<String> = Vec::new();
        let programs = [
            PROGRAM_ID,
            SPL_TOKEN_PROGRAM_ID,
            TOKEN_2022_PROGRAM_ID,
            SYSTEM_PROGRAM_ID,
            FARMS_PROGRAM_ID,
            COMPUTE_BUDGET_PROGRAM_ID,
            crate::protocol::stake_pool::CLOCK_SYSVAR_ID,
            INSTRUCTION_SYSVAR_ID,
        ];
        for (instruction, tag) in self.klend_discriminators(transaction) {
            let interesting = matches!(tag, Some(REFRESH_RESERVE) | Some(REFRESH_OBLIGATION))
                || KlendOp::from_discriminator(&instruction.data).is_some();
            if !interesting {
                continue;
            }
            for meta in &instruction.accounts {
                if programs.contains(&meta.address.as_str()) {
                    continue;
                }
                if !required.contains(&meta.address) {
                    required.push(meta.address.clone());
                }
            }
        }
        required
    }

    fn decode(&self, account: &AccountSnapshot) -> Option<SemanticAccount> {
        // Protocol-owned state first.
        if account.owner == PROGRAM_ID {
            if let Some(reserve) = state::decode_reserve(&account.data).ok() {
                return Some(self.decode_reserve_fields(&reserve));
            }
            if let Some(obligation) = state::decode_obligation(&account.data).ok() {
                return Some(self.decode_obligation_fields(&obligation));
            }
            if state::is_lending_market(&account.data) {
                // Recognised and deliberately not interpreted: the supported
                // actions read no field out of it, so decoding one would claim
                // a layout this phase never exercises.
                return Some(SemanticAccount {
                    kind: "kamino-lending-market".into(),
                    fields: Vec::new(),
                });
            }
            return None;
        }

        // Token accounts and mints, read by the shared standard-program
        // decoders. This adapter parses no token layout of its own.
        let program = TokenProgram::of(&account.owner)?;
        if let Some(amount) = program.account_amount(&account.data) {
            let mint = program.account_mint(&account.data)?;
            return Some(SemanticAccount {
                kind: "token-account".into(),
                fields: vec![
                    SemanticField {
                        name: "mint".into(),
                        value: FieldValue::Address(mint),
                        economic: false,
                    },
                    SemanticField {
                        // Decimals belong to the mint, which an individual
                        // account does not carry; the interpretation layer
                        // rescales once the reserve is known.
                        name: "amount".into(),
                        value: FieldValue::quantity(amount, 0),
                        economic: true,
                    },
                ],
            });
        }
        let supply = program.mint_supply(&account.data)?;
        let decimals = program.mint_decimals(&account.data)?;
        Some(SemanticAccount {
            kind: "mint".into(),
            fields: vec![
                SemanticField {
                    name: "supply".into(),
                    value: FieldValue::quantity(supply, 0),
                    economic: true,
                },
                SemanticField {
                    name: "decimals".into(),
                    value: FieldValue::Count(u64::from(decimals)),
                    economic: false,
                },
            ],
        })
    }

    fn prove_boundaries(
        &self,
        transaction: &HistoricalTransaction,
        pre: &[NamedAccount],
        post: &[NamedAccount],
    ) -> Result<Vec<String>> {
        // A KLend market names its own token program, and both appear in
        // production. The boundary contract therefore has to accept either,
        // which the shared prover expresses as the programs it will attribute a
        // validator-observed balance to.
        let contract = boundary::BoundaryContract {
            token_program: SPL_TOKEN_PROGRAM_ID,
            token_program_description: "the SPL Token program",
            token_account_description: "token account",
            account_amount: any_token_program_amount,
            account_mint: any_token_program_mint,
            read_only_exempt: &[
                crate::protocol::stake_pool::CLOCK_SYSVAR_ID,
                INSTRUCTION_SYSVAR_ID,
            ],
            interference_hint: |side, slot| {
                format!(
                    "Another transaction in slot {slot} wrote this account {} this one.",
                    if side == boundary::Side::Pre {
                        "before"
                    } else {
                        "after"
                    }
                )
            },
        };
        let proof = boundary::prove(&contract, transaction, pre, post)?;

        let mut assumptions = proof.assumptions;
        let (op, _) = self.operation(transaction)?;

        // Protocol corroboration: validator metadata says nothing about a
        // reserve's or an obligation's bytes, so they would otherwise rest on
        // the archive alone. Each is checked against something the validator did
        // observe.
        let reserve_label = Self::reserve_label(op);
        if let (Some(before), Some(after)) = (
            self.reserve_of(pre, reserve_label),
            self.reserve_of(post, reserve_label),
        ) {
            let vault = before.liquidity_supply_vault.clone();
            let observed = |side: &[NamedAccount]| -> Option<i128> {
                let named = side.iter().find(|n| n.address == vault)?;
                Some(i128::from(self.token_balance(&named.account)?))
            };
            if let (Some(opening), Some(closing)) = (observed(pre), observed(post)) {
                let observed_change = closing - opening;
                let recorded_change =
                    i128::from(after.available_amount) - i128::from(before.available_amount);
                anyhow::ensure!(
                    observed_change == recorded_change,
                    "the reserve records a {recorded_change} base-unit change in available \
                     liquidity while the validator-observed supply vault moved by \
                     {observed_change}; the archived reserve state is not this transaction's \
                     boundary"
                );
                assumptions.push(
                    "the reserve's available-liquidity change equals the observed balance change \
                     of the supply vault it names"
                        .into(),
                );
            }
            anyhow::ensure!(
                before.lending_market == after.lending_market,
                "the reserve changed lending market across the boundary"
            );
        }

        if let (Some(before), Some(after)) = (self.obligation_of(pre), self.obligation_of(post)) {
            anyhow::ensure!(
                before.lending_market == after.lending_market && before.owner == after.owner,
                "the obligation changed market or owner across the boundary"
            );
            assumptions
                .push("the obligation's market and owner are unchanged across the boundary".into());
        }

        assumptions.push(
            "reserve and obligation bytes rest on the archive and on V1 reproducing the original \
             post-state; validator metadata records no protocol account data"
                .into(),
        );
        assumptions.push(format!(
            "supported contract is one {} preceded by refreshReserve and refreshObligation for \
             the accounts it names, with cross-program invocation one level deep into the token \
             programs the reserve names{}",
            op.name(),
            if op.has_farms() {
                " and Kamino Farms"
            } else {
                ""
            }
        ));
        assumptions.push(
            "oracle prices are whatever refreshReserve wrote from the oracle accounts acquired \
             at the historical boundary; no price is fetched at replay time"
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
        let decimals = self
            .operation_from_accounts(accounts)
            .map(|op| self.liquidity_decimals(accounts, op))
            .unwrap_or(0);
        pairing::compare_decoded(
            accounts,
            v1,
            v2,
            |account| self.decode(account),
            move |field, quantity| {
                // Liquidity amounts render against the reserve's mint;
                // collateral is denominated in collateral tokens and scaled
                // values are exact integers that must not be rescaled at all.
                if field == "amount" || field == "available_amount" {
                    TokenQuantity::new(quantity.base_units, decimals)
                } else {
                    quantity
                }
            },
        )
    }

    fn protocol_id(&self) -> Option<crate::semantics::ProtocolId> {
        crate::semantics::ProtocolId::new("kamino-klend").ok()
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
        let mut subjects: Vec<EvaluableSubject> = subject(FindingDomain::Execution, "transaction")
            .into_iter()
            .collect();
        let Ok((op, _)) = self.operation(transaction) else {
            return subjects;
        };
        for (deposit, name, required, measurement) in PROMOTED {
            if deposit != op.is_deposit() {
                continue;
            }
            // A subject is evaluable when the account it is read from is part of
            // this observation - a property of the shape, never of what
            // happened to differ. The derived ones additionally need the state
            // the evaluator reads: declaring `debt_increased` for an obligation
            // whose bytes were not captured would be claiming a measurement
            // this adapter cannot make.
            let present = self.account(accounts, required).is_some();
            let evaluable = match measurement {
                Measurement::DirectUniversal | Measurement::DirectTypedField => present,
                Measurement::DerivedKamino => present && self.obligation_of(accounts).is_some(),
            };
            if evaluable {
                subjects.extend(subject(FindingDomain::Economic, name));
            }
        }
        subjects
    }

    fn decoded_sources_of(&self, subject: &str) -> &'static [(&'static str, &'static str)] {
        match subject {
            "liquidity_deposited" => &[("user-source-liquidity", "amount")],
            "reserve_liquidity_received" => &[("reserve-liquidity-supply", "amount")],
            "liquidity_borrowed" => &[("user-destination-liquidity", "amount")],
            "reserve_liquidity_drawn" => &[("reserve-source-liquidity", "amount")],
            "origination_fee" => &[("borrow-reserve-liquidity-fee-receiver", "amount")],
            // The slot the action touches is a property of the observation, so
            // every slot a debt could live in is named.
            "debt_increased" => &OBLIGATION_BORROW_FIELDS,
            "obligation_collateral_deposited" => &OBLIGATION_DEPOSIT_FIELDS,
            _ => &[],
        }
    }

    fn decoded_byte_ranges(&self, account_label: &str) -> &'static [std::ops::Range<usize>] {
        match account_label {
            "obligation" => state::OBLIGATION_DECODED_RANGES,
            "reserve" | "borrow-reserve" => state::RESERVE_DECODED_RANGES,
            "user-source-liquidity"
            | "user-destination-liquidity"
            | "reserve-liquidity-supply"
            | "reserve-source-liquidity"
            | "borrow-reserve-liquidity-fee-receiver"
            | "reserve-destination-deposit-collateral" => TOKEN_AMOUNT_RANGE,
            "reserve-collateral-mint" => MINT_SUPPLY_RANGE,
            _ => &[],
        }
    }

    fn summarize(&self, accounts: &[NamedAccount], result: &ExecutionResult) -> Vec<SemanticField> {
        let Some(op) = self.operation_from_accounts(accounts) else {
            return Vec::new();
        };
        let decimals = self.liquidity_decimals(accounts, op);
        let mut fields = Vec::new();

        // Every quantity is read from where value actually landed, never from
        // the instruction's stated amount: a candidate that computes a different
        // number cannot hide behind the argument it was handed.
        if op.is_deposit() {
            if let Some((opening, closing)) =
                self.token_moved(accounts, result, "user-source-liquidity")
            {
                fields.push(quantity_field(
                    "liquidity_deposited",
                    opening.saturating_sub(closing),
                    decimals,
                ));
            }
            if let Some((opening, closing)) =
                self.token_moved(accounts, result, "reserve-liquidity-supply")
            {
                fields.push(quantity_field(
                    "reserve_liquidity_received",
                    closing.saturating_sub(opening),
                    decimals,
                ));
            }
            if let (Some(before), Some(after)) = (
                self.account(accounts, "reserve-collateral-mint")
                    .and_then(|n| self.mint_supply(&n.account)),
                result
                    .accounts
                    .get("reserve-collateral-mint")
                    .and_then(|s| self.mint_supply(s)),
            ) {
                fields.push(quantity_field(
                    "collateral_minted",
                    after.saturating_sub(before),
                    0,
                ));
            }
            // The obligation's own record of the collateral, which is the
            // position the depositor actually holds.
            if let Some((before, after)) = self.obligation_positions(accounts, result) {
                let reserve = self.role_reserve_address(accounts, op);
                let opening = reserve
                    .as_deref()
                    .and_then(|r| before.collateral_in(r))
                    .map(|p| p.deposited_amount)
                    .unwrap_or(0);
                let closing = reserve
                    .as_deref()
                    .and_then(|r| after.collateral_in(r))
                    .map(|p| p.deposited_amount)
                    .unwrap_or(0);
                fields.push(quantity_field(
                    "obligation_collateral_deposited",
                    closing.saturating_sub(opening),
                    0,
                ));
            }
        } else {
            if let Some((opening, closing)) =
                self.token_moved(accounts, result, "user-destination-liquidity")
            {
                fields.push(quantity_field(
                    "liquidity_borrowed",
                    closing.saturating_sub(opening),
                    decimals,
                ));
            }
            if let Some((opening, closing)) =
                self.token_moved(accounts, result, "reserve-source-liquidity")
            {
                fields.push(quantity_field(
                    "reserve_liquidity_drawn",
                    opening.saturating_sub(closing),
                    decimals,
                ));
            }
            if let Some((opening, closing)) =
                self.token_moved(accounts, result, "borrow-reserve-liquidity-fee-receiver")
            {
                fields.push(quantity_field(
                    "origination_fee",
                    closing.saturating_sub(opening),
                    decimals,
                ));
            }
            // The debt. Not a token flow, not derivable from one, and the whole
            // reason this action is in the slice.
            if let Some((before, after)) = self.obligation_positions(accounts, result) {
                if let Some(reserve) = self.role_reserve_address(accounts, op) {
                    let opening = before
                        .borrow_against(&reserve)
                        .map(|p| p.borrowed_amount_sf)
                        .unwrap_or(0);
                    let closing = after
                        .borrow_against(&reserve)
                        .map(|p| p.borrowed_amount_sf)
                        .unwrap_or(0);
                    let base_units = fraction::delta_base_units(opening, closing);
                    fields.push(quantity_field(
                        "debt_increased",
                        u64::try_from(base_units.max(0)).unwrap_or(u64::MAX),
                        decimals,
                    ));
                    // The exact scaled evidence beside the converted quantity,
                    // so the conversion can be checked rather than trusted.
                    fields.push(SemanticField {
                        name: "debt_increased_scaled".into(),
                        value: FieldValue::Text(
                            fraction::delta_scaled(opening, closing).to_string(),
                        ),
                        economic: false,
                    });
                }
            }
        }
        fields
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

        // Whether it ran at all comes first, and when it differs it is the whole
        // story: a candidate that rejects the instruction produced no
        // quantities, and reporting the borrower's debt as "decreased to zero"
        // beside it would double-count one change as two.
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
                    severity: crate::diff::Severity::Critical,
                })
                .into_iter()
                .collect();
        }

        let Ok((op, _)) = self.operation(transaction) else {
            return Vec::new();
        };
        let before = self.summarize(accounts, v1);
        let after = self.summarize(accounts, v2);
        let mut findings = Vec::new();
        for (deposit, name, _, _) in PROMOTED {
            if deposit != op.is_deposit() {
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
                // A quantity a user receives, owes, or hands over is a balance,
                // and the generic layer already rates a changed balance High.
                severity: crate::diff::Severity::High,
            });
        }
        findings
    }

    fn state_features(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
    ) -> Vec<StateFeature> {
        let Ok((op, _)) = self.operation(transaction) else {
            return Vec::new();
        };
        let mut features = vec![
            StateFeature::text("semantic_action", op.semantic_action().as_str()),
            StateFeature::text("klend_action", op.action_id()),
            StateFeature::text("klend_instruction", op.name()),
        ];
        if let Some(amount) = self.requested_amount(transaction) {
            features.push(StateFeature::integer("requested_amount", amount as u128));
        }
        if let Some(reserve) = self.reserve_of(accounts, Self::reserve_label(op)) {
            features.push(StateFeature::text(
                "lending_market",
                reserve.lending_market.clone(),
            ));
            features.push(StateFeature::text(
                "liquidity_mint",
                reserve.liquidity_mint.clone(),
            ));
            features.push(StateFeature::integer(
                "reserve_available_liquidity",
                reserve.available_amount as u128,
            ));
            features.push(StateFeature::integer(
                "reserve_borrowed_base_units",
                fraction::to_base_units(reserve.borrowed_amount_sf),
            ));
            features.push(StateFeature::integer(
                "reserve_mint_decimals",
                reserve.mint_decimals as u128,
            ));
        }
        if let Some(obligation) = self.obligation_of(accounts) {
            features.push(StateFeature::integer(
                "obligation_deposit_positions",
                obligation.deposits.len() as u128,
            ));
            features.push(StateFeature::integer(
                "obligation_borrow_positions",
                obligation.borrows.len() as u128,
            ));
            features.push(StateFeature::integer(
                "obligation_has_debt",
                u128::from(obligation.has_debt),
            ));
        }
        features
    }

    /// Distances to thresholds KLend itself defines.
    ///
    /// One is modelled: how much of a reserve's available liquidity the action
    /// moves. A borrow that drains a reserve is where availability arithmetic
    /// stops having slack, and it is a real branch in the program. Nothing
    /// about *risk* is claimed — that would need an oracle price and a
    /// liquidation threshold, neither of which this phase implements.
    fn boundaries(
        &self,
        transaction: &HistoricalTransaction,
        accounts: &[NamedAccount],
    ) -> Vec<BoundaryDistance> {
        let (Ok((op, _)), Some(amount)) = (
            self.operation(transaction),
            self.requested_amount(transaction),
        ) else {
            return Vec::new();
        };
        let Some(reserve) = self.reserve_of(accounts, Self::reserve_label(op)) else {
            return Vec::new();
        };
        vec![BoundaryDistance::from_quantities(
            "amount_against_reserve_liquidity",
            "amount moved against the reserve's available liquidity; a small distance is an \
             interaction large enough to exhaust it",
            u128::from(amount),
            u128::from(reserve.available_amount),
        )]
    }
}

pub const INSTRUCTION_SYSVAR_ID: &str = "Sysvar1nstructions1111111111111111111111111";

const TOKEN_AMOUNT_RANGE: &[std::ops::Range<usize>] = &[std::ops::Range { start: 64, end: 72 }];
const MINT_SUPPLY_RANGE: &[std::ops::Range<usize>] = &[std::ops::Range { start: 36, end: 44 }];

fn quantity_field(name: &str, base_units: u64, decimals: u8) -> SemanticField {
    SemanticField {
        name: name.into(),
        value: FieldValue::quantity(base_units, decimals),
        economic: true,
    }
}

/// Read a token account's amount under whichever token program owns the bytes.
///
/// The boundary prover takes a function rather than a program id because a
/// KLend market names its own token program and both appear in production.
/// Neither branch parses a layout here: both call the shared decoders.
fn any_token_program_amount(data: &[u8]) -> Option<u64> {
    crate::standard_programs::spl_token::account_amount(data)
        .or_else(|| crate::standard_programs::token2022::account_amount(data))
}

fn any_token_program_mint(data: &[u8]) -> Option<String> {
    crate::standard_programs::spl_token::account_mint(data)
        .or_else(|| crate::standard_programs::token2022::account_mint(data))
}

impl KaminoKlendAdapter {
    /// Which operation an account set describes, inferred from the labels the
    /// adapter itself bound. `summarize` and `interpret` receive accounts but
    /// no transaction, and the labels are the adapter's own vocabulary.
    fn operation_from_accounts(&self, accounts: &[NamedAccount]) -> Option<KlendOp> {
        if self
            .account(accounts, "user-destination-liquidity")
            .is_some()
        {
            Some(KlendOp::BorrowV1)
        } else if self.account(accounts, "user-source-liquidity").is_some() {
            Some(KlendOp::DepositV1)
        } else {
            None
        }
    }

    fn role_reserve_address(&self, accounts: &[NamedAccount], op: KlendOp) -> Option<String> {
        self.account(accounts, Self::reserve_label(op))
            .map(|named| named.address.clone())
    }

    /// The obligation before and after one execution.
    fn obligation_positions(
        &self,
        accounts: &[NamedAccount],
        result: &ExecutionResult,
    ) -> Option<(state::Obligation, state::Obligation)> {
        let before = self.obligation_of(accounts)?;
        let after = state::decode_obligation(&result.accounts.get("obligation")?.data).ok()?;
        Some((before, after))
    }

    fn decode_reserve_fields(&self, reserve: &state::Reserve) -> SemanticAccount {
        SemanticAccount {
            kind: "kamino-reserve".into(),
            fields: vec![
                SemanticField {
                    name: "lending_market".into(),
                    value: FieldValue::Address(reserve.lending_market.clone()),
                    economic: false,
                },
                SemanticField {
                    name: "liquidity_mint".into(),
                    value: FieldValue::Address(reserve.liquidity_mint.clone()),
                    economic: false,
                },
                SemanticField {
                    name: "available_amount".into(),
                    value: FieldValue::quantity(reserve.available_amount, 0),
                    economic: true,
                },
                SemanticField {
                    // Exact, scaled, and reported as text because a `u128` does
                    // not fit a quantity. This is the raw evidence: a change
                    // here that no finding names is undeclarable, which is the
                    // correct outcome for a sub-base-unit debt movement.
                    name: "borrowed_amount_sf".into(),
                    value: FieldValue::Text(reserve.borrowed_amount_sf.to_string()),
                    economic: true,
                },
                SemanticField {
                    name: "collateral_total_supply".into(),
                    value: FieldValue::quantity(reserve.collateral_total_supply, 0),
                    economic: true,
                },
                SemanticField {
                    name: "mint_decimals".into(),
                    value: FieldValue::Count(u64::from(reserve.mint_decimals)),
                    economic: false,
                },
            ],
        }
    }

    fn decode_obligation_fields(&self, obligation: &state::Obligation) -> SemanticAccount {
        let mut fields = vec![
            SemanticField {
                name: "lending_market".into(),
                value: FieldValue::Address(obligation.lending_market.clone()),
                economic: false,
            },
            SemanticField {
                name: "owner".into(),
                value: FieldValue::Address(obligation.owner.clone()),
                economic: false,
            },
            SemanticField {
                name: "has_debt".into(),
                value: FieldValue::Flag(obligation.has_debt),
                economic: true,
            },
        ];
        // Per-slot fields with fixed names, so a subject can name the slot it
        // reads. An unoccupied slot contributes nothing rather than a zero.
        for position in &obligation.deposits {
            fields.push(SemanticField {
                name: format!("deposit_{}_reserve", position.index),
                value: FieldValue::Address(position.deposit_reserve.clone()),
                economic: false,
            });
            fields.push(SemanticField {
                name: format!("deposit_{}_amount", position.index),
                value: FieldValue::quantity(position.deposited_amount, 0),
                economic: true,
            });
        }
        for position in &obligation.borrows {
            fields.push(SemanticField {
                name: format!("borrow_{}_reserve", position.index),
                value: FieldValue::Address(position.borrow_reserve.clone()),
                economic: false,
            });
            fields.push(SemanticField {
                name: format!("borrow_{}_amount_sf", position.index),
                value: FieldValue::Text(position.borrowed_amount_sf.to_string()),
                economic: true,
            });
        }
        SemanticAccount {
            kind: "kamino-obligation".into(),
            fields,
        }
    }

    /// Bind each declared account position to its KLend role.
    fn labels(&self, transaction: &HistoricalTransaction) -> Vec<String> {
        let mut bindings = Vec::new();
        if let Ok((op, instruction)) = self.operation(transaction) {
            for (position, role) in op.roles().into_iter().enumerate() {
                let Some(meta) = instruction.accounts.get(position) else {
                    continue;
                };
                bindings.push(labels::RoleBinding::new(meta.address.clone(), role));
            }
        }
        labels::assign(
            transaction
                .account_keys
                .iter()
                .map(|key| key.address.as_str()),
            &bindings,
        )
    }
}

#[cfg(test)]
mod tests;
