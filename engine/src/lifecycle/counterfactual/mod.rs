//! Same captured bytes, different policy time. No capture or executor is called.
//! Proof contexts remain historical; applicability is a separate interpretation.
use crate::{
    lifecycle::frozen::selection::{canonical, digest, load},
    lifecycle::readiness::{
        self, evidence::ReadinessEvidenceManifest, LifecycleReadinessPolicy,
        LifecycleReadinessReport, ReadinessStatus,
    },
    lifecycle::resolution::{LifecycleResolution, PathStatus},
    lifecycle::{
        consequence::{
            classify_public_exposure, LifecycleConsequenceEvaluator,
            LifecycleImpactClassification as Impact, LifecycleImpactReport, LifecycleImpactSummary,
        },
        exposure::sha256,
        policy::{LifecycleScenario, LifecycleStatus},
        EntityType, LifecycleSnapshot,
    },
    path::position::{ProtocolPosition, WithdrawalReport},
    path::ExitPathType,
};
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CounterfactualScenario {
    pub id: String,
    pub label: String,
    pub lifecycle_policy_sha256: String,
    pub evaluation_time: DateTime<Utc>,
}
/// A world is a frozen collection of independently captured observations, not
/// one historical validator bank. Position and holder banks stay distinct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionIdentity {
    pub snapshot_sha256: String,
    pub exposure_snapshot_sha256: String,
    pub position_fixture_sha256: String,
    pub position_discovery_sha256: String,
    pub raw_position_sha256: String,
    pub evidence_artifact_digests: BTreeMap<String, String>,
    pub production_state_digest: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateApplicability {
    PreEvent,
    Applicable,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadinessImplication {
    pub applicability: GateApplicability,
    pub policy_result: Option<ReadinessStatus>,
    pub active_failure_mode_ids: Vec<String>,
    pub cause: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathImplication {
    pub entity_id: String,
    pub path_type: ExitPathType,
    pub historical_status: PathStatus,
    pub lifecycle_relevant: bool,
    pub cause: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedExposure {
    pub entity_id: String,
    pub raw_balance: String,
    pub raw_state_sha256: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub captured_slot: u64,
    pub lifecycle_status: LifecycleStatus,
    pub impact: Impact,
    pub cause: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopulationConsequences {
    pub positive_balance_entities: usize,
    pub requiring_transition: usize,
    pub stale_entities: usize,
    pub stale_protocol_exposures: usize,
    pub unaffected_zero_balance_entities: usize,
    pub unknown_role_affected_entities: usize,
    pub verified_protocol_integrations_affected: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterfactualResult {
    pub scenario: CounterfactualScenario,
    pub production_state_digest: String,
    pub balances_sha256: String,
    pub historical_execution_evidence_sha256: String,
    pub lifecycle_status: LifecycleStatus,
    /// Grouped entity IDs supply every classification without duplicating observations.
    pub entity_impacts: BTreeMap<Impact, Vec<usize>>,
    pub summary: LifecycleImpactSummary,
    pub population: PopulationConsequences,
    pub direct_holder: SelectedExposure,
    pub protocol_position: SelectedExposure,
    pub protocol_exposures: Vec<SelectedExposure>,
    pub path_implications: Vec<PathImplication>,
    pub readiness: ReadinessImplication,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityChange {
    pub from: Impact,
    pub to: Impact,
    pub entity_indices: Vec<usize>,
    pub cause: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffCategory {
    LifecycleMeaningChanged,
    ImpactClassificationChanged,
    PathRelevanceChanged,
    ReadinessChanged,
    OnChainStateChanged,
    Unchanged,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterfactualDiff {
    pub from_scenario: String,
    pub to_scenario: String,
    pub production_state_digest: String,
    pub changed_entities: Vec<EntityChange>,
    pub changed_lifecycle_states: Option<(LifecycleStatus, LifecycleStatus)>,
    pub changed_protocol_exposures: Vec<(SelectedExposure, SelectedExposure)>,
    pub changed_selected_exposures: Vec<(SelectedExposure, SelectedExposure)>,
    pub changed_path_implications: Vec<(PathImplication, PathImplication)>,
    pub changed_readiness_findings: Vec<String>,
    pub categories: Vec<DiffCategory>,
    pub unchanged_onchain_state: bool,
    pub lifecycle_meaning_changed: bool,
    pub onchain_state_changed: bool,
    pub cause: String,
    pub crossed_boundaries: Vec<String>,
    pub unchanged_facts: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CounterfactualReport {
    pub schema_version: u32,
    pub asset: String,
    pub production_world: ProductionIdentity,
    pub lifecycle_scenario: LifecycleScenario,
    pub scenario_sha256: String,
    pub lifecycle_policy_sha256: String,
    pub frozen_readiness: LifecycleReadinessReport,
    pub historical_direct_paths: serde_json::Value,
    pub historical_position_paths: serde_json::Value,
    pub frozen_position: ProtocolPosition,
    /// Sorted shared entity ledger; classification/diff rows use indices into it.
    pub population_entities: Vec<String>,
    pub scenarios: Vec<CounterfactualResult>,
    pub comparisons: Vec<CounterfactualDiff>,
    pub limitations: Vec<String>,
}

pub fn core_scenarios(policy: &LifecycleScenario) -> Result<Vec<CounterfactualScenario>> {
    let deadline = policy
        .policy
        .deadline
        .as_ref()
        .context("counterfactual requires an explicit deadline")?;
    let hash = digest(&policy.policy)?;
    Ok([
        (
            "before_transition",
            "Before transition",
            policy.policy.effective_at - Duration::nanoseconds(1),
        ),
        (
            "after_transition",
            "Transition effective, before deadline",
            policy.policy.effective_at,
        ),
        ("after_deadline", "Declared deadline effective", deadline.at),
    ]
    .into_iter()
    .map(|(id, label, evaluation_time)| CounterfactualScenario {
        id: id.into(),
        label: label.into(),
        lifecycle_policy_sha256: hash.clone(),
        evaluation_time,
    })
    .collect())
}

/// Reject mixed worlds and changed proof even when lifecycle statuses match.
pub fn compare(
    from: &CounterfactualResult,
    to: &CounterfactualResult,
    policy: &LifecycleScenario,
) -> Result<CounterfactualDiff> {
    ensure!(
        from.production_state_digest == to.production_state_digest,
        "different production-state digests: non-comparable counterfactual"
    );
    ensure!(
        from.direct_holder.entity_id == to.direct_holder.entity_id
            && from.protocol_position.entity_id == to.protocol_position.entity_id,
        "selected exposure identities changed"
    );
    ensure!(
        from.balances_sha256 == to.balances_sha256
            && from.direct_holder.raw_balance == to.direct_holder.raw_balance
            && from.protocol_position.raw_balance == to.protocol_position.raw_balance,
        "counterfactual token balances changed"
    );
    ensure!(
        from.historical_execution_evidence_sha256 == to.historical_execution_evidence_sha256,
        "historical execution evidence changed"
    );
    ensure!(
        from.scenario.lifecycle_policy_sha256 == to.scenario.lifecycle_policy_sha256
            && from.scenario.lifecycle_policy_sha256 == digest(&policy.policy)?,
        "counterfactual lifecycle policies differ"
    );
    ensure!(
        from.direct_holder.raw_state_sha256 == to.direct_holder.raw_state_sha256
            && from.direct_holder.captured_slot == to.direct_holder.captured_slot
            && from.protocol_position.raw_state_sha256 == to.protocol_position.raw_state_sha256
            && from.protocol_position.captured_slot == to.protocol_position.captured_slot,
        "selected captured account bytes or banks changed"
    );
    ensure!(
        from.protocol_exposures.len() == to.protocol_exposures.len(),
        "protocol exposure population changed"
    );
    let mut changed_protocol_exposures = vec![];
    for (a, b) in from.protocol_exposures.iter().zip(&to.protocol_exposures) {
        ensure!(
            a.entity_id == b.entity_id
                && a.raw_balance == b.raw_balance
                && a.raw_state_sha256 == b.raw_state_sha256
                && a.captured_slot == b.captured_slot,
            "protocol vault bytes or balances changed"
        );
        if a.lifecycle_status != b.lifecycle_status || a.impact != b.impact {
            changed_protocol_exposures.push((a.clone(), b.clone()));
        }
    }
    let changed_selected_exposures = [
        (&from.direct_holder, &to.direct_holder),
        (&from.protocol_position, &to.protocol_position),
    ]
    .into_iter()
    .filter(|(a, b)| a.lifecycle_status != b.lifecycle_status || a.impact != b.impact)
    .map(|(a, b)| (a.clone(), b.clone()))
    .collect();
    let a = assignments(from)?;
    let b = assignments(to)?;
    ensure!(
        a.keys().eq(b.keys()),
        "counterfactual entity population changed"
    );
    let mut groups: BTreeMap<(Impact, Impact), Vec<usize>> = BTreeMap::new();
    for (id, pre) in a {
        let post = b[&id];
        if pre != post {
            groups.entry((pre, post)).or_default().push(id);
        }
    }
    let cause = "Evaluation time changed under the identical explicit lifecycle policy; captured state and historical proof contexts remain frozen.".to_string();
    let changed_entities = groups
        .into_iter()
        .map(|((from, to), entity_indices)| EntityChange {
            from,
            to,
            entity_indices,
            cause: cause.clone(),
        })
        .collect::<Vec<_>>();
    ensure!(
        from.path_implications.len() == to.path_implications.len(),
        "path population changed"
    );
    let mut paths = vec![];
    for (a, b) in from.path_implications.iter().zip(&to.path_implications) {
        ensure!(
            a.entity_id == b.entity_id
                && a.path_type == b.path_type
                && a.historical_status == b.historical_status,
            "historical path proof changed"
        );
        if a.lifecycle_relevant != b.lifecycle_relevant {
            paths.push((a.clone(), b.clone()));
        }
    }
    let changed = from.lifecycle_status != to.lifecycle_status;
    let mut categories = vec![];
    if changed {
        categories.push(DiffCategory::LifecycleMeaningChanged);
    }
    if !changed_entities.is_empty() || from.protocol_position.impact != to.protocol_position.impact
    {
        categories.push(DiffCategory::ImpactClassificationChanged);
    }
    if !paths.is_empty() {
        categories.push(DiffCategory::PathRelevanceChanged);
    }
    let readiness_changed = from.readiness != to.readiness;
    if readiness_changed {
        categories.push(DiffCategory::ReadinessChanged);
    }
    if categories.is_empty() {
        categories.push(DiffCategory::Unchanged);
    }
    let lo = from
        .scenario
        .evaluation_time
        .min(to.scenario.evaluation_time);
    let hi = from
        .scenario
        .evaluation_time
        .max(to.scenario.evaluation_time);
    let mut crossed = vec![];
    if lo < policy.policy.effective_at && hi >= policy.policy.effective_at {
        crossed.push("/policy/effective_at".into());
    }
    if let Some(d) = &policy.policy.deadline {
        if lo < d.at && hi >= d.at {
            crossed.push("/policy/deadline/at".into());
        }
    }
    let finding_ids = if readiness_changed {
        from.readiness
            .active_failure_mode_ids
            .iter()
            .chain(&to.readiness.active_failure_mode_ids)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    } else {
        vec![]
    };
    Ok(CounterfactualDiff {
        from_scenario: from.scenario.id.clone(),
        to_scenario: to.scenario.id.clone(),
        production_state_digest: from.production_state_digest.clone(),
        changed_entities,
        changed_lifecycle_states: changed.then_some((from.lifecycle_status, to.lifecycle_status)),
        changed_protocol_exposures,
        changed_selected_exposures,
        changed_path_implications: paths,
        changed_readiness_findings: finding_ids,
        categories,
        unchanged_onchain_state: true,
        lifecycle_meaning_changed: changed,
        onchain_state_changed: false,
        cause,
        crossed_boundaries: crossed,
        unchanged_facts: vec![
            "token-account bytes and balances".into(),
            "vault and pool bytes".into(),
            "captured position bytes, range, shares and exposure".into(),
            "deployed program bytecode".into(),
            "historical execution results, banks and signer assumptions".into(),
        ],
    })
}
fn assignments(r: &CounterfactualResult) -> Result<BTreeMap<usize, Impact>> {
    let mut m = BTreeMap::new();
    for (class, ids) in &r.entity_impacts {
        for id in ids {
            ensure!(
                m.insert(*id, *class).is_none(),
                "duplicate counterfactual entity"
            );
        }
    }
    Ok(m)
}

pub struct FrozenCounterfactualWorld {
    evidence: readiness::VerifiedReadinessEvidence,
    snapshot: LifecycleSnapshot,
    policy: LifecycleScenario,
    direct: LifecycleResolution,
    position: ProtocolPosition,
    direct_paths: serde_json::Value,
    position_paths: serde_json::Value,
    readiness: LifecycleReadinessReport,
    identity: ProductionIdentity,
    balances_sha256: String,
    proof_sha256: String,
}
impl FrozenCounterfactualWorld {
    /// Pins every supplied input through the existing trusted readiness manifest.
    pub fn load(snapshot_path: &Path, scenario_path: &Path, readiness_path: &Path) -> Result<Self> {
        let policy: LifecycleReadinessPolicy = load(readiness_path)?;
        let base = readiness_path.parent().unwrap_or(Path::new("."));
        let manifest: ReadinessEvidenceManifest =
            serde_json::from_slice(&policy.evidence_manifest.read(base)?)?;
        let manifest_path = base.join(&policy.evidence_manifest.file);
        let base = manifest_path
            .parent()
            .context("missing manifest directory")?;
        let verified = manifest.verify(
            base,
            snapshot_path,
            scenario_path,
            &base.join(&manifest.direct_resolution.file),
            &base.join(&manifest.position_resolution.file),
            &base.join(&manifest.coverage.file),
        )?;
        let readiness = readiness::evaluate(&policy, &verified)?;
        let snapshot = LifecycleSnapshot::load(snapshot_path)?;
        ensure!(
            digest(&snapshot)? == manifest.snapshot.sha256,
            "noncanonical frozen exposure snapshot"
        );
        let scenario = LifecycleScenario::load(scenario_path)?;
        let direct: LifecycleResolution =
            serde_json::from_slice(&manifest.direct_resolution.read(base)?)?;
        let withdrawal: WithdrawalReport =
            serde_json::from_slice(&manifest.position_resolution.read(base)?)?;
        // Re-derive the PRE-execution position from captured bytes. No VM replay.
        let position = crate::path::position::meteora_dlmm::discover(
            &snapshot,
            &scenario,
            &manifest.withdrawal_discovery.read(base)?,
            &manifest.withdrawal_fixture.read(base)?,
        )?;
        ensure!(
            position == withdrawal.position,
            "frozen pre-execution position differs from captured derivation"
        );
        let mut evidence_artifact_digests = BTreeMap::new();
        for r in &readiness.evidence_refs {
            evidence_artifact_digests.insert(r.id.clone(), r.artifact.sha256.clone());
        }
        evidence_artifact_digests.insert(
            "readiness-policy".into(),
            sha256(&crate::lifecycle::artifact::read(readiness_path)?),
        );
        evidence_artifact_digests.insert(
            "readiness-manifest".into(),
            policy.evidence_manifest.sha256.clone(),
        );
        let graph = snapshot
            .exposures
            .as_ref()
            .context("counterfactual requires verified exposure snapshot")?;
        let mut identity = ProductionIdentity {
            snapshot_sha256: graph.source_snapshot_sha256.clone(),
            exposure_snapshot_sha256: manifest.snapshot.sha256,
            position_fixture_sha256: manifest.withdrawal_fixture.sha256,
            position_discovery_sha256: manifest.withdrawal_discovery.sha256,
            raw_position_sha256: position.raw_position_sha256.clone(),
            evidence_artifact_digests,
            production_state_digest: String::new(),
        };
        // Lifecycle policy has its own binding; the world includes only captured
        // state and historical artifacts, not the current evaluation time.
        identity.production_state_digest = digest(&(
            &identity.snapshot_sha256,
            &identity.exposure_snapshot_sha256,
            &identity.position_fixture_sha256,
            &identity.position_discovery_sha256,
            &identity.raw_position_sha256,
        ))?;
        let balances_sha256 = digest(
            &snapshot
                .entities
                .iter()
                .map(|e| (&e.id, &e.state.raw_balance))
                .collect::<Vec<_>>(),
        )?;
        let direct_paths = serde_json::to_value(&direct.paths)?;
        let position_paths = serde_json::to_value(&withdrawal.paths)?;
        let proof_sha256 = digest(&(
            direct_paths.clone(),
            position_paths.clone(),
            &readiness.path_evidence,
            &readiness.position_exit_evidence,
        ))?;
        Ok(Self {
            evidence: verified,
            snapshot,
            policy: scenario,
            direct,
            position,
            direct_paths,
            position_paths,
            readiness,
            identity,
            balances_sha256,
            proof_sha256,
        })
    }
    pub fn scenarios(&self) -> Result<Vec<CounterfactualScenario>> {
        core_scenarios(&self.policy)
    }
    /// Read-only access to the already verified evidence, never a deserialized proof.
    pub(crate) fn verified_evidence(&self) -> &readiness::VerifiedReadinessEvidence {
        &self.evidence
    }
    pub fn evaluate(&self, views: &[CounterfactualScenario]) -> Result<CounterfactualReport> {
        ensure!(!views.is_empty(), "no counterfactual scenarios");
        let mut views = views.to_vec();
        views.sort_by(|a, b| {
            a.evaluation_time
                .cmp(&b.evaluation_time)
                .then(a.id.cmp(&b.id))
        });
        let mut ids = BTreeSet::new();
        let mut results = vec![];
        for view in views {
            ensure!(
                !view.id.trim().is_empty()
                    && !view.label.trim().is_empty()
                    && ids.insert(view.id.clone()),
                "duplicate or empty counterfactual scenario ID"
            );
            ensure!(
                view.lifecycle_policy_sha256 == digest(&self.policy.policy)?,
                "scenario lifecycle-policy digest mismatch"
            );
            let impact = LifecycleConsequenceEvaluator::evaluate_validated(
                &self.snapshot,
                &self.policy,
                view.evaluation_time,
                view.evaluation_time,
                &self.identity.exposure_snapshot_sha256,
            )?;
            results.push(self.result(view, &impact)?);
        }
        let comparisons = results
            .windows(2)
            .map(|r| compare(&r[0], &r[1], &self.policy))
            .collect::<Result<Vec<_>>>()?;
        Ok(CounterfactualReport{schema_version:1,asset:self.snapshot.asset.name.clone(),production_world:self.identity.clone(),lifecycle_scenario:self.policy.clone(),scenario_sha256:self.policy.sha256()?,lifecycle_policy_sha256:digest(&self.policy.policy)?,frozen_readiness:self.readiness.clone(),historical_direct_paths:self.direct_paths.clone(),historical_position_paths:self.position_paths.clone(),frozen_position:self.position.clone(),population_entities:{let mut ids=self.snapshot.entities.iter().map(|e|e.id.clone()).collect::<Vec<_>>();ids.sort();ids},scenarios:results,comparisons,limitations:vec!["Counterfactual times are demo policy evaluations of the same captured world, not historical-bank reconstructions or predictions of future chain state.".into(),"Position exposure is re-derived from frozen PRE-execution capture, never a simulated post-withdrawal account. Independent holder, protocol and position capture banks are preserved, not merged into one validator bank.".into(),"PreEvent is lifecycle-gate applicability, not a Ready assurance finding. Applicable views consume the identical pinned assurance policy and historical findings; deadline passage alone cannot create Blocked.".into(),"Historical path proof is exact and conditional on original amount/range/venue/bank/scenario and local signing assumptions. Key possession remains unknown. No proof inheritance, new execution, RPC, valuation or official conversion claim.".into(),"NoIssuerEntitlement is this explicit policy's post-deadline terminology. It does not establish worthlessness, illegality, loss or inability to exit.".into()]})
    }
    fn result(
        &self,
        scenario: CounterfactualScenario,
        impact: &LifecycleImpactReport,
    ) -> Result<CounterfactualResult> {
        let status = impact.after.lifecycle_status;
        let applicability = match status {
            LifecycleStatus::Active => GateApplicability::PreEvent,
            LifecycleStatus::Unknown => GateApplicability::Unknown,
            _ => GateApplicability::Applicable,
        };
        let relevant = applicability == GateApplicability::Applicable;
        let mut groups: BTreeMap<Impact, Vec<usize>> = BTreeMap::new();
        for (index, e) in impact.entities.iter().enumerate() {
            groups
                .entry(e.impact_classification)
                .or_default()
                .push(index);
        }
        for ids in groups.values_mut() {
            ids.sort();
        }
        let direct = impact
            .entities
            .iter()
            .find(|e| e.entity_id == self.direct.entity_id)
            .context("selected direct holder absent")?;
        let proof = direct
            .onchain_evidence
            .iter()
            .find(|e| e.account == direct.token_account)
            .context("direct account byte proof absent")?;
        let direct_holder = SelectedExposure {
            entity_id: direct.entity_id.clone(),
            raw_balance: direct.balance.raw.clone(),
            raw_state_sha256: proof.raw_data_sha256.clone(),
            captured_slot: proof.reference.slot,
            lifecycle_status: status,
            impact: direct.impact_classification,
            cause: direct.reason.clone(),
        };
        let asset_index = self
            .position
            .assets
            .iter()
            .position(|a| a == &impact.asset_mint)
            .context("position asset absent")?;
        let raw = self.position.principal_exposure_raw[asset_index].clone();
        let (position_impact, reason) =
            classify_public_exposure(raw.parse::<u128>()? > 0, false, status, true)?;
        let protocol_position = SelectedExposure {
            entity_id: format!("solana-program-position:{}", self.position.position_id),
            raw_balance: raw,
            raw_state_sha256: self.position.raw_position_sha256.clone(),
            captured_slot: self.position.captured_slot,
            lifecycle_status: status,
            impact: position_impact,
            cause: reason
                .replace("Verified liquidity vault", "Decoded protocol position")
                .replace(
                    "exit or transition is not tested",
                    "this consequence classification supplies no execution proof",
                ),
        };
        let mut path_implications = vec![];
        for row in &self.direct.paths {
            path_implications.push(PathImplication {
                entity_id: self.direct.entity_id.clone(),
                path_type: row.path_type,
                historical_status: row.status,
                lifecycle_relevant: relevant
                    && direct.balance.raw.parse::<u64>()? > 0
                    && row.status != PathStatus::NotApplicable,
                cause: path_cause(row.path_type, relevant, row.status),
            });
        }
        let rows: Vec<crate::path::position::PositionPath> =
            serde_json::from_value(self.position_paths.clone())?;
        for row in rows {
            path_implications.push(PathImplication {
                entity_id: protocol_position.entity_id.clone(),
                path_type: row.path_type,
                historical_status: row.status,
                lifecycle_relevant: relevant
                    && protocol_position.raw_balance.parse::<u128>()? > 0
                    && row.status != PathStatus::NotApplicable,
                cause: path_cause(row.path_type, relevant, row.status),
            });
        }
        let protocol_exposures = impact
            .protocol_observations
            .iter()
            .map(|e| {
                let proof = e
                    .onchain_evidence
                    .iter()
                    .find(|p| p.account == e.token_account)
                    .context("protocol vault byte proof absent")?;
                Ok(SelectedExposure {
                    entity_id: e.exposure_id.clone(),
                    raw_balance: e.balance.raw.clone(),
                    raw_state_sha256: proof.raw_data_sha256.clone(),
                    captured_slot: proof.reference.slot,
                    lifecycle_status: status,
                    impact: e.impact_classification,
                    cause: e.reason.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let population = PopulationConsequences {
            positive_balance_entities: impact.summary.positive_balance_entities,
            requiring_transition: impact.summary.classifications[&Impact::RequiresTransition],
            stale_entities: impact.summary.classifications[&Impact::StaleExposure],
            stale_protocol_exposures: impact.summary.stale_protocol_exposures,
            unaffected_zero_balance_entities: impact
                .entities
                .iter()
                .filter(|e| e.balance.raw == "0" && e.impact_classification == Impact::Unaffected)
                .count(),
            unknown_role_affected_entities: impact
                .entities
                .iter()
                .filter(|e| {
                    e.entity_type == EntityType::Unknown
                        && e.balance.raw != "0"
                        && e.impact_classification != Impact::Unaffected
                })
                .count(),
            verified_protocol_integrations_affected: impact
                .protocol_observations
                .iter()
                .filter(|e| e.balance.raw != "0" && e.impact_classification != Impact::Unaffected)
                .map(|e| &e.pool_address)
                .collect::<BTreeSet<_>>()
                .len(),
        };
        let readiness=ReadinessImplication{applicability,policy_result:relevant.then_some(self.readiness.overall_status),active_failure_mode_ids:if relevant {self.readiness.prevented_rollout_conditions.iter().map(|m|m.requirement_id.clone()).collect()} else {vec![]},cause:match applicability {GateApplicability::PreEvent=>"Active policy semantics: no immediate lifecycle action requirement. Frozen assurance findings are retained but rollout gate is PreEvent.",GateApplicability::Applicable=>"Non-Active known policy semantics: lifecycle action is relevant. Existing assurance policy evaluates unchanged exact proof; active failure modes link requirement, observed evidence gap and policy effect.",GateApplicability::Unknown=>"Unknown lifecycle semantics: gate applicability remains unknown; no Ready or Blocked inference."}.into()};
        Ok(CounterfactualResult {
            scenario,
            production_state_digest: self.identity.production_state_digest.clone(),
            balances_sha256: self.balances_sha256.clone(),
            historical_execution_evidence_sha256: self.proof_sha256.clone(),
            lifecycle_status: status,
            entity_impacts: groups,
            summary: impact.summary.clone(),
            population,
            direct_holder,
            protocol_position,
            protocol_exposures,
            path_implications,
            readiness,
        })
    }
    /// Regeneration is authoritative: digest strings alone cannot legitimize a
    /// tampered classification, relevance, readiness or stored proof row.
    pub fn validate(&self, report: &CounterfactualReport) -> Result<()> {
        let views = report
            .scenarios
            .iter()
            .map(|s| s.scenario.clone())
            .collect::<Vec<_>>();
        ensure!(
            *report == self.evaluate(&views)?,
            "counterfactual report differs from frozen deterministic regeneration"
        );
        Ok(())
    }
}
fn path_cause(path: ExitPathType, relevant: bool, status: PathStatus) -> String {
    if status == PathStatus::NotApplicable {
        return "NotApplicable to this exact entity/context; lifecycle time does not change path applicability.".into();
    }
    if !relevant {
        return "No lifecycle path relevance is asserted for this view; historical proof retained."
            .into();
    }
    match path {ExitPathType::OfficialTransition=>"Official lifecycle completion is relevant; independent official proof is required.",ExitPathType::Transfer=>"Token mobility is relevant; movement cannot prove sale or official conversion.",ExitPathType::SecondaryMarketExit=>"Market mobility is relevant within original exact scope; sale cannot prove official conversion.",ExitPathType::Withdrawal=>"Native principal unwind is relevant within original exact position scope; fees, closure and conversion remain separate.",ExitPathType::Redemption=>"Distinct redemption relevance grants no adapter, eligibility or execution proof.",ExitPathType::Unknown=>"Unknown path retains its evidence boundary."}.into()
}
impl CounterfactualReport {
    pub fn to_json(&self) -> Result<String> {
        canonical(self)
    }
    pub fn render_text(&self) -> String {
        let mut s = format!(
            "{} COUNTERFACTUAL LIFECYCLE ANALYSIS\nProduction world: UNCHANGED\nDigest: {}\n",
            self.asset, self.production_world.production_state_digest
        );
        if let Some(first) = self.scenarios.first() {
            s.push_str(&format!(
                "Direct holder: {}\nProtocol position: {}\n",
                first.direct_holder.entity_id, first.protocol_position.entity_id
            ));
        }
        for r in &self.scenarios {
            let gate = r.readiness.policy_result.map_or_else(
                || format!("{:?}", r.readiness.applicability),
                |status| format!("{:?} / {:?}", r.readiness.applicability, status),
            );
            s.push_str(&format!(
                "\n{} — {}\nLifecycle: {:?}\nPositive balances: {}\nRequiresTransition: {}\nStale entities: {}\nStale protocol exposures: {}\nUnaffected zero balances: {}\nUnknown-role affected: {}\nVerified integrations affected: {}\nDirect holder: {} raw / {:?}\nFrozen LP principal: {} raw / {:?}\nReadiness: {}\n",
                r.scenario.label, r.scenario.evaluation_time, r.lifecycle_status,
                r.population.positive_balance_entities, r.population.requiring_transition,
                r.population.stale_entities, r.population.stale_protocol_exposures,
                r.population.unaffected_zero_balance_entities, r.population.unknown_role_affected_entities,
                r.population.verified_protocol_integrations_affected,
                r.direct_holder.raw_balance, r.direct_holder.impact,
                r.protocol_position.raw_balance, r.protocol_position.impact, gate,
            ));
            for p in &r.path_implications {
                if p.historical_status == PathStatus::Proven
                    || p.path_type == ExitPathType::OfficialTransition
                {
                    let entity = if p.entity_id == r.direct_holder.entity_id {
                        "Direct"
                    } else {
                        "Position"
                    };
                    s.push_str(&format!(
                        "{} {:?}: {:?}; lifecycle relevant={}\n",
                        entity, p.path_type, p.historical_status, p.lifecycle_relevant
                    ));
                }
            }
        }
        for d in &self.comparisons {
            s.push_str(&format!(
                "\n{} → {}: {} entity classifications changed\nLifecycle meaning changed: {}\nOn-chain state changed: {}\nCause: {:?}\n",
                d.from_scenario, d.to_scenario,
                d.changed_entities.iter().map(|g| g.entity_indices.len()).sum::<usize>(),
                if d.lifecycle_meaning_changed { "YES" } else { "NO" },
                if d.onchain_state_changed { "YES" } else { "NO" },
                d.crossed_boundaries,
            ));
        }
        s.push_str("\nFrozen pre-execution observations; exact historical banks/signing assumptions retained. No capture or execution.\n");
        s
    }
}
