//! Submitted assertions, verified historical evidence, assurance policy and a local
//! workflow guard remain distinct. No RPC, VM execution or natural-language parser.
use crate::{
    lifecycle::counterfactual::{
        CounterfactualReport, CounterfactualScenario, FrozenCounterfactualWorld, ProductionIdentity,
    },
    lifecycle::frozen::selection::{canonical, digest, load},
    lifecycle::policy::LifecycleStatus,
    lifecycle::readiness::{
        self, EvaluatedScope, EvidenceReference, EvidenceScope, FindingEffect,
        LifecycleReadinessPolicy, LifecycleReadinessReport, PathCondition, PathFact,
        ReadinessRequirement, ReadinessStatus, RequirementCondition, RequirementTarget,
    },
    lifecycle::resolution::{ArtifactRef, PathStatus, SignerAssumption},
    path::position::WithdrawalReport,
    path::ExitPathType,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanProvenance {
    DemonstrationNonIssuer,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssumption {
    CapturedDeployedProgramsInLocalLiteSvm,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PolicyDerivation {
    Original,
    RequireExistingExactRoute { requirement_id: String },
    PrincipalRemovalOnly { requirement_id: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidatePolicyBinding {
    pub artifact: ArtifactRef,
    pub parent_policy_sha256: String,
    pub derivation: PolicyDerivation,
    pub rationale: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedAction {
    pub id: String,
    pub path_type: ExitPathType,
    pub scope: EvidenceScope,
    pub signer: SignerAssumption,
    pub runtime_assumption: RuntimeAssumption,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalTokenDeltas {
    pub principal_removed_raw: BTreeMap<String, String>,
    pub owner_received_raw: BTreeMap<String, String>,
    pub destination_withheld_raw: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Claim {
    ExactPathSucceeded,
    ExactPrincipalTokenDeltas { expected: PrincipalTokenDeltas },
    ZeroRemainingLiquidityShares,
    NoRemainingProtocolFees,
    FeeCollectionCompleted,
    PositionClosed,
    CompletePositionExit,
    OfficialConversionCompleted,
    LifecycleCompletion,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAssertion {
    pub id: String,
    pub action_id: String,
    pub claim: Claim,
    pub evidence_ids: Vec<String>,
    /// Display text only; never parsed to evaluate the assertion.
    pub statement: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateRolloutPlan {
    pub schema_version: u32,
    pub id: String,
    pub version: u32,
    pub label: String,
    pub provenance: PlanProvenance,
    pub target_view: CounterfactualScenario,
    pub production_state_digest: String,
    pub assurance_policy: CandidatePolicyBinding,
    pub requested_actions: Vec<RequestedAction>,
    pub assertions: Vec<CandidateAssertion>,
    pub referenced_evidence: Vec<EvidenceReference>,
    pub required_assurance_conditions: Vec<String>,
}
fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        ensure!(
            !id.trim().is_empty() && seen.insert(id),
            "empty or duplicate candidate identifier"
        );
    }
    Ok(())
}
impl CandidateRolloutPlan {
    pub fn normalized(&self) -> Result<Self> {
        let mut p = self.clone();
        ensure!(
            p.schema_version == 1
                && p.version > 0
                && !p.id.trim().is_empty()
                && !p.label.trim().is_empty(),
            "invalid candidate identity/version"
        );
        ensure!(
            !p.requested_actions.is_empty()
                && !p.assertions.is_empty()
                && !p.assurance_policy.rationale.trim().is_empty(),
            "candidate actions, assertions and explicit policy rationale required"
        );
        unique_ids(p.requested_actions.iter().map(|a| a.id.as_str()))?;
        unique_ids(p.assertions.iter().map(|a| a.id.as_str()))?;
        unique_ids(p.referenced_evidence.iter().map(|a| a.id.as_str()))?;
        unique_ids(p.required_assurance_conditions.iter().map(String::as_str))?;
        ensure!(
            p.requested_actions
                .iter()
                .all(|a| p.assertions.iter().any(|c| c.action_id == a.id)),
            "every requested action must have a submitted assertion"
        );
        for c in &mut p.assertions {
            ensure!(
                !c.statement.trim().is_empty()
                    && p.requested_actions.iter().any(|a| a.id == c.action_id),
                "assertion has no explanation or requested action"
            );
            c.evidence_ids.sort();
            c.evidence_ids.dedup();
            ensure!(
                c.evidence_ids
                    .iter()
                    .all(|id| p.referenced_evidence.iter().any(|r| r.id == *id)),
                "assertion references undeclared evidence"
            );
        }
        p.requested_actions.sort_by(|a, b| a.id.cmp(&b.id));
        p.assertions.sort_by(|a, b| a.id.cmp(&b.id));
        p.referenced_evidence.sort_by(|a, b| a.id.cmp(&b.id));
        p.required_assurance_conditions.sort();
        Ok(p)
    }
}
/// This trust binding is supplied to the validator, separately from the candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RolloutEvidenceBinding {
    pub schema_version: u32,
    pub parent_policy: ArtifactRef,
    pub counterfactual: ArtifactRef,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceAssessment {
    Supported,
    Contradicted,
    NotEstablished,
    EvidenceSubstitution,
    ScopeMismatch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasonCode {
    ExactHistoricalPathSupported,
    ExactPrincipalTokenDeltasSupported,
    ZeroLiquiditySharesObserved,
    ProtocolAccruedFeesRemain,
    FeeCollectionNotTested,
    PositionAccountRetained,
    PositionClosureNotTested,
    OfficialTransitionNotTested,
    CompleteExitNotEstablished,
    LifecycleCompletionNotEstablished,
    PathEvidenceSubstitution,
    ExactScopeMismatch,
    SignerAssumptionMismatch,
    MissingCitedEvidence,
    ExactRequiredRouteFailed,
    PrincipalTokenDeltaMismatch,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidencePointer {
    pub evidence_id: String,
    pub artifact: ArtifactRef,
    pub source_pointer: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionAssessment {
    pub assertion_id: String,
    pub assessment: EvidenceAssessment,
    pub reason_codes: Vec<ReasonCode>,
    pub named_evidence_boundary: String,
    pub requirement_ids: Vec<String>,
    pub historical_paths: Vec<PathFact>,
    pub evidence_pointers: Vec<EvidencePointer>,
    pub explanation: String,
    pub truthful_statement: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateAcceptance {
    Accepted,
    NotAccepted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardDisposition {
    Permitted,
    RefusedCandidate,
    RefusedReadiness,
    RefusedCommandContext,
    RefusedRequestedScope,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum GateCommandCompletion {
    AssuranceEvaluation { exit_code: u8 },
    AnalysisCompleted { exit_code: u8 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyChange {
    pub parent_policy_sha256: String,
    pub new_policy_id: String,
    pub derivation: PolicyDerivation,
    pub parent_requirement: Option<ReadinessRequirement>,
    pub resulting_requirement: Option<ReadinessRequirement>,
    pub rationale: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionObservationProvenance {
    OriginalLocalWithdrawalPostExecution,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionObservation {
    pub provenance: PositionObservationProvenance,
    pub scope: EvidenceScope,
    pub principal_removed_raw: BTreeMap<String, String>,
    pub owner_received_raw: BTreeMap<String, String>,
    pub destination_withheld_transfer_fees_raw: BTreeMap<String, String>,
    pub retained_protocol_accrued_fees_raw: BTreeMap<String, String>,
    pub all_liquidity_shares_zero: bool,
    pub position_account_retained: bool,
    pub fee_collection: PathStatus,
    pub position_closure: PathStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateAssessment {
    pub schema_version: u32,
    pub candidate_plan_sha256: String,
    pub candidate: CandidateRolloutPlan,
    pub production_world: ProductionIdentity,
    pub counterfactual_artifact: ArtifactRef,
    pub target_lifecycle_status: LifecycleStatus,
    pub assessments: Vec<AssertionAssessment>,
    pub candidate_acceptance: CandidateAcceptance,
    pub policy_change: PolicyChange,
    pub readiness: LifecycleReadinessReport,
    pub readiness_exit_code: u8,
    pub requested_actions_covered_by_policy: bool,
    pub evaluation_command_exit_code: u8,
    pub guarded_workflow_disposition: GuardDisposition,
    pub position_observations: Vec<PositionObservation>,
    pub limitations: Vec<String>,
}
/// No Deserialize or public constructor: a workflow cannot trust a submitted report
/// or a candidate's self-declared Proven field as authorization.
pub struct VerifiedCandidateAssessment {
    report: CandidateAssessment,
}
impl VerifiedCandidateAssessment {
    pub fn report(&self) -> &CandidateAssessment {
        &self.report
    }
    pub fn command_exit_code(&self) -> u8 {
        self.report.evaluation_command_exit_code
    }
    pub fn to_json(&self) -> Result<String> {
        canonical(&self.report)
    }
    pub fn render_text(&self) -> String {
        let r = &self.report;
        let mut s=format!("ROLLOUT ASSUMPTION VALIDATION\nPlan: {} v{} (demonstration; non-issuer)\nTarget: {} / {} / {:?}\nCandidate: {:?}\nAssurance: {:?} / {:?} / {}\nReadiness exit: {}\nLocal workflow: {:?}\n\n",r.candidate.id,r.candidate.version,r.candidate.target_view.id,r.candidate.target_view.evaluation_time,r.target_lifecycle_status,r.candidate_acceptance,r.readiness.overall_status,r.readiness.evaluated_scope,r.readiness.policy.id,r.readiness_exit_code,r.guarded_workflow_disposition);
        for a in &r.assessments {
            s.push_str(&format!(
                "{}: {:?} {:?}\n  {}\n  Truthful wording: {}\n",
                a.assertion_id, a.assessment, a.reason_codes, a.explanation, a.truthful_statement
            ));
        }
        s.push_str("\nHistorical proof remains conditional on captured scope and local signing. No mainnet action, issuer intervention or loss prevention is claimed.\n");
        s
    }
}

pub struct RolloutValidator {
    world: FrozenCounterfactualWorld,
    counterfactual: CounterfactualReport,
    binding: RolloutEvidenceBinding,
    original_policy_path: PathBuf,
    withdrawal: WithdrawalReport,
}
impl RolloutValidator {
    pub fn load(binding_path: &Path) -> Result<Self> {
        let binding: RolloutEvidenceBinding = load(binding_path)?;
        ensure!(
            binding.schema_version == 1,
            "unsupported rollout evidence binding"
        );
        let base = binding_path.parent().unwrap_or(Path::new("."));
        let policy: LifecycleReadinessPolicy =
            serde_json::from_slice(&binding.parent_policy.read(base)?)?;
        let original_policy_path = base.join(&binding.parent_policy.file);
        let policy_base = original_policy_path
            .parent()
            .context("parent policy directory missing")?;
        let manifest: readiness::evidence::ReadinessEvidenceManifest =
            serde_json::from_slice(&policy.evidence_manifest.read(policy_base)?)?;
        let manifest_path = policy_base.join(&policy.evidence_manifest.file);
        let evidence_base = manifest_path
            .parent()
            .context("evidence directory missing")?;
        let world = FrozenCounterfactualWorld::load(
            &evidence_base.join(&manifest.snapshot.file),
            &evidence_base.join(&manifest.scenario.file),
            &original_policy_path,
        )?;
        let counterfactual: CounterfactualReport =
            serde_json::from_slice(&binding.counterfactual.read(base)?)?;
        world.validate(&counterfactual)?;
        let withdrawal: WithdrawalReport =
            serde_json::from_slice(&manifest.position_resolution.read(evidence_base)?)?;
        Ok(Self {
            world,
            counterfactual,
            binding,
            original_policy_path,
            withdrawal,
        })
    }
    pub fn counterfactual(&self) -> &CounterfactualReport {
        &self.counterfactual
    }
    fn policy_for(
        &self,
        p: &CandidateRolloutPlan,
        base: &Path,
    ) -> Result<(LifecycleReadinessPolicy, PolicyChange)> {
        let b = &p.assurance_policy;
        ensure!(
            b.parent_policy_sha256 == self.binding.parent_policy.sha256,
            "candidate parent policy is not the trusted parent"
        );
        let declared: LifecycleReadinessPolicy = serde_json::from_slice(&b.artifact.read(base)?)?;
        let original = &self.counterfactual.frozen_readiness.policy;
        let mut expected = original.clone();
        let mut before = None;
        let mut after = None;
        match &b.derivation {
            PolicyDerivation::Original => {
                ensure!(
                    b.artifact.sha256 == self.binding.parent_policy.sha256
                        && base.join(&b.artifact.file).canonicalize()?
                            == self.original_policy_path.canonicalize()?,
                    "original policy binding changed"
                );
            }
            PolicyDerivation::RequireExistingExactRoute { requirement_id } => {
                ensure!(
                    declared.id != original.id,
                    "demonstration variant needs its own policy identity"
                );
                let r = expected
                    .requirements
                    .iter_mut()
                    .find(|r| r.id == *requirement_id)
                    .context("unknown parent route requirement")?;
                ensure!(
                    !r.required
                        && matches!(&r.condition,RequirementCondition::Path{any_of} if any_of.len()==1 && any_of[0].path_type==ExitPathType::SecondaryMarketExit),
                    "variant must require one previously optional exact sale"
                );
                before = Some(r.clone());
                r.required = true;
                after = Some(r.clone());
                expected.id = declared.id.clone();
                expected.description = declared.description.clone();
            }
            PolicyDerivation::PrincipalRemovalOnly { requirement_id } => {
                ensure!(
                    declared.id != original.id,
                    "narrow policy needs its own policy identity"
                );
                let r = original
                    .requirements
                    .iter()
                    .find(|r| r.id == *requirement_id)
                    .context("unknown parent principal requirement")?;
                ensure!(
                    r.required
                        && matches!(&r.condition,RequirementCondition::Path{any_of} if any_of.len()==1 && any_of[0].path_type==ExitPathType::Withdrawal && any_of[0].scope.state_shape==readiness::StateShape::ProtocolPosition),
                    "narrow scope must be exact native principal withdrawal"
                );
                before = Some(r.clone());
                after = Some(r.clone());
                expected.requirements = vec![r.clone()];
                expected.evaluated_scope = EvaluatedScope::DemoEntityReadiness;
                expected.id = declared.id.clone();
                expected.description = declared.description.clone();
            }
        }
        ensure!(
            declared.normalized()? == expected.normalized()?,
            "undeclared assurance policy changes"
        );
        let change = PolicyChange {
            parent_policy_sha256: b.parent_policy_sha256.clone(),
            new_policy_id: declared.id.clone(),
            derivation: b.derivation.clone(),
            parent_requirement: before,
            resulting_requirement: after,
            rationale: b.rationale.clone(),
        };
        Ok((declared.normalized()?, change))
    }
    fn claim_requirement(
        &self,
        a: &RequestedAction,
        c: &CandidateAssertion,
        complete: bool,
    ) -> ReadinessRequirement {
        let path = if matches!(
            c.claim,
            Claim::OfficialConversionCompleted | Claim::LifecycleCompletion
        ) {
            ExitPathType::OfficialTransition
        } else if matches!(c.claim, Claim::ExactPathSucceeded) {
            a.path_type
        } else {
            ExitPathType::Withdrawal
        };
        ReadinessRequirement {id:format!("candidate-assertion-{}",c.id),label:c.statement.clone(),target:RequirementTarget::Entity{entity_id:a.scope.entity_id.clone()},required:true,
            condition:if complete {RequirementCondition::CompletePositionExit{scope:Box::new(a.scope.clone()),forbid_remaining_fees:false}} else {RequirementCondition::Path{any_of:vec![PathCondition{path_type:path,scope:a.scope.clone(),accepted_statuses:vec![PathStatus::Proven],blocking_statuses:vec![PathStatus::Failed],allow_local_signer_assumption:a.signer.signer_assumed_locally}]}},
            rollout_assumption:c.statement.clone(),remediation_requirement:"Supply independent evidence for this exact action; correcting wording does not execute or repair it.".into()}
    }
    fn assess_assertion(
        &self,
        p: &CandidateRolloutPlan,
        policy: &LifecycleReadinessPolicy,
        c: &CandidateAssertion,
    ) -> Result<AssertionAssessment> {
        let a = p
            .requested_actions
            .iter()
            .find(|a| a.id == c.action_id)
            .context("missing action")?;
        let mut claim_policy = self.counterfactual.frozen_readiness.policy.clone();
        claim_policy.id = format!("{}-claim-{}", p.id, c.id);
        claim_policy.description =
            "Demonstration assertion evidence check; not issuer policy or population readiness."
                .into();
        claim_policy.evaluated_scope = EvaluatedScope::DemoEntityReadiness;
        claim_policy.requirements = vec![self.claim_requirement(a, c, false)];
        let operation = readiness::evaluate(&claim_policy, self.world.verified_evidence())?;
        let operation_finding = &operation.findings[0];
        let desired = match &claim_policy.requirements[0].condition {
            RequirementCondition::Path { any_of } => any_of[0].path_type,
            _ => unreachable!(),
        };
        let original = &self.counterfactual.frozen_readiness;
        let cited = original
            .path_evidence
            .iter()
            .filter(|f| f.evidence_ids.iter().any(|id| c.evidence_ids.contains(id)))
            .collect::<Vec<_>>();
        let exact = cited
            .iter()
            .copied()
            .find(|f| f.path_type == desired && f.scope == a.scope);
        let principal = original.position_exit_evidence.iter().find(|f| {
            f.scope == a.scope && f.evidence_ids.iter().any(|id| c.evidence_ids.contains(id))
        });
        let historical_paths = original
            .path_evidence
            .iter()
            .filter(|f| {
                f.scope.entity_id == a.scope.entity_id
                    && (f.path_type == desired
                        || f.evidence_ids.iter().any(|id| c.evidence_ids.contains(id)))
            })
            .cloned()
            .collect();
        let requirement_ids = policy
            .requirements
            .iter()
            .filter(|r| match &r.condition {
                RequirementCondition::Path { any_of } => any_of.iter().any(|cond| {
                    cond.path_type == desired && cond.scope.entity_id == a.scope.entity_id
                }),
                RequirementCondition::CompletePositionExit { scope, .. } => {
                    scope.entity_id == a.scope.entity_id
                        && !matches!(
                            c.claim,
                            Claim::OfficialConversionCompleted | Claim::LifecycleCompletion
                        )
                }
                _ => false,
            })
            .map(|r| r.id.clone())
            .collect();
        let source_pointers = match &c.claim {
            Claim::ExactPrincipalTokenDeltas { .. } => vec!["/token_reconciliation"],
            Claim::ZeroRemainingLiquidityShares => vec![
                "/post_position_fields/liquidity_shares",
                "/all_position_liquidity_removed",
            ],
            Claim::NoRemainingProtocolFees | Claim::FeeCollectionCompleted => vec![
                "/post_position_fields/pending_fees_raw",
                "/fees_claimed_raw",
                "/paths",
            ],
            Claim::PositionClosed => vec!["/position_retained", "/post_position_fields", "/paths"],
            Claim::CompletePositionExit | Claim::LifecycleCompletion => vec![
                "/post_position_fields/pending_fees_raw",
                "/position_retained",
                "/paths",
            ],
            _ => vec!["/paths"],
        };
        let mut evidence_pointers = vec![];
        for id in &c.evidence_ids {
            let r = p
                .referenced_evidence
                .iter()
                .find(|r| r.id == *id)
                .context("undeclared citation")?;
            let pointers = if id == "position-resolution" {
                source_pointers.clone()
            } else if id.starts_with("measurement:") {
                vec!["/execution", "/rollback_verified", "/vm_clock"]
            } else {
                vec!["/"]
            };
            for pointer in pointers {
                evidence_pointers.push(EvidencePointer {
                    evidence_id: id.clone(),
                    artifact: r.artifact.clone(),
                    source_pointer: pointer.into(),
                });
            }
        }
        let mut result = AssertionAssessment {
            assertion_id: c.id.clone(),
            assessment: EvidenceAssessment::NotEstablished,
            reason_codes: vec![],
            named_evidence_boundary: format!("{}:exact-action-evidence-isolation", c.id),
            requirement_ids,
            historical_paths,
            evidence_pointers,
            explanation: operation_finding.reason.clone(),
            truthful_statement:
                "No independent exact assertion proof is established by the cited evidence.".into(),
        };
        if c.evidence_ids.is_empty() {
            result.reason_codes.push(ReasonCode::MissingCitedEvidence);
            return Ok(result);
        }
        // Proven movement/unwind cannot be substituted for another action's proof.
        if exact.is_none()
            && cited.iter().any(|f| {
                f.scope.entity_id == a.scope.entity_id
                    && f.path_type != desired
                    && f.status == PathStatus::Proven
            })
        {
            result.assessment = EvidenceAssessment::EvidenceSubstitution;
            result
                .reason_codes
                .push(ReasonCode::PathEvidenceSubstitution);
            if desired == ExitPathType::OfficialTransition {
                result
                    .reason_codes
                    .push(ReasonCode::OfficialTransitionNotTested);
            }
            if matches!(c.claim, Claim::LifecycleCompletion) {
                result
                    .reason_codes
                    .push(ReasonCode::LifecycleCompletionNotEstablished);
            }
            result.explanation="The cited operation has a different path identity. Movement or principal removal supplies no official-conversion execution proof.".into();
            result.truthful_statement="The cited original operation retains its exact historical proof; OfficialTransition remains NotTested and was not executed.".into();
            return Ok(result);
        }
        let Some(f) = exact else {
            result.assessment = if cited.is_empty() {
                EvidenceAssessment::NotEstablished
            } else {
                EvidenceAssessment::ScopeMismatch
            };
            result.reason_codes.push(if cited.is_empty() {
                ReasonCode::MissingCitedEvidence
            } else {
                ReasonCode::ExactScopeMismatch
            });
            return Ok(result);
        };
        if f.signer != a.signer || a.signer.authority != a.scope.authority {
            result.assessment = EvidenceAssessment::ScopeMismatch;
            result
                .reason_codes
                .push(ReasonCode::SignerAssumptionMismatch);
            return Ok(result);
        }
        if f.status == PathStatus::Failed
            && f.execution_attempted
            && f.rollback_verified == Some(true)
        {
            result.assessment = EvidenceAssessment::Contradicted;
            result
                .reason_codes
                .push(ReasonCode::ExactRequiredRouteFailed);
            result.explanation=format!("This exact required route failed under this captured execution context. {} Rollback verified; the lifecycle event is not identified as the cause.",f.reason);
            result.truthful_statement="This exact required route failed under this captured execution context. Other routes, venues, holders and future availability are not established by that failure.".into();
            return Ok(result);
        }
        if operation_finding.effect != FindingEffect::Satisfied {
            result.reason_codes.push(
                if desired == ExitPathType::OfficialTransition && f.status == PathStatus::NotTested
                {
                    ReasonCode::OfficialTransitionNotTested
                } else {
                    ReasonCode::MissingCitedEvidence
                },
            );
            return Ok(result);
        }
        match &c.claim {
            Claim::ExactPathSucceeded => {
                result.assessment = EvidenceAssessment::Supported;
                result
                    .reason_codes
                    .push(ReasonCode::ExactHistoricalPathSupported);
                result.truthful_statement="This exact action succeeded locally under the recorded captured-bank, signer and runtime assumptions; current/future availability is not established.".into();
            }
            Claim::ExactPrincipalTokenDeltas { expected } => {
                if let Some(f) = principal {
                    let actual = PrincipalTokenDeltas {
                        principal_removed_raw: f.principal_removed_raw.clone(),
                        owner_received_raw: f.owner_received_raw.clone(),
                        destination_withheld_raw: f.destination_withheld_raw.clone(),
                    };
                    result.assessment = if *expected == actual {
                        EvidenceAssessment::Supported
                    } else {
                        EvidenceAssessment::Contradicted
                    };
                    result.reason_codes.push(if *expected == actual {
                        ReasonCode::ExactPrincipalTokenDeltasSupported
                    } else {
                        ReasonCode::PrincipalTokenDeltaMismatch
                    });
                    result.truthful_statement=format!("Principal debits {:?}, public owner credits {:?}, destination withheld transfer fees {:?}; these withheld fees are separate from retained protocol-accrued fees.",actual.principal_removed_raw,actual.owner_received_raw,actual.destination_withheld_raw);
                } else {
                    result.reason_codes.push(ReasonCode::MissingCitedEvidence);
                }
            }
            Claim::ZeroRemainingLiquidityShares => {
                if principal.is_some()
                    && self.withdrawal.all_position_liquidity_removed == Some(true)
                {
                    result.assessment = EvidenceAssessment::Supported;
                    result
                        .reason_codes
                        .push(ReasonCode::ZeroLiquiditySharesObserved);
                    result.truthful_statement="All shares in this exact full-range local withdrawal became zero; this does not prove fee collection or position closure.".into();
                } else {
                    result.reason_codes.push(ReasonCode::MissingCitedEvidence);
                }
            }
            Claim::NoRemainingProtocolFees
            | Claim::FeeCollectionCompleted
            | Claim::PositionClosed
            | Claim::CompletePositionExit => {
                if let Some(f) = principal {
                    let residual = f.residual_fees_raw.values().any(|n| n != "0");
                    let retained = self.withdrawal.position_retained == Some(true);
                    match c.claim {
                        Claim::NoRemainingProtocolFees => {
                            result.assessment = if residual {
                                EvidenceAssessment::Contradicted
                            } else {
                                EvidenceAssessment::Supported
                            };
                            if residual {
                                result
                                    .reason_codes
                                    .push(ReasonCode::ProtocolAccruedFeesRemain);
                            }
                        }
                        Claim::FeeCollectionCompleted => {
                            if f.fee_collection == PathStatus::Proven {
                                result.assessment = EvidenceAssessment::Supported;
                            } else {
                                result.reason_codes.push(ReasonCode::FeeCollectionNotTested);
                            }
                        }
                        Claim::PositionClosed => {
                            if retained {
                                result.assessment = EvidenceAssessment::Contradicted;
                                result
                                    .reason_codes
                                    .push(ReasonCode::PositionAccountRetained);
                            }
                            result
                                .reason_codes
                                .push(ReasonCode::PositionClosureNotTested);
                        }
                        Claim::CompletePositionExit => {
                            claim_policy.requirements = vec![self.claim_requirement(a, c, true)];
                            let complete =
                                readiness::evaluate(&claim_policy, self.world.verified_evidence())?;
                            result.assessment = if residual || retained {
                                EvidenceAssessment::Contradicted
                            } else if complete.findings[0].effect == FindingEffect::Satisfied {
                                EvidenceAssessment::Supported
                            } else {
                                EvidenceAssessment::NotEstablished
                            };
                            result
                                .reason_codes
                                .push(ReasonCode::CompleteExitNotEstablished);
                            if residual {
                                result
                                    .reason_codes
                                    .push(ReasonCode::ProtocolAccruedFeesRemain);
                            }
                            if retained {
                                result
                                    .reason_codes
                                    .push(ReasonCode::PositionAccountRetained);
                            }
                            if f.fee_collection != PathStatus::Proven {
                                result.reason_codes.push(ReasonCode::FeeCollectionNotTested);
                            }
                            if f.position_closure != PathStatus::Proven {
                                result
                                    .reason_codes
                                    .push(ReasonCode::PositionClosureNotTested);
                            }
                            result.explanation = complete.findings[0].reason.clone();
                        }
                        _ => unreachable!(),
                    }
                    result.truthful_statement=format!("The captured principal withdrawal succeeded locally; protocol-accrued fees {:?} and the position account remained. Fee collection {:?}; position closure {:?}. Correcting the statement does not repair the position or establish broader readiness.",f.residual_fees_raw,f.fee_collection,f.position_closure);
                } else {
                    result.reason_codes.push(ReasonCode::MissingCitedEvidence);
                }
            }
            Claim::OfficialConversionCompleted | Claim::LifecycleCompletion => {
                // This arm is reachable only with independent exact official proof;
                // lifecycle completion additionally needs independent complete exit.
                if matches!(c.claim, Claim::OfficialConversionCompleted) {
                    result.assessment = EvidenceAssessment::Supported;
                    result
                        .reason_codes
                        .push(ReasonCode::ExactHistoricalPathSupported);
                } else {
                    result
                        .reason_codes
                        .push(ReasonCode::LifecycleCompletionNotEstablished);
                }
            }
        }
        Ok(result)
    }
    pub fn evaluate(
        &self,
        plan: &CandidateRolloutPlan,
        plan_base: &Path,
    ) -> Result<VerifiedCandidateAssessment> {
        let p = plan.normalized()?;
        ensure!(
            p.production_state_digest
                == self.counterfactual.production_world.production_state_digest,
            "candidate world mismatch"
        );
        let target = self
            .counterfactual
            .scenarios
            .iter()
            .find(|s| s.scenario == p.target_view)
            .context("candidate target is not an exact verified counterfactual view")?;
        let (policy, policy_change) = self.policy_for(&p, plan_base)?;
        let required = policy
            .requirements
            .iter()
            .filter(|r| r.required)
            .map(|r| r.id.clone())
            .collect::<Vec<_>>();
        ensure!(
            p.required_assurance_conditions == required,
            "candidate omits or invents required assurance conditions"
        );
        for r in &p.referenced_evidence {
            let verified = self
                .counterfactual
                .frozen_readiness
                .evidence_refs
                .iter()
                .find(|v| v.id == r.id)
                .context("unverified candidate evidence identity")?;
            ensure!(
                r.artifact == verified.artifact,
                "candidate evidence differs from the verified catalogue"
            );
        }
        for a in &p.requested_actions {
            ensure!(
                a.path_type != ExitPathType::Unknown,
                "requested action is unknown"
            );
            ensure!(
                a.scope.asset_mint == policy.asset_mint
                    && a.scope.scenario_sha256 == policy.scenario_sha256,
                "requested action asset or historical scenario mismatch"
            );
        }
        for c in &p.assertions {
            if let Claim::ExactPrincipalTokenDeltas { expected } = &c.claim {
                for n in expected
                    .principal_removed_raw
                    .values()
                    .chain(expected.owner_received_raw.values())
                    .chain(expected.destination_withheld_raw.values())
                {
                    let amount = n.parse::<u128>()?;
                    ensure!(
                        amount.to_string() == *n,
                        "noncanonical principal token delta"
                    );
                }
            }
        }
        let assessments = p
            .assertions
            .iter()
            .map(|c| self.assess_assertion(&p, &policy, c))
            .collect::<Result<Vec<_>>>()?;
        let candidate_acceptance = if assessments
            .iter()
            .all(|a| a.assessment == EvidenceAssessment::Supported)
        {
            CandidateAcceptance::Accepted
        } else {
            CandidateAcceptance::NotAccepted
        };
        // No observer time / PreEvent shortcut: always evaluate the submitted target's requirements.
        let readiness = readiness::evaluate(&policy, self.world.verified_evidence())?;
        let readiness_exit_code = readiness.overall_status.exit_code();
        let requested_actions_covered_by_policy = p.requested_actions.iter().all(|a| {
            policy
                .requirements
                .iter()
                .filter(|r| r.required)
                .any(|r| match &r.condition {
                    RequirementCondition::Path { any_of } => any_of
                        .iter()
                        .any(|c| c.path_type == a.path_type && c.scope == a.scope),
                    RequirementCondition::CompletePositionExit { scope, .. } => {
                        a.path_type == ExitPathType::Withdrawal && **scope == a.scope
                    }
                    _ => false,
                })
        });
        let evaluation_command_exit_code = if (candidate_acceptance
            == CandidateAcceptance::NotAccepted
            || !requested_actions_covered_by_policy)
            && readiness.overall_status == ReadinessStatus::Ready
        {
            5
        } else {
            readiness_exit_code
        };
        let guarded_workflow_disposition = guard_decision(
            candidate_acceptance,
            readiness.overall_status,
            GateCommandCompletion::AssuranceEvaluation {
                exit_code: readiness_exit_code,
            },
            requested_actions_covered_by_policy,
        );
        let position_observations = self
            .counterfactual
            .frozen_readiness
            .position_exit_evidence
            .iter()
            .filter(|f| p.requested_actions.iter().any(|a| a.scope == f.scope))
            .map(|f| PositionObservation {
                provenance: PositionObservationProvenance::OriginalLocalWithdrawalPostExecution,
                scope: f.scope.clone(),
                principal_removed_raw: f.principal_removed_raw.clone(),
                owner_received_raw: f.owner_received_raw.clone(),
                destination_withheld_transfer_fees_raw: f.destination_withheld_raw.clone(),
                retained_protocol_accrued_fees_raw: f.residual_fees_raw.clone(),
                all_liquidity_shares_zero: self.withdrawal.all_position_liquidity_removed
                    == Some(true),
                position_account_retained: self.withdrawal.position_retained == Some(true),
                fee_collection: f.fee_collection,
                position_closure: f.position_closure,
            })
            .collect();
        let report=CandidateAssessment{schema_version:1,candidate_plan_sha256:digest(&p)?,candidate:p,production_world:self.counterfactual.production_world.clone(),counterfactual_artifact:self.binding.counterfactual.clone(),target_lifecycle_status:target.lifecycle_status,assessments,candidate_acceptance,policy_change,readiness,readiness_exit_code,requested_actions_covered_by_policy,evaluation_command_exit_code,guarded_workflow_disposition,position_observations,
            limitations:vec!["Demonstration/non-issuer candidate assertions, evidence assessments, assurance status and workflow disposition are separate conclusions.".into(),"Principal-removal assurance is not complete-position exit, official conversion or population rollout readiness. Narrow Ready does not replace the original population Incomplete result.".into(),"Target policy evaluation time is explicit and independent of observer time. Retained readiness metadata and actual captured VM Clocks stay historical; no clock warping or future-state proof is introduced.".into(),"Historical evidence remains conditional on exact entity/amount/path/range/fraction/venue/bank/digests and assumed local signing; key possession and current/future execution availability remain unknown.".into(),"Protocol-accrued position fees and destination Token-2022 withheld transfer fees are separate accounting categories.".into(),"Only an inert temporary local marker can be guarded. No real issuer rollout, transaction, loss prevention or mainnet intervention is claimed.".into(),"Correcting wording or selecting a narrower explicit policy is not technical remediation and does not repair or execute any position action.".into()]};
        Ok(VerifiedCandidateAssessment { report })
    }
}

pub fn guard_decision(
    acceptance: CandidateAcceptance,
    status: ReadinessStatus,
    completion: GateCommandCompletion,
    requested_actions_covered_by_policy: bool,
) -> GuardDisposition {
    if !matches!(completion,GateCommandCompletion::AssuranceEvaluation{exit_code} if exit_code==status.exit_code())
    {
        return GuardDisposition::RefusedCommandContext;
    }
    if acceptance != CandidateAcceptance::Accepted {
        return GuardDisposition::RefusedCandidate;
    }
    if !requested_actions_covered_by_policy {
        return GuardDisposition::RefusedRequestedScope;
    }
    match status {
        ReadinessStatus::Ready => GuardDisposition::Permitted,
        ReadinessStatus::Blocked | ReadinessStatus::Incomplete => {
            GuardDisposition::RefusedReadiness
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardObservation {
    pub plan_id: String,
    pub disposition: GuardDisposition,
    pub readiness_exit_code: u8,
    pub marker_created: bool,
    pub marker_content_sha256: Option<String>,
    pub demonstration_only: bool,
}
pub fn validate_temporary_marker(path: &Path, temporary_root: &Path) -> Result<()> {
    let parent = path
        .parent()
        .context("temporary marker parent required")?
        .canonicalize()?;
    let temp = temporary_root.canonicalize()?;
    let slash_tmp = Path::new("/tmp").canonicalize()?;
    ensure!(
        parent.starts_with(&temp) || parent.starts_with(&slash_tmp),
        "guard marker must be under a temporary local directory"
    );
    ensure!(!path.exists(), "guard marker already exists");
    Ok(())
}
pub fn run_guarded_stub(
    assessment: &VerifiedCandidateAssessment,
    completion: GateCommandCompletion,
    marker: &Path,
) -> Result<GuardObservation> {
    run_guarded_stub_in(assessment, completion, marker, &std::env::temp_dir())
}
/// The local host supplies its temporary root explicitly across an empty-environment
/// worker boundary. This is an output restriction, never analytical evidence.
pub fn run_guarded_stub_in(
    assessment: &VerifiedCandidateAssessment,
    completion: GateCommandCompletion,
    marker: &Path,
    temporary_root: &Path,
) -> Result<GuardObservation> {
    validate_temporary_marker(marker, temporary_root)?;
    let r = assessment.report();
    let disposition = guard_decision(
        r.candidate_acceptance,
        r.readiness.overall_status,
        completion,
        r.requested_actions_covered_by_policy,
    );
    let mut observation = GuardObservation {
        plan_id: r.candidate.id.clone(),
        disposition,
        readiness_exit_code: r.readiness_exit_code,
        marker_created: false,
        marker_content_sha256: None,
        demonstration_only: true,
    };
    if disposition == GuardDisposition::Permitted {
        let bytes=format!("demo step permitted\nplan {}\ndigest {}\nprincipal-removal assurance only; no issuer/mainnet action\n",r.candidate.id,r.candidate_plan_sha256);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)?;
        file.write_all(bytes.as_bytes())?;
        observation.marker_created = true;
        observation.marker_content_sha256 =
            Some(crate::lifecycle::exposure::sha256(bytes.as_bytes()));
    }
    Ok(observation)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardRun {
    pub assessment: CandidateAssessment,
    pub observation: GuardObservation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RolloutDemoCases {
    pub schema_version: u32,
    pub plans: Vec<ArtifactRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolloutDemoReport {
    pub schema_version: u32,
    pub provenance: PlanProvenance,
    pub cases: Vec<GuardRun>,
    pub limitations: Vec<String>,
}
pub fn run_demo(
    validator: &RolloutValidator,
    cases: &RolloutDemoCases,
    base: &Path,
    marker_directory: &Path,
) -> Result<RolloutDemoReport> {
    run_demo_in(
        validator,
        cases,
        base,
        marker_directory,
        &std::env::temp_dir(),
    )
}
/// Explicit host scratch root; no environment needs to reach an evaluation worker.
pub fn run_demo_in(
    validator: &RolloutValidator,
    cases: &RolloutDemoCases,
    base: &Path,
    marker_directory: &Path,
    temporary_root: &Path,
) -> Result<RolloutDemoReport> {
    ensure!(
        cases.schema_version == 1 && !cases.plans.is_empty(),
        "invalid demo cases"
    );
    let mut evaluated = cases
        .plans
        .iter()
        .map(|r| {
            let plan: CandidateRolloutPlan = serde_json::from_slice(&r.read(base)?)?;
            let path = base.join(&r.file);
            validator.evaluate(&plan, path.parent().context("missing plan directory")?)
        })
        .collect::<Result<Vec<_>>>()?;
    evaluated.sort_by(|a, b| a.report().candidate.id.cmp(&b.report().candidate.id));
    unique_ids(evaluated.iter().map(|e| e.report().candidate.id.as_str()))?;
    let markers = (0..evaluated.len())
        .map(|i| marker_directory.join(format!("case-{i}.permitted")))
        .collect::<Vec<_>>();
    for marker in &markers {
        validate_temporary_marker(marker, temporary_root)?;
    }
    let cases = evaluated
        .iter()
        .zip(&markers)
        .map(|(a, marker)| {
            let observation = run_guarded_stub_in(
                a,
                GateCommandCompletion::AssuranceEvaluation {
                    exit_code: a.report().readiness_exit_code,
                },
                marker,
                temporary_root,
            )?;
            Ok(GuardRun {
                assessment: a.report().clone(),
                observation,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(RolloutDemoReport{schema_version:1,provenance:PlanProvenance::DemonstrationNonIssuer,cases,limitations:vec!["Observed stub behavior is local demonstration only; it does not block a real issuer rollout or protect mainnet funds.".into(),"Demo command success means demonstration analysis completed. Each candidate's typed acceptance, readiness exit code and actual marker observation remain separate.".into()]})
}
