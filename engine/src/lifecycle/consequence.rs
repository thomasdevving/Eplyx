//! Pure lifecycle consequences over validated, immutable production observations.
//! No RPC client, transaction builder, valuation or issuer-specific rule lives here.
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, path::Path};

use super::{
    decode::TokenAccountState,
    exposure::{sha256, ProtocolEvidence},
    policy::{LifecycleScenario, LifecycleStatus},
    AuthorityObservation, EntityType, EvidenceRef, LifecycleSnapshot,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LifecycleImpactClassification {
    Unaffected,
    RequiresTransition,
    StaleExposure,
    Unresolved,
}

/// Phase 4 deliberately has no successful-execution or stranded variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleExecutionStatus {
    NotTested,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleEvidenceRef {
    pub source_id: String,
    pub policy_fields: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BalanceObservationBasis {
    Phase2TokenAccount,
    Phase3VerifiedVault,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepresentedBalance {
    pub raw: String,
    pub decimal_base_units: String,
    pub basis: BalanceObservationBasis,
    /// Public SPL amount only, excluding withheld fees and encrypted balances.
    pub amount_definition: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleImpact {
    pub entity_id: String,
    pub token_account: String,
    pub entity_type: EntityType,
    pub verified_role: Option<String>,
    /// Lacks wallet compatibility or a verified integration. Never human identity proof.
    pub role_uncertain: bool,
    pub asset_mint: String,
    pub balance: RepresentedBalance,
    pub authority_observation: AuthorityObservation,
    pub authority_evidence: EvidenceRef,
    pub pre_lifecycle_status: LifecycleStatus,
    pub post_lifecycle_status: LifecycleStatus,
    pub pre_classification: LifecycleImpactClassification,
    pub impact_classification: LifecycleImpactClassification,
    pub economic_meaning_changed: bool,
    pub onchain_evidence: Vec<ProtocolEvidence>,
    pub lifecycle_evidence: Vec<LifecycleEvidenceRef>,
    pub reason: String,
    pub execution_status: LifecycleExecutionStatus,
}

/// A separate observation of the SAME entity, never added to holder capital totals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolLifecycleImpact {
    pub exposure_id: String,
    pub entity_id: String,
    pub entity_type: EntityType,
    pub verified_role: String,
    pub pool_address: String,
    pub token_account: String,
    pub asset_mint: String,
    pub balance: RepresentedBalance,
    pub pre_lifecycle_status: LifecycleStatus,
    pub post_lifecycle_status: LifecycleStatus,
    pub pre_classification: LifecycleImpactClassification,
    pub impact_classification: LifecycleImpactClassification,
    pub economic_meaning_changed: bool,
    pub onchain_evidence: Vec<ProtocolEvidence>,
    pub lifecycle_evidence: Vec<LifecycleEvidenceRef>,
    pub reason: String,
    pub execution_status: LifecycleExecutionStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleImpactSummary {
    pub entities_evaluated: usize,
    pub positive_balance_entities: usize,
    pub zero_public_balance_entities: usize,
    pub zero_public_balance_unresolved: usize,
    pub unknown_role_positive_entities: usize,
    pub uncertain_role_positive_entities: usize,
    pub classifications: BTreeMap<LifecycleImpactClassification, usize>,
    pub economic_meaning_changed_entities: usize,
    pub phase2_public_raw_exposure: String,
    pub phase2_affected_public_raw_exposure: String,
    pub verified_liquidity_venues: usize,
    pub verified_protocol_target_vaults: usize,
    pub stale_protocol_exposures: usize,
    pub phase3_verified_vault_public_raw_exposure: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleStateView {
    pub evaluated_at: DateTime<Utc>,
    pub lifecycle_status: LifecycleStatus,
    pub snapshot_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TechnicalStateDiff {
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleDiff {
    pub technical_state: TechnicalStateDiff,
    pub lifecycle_status_changed: bool,
    pub economic_meaning_changed_entities: usize,
    pub economic_meaning_changed_protocol_observations: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleImpactReport {
    pub schema_version: u32,
    pub asset_mint: String,
    pub scenario: LifecycleScenario,
    pub scenario_sha256: String,
    pub before: LifecycleStateView,
    pub after: LifecycleStateView,
    pub diff: LifecycleDiff,
    pub entities: Vec<LifecycleImpact>,
    pub protocol_observations: Vec<ProtocolLifecycleImpact>,
    pub summary: LifecycleImpactSummary,
    pub limitations: Vec<String>,
}

fn balance(state: &TokenAccountState, basis: BalanceObservationBasis) -> RepresentedBalance {
    RepresentedBalance {
        raw: state.raw_balance.clone(),
        decimal_base_units: state.ui_balance.clone(),
        basis,
        amount_definition: "Public SPL account amount / mint decimals; excludes withheld fees, encrypted balances and scaled display transforms".into(),
    }
}

fn classify(
    state: &TokenAccountState,
    status: LifecycleStatus,
    vault: bool,
) -> Result<(LifecycleImpactClassification, String)> {
    let positive = state.raw_balance.parse::<u64>()? > 0;
    let encrypted = state
        .extensions
        .iter()
        .any(|e| e.extension_type == "ConfidentialTransferAccount");
    classify_public_exposure(positive, encrypted, status, vault)
}

/// Shared public-exposure semantics for token accounts and decoded positions.
pub(crate) fn classify_public_exposure(
    positive: bool,
    encrypted: bool,
    status: LifecycleStatus,
    vault: bool,
) -> Result<(LifecycleImpactClassification, String)> {
    use LifecycleImpactClassification as C;
    let (class, reason) = if !positive && encrypted {
        (C::Unresolved, "Public balance is zero, but confidential account state prevents a conclusion of zero represented exposure")
    } else if !positive {
        (
            C::Unaffected,
            "No represented public token amount; account existence alone is not affected capital",
        )
    } else {
        match status {
            LifecycleStatus::Active => (C::Unaffected, "Positive public token amount retains Active lifecycle semantics under this policy"),
            LifecycleStatus::Unknown => (C::Unresolved, "Positive public token amount is observed; lifecycle entitlement semantics are Unknown"),
            LifecycleStatus::TransitionRequired | LifecycleStatus::PostDeadlineTransitionRequired if vault => (C::StaleExposure, "Verified liquidity vault retains the original asset while external policy requires transition; exit or transition is not tested"),
            LifecycleStatus::TransitionRequired | LifecycleStatus::PostDeadlineTransitionRequired => (C::RequiresTransition, "Positive public token amount is subject to the scenario transition requirement; owner role and executable transition path are not inferred"),
            LifecycleStatus::Expired | LifecycleStatus::NoIssuerEntitlement => (C::StaleExposure, "Public token amount remains, while the policy ends issuer entitlement; recovery, market value and exit or transition are not tested"),
        }
    };
    let reason = if positive && encrypted {
        format!("{reason}. Additional encrypted exposure is not quantified")
    } else {
        reason.into()
    };
    Ok((class, reason))
}

fn policy_evidence(scenario: &LifecycleScenario) -> Vec<LifecycleEvidenceRef> {
    let mut evidence: Vec<_> = scenario
        .sources
        .iter()
        .map(|s| {
            let mut fields = s.supports.clone();
            fields.sort();
            LifecycleEvidenceRef {
                source_id: s.id.clone(),
                policy_fields: fields,
            }
        })
        .collect();
    evidence.sort_by(|a, b| a.source_id.cmp(&b.source_id));
    evidence
}

pub struct LifecycleConsequenceEvaluator;

impl LifecycleConsequenceEvaluator {
    pub fn evaluate(
        snapshot: &LifecycleSnapshot,
        scenario: &LifecycleScenario,
        before_at: DateTime<Utc>,
        at: DateTime<Utc>,
    ) -> Result<LifecycleImpactReport> {
        snapshot.validate()?;
        let world = sha256(snapshot.to_json()?.as_bytes());
        Self::evaluate_validated(snapshot, scenario, before_at, at, &world)
    }

    /// Internal fast path for an immutably owned, already validated frozen world.
    /// Shares all consequence logic with the public checked evaluator.
    pub(crate) fn evaluate_validated(
        snapshot: &LifecycleSnapshot,
        scenario: &LifecycleScenario,
        before_at: DateTime<Utc>,
        at: DateTime<Utc>,
        snapshot_sha256: &str,
    ) -> Result<LifecycleImpactReport> {
        scenario.validate()?;
        ensure!(
            scenario.policy.asset_mint == snapshot.asset.mint,
            "lifecycle policy mint differs from frozen snapshot"
        );
        ensure!(before_at <= at, "--before must not follow --at");
        let pre = scenario.policy.status_at(before_at);
        let post = scenario.policy.status_at(at);
        let world = snapshot_sha256.to_owned();
        let sources = policy_evidence(scenario);
        let mint_proof = ProtocolEvidence::lifecycle(
            snapshot,
            &snapshot.asset.mint,
            &snapshot.mint_evidence,
            "SPL mint / Token-2022 extensions 3.1.1",
        )?;
        let refinements: BTreeMap<_, _> = snapshot
            .exposures
            .iter()
            .flat_map(|g| &g.account_exposures)
            .filter_map(|e| {
                e.refinement
                    .as_ref()
                    .map(|r| (e.phase2_entity_id.as_str(), r))
            })
            .collect();
        let mut entities = Vec::new();
        let mut summary = LifecycleImpactSummary::default();
        for c in [
            LifecycleImpactClassification::Unaffected,
            LifecycleImpactClassification::RequiresTransition,
            LifecycleImpactClassification::StaleExposure,
            LifecycleImpactClassification::Unresolved,
        ] {
            summary.classifications.insert(c, 0);
        }
        let mut total = 0u128;
        let mut affected = 0u128;
        for entity in &snapshot.entities {
            let refinement = refinements.get(entity.id.as_str());
            let vault = refinement.is_some_and(|r| r.classification == "LiquidityVault");
            let uncertain =
                refinement.is_none() && entity.entity_type != EntityType::WalletCompatible;
            let raw = entity.state.raw_balance.parse::<u64>()?;
            let (pre_classification, _) = classify(&entity.state, pre, vault)?;
            let (classification, reason) = classify(&entity.state, post, vault)?;
            let changed = pre != post && raw > 0;
            let mut proofs = vec![
                ProtocolEvidence::lifecycle(
                    snapshot,
                    &entity.token_account,
                    &entity.token_account_evidence,
                    "SPL token account / Token-2022 extensions 3.1.1",
                )?,
                mint_proof.clone(),
            ];
            if let Some(r) = refinement {
                proofs.extend(r.evidence.clone());
            }
            entities.push(LifecycleImpact {
                entity_id: entity.id.clone(),
                token_account: entity.token_account.clone(),
                entity_type: entity.entity_type.clone(),
                verified_role: refinement.map(|r| r.classification.clone()),
                role_uncertain: uncertain,
                asset_mint: snapshot.asset.mint.clone(),
                balance: balance(&entity.state, BalanceObservationBasis::Phase2TokenAccount),
                authority_observation: entity.authority_observation.clone(),
                authority_evidence: entity.authority_evidence.clone(),
                pre_lifecycle_status: pre,
                post_lifecycle_status: post,
                pre_classification,
                impact_classification: classification,
                economic_meaning_changed: changed,
                onchain_evidence: proofs,
                lifecycle_evidence: sources.clone(),
                reason,
                execution_status: LifecycleExecutionStatus::NotTested,
            });
            summary.entities_evaluated += 1;
            summary.positive_balance_entities += usize::from(raw > 0);
            summary.zero_public_balance_entities += usize::from(raw == 0);
            summary.zero_public_balance_unresolved += usize::from(
                raw == 0 && classification == LifecycleImpactClassification::Unresolved,
            );
            summary.unknown_role_positive_entities +=
                usize::from(raw > 0 && entity.entity_type == EntityType::Unknown);
            summary.uncertain_role_positive_entities += usize::from(raw > 0 && uncertain);
            *summary.classifications.get_mut(&classification).unwrap() += 1;
            summary.economic_meaning_changed_entities += usize::from(changed);
            total = total
                .checked_add(u128::from(raw))
                .context("public amount sum overflow")?;
            if classification != LifecycleImpactClassification::Unaffected {
                affected = affected
                    .checked_add(u128::from(raw))
                    .context("affected amount sum overflow")?;
            }
        }
        entities.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
        summary.phase2_public_raw_exposure = total.to_string();
        summary.phase2_affected_public_raw_exposure = affected.to_string();
        let mut protocol_observations = Vec::new();
        let mut protocol_total = 0u128;
        for exposure in snapshot
            .exposures
            .iter()
            .flat_map(|g| &g.protocol_exposures)
        {
            summary.verified_liquidity_venues += 1;
            let mut stale = false;
            for asset in exposure
                .assets
                .iter()
                .filter(|a| a.mint == snapshot.asset.mint)
            {
                let link = asset
                    .phase2_link
                    .as_ref()
                    .context("verified target vault lacks parent entity link")?;
                let (pre_classification, _) = classify(&asset.state, pre, true)?;
                let (classification, reason) = classify(&asset.state, post, true)?;
                let raw = asset.state.raw_balance.parse::<u64>()?;
                stale |= classification == LifecycleImpactClassification::StaleExposure;
                protocol_total = protocol_total
                    .checked_add(u128::from(raw))
                    .context("protocol amount sum overflow")?;
                summary.verified_protocol_target_vaults += 1;
                protocol_observations.push(ProtocolLifecycleImpact {
                    exposure_id: exposure.id.clone(),
                    entity_id: link.phase2_entity_id.clone(),
                    entity_type: link.original_classification.clone(),
                    verified_role: "LiquidityVault".into(),
                    pool_address: exposure.pool_address.clone(),
                    token_account: asset.vault.clone(),
                    asset_mint: asset.mint.clone(),
                    balance: balance(&asset.state, BalanceObservationBasis::Phase3VerifiedVault),
                    pre_lifecycle_status: pre,
                    post_lifecycle_status: post,
                    pre_classification,
                    impact_classification: classification,
                    economic_meaning_changed: pre != post && raw > 0,
                    onchain_evidence: vec![
                        exposure.pool_evidence.clone(),
                        exposure.program_evidence.clone(),
                        asset.vault_evidence.clone(),
                        asset.mint_evidence.clone(),
                    ],
                    lifecycle_evidence: sources.clone(),
                    reason,
                    execution_status: LifecycleExecutionStatus::NotTested,
                });
            }
            summary.stale_protocol_exposures += usize::from(stale);
        }
        protocol_observations.sort_by(|a, b| {
            (&a.exposure_id, &a.token_account).cmp(&(&b.exposure_id, &b.token_account))
        });
        summary.phase3_verified_vault_public_raw_exposure = protocol_total.to_string();
        let changed_protocol = protocol_observations
            .iter()
            .filter(|i| i.economic_meaning_changed)
            .count();
        Ok(LifecycleImpactReport {
            schema_version: 1, asset_mint: snapshot.asset.mint.clone(), scenario: scenario.clone(),
            scenario_sha256: scenario.sha256()?,
            before: LifecycleStateView { evaluated_at: before_at, lifecycle_status: pre, snapshot_sha256: world.clone() },
            after: LifecycleStateView { evaluated_at: at, lifecycle_status: post, snapshot_sha256: world },
            diff: LifecycleDiff { technical_state: TechnicalStateDiff::Unchanged, lifecycle_status_changed: pre != post,
                economic_meaning_changed_entities: summary.economic_meaning_changed_entities,
                economic_meaning_changed_protocol_observations: changed_protocol },
            entities, protocol_observations, summary,
            limitations: vec![
                "Lifecycle semantics are external assertions and explicit scenario assumptions, not facts decoded from chain bytes".into(),
                "Evaluation times are hypothetical semantic views of the same frozen observations, not historical or future chain captures".into(),
                "Phase 2 entity balances and Phase 3 protocol vault balances retain their separate RPC contexts; protocol amounts overlap holder amounts and must not be added".into(),
                "Only public SPL account amounts are quantified; withheld fees, encrypted amounts, scaled displays and LP entitlements are excluded".into(),
                "No transferability, swap, withdrawal, conversion, recovery, exitability, market value or stranded-position claim is proved; every execution status is NotTested".into(),
                "Exactly the captured entities and verified venues are evaluated; roles, owners, successor routes and undiscovered integrations are not inferred".into(),
            ],
        })
    }
}

impl LifecycleImpactReport {
    pub fn to_json(&self) -> Result<String> {
        crate::canonical::document(self)
    }
    /// Recompute every impact and summary from the frozen world and embedded policy.
    pub fn validate(&self, snapshot: &LifecycleSnapshot) -> Result<()> {
        ensure!(
            *self
                == LifecycleConsequenceEvaluator::evaluate(
                    snapshot,
                    &self.scenario,
                    self.before.evaluated_at,
                    self.after.evaluated_at
                )?,
            "lifecycle report differs from deterministic consequence model"
        );
        Ok(())
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| {
                format!("creating impact report {} (must not exist)", path.display())
            })?;
        file.write_all(self.to_json()?.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }
    pub fn render_text(&self) -> String {
        let s = &self.summary;
        let mut text = format!("Lifecycle Impact\nMint: {}\nScenario: {} ({})\nBefore: {} — {:?}\nAfter: {} — {:?}\nTechnical state: unchanged (same frozen snapshot SHA-256 {})\n\nEntities evaluated: {}\nPositive public balances: {}\nZero public balances: {}\nUnknown-role positive balances: {}\nOther/unknown uncertain-role positive balances: {}\nEconomic meaning changed: {}\n",
            self.asset_mint,self.scenario.id,self.scenario.scenario_version,
            self.before.evaluated_at,self.before.lifecycle_status,self.after.evaluated_at,self.after.lifecycle_status,
            self.before.snapshot_sha256,s.entities_evaluated,s.positive_balance_entities,s.zero_public_balance_entities,
            s.unknown_role_positive_entities,s.uncertain_role_positive_entities,s.economic_meaning_changed_entities);
        for (c, n) in &s.classifications {
            text.push_str(&format!("{c:?}: {n}\n"));
        }
        text.push_str(&format!(
            "Verified liquidity venues: {}\nStale protocol exposures: {}\n",
            s.verified_liquidity_venues, s.stale_protocol_exposures
        ));
        for p in &self.protocol_observations {
            text.push_str(&format!("\nVerified vault: {}\nPool: {}\nPhase 3 public amount: {} ({} raw)\nSame on-chain observation: {:?} -> {:?}\nImpact: {:?}\nExecution: {:?}\n",
                p.token_account,p.pool_address,p.balance.decimal_base_units,p.balance.raw,p.pre_lifecycle_status,p.post_lifecycle_status,p.impact_classification,p.execution_status));
        }
        for limitation in &self.limitations {
            text.push_str(&format!("\n{limitation}\n"));
        }
        text
    }
}
