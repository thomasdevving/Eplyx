//! The `eplyx` binary over a Token Migration V1 project: doctor, preflight through
//! the isolated offline worker, search, gate policies, the unsigned plan, show and
//! offline reproduction of a counterexample. No RPC and no network.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn eplyx(root: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("EPLYX_TOKEN", "CLI_AUTH_SENTINEL")
        .env("SOLANA_RPC_URL", "CLI_PROVIDER_SENTINEL_NOT_AN_ENDPOINT")
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn project(label: &str, candidate: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "eplyx-migration-cli-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let repo = eplyx_engine::repo_root();
    copy_tree(&repo.join("examples/migrations/minimal"), &root);
    fs::create_dir_all(root.join("target/deploy")).unwrap();
    fs::copy(
        repo.join(format!("artifacts/{candidate}.so")),
        root.join("target/deploy/migration.so"),
    )
    .unwrap();
    root.canonicalize().unwrap()
}

fn latest_run(root: &Path) -> String {
    let mut runs: Vec<String> = fs::read_dir(root.join(".eplyx/runs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    runs.sort();
    runs.pop().unwrap()
}

#[test]
fn migration_project_runs_the_whole_local_flow_offline() {
    let root = project("flow", "eplyx_token_migration");
    let (code, out, err) = eplyx(&root, &["doctor"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("not needed: the state is a synthetic fixture"));
    let (code, out, err) = eplyx(&root, &["migration", "analyse"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("Eplyx migration analysis") && out.contains("Passed with warnings"),
        "{out}"
    );
    assert!(out.contains("no transaction was signed or sent"), "{out}");
    let run = latest_run(&root);
    let report: Value = serde_json::from_slice(
        &fs::read(
            root.join(".eplyx/runs")
                .join(&run)
                .join("result/report.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(report["readiness"]["mechanism"]["status"], "Ready");
    let (code, out, _) = eplyx(
        &root,
        &["migration", "gate", "--run", &run, "--policy", "strict"],
    );
    assert_eq!(
        code, 1,
        "strict blocks the known frozen/authority requirements: {out}"
    );
    let (code, out, _) = eplyx(&root, &["migration", "gate", "--run", &run]);
    assert_eq!(code, 0, "{out}");
    let (code, out, err) = eplyx(&root, &["migration", "search", "--run", &run]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("No counterexample found within this search domain and budget."),
        "{out}"
    );
    let (code, _, err) = eplyx(
        &root,
        &["migration", "plan", "--run", &run, "--out", "unsigned.json"],
    );
    assert_eq!(code, 0, "{err}");
    let plan: Value =
        serde_json::from_slice(&fs::read(root.join("unsigned.json")).unwrap()).unwrap();
    assert!(plan["notice"]
        .as_str()
        .unwrap()
        .starts_with("UNSIGNED EXECUTION PLAN"));
    let text = serde_json::to_string(&plan).unwrap();
    assert!(
        !text.to_lowercase().contains("secret")
            && !text.contains("PRIVATE KEY")
            && !text.contains("\"signature\"")
    );
    let (code, out, _) = eplyx(&root, &["show", &run]);
    assert_eq!(code, 0);
    assert!(out.contains("Token Migration V1 rehearsal"));
    let (code, out, _) = eplyx(&root, &["runs", "--json"]);
    assert_eq!(code, 0);
    assert!(out.contains(&run));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn defective_candidate_counterexample_reproduces_offline_and_is_recorded() {
    let root = project("defect", "eplyx_token_migration_defect_deadline_inclusive");
    let (code, out, err) = eplyx(&root, &["migration", "search"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("UnexpectedSuccess"), "{out}");
    let run = latest_run(&root);
    let (gate_code, gate_out, gate_err) = eplyx(
        &root,
        &["migration", "gate", "--run", &run, "--format", "json"],
    );
    assert_eq!(gate_code, 1, "{gate_out}{gate_err}");
    assert!(gate_out.contains("COUNTEREXAMPLE_FOUND"));
    let saved: Vec<PathBuf> = fs::read_dir(root.join(".eplyx/counterexamples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(saved.len(), 1);
    let id = saved[0].file_stem().unwrap().to_string_lossy().into_owned();
    let (code, out, err) = eplyx(&root, &["migration", "reproduce", &id]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("No network used."));
    let records: Vec<PathBuf> = fs::read_dir(root.join(".eplyx/reproductions"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let record: Value = serde_json::from_slice(&fs::read(&records[0]).unwrap()).unwrap();
    assert_eq!(record["outcome"], "Reproduced");
    assert_eq!(record["no_rpc"], true);
    assert_eq!(record["binding"]["finding"], "UnexpectedSuccess");
    assert!(record["binding"]["world_sha256"].as_str().unwrap().len() == 64);
    // Tampering with the saved counterexample is detected and recorded as a failure.
    let mut value: Value = serde_json::from_slice(&fs::read(&saved[0]).unwrap()).unwrap();
    value["counterexample"]["derived_value_raw"] = "1".into();
    fs::write(&saved[0], serde_json::to_vec(&value).unwrap()).unwrap();
    let (code, _, _) = eplyx(&root, &["migration", "reproduce", &id]);
    assert_eq!(code, 2);
    let records: Vec<Value> = fs::read_dir(root.join(".eplyx/reproductions"))
        .unwrap()
        .map(|e| serde_json::from_slice(&fs::read(e.unwrap().path()).unwrap()).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert!(records
        .iter()
        .any(|r| r["outcome"] == "Failed" && r["failure_signature_matched"] == false));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn init_writes_a_migration_project_that_doctor_can_read() {
    let root =
        std::env::temp_dir().join(format!("eplyx-migration-cli-init-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let (code, out, err) = eplyx(&root, &["init", "--migration", "--fixture"]);
    assert_eq!(code, 0, "{out}{err}");
    for file in [
        "eplyx.toml",
        "migration.json",
        "fixtures/world.json",
        ".gitignore",
    ] {
        assert!(root.join(file).is_file(), "missing {file}");
    }
    let toml = fs::read_to_string(root.join("eplyx.toml")).unwrap();
    assert!(toml.contains("token_migration_v1") && toml.contains("[[invariants]]"));
    // The template deliberately leaves the mints blank: doctor reports what to fix.
    let (code, out, _) = eplyx(&root, &["doctor"]);
    assert_eq!(code, 2);
    assert!(
        out.contains("Project config      eplyx.toml (token_migration_v1)"),
        "{out}"
    );
    assert!(out.contains("✗ Migration spec"), "{out}");
    let (code, _, _) = eplyx(&root, &["init", "--migration"]);
    assert_ne!(code, 0, "init refuses to overwrite without --force");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn version_needs_no_project_config_store_or_network() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = eplyx(dir.path(), &["--version"]);
    assert_eq!(code, 0, "{err}");
    let lines: Vec<_> = out.lines().collect();
    assert_eq!(lines[0], format!("eplyx {}", env!("CARGO_PKG_VERSION")));
    assert!(lines[1].starts_with("commit "));
    assert!(lines[2].starts_with("target "));
    assert!(lines[3].contains("eplyx-migration-counterexample-search/v1"));
    let (_, out, _) = eplyx(dir.path(), &["-V"]);
    assert_eq!(out.trim(), format!("eplyx {}", env!("CARGO_PKG_VERSION")));
    let (code, out, err) = eplyx(dir.path(), &["version", "--json"]);
    assert_eq!(code, 0, "{err}");
    let value: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["engine"]["run_metadata_schema"], 2);
    assert_eq!(
        value["engine"]["change_specs"].as_object().unwrap().len(),
        3
    );
    assert!(value["platform"].as_str().unwrap().contains('-'));
    assert!(!out.contains(dir.path().to_string_lossy().as_ref()));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn doctor_separates_runtime_requirements_and_never_reads_provider_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args(["doctor"])
        .current_dir(dir.path())
        .env_clear()
        .env("SOLANA_RPC_URL", "DOCTOR_PROVIDER_SECRET_NOT_AN_ENDPOINT")
        .env("EPLYX_TOKEN", "DOCTOR_AUTH_SECRET")
        .output()
        .unwrap();
    let out = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        out.contains("✓ Installed binary")
            && out.contains("needs no Rust")
            && out.contains("run `eplyx init --migration`")
    );
    assert!(!out.contains("DOCTOR_PROVIDER_SECRET") && !out.contains("DOCTOR_AUTH_SECRET"));
    assert_eq!(
        eplyx(dir.path(), &["init", "--migration", "--fixture"]).0,
        0
    );
    let (code, out, _) = eplyx(dir.path(), &["doctor"]);
    assert_eq!(code, 2);
    assert!(out.contains("✗ Candidate program") && out.contains("cargo build-sbf"));
}

#[test]
fn structured_errors_and_immutable_inputs_are_enforced() {
    let root = project("json", "eplyx_token_migration");
    let (code, out, err) = eplyx(&root, &["migration", "analyse", "--format", "json"]);
    assert_eq!(code, 0, "{out}{err}");
    let value: Value = serde_json::from_str(&out).unwrap();
    let id = value["run_id"].as_str().unwrap();
    let run = root.join(".eplyx/runs").join(id);
    let original = fs::read(run.join("result/report.json")).unwrap();
    let (code, out, err) = eplyx(&root, &["migration", "analyse", "--format", "json"]);
    assert_eq!(code, 0, "{out}{err}");
    let again: Value = serde_json::from_str(&out).unwrap();
    let again = again["run_id"].as_str().unwrap();
    assert_eq!(
        original,
        fs::read(
            root.join(".eplyx/runs")
                .join(again)
                .join("result/report.json")
        )
        .unwrap()
    );
    // A different binding is a state incompatibility, not a new evaluation.
    let file = run.join("result/bindings.json");
    let mut bindings: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    let saved = fs::read(&file).unwrap();
    bindings["change_spec_id"] = "0".repeat(64).into();
    fs::write(&file, serde_json::to_vec(&bindings).unwrap()).unwrap();
    let (code, out, err) = eplyx(
        &root,
        &["migration", "gate", "--run", id, "--format", "json"],
    );
    assert_eq!(code, 4, "{out}{err}");
    assert_eq!(
        serde_json::from_str::<Value>(&out).unwrap()["error"]["code"],
        4
    );
    fs::write(file, saved).unwrap();
    // A missing or substituted CAS candidate is invalid input and cannot execute.
    let program = fs::read_dir(run.join("input/programs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let bytes = fs::read(&program).unwrap();
    fs::remove_file(&program).unwrap();
    let (code, out, _) = eplyx(
        &root,
        &["migration", "gate", "--run", id, "--format", "json"],
    );
    assert_eq!(code, 2);
    assert_eq!(
        serde_json::from_str::<Value>(&out).unwrap()["error"]["code"],
        2
    );
    let mut changed = bytes.clone();
    *changed.last_mut().unwrap() ^= 1;
    fs::write(&program, changed).unwrap();
    assert_eq!(
        eplyx(
            &root,
            &["migration", "gate", "--run", id, "--format", "json"]
        )
        .0,
        2
    );
    fs::write(program, bytes).unwrap();
    assert_eq!(fs::read(run.join("result/report.json")).unwrap(), original);
    let (code, out, _) = eplyx(&root, &["migration", "gate", "--format", "json"]);
    assert_eq!(code, 2);
    assert_eq!(
        serde_json::from_str::<Value>(&out).unwrap()["error"]["code"],
        2
    );
    #[cfg(unix)]
    {
        let other = tempfile::tempdir().unwrap();
        let private = other.path().join("private.json");
        fs::write(&private, b"{\"private\":\"EXTERNAL_FILE_SENTINEL\"}").unwrap();
        let report = run.join("result/report.json");
        fs::remove_file(&report).unwrap();
        std::os::unix::fs::symlink(&private, &report).unwrap();
        let (code, out, err) = eplyx(&root, &["show", id, "--format", "json"]);
        assert_eq!(code, 2);
        assert!(!out.contains("EXTERNAL_FILE_SENTINEL") && !err.contains("EXTERNAL_FILE_SENTINEL"));
        fs::remove_file(&report).unwrap();
        fs::write(report, &original).unwrap();
    }
    // Saved local history remains viewable after project configuration is removed.
    fs::remove_file(root.join("eplyx.toml")).unwrap();
    assert_eq!(eplyx(&root, &["runs", "--json"]).0, 0);
    assert_eq!(eplyx(&root, &["show", id]).0, 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn offline_worker_rejects_an_inherited_environment_and_store_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .args([
            "migration-worker",
            "--base",
            root.to_str().unwrap(),
            "--id",
            "run_test",
            "--action",
            "gate",
        ])
        .env_clear()
        .env("WORKER_SECRET_SENTINEL", "not-forwarded")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["data"]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("empty environment"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("not-forwarded"));
    #[cfg(unix)]
    {
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), root.join(".eplyx")).unwrap();
        assert_eq!(eplyx(&root, &["init", "--migration", "--fixture"]).0, 2);
        assert_eq!(fs::read_dir(other.path()).unwrap().count(), 0);
    }
}

#[test]
fn change_token_migration_uses_main_identity_and_activation() {
    let root = project("change", "eplyx_token_migration");
    let args = [
        "change",
        "token-migration",
        "--spec",
        "migration.json",
        "--mechanism",
        "target/deploy/migration.so",
        "--activation-slot",
        "2500",
        "--out",
        "change.json",
        "--format",
        "json",
    ];
    let (code, out, err) = eplyx(&root, &args);
    assert_eq!(code, 0, "{out}{err}");
    let change =
        eplyx_engine::change::ChangeSpec::parse(&fs::read(root.join("change.json")).unwrap())
            .unwrap();
    assert_eq!(change.activation.as_ref().unwrap().slot, Some(2500));
    let value: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["change"]["kind"], "token_migration");
    assert!(value["change"].get("activation").is_none());
    assert!(change
        .resolve(eplyx_engine::change::CandidateSource::Bytes(
            &fs::read(root.join("target/deploy/migration.so")).unwrap()
        ))
        .is_ok());
    assert_eq!(
        eplyx(&root, &args).0,
        2,
        "existing proposal is never overwritten"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_markdown_replays_without_changing_analytical_bytes() {
    use eplyx_engine::migration::report;
    let root = project("presentation-version", "eplyx_token_migration");
    let (code, out, err) = eplyx(&root, &["migration", "analyse", "--format", "json"]);
    assert_eq!(code, 0, "{out}{err}");
    let id = latest_run(&root);
    let result = root.join(".eplyx/runs").join(&id).join("result");
    let bytes = fs::read(result.join("report.json")).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    let current = fs::read_to_string(result.join("report.md")).unwrap();
    assert_eq!(current, report::markdown(&value));
    assert!(current.contains("**Passed with warnings**"));
    let legacy = report::markdown_version(&value, 1);
    assert!(legacy.contains("**PASS WITH WARNINGS**"));
    assert_ne!(current, legacy);
    let path = result.join("bindings.json");
    let mut binding: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    binding
        .as_object_mut()
        .unwrap()
        .remove("report_presentation_version");
    fs::write(path, serde_json::to_vec(&binding).unwrap()).unwrap();
    fs::write(result.join("report.md"), legacy).unwrap();
    let (code, out, err) = eplyx(
        &root,
        &["migration", "gate", "--run", &id, "--format", "json"],
    );
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(fs::read(result.join("report.json")).unwrap(), bytes);
    fs::write(result.join("report.md"), current).unwrap();
    let (code, _, _) = eplyx(
        &root,
        &["migration", "gate", "--run", &id, "--format", "json"],
    );
    assert_ne!(
        code, 0,
        "a substituted presentation cannot pass exact replay"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_pair_cli_uses_saved_world_and_portable_offline_reproduction() {
    let root = project("order", "eplyx_token_migration");
    // This example normally rehearses at a future activation. The order fixture
    // explicitly declares activation at the already pinned world Clock.
    let terms_path = root.join("migration.json");
    let mut terms: Value = serde_json::from_slice(&fs::read(&terms_path).unwrap()).unwrap();
    terms["window"]["activation"]["value"] = "1000".into();
    fs::write(&terms_path, serde_json::to_vec_pretty(&terms).unwrap()).unwrap();
    let (code, out, err) = eplyx(&root, &["migration", "analyse"]);
    assert_eq!(code, 0, "{out}{err}");
    let run = latest_run(&root);
    let plan: Value = serde_json::from_slice(
        &fs::read(
            root.join(".eplyx/runs")
                .join(&run)
                .join("result/migration.plan.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let sources: Vec<&str> = plan["units"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|u| u["class"] == "Migratable")
        .map(|u| u["source_account"].as_str().unwrap())
        .take(2)
        .collect();
    assert_eq!(sources.len(), 2);
    let out = root.join("order-case");
    let (code, stdout, err) = eplyx(
        &root,
        &[
            "migration",
            "order",
            "--run",
            &run,
            "--source-a",
            sources[0],
            "--source-b",
            sources[1],
            "--out",
            out.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert_eq!(code, 0, "{stdout}{err}");
    let analysis: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(analysis["change_spec_id"], plan["change_spec_id"]);
    assert!(out.join("case.json").is_file());
    let detached = tempfile::tempdir().unwrap();
    let (code, stdout, err) = eplyx(
        detached.path(),
        &[
            "migration",
            "reproduce-order",
            out.to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert_eq!(code, 0, "{stdout}{err}");
    assert_eq!(analysis, serde_json::from_str::<Value>(&stdout).unwrap());
    let (code, stdout, err) = eplyx(
        &root,
        &[
            "migration",
            "order",
            "--run",
            &run,
            "--source-a",
            sources[0],
            "--source-b",
            sources[0],
            "--out",
            root.join("invalid-order").to_str().unwrap(),
            "--format",
            "json",
        ],
    );
    assert_eq!(code, 2, "{stdout}{err}");
    assert_eq!(
        serde_json::from_str::<Value>(&stdout).unwrap()["error"]["kind"],
        "UnsupportedComposition"
    );
    fs::remove_dir_all(root).unwrap();
}
