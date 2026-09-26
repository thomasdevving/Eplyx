//! Explicit assurance policy + frozen, scope-checked evidence -> deterministic pre-flight.
//! This is separate from economic lifecycle policy and never performs execution or RPC.
pub mod evidence;

use crate::{
    lifecycle::frozen::selection::{canonical, digest},
    lifecycle::resolution::{ArtifactRef, PathStatus, SignerAssumption},
    path::ExitPathType,
};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadinessStatus {
    Ready,
    Blocked,
    Incomplete,
}
impl ReadinessStatus {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::Blocked => 1,
            Self::Incomplete => 5,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindingEffect {
    Blocking,
    IncompleteEvidence,
    Satisfied,
    Informational,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvaluatedScope {
    DemoEntityReadiness,
    /// One bounded candidate-conversion stress run over a freshly captured
    /// population. Distinct from both the single-entity gates and population
    /// rollout readiness: it never claims anything about untested accounts.
    ConversionStressReadiness,
    PopulationRolloutReadiness,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyType {
    DemoAssurancePolicy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateShape {
    DirectTokenAccount,
    ProtocolPosition,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceScope {
    pub asset_mint: String,
    pub entity_id: String,
    pub state_shape: StateShape,
    pub authority: String,
    pub exact_amount_raw: Option<String>,
    pub range: Option<[i32; 2]>,
    pub bps_to_remove: Option<u16>,
    pub venue: Option<String>,
    pub context_id: Option<String>,
    #[serde(with = "crate::numfmt::optional_u64_string")]
    pub captured_slot: Option<u64>,
    pub clock: Option<crate::path::ProbeClock>,
    pub capture_context: Option<crate::lifecycle::frozen::selection::pipeline::CaptureContext>,
    pub captured_state_sha256: Option<String>,
    pub fixture_sha256: Option<String>,
    pub source_before_raw: Option<String>,
    pub scenario_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathCondition {
    pub path_type: ExitPathType,
    pub scope: EvidenceScope,
    pub accepted_statuses: Vec<PathStatus>,
    pub blocking_statuses: Vec<PathStatus>,
    pub allow_local_signer_assumption: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RequirementCondition {
    Path {
        any_of: Vec<PathCondition>,
    },
    CompletePositionExit {
        scope: Box<EvidenceScope>,
        forbid_remaining_fees: bool,
    },
    PopulationCoverage {
        any_of_paths: Vec<ExitPathType>,
    },
    /// An operator-supplied candidate conversion plan, proven by actual execution
    /// at this exact scope. Deliberately a different slot from the lifecycle path
    /// matrix: a candidate plan is never an issuer-defined official transition.
    CandidateConversion {
        scope: Box<EvidenceScope>,
        plan_sha256: String,
        program_sha256: String,
        replacement_mint: String,
    },
    /// Every executable-candidate state shape discovered in a frozen stress plan
    /// must have at least one exact executed case. Satisfying this says which
    /// classes were exercised; it never says anything about their other members.
    SupportedShapeCoverage,
    /// Outcomes of the exact selected stress cases. A real failed case is the one
    /// condition here that blocks rather than merely leaving evidence incomplete.
    SelectedCaseOutcomes {
        allow_failed: bool,
        allow_indeterminate: bool,
        allow_unsupported: bool,
    },
    /// How complete the fresh population acquisition had to be. The two axes are
    /// required independently, because incomplete authority resolution is not the
    /// same finding as an incomplete token-account enumeration.
    PopulationAcquisition {
        required_enumeration: String,
        required_authority_resolution: String,
        max_unsupported_positive_balance_accounts: usize,
    },
    /// Population-wide candidate conversion: every positive-balance account
    /// observed in the capture needs its own proven conversion at its own full
    /// amount. A bounded sample can never satisfy this.
    PopulationConversionCoverage {
        plan_sha256: String,
        program_sha256: String,
    },
    EvidenceIsolation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RequirementTarget {
    Entity {
        entity_id: String,
    },
    /// One bounded stress run as a whole. Deliberately not `Rollout`: run-level
    /// stress conditions must never be mistaken for population readiness.
    StressRun {
        stress_id: String,
    },
    Rollout,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessRequirement {
    pub id: String,
    pub label: String,
    pub target: RequirementTarget,
    pub required: bool,
    pub condition: RequirementCondition,
    pub rollout_assumption: String,
    pub remediation_requirement: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleReadinessPolicy {
    pub schema_version: u32,
    pub id: String,
    pub policy_type: PolicyType,
    pub not_issuer_policy: bool,
    pub description: String,
    pub asset_mint: String,
    pub scenario_sha256: String,
    pub evaluated_scope: EvaluatedScope,
    pub evidence_manifest: ArtifactRef,
    pub requirements: Vec<ReadinessRequirement>,
}
fn normalize_paths(v: &mut Vec<ExitPathType>) {
    v.sort_by_key(|p| crate::lifecycle::resolution::path_name(*p));
    v.dedup();
}
impl LifecycleReadinessPolicy {
    pub fn normalized(&self) -> Result<Self> {
        let mut p = self.clone();
        ensure!(
            p.schema_version == 1
                && p.not_issuer_policy
                && !p.id.is_empty()
                && !p.description.is_empty(),
            "explicit non-issuer demonstration assurance policy required"
        );
        ensure!(
            p.requirements.iter().any(|r| r.required),
            "assurance policy requires a nonempty required scope"
        );
        p.requirements.sort_by(|a, b| a.id.cmp(&b.id));
        ensure!(
            p.requirements
                .iter()
                .map(|r| &r.id)
                .collect::<BTreeSet<_>>()
                .len()
                == p.requirements.len(),
            "duplicate requirement id"
        );
        for r in &mut p.requirements {
            ensure!(
                !r.id.is_empty()
                    && !r.label.is_empty()
                    && !r.rollout_assumption.is_empty()
                    && !r.remediation_requirement.is_empty(),
                "unexplained assurance requirement"
            );
            match &mut r.condition {
                RequirementCondition::Path { any_of } => {
                    ensure!(!any_of.is_empty(), "empty path alternatives");
                    for c in any_of.iter_mut() {
                        ensure!(
                            c.scope.asset_mint == p.asset_mint
                                && c.scope.scenario_sha256 == p.scenario_sha256,
                            "requirement asset/scenario mismatch"
                        );
                        ensure!(
                            matches!(&r.target,RequirementTarget::Entity{entity_id} if *entity_id==c.scope.entity_id),
                            "path target/entity mismatch"
                        );
                        ensure!(
                            c.path_type != ExitPathType::Unknown
                                && c.accepted_statuses == [PathStatus::Proven],
                            "this assurance gate accepts independently measured Proven paths only"
                        );
                        ensure!(
                            c.blocking_statuses.iter().all(|s| *s != PathStatus::Proven),
                            "accepted Proven evidence cannot also be a forbidden status"
                        );
                        c.blocking_statuses.sort_by_key(|s| format!("{s:?}"));
                        c.blocking_statuses.dedup();
                        validate_scope(&c.scope)?;
                    }
                    any_of.sort_by_cached_key(|c| canonical(c).expect("serializable condition"));
                    any_of.dedup();
                }
                RequirementCondition::CompletePositionExit { scope, .. } => {
                    validate_scope(scope)?;
                    ensure!(
                        scope.asset_mint == p.asset_mint
                            && scope.scenario_sha256 == p.scenario_sha256
                            && scope.state_shape == StateShape::ProtocolPosition
                            && matches!(&r.target,RequirementTarget::Entity{entity_id} if *entity_id==scope.entity_id),
                        "complete-exit target/scope mismatch"
                    );
                }
                RequirementCondition::PopulationCoverage { any_of_paths } => {
                    ensure!(
                        r.target == RequirementTarget::Rollout
                            && !any_of_paths.is_empty()
                            && !any_of_paths.contains(&ExitPathType::Unknown),
                        "invalid population requirement"
                    );
                    normalize_paths(any_of_paths);
                }
                RequirementCondition::CandidateConversion {
                    scope,
                    plan_sha256,
                    program_sha256,
                    replacement_mint,
                } => {
                    validate_scope(scope)?;
                    ensure!(
                        scope.asset_mint == p.asset_mint
                            && scope.scenario_sha256 == p.scenario_sha256
                            && matches!(&r.target,RequirementTarget::Entity{entity_id} if *entity_id==scope.entity_id),
                        "candidate-conversion target/scope mismatch"
                    );
                    ensure!(
                        plan_sha256.len() == 64
                            && program_sha256.len() == 64
                            && !replacement_mint.is_empty()
                            && *replacement_mint != p.asset_mint,
                        "a candidate-conversion requirement must pin its plan, candidate program and distinct replacement asset"
                    );
                }
                RequirementCondition::SupportedShapeCoverage
                | RequirementCondition::SelectedCaseOutcomes { .. } => {
                    ensure!(
                        matches!(r.target, RequirementTarget::StressRun { .. }),
                        "stress-run conditions must target the stress run, never an entity or a rollout"
                    );
                }
                RequirementCondition::PopulationAcquisition {
                    required_enumeration,
                    required_authority_resolution,
                    ..
                } => {
                    ensure!(
                        matches!(r.target, RequirementTarget::StressRun { .. }),
                        "population acquisition conditions target the stress run"
                    );
                    ensure!(
                        ["CompleteForQuery", "Partial", "Unavailable", "Unsupported"]
                            .contains(&required_enumeration.as_str()),
                        "unknown required enumeration completeness"
                    );
                    ensure!(
                        ["Complete", "Partial", "NotPerformed"]
                            .contains(&required_authority_resolution.as_str()),
                        "unknown required authority resolution completeness"
                    );
                }
                RequirementCondition::PopulationConversionCoverage {
                    plan_sha256,
                    program_sha256,
                } => {
                    ensure!(
                        r.target == RequirementTarget::Rollout,
                        "population conversion coverage is a rollout-scope condition"
                    );
                    ensure!(
                        plan_sha256.len() == 64 && program_sha256.len() == 64,
                        "population conversion coverage must pin its plan and candidate program"
                    );
                }
                RequirementCondition::EvidenceIsolation => {}
            }
        }
        match p.evaluated_scope {
            EvaluatedScope::PopulationRolloutReadiness => ensure!(
                p.requirements.iter().any(|r| r.required
                    && matches!(
                        r.condition,
                        RequirementCondition::PopulationCoverage { .. }
                            | RequirementCondition::PopulationConversionCoverage { .. }
                    )),
                "population rollout policy must explicitly require population coverage"
            ),
            // A stress policy may reason about its own run and its own exact
            // cases. It may never carry a rollout-scope requirement, so it can
            // never be read as population readiness.
            EvaluatedScope::ConversionStressReadiness => ensure!(
                p.requirements
                    .iter()
                    .all(|r| r.target != RequirementTarget::Rollout),
                "a conversion stress policy cannot imply population rollout readiness"
            ),
            EvaluatedScope::DemoEntityReadiness => ensure!(
                p.requirements
                    .iter()
                    .all(|r| matches!(r.target, RequirementTarget::Entity { .. })),
                "entity-only policy cannot imply stress-run or rollout readiness"
            ),
        }
        Ok(p)
    }
}
fn validate_scope(s: &EvidenceScope) -> Result<()> {
    ensure!(
        !s.entity_id.is_empty() && !s.authority.is_empty(),
        "exact entity and authority required"
    );
    for n in [&s.exact_amount_raw, &s.source_before_raw]
        .into_iter()
        .flatten()
    {
        let v = n.parse::<u64>()?;
        ensure!(v.to_string() == *n, "noncanonical raw amount");
    }
    if let Some(clock) = &s.clock {
        ensure!(
            s.captured_slot == Some(clock.slot),
            "scope clock/slot mismatch"
        );
    }
    if let Some(r) = s.range {
        ensure!(r[0] <= r[1], "invalid exact range");
    }
    if let Some(b) = s.bps_to_remove {
        ensure!(b > 0 && b <= 10000, "invalid exact fraction");
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceReference {
    pub id: String,
    pub artifact: ArtifactRef,
    pub description: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathFact {
    pub scope: EvidenceScope,
    pub path_type: ExitPathType,
    pub status: PathStatus,
    pub execution_attempted: bool,
    pub reconciled: bool,
    pub rollback_verified: Option<bool>,
    pub signer: SignerAssumption,
    pub evidence_ids: Vec<String>,
    pub reason: String,
}
/// One executed operator-supplied candidate conversion, with every distinction
/// this gate must not blur: provenance, digests, assumed authorities and the
/// separate question of issuer binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionFact {
    pub scope: EvidenceScope,
    pub status: PathStatus,
    pub provenance: crate::lifecycle::readiness::PlanProvenance,
    pub plan_sha256: String,
    pub program_sha256: String,
    pub replacement_mint: String,
    pub destination: String,
    pub execution_attempted: bool,
    pub reconciled: bool,
    pub rollback_verified: Option<bool>,
    pub holder_signer: SignerAssumption,
    pub candidate_authority_assumed_locally: bool,
    pub issuer_binding_established: bool,
    pub evidence_ids: Vec<String>,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteExitFact {
    pub scope: EvidenceScope,
    pub principal_unwind: PathStatus,
    pub fee_collection: PathStatus,
    pub position_closure: PathStatus,
    pub residual_fees_raw: BTreeMap<String, String>,
    pub principal_removed_raw: BTreeMap<String, String>,
    pub owner_received_raw: BTreeMap<String, String>,
    pub destination_withheld_raw: BTreeMap<String, String>,
    pub evidence_ids: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopulationEntityFact {
    pub entity_id: String,
    pub account_type: String,
    pub balance_raw: String,
    pub proven_full_amount_paths: Vec<ExitPathType>,
    /// Whether this exact entity has its own proven candidate conversion at its
    /// own full observed amount. Default false: historical population evidence
    /// predates candidate conversion and never carries this fact.
    #[serde(default)]
    pub proven_candidate_conversion: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopulationEvidence {
    pub token_account_entities: usize,
    pub positive_balance_entities: usize,
    pub distinct_owner_authorities: usize,
    pub entities_with_measured_amount: usize,
    pub represented_amount_raw: String,
    pub covered_amount_raw: String,
    pub without_evidence_raw: String,
    pub account_types: BTreeMap<String, crate::lifecycle::frozen::coverage::CoverageAggregate>,
    pub execution_status_counts: BTreeMap<String, usize>,
    pub evidence_ids: Vec<String>,
    pub entities: Vec<PopulationEntityFact>,
}
/// Bounded facts about one frozen stress run. Counts only; every one of them is
/// derived from exact executed cases and none of them is per-entity evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StressEvidence {
    pub stress_id: String,
    pub population_capture_sha256: String,
    pub stress_plan_sha256: String,
    pub candidate_plan_sha256: String,
    pub candidate_program_sha256: String,
    pub enumeration_completeness: String,
    pub authority_resolution_completeness: String,
    pub positive_balance_accounts: usize,
    pub selected_cases: usize,
    pub executed_cases: usize,
    pub proven_cases: usize,
    pub failed_cases: usize,
    pub indeterminate_cases: usize,
    pub unsupported_cases: usize,
    pub executable_shapes: usize,
    pub executable_shapes_with_executed_case: usize,
    pub uncovered_executable_shapes: Vec<String>,
    pub unsupported_positive_balance_accounts: usize,
    pub capture_required_positive_balance_accounts: usize,
    pub failed_case_ids: Vec<String>,
    pub indeterminate_case_ids: Vec<String>,
    pub unsupported_case_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
}

/// A published measurement is consumed only after input digests and scope relationships are checked.
/// No public/deserialization constructor; readiness cannot grant new execution proof.
pub struct VerifiedReadinessEvidence {
    pub(crate) asset_mint: String,
    pub(crate) scenario_sha256: String,
    pub(crate) lifecycle_event: crate::lifecycle::policy::AssetLifecyclePolicy,
    pub(crate) policy_evaluated_at: String,
    pub(crate) path_facts: Vec<PathFact>,
    pub(crate) conversion_facts: Vec<ConversionFact>,
    pub(crate) complete_exits: Vec<CompleteExitFact>,
    pub(crate) population: PopulationEvidence,
    pub(crate) evidence_refs: Vec<EvidenceReference>,
    pub(crate) isolation_verified: bool,
    pub(crate) stress: Option<StressEvidence>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessFinding {
    pub requirement_id: String,
    pub label: String,
    pub required: bool,
    pub scope: RequirementTarget,
    pub effect: FindingEffect,
    pub expected: String,
    pub observed: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub reason: String,
    pub remediation_requirement: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightFailureMode {
    pub requirement_id: String,
    pub scope: RequirementTarget,
    pub assumption: String,
    pub observed: Vec<String>,
    pub effect: FindingEffect,
    pub evidence_ids: Vec<String>,
    pub preflight_effect: String,
    pub real_incident_claimed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityReadiness {
    pub entity_id: String,
    pub status: ReadinessStatus,
    pub requirement_ids: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopulationReadiness {
    pub status: ReadinessStatus,
    pub positive_entities_required: usize,
    pub exact_entities_satisfied: usize,
    pub exhaustive_execution: bool,
    pub unresolved_account_types: BTreeMap<String, usize>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleReadinessReport {
    pub schema_version: u32,
    pub policy: LifecycleReadinessPolicy,
    pub policy_semantic_sha256: String,
    pub asset_mint: String,
    pub lifecycle_event: crate::lifecycle::policy::AssetLifecyclePolicy,
    pub policy_evaluated_at: String,
    pub evaluated_scope: EvaluatedScope,
    pub overall_status: ReadinessStatus,
    pub requirements: Vec<ReadinessRequirement>,
    pub findings: Vec<ReadinessFinding>,
    pub entity_readiness: Vec<EntityReadiness>,
    pub rollout_readiness: Option<PopulationReadiness>,
    pub path_evidence: Vec<PathFact>,
    pub position_exit_evidence: Vec<CompleteExitFact>,
    pub population_summary: PopulationSummary,
    pub prevented_rollout_conditions: Vec<PreflightFailureMode>,
    pub evidence_refs: Vec<EvidenceReference>,
    pub limitations: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopulationSummary {
    pub token_account_entities: usize,
    pub positive_balance_entities: usize,
    pub distinct_owner_authorities: usize,
    pub entities_with_measured_amount: usize,
    pub represented_amount_raw: String,
    pub covered_amount_raw: String,
    pub without_evidence_raw: String,
    pub account_types: BTreeMap<String, crate::lifecycle::frozen::coverage::CoverageAggregate>,
    pub execution_status_counts: BTreeMap<String, usize>,
}
fn aggregate_effects<'a>(effects: impl Iterator<Item = &'a FindingEffect>) -> ReadinessStatus {
    let v = effects.copied().collect::<Vec<_>>();
    if v.contains(&FindingEffect::Blocking) {
        ReadinessStatus::Blocked
    } else if v.contains(&FindingEffect::IncompleteEvidence) {
        ReadinessStatus::Incomplete
    } else {
        ReadinessStatus::Ready
    }
}
fn matches_scope(f: &EvidenceScope, c: &EvidenceScope) -> bool {
    f == c
}
fn accepted_path(f: &PathFact, c: &PathCondition) -> bool {
    c.accepted_statuses.contains(&f.status)
        && f.execution_attempted
        && f.reconciled
        && f.signer.authority == c.scope.authority
        && (f.signer.signer_possession_known
            || (c.allow_local_signer_assumption && f.signer.signer_assumed_locally))
}
fn population_satisfied(p: &PopulationEvidence, paths: &[ExitPathType]) -> usize {
    p.entities
        .iter()
        .filter(|e| {
            e.balance_raw != "0"
                && e.proven_full_amount_paths
                    .iter()
                    .any(|path| paths.contains(path))
        })
        .count()
}
fn evaluate_requirement(
    r: &ReadinessRequirement,
    e: &VerifiedReadinessEvidence,
) -> Result<ReadinessFinding> {
    let mut observed = vec![];
    let mut ids = vec![];
    let (effect, expected, reason) = match &r.condition {
        RequirementCondition::Path { any_of } => {
            let mut satisfied = false;
            let mut blocking = false;
            for c in any_of {
                let candidates = e
                    .path_facts
                    .iter()
                    .filter(|f| {
                        f.scope.asset_mint == c.scope.asset_mint && f.path_type == c.path_type
                    })
                    .collect::<Vec<_>>();
                for f in candidates {
                    let exact = matches_scope(&f.scope, &c.scope);
                    observed.push(format!(
                        "{:?} {:?}; entity {}; context {:?}; amount {:?}; exact scope {}. {}",
                        f.path_type,
                        f.status,
                        f.scope.entity_id,
                        f.scope.context_id,
                        f.scope.exact_amount_raw,
                        exact,
                        f.reason
                    ));
                    ids.extend(f.evidence_ids.clone());
                    if exact {
                        satisfied |= accepted_path(f, c);
                        blocking |= c.blocking_statuses.contains(&f.status)
                            && (f.status != PathStatus::Failed
                                || (f.execution_attempted && f.rollback_verified == Some(true)));
                    }
                }
            }
            let effect = if satisfied {
                FindingEffect::Satisfied
            } else if blocking {
                FindingEffect::Blocking
            } else {
                FindingEffect::IncompleteEvidence
            };
            (effect,"At least one declared exact path must be Proven, reconciled and compatible with the policy's signer assumptions.","Path identity, asset/entity/authority/amount/range/venue/fixture/bank/scenario must match; missing or incompatible proof is not execution failure.")
        }
        RequirementCondition::CompletePositionExit {
            scope,
            forbid_remaining_fees,
        } => {
            let fact = e
                .complete_exits
                .iter()
                .find(|f| matches_scope(&f.scope, scope));
            let effect = if let Some(f) = fact {
                observed.push(format!("Principal unwind {:?}; fee collection {:?}; position closure {:?}; residual fee ledger {:?}.",f.principal_unwind,f.fee_collection,f.position_closure,f.residual_fees_raw));
                ids.extend(f.evidence_ids.clone());
                let residual = f.residual_fees_raw.values().any(|v| v != "0");
                if *forbid_remaining_fees && residual {
                    FindingEffect::Blocking
                } else if f.principal_unwind == PathStatus::Proven
                    && f.fee_collection == PathStatus::Proven
                    && f.position_closure == PathStatus::Proven
                    && !residual
                {
                    FindingEffect::Satisfied
                } else {
                    FindingEffect::IncompleteEvidence
                }
            } else {
                FindingEffect::IncompleteEvidence
            };
            (effect,"Exact complete position exit requires proven principal unwind, proven fee collection/closure and no residual fee exposure.","Native principal withdrawal cannot establish unexecuted fee collection or position closure; residual exposure blocks only when explicitly forbidden by policy.")
        }
        RequirementCondition::PopulationCoverage { any_of_paths } => {
            let n = population_satisfied(&e.population, any_of_paths);
            observed.push(format!("{} of {} positive token-account entities have full represented-amount evidence on the requested paths; {} total observed accounts.",n,e.population.positive_balance_entities,e.population.token_account_entities));
            ids.extend(e.population.evidence_ids.clone());
            (if n==e.population.positive_balance_entities{FindingEffect::Satisfied}else{FindingEffect::IncompleteEvidence},"Every positive observed token-account entity must have its own full represented-amount proof on a requested path.","Sampled entities, protocol observations and venue reserves cannot establish peer-holder or population-wide actionability.")
        }
        RequirementCondition::CandidateConversion {
            scope,
            plan_sha256,
            program_sha256,
            replacement_mint,
        } => {
            let mut satisfied = false;
            for f in e
                .conversion_facts
                .iter()
                .filter(|f| f.scope.asset_mint == scope.asset_mint)
            {
                let exact = matches_scope(&f.scope, scope)
                    && f.plan_sha256 == *plan_sha256
                    && f.program_sha256 == *program_sha256
                    && f.replacement_mint == *replacement_mint;
                observed.push(format!(
                    "Candidate conversion {:?} under {:?} provenance; entity {}; amount {:?}; replacement {}; plan {}; candidate program {}; exact scope {}. Issuer binding established: {}. {}",
                    f.status, f.provenance, f.scope.entity_id, f.scope.exact_amount_raw,
                    f.replacement_mint, f.plan_sha256, f.program_sha256, exact,
                    f.issuer_binding_established, f.reason
                ));
                ids.extend(f.evidence_ids.clone());
                if exact {
                    satisfied |= f.status == PathStatus::Proven
                        && f.provenance
                            == crate::lifecycle::readiness::PlanProvenance::OperatorSupplied
                        && f.execution_attempted
                        && f.reconciled
                        && (f.holder_signer.signer_possession_known
                            || f.holder_signer.signer_assumed_locally);
                }
            }
            (if satisfied{FindingEffect::Satisfied}else{FindingEffect::IncompleteEvidence},
             "The declared candidate conversion plan must have actually executed and reconciled at this exact account, amount, replacement asset, plan version and candidate program build.",
             "This is evidence about the supplied candidate plan under its declared authority model only. It never establishes an issuer-defined official transition, an issuer relationship or any authorization.")
        }
        RequirementCondition::SupportedShapeCoverage => {
            let s = e.stress.as_ref();
            let effect = match s {
                Some(s) => {
                    observed.push(format!("{} of {} discovered executable state shapes have at least one exact executed case. {} positive-balance accounts use states this executor cannot exercise at all.",s.executable_shapes_with_executed_case,s.executable_shapes,s.unsupported_positive_balance_accounts));
                    for shape in &s.uncovered_executable_shapes {
                        observed.push(format!(
                            "Executable state shape {shape} has no executed case."
                        ));
                    }
                    ids.extend(s.evidence_ids.clone());
                    if s.executable_shapes > 0
                        && s.executable_shapes_with_executed_case == s.executable_shapes
                    {
                        FindingEffect::Satisfied
                    } else {
                        FindingEffect::IncompleteEvidence
                    }
                }
                None => FindingEffect::IncompleteEvidence,
            };
            (effect,"Every discovered executable-candidate state shape must have at least one exact executed case.","Exercising a state shape records which classes were reached. It never establishes anything about the other accounts sharing that shape; those remain untested.")
        }
        RequirementCondition::SelectedCaseOutcomes {
            allow_failed,
            allow_indeterminate,
            allow_unsupported,
        } => {
            let s = e.stress.as_ref();
            let effect = match s {
                Some(s) => {
                    observed.push(format!("{} of {} selected cases executed: {} proven, {} failed, {} indeterminate, {} unsupported.",s.executed_cases,s.selected_cases,s.proven_cases,s.failed_cases,s.indeterminate_cases,s.unsupported_cases));
                    for id in &s.failed_case_ids {
                        observed.push(format!("Selected case {id} actually executed the candidate conversion and failed, with its watched state rolled back."));
                    }
                    for id in &s.indeterminate_case_ids {
                        observed.push(format!("Selected case {id} could not establish an outcome from public state; this is missing evidence, not an execution failure."));
                    }
                    for id in &s.unsupported_case_ids {
                        observed.push(format!("Selected case {id} is outside the current executor's supported configuration; this is an executor boundary, not a failure."));
                    }
                    ids.extend(s.evidence_ids.clone());
                    if s.failed_cases > 0 && !allow_failed {
                        FindingEffect::Blocking
                    } else if s.selected_cases == 0
                        || s.proven_cases != s.selected_cases
                            && ((s.indeterminate_cases > 0 && !allow_indeterminate)
                                || (s.unsupported_cases > 0 && !allow_unsupported)
                                || s.proven_cases + usize::from(*allow_failed) * s.failed_cases
                                    != s.selected_cases)
                    {
                        FindingEffect::IncompleteEvidence
                    } else {
                        FindingEffect::Satisfied
                    }
                }
                None => FindingEffect::IncompleteEvidence,
            };
            (effect,"Every exact selected case must have executed and reconciled, with no disallowed failed, indeterminate or unsupported outcome.","A failed case is an actual executed instruction that failed with verified rollback, and it blocks. Unsupported and indeterminate outcomes are executor and evidence boundaries; they leave the finding incomplete and are never collapsed into failure.")
        }
        RequirementCondition::PopulationAcquisition {
            required_enumeration,
            required_authority_resolution,
            max_unsupported_positive_balance_accounts,
        } => {
            let s = e.stress.as_ref();
            let effect = match s {
                Some(s) => {
                    observed.push(format!("Token-account enumeration completeness: {} (policy requires {}). Authority resolution completeness: {} (policy requires {}). These two axes are independent.",s.enumeration_completeness,required_enumeration,s.authority_resolution_completeness,required_authority_resolution));
                    observed.push(format!("{} positive-balance accounts are unsupported by the current executor and {} still need authority capture; the policy allows at most {}.",s.unsupported_positive_balance_accounts,s.capture_required_positive_balance_accounts,max_unsupported_positive_balance_accounts));
                    ids.extend(s.evidence_ids.clone());
                    if s.enumeration_completeness == *required_enumeration
                        && s.authority_resolution_completeness == *required_authority_resolution
                        && s.unsupported_positive_balance_accounts
                            + s.capture_required_positive_balance_accounts
                            <= *max_unsupported_positive_balance_accounts
                    {
                        FindingEffect::Satisfied
                    } else {
                        FindingEffect::IncompleteEvidence
                    }
                }
                None => FindingEffect::IncompleteEvidence,
            };
            (effect,"The fresh population acquisition must meet the policy's declared completeness on both independent axes, and leave no more unresolved positive-balance state than the policy allows.","Enumeration completeness and authority-resolution completeness are separate facts. An incomplete acquisition leaves this finding incomplete; it is never evidence that the missing accounts cannot convert.")
        }
        RequirementCondition::PopulationConversionCoverage {
            plan_sha256,
            program_sha256,
        } => {
            let s = e.stress.as_ref();
            let effect = match s {
                Some(s)
                    if s.candidate_plan_sha256 == *plan_sha256
                        && s.candidate_program_sha256 == *program_sha256 =>
                {
                    observed.push(format!("{} of {} positive-balance accounts observed in this capture have their own proven candidate conversion at their own full observed amount.",s.proven_cases,s.positive_balance_accounts));
                    ids.extend(s.evidence_ids.clone());
                    if s.positive_balance_accounts > 0
                        && s.proven_cases == s.positive_balance_accounts
                    {
                        FindingEffect::Satisfied
                    } else {
                        FindingEffect::IncompleteEvidence
                    }
                }
                _ => FindingEffect::IncompleteEvidence,
            };
            (effect,"Every positive-balance account observed in the capture must have its own proven candidate conversion at its own full amount, under the declared plan and candidate program build.","A bounded selected sample cannot satisfy this. Neither a tested entity nor a tested state shape establishes anything about the accounts that were never executed.")
        }
        RequirementCondition::EvidenceIsolation => {
            observed.push(format!(
                "Evidence identity/scope isolation verified: {}.",
                e.isolation_verified
            ));
            ids.extend(e.evidence_refs.iter().map(|r| r.id.clone()));
            (if e.isolation_verified{FindingEffect::Satisfied}else{FindingEffect::IncompleteEvidence},"No path/entity/venue/amount/fee/closure proof escalation.","The gate consumes only pinned measurements with explicit source and scope bindings; no readiness finding grants new execution evidence.")
        }
    };
    observed.sort();
    observed.dedup();
    if observed.is_empty() {
        observed.push("No matching scoped evidence.".into());
    }
    ids.sort();
    ids.dedup();
    Ok(ReadinessFinding {
        requirement_id: r.id.clone(),
        label: r.label.clone(),
        required: r.required,
        scope: r.target.clone(),
        effect: if r.required {
            effect
        } else {
            FindingEffect::Informational
        },
        expected: expected.into(),
        observed,
        evidence_ids: ids,
        reason: reason.into(),
        remediation_requirement: r.remediation_requirement.clone(),
    })
}
pub fn evaluate(
    policy: &LifecycleReadinessPolicy,
    e: &VerifiedReadinessEvidence,
) -> Result<LifecycleReadinessReport> {
    let policy = policy.normalized()?;
    ensure!(
        policy.asset_mint == e.asset_mint && policy.scenario_sha256 == e.scenario_sha256,
        "assurance policy/evidence asset or economic scenario mismatch"
    );
    let findings = policy
        .requirements
        .iter()
        .map(|r| evaluate_requirement(r, e))
        .collect::<Result<Vec<_>>>()?;
    let overall_status =
        aggregate_effects(findings.iter().filter(|f| f.required).map(|f| &f.effect));
    let entity_ids = policy
        .requirements
        .iter()
        .filter(|r| r.required)
        .filter_map(|r| match &r.target {
            RequirementTarget::Entity { entity_id } => Some(entity_id.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let entity_readiness = entity_ids
        .into_iter()
        .map(|id| {
            let rows = findings
                .iter()
                .filter(|f| {
                    f.scope
                        == RequirementTarget::Entity {
                            entity_id: id.clone(),
                        }
                })
                .collect::<Vec<_>>();
            EntityReadiness {
                entity_id: id,
                status: aggregate_effects(rows.iter().filter(|f| f.required).map(|f| &f.effect)),
                requirement_ids: rows.iter().map(|f| f.requirement_id.clone()).collect(),
            }
        })
        .collect();
    let rollout_readiness = if policy.evaluated_scope == EvaluatedScope::PopulationRolloutReadiness
    {
        let pop_conditions = policy
            .requirements
            .iter()
            .filter_map(|r| match &r.condition {
                RequirementCondition::PopulationCoverage { any_of_paths } if r.required => {
                    Some(any_of_paths)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        // A conversion-coverage requirement additionally demands that this exact
        // entity has its own proven candidate conversion. An entity is satisfied
        // only when every required population condition holds for it.
        let conversion_required = policy.requirements.iter().any(|r| {
            r.required
                && matches!(
                    r.condition,
                    RequirementCondition::PopulationConversionCoverage { .. }
                )
        });
        let satisfied = |entity: &PopulationEntityFact| {
            entity.balance_raw != "0"
                && pop_conditions.iter().all(|paths| {
                    entity
                        .proven_full_amount_paths
                        .iter()
                        .any(|p| paths.contains(p))
                })
                && (!conversion_required || entity.proven_candidate_conversion)
        };
        let n = e
            .population
            .entities
            .iter()
            .filter(|x| satisfied(x))
            .count();
        Some(PopulationReadiness {
            status: overall_status,
            positive_entities_required: e.population.positive_balance_entities,
            exact_entities_satisfied: n,
            exhaustive_execution: e.population.positive_balance_entities > 0
                && n == e.population.positive_balance_entities,
            unresolved_account_types: {
                let mut counts = BTreeMap::new();
                for entity in &e.population.entities {
                    if entity.balance_raw != "0" && !satisfied(entity) {
                        *counts.entry(entity.account_type.clone()).or_insert(0) += 1;
                    }
                }
                counts
            },
        })
    } else {
        None
    };
    let prevented_rollout_conditions=findings.iter().filter(|f|f.effect!=FindingEffect::Satisfied).map(|f|{
        let r=policy.requirements.iter().find(|r|r.id==f.requirement_id).unwrap();
        PreflightFailureMode{requirement_id:r.id.clone(),scope:r.target.clone(),assumption:r.rollout_assumption.clone(),observed:f.observed.clone(),effect:f.effect,evidence_ids:f.evidence_ids.clone(),preflight_effect:"Eplyx would prevent a rollout from relying on this unverified, incompatible or contradicted assumption. This report is not a claim that a real rollout was blocked or a loss prevented.".into(),real_incident_claimed:false}
    }).collect();
    let mut path_evidence = e.path_facts.clone();
    path_evidence.sort_by_cached_key(|f| canonical(f).expect("serializable fact"));
    let mut complete = e.complete_exits.clone();
    complete.sort_by(|a, b| a.scope.entity_id.cmp(&b.scope.entity_id));
    let mut refs = e.evidence_refs.clone();
    refs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(LifecycleReadinessReport{schema_version:1,policy_semantic_sha256:digest(&policy)?,asset_mint:e.asset_mint.clone(),lifecycle_event:e.lifecycle_event.clone(),policy_evaluated_at:e.policy_evaluated_at.clone(),evaluated_scope:policy.evaluated_scope,overall_status,requirements:policy.requirements.clone(),policy,findings,entity_readiness,rollout_readiness,path_evidence,position_exit_evidence:complete,
        population_summary:PopulationSummary{token_account_entities:e.population.token_account_entities,positive_balance_entities:e.population.positive_balance_entities,distinct_owner_authorities:e.population.distinct_owner_authorities,entities_with_measured_amount:e.population.entities_with_measured_amount,represented_amount_raw:e.population.represented_amount_raw.clone(),covered_amount_raw:e.population.covered_amount_raw.clone(),without_evidence_raw:e.population.without_evidence_raw.clone(),account_types:e.population.account_types.clone(),execution_status_counts:e.population.execution_status_counts.clone()},prevented_rollout_conditions,evidence_refs:refs,
        limitations:vec!["This is a demonstration assurance policy, not an issuer's internal rollout policy, a subjective risk score or an asset safety judgment.".into(),"No VM execution or RPC is performed by the readiness gate. It verifies declared published measurement digests and bindings, then evaluates scoped evidence; it creates no new execution proof.".into(),"Ready means only that this policy's required conditions are met under its declared captured-state/signer assumptions; key possession, freshness/inclusion and future liquidity remain unproved.".into(),"The two state shapes retain their distinct captured banks. Entity actionability, population assurance, issuer lifecycle completion, principal unwind and full position exit are separate conclusions.".into(),"Earlier population adapter Unsupported statuses do not prove issuer/private blockers. Missing scoped evidence remains Incomplete unless an explicit policy blocker is evidenced.".into()]})
}
impl LifecycleReadinessReport {
    pub fn to_json(&self) -> Result<String> {
        canonical(self)
    }
    pub fn render_text(&self) -> String {
        let mut s=format!("LIFECYCLE PRE-FLIGHT\nAsset: {}\nPolicy: {} (DemoAssurancePolicy; not issuer policy)\nEvaluated scope: {:?}\nOverall readiness: {:?}\n\n",self.asset_mint,self.policy.id,self.evaluated_scope,self.overall_status);
        for f in &self.findings {
            s.push_str(&format!(
                "{:?}: {} [{}]\n",
                f.effect, f.label, f.requirement_id
            ));
            let requirement = self
                .requirements
                .iter()
                .find(|r| r.id == f.requirement_id)
                .expect("finding requirement");
            if let RequirementCondition::Path { any_of } = &requirement.condition {
                for c in any_of {
                    let exact = self
                        .path_evidence
                        .iter()
                        .find(|p| p.path_type == c.path_type && matches_scope(&p.scope, &c.scope));
                    let observed = exact.or_else(|| {
                        self.path_evidence.iter().find(|p| {
                            p.path_type == c.path_type && p.scope.entity_id == c.scope.entity_id
                        })
                    });
                    if let Some(p) = observed {
                        s.push_str(&format!(
                            "  {:?}: {:?}; requested raw {:?}; exact scope {}; bank {:?}.\n",
                            p.path_type,
                            p.status,
                            c.scope.exact_amount_raw,
                            exact.is_some(),
                            p.scope.captured_slot
                        ));
                        if exact.is_none() || p.status == PathStatus::Failed {
                            s.push_str(&format!("  {}\n", p.reason));
                        }
                    } else {
                        s.push_str("  No matching scoped evidence.\n");
                    }
                }
            } else {
                for o in &f.observed {
                    s.push_str(&format!("  {o}\n"));
                }
            }
        }
        s.push_str(&format!(
            "\nPopulation: {} accounts; {} positive; {} entities with measured amount.\n",
            self.population_summary.token_account_entities,
            self.population_summary.positive_balance_entities,
            self.population_summary.entities_with_measured_amount
        ));
        for f in &self.position_exit_evidence {
            s.push_str(&format!("Principal removed by mint (raw): {:?}; public owner credit: {:?}; destination withheld: {:?}.\n",f.principal_removed_raw,f.owner_received_raw,f.destination_withheld_raw));
            s.push_str(&format!(
                "Position {}: principal {:?}; fees {:?}; closure {:?}; remaining {:?}.\n",
                f.scope.entity_id,
                f.principal_unwind,
                f.fee_collection,
                f.position_closure,
                f.residual_fees_raw
            ));
        }
        s.push_str(
            "\nNo real incident/loss is claimed. No mainnet action or new execution occurs.\n",
        );
        s
    }
}
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanProvenance {
    /// A holder/visitor hypothesis about a future transition. Not executable.
    UserProposed,
    /// A candidate mechanism and configuration the operator intends to deploy.
    OperatorSupplied,
    /// An issuer-attested mechanism. No adapter and no accepted evidence exists here.
    IssuerVerified,
    /// A mechanism independently established from public issuer artifacts. None exists.
    PublicIssuerMechanism,
}
