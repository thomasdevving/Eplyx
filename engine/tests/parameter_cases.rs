use eplyx_engine::{
    canonical,
    change::ChangeSpec,
    lifecycle::{artifact, LifecycleSnapshot},
    parameter_cases as cases, parameter_change as p,
    path::{token_transfer, CapturedExecutionFixture},
    replay::hash_bytes,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn proposal() -> PathBuf {
    eplyx_engine::repo_root().join("docs/examples/parameter-case-set-change.json")
}
fn retained(group: usize, amount: &str) -> p::Input {
    let root = artifact::reference_root();
    let index: Value =
        artifact::load(&root.join("reports/phase7-evidence/execution-index.json")).unwrap();
    let id = format!("group-{group}-raw-{amount}");
    let entry = index["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["case_id"] == id)
        .unwrap();
    let bytes = artifact::read_relative(
        &root.join("reports/phase7-evidence"),
        entry["result_file"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(hash_bytes(&bytes), entry["result_sha256"]);
    let original: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(original["path_type"], "Transfer");
    assert_eq!(original["execution"]["success"], true);
    assert_eq!(original["deltas"]["reconciled"], true);
    assert_eq!(original["invalid_control"], false);
    assert_eq!(original["input_raw"], amount);
    assert_eq!(original["authority"]["authority_model"], "WalletCompatible");
    let fixture: CapturedExecutionFixture = artifact::load(&root.join(format!(
        "probes/phase7-captures/fixtures/group-{group}.json"
    )))
    .unwrap();
    assert_eq!(fixture.sha256().unwrap(), entry["fixture_sha256"]);
    let snapshot = LifecycleSnapshot::load(&root.join("snapshots/spacex-exposure.json")).unwrap();
    let source = original["group_id"]
        .as_str()
        .unwrap()
        .split(':')
        .nth(1)
        .unwrap();
    let entity = snapshot
        .entities
        .iter()
        .find(|e| e.token_account == source)
        .unwrap();
    let destination = "124XHuTYUNnCf2ABCNcCEdQFpHQ7Y6MnPe9NeouDB1az";
    let receiver = snapshot
        .entities
        .iter()
        .find(|e| e.token_account == destination)
        .unwrap();
    let input = p::Input {
        schema_version: 1,
        amount_raw: original["input_raw"].as_str().unwrap().parse().unwrap(),
        context: token_transfer::TransferContext {
            genesis_hash: snapshot.source.genesis_hash,
            minimum_slot: snapshot.source.max_observed_slot,
            mint: snapshot.asset.mint,
            program: eplyx_engine::standard_programs::token2022::PROGRAM_ID.into(),
            decimals: snapshot.mint_config.decimals,
            source: source.into(),
            owner: entity.state.owner.clone(),
            destination: destination.into(),
            destination_owner: Some(receiver.state.owner.clone()),
        },
        fixture_sha256: fixture.sha256().unwrap(),
        source_capture_sha256: None,
        fixture,
    };
    let plan = input.validate().unwrap();
    assert_eq!(
        serde_json::to_value(eplyx_engine::path::ProbeClock::from(&plan.clock)).unwrap(),
        original["vm_clock"]
    );
    assert_eq!(
        serde_json::to_value(eplyx_engine::path::ProbeMessage::from(&plan.message)).unwrap(),
        original["message"]
    );
    input
}
fn inputs() -> Vec<p::Input> {
    vec![retained(0, "17621"), retained(4, "309138")]
}
fn save_inputs(base: &Path, inputs: &[p::Input]) -> Vec<PathBuf> {
    inputs
        .iter()
        .enumerate()
        .map(|(i, input)| {
            let p = base.join(format!("retained-{i}.json"));
            std::fs::write(&p, canonical::document(input).unwrap()).unwrap();
            p
        })
        .collect()
}
fn prepared(base: &Path, inputs: &[p::Input]) -> PathBuf {
    let paths = save_inputs(base, inputs);
    let request = base.join("request");
    cases::prepare(&proposal(), &paths, &request).unwrap();
    request.join("manifest.json")
}
fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["parameter", "cases"])
        .args(args)
        .env("AWS_SECRET_ACCESS_KEY", "case-set-test-canary")
        .env("SOLANA_RPC_URL", "https://invalid.test/")
        .output()
        .unwrap()
}

#[test]
fn genuine_selected_cases_and_complete_offline_repeat() {
    let temporary = tempfile::tempdir().unwrap();
    let selected = inputs();
    let originals = selected
        .iter()
        .map(|i| canonical::document(i).unwrap())
        .collect::<Vec<_>>();
    let manifest_path = prepared(temporary.path(), &selected);
    let package = temporary.path().join("package");
    let result = cli(&[
        "analyse",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--out",
        package.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let summary: cases::Summary = artifact::load(&package.join("summary.json")).unwrap();
    assert_eq!(
        summary.change_spec_id,
        "7f24680b92d65edec79f97175be9a81cb35dbf8670581af13519c8f444da3a11"
    );
    assert_eq!(
        summary.counts,
        cases::Counts {
            selected_cases: 2,
            executed_pairs: 2,
            reconciled_pairs: 2,
            measured_consequence: 2,
            no_observed_consequence: 0,
            unavailable_or_failed: 0
        }
    );
    assert_ne!(summary.case_set_id, summary.result_sha256);
    assert_ne!(summary.rows[0]["source"], summary.rows[1]["source"]);
    assert_ne!(summary.rows[0]["clock"], summary.rows[1]["clock"]);
    for (i, row) in summary.rows.iter().enumerate() {
        let report: Value = serde_json::from_slice(
            &artifact::read_relative(&package, &summary.reports[i].path).unwrap(),
        )
        .unwrap();
        let spec = ChangeSpec::parse(&artifact::read(proposal()).unwrap()).unwrap();
        p::verify(&spec, &report).unwrap();
        let retained: p::Input = serde_json::from_value(report["retained_input"].clone()).unwrap();
        let original = selected
            .iter()
            .find(|s| s.context.source == retained.context.source)
            .unwrap();
        assert_eq!(
            canonical::document(&retained).unwrap(),
            canonical::document(original).unwrap()
        );
        assert_eq!(
            row["initial_source_balance_raw"],
            original.amount_raw.to_string()
        );
        let expected = if original.amount_raw == 17621 {
            ["17532", "17268", "-264", "89", "353", "264"]
        } else {
            ["307592", "302955", "-4637", "1546", "6183", "4637"]
        };
        for (field, value) in [
            "baseline_recipient_credit_raw",
            "proposed_recipient_credit_raw",
            "recipient_credit_difference_raw",
            "baseline_destination_withheld_change_raw",
            "proposed_destination_withheld_change_raw",
            "destination_withheld_difference_raw",
        ]
        .into_iter()
        .zip(expected)
        {
            assert_eq!(row["quantities"][field], value);
        }
        assert_eq!(row["report_sha256"], report["report_sha256"]);
        assert_eq!(
            row["quantities"]["baseline_recipient_credit_raw"],
            report["baseline"]["reconciliation"]["output_received_raw"]
        );
        assert_eq!(
            row["quantities"]["proposed_recipient_credit_raw"],
            report["proposed"]["reconciliation"]["output_received_raw"]
        );
    }
    assert_eq!(
        selected
            .iter()
            .map(|i| canonical::document(i).unwrap())
            .collect::<Vec<_>>(),
        originals
    );
    // Move only the self-contained package, then delete all request/input paths.
    let relocated = temporary.path().join("relocated");
    std::fs::rename(&package, &relocated).unwrap();
    std::fs::remove_dir_all(manifest_path.parent().unwrap()).unwrap();
    for p in save_inputs(temporary.path(), &selected) {
        std::fs::remove_file(p).unwrap();
    }
    for command in ["verify", "reproduce"] {
        let result = cli(&[command, "--package", relocated.to_str().unwrap()]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let receipt: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(receipt["offline"], true);
        assert_eq!(receipt["cases"].as_array().unwrap().len(), 2);
        assert!(receipt["cases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["verified"] == true));
        if command == "reproduce" {
            assert!(receipt["cases"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["reproduced"] == true));
        }
        assert!(!String::from_utf8_lossy(&result.stdout).contains("case-set-test-canary"));
    }
    if let Some(out) = std::env::var_os("EPLYX_PARAMETER_CASE_QUALIFICATION_OUT") {
        let out = PathBuf::from(out);
        std::fs::create_dir(&out).unwrap();
        for item in std::fs::read_dir(&relocated).unwrap() {
            let item = item.unwrap();
            std::fs::copy(item.path(), out.join(item.file_name())).unwrap();
        }
    }
}

#[test]
fn duplicates_do_not_become_distinct_by_renaming_or_amount() {
    for mutate_amount in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let input = retained(0, "17621");
        let mut other = input.clone();
        if mutate_amount {
            other.amount_raw = 10000;
        }
        let paths = save_inputs(dir.path(), &[input, other]);
        let error = cases::prepare(&proposal(), &paths, &dir.path().join("out")).unwrap_err();
        assert!(error.to_string().contains(if mutate_amount {
            "duplicate source"
        } else {
            "identifiers must be distinct"
        }));
        assert!(!dir.path().join("out").exists());
    }
}

#[test]
fn wrong_target_stale_state_unsupported_and_missing_capture_fail_before_execution() {
    for kind in ["target", "stale", "unsupported", "missing", "balance"] {
        let dir = tempfile::tempdir().unwrap();
        let mut selected = inputs();
        match kind {
            "target" => selected[1].context.mint = selected[1].context.destination.clone(),
            "stale" => (),
            "unsupported" => {
                selected[1].context.program = "11111111111111111111111111111111".into()
            }
            "missing" => {
                selected[1].fixture.evidence.pop();
                selected[1].fixture_sha256 = selected[1].fixture.sha256().unwrap();
            }
            "balance" => selected[1].amount_raw += 1,
            _ => unreachable!(),
        }
        let paths = save_inputs(dir.path(), &selected);
        let change = if kind == "stale" {
            let mut spec: Value =
                serde_json::from_slice(&artifact::read(proposal()).unwrap()).unwrap();
            spec.as_object_mut().unwrap().remove("change_spec_id");
            spec["change"]["operation"]["expected_current"]["basis_points"] = json!(51);
            let path = dir.path().join("stale.json");
            std::fs::write(&path, canonical::document(&spec).unwrap()).unwrap();
            path
        } else {
            proposal()
        };
        assert!(
            cases::prepare(&change, &paths, &dir.path().join("out")).is_err(),
            "{kind}"
        );
        assert!(!dir.path().join("out").exists());
    }
}

#[test]
fn identities_ignore_labels_locations_order_of_preparation_and_json_formatting() {
    let dir = tempfile::tempdir().unwrap();
    let selected = inputs();
    let paths = save_inputs(dir.path(), &selected);
    let a = cases::prepare(&proposal(), &paths, &dir.path().join("a")).unwrap();
    let b = cases::prepare(
        &proposal(),
        &paths.into_iter().rev().collect::<Vec<_>>(),
        &dir.path().join("b"),
    )
    .unwrap();
    assert_eq!(a.case_set_id, b.case_set_id);
    let mut cosmetic = a.clone();
    cosmetic.cases[0].label = Some("renamed source".into());
    cosmetic.cases[0].input.path = "somewhere-else.json".into();
    cosmetic.change.path = "elsewhere.json".into();
    assert_eq!(a.id().unwrap(), cosmetic.id().unwrap());
    cosmetic.contract.programs[0]["elf_sha256"] = json!("different");
    assert_ne!(a.id().unwrap(), cosmetic.id().unwrap());
    let mut unknown = serde_json::to_value(&a).unwrap();
    unknown["production_coverage"] = json!(1);
    assert!(serde_json::from_value::<cases::Manifest>(unknown).is_err());
}

#[test]
fn tampered_references_contract_manifest_reports_and_summary_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = prepared(dir.path(), &inputs());
    let original = std::fs::read(&manifest_path).unwrap();
    for kind in ["input", "manifest", "runtime", "path", "unknown"] {
        let mut m: Value = serde_json::from_slice(&original).unwrap();
        match kind {
            "input" => m["cases"][0]["input"]["sha256"] = json!("0".repeat(64)),
            "manifest" => m["case_set_id"] = json!("0".repeat(64)),
            "runtime" => m["contract"]["parameter_runtime"]["profile"] = json!("changed"),
            "path" => m["cases"][0]["input"]["path"] = json!("../retained-0.json"),
            "unknown" => m["cases"][0]["input"]["extra"] = json!(true),
            _ => unreachable!(),
        }
        std::fs::write(&manifest_path, canonical::document(&m).unwrap()).unwrap();
        assert!(cases::analyze(&manifest_path, &dir.path().join("out")).is_err());
        assert!(!dir.path().join("out").exists());
    }
    std::fs::write(&manifest_path, &original).unwrap();
    let out = dir.path().join("out");
    let summary = cases::analyze(&manifest_path, &out).unwrap();
    let path = out.join(&summary.reports[0].path);
    let report = std::fs::read(&path).unwrap();
    // Even replacing the file commitment cannot bypass the existing report verifier.
    let mut tampered: Value = serde_json::from_slice(&report).unwrap();
    tampered["baseline"]["reconciliation"]["output_received_raw"] = json!("0");
    let bytes = canonical::document(&tampered).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let mut summary = summary;
    summary.reports[0].sha256 = hash_bytes(bytes.as_bytes());
    std::fs::write(
        out.join("summary.json"),
        canonical::document(&summary).unwrap(),
    )
    .unwrap();
    let receipt = cases::check(&out, true).unwrap();
    assert_eq!(receipt["status"], "case_set_evidence_blocked");
    assert_eq!(receipt["cases"][0]["verified"], false);
    assert_eq!(receipt["cases"][1]["verified"], true);
    assert!(receipt["cases"][1]["reproduced"].is_null());
    assert_eq!(receipt["reproduction_performed"], false);
    std::fs::write(&path, &report).unwrap();
    summary.reports[0].sha256 = hash_bytes(&report);
    summary.rows[0]["quantities"]["proposed_recipient_credit_raw"] = json!("0");
    summary.result_sha256 = summary.digest().unwrap();
    std::fs::write(
        out.join("summary.json"),
        canonical::document(&summary).unwrap(),
    )
    .unwrap();
    assert_eq!(
        cases::check(&out, false).unwrap()["summary_verified"],
        false
    );
}

#[test]
fn repeated_analysis_preserves_single_case_reports_and_result_identity() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = prepared(dir.path(), &inputs());
    let a = cases::analyze(&manifest_path, &dir.path().join("a")).unwrap();
    let mut manifest: cases::Manifest = artifact::load(&manifest_path).unwrap();
    manifest.cases[0].label = Some("cosmetic label only".into());
    let input_path = manifest_path
        .parent()
        .unwrap()
        .join(&manifest.cases[0].input.path);
    let input: Value = artifact::load(&input_path).unwrap();
    let reformatted = serde_json::to_vec(&input).unwrap();
    std::fs::write(&input_path, &reformatted).unwrap();
    manifest.cases[0].input.sha256 = hash_bytes(&reformatted);
    std::fs::write(&manifest_path, canonical::document(&manifest).unwrap()).unwrap();
    let b = cases::analyze(&manifest_path, &dir.path().join("b")).unwrap();
    assert_eq!(a.result_sha256, b.result_sha256);
    for (ra, rb) in a.reports.iter().zip(&b.reports) {
        assert_eq!(ra.sha256, rb.sha256);
    }
    assert!(cases::analyze(&manifest_path, &dir.path().join("a")).is_err());
}
