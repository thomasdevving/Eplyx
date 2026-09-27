//! Pinned STA sync assertions over fresh MAIN dashboard fixtures: what a
//! document may carry, how it binds to exact digests, and that the local
//! dashboard shows (never changes) cloud link and sync state.
use eplyx_engine::{
    cloud::{
        contract::{self, Artifact},
        local::{self, CloudLink, RunSyncState, SyncStatus},
    },
    dashboard::{store::Store, view, Dashboard},
    local_store::SavedMigrationCounterexample,
    repo_root,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const HEALTHY: &str = "run_20260926202758877209000_e5db6948abca";
const UNDERFUNDED: &str = "run_20260926202807178818000_c43467360081";
const RESERVE_BOUNDARY: &str = "cx_7749391abe584a4c5a4e31b9";
const REPRO: &str = "repro_20260926202825424374000_7749391abe584a4c5a4e31b9";
const LOCAL_ID: &str = "local_6013395f55d1cf819125";

static NEXT: AtomicU64 = AtomicU64::new(0);

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

fn project() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "eplyx-cloud-contract-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    copy_tree(
        &repo_root().join("fixtures/dashboard/transition-acceptance"),
        &root,
    );
    for child in ["runs", "counterexamples", "reproductions", "cache"] {
        fs::create_dir_all(root.join(".eplyx").join(child)).unwrap();
    }
    root.canonicalize().unwrap()
}

fn resign(artifact: &mut Artifact, text: String) {
    *artifact = Artifact::new(text.into_bytes()).unwrap();
}

#[test]
fn every_fixture_run_builds_a_verified_document_with_stable_identity() {
    let root = project();
    let store = Store::open(&root).unwrap();
    for run in store.run_ids().unwrap().0 {
        let document = contract::run_document(&store, LOCAL_ID, &run).unwrap();
        let verified = document.verify().unwrap();
        let again = contract::run_document(&store, LOCAL_ID, &run).unwrap();
        assert_eq!(verified.core_sha256, again.core_sha256().unwrap());
        // The engine summary from synced bytes equals the local summary.
        let local = view::run_summary(&store, &run);
        assert_eq!(verified.summary, local, "{run}");
        let text = serde_json::to_string(&document).unwrap();
        assert!(!text.contains("rpc_origin"), "wallet capture never syncs");
        assert!(!text.contains(root.to_string_lossy().as_ref()));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn candidate_binary_and_captures_contribute_sizes_only() {
    let root = project();
    let package = root
        .join(".eplyx/runs")
        .join(HEALTHY)
        .join("input/programs");
    fs::create_dir_all(&package).unwrap();
    let program: Vec<u8> = (0..4096u32).map(|i| (i * 7 % 251) as u8).collect();
    fs::write(package.join("program.so"), &program).unwrap();
    let marker = "population-capture-marker-bytes";
    fs::write(
        root.join(".eplyx/runs")
            .join(HEALTHY)
            .join("input/world.json"),
        format!("{{\"marker\":\"{marker}\"}}"),
    )
    .unwrap();
    let store = Store::open(&root).unwrap();
    let document = contract::run_document(&store, LOCAL_ID, HEALTHY).unwrap();
    let text = serde_json::to_string(&document).unwrap();
    assert!(!text.contains(marker));
    let hex: String = program[..32].iter().map(|b| format!("{b:02x}")).collect();
    assert!(!text.contains(&hex));
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&program[..48]);
    assert!(!text.contains(&b64));
    assert!(document.local_artifact_sizes["world.json"].is_some());
    assert!(!view::ARTIFACTS
        .iter()
        .any(|(name, ..)| name.ends_with(".so")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tampered_or_leaky_run_documents_are_refused() {
    let root = project();
    let store = Store::open(&root).unwrap();
    let base = contract::run_document(&store, LOCAL_ID, UNDERFUNDED).unwrap();

    let mut digest = base.clone();
    digest.report.sha256 = "0".repeat(64);
    assert!(format!("{:#}", digest.verify().err().unwrap()).contains("SHA-256"));

    let mut path = base.clone();
    let text = path.report.text.replacen(
        "\"adapter\":",
        "\"note\":\"/Users/dev/project/.eplyx\",\"adapter\":",
        1,
    );
    resign(&mut path.report, text);
    assert!(format!("{:#}", path.verify().err().unwrap()).contains("absolute local path"));

    let mut url = base.clone();
    let text = url.report.text.replacen(
        "\"adapter\":",
        "\"note\":\"https://provider.invalid/?api-key=example\",\"adapter\":",
        1,
    );
    resign(&mut url.report, text);
    assert!(format!("{:#}", url.verify().err().unwrap()).contains("URL"));

    let mut package = base.clone();
    let text = package.state_input.as_ref().unwrap().text.replace(
        "\"max_rehearsal_units\": 1024",
        "\"max_rehearsal_units\": 1023",
    );
    let mut state: Value = serde_json::from_str(&text).unwrap();
    state["max_rehearsal_units"] = json!(1);
    resign(
        package.state_input.as_mut().unwrap(),
        serde_json::to_string(&state).unwrap(),
    );
    assert!(
        package.verify().is_err(),
        "state descriptor bound in MAIN analytical input"
    );

    let mut renamed = base.clone();
    renamed.run_id = HEALTHY.into();
    assert!(renamed.verify().is_err());

    let mut gate = base.clone();
    let text = gate.metadata.text.replace("\"Block\"", "\"Pass\"");
    resign(&mut gate.metadata, text);
    assert!(
        gate.verify().is_err(),
        "metadata gate must equal the report"
    );

    let mut schema = base.clone();
    schema.schema = "eplyx.cloud.run.v0".into();
    assert!(schema.verify().is_err());

    let mut unknown: Value = serde_json::to_value(&base).unwrap();
    unknown["extra"] = json!(1);
    assert!(serde_json::from_value::<contract::RunDocument>(unknown).is_err());

    let mut oversized = base.clone();
    resign(&mut oversized.report, "x".repeat(5 * 1024 * 1024));
    assert!(format!("{:#}", oversized.verify().err().unwrap()).contains("bound"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn counterexamples_and_reproductions_bind_to_their_exact_parents() {
    let root = project();
    let store = Store::open(&root).unwrap();
    let parent = contract::run_document(&store, LOCAL_ID, UNDERFUNDED)
        .unwrap()
        .verify()
        .unwrap();
    let other = contract::run_document(&store, LOCAL_ID, HEALTHY)
        .unwrap()
        .verify()
        .unwrap();
    let document = contract::counterexample_document(&store, LOCAL_ID, RESERVE_BOUNDARY).unwrap();
    let verified = document.verify().unwrap();
    assert_eq!(verified.summary["kind"], "Derived");
    contract::bind_counterexample(&verified.saved, &parent.run).unwrap();
    assert!(contract::bind_counterexample(&verified.saved, &other.run).is_err());

    let mut tampered = document.clone();
    let mut saved: Value = serde_json::from_str(&tampered.file.text).unwrap();
    saved["counterexample"]["derived_value_raw"] = json!("1");
    resign(&mut tampered.file, serde_json::to_string(&saved).unwrap());
    assert!(format!("{:#}", tampered.verify().err().unwrap()).contains("identity"));

    let mut moved = verified.saved.clone();
    moved.search_sha256 = "f".repeat(64);
    assert!(contract::bind_counterexample(&moved, &parent.run).is_err());

    let reproduction = contract::reproduction_document(&store, LOCAL_ID, REPRO, &[]).unwrap();
    let record = reproduction.verify().unwrap();
    contract::bind_reproduction(&record, &verified.saved).unwrap();
    let mut forged = record.clone();
    forged.binding.as_mut().unwrap().world_sha256 = "0".repeat(64);
    assert!(contract::bind_reproduction(&forged, &verified.saved).is_err());
    let mut forged = record.clone();
    forged.no_rpc = false;
    assert!(contract::bind_reproduction(&forged, &verified.saved).is_err());
    let mut forged = record.clone();
    forged.binding = None;
    assert!(contract::bind_reproduction(&forged, &verified.saved).is_err());
    let other_cx: SavedMigrationCounterexample = serde_json::from_slice(
        &fs::read(root.join(".eplyx/counterexamples/cx_44068b83b0774f8be4d959a4.json")).unwrap(),
    )
    .unwrap();
    assert!(contract::bind_reproduction(&record, &other_cx).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reproduction_errors_with_local_paths_are_refused_without_rewriting() {
    let root = project();
    let file = root
        .join(".eplyx/reproductions")
        .join(format!("{REPRO}.json"));
    let mut record: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    record["error"] = json!(format!("missing {}/.eplyx/runs/x", root.display()));
    let raw = serde_json::to_vec_pretty(&record).unwrap();
    fs::write(&file, &raw).unwrap();
    let store = Store::open(&root).unwrap();
    assert!(contract::reproduction_document(&store, LOCAL_ID, REPRO, &[]).is_err());
    let (code, stdout, stderr) = eplyx(
        &root,
        &root.join("credentials"),
        &["sync", "--dry-run"],
        &[],
    );
    assert_eq!(
        code, 2,
        "a leaky selected reproduction must fail the whole plan"
    );
    assert!(!stdout.contains("skipped"));
    assert!(stderr.contains("absolute local path"));
    assert_eq!(
        fs::read(&file).unwrap(),
        raw,
        "leaky history stays exact and local under the approved MAIN contract"
    );
    fs::remove_dir_all(root).unwrap();
}

fn get(address: std::net::SocketAddr, path: &str) -> Value {
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
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[test]
fn local_dashboard_shows_link_and_sync_state_read_only() {
    let root = project();
    let base = root.join(".eplyx");
    let before = fs::read(base.join("runs").join(HEALTHY).join("result/report.json")).unwrap();
    let serve = |root: &Path| {
        let dashboard = Dashboard::bind(root, Some(0), json!({"version": "test"})).unwrap();
        let address = dashboard.address();
        std::thread::spawn(move || dashboard.serve());
        address
    };
    let address = serve(&root);
    assert_eq!(get(address, "/api/project")["cloud"]["linked"], false);
    assert!(get(address, "/api/runs")["runs"][0]["sync"].is_null());
    local::write_link(
        &base,
        Some(&CloudLink {
            server: "https://cloud.example".into(),
            workspace_id: "ws_0123456789abcdef0123".into(),
            project_id: "proj_01JQZ3K5R8V2X7M9N4T6W1Y0BC".into(),
            linked_at: "2026-09-25T00:00:00Z".into(),
        }),
    )
    .unwrap();
    local::write_state(
        &base,
        &RunSyncState {
            schema_version: 1,
            run_id: HEALTHY.into(),
            server: "https://cloud.example".into(),
            cloud_project_id: "proj_01JQZ3K5R8V2X7M9N4T6W1Y0BC".into(),
            status: SyncStatus::Synced,
            core_sha256: None,
            search_sha256: None,
            counterexamples_synced: 0,
            reproductions_synced: 0,
            last_attempt_at: "2026-09-25T00:00:00Z".into(),
            last_synced_at: Some("2026-09-25T00:00:00Z".into()),
            error: None,
            url: None,
        },
    )
    .unwrap();
    let project = get(address, "/api/project");
    assert_eq!(project["cloud"]["linked"], true);
    assert_eq!(
        project["cloud"]["project_id"],
        "proj_01JQZ3K5R8V2X7M9N4T6W1Y0BC"
    );
    assert_eq!(project["cloud"]["synced_runs"], 1);
    assert_eq!(project["project"]["id"], LOCAL_ID, "local ID is kept");
    let detail = get(address, &format!("/api/runs/{HEALTHY}"));
    assert_eq!(detail["sync"]["status"], "synced");
    // The dashboard still refuses every write, including to cloud state.
    let mut stream = TcpStream::connect(address).unwrap();
    write!(stream, "POST /api/project HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", address.port()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 405"), "{response}");
    assert_eq!(
        before,
        fs::read(base.join("runs").join(HEALTHY).join("result/report.json")).unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}

fn eplyx(root: &Path, config: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_eplyx"));
    command.env_clear();
    command
        .current_dir(root)
        .args(args)
        .env("EPLYX_CONFIG_DIR", config);
    for name in [
        "EPLYX_TOKEN",
        "EPLYX_PROJECT_ID",
        "EPLYX_URL",
        "SOLANA_RPC_URL",
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

#[test]
fn cli_sync_fails_clearly_without_link_or_login_and_dry_run_sends_nothing() {
    let root = project();
    let config = root.join("config-home");
    let before = fs::read(
        root.join(".eplyx/runs")
            .join(UNDERFUNDED)
            .join("result/report.json"),
    )
    .unwrap();
    let (code, _, err) = eplyx(&root, &config, &["sync"], &[]);
    assert_eq!(code, 2);
    assert!(err.contains("not linked"), "{err}");
    let (code, _, err) = eplyx(
        &root,
        &config,
        &["sync"],
        &[
            ("EPLYX_PROJECT_ID", "proj_01JQZ3K5R8V2X7M9N4T6W1Y0BC"),
            ("EPLYX_URL", "http://127.0.0.1:9"),
        ],
    );
    assert_eq!(code, 2);
    assert!(err.contains("not signed in"), "{err}");
    let secret = "provider-sentinel-unused";
    let token = "eplyx_u_dry-run-token-value-123";
    let (code, out, _) = eplyx(
        &root,
        &config,
        &["sync", "--dry-run", "--json"],
        &[("SOLANA_RPC_URL", secret), ("EPLYX_TOKEN", token)],
    );
    assert_eq!(code, 0);
    assert!(!out.contains(secret) && !out.contains(token));
    let documents: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(documents.as_array().unwrap().len(), 4);
    let (code, out, _) = eplyx(&root, &config, &["sync", "--dry-run"], &[]);
    assert_eq!(code, 0);
    assert!(out.contains("never uploads source code"), "{out}");
    // Offline: an unreachable server is a clear retryable error; nothing local changes.
    local::write_link(
        &root.join(".eplyx"),
        Some(&CloudLink {
            server: "http://127.0.0.1:9".into(),
            workspace_id: "ws_0123456789abcdef0123".into(),
            project_id: "proj_01JQZ3K5R8V2X7M9N4T6W1Y0BC".into(),
            linked_at: "2026-09-25T00:00:00Z".into(),
        }),
    )
    .unwrap();
    let (code, _, err) = eplyx(
        &root,
        &config,
        &["sync", "--latest"],
        &[("EPLYX_TOKEN", token)],
    );
    assert_eq!(code, 2);
    assert!(
        err.contains("could not reach") && err.contains("retry"),
        "{err}"
    );
    let state = local::read_state(&root.join(".eplyx"), UNDERFUNDED)
        .unwrap()
        .unwrap();
    assert_eq!(state.status, SyncStatus::Failed);
    assert!(state.last_synced_at.is_none());
    assert_eq!(
        before,
        fs::read(
            root.join(".eplyx/runs")
                .join(UNDERFUNDED)
                .join("result/report.json")
        )
        .unwrap()
    );
    // No token or secret lands anywhere in the local store.
    let mut stack = vec![root.join(".eplyx")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let text = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
                assert!(
                    !text.contains(token) && !text.contains(secret),
                    "{}",
                    path.display()
                );
            }
        }
    }
    // Local commands keep working with a token present and no network.
    let (code, out, _) = eplyx(
        &root,
        &config,
        &["runs", "--json"],
        &[("EPLYX_TOKEN", token)],
    );
    assert_eq!(code, 0);
    assert!(out.contains(UNDERFUNDED));
    fs::remove_dir_all(root).unwrap();
}
