//! Public CLI, portable CAS and standalone credential-free reproduction.
use eplyx_engine::{canonical, change::ChangeSpec, interaction as i};
use serde_json::Value;
use std::{path::Path, process::Command};
fn run(binary: &Path, cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary)
        .current_dir(cwd)
        .env_clear()
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn normal_cli_and_standalone_offline_artifact() {
    let root = eplyx_engine::repo_root();
    let temp = tempfile::tempdir().unwrap();
    let original = Path::new(env!("CARGO_BIN_EXE_eplyx"));
    let candidate = root.join("artifacts/fixture_stake_pool_config_v2.so");
    let program = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";
    let upgrade = ChangeSpec::program_upgrade(program, &std::fs::read(&candidate).unwrap());
    let spec = temp.path().join("upgrade.json");
    std::fs::write(&spec, upgrade.to_document().unwrap()).unwrap();
    let out = temp.path().join("portable");
    let bundle = root.join("deploy/bundle");
    let parameter = root.join("docs/examples/stake-pool-parameter-change.json");
    let args = [
        "interaction",
        "analyse",
        "--upgrade",
        spec.to_str().unwrap(),
        "--parameter",
        parameter.to_str().unwrap(),
        "--bundle",
        bundle.to_str().unwrap(),
        "--record-id",
        "mainnet-spl-stake-pool-151010f709e113e7",
        "--candidate",
        candidate.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--format",
        "json",
    ];
    let output = run(original, temp.path(), &args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["status"], "no_measured_interaction");
    assert_eq!(receipt["recipient_effects"]["interaction"]["value"], "0");
    assert!(!run(original, temp.path(), &args).status.success());
    let report = i::load(&out).unwrap();
    i::verify(&report).unwrap();
    // The only runtime inputs below are the copied executable and CAS directory.
    // No source checkout, bundle, provider, config recipe, PATH or credentials.
    let standalone_root = temp.path().join("isolated");
    std::fs::create_dir(&standalone_root).unwrap();
    let portable = standalone_root.join("artifact");
    std::fs::rename(&out, &portable).unwrap();
    std::fs::remove_file(&spec).unwrap();
    let out = portable;
    let standalone = standalone_root.join("eplyx");
    std::fs::copy(original, &standalone).unwrap();
    assert_eq!(std::fs::read_dir(&standalone_root).unwrap().count(), 2);
    for op in ["verify", "reproduce"] {
        let output = run(
            &standalone,
            &standalone_root,
            &[
                "interaction",
                op,
                "--artifact",
                out.to_str().unwrap(),
                "--format",
                "json",
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let v: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            v["operation"],
            if op == "verify" {
                "verified"
            } else {
                "reproduced"
            }
        );
        assert_eq!(v["offline"], true);
        assert_eq!(v["report_sha256"], receipt["report_sha256"]);
        assert_eq!(v["vm_execution_requested"], op != "verify");
    }
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
    let report_path = out
        .join("objects")
        .join(manifest["objects"]["report"].as_str().unwrap());
    let mut value = serde_json::to_value(&report).unwrap();
    value["r11"]["parent"] = value["r01"]["parent"].clone();
    value.as_object_mut().unwrap().remove("report_sha256");
    value["report_sha256"] = canonical::digest(&value).unwrap().into();
    std::fs::write(report_path, canonical::document(&value).unwrap()).unwrap();
    assert!(!run(
        &standalone,
        temp.path(),
        &[
            "interaction",
            "verify",
            "--artifact",
            out.to_str().unwrap(),
            "--format",
            "json"
        ]
    )
    .status
    .success());
}
