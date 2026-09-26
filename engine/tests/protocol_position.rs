//! Controlled earlier-provenance reconstruction; real position execution uses the unchanged full finalized Phase 10 batch.
use eplyx_engine::{
    lifecycle::artifact::reference_root as repo_root,
    lifecycle::frozen::selection::canonical,
    lifecycle::resolution::PathStatus,
    lifecycle::{
        exposure::{self, AdapterRun},
        normalize,
        policy::LifecycleScenario,
        AssetDescriptor, LifecycleSnapshot, RpcEvidence,
    },
    path::position::{
        meteora_dlmm as adapter, AuthorityModel, ProtocolPosition, WithdrawalProbe,
        WithdrawalReport,
    },
};
use serde_json::Value;
use std::{path::PathBuf, process::Command, sync::OnceLock};
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
fn policy() -> LifecycleScenario {
    serde_json::from_str(include_str!(
        "../../fixtures/lifecycle/main-t7/scenarios/spacex-transition.json"
    ))
    .unwrap()
}
fn fixture() -> Vec<u8> {
    std::fs::read(repo_root().join("evidence/withdrawal/phase10/execution-capture.json")).unwrap()
}
fn discovery() -> Vec<u8> {
    std::fs::read(repo_root().join("evidence/withdrawal/phase10/position-discovery.json")).unwrap()
}
fn selected() -> ProtocolPosition {
    adapter::discover(&baseline(), &policy(), &discovery(), &fixture()).unwrap()
}
fn execute(p: &ProtocolPosition, w: &WithdrawalProbe, f: &[u8]) -> WithdrawalReport {
    adapter::execute(p, w, &baseline(), &policy(), &discovery(), f).unwrap()
}
fn successful() -> &'static WithdrawalReport {
    static R: OnceLock<WithdrawalReport> = OnceLock::new();
    R.get_or_init(|| {
        let p = selected();
        execute(&p, &WithdrawalProbe::full(&p), &fixture())
    })
}
fn without(address: &str) -> Vec<u8> {
    let mut f: Value = serde_json::from_slice(&fixture()).unwrap();
    let i = f["records"][2]["request"]["params"][0]
        .as_array()
        .unwrap()
        .iter()
        .position(|v| v == address)
        .unwrap();
    f["records"][2]["response"]["result"]["value"][i] = Value::Null;
    canonical(&f).unwrap().into_bytes()
}
#[test]
fn native_withdrawal_executes_real_deployed_dlmm_and_both_token_programs() {
    let r = successful();
    assert_eq!(r.status, PathStatus::Proven, "{:?}", r.precondition_error);
    assert!(r.execution_attempted);
    let e = r.execution.as_ref().unwrap();
    assert!(e.success);
    assert!(e
        .logs
        .iter()
        .any(|l| l.contains("Instruction: RemoveLiquidityByRange2")));
    for pr in [
        "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    ] {
        assert!(e.inner_instructions.iter().any(|i| i.program == pr));
    }
    assert_eq!(r.position.principal_exposure_raw, ["21296808", "8132564"]);
    assert_eq!(r.probe.bps_to_remove, 10000);
    assert_eq!(r.scope.lower_bin_id, -102);
    assert_eq!(r.scope.upper_bin_id, -53);
}
#[test]
fn actual_user_vault_fee_and_bin_amounts_conserve_exactly() {
    let r = successful();
    assert_eq!(r.status, PathStatus::Proven);
    assert_eq!(r.token_reconciliation.len(), 2);
    for (i, t) in r.token_reconciliation.iter().enumerate() {
        let user = t.user_credit_raw.parse::<u64>().unwrap();
        let fee = t.withheld_credit_raw.parse::<u64>().unwrap();
        let reserve = t.reserve_debit_raw.parse::<u64>().unwrap();
        assert_eq!(user + fee, reserve);
        assert_eq!(t.bin_principal_decrease_raw, t.reserve_debit_raw);
        assert_eq!(t.protocol_calculated_principal_raw, t.reserve_debit_raw);
        assert_eq!(t.transfer_fee_raw, t.withheld_credit_raw);
        let bins = r
            .bin_reconciliation
            .iter()
            .map(|b| b.principal_removed_raw[i].parse::<u64>().unwrap())
            .sum::<u64>();
        assert_eq!(bins, reserve);
    }
    assert_eq!(r.token_reconciliation[0].transfer_fee_raw, "106485");
    assert_eq!(r.token_reconciliation[0].user_credit_raw, "21190323");
    assert_eq!(r.token_reconciliation[1].user_credit_raw, "8132564");
}
#[test]
fn actual_position_liquidity_is_removed_and_fees_remain_unclaimed() {
    let r = successful();
    assert_eq!(r.status, PathStatus::Proven);
    assert_eq!(r.position_retained, Some(true));
    assert_eq!(r.all_position_liquidity_removed, Some(true));
    assert_eq!(r.fees_claimed_raw, Some(["0".into(), "0".into()]));
    for b in &r.bin_reconciliation {
        let before = b.shares_before.parse::<u128>().unwrap();
        assert_eq!(b.shares_after, "0");
        assert_eq!(b.removed_shares, b.shares_before);
        assert_eq!(
            b.supply_before.parse::<u128>().unwrap() - b.supply_after.parse::<u128>().unwrap(),
            before
        );
    }
    assert_eq!(
        r.post_position_fields.as_ref().unwrap()["pending_fees_raw"],
        serde_json::to_value(&r.position.calculated_accrued_fees_raw).unwrap()
    );
    assert_eq!(r.paths[0].status, PathStatus::NotTested);
    assert_eq!(r.paths[1].status, PathStatus::Unsupported);
    assert_eq!(r.paths[2].status, PathStatus::NotApplicable);
    assert_eq!(r.paths[3].status, PathStatus::NotApplicable);
}
#[test]
fn wrong_pool_or_position_authority_cannot_execute() {
    let p = selected();
    let mut w = WithdrawalProbe::full(&p);
    w.pool = p.authority.clone();
    let r = execute(&p, &w, &fixture());
    assert_eq!(r.status, PathStatus::Indeterminate);
    assert!(!r.execution_attempted);
    w = WithdrawalProbe::full(&p);
    w.authority = p.pool.clone();
    let r = execute(&p, &w, &fixture());
    assert_eq!(r.status, PathStatus::Indeterminate);
    assert!(!r.execution_attempted);
    assert!(r.precondition_error.unwrap().contains("position authority"));
}
#[test]
fn missing_bin_and_bitmap_state_are_explicitly_indeterminate() {
    let p = selected();
    for address in [
        "2bfW1DbvLdmFfo76LeQsYM91onzLjpxS5VgLgwFZtssi",
        "4VSLTuneC2hvm82x4DH38w4HMn4mviorTrQKAJRYU6if",
    ] {
        let f = without(address);
        let mut controlled = p.clone();
        controlled.fixture_sha256 = exposure::sha256(&f);
        let r = execute(&controlled, &WithdrawalProbe::full(&controlled), &f);
        assert_eq!(r.status, PathStatus::Indeterminate);
        assert!(!r.execution_attempted);
        assert!(r
            .precondition_error
            .unwrap()
            .contains("missing required public account"));
    }
}
#[test]
fn failed_native_transaction_rolls_back_every_watched_production_account() {
    let p = selected();
    let mut w = WithdrawalProbe::full(&p);
    w.compute_unit_limit = 1;
    let r = execute(&p, &w, &fixture());
    assert_eq!(r.status, PathStatus::Failed);
    assert!(r.execution_attempted);
    assert!(!r.execution.as_ref().unwrap().success);
    assert_eq!(r.rollback_verified, Some(true));
    assert!(r.watched_state_changes.is_empty());
    assert!(r.token_reconciliation.is_empty());
    assert_eq!(r.paths[4].status, PathStatus::Failed);
}
#[test]
fn signer_assumption_never_claims_private_key_possession() {
    let p = selected();
    assert_eq!(p.authority_model, AuthorityModel::DirectSigner);
    assert!(!p.signer.signer_possession_known);
    assert!(p.signer.signer_assumed_locally);
    assert_eq!(p.authority, "55uxDcXEaUjwit2Mv3EoNrtaGqTTUaUCvXpABpkoUUoE");
    assert_ne!(p.authority, p.pool);
}
#[test]
fn complete_offline_execution_and_message_are_identical() {
    let r = successful();
    let again = execute(&r.position, &r.probe, &fixture());
    assert_eq!(*r, again);
    let serialized = r.to_json().unwrap();
    let decoded: WithdrawalReport = serde_json::from_str(&serialized).unwrap();
    assert_eq!(*r, decoded);
    assert_eq!(decoded.to_json().unwrap(), serialized);
}
#[test]
fn vault_accounts_and_changed_position_artifacts_do_not_grant_proof() {
    let p = selected();
    let mut controlled = p.clone();
    controlled.position_id = "HgQRhiATjX9jTWh7QgLWnhCeR7PaBqoWVf4vSaL61YVv".into();
    let r = execute(&controlled, &WithdrawalProbe::full(&controlled), &fixture());
    assert_eq!(r.status, PathStatus::Indeterminate);
    assert!(!r.execution_attempted);
    controlled = p;
    controlled.principal_exposure_raw[0] = "1".into();
    let r = execute(&controlled, &WithdrawalProbe::full(&controlled), &fixture());
    assert_eq!(r.status, PathStatus::Indeterminate);
    assert!(!r.execution_attempted);
}
#[test]
fn cli_is_portable_offline_byte_identical_and_protects_existing_outputs() {
    let root = repo_root();
    let tmp: PathBuf =
        std::env::temp_dir().join(format!("eplyx-phase10-cli-{}", std::process::id()));
    std::fs::create_dir(&tmp).unwrap();
    for (command, artifact, output) in [
        (
            "discover-position",
            "probes/spacex-dlmm-position.json",
            "position.json",
        ),
        (
            "probe-withdrawal",
            "reports/spacex-dlmm-withdrawal.json",
            "withdrawal.json",
        ),
    ] {
        let args = vec![
            "--snapshot".to_string(),
            root.join("snapshots/spacex-exposure.json")
                .display()
                .to_string(),
            "--scenario".into(),
            root.join("scenarios/spacex-transition.json")
                .display()
                .to_string(),
            "--discovery".into(),
            root.join("evidence/withdrawal/phase10/position-discovery.json")
                .display()
                .to_string(),
            "--fixture".into(),
            root.join("evidence/withdrawal/phase10/execution-capture.json")
                .display()
                .to_string(),
            "--format".into(),
            "json".into(),
            "--out".into(),
            tmp.join(output).display().to_string(),
        ];
        let run = |extra: bool| {
            let mut c = Command::new(env!("CARGO_BIN_EXE_eplyx"));
            c.current_dir(&tmp)
                .arg("path")
                .arg(command)
                .args(&args)
                .env_remove("SOLANA_RPC_URL")
                .env_remove("RPC_URL")
                .env_remove("MAINNET_RPC_URL")
                .env_remove("SOLANA_MAINNET_RPC");
            if extra {
                c.arg("--position").arg(
                    eplyx_engine::repo_root()
                        .join("fixtures/path/main/probes/spacex-dlmm-position.json"),
                );
            }
            c.output().unwrap()
        };
        let extra = command == "probe-withdrawal";
        let actual = run(extra);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        let published = std::fs::read(
            eplyx_engine::repo_root()
                .join("fixtures/path/main")
                .join(artifact),
        )
        .unwrap();
        assert_eq!(actual.stdout, published);
        assert_eq!(std::fs::read(tmp.join(output)).unwrap(), published);
        let repeated = run(extra);
        assert!(!repeated.status.success());
        assert_eq!(std::fs::read(tmp.join(output)).unwrap(), published);
    }
    std::fs::remove_dir_all(tmp).unwrap();
}
