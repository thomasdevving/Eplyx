//! Built-binary identity, determinism and offline worker isolation.
use eplyx_engine::{
    change::ChangeSpec,
    lifecycle::{artifact::reference_root, policy::LifecycleScenario},
};
use serde_json::Value;
use std::process::{Command, Output};
fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(args)
        .env_clear()
        .env("SOLANA_RPC_URL", "sentinel-no-network")
        .env("EPLYX_TOKEN", "sentinel-never-forwarded")
        .output()
        .unwrap()
}
#[test]
fn lifecycle_analysis_is_bound_deterministic_and_does_not_inherit_parent_secrets() {
    let root = reference_root();
    let dir = tempfile::tempdir().unwrap();
    let scenario = root.join("scenarios/spacex-transition.json");
    let snapshot = root.join("snapshots/spacex-exposure.json");
    let proposal = dir.path().join("change.json");
    let made = cli(&[
        "change",
        "lifecycle",
        "--scenario",
        scenario.to_str().unwrap(),
        "--out",
        proposal.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let spec = ChangeSpec::parse(&std::fs::read(&proposal).unwrap()).unwrap();
    let terms = LifecycleScenario::load(&scenario).unwrap();
    let at = terms.policy.deadline.as_ref().unwrap().at.to_rfc3339();
    let out = dir.path().join("report.json");
    let args = [
        "lifecycle",
        "analyse",
        "--snapshot",
        snapshot.to_str().unwrap(),
        "--scenario",
        scenario.to_str().unwrap(),
        "--change-spec",
        proposal.to_str().unwrap(),
        "--at",
        at.as_str(),
        "--format",
        "json",
    ];
    let first = cli(&args);
    let second = cli(&args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["change"]["change_spec_id"], spec.id().unwrap());
    assert_eq!(report["change"]["kind"], "lifecycle_change");
    assert!(report["impact"]["entities"].as_array().unwrap().len() > 17000);
    assert!(!String::from_utf8_lossy(&first.stdout).contains("sentinel"));
    let mut saved = args.to_vec();
    saved.extend(["--out", out.to_str().unwrap()]);
    let write = cli(&saved);
    assert!(write.status.success());
    assert_eq!(std::fs::read(&out).unwrap(), first.stdout);
    let overwrite = cli(&saved);
    assert_eq!(overwrite.status.code(), Some(2));
    assert_eq!(std::fs::read(&out).unwrap(), first.stdout);
    let error: Value = serde_json::from_slice(&overwrite.stdout).unwrap();
    assert_eq!(error["error"]["code"], 2);
}
#[test]
fn directly_invoked_lifecycle_worker_refuses_inherited_environment() {
    let run = Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["lifecycle-worker", "{}"])
        .env_clear()
        .env("WORKER_SECRET_SENTINEL", "not-forwarded")
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&run.stderr).contains("empty environment"));
    assert!(!String::from_utf8_lossy(&run.stderr).contains("not-forwarded"));
}
