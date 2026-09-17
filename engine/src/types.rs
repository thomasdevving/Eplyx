//! The fixture format: a self-contained description of "this exact state, this
//! exact transaction".
//!
//! This is the format the whole system is built around. Everything downstream -
//! replay, minimisation, and eventually mainnet-derived corpora - reads and
//! writes these structures, so it is deliberately engine-agnostic: no litesvm
//! types appear here, and addresses are plain base58 strings.

use serde::{Deserialize, Serialize};

/// A raw account, exactly as the runtime will see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSnapshot {
    /// A decimal string: a pool holding 15 million SOL is 1.5e16 lamports, past
    /// the 2^53 where a JSON number starts rounding in most parsers.
    #[serde(with = "crate::numfmt::u64_string")]
    pub lamports: u64,
    /// base58 program address that owns this account.
    pub owner: String,
    #[serde(with = "crate::hexfmt")]
    pub data: Vec<u8>,
    #[serde(default)]
    pub executable: bool,
    #[serde(default)]
    pub rent_epoch: u64,
}

/// An account in a fixture, tagged with a stable human-readable label. Labels
/// are how diffs and reports refer to accounts, so they survive address changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedAccount {
    pub label: String,
    pub address: String,
    pub account: AccountSnapshot,
}

/// A signing identity, stored as its ed25519 seed so the fixture reproduces the
/// same addresses and signatures on every machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeypairSpec {
    pub label: String,
    #[serde(with = "crate::hexfmt")]
    pub seed: Vec<u8>,
    /// Derived address, denormalised so fixture files are readable without
    /// running key derivation.
    pub address: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountMetaSpec {
    pub address: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionSpec {
    pub program: String,
    pub accounts: Vec<AccountMetaSpec>,
    #[serde(with = "crate::hexfmt")]
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Healthy,
    Moderate,
    Small,
    Large,
    Fractional,
    NearLiquidation,
    Boundary,
    WithdrawBoundary,
    LiquidationBoundary,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Moderate => "moderate",
            Self::Small => "small",
            Self::Large => "large",
            Self::Fractional => "fractional",
            Self::NearLiquidation => "near-liquidation",
            Self::Boundary => "boundary",
            Self::WithdrawBoundary => "withdraw-boundary",
            Self::LiquidationBoundary => "liquidation-boundary",
        }
    }

    pub const ALL: [Category; 9] = [
        Category::Healthy,
        Category::Moderate,
        Category::Small,
        Category::Large,
        Category::Fractional,
        Category::NearLiquidation,
        Category::Boundary,
        Category::WithdrawBoundary,
        Category::LiquidationBoundary,
    ];
}

/// One unit of differential work: an initial state plus the single transaction
/// to replay against both program versions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fixture {
    /// Stable identifier. Reports and `eplyx reproduce` key off this.
    pub id: String,
    pub category: Category,
    /// Human-readable description of the action being replayed.
    pub scenario: String,
    /// Why this fixture is in the corpus.
    pub notes: String,
    pub keypairs: Vec<KeypairSpec>,
    pub accounts: Vec<NamedAccount>,
    /// Label of the keypair paying the transaction fee. Deliberately distinct
    /// from the position owner so fee deduction never contaminates the
    /// economic balance comparison.
    pub fee_payer: String,
    /// Labels of keypairs that must sign, in addition to the fee payer.
    pub signers: Vec<String>,
    pub instruction: InstructionSpec,
    /// Labels of accounts whose post-state is compared.
    pub watch: Vec<String>,
}

impl Fixture {
    pub fn account(&self, label: &str) -> Option<&NamedAccount> {
        self.accounts.iter().find(|a| a.label == label)
    }

    pub fn keypair(&self, label: &str) -> Option<&KeypairSpec> {
        self.keypairs.iter().find(|k| k.label == label)
    }

    /// Map an address back to its label, for readable diff output.
    pub fn label_for(&self, address: &str) -> Option<&str> {
        self.accounts
            .iter()
            .find(|a| a.address == address)
            .map(|a| a.label.as_str())
    }
}
