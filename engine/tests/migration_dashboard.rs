//! Token Migration V1 runs in the read-only dashboard and the optional sync
//! contract: the same store, views that only select engine fields, the engine gate
//! evaluator for policy views, and documents that pass the privacy scanner.
use eplyx_engine::dashboard::Dashboard;
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

fn eplyx(root: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_eplyx"));
    command
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("EPLYX_CONFIG_DIR", root.join("config-home"));
    for name in [
        "SOLANA_RPC_URL",
        "EPLYX_TOKEN",
        "EPLYX_PROJECT_ID",
        "EPLYX_CLOUD_URL",
        "CI",
    ] {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command.output().unwrap();
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

fn install(root: &Path, candidate: &str) {
    fs::copy(
        eplyx_engine::repo_root().join(format!("artifacts/{candidate}.so")),
        root.join("target/deploy/migration.so"),
    )
    .unwrap();
}

fn project() -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("eplyx-migration-dashboard-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    copy_tree(
        &eplyx_engine::repo_root().join("examples/migrations/minimal"),
        &root,
    );
    fs::create_dir_all(root.join("target/deploy")).unwrap();
    root.canonicalize().unwrap()
}

fn get(address: SocketAddr, path: &str) -> Value {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        address.port()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{path}: {response}");
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

fn runs(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(root.join(".eplyx/runs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    ids
}

#[test]
fn migration_runs_are_displayed_and_compared() {
    let root = project();
    install(&root, "eplyx_token_migration");
    let (code, out, err) = eplyx(&root, &["migration", "analyse"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    install(&root, "eplyx_token_migration_defect_deadline_inclusive");
    let (code, out, err) = eplyx(&root, &["migration", "search"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    let ids = runs(&root);
    assert_eq!(ids.len(), 2);

    let dashboard = Dashboard::bind(&root, Some(0), json!({"version": "test"})).unwrap();
    let address = dashboard.address();
    std::thread::spawn(move || dashboard.serve());
    let listed = get(address, "/api/runs");
    let rows = listed["runs"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row["kind"], "token_migration");
        assert_eq!(row["state"], "Complete", "{row:#}");
        assert_eq!(row["official_transition"], "NotTested");
    }
    let defect = &ids[1];
    let detail = get(address, &format!("/api/runs/{defect}"));
    assert_eq!(detail["answers"].as_array().unwrap().len(), 8);
    assert!(detail["migration_detail"]["reconciliation"]["equations"].is_array());
    assert_eq!(
        detail["gate_detail"]["consistent_with_engine"], true,
        "{:#}",
        detail["gate_detail"]
    );
    assert_eq!(
        detail["gate_detail"]["policies"][1]["preflight"]["outcome"],
        "Block"
    );
    assert_eq!(
        detail["gate_detail"]["policies"][0]["with_search"]["outcome"], "Block",
        "a counterexample tightens the gate"
    );
    assert!(detail["search_detail"]["counterexamples"][0]["saved"] == true);
    let counterexamples = get(address, "/api/counterexamples");
    let files = counterexamples["counterexamples"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["state"], "Valid", "{:#}", files[0]);
    assert_eq!(files[0]["kind"], "Derived");
    let cx = files[0]["id"].as_str().unwrap();
    let cx_detail = get(address, &format!("/api/counterexamples/{cx}"));
    assert_eq!(cx_detail["search_artifact_matches"], true);
    assert!(cx_detail["reproduction_inputs"]["mutations"].is_array());
    let comparison = get(
        address,
        &format!("/api/compare?left={}&right={}", ids[0], ids[1]),
    );
    assert_eq!(comparison["inputs"][0]["label"], "Candidate program hash");
    assert_eq!(comparison["inputs"][0]["changed"], true);
    assert_eq!(comparison["kind"], "token_migration");

    // The pinned source sync assertions are retained in T9's contract suite.
    fs::remove_dir_all(root).unwrap();
}
