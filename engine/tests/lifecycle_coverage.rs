//! Coverage assurance uses genuine captured execution, never quote-only evidence.
use base64::Engine as _;
use eplyx_engine::{
    change::ChangeSpec,
    lifecycle::artifact::reference_root as repo_root,
    lifecycle::frozen::coverage::{
        self, CaseStatus, CoverageCase, CoverageClassification as C, CoveragePlan, CoverageReport,
    },
    lifecycle::{
        consequence::LifecycleImpactReport,
        exposure::{self, AdapterRun},
        normalize,
        policy::LifecycleScenario,
        AssetDescriptor, LifecycleSnapshot, RpcEvidence,
    },
    path::{ExecutionProbeSpec, ExitPathType},
};
use serde_json::Value;
use std::sync::OnceLock;
fn world() -> &'static (LifecycleSnapshot, LifecycleImpactReport) {
    static W: OnceLock<(LifecycleSnapshot, LifecycleImpactReport)> = OnceLock::new();
    W.get_or_init(|| {
        let f: Value = serde_json::from_str(include_str!(
            "../../fixtures/lifecycle/sta/fixtures/spacex-dlmm-exit-baseline.json"
        ))
        .unwrap();
        from_baseline(f)
    })
}
fn from_baseline(f: Value) -> (LifecycleSnapshot, LifecycleImpactReport) {
    let b = &f["baseline"];
    let mut s = normalize(
        serde_json::from_value::<AssetDescriptor>(b["asset"].clone()).unwrap(),
        b["captured_at"].as_str().unwrap().into(),
        b["rpc_origin"].as_str().unwrap().into(),
        serde_json::from_value::<Vec<RpcEvidence>>(b["evidence"].clone()).unwrap(),
    )
    .unwrap();
    let graph = exposure::normalize_graph(
        &s,
        vec![serde_json::from_value::<AdapterRun>(f["adapter_run"].clone()).unwrap()],
    )
    .unwrap();
    s.schema_version = 2;
    s.exposures = Some(graph);
    let scenario: LifecycleScenario = serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/sta/scenarios/spacex-transition.json"
    ))
    .unwrap();
    let impact = ChangeSpec::lifecycle(&scenario)
        .unwrap()
        .compare_lifecycle(
            &s,
            &scenario,
            scenario.policy.effective_at - chrono::Duration::seconds(1),
            scenario.policy.effective_at,
        )
        .unwrap();
    (s, impact)
}
fn seed() -> ExecutionProbeSpec {
    let mut p: ExecutionProbeSpec = serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/sta/probes/spacex-usdc-dlmm-exit.json"
    ))
    .unwrap();
    p.snapshot_sha256 = world().1.after.snapshot_sha256.clone();
    p
}
fn plan(amounts: &[u64]) -> CoveragePlan {
    eplyx_engine::lifecycle::frozen::coverage::build_plan(
        &world().0,
        &world().1,
        &[seed()],
        amounts,
    )
    .unwrap()
}
fn evaluate(plan: &CoveragePlan) -> anyhow::Result<CoverageReport> {
    eplyx_engine::lifecycle::frozen::coverage::evaluate(
        &world().0,
        &world().1,
        plan,
        &repo_root().join("probes"),
    )
}
fn partial() -> &'static CoverageReport {
    static R: OnceLock<CoverageReport> = OnceLock::new();
    R.get_or_init(|| evaluate(&plan(&[1000, 10000, 30000])).unwrap())
}
fn target(r: &CoverageReport) -> &coverage::EntityCoverage {
    r.entities
        .iter()
        .find(|e| e.entity_id == seed().target_entity)
        .unwrap()
}
#[test]
fn real_amount_matrix_uses_maximum_not_sum_and_retains_exact_witnesses() {
    let r = partial();
    let e = target(r);
    assert_eq!(e.classification, C::PartiallyProven);
    assert_eq!(e.represented_amount_covered_raw, "30000");
    assert_eq!(e.represented_amount_without_evidence_raw, "30354");
    assert_eq!(r.portfolio.represented_amount_covered_raw, "30000");
    let mut amounts = Vec::new();
    for c in &r.cases {
        if let Some(report) = &c.execution_report {
            assert_eq!(c.status, CaseStatus::Succeeded);
            assert!(report.deltas.as_ref().unwrap().reconciled);
            assert!(!report.assumptions.is_empty());
            assert_eq!(report.vm_clock.as_ref().unwrap().slot, 447884621);
            amounts.push(c.case.input_amount_raw.clone());
        }
    }
    amounts.sort();
    assert_eq!(amounts, vec!["1000", "10000", "10000", "30000"]);
}
#[test]
fn full_amount_is_conditionally_proven_and_insufficient_balance_is_not_execution() {
    let r = evaluate(&plan(&[60354, 60355])).unwrap();
    assert_eq!(target(&r).classification, C::Proven);
    assert_eq!(target(&r).represented_amount_without_evidence_raw, "0");
    let bad = r
        .cases
        .iter()
        .find(|c| c.case.input_amount_raw == "60355")
        .unwrap();
    assert_eq!(bad.status, CaseStatus::Indeterminate);
    assert!(bad.execution_report.as_ref().unwrap().execution.is_none());
    assert!(bad.reason.contains("insufficient"));
    assert_eq!(
        serde_json::to_value(r.official_transition).unwrap(),
        "NotTested"
    );
    assert!(r
        .entities
        .iter()
        .all(|e| serde_json::to_value(e.official_transition).unwrap() == "NotTested"));
}
#[test]
fn alternate_venue_is_a_separate_untested_cohort_without_copying_evidence() {
    let mut p = plan(&[1000]);
    p.cases.push(CoverageCase {
        id: "other-venue".into(),
        target_entity: seed().target_entity,
        path_type: ExitPathType::SecondaryMarketExit,
        venue: Some("uncaptured-candidate-venue".into()),
        input_amount_raw: "1000".into(),
        execution_probe: None,
        execution_report: None,
    });
    let r = evaluate(&p).unwrap();
    assert_eq!(r.by_venue.len(), 2);
    assert_eq!(
        r.by_venue["uncaptured-candidate-venue"].represented_amount_covered_raw,
        "0"
    );
    assert_eq!(
        r.cases
            .iter()
            .find(|c| c.case.id == "other-venue")
            .unwrap()
            .status,
        CaseStatus::Untested
    );
    assert_eq!(r.portfolio.represented_amount_covered_raw, "10000");
}
#[test]
fn representative_selection_is_stable_and_never_extrapolates_to_peers_or_vaults() {
    let r = partial();
    assert_eq!(
        r.representatives,
        coverage::representatives(&world().0, &world().1).unwrap()
    );
    assert_eq!(r.portfolio.entities, world().1.entities.len());
    assert_eq!(r.portfolio.classifications[&C::PartiallyProven], 1);
    let vault = r
        .entities
        .iter()
        .find(|e| e.account_type == "ProgramOwnedAuthority")
        .unwrap();
    assert_eq!(vault.classification, C::Unsupported);
    assert_eq!(vault.represented_amount_covered_raw, "0");
    for path in ["OfficialTransition", "Redemption", "Withdrawal", "Transfer"] {
        assert_eq!(r.by_path_type[path].represented_amount_covered_raw, "0");
        assert_eq!(
            r.by_path_type[path].classifications[&C::Unsupported],
            r.entities.len()
        );
    }
    assert_eq!(
        r.portfolio.represented_amount_raw,
        world().1.summary.phase2_public_raw_exposure
    );
}
#[test]
fn empty_matrix_is_untested_and_zero_public_balance_is_never_proven() {
    let mut p = plan(&[1000]);
    p.cases.clear();
    let r = evaluate(&p).unwrap();
    assert_eq!(target(&r).classification, C::Untested);
    assert_eq!(r.portfolio.represented_amount_covered_raw, "0");
    assert!(r
        .entities
        .iter()
        .filter(|e| e.represented_balance_raw == "0")
        .all(|e| e.classification != C::Proven));
}
#[test]
fn official_only_request_is_unsupported_capability_with_transition_still_not_tested() {
    let mut p = plan(&[1000]);
    p.cases.clear();
    p.requested_paths = vec![ExitPathType::OfficialTransition];
    let r = evaluate(&p).unwrap();
    assert!(r
        .entities
        .iter()
        .all(|e| e.classification == C::Unsupported));
    assert_eq!(
        serde_json::to_value(r.official_transition).unwrap(),
        "NotTested"
    );
}
#[test]
fn actual_slippage_failure_retains_atomic_rollback_and_gives_no_amount_coverage() {
    let mut p = plan(&[1000]);
    p.cases.retain(|c| c.execution_probe.is_some());
    p.cases.truncate(1);
    let c = &mut p.cases[0];
    c.execution_probe.as_mut().unwrap().minimum_output_raw = "18446744073709551615".into();
    let r = evaluate(&p).unwrap();
    assert_eq!(r.cases[0].status, CaseStatus::Failed);
    assert!(r.cases[0]
        .execution_report
        .as_ref()
        .unwrap()
        .execution
        .is_some());
    assert_eq!(r.portfolio.represented_amount_covered_raw, "0");
    assert_eq!(target(&r).classification, C::Untested);
}
#[test]
fn case_identity_path_amount_and_population_fingerprints_are_enforced() {
    for variant in 0..8 {
        let mut p = plan(&[1000]);
        let c = p
            .cases
            .iter_mut()
            .find(|c| c.execution_probe.is_some())
            .unwrap();
        match variant {
            0 => p.snapshot_sha256 = "bad".into(),
            1 => p.impact_sha256 = "bad".into(),
            2 => c.input_amount_raw = "2".into(),
            3 => c.target_entity = "missing".into(),
            4 => c.venue = Some("wrong".into()),
            5 => c.path_type = ExitPathType::Transfer,
            6 => c.input_amount_raw = "01000".into(),
            _ => p.requested_paths.push(ExitPathType::SecondaryMarketExit),
        };
        assert!(evaluate(&p).is_err(), "variant {variant}");
    }
}
#[test]
fn missing_fixture_and_duplicate_cases_never_silently_skip() {
    let mut p = plan(&[1000]);
    p.cases
        .iter_mut()
        .find(|c| c.execution_probe.is_some())
        .unwrap()
        .execution_probe
        .as_mut()
        .unwrap()
        .fixture = "missing.json".into();
    assert!(evaluate(&p)
        .unwrap_err()
        .to_string()
        .contains("missing captured"));
    let mut p = plan(&[1000]);
    p.cases.push(p.cases[0].clone());
    assert!(evaluate(&p).is_err());
}
#[test]
fn prior_phase5_result_requires_matching_fresh_replay_and_tampering_is_rejected() {
    let dir = std::env::temp_dir().join(format!("eplyx-coverage-prior-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = plan(&[1000]);
    p.cases.retain(|c| c.execution_probe.is_some());
    p.cases.truncate(1);
    let c = &mut p.cases[0];
    c.execution_probe.as_mut().unwrap().fixture = repo_root()
        .join("probes/spacex-usdc-dlmm-fixture.json")
        .to_string_lossy()
        .into();
    c.execution_report = Some("prior.json".into());
    let mut without = p.clone();
    without.cases[0].execution_report = None;
    let original = coverage::evaluate(&world().0, &world().1, &without, &dir).unwrap();
    let mut report = original.cases[0].execution_report.clone().unwrap();
    eplyx_engine::path::save_json(&report, &dir.join("prior.json")).unwrap();
    assert!(coverage::evaluate(&world().0, &world().1, &p, &dir).is_ok());
    report.deltas.as_mut().unwrap().input_debited_raw = "1".into();
    std::fs::write(dir.join("prior.json"), report.to_json().unwrap()).unwrap();
    assert!(coverage::evaluate(&world().0, &world().1, &p, &dir).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn canonical_report_roundtrip_replay_and_tamper_detection() {
    let r = partial();
    let p = plan(&[1000, 10000, 30000]);
    let restored: CoverageReport = serde_json::from_str(&r.to_json().unwrap()).unwrap();
    assert_eq!(*r, restored);
    r.validate(&world().0, &world().1, &p, &repo_root().join("probes"))
        .unwrap();
    let mut bad = r.clone();
    bad.portfolio.represented_amount_covered_raw = "1".into();
    assert!(bad
        .validate(&world().0, &world().1, &p, &repo_root().join("probes"))
        .is_err());
}
#[test]
fn invalid_amount_matrix_and_seed_fingerprints_fail_before_plan_generation() {
    assert!(coverage::build_plan(&world().0, &world().1, &[seed()], &[0]).is_err());
    assert!(coverage::build_plan(&world().0, &world().1, &[seed()], &[]).is_err());
    let mut p = seed();
    p.scenario_sha256 = "wrong".into();
    assert!(coverage::build_plan(&world().0, &world().1, &[p], &[1000]).is_err());
}
#[test]
fn plan_json_is_strict_and_deterministic() {
    let p = plan(&[30000, 1000, 1000]);
    assert_eq!(p, plan(&[1000, 30000]));
    let mut v = serde_json::to_value(&p).unwrap();
    v["invented_assurance"] = true.into();
    assert!(serde_json::from_value::<CoveragePlan>(v).is_err());
}

#[test]
fn differing_population_bank_balance_retains_witness_but_covers_no_historical_amount() {
    // Controlled earlier observation only; the actual execution fixture is unchanged.
    let mut f: Value = serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/sta/fixtures/spacex-dlmm-exit-baseline.json"
    ))
    .unwrap();
    let address = seed()
        .target_entity
        .trim_start_matches("solana-token-account:")
        .to_string();
    let account = f["baseline"]["evidence"][2]["result"]["value"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|a| a["pubkey"] == address)
        .unwrap();
    let engine = base64::engine::general_purpose::STANDARD;
    let mut bytes = engine
        .decode(account["account"]["data"][0].as_str().unwrap())
        .unwrap();
    bytes[64..72].copy_from_slice(&60353u64.to_le_bytes());
    account["account"]["data"][0] = engine.encode(bytes).into();
    let (s, i) = from_baseline(f);
    let mut spec = seed();
    spec.snapshot_sha256 = i.after.snapshot_sha256.clone();
    let p = coverage::build_plan(&s, &i, &[spec], &[1000]).unwrap();
    let r = coverage::evaluate(&s, &i, &p, &repo_root().join("probes")).unwrap();
    assert_eq!(target(&r).classification, C::Untested);
    assert_eq!(r.portfolio.represented_amount_covered_raw, "0");
    assert!(r.cases.iter().any(|c| c.status == CaseStatus::Succeeded));
    let path = target(&r)
        .paths
        .iter()
        .find(|p| p.path_type == ExitPathType::SecondaryMarketExit)
        .unwrap();
    assert_eq!(path.largest_successful_input_raw, "10000");
    assert!(path.reason.contains("different captured balance"));
}
#[test]
fn population_before_view_can_differ_without_changing_after_view_or_execution_clock() {
    let s = &world().0;
    let scenario = &world().1.scenario;
    let i = ChangeSpec::lifecycle(scenario)
        .unwrap()
        .compare_lifecycle(
            s,
            scenario,
            scenario.policy.effective_at,
            scenario.policy.effective_at,
        )
        .unwrap();
    let p = coverage::build_plan(s, &i, &[seed()], &[1000]).unwrap();
    let r = coverage::evaluate(s, &i, &p, &repo_root().join("probes")).unwrap();
    assert_eq!(r.portfolio.represented_amount_covered_raw, "10000");
    assert_eq!(i.diff.economic_meaning_changed_entities, 0);
}
