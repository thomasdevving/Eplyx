//! Kamino KLend account layouts.
//!
//! Only the fields the two supported actions need. Nothing here is generic:
//! these offsets are one protocol's private state, read under a manually
//! reviewed interface, and they belong in the adapter rather than in
//! [`crate::standard_programs`].
//!
//! ## Provenance
//!
//! Derived from the Anchor IDL published on chain by the KLend upgrade
//! authority (`kamino_lending` 1.25.0), cross-checked field by field against
//! the deployed accounts' byte lengths. That makes the layout claim
//! `UpgradeAuthorityPublished`, **not** a verified build: the upgrade authority
//! publishes the IDL and is also the party whose change this tool exists to
//! measure. See [`INTERFACE`].
//!
//! ## Why there is no sequential reader here
//!
//! `protocol::stake_pool` needs one, because the `StakePool` layout carries
//! three `Option<Pubkey>` and three `FutureEpoch<Fee>` fields whose lengths
//! depend on their contents, so a table of constants would silently mis-read a
//! pool configured differently. Kamino's Anchor structs are **fixed-size in
//! their entirety** — every field is a scalar, a `publicKey`, or a fixed array —
//! so an offset table is exact and a cursor would buy nothing. The two are not
//! the same shape, and Phase U2 does not extract a shared reader on the strength
//! of a resemblance that stops at "both read bytes".

use crate::standard_programs::{address_at, u128_at, u64_at, u8_at, Decoded, MalformedReason};

/// What the layout below was derived from, and what that is worth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceIdentity {
    pub program_id: &'static str,
    pub idl_name: &'static str,
    pub idl_version: &'static str,
    /// sha256 of the decompressed IDL JSON as published on chain.
    pub idl_sha256: &'static str,
    /// The account holding that IDL, so a reader can refetch and rehash it.
    pub idl_account: &'static str,
}

/// The interface this adapter was written against.
///
/// Recorded, never inferred. An on-chain IDL is evidence about *intent*, never
/// about bytes: it is published by the upgrade authority, which is exactly the
/// party whose change a differential gate is watching. Nothing in this adapter
/// claims source-to-bytecode equivalence, and the fidelity gate does not rest
/// on the IDL at all — it rests on the deployed V1 binary reproducing the
/// original mainnet post-state.
pub const INTERFACE: InterfaceIdentity = InterfaceIdentity {
    program_id: super::PROGRAM_ID,
    idl_name: "kamino_lending",
    idl_version: "1.25.0",
    idl_sha256: "8ac43c0a2f4a927ea0fd0cbd562efa1b61cb95afdde299ad5c0608079cf09164",
    idl_account: "8qLKwp1fk8WyqmzarkuMeZEX3AzL4VDSmA2UZTKT2aCJ",
};

// Anchor account discriminators: sha256("account:<Name>")[..8].
pub const RESERVE_DISCRIMINATOR: [u8; 8] = [43, 242, 204, 202, 26, 247, 59, 127];
pub const OBLIGATION_DISCRIMINATOR: [u8; 8] = [168, 206, 141, 106, 88, 76, 172, 167];
pub const LENDING_MARKET_DISCRIMINATOR: [u8; 8] = [246, 114, 50, 98, 72, 157, 28, 120];

/// Exact serialized sizes. A buffer of any other length is not this structure,
/// and is refused rather than read at whatever offsets happen to fit.
pub const RESERVE_LEN: usize = 8624;
pub const OBLIGATION_LEN: usize = 3344;

// --- Reserve -------------------------------------------------------------
const RESERVE_LENDING_MARKET: usize = 32;
const RESERVE_LIQUIDITY: usize = 128;
const LIQUIDITY_MINT: usize = RESERVE_LIQUIDITY;
const LIQUIDITY_SUPPLY_VAULT: usize = RESERVE_LIQUIDITY + 32;
const LIQUIDITY_FEE_VAULT: usize = RESERVE_LIQUIDITY + 64;
const LIQUIDITY_AVAILABLE: usize = RESERVE_LIQUIDITY + 96;
const LIQUIDITY_BORROWED_SF: usize = RESERVE_LIQUIDITY + 104;
const LIQUIDITY_MINT_DECIMALS: usize = RESERVE_LIQUIDITY + 144;
const RESERVE_COLLATERAL: usize = 2560;
const COLLATERAL_MINT: usize = RESERVE_COLLATERAL;
const COLLATERAL_TOTAL_SUPPLY: usize = RESERVE_COLLATERAL + 32;
const COLLATERAL_SUPPLY_VAULT: usize = RESERVE_COLLATERAL + 40;

// --- Obligation ----------------------------------------------------------
const OBLIGATION_LENDING_MARKET: usize = 32;
const OBLIGATION_OWNER: usize = 64;
const OBLIGATION_DEPOSITS: usize = 96;
const DEPOSIT_STRIDE: usize = 136;
pub const DEPOSIT_SLOTS: usize = 8;
const DEPOSIT_RESERVE: usize = 0;
const DEPOSIT_AMOUNT: usize = 32;
const OBLIGATION_BORROWS: usize = 1208;
const BORROW_STRIDE: usize = 200;
pub const BORROW_SLOTS: usize = 5;
const BORROW_RESERVE: usize = 0;
const BORROW_AMOUNT_SF: usize = 88;
const OBLIGATION_HAS_DEBT: usize = 2287;

// Tripwires. The IDL declares these structures back to back, so an offset that
// drifts out of step with a neighbour is a compile error rather than a silently
// wrong read.
const _: () =
    assert!(OBLIGATION_DEPOSITS + DEPOSIT_SLOTS * DEPOSIT_STRIDE + 8 + 16 == OBLIGATION_BORROWS);
const _: () = assert!(OBLIGATION_BORROWS + BORROW_SLOTS * BORROW_STRIDE <= OBLIGATION_LEN);
const _: () = assert!(RESERVE_COLLATERAL > RESERVE_LIQUIDITY);

/// A KLend reserve, in the fields the supported actions read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reserve {
    pub lending_market: String,
    pub liquidity_mint: String,
    pub liquidity_supply_vault: String,
    pub liquidity_fee_vault: String,
    /// Liquidity sitting in the vault, in token base units.
    pub available_amount: u64,
    /// Outstanding borrows as a scaled fraction. See
    /// [`super::fraction`] — this is **not** a token amount.
    pub borrowed_amount_sf: u128,
    pub mint_decimals: u8,
    pub collateral_mint: String,
    pub collateral_total_supply: u64,
    pub collateral_supply_vault: String,
}

/// One borrow position inside an obligation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BorrowPosition {
    pub index: usize,
    pub borrow_reserve: String,
    /// Scaled fraction, `U68F60`.
    pub borrowed_amount_sf: u128,
}

/// One collateral position inside an obligation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollateralPosition {
    pub index: usize,
    pub deposit_reserve: String,
    /// Collateral tokens, an ordinary integer rather than a fraction.
    pub deposited_amount: u64,
}

/// A KLend obligation: one borrower's position in one market.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    pub lending_market: String,
    pub owner: String,
    pub has_debt: bool,
    /// Only occupied slots. An empty slot names the default address and holds
    /// nothing, and reporting eight of them as positions would invent seven.
    pub deposits: Vec<CollateralPosition>,
    pub borrows: Vec<BorrowPosition>,
}

/// The all-zero address an unoccupied position slot carries.
const EMPTY_SLOT: &str = "11111111111111111111111111111111";

fn anchor_discriminator(data: &[u8]) -> Option<[u8; 8]> {
    data.get(..8)?.try_into().ok()
}

/// Decode a reserve. `NotApplicable` for anything that is not one.
pub fn decode_reserve(data: &[u8]) -> Decoded<Reserve> {
    if anchor_discriminator(data) != Some(RESERVE_DISCRIMINATOR) {
        return Decoded::NotApplicable;
    }
    if data.len() != RESERVE_LEN {
        // The discriminator says reserve and the length says otherwise. That is
        // a layout this build does not know, not an absent account.
        return Decoded::Malformed(MalformedReason::UnexpectedLength { found: data.len() });
    }
    let truncated = || {
        Decoded::Malformed(MalformedReason::Truncated {
            needed: RESERVE_LEN,
            found: data.len(),
        })
    };
    let (
        Some(lending_market),
        Some(liquidity_mint),
        Some(liquidity_supply_vault),
        Some(liquidity_fee_vault),
        Some(available_amount),
        Some(borrowed_amount_sf),
        Some(mint_decimals),
        Some(collateral_mint),
        Some(collateral_total_supply),
        Some(collateral_supply_vault),
    ) = (
        address_at(data, RESERVE_LENDING_MARKET),
        address_at(data, LIQUIDITY_MINT),
        address_at(data, LIQUIDITY_SUPPLY_VAULT),
        address_at(data, LIQUIDITY_FEE_VAULT),
        u64_at(data, LIQUIDITY_AVAILABLE),
        u128_at(data, LIQUIDITY_BORROWED_SF),
        u64_at(data, LIQUIDITY_MINT_DECIMALS),
        address_at(data, COLLATERAL_MINT),
        u64_at(data, COLLATERAL_TOTAL_SUPPLY),
        address_at(data, COLLATERAL_SUPPLY_VAULT),
    )
    else {
        return truncated();
    };
    // `mintDecimals` is a u64 in the layout and a u8 in every sane mint. A
    // value outside that range means the offsets are wrong, and reading on
    // would produce confident nonsense.
    let Ok(mint_decimals) = u8::try_from(mint_decimals) else {
        return Decoded::Malformed(MalformedReason::InvalidDiscriminant {
            at: LIQUIDITY_MINT_DECIMALS,
            value: u32::try_from(mint_decimals).unwrap_or(u32::MAX),
        });
    };
    Decoded::Decoded(Reserve {
        lending_market,
        liquidity_mint,
        liquidity_supply_vault,
        liquidity_fee_vault,
        available_amount,
        borrowed_amount_sf,
        mint_decimals,
        collateral_mint,
        collateral_total_supply,
        collateral_supply_vault,
    })
}

/// Decode an obligation. `NotApplicable` for anything that is not one.
pub fn decode_obligation(data: &[u8]) -> Decoded<Obligation> {
    if anchor_discriminator(data) != Some(OBLIGATION_DISCRIMINATOR) {
        return Decoded::NotApplicable;
    }
    if data.len() != OBLIGATION_LEN {
        return Decoded::Malformed(MalformedReason::UnexpectedLength { found: data.len() });
    }
    let truncated = || {
        Decoded::Malformed(MalformedReason::Truncated {
            needed: OBLIGATION_LEN,
            found: data.len(),
        })
    };
    let (Some(lending_market), Some(owner), Some(has_debt)) = (
        address_at(data, OBLIGATION_LENDING_MARKET),
        address_at(data, OBLIGATION_OWNER),
        u8_at(data, OBLIGATION_HAS_DEBT),
    ) else {
        return truncated();
    };

    let mut deposits = Vec::new();
    for index in 0..DEPOSIT_SLOTS {
        let base = OBLIGATION_DEPOSITS + index * DEPOSIT_STRIDE;
        let (Some(reserve), Some(amount)) = (
            address_at(data, base + DEPOSIT_RESERVE),
            u64_at(data, base + DEPOSIT_AMOUNT),
        ) else {
            return truncated();
        };
        if reserve == EMPTY_SLOT {
            continue;
        }
        deposits.push(CollateralPosition {
            index,
            deposit_reserve: reserve,
            deposited_amount: amount,
        });
    }

    let mut borrows = Vec::new();
    for index in 0..BORROW_SLOTS {
        let base = OBLIGATION_BORROWS + index * BORROW_STRIDE;
        let (Some(reserve), Some(amount)) = (
            address_at(data, base + BORROW_RESERVE),
            u128_at(data, base + BORROW_AMOUNT_SF),
        ) else {
            return truncated();
        };
        if reserve == EMPTY_SLOT {
            continue;
        }
        borrows.push(BorrowPosition {
            index,
            borrow_reserve: reserve,
            borrowed_amount_sf: amount,
        });
    }

    Decoded::Decoded(Obligation {
        lending_market,
        owner,
        has_debt: has_debt == 1,
        deposits,
        borrows,
    })
}

/// Byte ranges this adapter decodes out of an obligation.
///
/// Exactly the position amounts it reads, plus the derived value fields KLend
/// recomputes from them. The derived ones are decoded as context rather than as
/// economics - `borrowFactorAdjustedDebtValueSf` is a function of the debt and
/// of an oracle price, not an independent fact - but their bytes are listed
/// here so that a debt change a finding *did* name does not also surface as
/// unexplained structural evidence for its own consequences.
///
/// Everything outside these ranges is state this adapter does not interpret, so
/// no finding can speak for it.
pub const OBLIGATION_DECODED_RANGES: &[std::ops::Range<usize>] = &[
    // deposits[i].depositedAmount
    DEP0..DEP0 + 8,
    DEP1..DEP1 + 8,
    DEP2..DEP2 + 8,
    DEP3..DEP3 + 8,
    DEP4..DEP4 + 8,
    DEP5..DEP5 + 8,
    DEP6..DEP6 + 8,
    DEP7..DEP7 + 8,
    // deposits[i].marketValueSf - recomputed from the deposit and a price
    DEP0 + 8..DEP0 + 24,
    DEP1 + 8..DEP1 + 24,
    DEP2 + 8..DEP2 + 24,
    DEP3 + 8..DEP3 + 24,
    DEP4 + 8..DEP4 + 24,
    DEP5 + 8..DEP5 + 24,
    DEP6 + 8..DEP6 + 24,
    DEP7 + 8..DEP7 + 24,
    // borrows[i].borrowedAmountSf, then the two value fields derived from it
    BOR0..BOR0 + 48,
    BOR1..BOR1 + 48,
    BOR2..BOR2 + 48,
    BOR3..BOR3 + 48,
    BOR4..BOR4 + 48,
    // depositedValueSf, and the obligation-wide value aggregates
    1192..1208,
    2208..2272,
];

// Absolute offsets of the fields above, so the table reads as a table.
const DEP0: usize = OBLIGATION_DEPOSITS + DEPOSIT_AMOUNT;
const DEP1: usize = DEP0 + DEPOSIT_STRIDE;
const DEP2: usize = DEP1 + DEPOSIT_STRIDE;
const DEP3: usize = DEP2 + DEPOSIT_STRIDE;
const DEP4: usize = DEP3 + DEPOSIT_STRIDE;
const DEP5: usize = DEP4 + DEPOSIT_STRIDE;
const DEP6: usize = DEP5 + DEPOSIT_STRIDE;
const DEP7: usize = DEP6 + DEPOSIT_STRIDE;
const BOR0: usize = OBLIGATION_BORROWS + BORROW_AMOUNT_SF;
const BOR1: usize = BOR0 + BORROW_STRIDE;
const BOR2: usize = BOR1 + BORROW_STRIDE;
const BOR3: usize = BOR2 + BORROW_STRIDE;
const BOR4: usize = BOR3 + BORROW_STRIDE;

/// Byte ranges this adapter decodes out of a reserve.
pub const RESERVE_DECODED_RANGES: &[std::ops::Range<usize>] = &[
    // liquidity.totalAvailableAmount, then borrowedAmountSf and the price the
    // refresh wrote beside it.
    LIQUIDITY_AVAILABLE..LIQUIDITY_AVAILABLE + 8,
    LIQUIDITY_BORROWED_SF..LIQUIDITY_BORROWED_SF + 16,
    // collateral.mintTotalSupply
    COLLATERAL_TOTAL_SUPPLY..COLLATERAL_TOTAL_SUPPLY + 8,
];

/// Whether a buffer is a KLend lending market.
///
/// Only the discriminator is checked: the adapter needs to *recognise* a market
/// for its invariants and reads no field out of it, so decoding one would be
/// claiming a layout this phase never exercises.
pub fn is_lending_market(data: &[u8]) -> bool {
    anchor_discriminator(data) == Some(LENDING_MARKET_DISCRIMINATOR)
}

impl Obligation {
    /// The borrow position against `reserve`, if the obligation holds one.
    pub fn borrow_against(&self, reserve: &str) -> Option<&BorrowPosition> {
        self.borrows
            .iter()
            .find(|position| position.borrow_reserve == reserve)
    }

    /// The collateral position in `reserve`, if the obligation holds one.
    pub fn collateral_in(&self, reserve: &str) -> Option<&CollateralPosition> {
        self.deposits
            .iter()
            .find(|position| position.deposit_reserve == reserve)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Byte builders for the two layouts, used by the adapter's tests.
    //!
    //! Written as field placement rather than pasted blobs, so a test states
    //! what it is asserting and a layout change breaks it visibly.
    use super::*;

    pub fn address_bytes(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    pub fn address_of(byte: u8) -> String {
        bs58::encode(address_bytes(byte)).into_string()
    }

    pub struct ReserveBuilder {
        pub data: Vec<u8>,
    }

    impl ReserveBuilder {
        pub fn new() -> Self {
            let mut data = vec![0_u8; RESERVE_LEN];
            data[..8].copy_from_slice(&RESERVE_DISCRIMINATOR);
            Self { data }
        }
        pub fn lending_market(mut self, byte: u8) -> Self {
            self.data[RESERVE_LENDING_MARKET..RESERVE_LENDING_MARKET + 32]
                .copy_from_slice(&address_bytes(byte));
            self
        }
        pub fn liquidity_mint(mut self, byte: u8) -> Self {
            self.data[LIQUIDITY_MINT..LIQUIDITY_MINT + 32].copy_from_slice(&address_bytes(byte));
            self
        }
        pub fn supply_vault(mut self, byte: u8) -> Self {
            self.data[LIQUIDITY_SUPPLY_VAULT..LIQUIDITY_SUPPLY_VAULT + 32]
                .copy_from_slice(&address_bytes(byte));
            self
        }
        pub fn fee_vault(mut self, byte: u8) -> Self {
            self.data[LIQUIDITY_FEE_VAULT..LIQUIDITY_FEE_VAULT + 32]
                .copy_from_slice(&address_bytes(byte));
            self
        }
        pub fn available(mut self, amount: u64) -> Self {
            self.data[LIQUIDITY_AVAILABLE..LIQUIDITY_AVAILABLE + 8]
                .copy_from_slice(&amount.to_le_bytes());
            self
        }
        pub fn borrowed_sf(mut self, value: u128) -> Self {
            self.data[LIQUIDITY_BORROWED_SF..LIQUIDITY_BORROWED_SF + 16]
                .copy_from_slice(&value.to_le_bytes());
            self
        }
        pub fn decimals(mut self, decimals: u8) -> Self {
            self.data[LIQUIDITY_MINT_DECIMALS..LIQUIDITY_MINT_DECIMALS + 8]
                .copy_from_slice(&u64::from(decimals).to_le_bytes());
            self
        }
        pub fn collateral_mint(mut self, byte: u8) -> Self {
            self.data[COLLATERAL_MINT..COLLATERAL_MINT + 32].copy_from_slice(&address_bytes(byte));
            self
        }
        pub fn collateral_supply(mut self, amount: u64) -> Self {
            self.data[COLLATERAL_TOTAL_SUPPLY..COLLATERAL_TOTAL_SUPPLY + 8]
                .copy_from_slice(&amount.to_le_bytes());
            self
        }
        pub fn collateral_vault(mut self, byte: u8) -> Self {
            self.data[COLLATERAL_SUPPLY_VAULT..COLLATERAL_SUPPLY_VAULT + 32]
                .copy_from_slice(&address_bytes(byte));
            self
        }
        pub fn build(self) -> Vec<u8> {
            self.data
        }
    }

    pub struct ObligationBuilder {
        pub data: Vec<u8>,
    }

    impl ObligationBuilder {
        pub fn new() -> Self {
            let mut data = vec![0_u8; OBLIGATION_LEN];
            data[..8].copy_from_slice(&OBLIGATION_DISCRIMINATOR);
            Self { data }
        }
        pub fn lending_market(mut self, byte: u8) -> Self {
            self.data[OBLIGATION_LENDING_MARKET..OBLIGATION_LENDING_MARKET + 32]
                .copy_from_slice(&address_bytes(byte));
            self
        }
        pub fn owner(mut self, byte: u8) -> Self {
            self.data[OBLIGATION_OWNER..OBLIGATION_OWNER + 32]
                .copy_from_slice(&address_bytes(byte));
            self
        }
        pub fn has_debt(mut self, debt: bool) -> Self {
            self.data[OBLIGATION_HAS_DEBT] = u8::from(debt);
            self
        }
        pub fn deposit(mut self, slot: usize, reserve: u8, amount: u64) -> Self {
            let base = OBLIGATION_DEPOSITS + slot * DEPOSIT_STRIDE;
            self.data[base..base + 32].copy_from_slice(&address_bytes(reserve));
            self.data[base + DEPOSIT_AMOUNT..base + DEPOSIT_AMOUNT + 8]
                .copy_from_slice(&amount.to_le_bytes());
            self
        }
        pub fn borrow(mut self, slot: usize, reserve: u8, amount_sf: u128) -> Self {
            let base = OBLIGATION_BORROWS + slot * BORROW_STRIDE;
            self.data[base..base + 32].copy_from_slice(&address_bytes(reserve));
            self.data[base + BORROW_AMOUNT_SF..base + BORROW_AMOUNT_SF + 16]
                .copy_from_slice(&amount_sf.to_le_bytes());
            self
        }
        pub fn build(self) -> Vec<u8> {
            self.data
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fixtures::*, *};
    use crate::protocol::kamino::fraction;

    #[test]
    fn a_reserve_decodes_every_field_the_adapter_reads() {
        let data = ReserveBuilder::new()
            .lending_market(1)
            .liquidity_mint(2)
            .supply_vault(3)
            .fee_vault(4)
            .available(9_000_000)
            .borrowed_sf(1_500_u128 << fraction::FRACTION_BITS)
            .decimals(6)
            .collateral_mint(5)
            .collateral_supply(42_000)
            .collateral_vault(6)
            .build();
        let reserve = decode_reserve(&data).ok().expect("decodes");
        assert_eq!(reserve.lending_market, address_of(1));
        assert_eq!(reserve.liquidity_mint, address_of(2));
        assert_eq!(reserve.liquidity_supply_vault, address_of(3));
        assert_eq!(reserve.liquidity_fee_vault, address_of(4));
        assert_eq!(reserve.available_amount, 9_000_000);
        assert_eq!(fraction::to_base_units(reserve.borrowed_amount_sf), 1_500);
        assert_eq!(reserve.mint_decimals, 6);
        assert_eq!(reserve.collateral_mint, address_of(5));
        assert_eq!(reserve.collateral_total_supply, 42_000);
        assert_eq!(reserve.collateral_supply_vault, address_of(6));
    }

    #[test]
    fn an_obligation_reports_only_occupied_position_slots() {
        let data = ObligationBuilder::new()
            .lending_market(1)
            .owner(7)
            .has_debt(true)
            .deposit(0, 10, 5_000)
            .deposit(3, 11, 250)
            .borrow(0, 12, 800_u128 << fraction::FRACTION_BITS)
            .build();
        let obligation = decode_obligation(&data).ok().expect("decodes");
        assert_eq!(obligation.lending_market, address_of(1));
        assert_eq!(obligation.owner, address_of(7));
        assert!(obligation.has_debt);
        // Eight deposit slots exist; two are occupied. Reporting eight would
        // invent six positions that hold nothing.
        assert_eq!(obligation.deposits.len(), 2);
        assert_eq!(obligation.deposits[0].index, 0);
        assert_eq!(obligation.deposits[1].index, 3);
        assert_eq!(obligation.deposits[1].deposited_amount, 250);
        assert_eq!(obligation.borrows.len(), 1);
        assert_eq!(
            fraction::to_base_units(obligation.borrows[0].borrowed_amount_sf),
            800
        );
    }

    #[test]
    fn positions_are_found_by_their_reserve() {
        let data = ObligationBuilder::new()
            .deposit(0, 10, 5_000)
            .borrow(2, 12, 1 << fraction::FRACTION_BITS)
            .build();
        let obligation = decode_obligation(&data).ok().expect("decodes");
        assert_eq!(
            obligation
                .collateral_in(&address_of(10))
                .unwrap()
                .deposited_amount,
            5_000
        );
        assert_eq!(obligation.borrow_against(&address_of(12)).unwrap().index, 2);
        assert!(obligation.borrow_against(&address_of(99)).is_none());
        assert!(obligation.collateral_in(&address_of(99)).is_none());
    }

    /// The two layouts must not decode as each other, and neither may read a
    /// buffer that merely happens to be long enough.
    #[test]
    fn the_discriminator_gates_every_decode() {
        let reserve = ReserveBuilder::new().build();
        let obligation = ObligationBuilder::new().build();
        assert_eq!(decode_obligation(&reserve), Decoded::NotApplicable);
        assert_eq!(decode_reserve(&obligation), Decoded::NotApplicable);
        // A buffer of the right length with no discriminator is not a reserve.
        assert_eq!(
            decode_reserve(&vec![0_u8; RESERVE_LEN]),
            Decoded::NotApplicable
        );
        assert!(!is_lending_market(&reserve));
    }

    /// A discriminator that claims a reserve over the wrong number of bytes is
    /// malformed, never an absent account.
    #[test]
    fn a_right_tag_with_a_wrong_length_is_malformed() {
        let mut short = ReserveBuilder::new().build();
        short.truncate(RESERVE_LEN - 1);
        assert!(decode_reserve(&short).is_malformed());

        let mut long = ObligationBuilder::new().build();
        long.push(0);
        assert!(decode_obligation(&long).is_malformed());
    }

    #[test]
    fn an_implausible_decimal_count_is_malformed_rather_than_truncated_silently() {
        let mut data = ReserveBuilder::new().build();
        data[LIQUIDITY_MINT_DECIMALS..LIQUIDITY_MINT_DECIMALS + 8]
            .copy_from_slice(&1_000_u64.to_le_bytes());
        assert!(decode_reserve(&data).is_malformed());
    }

    /// The whole point of keeping the raw scaled value: the adapter never
    /// mistakes a fraction for a token amount.
    #[test]
    fn a_borrowed_amount_is_a_fraction_not_a_token_count() {
        let data = ObligationBuilder::new()
            .borrow(0, 12, 1_000_000_u128 << fraction::FRACTION_BITS)
            .build();
        let obligation = decode_obligation(&data).ok().expect("decodes");
        let raw = obligation.borrows[0].borrowed_amount_sf;
        assert_eq!(fraction::to_base_units(raw), 1_000_000);
        assert!(
            raw > 1_000_000,
            "the stored value is scaled by 2^60 and is not itself a token count"
        );
    }

    #[test]
    fn the_interface_records_what_it_rests_on() {
        assert_eq!(INTERFACE.idl_version, "1.25.0");
        assert_eq!(INTERFACE.program_id, super::super::PROGRAM_ID);
        assert_eq!(INTERFACE.idl_sha256.len(), 64);
    }
}
