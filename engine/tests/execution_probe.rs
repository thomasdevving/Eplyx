//! Controlled offline execution of genuine captured DLMM/token SBF and route state.
//! Minimized earlier source provenance is explicitly reconstructed, never production.
use eplyx_engine::{
    lifecycle::{
        consequence::{LifecycleExecutionStatus, LifecycleImpactClassification},
        exposure::{self, AdapterRun},
        normalize,
        policy::LifecycleScenario,
        AssetDescriptor, LifecycleSnapshot, RpcEvidence,
    },
    path::{
        self,
        meteora_dlmm::{self, DexSwapExitProbe},
        CapturedExecutionFixture, ExecutionProbe, ExecutionProbeSpec, ExitPathType,
        ExitabilityReport, ProbeExecutionStatus,
    },
};
use serde_json::Value;
use std::{path::PathBuf, sync::OnceLock};

fn baseline() -> LifecycleSnapshot {
    let f: Value = serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/main-t7/fixtures/spacex-dlmm-exit-baseline.json"
    ))
    .unwrap();
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
    s
}
fn scenario() -> LifecycleScenario {
    serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/main-t7/scenarios/spacex-transition.json"
    ))
    .unwrap()
}
fn fixture() -> CapturedExecutionFixture {
    static F: OnceLock<CapturedExecutionFixture> = OnceLock::new();
    F.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../fixtures/lifecycle/main-t7/probes/spacex-usdc-dlmm-fixture.json"
        ))
        .unwrap()
    })
    .clone()
}
fn spec(s: &LifecycleSnapshot) -> ExecutionProbeSpec {
    let mut p: ExecutionProbeSpec = serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/main-t7/probes/spacex-usdc-dlmm-exit.json"
    ))
    .unwrap();
    p.snapshot_sha256 = exposure::sha256(s.to_json().unwrap().as_bytes());
    p
}
fn execute(
    s: &LifecycleSnapshot,
    p: &ExecutionProbeSpec,
    f: &CapturedExecutionFixture,
) -> ExitabilityReport {
    let scenario = scenario();
    path::run(s, &scenario, p, f, scenario.policy.effective_at).unwrap()
}
fn successful() -> ExitabilityReport {
    static R: OnceLock<ExitabilityReport> = OnceLock::new();
    R.get_or_init(|| {
        let s = baseline();
        execute(&s, &spec(&s), &fixture())
    })
    .clone()
}
fn corrupt_account(f: &mut CapturedExecutionFixture, address: &str) {
    let index = f.evidence[3].params[0]
        .as_array()
        .unwrap()
        .iter()
        .position(|a| a == address)
        .unwrap();
    f.evidence[3].result["value"][index] = Value::Null;
}

#[test]
fn controlled_capture_runs_actual_swap2_and_both_deployed_token_programs() {
    let r = successful();
    assert_eq!(r.execution_status, ProbeExecutionStatus::Succeeded);
    assert!(r.blocker.is_none());
    let e = r.execution.as_ref().unwrap();
    assert!(e.success);
    assert_eq!(e.compute_units, 45101);
    assert!(e.logs.iter().any(|l| l.contains("Instruction: Swap2")));
    assert!(e
        .inner_instructions
        .iter()
        .any(|i| i.program == meteora_dlmm::program_ids()[1]));
    assert!(e
        .inner_instructions
        .iter()
        .any(|i| i.program == meteora_dlmm::program_ids()[2]));
    assert!(r.preconditions.iter().all(|p| p.proven));
    assert_eq!(r.execution_message.as_ref().unwrap().required_signatures, 2);
}

#[test]
fn insufficient_balance_is_a_clean_precondition_failure_without_execution() {
    let s = baseline();
    let mut p = spec(&s);
    p.input_amount_raw = "60355".into();
    let r = execute(&s, &p, &fixture());
    assert_eq!(r.execution_status, ProbeExecutionStatus::Indeterminate);
    assert!(r.execution.is_none());
    assert!(r
        .blocker
        .unwrap()
        .contains("insufficient input token balance"));
}

#[test]
fn wrong_pool_mint_and_vault_as_source_are_rejected_before_execution() {
    let s = baseline();
    for variant in 0..3 {
        let mut p = spec(&s);
        match variant {
            0 => p.output_mint = p.input_mint.clone(),
            1 => p.pool = "11111111111111111111111111111111".into(),
            _ => {
                p.target_entity = s
                    .entities
                    .iter()
                    .find(|e| {
                        e.entity_type == eplyx_engine::lifecycle::EntityType::ProgramOwnedAuthority
                    })
                    .unwrap()
                    .id
                    .clone()
            }
        }
        let r = execute(&s, &p, &fixture());
        assert_eq!(r.execution_status, ProbeExecutionStatus::Indeterminate);
        assert!(r.execution.is_none());
    }
}

#[test]
fn missing_bin_array_or_programdata_is_indeterminate_never_a_quote_fallback() {
    let s = baseline();
    let p = spec(&s);
    let plan = DexSwapExitProbe
        .build_execution(&s, &p, &fixture())
        .unwrap();
    let bin = plan
        .account_evidence
        .iter()
        .find(|e| e.address == "9PPwitu3fYPVzYby2BKFSdCF93psUCZjfdWznUX8bact")
        .unwrap()
        .address
        .clone();
    for address in [bin, "HZcJwcJ2njPDxZtpPoKnF8v2w9QAx2rS7TdJPSRkbEhu".into()] {
        let mut f = fixture();
        corrupt_account(&mut f, &address);
        let mut p = p.clone();
        p.fixture_sha256 = f.sha256().unwrap();
        let r = execute(&s, &p, &f);
        assert_eq!(r.execution_status, ProbeExecutionStatus::Indeterminate);
        assert!(r.execution.is_none());
        assert!(r
            .blocker
            .unwrap()
            .contains("missing required protocol account"));
    }
}

#[test]
fn token2022_epoch_fee_is_executed_and_matches_vault_withheld_delta() {
    let r = successful();
    let d = r.deltas.unwrap();
    let fees = d.fees.unwrap();
    assert_eq!(fees.active_epoch, 1036);
    assert_eq!(fees.token_2022_transfer_fee_raw, "50");
    assert_eq!(d.token_accounts[2].withheld_fee_change_raw, "50");
    assert_eq!(d.token_accounts[2].change_raw, "9950");
    assert_eq!(fees.dlmm_swap_fee_raw, "2");
    assert_eq!(fees.dlmm_protocol_fee_raw, "0");
    assert_eq!(fees.dlmm_liquidity_provider_fee_raw, "2");
    assert_eq!(fees.host_fee_raw, "0");
}

#[test]
fn exact_user_vault_bin_and_fee_deltas_reconcile() {
    let r = successful();
    let d = r.deltas.unwrap();
    assert!(d.reconciled);
    assert_eq!(d.input_debited_raw, "10000");
    assert_eq!(d.output_received_raw, "6065");
    assert_eq!(d.output_decimal_base_units, "0.006065");
    assert_eq!(d.token_accounts[0].before_raw, "60354");
    assert_eq!(d.token_accounts[0].after_raw, "50354");
    assert_eq!(d.token_accounts[1].before_raw, "24545");
    assert_eq!(d.token_accounts[1].after_raw, "30610");
    assert_eq!(d.token_accounts[3].change_raw, "-6065");
    let dx = d.token_accounts[2].change_raw.parse::<i128>().unwrap();
    let w = d.token_accounts[2]
        .withheld_fee_change_raw
        .parse::<i128>()
        .unwrap();
    assert_eq!(dx + w, 10000);
    assert!(d
        .reconciliation
        .iter()
        .any(|l| l.contains("X delta 9948, Y delta -6065")));
    assert!(d
        .account_data
        .iter()
        .any(|a| a.address == r.probe.pool && !a.changed_ranges.is_empty()));
}

#[test]
fn lifecycle_assessment_attaches_market_exit_without_claiming_official_transition_or_vault_exit() {
    let r = successful();
    let a = r.lifecycle_assessment;
    assert_eq!(
        a.entity_impact.impact_classification,
        LifecycleImpactClassification::RequiresTransition
    );
    assert_eq!(a.tested_path, ExitPathType::SecondaryMarketExit);
    assert_eq!(a.secondary_market_exit, ProbeExecutionStatus::Succeeded);
    assert_eq!(a.official_transition, LifecycleExecutionStatus::NotTested);
    assert!(!a.venue_vault_exitability_tested);
    assert_eq!(a.source_balance_at_phase5_raw.as_deref(), Some("60354"));
}

#[test]
fn impossible_minimum_output_executes_and_fails_atomically_with_real_error() {
    let s = baseline();
    let mut p = spec(&s);
    p.minimum_output_raw = u64::MAX.to_string();
    let r = execute(&s, &p, &fixture());
    assert_eq!(r.execution_status, ProbeExecutionStatus::Failed);
    let ex = r.execution.unwrap();
    assert!(!ex.success);
    assert!(ex.error.is_some());
    assert!(ex.logs.iter().any(|l| l.contains("Swap2")));
    let d = r.deltas.unwrap();
    assert!(d.reconciled);
    assert!(d
        .token_accounts
        .iter()
        .all(|a| a.change_raw == "0" && a.withheld_fee_change_raw == "0"));
}

#[test]
fn offline_replay_is_identical_including_logs_exact_message_and_post_state() {
    let s = baseline();
    let p = spec(&s);
    let first = execute(&s, &p, &fixture());
    let second = execute(&s, &p, &fixture());
    assert_eq!(first, second);
    assert_eq!(first.to_json().unwrap(), second.to_json().unwrap());
    assert_eq!(first.execution_status, ProbeExecutionStatus::Succeeded);
}

#[test]
fn report_roundtrip_and_reexecution_detect_tampered_deltas() {
    let s = baseline();
    let r = successful();
    let bytes = r.to_json().unwrap();
    let mut decoded: ExitabilityReport = serde_json::from_str(&bytes).unwrap();
    assert_eq!(decoded.to_json().unwrap(), bytes);
    decoded.validate(&s, &scenario(), &fixture()).unwrap();
    decoded.deltas.as_mut().unwrap().output_received_raw = "999999".into();
    assert!(decoded.validate(&s, &scenario(), &fixture()).is_err());
}

#[test]
fn frozen_hash_bindings_and_capture_contexts_are_enforced() {
    let s = baseline();
    let mut p = spec(&s);
    let f = fixture();
    p.fixture_sha256 = "bad".into();
    assert!(path::run(&s, &scenario(), &p, &f, scenario().policy.effective_at).is_err());
    let mut f = fixture();
    f.evidence[3].result["context"]["slot"] = serde_json::json!(1);
    let mut p = spec(&s);
    p.fixture_sha256 = f.sha256().unwrap();
    assert_eq!(
        execute(&s, &p, &f).execution_status,
        ProbeExecutionStatus::Indeterminate
    );
}

#[test]
fn production_probe_and_fixture_load_offline_and_report_outputs_cannot_overwrite() {
    let path = eplyx_engine::lifecycle::artifact::reference_root()
        .join("probes/spacex-usdc-dlmm-exit.json");
    let (p, f) = path::load_probe(&path).unwrap();
    assert_eq!(p.fixture_sha256, f.sha256().unwrap());
    assert!(!PathBuf::from(p.fixture).is_absolute());
    let r = successful();
    let path = std::env::temp_dir().join(format!("eplyx-probe-result-{}.json", std::process::id()));
    path::save_json(&r, &path).unwrap();
    assert!(path::save_json(&r, &path).is_err());
    std::fs::remove_file(path).unwrap();
}
