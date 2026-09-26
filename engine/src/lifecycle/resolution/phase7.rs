//! Adapter for digest-bound Phase 7 artifacts; replay only the requested entity.
use super::*;
use crate::{
    lifecycle::frozen::coverage::{CaseStatus, CoverageReport},
    lifecycle::frozen::selection::{
        discovery::VenueInventory,
        pipeline::{
            self, CaptureManifest, ExecutionEvidence, ExecutionIndex, LifecycleCoverageDeltaReport,
        },
        ExpansionPlan,
    },
    lifecycle::{consequence::LifecycleImpactReport, policy::LifecycleScenario, LifecycleSnapshot},
    path::CapturedExecutionFixture,
};
use anyhow::Context;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionBundle {
    pub schema_version: u32,
    pub baseline: ArtifactRef,
    pub impact: ArtifactRef,
    pub expansion_plan: ArtifactRef,
    pub inventory: ArtifactRef,
    pub captures: ArtifactRef,
    pub execution_index: ArtifactRef,
    pub coverage: ArtifactRef,
}
fn read<T: serde::de::DeserializeOwned>(r: &ArtifactRef, base: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&r.read(base)?)?)
}
pub(super) fn project(e: &ExecutionEvidence, reference: ArtifactRef) -> Result<VerifiedExecution> {
    let status = match e.status {
        CaseStatus::Succeeded => {
            let d = e.deltas.as_ref().context("success lacks deltas")?;
            ensure!(
                e.execution.as_ref().is_some_and(|x| x.success)
                    && d.reconciled
                    && d.input_debited_raw == e.input_raw
                    && !e.invalid_control,
                "unreconciled/control success"
            );
            PathStatus::Proven
        }
        CaseStatus::Failed => {
            ensure!(
                e.execution.as_ref().is_some_and(|x| !x.success)
                    && e.rollback_verified == Some(true),
                "failed execution lacks rollback proof"
            );
            PathStatus::Failed
        }
        CaseStatus::Indeterminate => PathStatus::Indeterminate,
        CaseStatus::Unsupported => PathStatus::Unsupported,
        CaseStatus::Untested => PathStatus::NotTested,
    };
    let d = e.deltas.as_ref();
    let output = d.and_then(|d| d.token_accounts.get(1));
    let fees = d.and_then(|d| d.fees.as_ref());
    let withheld = if e.path_type == ExitPathType::Transfer {
        output.map(|t| t.withheld_fee_change_raw.clone())
    } else {
        fees.map(|f| f.token_2022_transfer_fee_raw.clone())
    };
    Ok(VerifiedExecution(PathAttempt {
        case_id: e.case_id.clone(),
        entity_id: e.entity_id.clone(),
        path_type: e.path_type,
        status,
        context_id: e.context_id.clone(),
        exact_input_raw: e.input_raw.clone(),
        invalid_control: e.invalid_control,
        source_before_raw: e.source_before_raw.clone(),
        captured_state_sha256: e.captured_state_sha256.clone(),
        fixture_sha256: e.fixture_sha256.clone(),
        capture_context: e.capture_context,
        clock: e.vm_clock,
        output_mint: output.map(|t| t.mint.clone()),
        destination: output.map(|t| t.address.clone()),
        actual_output_raw: d.map(|d| d.output_received_raw.clone()),
        token_transfer_withheld_raw: withheld,
        dlmm_fee_raw: fees.map(|f| f.dlmm_swap_fee_raw.clone()),
        dlmm_protocol_fee_raw: fees.map(|f| f.dlmm_protocol_fee_raw.clone()),
        signer: SignerAssumption {
            authority: e.authority.pubkey.clone(),
            signer_possession_known: e.authority.signer_possession_known,
            signer_assumed_locally: e.authority.signer_assumed_locally,
            wording: e.authority.wording.clone(),
        },
        execution_attempted: e.execution.is_some(),
        error: e
            .blocker
            .clone()
            .or_else(|| e.execution.as_ref().and_then(|x| x.error.clone())),
        rollback_verified: e.rollback_verified,
        execution_assumptions: e.assumptions.clone(),
        evidence: reference,
    }))
}
pub fn resolve(
    bundle_path: &Path,
    s: &LifecycleSnapshot,
    scenario: &LifecycleScenario,
    entity: &str,
    coverage_path: &Path,
    discovery_path: &Path,
) -> Result<LifecycleResolution> {
    let bundle: ResolutionBundle = load(bundle_path)?;
    ensure!(
        bundle.schema_version == 1,
        "unsupported resolution bundle schema"
    );
    let base = bundle_path.parent().unwrap_or_else(|| Path::new("."));
    let b: CoverageReport = read(&bundle.baseline, base)?;
    let impact: LifecycleImpactReport = read(&bundle.impact, base)?;
    let p: ExpansionPlan = read(&bundle.expansion_plan, base)?;
    let inventory: VenueInventory = read(&bundle.inventory, base)?;
    let m: CaptureManifest = read(&bundle.captures, base)?;
    let index: ExecutionIndex = read(&bundle.execution_index, base)?;
    let delta: LifecycleCoverageDeltaReport = read(&bundle.coverage, base)?;
    ensure!(
        crate::lifecycle::exposure::sha256(&crate::lifecycle::artifact::read(coverage_path)?)
            == bundle.coverage.sha256,
        "supplied coverage differs from bundle"
    );
    ensure!(
        scenario.sha256()? == impact.scenario_sha256
            && *scenario == impact.scenario
            && digest(s)? == p.snapshot_sha256
            && digest(&impact)? == p.impact_sha256,
        "resolution population/policy fingerprint mismatch"
    );
    impact.validate(s)?;
    p.validate(s, &b, &inventory)?;
    ensure!(
        delta.schema_version == 2
            && m.schema_version == 1
            && index.schema_version == 1
            && delta.baseline_sha256 == digest(&b)?
            && delta.plan_sha256 == digest(&p)?
            && m.plan_sha256 == delta.plan_sha256
            && index.plan_sha256 == delta.plan_sha256
            && delta.capture_manifest_sha256 == digest(&m)?
            && index.capture_manifest_sha256 == delta.capture_manifest_sha256
            && delta.execution_index_sha256 == digest(&index)?
            && delta.evidence == index.results,
        "Phase 7 report/index/manifest bindings differ"
    );
    let id = if entity.starts_with("solana-token-account:") {
        entity.to_string()
    } else {
        format!("solana-token-account:{entity}")
    };
    let target = impact
        .entities
        .iter()
        .find(|e| e.entity_id == id)
        .context("entity absent from observed population")?;
    let source = s
        .entities
        .iter()
        .find(|e| e.id == id)
        .context("source absent from snapshot")?;
    let discovery = DiscoveryManifest::load(discovery_path)?;
    ensure!(
        discovery.snapshot_sha256 == p.snapshot_sha256
            && discovery.scenario_sha256 == impact.scenario_sha256,
        "discovery population/policy fingerprint mismatch"
    );
    let capture_dir = base
        .join(&bundle.captures.file)
        .parent()
        .context("capture directory missing")?
        .to_path_buf();
    let result_dir = base
        .join(&bundle.execution_index.file)
        .parent()
        .context("result directory missing")?
        .to_path_buf();
    let mut verified = Vec::new();
    for g in p.selected.iter().filter(|g| g.candidate.entity_id == id) {
        let binding = m
            .bindings
            .iter()
            .find(|x| x.group_id == g.candidate.id)
            .context("selected entity capture absent")?;
        ensure!(
            binding.fixture_file == g.fixture_reference
                && binding.fixture_sha256.is_some() != binding.error.is_some(),
            "capture binding differs from plan"
        );
        let fixture: Option<CapturedExecutionFixture> = binding
            .fixture_sha256
            .as_ref()
            .map(|h| {
                read(
                    &ArtifactRef {
                        file: binding.fixture_file.clone(),
                        sha256: h.clone(),
                    },
                    &capture_dir,
                )
            })
            .transpose()?;
        let world = crate::lifecycle::frozen::selection::discovery::execution_world(
            s,
            &inventory,
            Some(&g.candidate.context_id),
        )?;
        for a in &g.amount_matrix {
            let case = format!("group-{}-raw-{}", g.selection_order, a.raw);
            let refs: Vec<_> = index
                .results
                .iter()
                .filter(|r| r.case_id == case && r.group_id == g.candidate.id)
                .collect();
            ensure!(
                refs.len() == 1,
                "missing/duplicate selected execution reference"
            );
            let r = refs[0];
            let artifact = ArtifactRef {
                file: r.result_file.clone(),
                sha256: r.result_sha256.clone(),
            };
            let e: ExecutionEvidence = read(&artifact, &result_dir)?;
            ensure!(
                e.fixture_sha256 == r.fixture_sha256
                    && e.status == r.status
                    && e == pipeline::measure(
                        &world,
                        &p,
                        g,
                        a,
                        binding,
                        fixture.as_ref(),
                        &inventory
                    )?,
                "evidence disagrees with fresh exact entity/path/context replay"
            );
            let relative = crate::lifecycle::artifact::display_path(
                &Path::new(&bundle.execution_index.file)
                    .parent()
                    .unwrap()
                    .join(&r.result_file),
            );
            verified.push(project(
                &e,
                ArtifactRef {
                    file: relative,
                    sha256: r.result_sha256.clone(),
                },
            )?);
        }
    }
    let paths = LifecyclePathResolver::resolve(target, &discovery, &verified)?;
    let successful: Vec<_> = paths
        .iter()
        .filter(|r| r.status == PathStatus::Proven)
        .map(|r| path_name(r.path_type))
        .collect();
    let conclusion=format!("This entity has {} independently replayed bounded successful paths: {}. OfficialTransition and Redemption are not proven unless their own rows contain direct proof. Captured mobility does not prove completion of the issuer-defined lifecycle change.",successful.len(),successful.join(", "));
    Ok(LifecycleResolution {
        schema_version: 1,
        resolver_version: "lifecycle-path-v1".into(),
        entity_id: id,
        token_account: target.token_account.clone(),
        owner_authority: source.state.owner.clone(),
        asset_mint: target.asset_mint.clone(),
        entity_type: target.entity_type.clone(),
        verified_role: target.verified_role.clone(),
        observed_onchain_evidence: target.onchain_evidence.clone(),
        policy_evidence: target.lifecycle_evidence.clone(),
        observed_public_balance_raw: target.balance.raw.clone(),
        lifecycle_status: target.post_lifecycle_status,
        impact_classification: target.impact_classification,
        policy_evaluated_at: impact.after.evaluated_at,
        snapshot_sha256: p.snapshot_sha256,
        scenario_sha256: impact.scenario_sha256,
        coverage_sha256: bundle.coverage.sha256.clone(),
        discovery_sha256: digest(&discovery)?,
        evidence_bundle_sha256: digest(&bundle)?,
        baseline_execution_status: target.execution_status,
        paths,
        conclusion,
        limitations: vec![
            "Observed population, external policy, current captured local execution and research inferences are separate evidence layers.".into(),
            "Only this requested entity is replayed; no new holder sampling, venue capture or amount campaign occurs.".into(),
            "Policy time does not set execution Clock. Each exact attempt retains its own bank/state identity, original-owner signer assumption and runtime limitations.".into(),
            "No independently established authorization, inclusion, future liquidity, issuer entitlement or legal conclusion follows.".into(),
        ],
    })
}
