//! The protocol adapter seam.
//!
//! Through Phase 6 this was a module convention: `corpus`, `interpret` and
//! `impact` knew what a health factor was, and the mainnet path bolted a second
//! hard-coded contract next to the fixture one. Two protocols is where that
//! stops scaling, so the seam is a trait from here on.
//!
//! An adapter owns exactly the knowledge that is specific to one program: which
//! transactions it can replay exactly, what its accounts mean, how an archived
//! snapshot is proved against validator metadata, and what an execution
//! difference means economically. Everything it hands back is protocol-agnostic,
//! which keeps `executor`, `diff` and `report` unable to learn protocol
//! semantics by accident.
//!
//! Quantities stay integer. A token amount is base units plus the mint's
//! decimal count, never a scaled float, and it serializes as a decimal string
//! for the same reason [`crate::money::Usd`] does: a JSON number becomes a
//! double in most consumers, and a u64 token amount does not survive that.

pub mod stake_pool;
pub mod token2022;

use crate::{
    executor::ExecutionResult,
    ingest::transactions::HistoricalTransaction,
    types::{AccountSnapshot, NamedAccount},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt;

/// An integer token amount, interpreted against its mint's decimal count.
///
/// Decimals travel with the value because they are a property of the mint, not
/// of the protocol: the same adapter handles a 6-decimal stablecoin and an
/// 8-decimal tokenized equity in the same run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenQuantity {
    pub base_units: u64,
    pub decimals: u8,
}

fn render(base_units: u128, decimals: u8) -> String {
    if decimals == 0 {
        return base_units.to_string();
    }
    // Placed by shifting the digits, never by computing a scale. `decimals` is
    // a `u8` read from a decoded mint, so `10u128.pow(decimals)` overflows at
    // 39 and aborted the process; switching to exponent notation past that
    // fixed the panic but produced a string the deserializer rejects, which
    // traded a crash for a value that could not round trip. Shifting is exact
    // for every input and always yields a plain decimal.
    let digits = base_units.to_string();
    let decimals = usize::from(decimals);
    if digits.len() > decimals {
        let split = digits.len() - decimals;
        format!("{}.{}", &digits[..split], &digits[split..])
    } else {
        format!("0.{}{}", "0".repeat(decimals - digits.len()), digits)
    }
}

impl TokenQuantity {
    pub fn new(base_units: u64, decimals: u8) -> Self {
        Self {
            base_units,
            decimals,
        }
    }

    /// Signed difference `self - other`, in base units.
    ///
    /// Returns `None` when the two sides carry different decimal counts: that
    /// means they came from different mints and subtracting them would be
    /// meaningless rather than merely imprecise.
    pub fn delta(self, other: Self) -> Option<SignedTokenQuantity> {
        if self.decimals != other.decimals {
            return None;
        }
        Some(SignedTokenQuantity {
            base_units: i128::from(self.base_units) - i128::from(other.base_units),
            decimals: self.decimals,
        })
    }
}

impl fmt::Display for TokenQuantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&render(u128::from(self.base_units), self.decimals))
    }
}

/// A signed token delta.
///
/// The magnitude is `i128` internally so that a full-range `u64` decrease is
/// representable, but it is never serialized inside an internally-tagged enum -
/// see [`crate::diff::Difference`] - so the report path renders it as a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignedTokenQuantity {
    pub base_units: i128,
    pub decimals: u8,
}

impl SignedTokenQuantity {
    pub fn is_zero(self) -> bool {
        self.base_units == 0
    }
}

impl fmt::Display for SignedTokenQuantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.base_units < 0 { "-" } else { "+" };
        write!(
            f,
            "{sign}{}",
            render(self.base_units.unsigned_abs(), self.decimals)
        )
    }
}

impl Serialize for SignedTokenQuantity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// Parse the decimal-string form back. The fraction's digit count *is* the
/// decimal count, so a value round-trips without carrying the mint's decimals
/// separately.
impl<'de> Deserialize<'de> for SignedTokenQuantity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let text = String::deserialize(deserializer)?;
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(&text)),
        };
        let (whole, fraction) = match digits.split_once('.') {
            Some((whole, fraction)) => (whole, fraction),
            None => (digits, ""),
        };
        if whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(D::Error::custom(format!("invalid token amount {text:?}")));
        }
        let decimals = u8::try_from(fraction.len())
            .map_err(|_| D::Error::custom("implausible decimal count"))?;
        let combined = format!("{whole}{fraction}");
        let magnitude: i128 = combined
            .parse()
            .map_err(|_| D::Error::custom(format!("token amount {text:?} overflows")))?;
        Ok(Self {
            base_units: if negative { -magnitude } else { magnitude },
            decimals,
        })
    }
}

/// One decoded protocol-level field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldValue {
    /// A token amount. Rendered as a decimal string, never a JSON number.
    Quantity {
        #[serde(serialize_with = "quantity_as_string")]
        amount: TokenQuantity,
        base_units: u64,
        decimals: u8,
    },
    Address(String),
    Flag(bool),
    Count(u64),
    Text(String),
}

fn quantity_as_string<S: serde::Serializer>(
    value: &TokenQuantity,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

impl FieldValue {
    pub fn quantity(base_units: u64, decimals: u8) -> Self {
        Self::Quantity {
            amount: TokenQuantity::new(base_units, decimals),
            base_units,
            decimals,
        }
    }

    pub fn as_quantity(&self) -> Option<TokenQuantity> {
        match self {
            Self::Quantity { amount, .. } => Some(*amount),
            _ => None,
        }
    }

    pub fn render(&self) -> String {
        match self {
            Self::Quantity { amount, .. } => amount.to_string(),
            Self::Address(address) => address.clone(),
            Self::Flag(flag) => flag.to_string(),
            Self::Count(count) => count.to_string(),
            Self::Text(text) => text.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SemanticField {
    pub name: String,
    pub value: FieldValue,
    /// Whether a change in this field moves money. Compute and bookkeeping
    /// fields are decoded for context but must not drive an economic verdict.
    pub economic: bool,
}

/// One account, decoded into protocol terms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SemanticAccount {
    pub kind: String,
    pub fields: Vec<SemanticField>,
}

impl SemanticAccount {
    pub fn field(&self, name: &str) -> Option<&SemanticField> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// One protocol quantity reported for both builds, whether or not it differs.
///
/// Distinct from [`EconomicChange`], which exists only when something changed.
/// A preserved economic outcome is a real result and has to be visible as one:
/// "both builds minted 0.077645365 pool tokens" is the answer a behaviour-
/// preserving upgrade should produce, and an empty change list alone does not
/// say it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicObservation {
    pub field: String,
    pub v1: String,
    pub v2: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<SignedTokenQuantity>,
    /// Whether a change in this quantity would move money.
    pub economic: bool,
}

/// One economically meaningful difference between the two builds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicChange {
    pub account_label: String,
    pub account_kind: String,
    pub field: String,
    pub v1: String,
    pub v2: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<SignedTokenQuantity>,
}

/// Accept only a message whose executable account set is exactly its static keys.
///
/// A v0 message that resolves no address lookup tables has the same accounts,
/// instructions and signers as a legacy one, and `replay` rebuilds both into the
/// same legacy `Message`, so it executes identically. A message that *does*
/// resolve tables is refused: the addresses are normalized for inspection, but
/// the lookup itself is not replayed, and pretending otherwise would put an
/// unproved account list under an exactness claim.
pub fn require_executable_message(transaction: &HistoricalTransaction) -> Result<()> {
    match transaction.version.as_str() {
        "legacy" => Ok(()),
        "v0" if transaction.loaded_address_count == 0 => Ok(()),
        "v0" => anyhow::bail!(
            "message resolves {} address lookup table entries; lookup tables are normalized \
             but not executed",
            transaction.loaded_address_count
        ),
        other => anyhow::bail!("unsupported message version {other}"),
    }
}

// ---------------------------------------------------------------------------
// Phase 9: protocol semantics for production-corpus construction
// ---------------------------------------------------------------------------

/// What an interaction *does*, in the protocol's own vocabulary.
///
/// Classified by the adapter, which understands the supported protocol - never
/// inferred from arbitrary bytecode, and never from a language model. The set is
/// the union across supported protocols plus [`SemanticAction::Unknown`], so a
/// corpus record's action stays a stable string across adapter versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAction {
    Transfer,
    Mint,
    Burn,
    Approve,
    Revoke,
    Deposit,
    Withdraw,
    Stake,
    Unstake,
    Claim,
    Rebalance,
    Liquidate,
    Swap,
    /// The adapter recognized the transaction but has no name for it. A record
    /// carrying this is still auditable; it simply cannot be stratified by action.
    Unknown,
}

impl SemanticAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transfer => "transfer",
            Self::Mint => "mint",
            Self::Burn => "burn",
            Self::Approve => "approve",
            Self::Revoke => "revoke",
            Self::Deposit => "deposit",
            Self::Withdraw => "withdraw",
            Self::Stake => "stake",
            Self::Unstake => "unstake",
            Self::Claim => "claim",
            Self::Rebalance => "rebalance",
            Self::Liquidate => "liquidate",
            Self::Swap => "swap",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for SemanticAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable identity of the economic entity an interaction touches.
///
/// This exists so that a hundred interactions with one token account are not
/// counted as a hundred independent exposures. `kind` names what sort of entity
/// it is in the protocol's terms; `id` is the address that identifies it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityId {
    pub kind: String,
    pub id: String,
}

impl EntityId {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.id)
    }
}

/// A deterministic value extracted from historical state by the adapter.
///
/// Integer rather than floating point throughout: these feed selection ranking
/// and the canonical corpus hash, both of which must reproduce exactly.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeatureValue {
    Integer { value: u128 },
    Text { value: String },
}

impl FeatureValue {
    pub fn integer(value: impl Into<u128>) -> Self {
        Self::Integer {
            value: value.into(),
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self::Text {
            value: value.into(),
        }
    }

    pub fn as_integer(&self) -> Option<u128> {
        match self {
            Self::Integer { value } => Some(*value),
            Self::Text { .. } => None,
        }
    }

    pub fn render(&self) -> String {
        match self {
            Self::Integer { value } => value.to_string(),
            Self::Text { value } => value.clone(),
        }
    }
}

/// One named, protocol-defined observation about the historical state.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StateFeature {
    pub name: String,
    pub value: FeatureValue,
}

impl StateFeature {
    pub fn integer(name: impl Into<String>, value: impl Into<u128>) -> Self {
        Self {
            name: name.into(),
            value: FeatureValue::integer(value),
        }
    }

    pub fn text(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: FeatureValue::text(value),
        }
    }
}

/// How close a historical state sits to an economic threshold the protocol
/// actually defines.
///
/// Boundaries are declared by the adapter, never invented generically: a
/// distance is only meaningful when something real happens on the other side of
/// it. `distance_bps` is basis points of the reference quantity, so 0 means the
/// state sits exactly on the boundary and 10_000 means it is a whole reference
/// away.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BoundaryDistance {
    pub name: String,
    pub description: String,
    pub distance_bps: u32,
    /// The quantities the distance was computed from, so a reader can check it.
    pub reference: String,
    pub observed: String,
}

impl BoundaryDistance {
    /// Distance between `observed` and `reference`, in basis points of
    /// `reference`. A zero reference yields `10_000` - maximally far - rather
    /// than a division by zero, since nothing can be near a boundary that has no
    /// magnitude.
    pub fn from_quantities(
        name: impl Into<String>,
        description: impl Into<String>,
        observed: u128,
        reference: u128,
    ) -> Self {
        let distance_bps = if reference == 0 {
            10_000
        } else {
            let gap = observed.abs_diff(reference);
            u32::try_from(gap.saturating_mul(10_000) / reference).unwrap_or(u32::MAX)
        };
        Self {
            name: name.into(),
            description: description.into(),
            distance_bps,
            reference: reference.to_string(),
            observed: observed.to_string(),
        }
    }
}

/// What an adapter knows about one program.
pub trait ProtocolAdapter: Sync {
    fn name(&self) -> &'static str;

    /// Version of this adapter's semantic interpretation.
    ///
    /// Recorded in a corpus manifest and folded into the canonical hash, so a
    /// corpus built under a materially different interpretation cannot silently
    /// appear identical to an older one. Bump it whenever `semantic_action`,
    /// `economic_entity_id`, `state_features` or `boundaries` change meaning.
    fn adapter_version(&self) -> u32 {
        1
    }

    /// What this interaction does, in the protocol's vocabulary.
    fn semantic_action(&self, _transaction: &HistoricalTransaction) -> SemanticAction {
        SemanticAction::Unknown
    }

    /// The economic entity this interaction primarily affects.
    ///
    /// `None` where the protocol has no stable notion of one, in which case the
    /// corpus counts observations only and says so rather than inventing an
    /// entity per transaction.
    fn economic_entity_id(
        &self,
        _transaction: &HistoricalTransaction,
        _accounts: &[NamedAccount],
    ) -> Option<EntityId> {
        None
    }

    /// Deterministic features of the historical pre-state, for ranking and
    /// stratification.
    fn state_features(
        &self,
        _transaction: &HistoricalTransaction,
        _accounts: &[NamedAccount],
    ) -> Vec<StateFeature> {
        Vec::new()
    }

    /// Distances to economic thresholds this protocol defines.
    fn boundaries(
        &self,
        _transaction: &HistoricalTransaction,
        _accounts: &[NamedAccount],
    ) -> Vec<BoundaryDistance> {
        Vec::new()
    }

    fn program_id(&self) -> &'static str;

    /// Reject any transaction outside the contract this adapter replays exactly.
    ///
    /// The bar is exactness, not best effort: a shape this adapter cannot prove
    /// is an error, never a silently approximated replay.
    fn accept(&self, transaction: &HistoricalTransaction) -> Result<()>;

    /// Whether this adapter's contract admits cross-program invocation.
    ///
    /// Defaults to `false`, so an adapter written before CPI replay existed
    /// keeps its narrower guarantee rather than inheriting a wider one.
    fn supports_cpi(&self) -> bool {
        false
    }

    /// Programs the supported contract is known to reach, directly or by CPI.
    ///
    /// This is a floor, not the resolved set: dependencies are discovered from
    /// the transaction itself and this list only adds what the protocol knows
    /// it needs. Declaring a program here never causes it to execute; it causes
    /// it to be resolved at the historical slot and recorded.
    fn dependency_programs(&self) -> &'static [&'static str] {
        &[]
    }

    /// Stable semantic label for the message key at `index`.
    ///
    /// Labels are how diffs and reports name accounts, so they must be derived
    /// from the transaction's structure rather than from an address, and must be
    /// unique within one record.
    fn label(&self, transaction: &HistoricalTransaction, index: usize) -> String;

    /// Decode an account's bytes. `None` for accounts this adapter does not own.
    fn decode(&self, account: &AccountSnapshot) -> Option<SemanticAccount>;

    /// Accounts the protocol knows this transaction depends on.
    ///
    /// A discovery route of its own, beside the message's instruction metas and
    /// the validator's inner instructions: what a program reads is protocol
    /// knowledge, and an account reached only that way would otherwise be
    /// acquired without anything recording why it was needed.
    fn required_accounts(&self, _transaction: &HistoricalTransaction) -> Vec<String> {
        Vec::new()
    }

    /// Prove archived boundary snapshots against validator-observed metadata.
    ///
    /// Returns the human-readable assumptions the proof rests on, which are
    /// recorded in the replay record so a reader can see what was and was not
    /// established independently of the replay itself.
    fn prove_boundaries(
        &self,
        transaction: &HistoricalTransaction,
        pre: &[NamedAccount],
        post: &[NamedAccount],
    ) -> Result<Vec<String>>;

    /// Economic interpretation of one V1/V2 execution pair.
    fn interpret(
        &self,
        accounts: &[NamedAccount],
        v1: &ExecutionResult,
        v2: &ExecutionResult,
    ) -> Vec<EconomicChange>;

    /// Headline protocol quantities produced by one execution.
    ///
    /// These are what the protocol's users would recognize - the amount
    /// deposited, the shares received - rather than the account fields they are
    /// derived from. Reported for both builds side by side, so a preserved
    /// outcome is stated rather than inferred from silence. Empty by default:
    /// an adapter that has nothing to derive reports nothing rather than
    /// inventing a summary.
    fn summarize(
        &self,
        _accounts: &[NamedAccount],
        _result: &ExecutionResult,
    ) -> Vec<SemanticField> {
        Vec::new()
    }

    /// This protocol's stable id in the finding vocabulary.
    ///
    /// Defaults to [`ProtocolAdapter::name`], which is already the slug teams
    /// see. `None` means this adapter does not participate in expectation
    /// review at all, which is the safe default: a protocol whose subjects have
    /// not been deliberately promoted should not have teams writing TOML
    /// against them.
    fn protocol_id(&self) -> Option<crate::semantics::ProtocolId> {
        crate::semantics::ProtocolId::new(self.name()).ok()
    }

    /// The precise action in the finding vocabulary.
    ///
    /// Finer than [`ProtocolAdapter::semantic_action`], which is deliberately
    /// coarse so the corpus selector can stratify on it. An expectation keys on
    /// this one: `withdraw_sol` must not silently widen to cover a future
    /// `withdraw_stake`.
    fn action_id(
        &self,
        _transaction: &HistoricalTransaction,
    ) -> Option<crate::semantics::ActionId> {
        None
    }

    /// What this observation is *able* to measure, whether or not anything
    /// changed.
    ///
    /// Derived from the observation's own shape - which instruction it is and
    /// which accounts it names - and never from what happened to differ between
    /// two builds. That is the whole point: it is what lets the review engine
    /// tell "the candidate stopped doing this" from "this corpus cannot tell
    /// you", and a capability computed from observed differences would collapse
    /// the two.
    fn evaluable_subjects(
        &self,
        _transaction: &HistoricalTransaction,
        _accounts: &[NamedAccount],
    ) -> Vec<crate::semantics::EvaluableSubject> {
        Vec::new()
    }

    /// Which decoded `(account label, field)` a named subject reads.
    ///
    /// The generic layer cannot tell a decoded change that a named finding
    /// already speaks for from one it silently drops. This is how an adapter
    /// says which is which — per subject, so the mapping can be checked against
    /// the findings actually emitted for *this* observation rather than against
    /// a static list that is true for some operations and not others.
    ///
    /// `pool-mint/supply` is the example that matters: it is the burn on a
    /// withdrawal and a consequence of the mint on a deposit, and a flat list
    /// treated it as spoken for either way, including when nothing was named.
    fn decoded_source_of(&self, _subject: &str) -> Option<(&'static str, &'static str)> {
        None
    }

    /// Byte ranges this adapter decodes out of one account.
    ///
    /// Lets the generic layer prove that a raw byte change is accounted for.
    /// Bytes outside these ranges are not explained by any economic finding,
    /// however many were reported: a candidate that alters a manager key while
    /// changing a share calculation has done two things, and only one of them
    /// is nameable.
    fn decoded_byte_ranges(&self, _account_label: &str) -> &'static [std::ops::Range<usize>] {
        &[]
    }

    /// Named differences between one V1/V2 pair, in the finding vocabulary.
    ///
    /// Only subjects this adapter has deliberately promoted appear here. A
    /// field in `summarize` is a reporting detail; a subject here is a
    /// compatibility commitment, because teams will name it in a file they
    /// expect to keep working.
    fn named_findings(
        &self,
        _transaction: &HistoricalTransaction,
        _accounts: &[NamedAccount],
        _v1: &ExecutionResult,
        _v2: &ExecutionResult,
    ) -> Vec<crate::semantics::NamedFinding> {
        Vec::new()
    }
}

/// Pair two summaries field by field into one comparison.
///
/// Fields are matched by name and reported in the order the V1 summary produced
/// them, which keeps the rendering stable across runs. A field only one side
/// produced is dropped rather than compared against a blank: the adapters here
/// derive the same fields from either execution, so a missing one means the
/// candidate failed before producing it, and the failure is reported elsewhere.
pub fn pair_summaries(v1: &[SemanticField], v2: &[SemanticField]) -> Vec<EconomicObservation> {
    v1.iter()
        .filter_map(|field| {
            let other = v2.iter().find(|candidate| candidate.name == field.name)?;
            Some(EconomicObservation {
                field: field.name.clone(),
                v1: field.value.render(),
                v2: other.value.render(),
                delta: match (field.value.as_quantity(), other.value.as_quantity()) {
                    (Some(before), Some(after)) => after.delta(before),
                    _ => None,
                },
                economic: field.economic,
            })
        })
        .collect()
}

/// Every adapter the engine knows about.
///
/// Deliberately a lookup rather than a registration hook: an adapter that is
/// not compiled in cannot be selected by a corpus file, so a record can never
/// name a protocol this build cannot actually reason about.
pub fn adapter_for(program_id: &str) -> Option<&'static dyn ProtocolAdapter> {
    const TOKEN_2022: token2022::Token2022Adapter = token2022::Token2022Adapter;
    const STAKE_POOL: stake_pool::StakePoolAdapter = stake_pool::StakePoolAdapter;
    if program_id == TOKEN_2022.program_id() {
        return Some(&TOKEN_2022);
    }
    if program_id == STAKE_POOL.program_id() {
        return Some(&STAKE_POOL);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_render_against_their_mints_decimals() {
        assert_eq!(TokenQuantity::new(7_157, 8).to_string(), "0.00007157");
        assert_eq!(TokenQuantity::new(50_000_000, 6).to_string(), "50.000000");
        assert_eq!(TokenQuantity::new(42, 0).to_string(), "42");
    }

    #[test]
    fn deltas_carry_a_sign_and_keep_full_u64_range() {
        let before = TokenQuantity::new(u64::MAX, 6);
        let after = TokenQuantity::new(0, 6);
        let delta = after.delta(before).expect("same mint");
        assert_eq!(delta.base_units, -i128::from(u64::MAX));
        assert!(delta.to_string().starts_with('-'));
    }

    /// Subtracting amounts from different mints is meaningless, not imprecise.
    #[test]
    fn quantities_from_different_mints_do_not_subtract() {
        assert!(TokenQuantity::new(1, 6)
            .delta(TokenQuantity::new(1, 8))
            .is_none());
    }

    #[test]
    fn quantities_serialize_as_strings_not_json_numbers() {
        let json = serde_json::to_string(&FieldValue::quantity(7_157, 8)).unwrap();
        assert!(json.contains("\"0.00007157\""), "{json}");
    }

    #[test]
    fn signed_quantities_round_trip_through_their_string_form() {
        for value in [
            SignedTokenQuantity {
                base_units: -10_000_000,
                decimals: 6,
            },
            SignedTokenQuantity {
                base_units: 7_157,
                decimals: 8,
            },
            SignedTokenQuantity {
                base_units: 0,
                decimals: 0,
            },
            SignedTokenQuantity {
                base_units: -i128::from(u64::MAX),
                decimals: 9,
            },
        ] {
            let json = serde_json::to_string(&value).unwrap();
            let parsed: SignedTokenQuantity = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, value, "round trip of {json}");
        }
    }

    #[test]
    fn summaries_pair_by_name_and_keep_v1_order() {
        let v1 = vec![
            SemanticField {
                name: "pool_tokens_received".into(),
                value: FieldValue::quantity(77_645_365, 9),
                economic: true,
            },
            SemanticField {
                name: "sol_deposited".into(),
                value: FieldValue::quantity(100_000_000, 9),
                economic: true,
            },
        ];
        let v2 = vec![
            SemanticField {
                name: "sol_deposited".into(),
                value: FieldValue::quantity(100_000_000, 9),
                economic: true,
            },
            SemanticField {
                name: "pool_tokens_received".into(),
                value: FieldValue::quantity(77_567_719, 9),
                economic: true,
            },
        ];
        let paired = pair_summaries(&v1, &v2);
        assert_eq!(paired[0].field, "pool_tokens_received");
        assert_eq!(paired[0].v1, "0.077645365");
        assert_eq!(paired[0].v2, "0.077567719");
        assert_eq!(paired[0].delta.unwrap().base_units, -77_646);
        assert_eq!(paired[1].field, "sol_deposited");
        assert!(paired[1].delta.unwrap().is_zero());
    }

    /// A preserved quantity is still reported. Silence is not a result.
    #[test]
    fn an_unchanged_summary_field_is_still_reported() {
        let field = vec![SemanticField {
            name: "pool_tokens_received".into(),
            value: FieldValue::quantity(77_645_365, 9),
            economic: true,
        }];
        let paired = pair_summaries(&field, &field);
        assert_eq!(paired.len(), 1);
        assert_eq!(paired[0].v1, paired[0].v2);
        assert!(paired[0].delta.unwrap().is_zero());
    }

    #[test]
    fn an_economic_change_round_trips() {
        let change = EconomicChange {
            account_label: "source".into(),
            account_kind: "token-account".into(),
            field: "amount".into(),
            v1: "0.000000".into(),
            v2: "1.000000".into(),
            delta: Some(SignedTokenQuantity {
                base_units: 1_000_000,
                decimals: 6,
            }),
        };
        let json = serde_json::to_string(&change).unwrap();
        assert_eq!(
            serde_json::from_str::<EconomicChange>(&json).unwrap(),
            change
        );
    }

    #[test]
    fn an_unknown_program_has_no_adapter() {
        assert!(adapter_for("11111111111111111111111111111111").is_none());
        assert!(adapter_for(token2022::PROGRAM_ID).is_some());
        assert!(adapter_for(stake_pool::PROGRAM_ID).is_some());
    }

    /// The CPI contract is opt-in per adapter. Token-2022's path was proved
    /// without it and keeps the narrower guarantee.
    #[test]
    fn cpi_support_is_declared_per_adapter() {
        assert!(!adapter_for(token2022::PROGRAM_ID).unwrap().supports_cpi());
        assert!(adapter_for(stake_pool::PROGRAM_ID).unwrap().supports_cpi());
        assert!(adapter_for(token2022::PROGRAM_ID)
            .unwrap()
            .dependency_programs()
            .is_empty());
    }
}

#[cfg(test)]
mod malformed_input {
    use super::*;

    /// `decimals` is a `u8` read from a decoded mint, so it can name more
    /// places than a `u128` scale can represent. That must neither abort the
    /// process nor produce a string the wire format cannot read back.
    #[test]
    fn an_extreme_decimal_count_still_renders_a_readable_decimal() {
        for decimals in [38_u8, 39, 100, u8::MAX] {
            let rendered = TokenQuantity::new(1_500, decimals).to_string();
            let (whole, fraction) = rendered.split_once('.').expect("a decimal point");
            assert_eq!(whole, "0");
            assert_eq!(fraction.len(), usize::from(decimals), "{decimals}");
            assert!(
                fraction.bytes().all(|b| b.is_ascii_digit()),
                "{rendered} is not a plain decimal"
            );
        }
        // The ordinary paths are unchanged.
        assert_eq!(
            TokenQuantity::new(1_500_000_000, 9).to_string(),
            "1.500000000"
        );
        assert_eq!(TokenQuantity::new(7_157, 8).to_string(), "0.00007157");
        assert_eq!(TokenQuantity::new(42, 0).to_string(), "42");
    }

    /// The defect the previous fix introduced. "Renderable" is not the
    /// contract; round-tripping is, and asserting only that the string was
    /// non-empty is how exponent notation reached the wire.
    #[test]
    fn an_extreme_quantity_survives_the_wire_format() {
        let value = crate::semantics::SemanticValue::quantity(1_500, 39);
        let json = serde_json::to_string(&value).unwrap();
        assert!(
            !json.contains('e'),
            "exponent notation cannot be read back: {json}"
        );
        assert_eq!(
            serde_json::from_str::<crate::semantics::SemanticValue>(&json).unwrap(),
            value
        );
    }
}
