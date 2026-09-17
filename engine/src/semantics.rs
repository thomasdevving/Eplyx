//! The finding vocabulary: what a change *is*, independent of how big it was.
//!
//! This is the first public contract a protocol team writes against. An
//! `expected-changes.toml` keys on these names, so renaming one is a migration
//! for everybody who uses Eplyx, not a refactor. The shape is therefore fixed
//! deliberately and versioned explicitly.
//!
//! ```text
//! protocol / action / domain / subject / change
//!
//! spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased
//! spl-stake-pool/withdraw_sol/execution/transaction/now_reverts
//! token-2022/approve/authority/delegate/changed
//! ```
//!
//! Ownership is split on purpose. The **adapter** owns `protocol`, `action` and
//! `subject`, because only it knows that a stake pool mints pool tokens. The
//! **engine** owns `domain` and `change`, because those are the axes every
//! protocol shares — and if adapters chose them, the same concept would arrive
//! as `less_received`, `output_decreased`, `user_value_lower` and
//! `reduced_output` from four different adapters.
//!
//! Deliberately *not* part of the identity: severity, the V1 and V2 values, the
//! delta, how many observations or entities were affected, which observations
//! they were, and the bundle hash. Those are properties of one run over one
//! corpus. The identity has to survive a corpus refresh, or an expectation
//! declared last month would stop matching for reasons that have nothing to do
//! with the candidate.
//!
//! The adapter *version* is also not in the identity. Putting it there would
//! turn every adapter bugfix into a breaking expectation migration. When the
//! meaning of a subject genuinely changes, [`SEMANTIC_SCHEMA_VERSION`] is what
//! moves, and expectations written against an incompatible schema are refused
//! rather than silently reinterpreted.

use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::diff::Severity;
use crate::protocol::TokenQuantity;

/// The meaning of the vocabulary below, not the code that produces it.
///
/// Bumped when a subject's meaning changes, never when an adapter is fixed.
///
/// 2: `pool_tokens_burned` is the mint's supply decrease. Under 1 it was the
/// source account's debit, which on a withdrawal includes a manager fee
/// transferred to another account — so an expectation written against burn or
/// supply semantics was evaluated against a different quantity. The holder's
/// debit is still available, under `pool_tokens_debited`. Expectation files
/// written against schema 1 are refused rather than reinterpreted, which is
/// what this constant exists for.
pub const SEMANTIC_SCHEMA_VERSION: u32 = 2;

/// Which axis of behaviour a finding is about.
///
/// Engine-owned. Four axes, chosen because they are the ones every protocol
/// has; a fifth should arrive with a schema bump and a reason, not because one
/// adapter wanted somewhere to put something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingDomain {
    /// Whether the transaction ran at all, and how it ended.
    Execution,
    /// Quantities that belong to somebody: balances, supply, fees, shares.
    Economic,
    /// Who is permitted to act: delegates, owners, signers.
    Authority,
    /// Whether a thing exists: accounts, positions, pools.
    Lifecycle,
}

impl FindingDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Execution => "execution",
            Self::Economic => "economic",
            Self::Authority => "authority",
            Self::Lifecycle => "lifecycle",
        }
    }
}

/// What happened to the subject.
///
/// Engine-owned, and kept small on purpose. Inventing twenty change kinds for
/// protocols that do not exist yet would guarantee that half of them are wrong.
/// If `liquidatable false -> true` later needs its own kind rather than
/// [`ChangeKind::Enabled`], that arrives with a schema bump.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Increased,
    Decreased,
    /// Changed in a way that is not ordered: an address, an opaque field.
    Changed,

    NowReverts,
    NowSucceeds,

    Enabled,
    Disabled,

    Created,
    Removed,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Increased => "increased",
            Self::Decreased => "decreased",
            Self::Changed => "changed",
            Self::NowReverts => "now_reverts",
            Self::NowSucceeds => "now_succeeds",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Created => "created",
            Self::Removed => "removed",
        }
    }

    /// The direction a quantity moved, from its signed delta.
    pub fn from_delta(delta: i128) -> Self {
        match delta {
            d if d > 0 => Self::Increased,
            d if d < 0 => Self::Decreased,
            _ => Self::Changed,
        }
    }
}

macro_rules! slug {
    ($name:ident, $what:literal) => {
        /// An adapter-owned name in the finding vocabulary.
        ///
        /// Lowercase, ASCII, and free of `/` so the canonical string can never
        /// be ambiguous to parse. The engine validates the shape; the adapter
        /// owns which names exist.
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                validate_slug(&value, $what)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = anyhow::Error;
            fn try_from(value: String) -> Result<Self> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

slug!(ProtocolId, "a protocol id");
slug!(ActionId, "an action id");
slug!(SemanticSubject, "a subject");

/// Names must be stable, typeable and unambiguous in the canonical string.
fn validate_slug(value: &str, what: &str) -> Result<()> {
    if value.is_empty() {
        bail!("{what} cannot be empty");
    }
    if value.len() > 64 {
        bail!("{what} is longer than 64 characters: {value:?}");
    }
    if !value.starts_with(|c: char| c.is_ascii_lowercase()) {
        bail!("{what} must start with a lowercase ASCII letter: {value:?}");
    }
    if let Some(bad) = value
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-'))
    {
        bail!(
            "{what} may contain only lowercase ASCII letters, digits, '_' and '-'; \
             found {bad:?} in {value:?}"
        );
    }
    Ok(())
}

/// What a finding *is*.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FindingFingerprint {
    pub protocol: ProtocolId,
    pub action: ActionId,
    pub domain: FindingDomain,
    pub subject: SemanticSubject,
    pub change: ChangeKind,
}

impl FindingFingerprint {
    /// What this finding would need in order to be measurable at all.
    ///
    /// Dropping the change kind is the whole point: it is what lets the review
    /// engine ask "could this have been observed?" separately from "was it?".
    pub fn evaluable_subject(&self) -> EvaluableSubject {
        EvaluableSubject {
            protocol: self.protocol.clone(),
            action: self.action.clone(),
            domain: self.domain,
            subject: self.subject.clone(),
        }
    }
}

impl fmt::Display for FindingFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}/{}",
            self.protocol,
            self.action,
            self.domain.as_str(),
            self.subject,
            self.change.as_str()
        )
    }
}

impl FromStr for FindingFingerprint {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        let parts: Vec<&str> = text.split('/').collect();
        let [protocol, action, domain, subject, change] = parts.as_slice() else {
            bail!(
                "a fingerprint has five '/'-separated parts \
                 (protocol/action/domain/subject/change), found {} in {text:?}",
                parts.len()
            );
        };
        Ok(Self {
            protocol: ProtocolId::new(*protocol)?,
            action: ActionId::new(*action)?,
            domain: parse_enum(domain, "domain")?,
            subject: SemanticSubject::new(*subject)?,
            change: parse_enum(change, "change")?,
        })
    }
}

fn parse_enum<T: serde::de::DeserializeOwned>(value: &str, what: &str) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|_| anyhow::anyhow!("{value:?} is not a known {what}"))
}

/// Something one observation is able to measure, whether or not it changed.
///
/// This is what separates "the candidate no longer does that" from "this corpus
/// cannot tell you". Without it the two are only distinguishable by guessing,
/// and a team's intentional change would be reported as reverted whenever their
/// corpus lost coverage of it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EvaluableSubject {
    pub protocol: ProtocolId,
    pub action: ActionId,
    pub domain: FindingDomain,
    pub subject: SemanticSubject,
}

impl fmt::Display for EvaluableSubject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}",
            self.protocol,
            self.action,
            self.domain.as_str(),
            self.subject
        )
    }
}

/// A value a subject can take.
///
/// Quantities carry their own scale, as everywhere else in this engine, and
/// serialize as decimal strings rather than JSON numbers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SemanticValue {
    Quantity {
        #[serde(with = "quantity_string")]
        quantity: TokenQuantity,
    },
    Flag {
        value: bool,
    },
    Address {
        address: String,
    },
}

/// A quantity on the wire is a decimal string and nothing else.
///
/// Not a JSON number: `base_units` reaches 1.8e19, and most consumers parse a
/// JSON number into a double, which is exact only to 2^53. The fraction's digit
/// count *is* the decimal count, so the scale round-trips without a second
/// field that could disagree with it. Same shape `SignedTokenQuantity` already
/// uses.
mod quantity_string {
    use super::TokenQuantity;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &TokenQuantity, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<TokenQuantity, D::Error> {
        use serde::de::Error as _;
        let text = String::deserialize(d)?;
        let (whole, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));
        if whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(D::Error::custom(format!("invalid token amount {text:?}")));
        }
        let decimals = u8::try_from(fraction.len())
            .map_err(|_| D::Error::custom(format!("{text:?} has too many decimal places")))?;
        let base_units = format!("{whole}{fraction}")
            .parse::<u64>()
            .map_err(|_| D::Error::custom(format!("token amount {text:?} does not fit")))?;
        Ok(TokenQuantity::new(base_units, decimals))
    }
}

impl SemanticValue {
    pub fn quantity(base_units: u64, decimals: u8) -> Self {
        Self::Quantity {
            quantity: TokenQuantity::new(base_units, decimals),
        }
    }

    fn as_quantity(&self) -> Option<&TokenQuantity> {
        match self {
            Self::Quantity { quantity } => Some(quantity),
            _ => None,
        }
    }
}

/// One measured difference, named.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedFinding {
    pub fingerprint: FindingFingerprint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<SemanticValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<SemanticValue>,
    /// Magnitude of the change relative to the baseline, signed, in basis
    /// points. `None` when no relative measure is defined — see
    /// [`RelativeDelta`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_delta_bps: Option<i64>,
    /// What the diff layer rated this. Independent of review status: a finding
    /// is critical because of what it does, not because of whether anybody
    /// expected it.
    pub severity: Severity,
}

/// Why a relative bound could not be computed.
///
/// A bound that cannot be evaluated is never quietly treated as satisfied: a
/// zero baseline makes a percentage undefined, not zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndefinedBound {
    /// `baseline == 0`: there is no denominator.
    ZeroBaseline,
    /// The subject is not an ordered quantity, so "25 bps" means nothing.
    NotAQuantity,
    /// The two sides carry different decimal counts, so this contract has no
    /// evidence they are on the same scale.
    ///
    /// Not a claim that they are different assets - USDC at 6 decimals against
    /// USDC at 9 is more likely an adapter or schema fault than two currencies.
    /// Either way the comparison is refused: what is missing is the proof that
    /// the two numbers mean the same thing, and inventing it is exactly the
    /// step a gate must not take.
    IncomparableScales,
    /// One side was not measured.
    NotMeasured,
}

impl UndefinedBound {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ZeroBaseline => "baseline quantity is zero",
            Self::NotAQuantity => "subject is not an ordered quantity",
            Self::IncomparableScales => "the two sides use different decimal scales",
            Self::NotMeasured => "one side was not measured",
        }
    }
}

/// The relative size of a change, or why there is no such thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelativeDelta {
    Bps(i64),
    Undefined(UndefinedBound),
}

/// `|candidate - baseline| / |baseline|`, in basis points, rounded **away from
/// zero**.
///
/// Rounding away from zero is what makes a bound a bound. A true change of
/// 20.01 bps that truncated to 20 would slip through `max_delta_bps = 20`, and
/// a gate that can be passed by rounding is not a gate. The cost is that a
/// reported magnitude can overstate the true one by less than one basis point,
/// which is the safe direction.
///
/// Integer throughout: no floating point reaches this path, for the same reason
/// it does not reach the valuation path.
pub fn relative_delta(baseline: &SemanticValue, candidate: &SemanticValue) -> RelativeDelta {
    let (Some(baseline), Some(candidate)) = (baseline.as_quantity(), candidate.as_quantity())
    else {
        return RelativeDelta::Undefined(UndefinedBound::NotAQuantity);
    };
    // Same scale, or no comparison: a ratio between two differently scaled
    // quantities is not merely imprecise, it is unproven.
    if baseline.decimals != candidate.decimals {
        return RelativeDelta::Undefined(UndefinedBound::IncomparableScales);
    }
    if baseline.base_units == 0 {
        return RelativeDelta::Undefined(UndefinedBound::ZeroBaseline);
    }
    let base = i128::from(baseline.base_units);
    let delta = i128::from(candidate.base_units) - base;
    let magnitude = (delta.unsigned_abs() * 10_000).div_ceil(base.unsigned_abs());
    // A magnitude beyond i64 cannot be expressed, and no bound worth writing is
    // anywhere near it. Saturating keeps the type honest without pretending to
    // a precision the report cannot carry.
    let magnitude = i64::try_from(magnitude).unwrap_or(i64::MAX);
    RelativeDelta::Bps(if delta < 0 { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(text: &str) -> FindingFingerprint {
        text.parse().expect("a valid fingerprint")
    }

    /// The canonical string is what appears in reports, the CLI and GitHub
    /// output. If it does not survive a round trip it is not an identity.
    #[test]
    fn the_canonical_string_round_trips() {
        for text in [
            "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased",
            "spl-stake-pool/withdraw_sol/execution/transaction/now_reverts",
            "spl-stake-pool/withdraw_sol/economic/sol_received_by_user/decreased",
            "token-2022/mint_to/economic/token_supply/increased",
            "token-2022/approve/authority/delegate/changed",
        ] {
            let parsed = fingerprint(text);
            assert_eq!(parsed.to_string(), text);
            assert_eq!(fingerprint(&parsed.to_string()), parsed);
        }
    }

    #[test]
    fn a_fingerprint_survives_a_json_round_trip() {
        let original = fingerprint("token-2022/approve/authority/delegate/changed");
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(
            serde_json::from_str::<FindingFingerprint>(&json).unwrap(),
            original
        );
    }

    #[test]
    fn malformed_fingerprints_are_rejected() {
        for text in [
            "spl-stake-pool/deposit_sol/economic/pool_tokens_received",
            "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased/extra",
            "spl-stake-pool/deposit_sol/made_up_domain/pool_tokens_received/decreased",
            "spl-stake-pool/deposit_sol/economic/pool_tokens_received/went_down",
            "SPL-Stake-Pool/deposit_sol/economic/pool_tokens_received/decreased",
            "spl-stake-pool/deposit sol/economic/pool_tokens_received/decreased",
            "2022/deposit_sol/economic/pool_tokens_received/decreased",
        ] {
            assert!(
                text.parse::<FindingFingerprint>().is_err(),
                "accepted {text:?}"
            );
        }
    }

    /// The identity must survive a corpus refresh. Everything that varies with
    /// one run over one corpus lives outside it.
    #[test]
    fn run_specific_facts_are_not_part_of_the_identity() {
        let json = serde_json::to_value(fingerprint(
            "spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased",
        ))
        .unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["action", "change", "domain", "protocol", "subject"]);
        for absent in [
            "severity",
            "baseline",
            "candidate",
            "delta",
            "relative_delta_bps",
            "affected_observations",
            "affected_entities",
            "observation_ids",
            "bundle_sha256",
            "adapter_version",
        ] {
            assert!(
                json.get(absent).is_none(),
                "{absent} leaked into the identity"
            );
        }
    }

    /// An adapter bugfix must not invalidate everybody's expectations, so the
    /// adapter version is carried beside the fingerprint, never inside it.
    #[test]
    fn the_adapter_version_is_not_in_the_identity() {
        assert!(
            "spl-stake-pool@2/deposit_sol/economic/pool_tokens_received/decreased"
                .parse::<FindingFingerprint>()
                .is_err()
        );
    }

    /// Dropping the change kind is what lets the review engine ask whether a
    /// declaration *could* have been observed, separately from whether it was.
    #[test]
    fn the_evaluable_subject_drops_only_the_change() {
        let decreased =
            fingerprint("spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased");
        let increased =
            fingerprint("spl-stake-pool/deposit_sol/economic/pool_tokens_received/increased");
        assert_eq!(decreased.evaluable_subject(), increased.evaluable_subject());
        assert_eq!(
            decreased.evaluable_subject().to_string(),
            "spl-stake-pool/deposit_sol/economic/pool_tokens_received"
        );

        let other_action =
            fingerprint("spl-stake-pool/withdraw_sol/economic/pool_tokens_received/decreased");
        assert_ne!(
            decreased.evaluable_subject(),
            other_action.evaluable_subject(),
            "an expectation for one action must not be satisfied by another"
        );
    }

    // ---- relative bounds ------------------------------------------------

    fn bps(baseline: u64, candidate: u64) -> RelativeDelta {
        relative_delta(
            &SemanticValue::quantity(baseline, 9),
            &SemanticValue::quantity(candidate, 9),
        )
    }

    #[test]
    fn a_relative_delta_is_measured_against_the_baseline() {
        // 100.000 -> 99.800 is 20 bps down.
        assert_eq!(bps(100_000, 99_800), RelativeDelta::Bps(-20));
        assert_eq!(bps(100_000, 100_200), RelativeDelta::Bps(20));
        assert_eq!(bps(100_000, 100_000), RelativeDelta::Bps(0));
    }

    /// A gate that can be passed by rounding is not a gate.
    #[test]
    fn magnitudes_round_away_from_zero() {
        // 20.01 bps down: truncation would report 20 and slip through a
        // max_delta_bps of 20.
        let delta = bps(1_000_000, 997_999);
        assert_eq!(delta, RelativeDelta::Bps(-21));

        let up = bps(1_000_000, 1_002_001);
        assert_eq!(up, RelativeDelta::Bps(21));

        // An exact bound stays exact rather than being pushed over.
        assert_eq!(bps(1_000_000, 998_000), RelativeDelta::Bps(-20));
    }

    /// Undefined is not zero, and not "fine".
    #[test]
    fn a_zero_baseline_has_no_relative_delta() {
        assert_eq!(
            bps(0, 500),
            RelativeDelta::Undefined(UndefinedBound::ZeroBaseline)
        );
        assert_eq!(
            bps(0, 0),
            RelativeDelta::Undefined(UndefinedBound::ZeroBaseline)
        );
    }

    /// Refused for lack of evidence that the two numbers share a scale, which
    /// is not the same as having established that they are different assets.
    #[test]
    fn quantities_on_different_scales_are_not_compared() {
        assert_eq!(
            relative_delta(
                &SemanticValue::quantity(100, 9),
                &SemanticValue::quantity(100, 6),
            ),
            RelativeDelta::Undefined(UndefinedBound::IncomparableScales)
        );
    }

    #[test]
    fn an_unordered_subject_has_no_relative_delta() {
        assert_eq!(
            relative_delta(
                &SemanticValue::Flag { value: false },
                &SemanticValue::Flag { value: true },
            ),
            RelativeDelta::Undefined(UndefinedBound::NotAQuantity)
        );
        assert_eq!(
            relative_delta(
                &SemanticValue::Address {
                    address: "a".into()
                },
                &SemanticValue::quantity(1, 0),
            ),
            RelativeDelta::Undefined(UndefinedBound::NotAQuantity)
        );
    }

    /// No bound anybody writes is near i64::MAX, and saturating keeps the
    /// reported type honest rather than wrapping into a small number.
    #[test]
    fn an_enormous_relative_change_saturates_rather_than_wrapping() {
        assert_eq!(bps(1, u64::MAX), RelativeDelta::Bps(i64::MAX));
    }

    #[test]
    fn a_direction_comes_from_the_signed_delta() {
        assert_eq!(ChangeKind::from_delta(5), ChangeKind::Increased);
        assert_eq!(ChangeKind::from_delta(-5), ChangeKind::Decreased);
        assert_eq!(ChangeKind::from_delta(0), ChangeKind::Changed);
    }

    /// Quantities serialize as decimal strings, never JSON numbers, for the
    /// same reason money does.
    #[test]
    fn a_quantity_does_not_serialize_as_a_json_number() {
        let value = SemanticValue::quantity(1_500_000_000, 9);
        let rendered = serde_json::to_string(&value).unwrap();
        assert_eq!(rendered, r#"{"kind":"quantity","quantity":"1.500000000"}"#);
        assert_eq!(
            serde_json::from_str::<SemanticValue>(&rendered).unwrap(),
            value,
            "the scale round-trips from the fraction's digit count"
        );

        // A value past 2^53, where a JSON number would silently lose precision.
        let large = SemanticValue::quantity(18_446_744_073_709_551_615, 0);
        let rendered = serde_json::to_string(&large).unwrap();
        assert!(rendered.contains(r#""18446744073709551615""#), "{rendered}");
        assert_eq!(
            serde_json::from_str::<SemanticValue>(&rendered).unwrap(),
            large
        );
    }
}
