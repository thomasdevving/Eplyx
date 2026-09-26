//! Offline orchestration. Frozen measurement hashes never become hashes of the
//! newly generated scenario. A checked semantic bridge is explicit in the report.
use super::*;
use crate::{
    lifecycle::frozen::selection::load,
    lifecycle::notice::identity::{IdentityReport, MintVerification},
    lifecycle::readiness::{
        self, evidence::ReadinessEvidenceManifest, LifecycleReadinessPolicy,
        LifecycleReadinessReport,
    },
    lifecycle::{
        consequence::{LifecycleConsequenceEvaluator, LifecycleImpactReport},
        LifecycleSnapshot,
    },
};
use anyhow::Context;
use serde_json::Value;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeWorkflow {
    pub schema_version: u32,
    pub source: ArtifactRef,
    pub config: ArtifactRef,
    pub identity_report: ArtifactRef,
    pub readiness_policy: ArtifactRef,
}
impl NoticeWorkflow {
    pub fn load(path: &Path) -> Result<Self> {
        let w: Self = load(path)?;
        ensure!(w.schema_version == 1, "unknown notice workflow schema");
        Ok(w)
    }
    pub fn source(&self, base: &Path) -> Result<CapturedSourceDocument> {
        let s: CapturedSourceDocument = serde_json::from_slice(&self.source.read(base)?)?;
        s.verify(base)?;
        Ok(s)
    }
    pub fn config(&self, base: &Path) -> Result<DemoEvaluationConfig> {
        Ok(serde_json::from_slice(&self.config.read(base)?)?)
    }
    pub fn normalize(&self, base: &Path) -> Result<VerifiedLifecycleEvent> {
        let source = self.source(base)?;
        let mut event = prestocks::CapturedIssuerNotice.normalize(&source)?;
        let chain: IdentityReport = serde_json::from_slice(&self.identity_report.read(base)?)?;
        event.source_identity = verify_identity(
            &mut event.source_asset.issuer_asserted_mint,
            &chain.source_verification,
            base,
        )?;
        event.successor_identity = verify_identity(
            &mut event.successor_asset.issuer_asserted_mint,
            &chain.successor_verification,
            base,
        )?;
        for (pointer, identity) in [
            ("/source_identity", &event.source_identity),
            ("/successor_identity", &event.successor_identity),
        ] {
            event.field_provenance.insert(
                pointer.into(),
                ScenarioFieldBinding {
                    classifications: vec![
                        ProvenanceClass::Derived,
                        ProvenanceClass::OnChainVerified,
                    ],
                    provenance: identity.provenance.clone(),
                },
            );
        }
        Ok(VerifiedLifecycleEvent(event))
    }
    pub fn verify_event(
        &self,
        base: &Path,
        event: &NormalizedLifecycleEvent,
    ) -> Result<VerifiedLifecycleEvent> {
        let actual = self.normalize(base)?;
        ensure!(
            actual.event() == event,
            "normalized event differs from captured source/identity regeneration"
        );
        Ok(actual)
    }
    pub fn generate(
        &self,
        base: &Path,
        event: &NormalizedLifecycleEvent,
    ) -> Result<(LifecycleScenario, ScenarioBinding)> {
        self.verify_event(base, event)?.generate(
            &self.source(base)?,
            &self.config(base)?,
            &self.config,
        )
    }
    pub fn verify_scenario(
        &self,
        base: &Path,
        event: &NormalizedLifecycleEvent,
        scenario: &LifecycleScenario,
        binding: &ScenarioBinding,
    ) -> Result<()> {
        let (expected, proof) = self.generate(base, event)?;
        ensure!(
            &expected == scenario && &proof == binding,
            "scenario or provenance differs from deterministic regeneration"
        );
        Ok(())
    }
    pub fn preflight(
        &self,
        base: &Path,
        event: &NormalizedLifecycleEvent,
        scenario: &LifecycleScenario,
        binding: &ScenarioBinding,
    ) -> Result<(NoticePreflightReport, LifecycleImpactReport)> {
        self.verify_scenario(base, event, scenario, binding)?;
        let policy: LifecycleReadinessPolicy =
            serde_json::from_slice(&self.readiness_policy.read(base)?)?;
        let policy_base = base
            .join(&self.readiness_policy.file)
            .parent()
            .context("policy directory missing")?
            .to_path_buf();
        let manifest: ReadinessEvidenceManifest =
            serde_json::from_slice(&policy.evidence_manifest.read(&policy_base)?)?;
        let manifest_base = policy_base
            .join(&policy.evidence_manifest.file)
            .parent()
            .context("manifest directory missing")?
            .to_path_buf();
        let original = LifecycleScenario::load(&manifest_base.join(&manifest.scenario.file))?;
        check_compatibility(&original, scenario, event)?;
        let config = self.config(base)?;
        ensure!(
            config.evaluate_at == config.effective_at,
            "frozen assurance applies only at the original demo evaluation boundary"
        );
        let verified = manifest.verify(
            &manifest_base,
            &manifest_base.join(&manifest.snapshot.file),
            &manifest_base.join(&manifest.scenario.file),
            &manifest_base.join(&manifest.direct_resolution.file),
            &manifest_base.join(&manifest.position_resolution.file),
            &manifest_base.join(&manifest.coverage.file),
        )?;
        let snapshot = LifecycleSnapshot::load(&manifest_base.join(&manifest.snapshot.file))?;
        let impact = LifecycleConsequenceEvaluator::evaluate(
            &snapshot,
            scenario,
            config.effective_at - chrono::Duration::nanoseconds(1),
            config.evaluate_at,
        )?;
        let readiness = readiness::evaluate(&policy, &verified)?;
        // Reuse validated exact path matrices, without replay, promotion or rewriting
        // their original scenario/captured-state/assumed signer contexts.
        let direct: crate::lifecycle::resolution::LifecycleResolution =
            serde_json::from_slice(&manifest.direct_resolution.read(&manifest_base)?)?;
        let entity = impact
            .entities
            .iter()
            .find(|e| e.entity_id == direct.entity_id)
            .context("notice impact does not contain measured direct entity")?;
        ensure!(
            entity.asset_mint == direct.asset_mint
                && entity.token_account == direct.token_account
                && entity.entity_type == direct.entity_type
                && entity.verified_role == direct.verified_role
                && entity.balance.raw == direct.observed_public_balance_raw
                && entity.post_lifecycle_status == direct.lifecycle_status
                && entity.impact_classification == direct.impact_classification
                && direct.policy_evaluated_at == config.evaluate_at,
            "generated impact is incompatible with frozen direct path resolution"
        );
        let position: crate::path::position::WithdrawalReport =
            serde_json::from_slice(&manifest.position_resolution.read(&manifest_base)?)?;
        let resolution=NoticeResolutionReuse{original_scenario_sha256:manifest.scenario.sha256.clone(),generated_scenario_sha256:scenario.sha256()?,direct_resolution:manifest.direct_resolution.clone(),position_resolution:manifest.position_resolution.clone(),direct_entity_id:direct.entity_id,direct_paths:serde_json::to_value(direct.paths)?,position_entity_id:position.position.position_id.clone(),position_paths:serde_json::to_value(position.paths)?,context_rule:"Exact frozen path matrices reused after checked asset/status/boundary/deadline semantic equivalence. Original proof contexts/hashes remain original; verified successor identity adds no execution assurance. Banks and signer assumptions remain separate.".into()};
        let report=NoticePreflightReport{schema_version:1,workflow_sha256:digest(self)?,source_document:self.source.clone(),event_sha256:digest(event)?,scenario_binding:binding.clone(),impact_sha256:sha256(impact.to_json()?.as_bytes()),generated_scenario:scenario.clone(),semantic_compatibility:SemanticCompatibility{original_scenario:manifest.scenario,generated_scenario_sha256:scenario.sha256()?,economic_policy_equivalent:true,additive_successor_identity:true,proof_contexts_rewritten:false,readiness_policy_rewritten:false},resolution,readiness_sha256:digest(&readiness)?,readiness,limitations:vec!["Source publication is an issuer assertion. Observed initialized mint identity comes from independent captured RPC bytes; neither proves execution, legal entitlement or issuer affiliation.".into(),"No new HTTP/RPC, VM execution, notice refresh or mainnet action. Original captured proof banks and local assumed signer privileges remain unchanged.".into(),"The generated scenario drives existing offline impact. Readiness consumes the unchanged original policy and verified original measurements only after exact economic-policy compatibility; successor identity never provides conversion proof.".into(),"Historical HTTP status/MIME headers were not retained. Null metadata stays Unknown. Deadline uses explicit demo interpretation of the start of the stated minute; evaluation boundary is DemoConfigured.".into()]};
        Ok((report, impact))
    }
}
pub(super) fn verify_identity(
    assertion: &mut Field<String>,
    mint: &MintVerification,
    base: &Path,
) -> Result<AssetIdentity> {
    ensure!(
        assertion.value == mint.mint,
        "issuer mint does not match independent chain verification"
    );
    let raw: Value = serde_json::from_slice(&mint.evidence.artifact.read(base)?)?;
    let account = raw
        .pointer(&mint.evidence.pointer)
        .context("missing mint evidence pointer")?;
    let parts: Vec<_> = mint.evidence.pointer.split('/').collect();
    ensure!(
        parts.len() == 7
            && parts[1] == "records"
            && parts[3] == "response"
            && parts[4] == "result"
            && parts[5] == "value",
        "unsupported mint RPC pointer"
    );
    let record = &raw["records"][parts[2].parse::<usize>()?];
    let address_index = parts.last().unwrap().parse::<usize>()?;
    ensure!(
        record["method"] == "getMultipleAccounts"
            && record["params"][0][address_index] == assertion.value
            && record["response"]["result"]["context"]["slot"].as_u64() == Some(mint.slot),
        "mint evidence address/bank binding mismatch"
    );
    let bytes = crate::lifecycle::decode::raw_account_bytes(account)?;
    let decoded = crate::lifecycle::decode::decode_mint(account)?;
    ensure!(
        sha256(&bytes) == mint.raw_data_sha256
            && account["owner"] == mint.runtime_owner
            && decoded == mint.configuration
            && decoded.is_initialized
            && !account["executable"].as_bool().unwrap_or(true),
        "independent mint account verification mismatch"
    );
    let loc = SourceLocation {
        source_digest: mint.evidence.artifact.sha256.clone(),
        pointer: mint.evidence.pointer.clone(),
        byte_range: None,
        exact_text: None,
    };
    assertion
        .classifications
        .push(ProvenanceClass::OnChainVerified);
    assertion.provenance.push(loc.clone());
    let metadata = decoded
        .extensions
        .iter()
        .find(|e| e.extension_type == "TokenMetadata");
    Ok(AssetIdentity {
        asserted_mint: assertion.value.clone(),
        observed_mint: Some(mint.mint.clone()),
        status: IdentityStatus::Verified,
        slot: Some(mint.slot),
        observed_name: metadata.and_then(|e| e.config["name"].as_str().map(str::to_owned)),
        observed_symbol: metadata.and_then(|e| e.config["symbol"].as_str().map(str::to_owned)),
        provenance: vec![loc],
    })
}
pub(super) fn check_compatibility(
    original: &LifecycleScenario,
    generated: &LifecycleScenario,
    event: &NormalizedLifecycleEvent,
) -> Result<()> {
    ensure!(original.policy.asset_mint==generated.policy.asset_mint && original.policy.effective_at==generated.policy.effective_at && original.policy.before==generated.policy.before && original.policy.after==generated.policy.after && original.policy.deadline==generated.policy.deadline,"generated economic policy differs from frozen proof scenario; new proof/policy binding required");
    ensure!(
        original.policy.successor.is_none()
            && generated
                .policy
                .successor
                .as_ref()
                .is_some_and(|s| s.mint == event.successor_asset.issuer_asserted_mint.value)
            && event.successor_identity.status == IdentityStatus::Verified
            && event.official_execution_status.value == PathStatus::NotTested
            && event.official_mechanism.value == MechanismType::Unknown
            && event.conversion_ratio.value.is_none(),
        "only independently verified identity addition is compatible; no execution escalation"
    );
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCompatibility {
    pub original_scenario: ArtifactRef,
    pub generated_scenario_sha256: String,
    pub economic_policy_equivalent: bool,
    pub additive_successor_identity: bool,
    pub proof_contexts_rewritten: bool,
    pub readiness_policy_rewritten: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeResolutionReuse {
    pub original_scenario_sha256: String,
    pub generated_scenario_sha256: String,
    pub direct_resolution: ArtifactRef,
    pub position_resolution: ArtifactRef,
    pub direct_entity_id: String,
    pub direct_paths: Value,
    pub position_entity_id: String,
    pub position_paths: Value,
    pub context_rule: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticePreflightReport {
    pub schema_version: u32,
    pub workflow_sha256: String,
    pub source_document: ArtifactRef,
    pub event_sha256: String,
    pub scenario_binding: ScenarioBinding,
    pub impact_sha256: String,
    pub generated_scenario: LifecycleScenario,
    pub semantic_compatibility: SemanticCompatibility,
    pub resolution: NoticeResolutionReuse,
    pub readiness_sha256: String,
    pub readiness: LifecycleReadinessReport,
    pub limitations: Vec<String>,
}
impl NoticePreflightReport {
    pub fn to_json(&self) -> Result<String> {
        canonical(self)
    }
    pub fn render_text(&self) -> String {
        format!("NOTICE → EVENT → SCENARIO → PRE-FLIGHT\nGenerated scenario: {}\nEconomic policy compatibility: verified; original proof contexts retained\nOfficialTransition: NotTested\n{:?}: {:?}\nNo new execution or capture.\n",self.generated_scenario.id,self.readiness.evaluated_scope,self.readiness.overall_status)
    }
}
