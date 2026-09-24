//! Phase C1: the gate evaluates a `ChangeSpec`, not a filename.
//!
//! Everything here runs over committed, frozen bundles, offline. The pilot
//! bundle (schema 1) is the cheap one and carries the mismatch tests; the
//! schema-2 Orca, Drift and replay-only bundles are the regression controls.

use std::path::{Path, PathBuf};

use eplyx_engine::{
    change::{Activation, CandidateSource, Change, ChangeSpec, ExecutableArtifact},
    ci::{self, ChangeInput, CheckError, CiReport},
    universal::evidence::{EvidenceKind, EvidenceStore},
};

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/examples")
}

fn pilot() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../deploy/bundle")
}

fn baseline_of(bundle: &Path) -> Vec<u8> {
    std::fs::read(bundle.join("binaries/current.so")).unwrap()
}

fn canonical(report: &CiReport) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(report).unwrap();
    bytes.push(b'\n');
    bytes
}

fn spec_for(bundle: &Path, program: &str) -> ChangeSpec {
    ChangeSpec::program_upgrade(program, &baseline_of(bundle))
}

fn with_spec(
    bundle: &Path,
    spec: ChangeSpec,
    source: CandidateSource<'_>,
) -> Result<CiReport, CheckError> {
    ci::check_change(
        bundle,
        &ChangeInput::Spec {
            spec: &spec,
            source: Some(source),
        },
        None,
    )
}

const STAKE_POOL: &str = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";
const WHIRLPOOL: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
const WHIRLPOOL_PROGRAMDATA: &str = "CtXfPzz36dH5Ws4UYKZvrQ1Xqzn42ecDW6y8NKuiN8nD";
const WHIRLPOOL_AUTHORITY: &str = "GwH3Hiv5mACLX3ufTw1pFsrhSPon5tdw252DBs4Rx4PV";

/// §9. `--candidate` is exactly the minimal spec: same identity, same bytes of
/// report, whether the bytes arrive by file, by store, or implicitly.
#[test]
fn the_candidate_flag_is_the_minimal_program_upgrade() {
    let bundle = pilot();
    let implicit = ci::check(&bundle, &bundle.join("binaries/current.so"), None).unwrap();
    let spec = spec_for(&bundle, STAKE_POOL);
    let change = implicit.change.as_ref().unwrap();
    assert_eq!(change.change_spec_id, spec.id().unwrap());
    assert_eq!(change.kind.as_str(), "program_upgrade");
    assert_eq!(change.target_program_id, STAKE_POOL);
    assert_eq!(change.candidate_sha256, implicit.candidate.sha256);

    let by_file = with_spec(
        &bundle,
        spec.clone(),
        CandidateSource::File(&bundle.join("binaries/current.so")),
    )
    .unwrap();
    assert_eq!(canonical(&implicit), canonical(&by_file));

    let scratch = tempfile::tempdir().unwrap();
    EvidenceStore::at(scratch.path())
        .put(EvidenceKind::ProgramBinary, &baseline_of(&bundle))
        .unwrap();
    let by_store = with_spec(&bundle, spec, CandidateSource::Store(scratch.path())).unwrap();
    assert_eq!(canonical(&implicit), canonical(&by_store));
}

/// §8. The spec says A; the file holds B. Nothing executes.
#[test]
fn a_spec_for_one_candidate_never_executes_another() {
    let bundle = pilot();
    let scratch = tempfile::tempdir().unwrap();
    let other = scratch.path().join("other.so");
    let mut bytes = baseline_of(&bundle);
    bytes[100] ^= 1;
    std::fs::write(&other, &bytes).unwrap();
    let error = with_spec(
        &bundle,
        spec_for(&bundle, STAKE_POOL),
        CandidateSource::File(&other),
    )
    .unwrap_err();
    assert_eq!(error.exit_code(), 2, "{error}");
    assert!(
        format!("{error}").contains("change spec describes"),
        "{error}"
    );
}

/// §5, §6. A store without the object, and a store whose object was altered.
#[test]
fn candidate_evidence_must_exist_and_match_its_address() {
    let bundle = pilot();
    let spec = spec_for(&bundle, STAKE_POOL);
    let scratch = tempfile::tempdir().unwrap();
    let missing = with_spec(
        &bundle,
        spec.clone(),
        CandidateSource::Store(scratch.path()),
    );
    let error = missing.unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(
        format!("{error}").contains("missing evidence object"),
        "{error}"
    );

    let path = EvidenceStore::at(scratch.path())
        .path(
            &EvidenceStore::at(scratch.path())
                .put(EvidenceKind::ProgramBinary, &baseline_of(&bundle))
                .unwrap(),
        )
        .unwrap();
    let mut tampered = baseline_of(&bundle);
    tampered[0] ^= 1;
    std::fs::write(&path, &tampered).unwrap();
    let error = with_spec(
        &bundle,
        spec.clone(),
        CandidateSource::Store(scratch.path()),
    )
    .unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(format!("{error}").contains("hash differs"), "{error}");

    let no_source = ci::check_change(
        &bundle,
        &ChangeInput::Spec {
            spec: &spec,
            source: None,
        },
        None,
    )
    .unwrap_err();
    assert_eq!(no_source.exit_code(), 2);
}

/// §7. A spec for another program is incompatible with the bundle, exit 4.
#[test]
fn a_spec_for_another_program_is_refused() {
    let bundle = pilot();
    let error = with_spec(
        &bundle,
        spec_for(&bundle, WHIRLPOOL),
        CandidateSource::File(&bundle.join("binaries/current.so")),
    )
    .unwrap_err();
    assert_eq!(error.exit_code(), 4, "{error}");
    assert!(format!("{error}").contains("targets program"), "{error}");
}

fn stated(
    bundle: &Path,
    programdata: Option<&str>,
    authority: Option<&str>,
    replaces: Option<&[u8]>,
) -> ChangeSpec {
    let mut spec = spec_for(bundle, WHIRLPOOL);
    let Change::ProgramUpgrade {
        target,
        replaces: replaced,
        expected_upgrade_authority,
        ..
    } = &mut spec.change;
    target.programdata_address = programdata.map(Into::into);
    *expected_upgrade_authority = authority.map(Into::into);
    *replaced = replaces.map(ExecutableArtifact::of);
    spec
}

/// Stated expectations are proved from schema-2 evidence or refused; a
/// schema-1 bundle carries none of it, so a stated one cannot be satisfied.
#[test]
fn stated_target_expectations_are_proved_from_bundle_evidence() {
    let orca = examples().join("phase-u12-orca-semantic-bundle");
    let baseline = baseline_of(&orca);
    let file = orca.join("binaries/current.so");
    let exact = stated(
        &orca,
        Some(WHIRLPOOL_PROGRAMDATA),
        Some(WHIRLPOOL_AUTHORITY),
        Some(&baseline),
    );
    let report = with_spec(&orca, exact.clone(), CandidateSource::File(&file)).unwrap();
    assert_eq!(report.exit_code(), 0);
    assert_eq!(report.change.unwrap().change_spec_id, exact.id().unwrap());

    for (what, spec) in [
        (
            "programdata",
            stated(&orca, Some(WHIRLPOOL_AUTHORITY), None, None),
        ),
        (
            "authority",
            stated(&orca, None, Some(WHIRLPOOL_PROGRAMDATA), None),
        ),
        (
            "baseline",
            stated(&orca, None, None, Some(b"another deployment")),
        ),
    ] {
        let error = with_spec(&orca, spec, CandidateSource::File(&file)).unwrap_err();
        assert_eq!(error.exit_code(), 4, "{what}: {error}");
    }

    let mut unprovable = spec_for(&pilot(), STAKE_POOL);
    let Change::ProgramUpgrade { target, .. } = &mut unprovable.change;
    target.programdata_address = Some(WHIRLPOOL_PROGRAMDATA.into());
    let error = with_spec(
        &pilot(),
        unprovable,
        CandidateSource::File(&pilot().join("binaries/current.so")),
    )
    .unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(
        format!("{error}").contains("no ProgramData evidence"),
        "{error}"
    );
}

/// §13. The proposal never reaches the baseline side: two different specs over
/// one bundle report the same bundle identity and the same replay proof.
#[test]
fn a_change_spec_does_not_touch_baseline_proof_identity() {
    let orca = examples().join("phase-u12-orca-semantic-bundle");
    let file = orca.join("binaries/current.so");
    let manifest = std::fs::read(orca.join("bundle.json")).unwrap();
    let plain = ci::check(&orca, &file, None).unwrap();
    let mut scheduled = stated(&orca, Some(WHIRLPOOL_PROGRAMDATA), None, None);
    scheduled.activation = Some(Activation {
        slot: Some(500_000_000),
        unix_timestamp: None,
    });
    scheduled.metadata.label = Some("scheduled".into());
    let other = with_spec(&orca, scheduled, CandidateSource::File(&file)).unwrap();
    assert_ne!(plain.change, other.change);
    assert_eq!(plain.bundle, other.bundle);
    assert_eq!(plain.replay_proof, other.replay_proof);
    assert_eq!(plain.semantic_binding, other.semantic_binding);
    assert_eq!(plain.summary, other.summary);
    assert_eq!(std::fs::read(orca.join("bundle.json")).unwrap(), manifest);
}

/// §10, §11, §12. The frozen schema-2 controls keep their verdicts, and each
/// now names the change it evaluated.
#[test]
fn frozen_schema_two_controls_keep_their_verdicts() {
    for (directory, program, adapter, exit) in [
        (
            "phase-u14-drift-semantic-bundle",
            "dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH",
            "drift-settle-pnl",
            0,
        ),
        (
            "phase-u12-orca-semantic-bundle",
            WHIRLPOOL,
            "orca-whirlpool",
            0,
        ),
        ("phase-u11-2-checkpointed-bundle", WHIRLPOOL, "none", 2),
    ] {
        let bundle = examples().join(directory);
        let report = ci::check(&bundle, &bundle.join("binaries/current.so"), None).unwrap();
        assert_eq!(report.bundle.adapter, adapter, "{directory}");
        assert_eq!(report.exit_code(), exit, "{directory}");
        assert_eq!(report.replay_proof.as_ref().unwrap().status, "matched");
        let change = report.change.unwrap();
        assert_eq!(change.target_program_id, program);
        assert_eq!(
            change.change_spec_id,
            spec_for(&bundle, program).id().unwrap(),
            "{directory}"
        );
    }
}

/// §8 through the binary: the exit code a pipeline sees, and the spec written
/// by `eplyx change program-upgrade` round-tripping through `--artifacts`.
#[test]
fn the_cli_binds_and_refuses_through_its_exit_codes() {
    let bundle = pilot();
    let scratch = tempfile::tempdir().unwrap();
    let spec_path = scratch.path().join("change.json");
    let store = scratch.path().join("store");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args([
            "change",
            "program-upgrade",
            "--program",
            STAKE_POOL,
            "--candidate",
        ])
        .arg(bundle.join("binaries/current.so"))
        .args(["--label", "pilot baseline", "--store"])
        .arg(&store)
        .arg("--out")
        .arg(&spec_path)
        .status()
        .unwrap();
    assert!(status.success());
    let spec = ChangeSpec::load(&spec_path).unwrap();

    let run = |extra: &[&std::ffi::OsStr]| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx"))
            .args(["ci", "check", "--format", "json", "--bundle"])
            .arg(&bundle)
            .arg("--change-spec")
            .arg(&spec_path)
            .args(extra)
            .output()
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        (output.status.code(), body)
    };
    let (code, body) = run(&["--artifacts".as_ref(), store.as_os_str()]);
    assert_eq!(code, Some(0), "{body}");
    assert_eq!(body["change"]["change_spec_id"], spec.id().unwrap());

    let other = scratch.path().join("other.so");
    let mut bytes = baseline_of(&bundle);
    bytes[7] ^= 1;
    std::fs::write(&other, bytes).unwrap();
    let (code, body) = run(&["--candidate".as_ref(), other.as_os_str()]);
    assert_eq!(code, Some(2), "{body}");
    assert_eq!(body["status"], "error");
    assert!(body.get("findings").is_none(), "no analysis on a mismatch");
}
