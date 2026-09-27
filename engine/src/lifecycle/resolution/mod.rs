//! Generic path resolution: external mechanism discovery never grants execution proof.
pub mod phase7;

pub(crate) mod current;
use crate::{
    lifecycle::frozen::selection::{canonical, digest, load},
    lifecycle::{
        consequence::{LifecycleImpact, LifecycleImpactClassification},
        policy::LifecycleStatus,
        EntityType,
    },
    path::{ExitPathType, ProbeClock},
};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathStatus {
    Proven,
    Failed,
    Indeterminate,
    Unsupported,
    NotTested,
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub file: String,
    pub sha256: String,
}
impl ArtifactRef {
    pub fn read(&self, base: &Path) -> Result<Vec<u8>> {
        let bytes = crate::lifecycle::artifact::read_relative(base, &self.file)?;
        ensure!(
            crate::lifecycle::exposure::sha256(&bytes) == self.sha256,
            "artifact digest mismatch: {}",
            self.file
        );
        Ok(bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceKind {
    ObservedOnchainState,
    LocalExecution,
    ExternalPolicy,
    ExternalMechanism,
    ResearchInference,
    ScenarioAssumption,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoverySource {
    pub id: String,
    pub kind: EvidenceKind,
    pub reference: String,
    pub artifact: ArtifactRef,
    pub description: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MechanismBoundary {
    OnchainCandidateUnverified,
    IssuerMediatedOutsideExecutor,
    NoSupportedAdapter,
    Unresolved,
    RequiredEvidenceUnavailable,
}
fn discovery_status(boundary: MechanismBoundary) -> PathStatus {
    match boundary {
        MechanismBoundary::IssuerMediatedOutsideExecutor
        | MechanismBoundary::NoSupportedAdapter => PathStatus::Unsupported,
        MechanismBoundary::OnchainCandidateUnverified | MechanismBoundary::Unresolved => {
            PathStatus::NotTested
        }
        MechanismBoundary::RequiredEvidenceUnavailable => PathStatus::Indeterminate,
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathDiscovery {
    pub path_type: ExitPathType,
    pub boundary: MechanismBoundary,
    pub requested_contexts: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub facts: Vec<String>,
    pub reason: String,
    pub limitations: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryManifest {
    pub schema_version: u32,
    pub asset_mint: String,
    pub snapshot_sha256: String,
    pub scenario_sha256: String,
    pub sources: Vec<DiscoverySource>,
    pub paths: Vec<PathDiscovery>,
    pub investigation_scope: Vec<String>,
}
impl DiscoveryManifest {
    pub fn load(path: &Path) -> Result<Self> {
        let m: Self = load(path)?;
        m.validate()?;
        for source in &m.sources {
            source
                .artifact
                .read(path.parent().unwrap_or_else(|| Path::new(".")))?;
        }
        Ok(m)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 1, "unsupported discovery schema");
        let ids: BTreeSet<_> = self.sources.iter().map(|s| &s.id).collect();
        ensure!(
            ids.len() == self.sources.len() && !ids.is_empty(),
            "missing/duplicate discovery source"
        );
        ensure!(
            self.sources
                .iter()
                .all(|s| !matches!(s.kind, EvidenceKind::LocalExecution)
                    && !s.description.is_empty()),
            "discovery cannot fabricate local execution evidence"
        );
        let types: BTreeSet<_> = self.paths.iter().map(|p| path_name(p.path_type)).collect();
        ensure!(
            types.len() == self.paths.len() && types.len() == 5 && !types.contains("Unknown"),
            "exact five distinct discovery paths required"
        );
        for p in &self.paths {
            ensure!(
                !p.reason.is_empty()
                    && !p.limitations.is_empty()
                    && p.evidence_ids.iter().all(|id| ids.contains(id)),
                "unbound discovery claim"
            );
            ensure!(
                p.requested_contexts.iter().collect::<BTreeSet<_>>().len()
                    == p.requested_contexts.len(),
                "duplicate requested context"
            );
        }
        Ok(())
    }
}
pub fn path_name(p: ExitPathType) -> &'static str {
    match p {
        ExitPathType::OfficialTransition => "OfficialTransition",
        ExitPathType::Redemption => "Redemption",
        ExitPathType::SecondaryMarketExit => "SecondaryMarketExit",
        ExitPathType::Transfer => "Transfer",
        ExitPathType::Withdrawal => "Withdrawal",
        ExitPathType::Unknown => "Unknown",
    }
}
const PATHS: [ExitPathType; 5] = [
    ExitPathType::OfficialTransition,
    ExitPathType::Redemption,
    ExitPathType::SecondaryMarketExit,
    ExitPathType::Transfer,
    ExitPathType::Withdrawal,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignerAssumption {
    pub authority: String,
    pub signer_possession_known: bool,
    pub signer_assumed_locally: bool,
    pub wording: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathAttempt {
    pub case_id: String,
    pub entity_id: String,
    pub path_type: ExitPathType,
    pub status: PathStatus,
    pub context_id: String,
    pub exact_input_raw: String,
    pub invalid_control: bool,
    pub source_before_raw: Option<String>,
    pub captured_state_sha256: Option<String>,
    pub fixture_sha256: Option<String>,
    pub capture_context: crate::lifecycle::frozen::selection::pipeline::CaptureContext,
    pub clock: Option<ProbeClock>,
    pub output_mint: Option<String>,
    pub destination: Option<String>,
    pub actual_output_raw: Option<String>,
    pub token_transfer_withheld_raw: Option<String>,
    pub dlmm_fee_raw: Option<String>,
    pub dlmm_protocol_fee_raw: Option<String>,
    pub signer: SignerAssumption,
    pub execution_attempted: bool,
    pub error: Option<String>,
    pub rollback_verified: Option<bool>,
    pub execution_assumptions: Vec<String>,
    pub evidence: ArtifactRef,
}
/// No public/deserialization constructor: the adapter must hash-check and replay.
#[derive(Clone, Debug)]
pub struct VerifiedExecution(pub(super) PathAttempt);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathContextResolution {
    pub context_id: String,
    pub status: PathStatus,
    pub attempts: Vec<PathAttempt>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathResolution {
    pub path_type: ExitPathType,
    pub status: PathStatus,
    pub entity_id: String,
    pub contexts: Vec<PathContextResolution>,
    pub discovery: PathDiscovery,
    pub evidence: Vec<DiscoverySource>,
    pub reason: String,
    pub limitations: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleResolution {
    pub schema_version: u32,
    pub resolver_version: String,
    pub entity_id: String,
    pub token_account: String,
    pub owner_authority: String,
    pub asset_mint: String,
    pub entity_type: EntityType,
    pub verified_role: Option<String>,
    pub observed_onchain_evidence: Vec<crate::lifecycle::exposure::ProtocolEvidence>,
    pub policy_evidence: Vec<crate::lifecycle::consequence::LifecycleEvidenceRef>,
    pub observed_public_balance_raw: String,
    pub lifecycle_status: LifecycleStatus,
    pub impact_classification: LifecycleImpactClassification,
    pub policy_evaluated_at: chrono::DateTime<chrono::Utc>,
    pub snapshot_sha256: String,
    pub scenario_sha256: String,
    pub coverage_sha256: String,
    pub discovery_sha256: String,
    pub evidence_bundle_sha256: String,
    pub baseline_execution_status: crate::lifecycle::consequence::LifecycleExecutionStatus,
    pub paths: Vec<PathResolution>,
    pub conclusion: String,
    pub limitations: Vec<String>,
}
fn measured_status(attempts: &[PathAttempt], fallback: PathStatus) -> PathStatus {
    // The row means at least one bounded proof; each context/point stays explicit.
    if attempts
        .iter()
        .any(|a| a.status == PathStatus::Proven && !a.invalid_control)
    {
        PathStatus::Proven
    } else if attempts
        .iter()
        .any(|a| a.status == PathStatus::Failed && !a.invalid_control)
    {
        PathStatus::Failed
    } else if attempts
        .iter()
        .any(|a| a.status == PathStatus::Indeterminate || a.invalid_control)
    {
        PathStatus::Indeterminate
    } else if attempts.iter().any(|a| a.status == PathStatus::Unsupported) {
        PathStatus::Unsupported
    } else if !attempts.is_empty() {
        PathStatus::NotTested
    } else {
        fallback
    }
}
pub struct LifecyclePathResolver;
impl LifecyclePathResolver {
    pub fn resolve(
        impact: &LifecycleImpact,
        discovery: &DiscoveryManifest,
        executions: &[VerifiedExecution],
    ) -> Result<Vec<PathResolution>> {
        Self::resolve_entity(
            &impact.entity_id,
            &impact.asset_mint,
            impact.entity_type == EntityType::WalletCompatible && impact.verified_role.is_none(),
            discovery,
            executions,
        )
    }
    pub(crate) fn resolve_entity(
        entity_id: &str,
        asset_mint: &str,
        direct_holder: bool,
        discovery: &DiscoveryManifest,
        executions: &[VerifiedExecution],
    ) -> Result<Vec<PathResolution>> {
        discovery.validate()?;
        ensure!(
            asset_mint == discovery.asset_mint,
            "discovery asset differs from entity"
        );
        let mut rows = Vec::new();
        for path in PATHS {
            let mut d = discovery
                .paths
                .iter()
                .find(|d| d.path_type == path)
                .unwrap()
                .clone();
            d.requested_contexts.sort();
            d.evidence_ids.sort();
            d.facts.sort();
            d.limitations.sort();
            let mut evidence: Vec<_> = discovery
                .sources
                .iter()
                .filter(|s| d.evidence_ids.contains(&s.id))
                .cloned()
                .collect();
            evidence.sort_by(|a, b| a.id.cmp(&b.id));
            let mut attempts: Vec<_> = executions
                .iter()
                .map(|e| &e.0)
                .filter(|a| a.entity_id == entity_id && a.path_type == path)
                .cloned()
                .collect();
            attempts.sort_by(|a, b| {
                (&a.context_id, &a.exact_input_raw, &a.case_id).cmp(&(
                    &b.context_id,
                    &b.exact_input_raw,
                    &b.case_id,
                ))
            });
            ensure!(
                attempts
                    .iter()
                    .map(|a| &a.case_id)
                    .collect::<BTreeSet<_>>()
                    .len()
                    == attempts.len(),
                "duplicate measured attempt"
            );
            let not_applicable = path == ExitPathType::Withdrawal && direct_holder;
            ensure!(
                !not_applicable || attempts.is_empty(),
                "withdrawal evidence conflicts with direct-holder applicability"
            );
            let fallback = discovery_status(d.boundary);
            let ids: BTreeSet<_> = d
                .requested_contexts
                .iter()
                .cloned()
                .chain(attempts.iter().map(|a| a.context_id.clone()))
                .collect();
            let contexts = ids
                .into_iter()
                .map(|id| {
                    let points: Vec<_> = attempts
                        .iter()
                        .filter(|a| a.context_id == id)
                        .cloned()
                        .collect();
                    PathContextResolution {
                        context_id: id,
                        status: if not_applicable {
                            PathStatus::NotApplicable
                        } else {
                            measured_status(&points, fallback)
                        },
                        attempts: points,
                    }
                })
                .collect();
            let status = if not_applicable {
                PathStatus::NotApplicable
            } else {
                measured_status(&attempts, fallback)
            };
            let reason = if not_applicable {
                "Direct wallet-compatible token account with no verified protocol/LP position in this entity context; a venue vault does not grant holder withdrawal rights.".into()
            } else if status == PathStatus::Proven {
                "At least one exact entity/path/context/amount execution was successful and reconciled after fresh offline replay. Proof applies only to listed successful points.".into()
            } else if status == PathStatus::Failed {
                "Listed bounded executions failed under their captured state; no successful point was established. This is not a global impossibility claim.".into()
            } else if status == PathStatus::Indeterminate && !attempts.is_empty() {
                "Listed attempts could not establish execution success/failure; blockers and invalid controls remain explicit.".into()
            } else {
                d.reason.clone()
            };
            let mut limits = d.limitations.clone();
            limits.extend(["Unsupported describes the executor/evidence boundary, never non-existence or impossibility.".into(), "No inheritance across paths, entities, venues or unmeasured amounts; independent points do not establish intervals, proceeds or simultaneous capacity.".into(), "Original owner locally assumed to sign; signer possession/authorization and mainnet inclusion remain unproved.".into()]);
            limits.sort();
            limits.dedup();
            rows.push(PathResolution {
                path_type: path,
                status,
                entity_id: entity_id.to_string(),
                contexts,
                discovery: d,
                evidence,
                reason,
                limitations: limits,
            });
        }
        Ok(rows)
    }
}
impl LifecycleResolution {
    pub fn to_json(&self) -> Result<String> {
        canonical(self)
    }
    pub fn validate(
        &self,
        bundle: &Path,
        snapshot: &crate::lifecycle::LifecycleSnapshot,
        scenario: &crate::lifecycle::policy::LifecycleScenario,
        coverage: &Path,
        discovery: &Path,
    ) -> Result<()> {
        ensure!(
            *self
                == phase7::resolve(
                    bundle,
                    snapshot,
                    scenario,
                    &self.entity_id,
                    coverage,
                    discovery
                )?,
            "path matrix differs from verified offline resolution"
        );
        Ok(())
    }
    pub fn render_text(&self) -> String {
        let mut out = format!("Lifecycle path resolution\nEntity: {}\nOwner: {}\nPublic amount: {} raw\nLifecycle: {:?} / {:?}\nPolicy time: {}\n", self.entity_id,self.owner_authority,self.observed_public_balance_raw,self.lifecycle_status,self.impact_classification,self.policy_evaluated_at);
        for row in &self.paths {
            out.push_str(&format!(
                "\n{}: {:?}\n{}\n",
                path_name(row.path_type),
                row.status,
                row.reason
            ));
            for context in &row.contexts {
                out.push_str(&format!(
                    "  Context {}: {:?}\n",
                    context.context_id, context.status
                ));
                for a in &context.attempts {
                    out.push_str(&format!(
                        "    {} raw: {:?}; output {} raw; withheld {} raw; slot {}; evidence {}\n",
                        a.exact_input_raw,
                        a.status,
                        a.actual_output_raw.as_deref().unwrap_or("unmeasured"),
                        a.token_transfer_withheld_raw
                            .as_deref()
                            .unwrap_or("unmeasured"),
                        a.clock
                            .as_ref()
                            .map(|c| c.slot.to_string())
                            .unwrap_or_else(|| "not executed".into()),
                        a.evidence.sha256
                    ));
                }
            }
        }
        out.push_str(&format!("\n{}\nSigner possession unknown; original owner locally assumed to sign. No mainnet transaction.\n",self.conclusion));
        out
    }
}

#[cfg(test)]
mod tests;
