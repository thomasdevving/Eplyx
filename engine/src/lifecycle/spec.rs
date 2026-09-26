//! Declared lifecycle terms. Unknown terms never imply an executable mechanism.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::policy::{LifecycleScenario, LifecycleSourceKind, LifecycleStatus};
use crate::change::{Activation, BoundChange, Change, ChangeBinding, ChangeMetadata, ChangeSpec};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub mint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ratio {
    pub numerator: String,
    pub denominator: String,
    pub rounding: crate::migration::spec::Rounding,
    pub fee_bps: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Eligibility {
    Unknown,
    /// A declaration, not verified eligibility or evidence of a mechanism.
    Declared {
        policy: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deadline {
    #[serde(with = "crate::numfmt::i64_string")]
    pub unix_timestamp: i64,
    pub after: LifecycleStatus,
}

/// Content and field-level provenance identify a declaration. Local artifact
/// paths, run timestamps and display descriptions do not identify the proposal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssertionSource {
    pub id: String,
    pub kind: LifecycleSourceKind,
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    pub supports: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleChange {
    pub asset: Asset,
    /// None means unknown; a destination is never invented from a ticker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<Asset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<Ratio>,
    pub eligibility: Eligibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<Deadline>,
    pub before: LifecycleStatus,
    pub after: LifecycleStatus,
    pub sources: Vec<AssertionSource>,
}

impl LifecycleChange {
    pub fn validate(&self, activation: Option<&Activation>) -> Result<()> {
        crate::change::canonical_address(&self.asset.mint, "lifecycle asset")?;
        let activation = activation.context("lifecycle activation time is required")?;
        ensure!(
            activation.slot.is_none(),
            "lifecycle activation uses a time, not a slot"
        );
        let at = activation
            .unix_timestamp
            .context("lifecycle activation time is required")?;
        ensure!(
            chrono::DateTime::from_timestamp(at, 0).is_some(),
            "invalid lifecycle activation time"
        );
        ensure!(
            matches!(
                self.before,
                LifecycleStatus::Active | LifecycleStatus::Unknown
            ),
            "invalid before policy"
        );
        ensure!(
            matches!(
                self.after,
                LifecycleStatus::TransitionRequired | LifecycleStatus::Unknown
            ),
            "invalid after policy"
        );
        let mut required = BTreeSet::from([
            "/activation/unix_timestamp",
            "/change/before",
            "/change/after",
        ]);
        if let Some(destination) = &self.destination {
            crate::change::canonical_address(&destination.mint, "lifecycle destination")?;
            ensure!(
                destination.mint != self.asset.mint,
                "successor is the source asset"
            );
            required.insert("/change/destination");
        }
        if let Some(ratio) = &self.ratio {
            ensure!(
                crate::migration::spec::canonical_u64(&ratio.numerator)? > 0
                    && crate::migration::spec::canonical_u64(&ratio.denominator)? > 0,
                "ratio terms must be positive"
            );
            ensure!(ratio.fee_bps <= 10_000, "fee exceeds 10000 basis points");
            ensure!(self.destination.is_some(), "a ratio requires a destination");
            required.insert("/change/ratio");
        }
        if let Eligibility::Declared { policy } = &self.eligibility {
            ensure!(!policy.trim().is_empty(), "empty eligibility declaration");
            required.insert("/change/eligibility");
        }
        if let Some(deadline) = &self.deadline {
            ensure!(
                deadline.unix_timestamp > at
                    && chrono::DateTime::from_timestamp(deadline.unix_timestamp, 0).is_some(),
                "deadline must follow activation"
            );
            ensure!(
                matches!(
                    deadline.after,
                    LifecycleStatus::PostDeadlineTransitionRequired
                        | LifecycleStatus::Expired
                        | LifecycleStatus::NoIssuerEntitlement
                        | LifecycleStatus::Unknown
                ),
                "invalid post-deadline policy"
            );
            required.insert("/change/deadline");
        }
        let mut covered = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for source in &self.sources {
            ensure!(
                !source.id.trim().is_empty() && ids.insert(&source.id),
                "missing or duplicate lifecycle source"
            );
            ensure!(
                !source.reference.trim().is_empty() && !source.supports.is_empty(),
                "missing lifecycle provenance"
            );
            if let Some(hash) = &source.content_sha256 {
                ensure!(
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "invalid lifecycle source hash"
                );
            }
            let mut unique = BTreeSet::new();
            for pointer in &source.supports {
                ensure!(
                    required.contains(pointer.as_str()) && unique.insert(pointer),
                    "unknown or duplicate lifecycle provenance pointer"
                );
                covered.insert(pointer.as_str());
            }
        }
        ensure!(
            covered == required,
            "lifecycle terms lack field-level provenance"
        );
        Ok(())
    }
}

impl ChangeSpec {
    pub fn compare_lifecycle(
        &self,
        snapshot: &super::LifecycleSnapshot,
        scenario: &LifecycleScenario,
        before: chrono::DateTime<chrono::Utc>,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<super::consequence::LifecycleImpactReport> {
        self.bind_lifecycle(scenario)?;
        super::consequence::LifecycleConsequenceEvaluator::evaluate(snapshot, scenario, before, at)
    }

    /// A captured notice/scenario establishes declared policy only. Ratios and
    /// eligibility absent from it stay unknown in the identifying ChangeSpec.
    pub fn lifecycle(scenario: &LifecycleScenario) -> Result<Self> {
        scenario.validate()?;
        ensure!(
            scenario.policy.effective_at.timestamp_subsec_nanos() == 0
                && scenario
                    .policy
                    .deadline
                    .as_ref()
                    .is_none_or(|d| d.at.timestamp_subsec_nanos() == 0),
            "lifecycle ChangeSpec requires whole-second boundaries"
        );
        let sources = scenario
            .sources
            .iter()
            .map(|source| -> Result<AssertionSource> {
                let supports = source
                    .supports
                    .iter()
                    .map(|p| {
                        Ok(match p.as_str() {
                            "/policy/effective_at" => "/activation/unix_timestamp",
                            "/policy/before" => "/change/before",
                            "/policy/after" => "/change/after",
                            "/policy/deadline" => "/change/deadline",
                            "/policy/successor" => "/change/destination",
                            _ => anyhow::bail!("unsupported policy pointer"),
                        }
                        .to_owned())
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(AssertionSource {
                    id: source.id.clone(),
                    kind: source.kind,
                    reference: source.reference.clone(),
                    content_sha256: source.content_sha256.clone(),
                    supports,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let spec = Self {
            schema_version: crate::change::CHANGE_SPEC_SCHEMA,
            change_spec_id: None,
            change: Change::LifecycleChange(Box::new(LifecycleChange {
                asset: Asset {
                    mint: scenario.policy.asset_mint.clone(),
                },
                destination: scenario.policy.successor.as_ref().map(|s| Asset {
                    mint: s.mint.clone(),
                }),
                ratio: None,
                eligibility: Eligibility::Unknown,
                deadline: scenario.policy.deadline.as_ref().map(|d| Deadline {
                    unix_timestamp: d.at.timestamp(),
                    after: d.after,
                }),
                before: scenario.policy.before,
                after: scenario.policy.after,
                sources,
            })),
            activation: Some(Activation {
                slot: None,
                unix_timestamp: Some(scenario.policy.effective_at.timestamp()),
            }),
            metadata: ChangeMetadata {
                label: Some(scenario.id.clone()),
                source: Some(scenario.change.description.clone()),
            },
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn as_lifecycle(&self) -> Option<&LifecycleChange> {
        match &self.change {
            Change::LifecycleChange(change) => Some(change),
            _ => None,
        }
    }

    pub fn bind_lifecycle(&self, scenario: &LifecycleScenario) -> Result<ChangeBinding> {
        self.validate()?;
        let change = self.as_lifecycle().context("expected lifecycle change")?;
        let derived = Self::lifecycle(scenario)?;
        // The scenario evaluator has no ratio or eligibility executor. Such
        // declarations need separate assurance evidence, never implicit success.
        let mut declared = derived.as_lifecycle().unwrap().clone();
        declared.ratio = change.ratio.clone();
        declared.eligibility = change.eligibility.clone();
        declared.sources = change.sources.clone();
        ensure!(
            &declared == change && derived.activation == self.activation,
            "scenario policy differs from lifecycle ChangeSpec"
        );
        // Source facts for evaluated policy must match, even when additional
        // declarations introduce separately sourced ratio/eligibility fields.
        for expected in &derived.as_lifecycle().unwrap().sources {
            ensure!(
                change.sources.iter().any(|s| s.id == expected.id
                    && s.kind == expected.kind
                    && s.reference == expected.reference
                    && s.content_sha256 == expected.content_sha256
                    && expected.supports.iter().all(|p| s.supports.contains(p))),
                "scenario provenance differs from lifecycle ChangeSpec"
            );
        }
        Ok(ChangeBinding {
            change_spec_id: self.id()?,
            change: BoundChange::LifecycleChange {
                asset_mint: change.asset.mint.clone(),
                destination_mint: change.destination.as_ref().map(|d| d.mint.clone()),
            },
        })
    }
}
