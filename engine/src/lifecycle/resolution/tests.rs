//! Pure resolver tests use controlled status facts; production proof replay is separate.
use super::*;
use crate::{
    lifecycle::consequence::LifecycleImpactReport,
    lifecycle::frozen::selection::pipeline::{ExecutionEvidence, ExecutionIndex},
};
use std::sync::OnceLock;

struct Corpus {
    impact: LifecycleImpact,
    discovery: DiscoveryManifest,
    executions: Vec<VerifiedExecution>,
    original: Vec<ExecutionEvidence>,
}
fn corpus() -> &'static Corpus {
    static C: OnceLock<Corpus> = OnceLock::new();
    C.get_or_init(|| {
        let root = crate::lifecycle::artifact::reference_root();
        let report: LifecycleImpactReport =
            load(&root.join("reports/spacex-transition-impact.json")).unwrap();
        let index: ExecutionIndex =
            load(&root.join("reports/phase7-evidence/execution-index.json")).unwrap();
        let original: Vec<ExecutionEvidence> = index
            .results
            .iter()
            .map(|r| load(&root.join("reports/phase7-evidence").join(&r.result_file)).unwrap())
            .filter(|e: &ExecutionEvidence| e.entity_id.contains("741ZXYKz"))
            .collect();
        let executions = original
            .iter()
            .map(|e| {
                phase7::project(
                    e,
                    ArtifactRef {
                        file: format!("{}.json", e.case_id),
                        sha256: digest(e).unwrap(),
                    },
                )
                .unwrap()
            })
            .collect();
        let impact = report
            .entities
            .into_iter()
            .find(|i| i.entity_id == original[0].entity_id)
            .unwrap();
        Corpus {
            impact,
            discovery: DiscoveryManifest::load(
                &root.join("probes/spacex-lifecycle-path-discovery.json"),
            )
            .unwrap(),
            executions,
            original,
        }
    })
}
fn rows(e: &[VerifiedExecution]) -> Vec<PathResolution> {
    let c = corpus();
    LifecyclePathResolver::resolve(&c.impact, &c.discovery, e).unwrap()
}
fn row(rows: &[PathResolution], path: ExitPathType) -> &PathResolution {
    rows.iter().find(|r| r.path_type == path).unwrap()
}
#[test]
fn transfer_cannot_prove_official_transition() {
    let c = corpus();
    let e: Vec<_> = c
        .executions
        .iter()
        .filter(|e| e.0.path_type == ExitPathType::Transfer)
        .cloned()
        .collect();
    let r = rows(&e);
    assert_eq!(row(&r, ExitPathType::Transfer).status, PathStatus::Proven);
    assert_eq!(
        row(&r, ExitPathType::OfficialTransition).status,
        PathStatus::NotTested
    );
    assert!(row(&r, ExitPathType::OfficialTransition)
        .contexts
        .iter()
        .all(|c| c.attempts.is_empty()));
}
#[test]
fn market_exit_cannot_prove_redemption() {
    let c = corpus();
    let e: Vec<_> = c
        .executions
        .iter()
        .filter(|e| e.0.path_type == ExitPathType::SecondaryMarketExit)
        .cloned()
        .collect();
    let r = rows(&e);
    assert_eq!(
        row(&r, ExitPathType::SecondaryMarketExit).status,
        PathStatus::Proven
    );
    assert_eq!(
        row(&r, ExitPathType::Redemption).status,
        PathStatus::Unsupported
    );
    assert!(row(&r, ExitPathType::Redemption)
        .contexts
        .iter()
        .all(|c| c.attempts.is_empty()));
}
#[test]
fn unsupported_is_not_failed_or_nonexistent() {
    let r = rows(&[]);
    let redemption = row(&r, ExitPathType::Redemption);
    assert_eq!(redemption.status, PathStatus::Unsupported);
    assert!(redemption
        .limitations
        .iter()
        .any(|l| l.contains("never non-existence")));
    assert_eq!(
        discovery_status(MechanismBoundary::NoSupportedAdapter),
        PathStatus::Unsupported
    );
}
#[test]
fn not_tested_never_becomes_proven() {
    let r = rows(&[]);
    assert_eq!(
        row(&r, ExitPathType::OfficialTransition).status,
        PathStatus::NotTested
    );
    assert_eq!(
        row(&r, ExitPathType::Transfer).status,
        PathStatus::NotTested
    );
    assert_eq!(
        discovery_status(MechanismBoundary::OnchainCandidateUnverified),
        PathStatus::NotTested
    );
}
#[test]
fn direct_holder_withdrawal_is_not_applicable() {
    let c = corpus();
    let r = rows(&c.executions);
    assert_eq!(
        row(&r, ExitPathType::Withdrawal).status,
        PathStatus::NotApplicable
    );
    let mut contrast = c.impact.clone();
    contrast.entity_type = EntityType::ProgramOwnedAuthority;
    contrast.verified_role = Some("VerifiedProtocolReserve".into());
    let r = LifecyclePathResolver::resolve(&contrast, &c.discovery, &[]).unwrap();
    assert_eq!(
        row(&r, ExitPathType::Withdrawal).status,
        PathStatus::NotTested
    );
}
#[test]
fn entity_proof_does_not_inherit() {
    let c = corpus();
    let mut peer = c.impact.clone();
    peer.entity_id = "different-observed-entity".into();
    let r = LifecyclePathResolver::resolve(&peer, &c.discovery, &c.executions).unwrap();
    assert_eq!(
        row(&r, ExitPathType::Transfer).status,
        PathStatus::NotTested
    );
    assert_eq!(
        row(&r, ExitPathType::SecondaryMarketExit).status,
        PathStatus::NotTested
    );
    assert!(r
        .iter()
        .flat_map(|r| &r.contexts)
        .all(|c| c.attempts.is_empty()));
}
#[test]
fn venue_proof_does_not_inherit() {
    let c = corpus();
    let mut d = c.discovery.clone();
    d.paths
        .iter_mut()
        .find(|p| p.path_type == ExitPathType::SecondaryMarketExit)
        .unwrap()
        .requested_contexts
        .push("uncaptured-alternative-venue".into());
    let r = LifecyclePathResolver::resolve(&c.impact, &d, &c.executions).unwrap();
    let market = row(&r, ExitPathType::SecondaryMarketExit);
    assert_eq!(market.status, PathStatus::Proven);
    let alternative = market
        .contexts
        .iter()
        .find(|c| c.context_id == "uncaptured-alternative-venue")
        .unwrap();
    assert_eq!(alternative.status, PathStatus::NotTested);
    assert!(alternative.attempts.is_empty());
}
#[test]
fn local_assumed_signer_never_becomes_possession() {
    let c = corpus();
    for e in &c.executions {
        assert!(!e.0.signer.signer_possession_known);
        assert!(e.0.signer.signer_assumed_locally);
        assert_eq!(e.0.signer.wording, "original owner locally assumed to sign");
    }
}
#[test]
fn exact_scope_fees_banks_and_control_points_are_preserved() {
    let c = corpus();
    let r = rows(&c.executions);
    let market = row(&r, ExitPathType::SecondaryMarketExit);
    assert_eq!(market.contexts.len(), 1);
    assert_eq!(
        market.contexts[0].context_id,
        "22PthLk8TYnurtbWKRyECFd99cHHbfsbPNHfeMetzfZg"
    );
    let a = market.contexts[0]
        .attempts
        .iter()
        .find(|a| a.exact_input_raw == "17621")
        .unwrap();
    assert_eq!(a.actual_output_raw.as_deref(), Some("10567"));
    assert_eq!(a.token_transfer_withheld_raw.as_deref(), Some("89"));
    assert_eq!(a.dlmm_fee_raw.as_deref(), Some("351"));
    assert_eq!(a.dlmm_protocol_fee_raw.as_deref(), Some("35"));
    assert_eq!(a.clock.as_ref().unwrap().slot, 448018537);
    let transfer = row(&r, ExitPathType::Transfer);
    let a = transfer.contexts[0]
        .attempts
        .iter()
        .find(|a| a.exact_input_raw == "17621")
        .unwrap();
    assert_eq!(a.actual_output_raw.as_deref(), Some("17532"));
    assert_eq!(a.clock.as_ref().unwrap().slot, 448018365);
    for path in [market, transfer] {
        assert_eq!(path.contexts[0].attempts.len(), 5);
        let control = path.contexts[0]
            .attempts
            .iter()
            .find(|a| a.invalid_control)
            .unwrap();
        assert_eq!(control.status, PathStatus::Indeterminate);
        assert!(!control.execution_attempted);
        assert_eq!(control.exact_input_raw, "17622");
    }
}
#[test]
fn failed_evidence_never_becomes_proven() {
    let c = corpus();
    let mut facts = c.executions.clone();
    for e in &mut facts {
        e.0.status = PathStatus::Failed;
        e.0.execution_attempted = true;
        e.0.rollback_verified = Some(true);
    }
    let r = rows(&facts);
    assert_eq!(row(&r, ExitPathType::Transfer).status, PathStatus::Failed);
    assert_eq!(
        row(&r, ExitPathType::SecondaryMarketExit).status,
        PathStatus::Failed
    );
}
#[test]
fn indeterminate_evidence_never_becomes_proven() {
    let c = corpus();
    let mut facts = c.executions.clone();
    for e in &mut facts {
        e.0.status = PathStatus::Indeterminate;
    }
    let r = rows(&facts);
    assert_eq!(
        row(&r, ExitPathType::Transfer).status,
        PathStatus::Indeterminate
    );
    assert_eq!(
        row(&r, ExitPathType::SecondaryMarketExit).status,
        PathStatus::Indeterminate
    );
}
#[test]
fn evidence_and_discovery_input_order_do_not_change_resolution() {
    let c = corpus();
    let r = rows(&c.executions);
    let mut e = c.executions.clone();
    e.reverse();
    let mut d = c.discovery.clone();
    d.sources.reverse();
    d.paths.reverse();
    for p in &mut d.paths {
        p.facts.reverse();
        p.limitations.reverse();
        p.evidence_ids.reverse();
        p.requested_contexts.reverse();
    }
    let reordered = LifecyclePathResolver::resolve(&c.impact, &d, &e).unwrap();
    assert_eq!(canonical(&r).unwrap(), canonical(&reordered).unwrap());
}
#[test]
fn matrix_rows_roundtrip_with_strict_status_parsing() {
    let r = rows(&corpus().executions);
    let bytes = canonical(&r).unwrap();
    let decoded: Vec<PathResolution> = serde_json::from_str(&bytes).unwrap();
    assert_eq!(canonical(&decoded).unwrap(), bytes);
    for vague in [
        "Safe",
        "Unsafe",
        "ExitPossible",
        "ExitImpossible",
        "Healthy",
        "Risky",
    ] {
        assert!(serde_json::from_str::<PathStatus>(&format!("\"{vague}\"")).is_err());
    }
    assert!(serde_json::from_str::<Vec<PathResolution>>(&bytes.replacen(
        "\"status\":",
        "\"invented\": true, \"status\":",
        1
    ))
    .is_err());
}
#[test]
fn generic_logic_has_no_asset_issuer_or_target_literals() {
    for source in [include_str!("mod.rs"), include_str!("phase7.rs")] {
        for text in ["SPACEX", "PreStocks", "PreANxu", "741ZXYK", "Xs3oZwb"] {
            assert!(
                !source.contains(text),
                "asset-specific literal leaked: {text}"
            );
        }
    }
    let c = corpus();
    let mut generic = c.impact.clone();
    generic.asset_mint = "another-asset".into();
    generic.entity_id = "generic-token-entity".into();
    let mut discovery = c.discovery.clone();
    discovery.asset_mint = generic.asset_mint.clone();
    discovery
        .sources
        .iter_mut()
        .for_each(|s| s.description = "Controlled generic source".into());
    let r = LifecyclePathResolver::resolve(&generic, &discovery, &[]).unwrap();
    assert_eq!(r.len(), 5);
    assert!(r.iter().all(|r| r.entity_id == generic.entity_id));
}
#[test]
fn projection_rejects_claimed_success_without_vm_or_reconciliation() {
    let c = corpus();
    let original = c
        .original
        .iter()
        .find(|e| e.status == crate::lifecycle::frozen::coverage::CaseStatus::Succeeded)
        .unwrap();
    for variant in 0..4 {
        let mut e = original.clone();
        match variant {
            0 => e.execution = None,
            1 => e.execution.as_mut().unwrap().success = false,
            2 => e.deltas.as_mut().unwrap().reconciled = false,
            _ => e.invalid_control = true,
        }
        assert!(phase7::project(
            &e,
            ArtifactRef {
                file: "controlled.json".into(),
                sha256: "controlled".into()
            }
        )
        .is_err());
    }
}
#[test]
fn malformed_discovery_and_unbound_claims_fail_closed() {
    let c = corpus();
    let mut d = c.discovery.clone();
    d.schema_version = 99;
    assert!(d.validate().is_err());
    d = c.discovery.clone();
    d.paths[0].evidence_ids.push("nonexistent-source".into());
    assert!(d.validate().is_err());
    d = c.discovery.clone();
    d.paths.push(d.paths[0].clone());
    assert!(d.validate().is_err());
    d = c.discovery.clone();
    d.sources[0].kind = EvidenceKind::LocalExecution;
    assert!(d.validate().is_err());
}
