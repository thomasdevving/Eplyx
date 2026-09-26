//! Public replay, CLI portability and discovery evidence validation for one entity.
#[path = "common/lifecycle.rs"]
mod fixture;
use eplyx_engine::{
    lifecycle::artifact::reference_root as repo_root,
    lifecycle::frozen::selection::{self as expansion, pipeline::ExecutionIndex},
    lifecycle::resolution::{
        self, phase7::ResolutionBundle, DiscoveryManifest, LifecycleResolution, PathStatus,
    },
    lifecycle::{decode, policy::LifecycleScenario, LifecycleSnapshot},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};
const ENTITY: &str = "741ZXYKzgjPZucRhPUQFK7kKAgP1ESJwimhumgGpuPHs";
fn inputs() -> &'static (LifecycleSnapshot, LifecycleScenario) {
    static C: OnceLock<(LifecycleSnapshot, LifecycleScenario)> = OnceLock::new();
    C.get_or_init(|| {
        let r = repo_root();
        (
            LifecycleSnapshot::load(&r.join("snapshots/spacex-exposure.json")).unwrap(),
            LifecycleScenario::load(&r.join("scenarios/spacex-transition.json")).unwrap(),
        )
    })
}
fn resolve(bundle: &Path) -> anyhow::Result<LifecycleResolution> {
    let (s, p) = inputs();
    let r = repo_root();
    eplyx_engine::lifecycle::resolution::phase7::resolve(
        bundle,
        s,
        p,
        ENTITY,
        &r.join("reports/spacex-lifecycle-coverage-phase7.json"),
        &r.join("probes/spacex-lifecycle-path-discovery.json"),
    )
}
fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("eplyx-phase8-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    p
}
#[test]
fn public_offline_replay_reproduces_and_roundtrips_saved_matrix() {
    let r = repo_root();
    let actual = resolve(&r.join("probes/spacex-phase8-evidence-bundle.json")).unwrap();
    let bytes = std::fs::read(r.join("reports/spacex-lifecycle-path-resolution.json")).unwrap();
    let saved: LifecycleResolution = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(actual, saved);
    assert_eq!(actual.to_json().unwrap().as_bytes(), bytes);
    assert_eq!(
        saved.paths.iter().map(|p| p.status).collect::<Vec<_>>(),
        vec![
            PathStatus::NotTested,
            PathStatus::Unsupported,
            PathStatus::Proven,
            PathStatus::Proven,
            PathStatus::NotApplicable
        ]
    );
    assert_eq!(
        saved.baseline_execution_status,
        eplyx_engine::lifecycle::consequence::LifecycleExecutionStatus::NotTested
    );
    assert_eq!(saved.observed_public_balance_raw, "17621");
}
#[test]
fn replay_rejects_rehashed_fabricated_execution_before_resolution() {
    let dir = fixture::package("rehashed-proof");
    let r = &dir;
    let base = r.join("probes");
    let mut bundle: ResolutionBundle =
        expansion::load(&base.join("spacex-phase8-evidence-bundle.json")).unwrap();
    let index_path = base.join(&bundle.execution_index.file);
    let mut index: ExecutionIndex = expansion::load(&index_path).unwrap();
    let reference = index
        .results
        .iter_mut()
        .find(|r| r.case_id == "group-0-raw-176")
        .unwrap();
    let mut evidence: eplyx_engine::lifecycle::frozen::selection::pipeline::ExecutionEvidence =
        expansion::load(&index_path.parent().unwrap().join(&reference.result_file)).unwrap();
    evidence.deltas.as_mut().unwrap().output_received_raw = "invented-output".into();
    reference.result_sha256 = expansion::digest(&evidence).unwrap();
    reference.result_file = "fabricated-result.json".into();
    eplyx_engine::lifecycle::frozen::selection::save(
        &evidence,
        &index_path.parent().unwrap().join(&reference.result_file),
    )
    .unwrap();
    // Controlled copies also rebind index/coverage digests. Hash consistency alone
    // must still be insufficient to fabricate a successful economic result.
    let index_file = index_path.parent().unwrap().join("fabricated-index.json");
    eplyx_engine::lifecycle::frozen::selection::save(&index, &index_file).unwrap();
    bundle.execution_index.file = index_file
        .strip_prefix(&base)
        .unwrap()
        .to_str()
        .unwrap()
        .into();
    bundle.execution_index.sha256 = expansion::digest(&index).unwrap();
    let mut coverage: eplyx_engine::lifecycle::frozen::selection::pipeline::LifecycleCoverageDeltaReport =
        expansion::load(&r.join("reports/spacex-lifecycle-coverage-phase7.json")).unwrap();
    coverage.execution_index_sha256 = expansion::digest(&index).unwrap();
    coverage.evidence = index.results.clone();
    let coverage_file = base.join("fabricated-coverage.json");
    eplyx_engine::lifecycle::frozen::selection::save(&coverage, &coverage_file).unwrap();
    bundle.coverage.file = "fabricated-coverage.json".into();
    bundle.coverage.sha256 = expansion::digest(&coverage).unwrap();
    let bundle_file = base.join("fabricated-bundle.json");
    eplyx_engine::lifecycle::frozen::selection::save(&bundle, &bundle_file).unwrap();
    let (snapshot, scenario) = inputs();
    let err = resolution::phase7::resolve(
        &bundle_file,
        snapshot,
        scenario,
        ENTITY,
        &coverage_file,
        &r.join("probes/spacex-lifecycle-path-discovery.json"),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("fresh exact entity/path/context replay"),
        "{err}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn cli_is_portable_canonical_offline_and_protects_outputs() {
    let r = repo_root();
    let dir = tmp("cli");
    let out = dir.join("resolution.json");
    let args = vec![
        "resolve-paths".to_owned(),
        "--snapshot".into(),
        r.join("snapshots/spacex-exposure.json")
            .to_string_lossy()
            .into_owned(),
        "--scenario".into(),
        r.join("scenarios/spacex-transition.json")
            .to_string_lossy()
            .into_owned(),
        "--entity".into(),
        ENTITY.into(),
        "--coverage".into(),
        r.join("reports/spacex-lifecycle-coverage-phase7.json")
            .to_string_lossy()
            .into_owned(),
        "--discovery".into(),
        r.join("probes/spacex-lifecycle-path-discovery.json")
            .to_string_lossy()
            .into_owned(),
        "--evidence-bundle".into(),
        r.join("probes/spacex-phase8-evidence-bundle.json")
            .to_string_lossy()
            .into_owned(),
        "--format".into(),
        "json".into(),
        "--out".into(),
        out.to_string_lossy().into_owned(),
    ];
    let run = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_eplyx"));
        c.arg("lifecycle").args(&args).current_dir(&dir);
        for v in [
            "SOLANA_RPC_URL",
            "RPC_URL",
            "ARCHIVE_RPC_URL",
            "SOLANA_ARCHIVE_RPC_URL",
        ] {
            c.env_remove(v);
        }
        c.output().unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, std::fs::read(&out).unwrap());
    assert_eq!(
        result.stdout,
        std::fs::read(r.join("reports/spacex-lifecycle-path-resolution.json")).unwrap()
    );
    let before = std::fs::read(&out).unwrap();
    let repeat = run();
    assert!(!repeat.status.success());
    assert!(
        serde_json::from_slice::<serde_json::Value>(&repeat.stdout).unwrap()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already exists")
    );
    assert_eq!(before, std::fs::read(&out).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn discovery_artifact_tampering_and_unknown_boundaries_are_rejected() {
    let dir = fixture::package("discovery");
    let base = dir.join("probes");
    let mut manifest: DiscoveryManifest =
        expansion::load(&base.join("spacex-lifecycle-path-discovery.json")).unwrap();
    std::fs::write(base.join("changed-source.txt"), "changed issuer evidence").unwrap();
    manifest.sources[0].artifact.file = "changed-source.txt".into();
    let path = base.join("changed-discovery.json");
    expansion::save(&manifest, &path).unwrap();
    assert!(DiscoveryManifest::load(&path)
        .unwrap_err()
        .to_string()
        .contains("artifact digest mismatch"));
    let json = manifest.paths[0].clone();
    let text = expansion::canonical(&json)
        .unwrap()
        .replace("OnchainCandidateUnverified", "Proven");
    assert!(serde_json::from_str::<resolution::PathDiscovery>(&text).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn published_successor_and_bounded_chain_sample_are_real_but_not_transition_proof() {
    let r = repo_root();
    let j: serde_json::Value =
        expansion::load(&r.join("evidence/resolution/phase8/chain-investigation.json")).unwrap();
    let record = &j["records"][0];
    assert_eq!(record["method"], "getMultipleAccounts");
    assert_eq!(record["response"]["result"]["context"]["slot"], 448024657);
    let values = &record["response"]["result"]["value"];
    let source = decode::decode_mint(&values[0]).unwrap();
    let successor = decode::decode_mint(&values[1]).unwrap();
    assert!(source.is_token_2022 && successor.is_token_2022);
    assert_eq!(source.decimals, 9);
    assert_eq!(successor.decimals, 8);
    assert!(successor.is_initialized);
    assert_eq!(
        source.mint_authority.as_deref(),
        Some("WV9PJN7XTmTLVwbutCLFxp8TyePee6Xq5mRq6Fti5Wc")
    );
    assert!(
        std::fs::read_to_string(r.join("evidence/resolution/phase8/issuer-spacex.html"))
            .unwrap()
            .contains(record["params"][0][1].as_str().unwrap())
    );
    let manifest =
        DiscoveryManifest::load(&r.join("probes/spacex-lifecycle-path-discovery.json")).unwrap();
    assert_eq!(
        manifest.paths[0].boundary,
        resolution::MechanismBoundary::OnchainCandidateUnverified
    );
}
