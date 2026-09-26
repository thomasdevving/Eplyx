use eplyx_engine::{
    lifecycle::frozen::selection::{self as expansion, digest},
    lifecycle::notice::{workflow::NoticeWorkflow, NormalizedLifecycleEvent},
    lifecycle::readiness::ReadinessStatus,
    lifecycle::resolution::PathStatus,
};
use std::{path::PathBuf, process::Command};
fn root() -> PathBuf {
    eplyx_engine::lifecycle::artifact::reference_root()
}
#[test]
fn notice_cannot_bypass_existing_readiness_requirements() {
    let r = root();
    let base = r.join("probes");
    let w = NoticeWorkflow::load(&base.join("spacex-notice-workflow.json")).unwrap();
    let e = w.normalize(&base).unwrap().event().clone();
    let (s, b) = w.generate(&base, &e).unwrap();
    let (report, impact) = w.preflight(&base, &e, &s, &b).unwrap();
    assert_eq!(
        report.readiness.overall_status,
        ReadinessStatus::Incomplete,
        "notice cannot bypass unchanged official/complete-exit/population requirements"
    );
    let old: eplyx_engine::lifecycle::readiness::LifecycleReadinessReport =
        expansion::load(&r.join("reports/spacex-lifecycle-readiness.json")).unwrap();
    assert_eq!(
        report.readiness, old,
        "same original policy/evidence must produce identical gate"
    );
    assert_eq!(report.readiness_sha256, digest(&old).unwrap());
    assert_eq!(impact.entities.len(), 17957);
    assert_eq!(impact.scenario, s);
    assert_eq!(
        impact.to_json().unwrap().as_bytes(),
        std::fs::read(r.join("reports/spacex-notice-transition-impact.json")).unwrap()
    );
    assert_eq!(e.official_execution_status.value, PathStatus::NotTested);
    assert!(!report.semantic_compatibility.proof_contexts_rewritten);
    assert!(!report.semantic_compatibility.readiness_policy_rewritten);
    assert_eq!(report.resolution.direct_paths[0]["status"], "NotTested");
    let canonical: NormalizedLifecycleEvent =
        expansion::load(&r.join("reports/spacex-lifecycle-event.json")).unwrap();
    assert_eq!(e, canonical);
    let published: eplyx_engine::lifecycle::notice::workflow::NoticePreflightReport =
        expansion::load(&r.join("reports/spacex-notice-preflight.json")).unwrap();
    assert_eq!(report.to_json().unwrap(), published.to_json().unwrap());
}
#[test]
fn offline_cli_replays_portably_and_protects_outputs() {
    let r = root();
    let tmp = std::env::temp_dir().join(format!("eplyx-notice-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let binary = env!("CARGO_BIN_EXE_eplyx");
    let workflow = r.join("probes/spacex-notice-workflow.json");
    for (cmd, artifact, code) in [
        ("ingest-notice", "reports/spacex-lifecycle-event.json", 0),
        (
            "scenario-from-event",
            "scenarios/spacex-transition-ingested.json",
            0,
        ),
        (
            "preflight-from-notice",
            "reports/spacex-notice-preflight.json",
            5,
        ),
    ] {
        let out = tmp.join(format!("{cmd}.json"));
        let mut command = Command::new(binary);
        command
            .current_dir(&tmp)
            .env_remove("SOLANA_RPC_URL")
            .arg("lifecycle")
            .args([cmd, "--workflow"])
            .arg(&workflow)
            .args(["--format", "json", "--out"])
            .arg(&out);
        if cmd != "ingest-notice" {
            command.arg("--event").arg(tmp.join("ingest-notice.json"));
        }
        if cmd == "scenario-from-event" {
            command.arg("--out-binding").arg(tmp.join("binding.json"));
        }
        if cmd == "preflight-from-notice" {
            command
                .arg("--scenario")
                .arg(tmp.join("scenario-from-event.json"))
                .arg("--binding")
                .arg(tmp.join("binding.json"));
        }
        let result = command.output().unwrap();
        assert_eq!(
            result.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let expected = std::fs::read(r.join(artifact)).unwrap();
        assert_eq!(result.stdout, expected);
        assert_eq!(std::fs::read(&out).unwrap(), expected);
        let protected = Command::new(binary)
            .current_dir(&tmp)
            .arg("lifecycle")
            .args([cmd, "--workflow"])
            .arg(&workflow)
            .arg("--out")
            .arg(&out)
            .output()
            .unwrap();
        assert_eq!(protected.status.code(), Some(2));
        assert_eq!(std::fs::read(out).unwrap(), expected);
    }
    let mut e: NormalizedLifecycleEvent =
        expansion::load(&r.join("reports/spacex-lifecycle-event.json")).unwrap();
    e.deadline.value += chrono::Duration::days(1);
    let tampered = tmp.join("tampered.json");
    eplyx_engine::lifecycle::frozen::selection::save(&e, &tampered).unwrap();
    let out = tmp.join("rejected.json");
    let bad = Command::new(binary)
        .current_dir(&tmp)
        .arg("lifecycle")
        .args(["scenario-from-event", "--workflow"])
        .arg(&workflow)
        .arg("--event")
        .arg(tampered)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(!out.exists());
    std::fs::remove_dir_all(tmp).unwrap();
}
