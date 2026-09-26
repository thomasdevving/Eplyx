//! Pure frozen-evidence evaluation. These tests do not execute any transaction.
#[path = "common/lifecycle.rs"]
mod fixture;
use eplyx_engine::{
    lifecycle::artifact::reference_root as repo_root,
    lifecycle::frozen::selection::{canonical, load},
    lifecycle::readiness::{
        self, evidence::ReadinessEvidenceManifest, EvaluatedScope, LifecycleReadinessPolicy,
        LifecycleReadinessReport, ReadinessStatus, RequirementCondition,
    },
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
};
fn policy() -> LifecycleReadinessPolicy {
    load(&repo_root().join("policies/stocklana-spacex-preflight-v1.json")).unwrap()
}
fn evidence() -> &'static readiness::VerifiedReadinessEvidence {
    static E: OnceLock<readiness::VerifiedReadinessEvidence> = OnceLock::new();
    E.get_or_init(|| {
        let root = repo_root();
        let m: ReadinessEvidenceManifest =
            load(&root.join("probes/spacex-readiness-evidence.json")).unwrap();
        m.verify(
            &root.join("probes"),
            &root.join("snapshots/spacex-exposure.json"),
            &root.join("scenarios/spacex-transition.json"),
            &root.join("reports/spacex-lifecycle-path-resolution-phase9.json"),
            &root.join("reports/spacex-dlmm-withdrawal.json"),
            &root.join("reports/spacex-lifecycle-coverage-phase7.json"),
        )
        .unwrap()
    })
}
fn cli(policy: &Path, out: Option<&Path>, direct: Option<&Path>) -> Output {
    let root = repo_root();
    let mut c = Command::new(env!("CARGO_BIN_EXE_eplyx"));
    c.current_dir(std::env::temp_dir())
        .env_remove("SOLANA_RPC_URL")
        .arg("lifecycle")
        .args(["readiness", "--snapshot"])
        .arg(root.join("snapshots/spacex-exposure.json"))
        .arg("--scenario")
        .arg(root.join("scenarios/spacex-transition.json"))
        .arg("--policy")
        .arg(policy)
        .arg("--direct-resolution")
        .arg(
            direct
                .map(PathBuf::from)
                .unwrap_or(root.join("reports/spacex-lifecycle-path-resolution-phase9.json")),
        )
        .arg("--position-resolution")
        .arg(root.join("reports/spacex-dlmm-withdrawal.json"))
        .arg("--coverage")
        .arg(root.join("reports/spacex-lifecycle-coverage-phase7.json"))
        .args(["--format", "json"]);
    if let Some(p) = out {
        c.arg("--out").arg(p);
    }
    c.output().unwrap()
}
#[test]
fn offline_existing_evidence_reproduces_published_report_and_boundaries() {
    let p = policy();
    let r = readiness::evaluate(&p, evidence()).unwrap();
    assert_eq!(r.overall_status, ReadinessStatus::Incomplete);
    assert_eq!(
        r.to_json().unwrap(),
        std::fs::read_to_string(repo_root().join("reports/spacex-lifecycle-readiness.json"))
            .unwrap()
    );
    let round: LifecycleReadinessReport = serde_json::from_str(&r.to_json().unwrap()).unwrap();
    assert_eq!(round.to_json().unwrap(), r.to_json().unwrap());
    let pop = r.rollout_readiness.unwrap();
    assert_eq!(pop.exact_entities_satisfied, 4);
    assert_eq!(pop.positive_entities_required, 10155);
    assert!(!pop.exhaustive_execution);
    assert_eq!(pop.unresolved_account_types["Unknown"], 2969);
    assert_eq!(pop.unresolved_account_types["ProgramOwnedAuthority"], 68);
    let lp = &r.position_exit_evidence[0];
    assert_eq!(
        lp.fee_collection,
        eplyx_engine::lifecycle::resolution::PathStatus::NotTested
    );
    assert_eq!(
        lp.position_closure,
        eplyx_engine::lifecycle::resolution::PathStatus::NotTested
    );
    assert_eq!(
        lp.residual_fees_raw
            .values()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["126543".into(), "113735".into()].into()
    );
    assert!(r
        .findings
        .iter()
        .find(|f| f.requirement_id == "optional-failed-route")
        .unwrap()
        .observed
        .iter()
        .any(|o| o.contains("BitmapExtensionAccountIsNotProvided")));
    assert!(r
        .prevented_rollout_conditions
        .iter()
        .all(|m| !m.real_incident_claimed));
}
#[test]
fn stale_or_mismatched_policy_scope_is_incomplete_and_signer_is_conditional() {
    let mut p = policy();
    p.evaluated_scope = EvaluatedScope::DemoEntityReadiness;
    p.requirements.retain(|r| r.id == "direct-mobility");
    assert_eq!(
        readiness::evaluate(&p, evidence()).unwrap().overall_status,
        ReadinessStatus::Ready
    );
    if let RequirementCondition::Path { any_of } = &mut p.requirements[0].condition {
        for c in any_of {
            c.scope.venue = Some("uncaptured-venue".into());
            c.allow_local_signer_assumption = false;
        }
    }
    assert_eq!(
        readiness::evaluate(&p, evidence()).unwrap().overall_status,
        ReadinessStatus::Incomplete
    );
}
#[test]
fn cli_gate_codes_portability_canonical_output_and_digest_protection() {
    let root = repo_root();
    let temp = fixture::package("readiness-cli");
    let original = root.join("policies/stocklana-spacex-preflight-v1.json");
    let out = temp.join("report.json");
    let run = cli(&original, Some(&out), None);
    assert_eq!(
        run.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, std::fs::read(&out).unwrap());
    assert_eq!(
        run.stdout,
        std::fs::read(root.join("reports/spacex-lifecycle-readiness.json")).unwrap()
    );
    let protected = cli(&original, Some(&out), None);
    assert_eq!(protected.status.code(), Some(2));
    assert_eq!(run.stdout, std::fs::read(&out).unwrap());
    let mut p = policy();
    p.evidence_manifest.file = "probes/spacex-readiness-evidence.json".into();
    p.evaluated_scope = EvaluatedScope::DemoEntityReadiness;
    p.requirements
        .retain(|r| r.id == "direct-mobility" || r.id == "lp-principal-unwind");
    let ready = temp.join("ready.json");
    std::fs::write(&ready, canonical(&p).unwrap()).unwrap();
    let r = cli(&ready, None, None);
    assert_eq!(
        r.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let report: LifecycleReadinessReport = serde_json::from_slice(&r.stdout).unwrap();
    assert!(report.rollout_readiness.is_none());
    assert_eq!(report.entity_readiness.len(), 2);
    let mut p = policy();
    p.evidence_manifest.file = "probes/spacex-readiness-evidence.json".into();
    p.requirements
        .iter_mut()
        .find(|r| r.id == "optional-failed-route")
        .unwrap()
        .required = true;
    let blocked = temp.join("blocked.json");
    std::fs::write(&blocked, canonical(&p).unwrap()).unwrap();
    let r = cli(&blocked, None, None);
    assert_eq!(
        r.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let tampered = temp.join("tampered.json");
    let mut bytes =
        std::fs::read(root.join("reports/spacex-lifecycle-path-resolution-phase9.json")).unwrap();
    bytes.push(b'\n');
    std::fs::write(&tampered, bytes).unwrap();
    let r = cli(&original, None, Some(&tampered));
    assert_eq!(r.status.code(), Some(2));
    assert!(
        serde_json::from_slice::<serde_json::Value>(&r.stdout).unwrap()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("digest mismatch")
    );
    std::fs::remove_dir_all(temp).unwrap();
}
