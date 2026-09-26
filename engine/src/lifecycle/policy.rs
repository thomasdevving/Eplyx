//! External lifecycle specifications. Policy assertions are never RPC facts.
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

use super::exposure::sha256;
/// Description in an archived scenario input; MAIN proposal identity lives in ChangeSpec.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioDescription {
    pub description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleStatus {
    Active,
    TransitionRequired,
    PostDeadlineTransitionRequired,
    Expired,
    NoIssuerEntitlement,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleDeadline {
    pub at: DateTime<Utc>,
    pub after: LifecycleStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuccessorAsset {
    pub mint: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetLifecyclePolicy {
    pub asset_mint: String,
    /// Inclusive boundary. Times earlier than this use `before`.
    pub effective_at: DateTime<Utc>,
    pub before: LifecycleStatus,
    pub after: LifecycleStatus,
    /// Inclusive expiration boundary; not a promise that transition is executable.
    pub deadline: Option<LifecycleDeadline>,
    /// Declarative identity only. No conversion route is inferred.
    pub successor: Option<SuccessorAsset>,
}

impl AssetLifecyclePolicy {
    pub fn status_at(&self, at: DateTime<Utc>) -> LifecycleStatus {
        if at < self.effective_at {
            self.before
        } else if let Some(deadline) = &self.deadline {
            if at >= deadline.at {
                deadline.after
            } else {
                self.after
            }
        } else {
            self.after
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleSourceKind {
    ExternalPolicy,
    ScenarioAssumption,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleSource {
    pub id: String,
    pub kind: LifecycleSourceKind,
    pub reference: String,
    pub description: String,
    pub captured_at: DateTime<Utc>,
    /// JSON pointers into this scenario, binding assertions to their sources.
    pub supports: Vec<String>,
    pub artifact: Option<String>,
    pub content_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleScenarioType {
    LifecycleChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleScenario {
    pub schema_version: u32,
    pub scenario_type: LifecycleScenarioType,
    pub id: String,
    pub scenario_version: String,
    pub captured_at: DateTime<Utc>,
    pub change: ScenarioDescription,
    pub policy: AssetLifecyclePolicy,
    pub sources: Vec<LifecycleSource>,
}

impl LifecycleScenario {
    pub fn load(path: &Path) -> Result<Self> {
        let scenario: Self = serde_json::from_slice(&crate::lifecycle::artifact::read(path)?)?;
        scenario.validate()?;
        for source in &scenario.sources {
            if let (Some(artifact), Some(expected)) = (&source.artifact, &source.content_sha256) {
                let bytes = crate::lifecycle::artifact::read_relative(
                    path.parent().unwrap_or_else(|| Path::new(".")),
                    artifact,
                )
                .with_context(|| format!("reading lifecycle source artifact {artifact}"))?;
                ensure!(
                    sha256(&bytes) == *expected,
                    "lifecycle source artifact hash mismatch: {}",
                    source.id
                );
            }
        }
        Ok(scenario)
    }
    pub fn to_json(&self) -> Result<String> {
        crate::canonical::document(self)
    }
    pub fn sha256(&self) -> Result<String> {
        Ok(sha256(self.to_json()?.as_bytes()))
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported lifecycle scenario schema"
        );
        ensure!(
            !self.id.trim().is_empty()
                && !self.scenario_version.trim().is_empty()
                && !self.change.description.trim().is_empty(),
            "missing scenario identity/description"
        );
        crate::change::canonical_address(&self.policy.asset_mint, "lifecycle asset mint")?;
        ensure!(
            matches!(
                self.policy.before,
                LifecycleStatus::Active | LifecycleStatus::Unknown
            ),
            "before status must be Active or Unknown"
        );
        ensure!(
            matches!(
                self.policy.after,
                LifecycleStatus::TransitionRequired | LifecycleStatus::Unknown
            ),
            "after status must be TransitionRequired or Unknown"
        );
        let mut required =
            BTreeSet::from(["/policy/effective_at", "/policy/before", "/policy/after"]);
        if let Some(deadline) = &self.policy.deadline {
            ensure!(
                deadline.at > self.policy.effective_at,
                "deadline must follow effective_at"
            );
            ensure!(
                matches!(
                    deadline.after,
                    LifecycleStatus::PostDeadlineTransitionRequired
                        | LifecycleStatus::Expired
                        | LifecycleStatus::NoIssuerEntitlement
                        | LifecycleStatus::Unknown
                ),
                "unsupported post-deadline status"
            );
            required.insert("/policy/deadline");
        }
        if let Some(successor) = &self.policy.successor {
            crate::change::canonical_address(&successor.mint, "successor mint")?;
            ensure!(
                successor.mint != self.policy.asset_mint
                    && !successor.description.trim().is_empty(),
                "invalid successor identity"
            );
            required.insert("/policy/successor");
        }
        let mut ids = BTreeSet::new();
        let mut covered = BTreeSet::new();
        for source in &self.sources {
            ensure!(
                !source.id.trim().is_empty() && ids.insert(&source.id),
                "missing or duplicate lifecycle source id"
            );
            ensure!(
                !source.reference.trim().is_empty()
                    && !source.description.trim().is_empty()
                    && !source.supports.is_empty(),
                "incomplete lifecycle provenance"
            );
            ensure!(
                source.captured_at <= self.captured_at,
                "source capture follows scenario capture"
            );
            ensure!(
                source.artifact.is_some() == source.content_sha256.is_some(),
                "artifact and hash must be supplied together"
            );
            if let Some(hash) = &source.content_sha256 {
                ensure!(
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "invalid source content hash"
                );
                ensure!(
                    !source.artifact.as_ref().unwrap().trim().is_empty(),
                    "empty source artifact"
                );
            }
            let mut unique = BTreeSet::new();
            for pointer in &source.supports {
                ensure!(
                    required.contains(pointer.as_str()) && unique.insert(pointer),
                    "unknown or duplicate policy provenance pointer"
                );
                covered.insert(pointer.as_str());
            }
        }
        ensure!(
            covered == required,
            "lifecycle policy lacks field-level provenance"
        );
        Ok(())
    }
}
