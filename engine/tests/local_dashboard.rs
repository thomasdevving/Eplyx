//! Milestone 16 local dashboard: loopback API over a copied `.eplyx/` fixture.
use eplyx_engine::{
    dashboard::{store::Store, Dashboard},
    replay::hash_bytes as sha256,
    repo_root,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

#[allow(dead_code)]
mod ids {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dashboard/ids.rs"
    ));
}
use ids::{DERIVED, HEALTHY, HEALTHY_NO_SEARCH, SECOND_RUN, UNDERFUNDED, UNDERFUNDED_EARLY};

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

fn project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "eplyx-dashboard-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    copy_tree(&repo_root().join("fixtures/dashboard").join(name), &root);
    // Git does not keep empty directories; the CLI always creates these.
    for child in ["runs", "counterexamples", "reproductions", "cache"] {
        fs::create_dir_all(root.join(".eplyx").join(child)).unwrap();
    }
    root.canonicalize().unwrap()
}

fn serve(root: &Path) -> SocketAddr {
    let dashboard = Dashboard::bind(root, Some(0), json!({"version": "test"})).unwrap();
    let address = dashboard.address();
    std::thread::spawn(move || dashboard.serve());
    address
}

fn raw_request(address: SocketAddr, request: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let text = String::from_utf8_lossy(&response).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map_or(String::new(), |(_, b)| b.to_owned());
    (status, body)
}

fn get_raw(address: SocketAddr, path: &str) -> (u16, String) {
    raw_request(
        address,
        &format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            address.port()
        ),
    )
}

fn get(address: SocketAddr, path: &str) -> Value {
    let (status, body) = get_raw(address, path);
    assert_eq!(status, 200, "{path}: {body}");
    serde_json::from_str(&body).unwrap()
}

fn store_digest(root: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, path: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                walk(base, &entry.path(), out);
            } else if kind.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(relative, sha256(&fs::read(entry.path()).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    let base = root.join(".eplyx");
    for child in ["runs", "counterexamples", "reproductions"] {
        walk(&base, &base.join(child), &mut out);
    }
    out.insert(
        "project.json".into(),
        sha256(&fs::read(base.join("project.json")).unwrap()),
    );
    out
}

#[test]
fn binds_loopback_and_lists_runs_newest_first() {
    let root = project("transition-acceptance");
    let address = serve(&root);
    assert!(address.ip().is_loopback());
    let runs = get(address, "/api/runs")["runs"].clone();
    let ids: Vec<&str> = runs
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [UNDERFUNDED, HEALTHY, UNDERFUNDED_EARLY, HEALTHY_NO_SEARCH]
    );
    let numbers: Vec<u64> = runs
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, [4, 3, 2, 1]);
    assert_eq!(
        get(address, "/api/runs")["runs"],
        runs,
        "ordering is deterministic"
    );
    let latest = &runs[0];
    assert_eq!(latest["gate"]["outcome"], "Block");
    assert_eq!(latest["readiness"]["funding"], "Blocked");
    assert_eq!(latest["stress"]["deviating"], 1);
    assert_eq!(latest["search"]["observed"], 1);
    assert_eq!(latest["search"]["derived"], 1);
    assert_eq!(latest["saved_counterexamples"]["total"], 2);
    assert_eq!(runs[1]["search"]["total"], 0);
    assert!(
        runs[3]["search"].is_null(),
        "no search is not zero counterexamples"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_endpoint_reports_local_usage_without_rpc() {
    let root = project("transition-acceptance");
    let address = serve(&root);
    let project = get(address, "/api/project");
    assert_eq!(project["project"]["name"], "transition-acceptance");
    let stats = &project["stats"];
    assert_eq!(stats["runs"], 4);
    assert_eq!(stats["preflights"], 4);
    assert_eq!(stats["searches"], 3);
    assert_eq!(stats["counterexamples_saved"], 4);
    assert_eq!(stats["blocked"], 2);
    assert_eq!(stats["warned"], 2);
    assert_eq!(
        stats["offline_reproductions"], 2,
        "recorded by eplyx migration reproduce"
    );
    assert_eq!(stats["reproductions_succeeded"], 2);
    assert_eq!(stats["reproductions_failed"], 0);
    assert!(stats["latest_reproduction"]
        .as_str()
        .unwrap()
        .ends_with('Z'));
    assert_eq!(
        stats["run_sources"]["local"], 4,
        "copied from recorded metadata"
    );
    assert_eq!(project["latest"]["id"], UNDERFUNDED);
    assert_eq!(project["store"]["path"], ".eplyx/");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn run_detail_copies_engine_results_and_reuses_the_engine_gate() {
    let root = project("transition-acceptance");
    let address = serve(&root);
    for id in [HEALTHY, UNDERFUNDED] {
        let detail = get(address, &format!("/api/runs/{id}"));
        let report: Value = serde_json::from_slice(
            &fs::read(root.join(format!(".eplyx/runs/{id}/result/report.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(
            detail["invariant_results"], report["invariants"],
            "invariants are rendered, not recomputed"
        );
        assert_eq!(detail["gate_detail"]["saved"], report["deployment_gate"]);
        assert_eq!(detail["gate_detail"]["consistent_with_engine"], true);
        assert_eq!(detail["migration_detail"]["execution"], report["execution"]);
        for artifact in detail["evidence"]["artifacts"].as_array().unwrap() {
            assert!(
                !artifact["path"].as_str().unwrap().contains('\\'),
                "artifact paths use forward slashes"
            );
        }
    }
    let healthy = get(address, &format!("/api/runs/{HEALTHY}"));
    let policies = healthy["gate_detail"]["policies"].as_array().unwrap();
    assert_eq!(policies[0]["preflight"]["outcome"], "Warn");
    assert_eq!(policies[1]["policy"], "strict");
    assert_eq!(
        policies[1]["preflight"]["outcome"], "Block",
        "strict blocks Incomplete in the engine gate"
    );
    assert_eq!(healthy["search"]["total"], 0);
    let underfunded = get(address, &format!("/api/runs/{UNDERFUNDED}"));
    assert_eq!(
        underfunded["gate_detail"]["policies"][0]["with_search"]["outcome"],
        "Block"
    );
    assert_eq!(
        underfunded["search_detail"]["counterexamples"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(underfunded["search_detail"]["counterexamples"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["saved"] == true));
    assert_eq!(
        underfunded["migration_detail"]["reconciliation"]["available_reserve_raw"],
        "1000000"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn counterexamples_keep_observed_and_derived_distinct() {
    let root = project("transition-acceptance");
    let address = serve(&root);
    let list = get(address, "/api/counterexamples")["counterexamples"].clone();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 4);
    let latest: Vec<&Value> = list
        .iter()
        .filter(|c| c["parent_run"] == UNDERFUNDED)
        .collect();
    assert_eq!(latest.iter().filter(|c| c["kind"] == "Observed").count(), 1);
    assert_eq!(latest.iter().filter(|c| c["kind"] == "Derived").count(), 1);
    for c in list {
        assert_eq!(c["state"], "Valid", "{}", c["id"]);
        let claim = if c["kind"] == "Observed" {
            "Synthetic fixture state"
        } else {
            "Derived local state"
        };
        assert_eq!(c["claim"], claim);
    }
    let detail = get(address, &format!("/api/counterexamples/{DERIVED}"));
    assert_eq!(detail["kind"], "Derived");
    assert_eq!(detail["dimension"], "DeadlineBoundary");
    assert_eq!(detail["derived_value_raw"], "900000");
    assert_eq!(detail["boundary"]["first_deviating_value_raw"], "900000");
    assert_eq!(detail["minimized"], true);
    assert!(
        detail["failure"]["rollback_verified"].is_null(),
        "unexpected success is not a rejected transaction"
    );
    assert_eq!(detail["search_artifact_matches"], true);
    assert_eq!(
        detail["reproduce"],
        format!("eplyx migration reproduce {DERIVED}")
    );
    assert_eq!(
        detail["replay_inputs"]["search"],
        format!(".eplyx/runs/{UNDERFUNDED}/search")
    );
    let reproductions = &detail["reproductions"];
    assert_eq!(reproductions["count"], 2);
    assert_eq!(reproductions["last_outcome"], "Reproduced");
    assert_eq!(
        reproductions["history"][0]["failure_signature_matched"],
        true
    );
    assert_eq!(reproductions["history"][0]["no_rpc"], true);
    let unreproduced = list
        .iter()
        .find(|c| c["parent_run"] == UNDERFUNDED_EARLY)
        .unwrap();
    assert_eq!(unreproduced["reproductions"]["count"], 0);
    fs::remove_dir_all(root).unwrap();
}

fn statuses(comparison: &Value) -> BTreeMap<String, u64> {
    serde_json::from_value(comparison["counterexamples"]["counts"].clone()).unwrap()
}

#[test]
fn comparison_reports_declared_differences_without_claiming_resolution() {
    let root = project("transition-acceptance");
    let address = serve(&root);
    let forward = get(
        address,
        &format!("/api/compare?left={HEALTHY}&right={UNDERFUNDED}"),
    );
    let changed = |group: &str| -> Vec<String> {
        forward[group]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["changed"] == true)
            .map(|f| f["label"].as_str().unwrap().to_owned())
            .collect()
    };
    let inputs = changed("inputs");
    assert!(inputs.contains(&"Destination funding".to_owned()));
    assert!(
        inputs.contains(&"Candidate program hash".to_owned()),
        "the deficient-funding fixture also uses the pinned deadline defect"
    );
    assert!(!inputs.contains(&"Source asset".to_owned()));
    assert!(changed("results").contains(&"Migrated holders".to_owned()));
    assert_eq!(forward["gate"]["left"]["outcome"], "Warn");
    assert_eq!(forward["gate"]["right"]["outcome"], "Block");
    let cx = &forward["counterexamples"];
    assert_eq!(
        (cx["left_total"].as_u64(), cx["right_total"].as_u64()),
        (Some(0), Some(2))
    );
    assert_eq!(
        cx["comparable"], false,
        "different parent accounts and search domains"
    );
    assert_eq!(
        statuses(&forward),
        BTreeMap::from([("only_right".into(), 2)])
    );
    let invariant = forward["invariants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["invariant_type"] == "reserve_covers_eligible_holders")
        .unwrap();
    assert_eq!(
        (invariant["left"].as_str(), invariant["right"].as_str()),
        (Some("Satisfied"), Some("Violated"))
    );
    assert!(forward["causality"]
        .as_str()
        .unwrap()
        .contains("does not infer"));

    let backward = get(
        address,
        &format!("/api/compare?left={UNDERFUNDED}&right={HEALTHY}"),
    );
    let counts = statuses(&backward);
    assert!(
        !counts.contains_key("resolved"),
        "different search domains never claim resolution: {counts:?}"
    );
    assert_eq!(counts.values().sum::<u64>(), 2);
    assert!(
        counts.get("only_left").copied().unwrap_or(0) > 0,
        "absence does not imply the same state was re-executed"
    );
    let wording = serde_json::to_string(&backward["counterexamples"])
        .unwrap()
        .to_lowercase();
    assert!(!wording.contains("fixed"), "comparison never says fixed");

    let repeat = get(
        address,
        &format!("/api/compare?left={UNDERFUNDED_EARLY}&right={UNDERFUNDED}"),
    );
    assert_eq!(
        statuses(&repeat),
        BTreeMap::from([("persistent".into(), 2)])
    );

    let without_search = get(
        address,
        &format!("/api/compare?left={UNDERFUNDED}&right={HEALTHY_NO_SEARCH}"),
    );
    assert_eq!(
        statuses(&without_search),
        BTreeMap::from([("only_left".into(), 2)])
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn equivalent_search_conditions_can_report_resolution() {
    let root = project("transition-acceptance");
    // Deliberately synthetic view input: exercise the comparison contract without
    // claiming these edited bytes are an engine execution or rewriting a fixture.
    let base = root.join(".eplyx/runs");
    let path = base
        .join(UNDERFUNDED_EARLY)
        .join("search/migration-search.json");
    let mut passing: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    passing["counterexamples"] = json!([]);
    let probe = passing["trace"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["dimension"] == "DeadlineBoundary" && p["value_raw"] == "900000")
        .unwrap();
    probe["behaves_as_specified"] = json!(true);
    fs::write(&path, serde_json::to_vec(&passing).unwrap()).unwrap();
    let compare = || {
        eplyx_engine::dashboard::view::compare(
            &Store::open(&root).unwrap(),
            UNDERFUNDED,
            UNDERFUNDED_EARLY,
        )
        .unwrap()
    };
    let comparison = compare();
    assert_eq!(comparison["counterexamples"]["comparable"], true);
    assert_eq!(
        statuses(&comparison),
        BTreeMap::from([("resolved".into(), 1), ("only_left".into(), 1)])
    );
    for field in [
        "parent_source_account",
        "derived_world_sha256",
        "message_sha256",
        "kind",
        "value_raw",
    ] {
        let mut changed = passing.clone();
        if field == "parent_source_account" {
            changed[field] = json!("different-account");
        } else {
            for p in changed["trace"].as_array_mut().unwrap() {
                p[field] = json!("different");
            }
        }
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(
            !statuses(&compare()).contains_key("resolved"),
            "{field} cannot inherit a result"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn traversal_foreign_hosts_and_writes_are_rejected() {
    let root = project("transition-acceptance");
    let outside = root
        .parent()
        .unwrap()
        .join(format!("outside-secret-{}", std::process::id()));
    fs::write(&outside, "OUTSIDE-SECRET").unwrap();
    let address = serve(&root);
    for path in [
        "/api/runs/../project",
        "/api/runs/%2e%2e%2f%2e%2e%2fetc",
        "/api/runs/run_x%00",
        "/api/runs/..%5c..%5cwindows",
        &format!("/api/runs/{UNDERFUNDED}/artifacts/..%2fmetadata.json"),
        "/assets/../../Cargo.toml",
        "/counterexamples/..",
    ] {
        let (status, body) = get_raw(address, path);
        assert_eq!(status, 400, "{path}");
        assert!(!body.contains("OUTSIDE-SECRET"));
    }
    for path in [
        "/api/runs/run_missing",
        "/api/counterexamples/cx_missing",
        &format!("/api/runs/{UNDERFUNDED}/artifacts/program.so"),
        &format!("/api/runs/{UNDERFUNDED}/artifacts/population.capture.json"),
        "/etc/passwd",
        "/api/runs/RUN_UPPER",
    ] {
        assert_eq!(get_raw(address, path).0, 404, "{path}");
    }
    let (status, _) = raw_request(
        address,
        &format!(
            "GET /api/project HTTP/1.1\r\nHost: attacker.example:{}\r\n\r\n",
            address.port()
        ),
    );
    assert_eq!(status, 403, "DNS-rebinding hosts are refused");
    let (status, _) = raw_request(
        address,
        &format!(
            "POST /api/runs HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\n\r\n",
            address.port()
        ),
    );
    assert_eq!(status, 405);
    let (status, _) = raw_request(
        address,
        &format!(
            "DELETE /api/runs/{UNDERFUNDED} HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
            address.port()
        ),
    );
    assert_eq!(status, 405);
    fs::remove_file(outside).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_escape_the_store() {
    use std::os::unix::fs::symlink;
    let root = project("transition-acceptance");
    let outside = project("second-asset");
    let base = root.join(".eplyx");
    // A symlinked run directory is never listed or followed.
    symlink(
        outside.join(format!(".eplyx/runs/{SECOND_RUN}")),
        base.join("runs/run_20990101000000000_linked"),
    )
    .unwrap();
    // A symlinked member makes its run unreadable instead of being followed.
    let member = base
        .join("runs")
        .join(HEALTHY_NO_SEARCH)
        .join("result/report.json");
    fs::remove_file(&member).unwrap();
    symlink(
        outside.join(format!(".eplyx/runs/{SECOND_RUN}/result/report.json")),
        &member,
    )
    .unwrap();
    // A symlinked counterexample file is ignored.
    symlink(
        outside.join(".eplyx/project.json"),
        base.join("counterexamples/cx_linkedfile.json"),
    )
    .unwrap();
    let address = serve(&root);
    let runs = get(address, "/api/runs")["runs"].clone();
    assert!(runs
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["id"] != "run_20990101000000000_linked"));
    assert_eq!(
        get_raw(address, "/api/runs/run_20990101000000000_linked").0,
        404
    );
    let linked = runs
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == HEALTHY_NO_SEARCH)
        .unwrap();
    assert_eq!(linked["state"], "Unreadable");
    assert!(
        linked["population"]["token_accounts_observed"].is_null(),
        "no data read through the link"
    );
    assert_eq!(
        get_raw(
            address,
            &format!("/api/runs/{HEALTHY_NO_SEARCH}/artifacts/report.json")
        )
        .0,
        500
    );
    assert_eq!(
        get_raw(address, "/api/counterexamples/cx_linkedfile").0,
        404
    );
    assert!(
        get(address, "/api/project")["stats"]["ignored_store_entries"]
            .as_u64()
            .unwrap()
            >= 2
    );
    // A symlinked store root is refused outright.
    let linked_root = project("second-asset");
    fs::remove_dir_all(linked_root.join(".eplyx")).unwrap();
    symlink(&base, linked_root.join(".eplyx")).unwrap();
    assert!(Store::open(&linked_root).is_err());
    for path in [root, outside, linked_root] {
        fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn provider_origins_are_sanitized_and_no_secret_is_served() {
    let root = project("second-asset");
    let capture = root
        .join(".eplyx/runs")
        .join(SECOND_RUN)
        .join("result/wallet.capture.json");
    fs::write(&capture, br#"{"rpc_origin":"PROVIDER-SECRET-SENTINEL"}"#).unwrap();
    let address = serve(&root);
    for path in [
        "/api/project".to_owned(),
        "/api/runs".to_owned(),
        format!("/api/runs/{SECOND_RUN}"),
        "/api/counterexamples".to_owned(),
    ] {
        assert!(!get_raw(address, &path).1.contains("SECRET"), "{path}");
    }
    assert_eq!(
        get_raw(
            address,
            &format!("/api/runs/{SECOND_RUN}/artifacts/wallet.capture.json")
        )
        .0,
        404
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn serving_never_mutates_analytical_artifacts() {
    let root = project("transition-acceptance");
    let before = store_digest(&root);
    let address = serve(&root);
    for path in [
        "/".to_owned(),
        "/api/project".to_owned(),
        "/api/runs".to_owned(),
        "/api/counterexamples".to_owned(),
        format!("/api/runs/{UNDERFUNDED}"),
        format!("/api/runs/{UNDERFUNDED}/artifacts/report.json"),
        format!("/api/counterexamples/{DERIVED}"),
        format!("/api/counterexamples/{DERIVED}/raw"),
        format!("/api/compare?left={HEALTHY}&right={UNDERFUNDED}"),
    ] {
        assert_eq!(get_raw(address, &path).0, 200, "{path}");
    }
    assert_eq!(
        before,
        store_digest(&root),
        "runs, counterexamples and project.json are unchanged"
    );
    let index: Value =
        serde_json::from_slice(&fs::read(root.join(".eplyx/cache/dashboard-index.json")).unwrap())
            .unwrap();
    assert_eq!(
        index["version"],
        eplyx_engine::dashboard::store::INDEX_VERSION
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_or_corrupt_index_is_rebuilt_from_sources() {
    let root = project("transition-acceptance");
    let cache = root.join(".eplyx/cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("dashboard-index.json"), b"{not json").unwrap();
    let address = serve(&root);
    assert_eq!(
        get(address, "/api/runs")["runs"].as_array().unwrap().len(),
        4
    );
    let rebuilt: Value =
        serde_json::from_slice(&fs::read(cache.join("dashboard-index.json")).unwrap()).unwrap();
    assert_eq!(rebuilt["runs"].as_object().unwrap().len(), 4);
    // A cached summary is only a cache: a stale entry is replaced once its
    // source file changes, never trusted over the artifact.
    let mut tampered = rebuilt.clone();
    tampered["runs"][UNDERFUNDED]["summary"]["gate"]["outcome"] = json!("Pass");
    fs::write(
        cache.join("dashboard-index.json"),
        serde_json::to_vec(&tampered).unwrap(),
    )
    .unwrap();
    let metadata = root
        .join(".eplyx/runs")
        .join(UNDERFUNDED)
        .join("metadata.json");
    let bytes = fs::read(&metadata).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    fs::write(&metadata, &bytes).unwrap();
    let address = serve(&root);
    let runs = get(address, "/api/runs")["runs"].clone();
    assert_eq!(runs[0]["gate"]["outcome"], "Block");
    // A missing index is rebuilt too.
    fs::remove_file(cache.join("dashboard-index.json")).unwrap();
    let address = serve(&root);
    assert_eq!(
        get(address, "/api/runs")["runs"].as_array().unwrap().len(),
        4
    );
    assert!(cache.join("dashboard-index.json").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn empty_project_and_unfinished_runs_are_explicit() {
    let root = project("second-asset");
    let base = root.join(".eplyx");
    fs::remove_dir_all(base.join("runs")).unwrap();
    fs::create_dir(base.join("runs")).unwrap();
    let address = serve(&root);
    let project = get(address, "/api/project");
    assert_eq!(project["stats"]["runs"], 0);
    assert!(project["latest"].is_null());
    assert_eq!(
        get(address, "/api/counterexamples")["counterexamples"],
        json!([])
    );
    fs::create_dir_all(base.join("runs/run_20260101000000000_abcdef012345/package")).unwrap();
    let runs = get(address, "/api/runs")["runs"].clone();
    assert_eq!(runs[0]["state"], "Unfinished");
    assert!(
        get(address, "/api/project")["latest"].is_null(),
        "an unfinished run is not the latest result"
    );
    let (status, body) = get_raw(address, "/");
    assert_eq!(status, 200);
    assert!(body.contains("/assets/dashboard.js"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hundred_runs_and_hundreds_of_counterexamples_stay_responsive() {
    let root = project("transition-acceptance");
    let base = root.join(".eplyx");
    let template = base.join("runs").join(HEALTHY);
    for n in 0..100 {
        let id = format!("run_20270101{n:09}_ce7b4d55310c");
        copy_tree(&template, &base.join("runs").join(&id));
        let path = base.join("runs").join(&id).join("metadata.json");
        let mut meta: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        meta["run_id"] = json!(id);
        fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
    }
    let saved: Vec<PathBuf> = fs::read_dir(base.join("counterexamples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    for (n, path) in saved.iter().cycle().take(300).enumerate() {
        fs::copy(
            path,
            base.join("counterexamples")
                .join(format!("cx_perf{n:05}.json")),
        )
        .unwrap();
    }
    let cold = Instant::now();
    let address = serve(&root);
    let cold = cold.elapsed();
    let warm = Instant::now();
    let runs = get(address, "/api/runs");
    let listed = get(address, "/api/counterexamples");
    let project = get(address, "/api/project");
    let warm = warm.elapsed();
    assert_eq!(runs["runs"].as_array().unwrap().len(), 104);
    assert_eq!(listed["counterexamples"].as_array().unwrap().len(), 304);
    assert_eq!(project["stats"]["runs"], 104);
    assert!(
        cold < Duration::from_secs(20),
        "cold index build took {cold:?}"
    );
    assert!(
        warm < Duration::from_secs(3),
        "warm summaries took {warm:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_dashboard_starts_without_config_or_rpc_and_hides_the_secret() {
    let root = project("transition-acceptance");
    assert!(!root.join("eplyx.toml").exists());
    let mut child = Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .arg("--config")
        .arg(root.join("eplyx.toml"))
        .args(["dashboard", "--no-open", "--port", "0"])
        .env_clear()
        .env("SOLANA_RPC_URL", "CLI-RPC-SECRET-NOT-AN-ENDPOINT")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut banner = String::new();
    let mut buffer = [0u8; 512];
    let deadline = Instant::now() + Duration::from_secs(30);
    while !banner.contains("Press Ctrl+C") && Instant::now() < deadline {
        let read = stdout.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        banner.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    assert!(banner.contains("Eplyx dashboard"), "{banner}");
    assert!(banner.contains("Project: transition-acceptance"));
    assert!(banner.contains("Runs: 4"));
    assert!(banner.contains("Counterexamples: 4"));
    let url = banner
        .lines()
        .find(|l| l.starts_with("http://127.0.0.1:"))
        .unwrap();
    let address: SocketAddr = url.trim_start_matches("http://").parse().unwrap();
    assert!(address.ip().is_loopback());
    let project = get(address, "/api/project");
    assert_eq!(project["context"]["config"]["state"], "Missing");
    for path in ["/api/project", "/api/runs", "/api/counterexamples"] {
        assert!(!get_raw(address, path).1.contains("CLI-RPC-SECRET"));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_dashboard_requires_a_local_store() {
    let root = std::env::temp_dir().join(format!("eplyx-dashboard-empty-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_eplyx"))
        .arg("--config")
        .arg(root.join("eplyx.toml"))
        .args(["dashboard", "--no-open", "--port", "0"])
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("eplyx init"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn run_source_and_reproduction_records_are_copied_not_inferred() {
    let root = project("transition-acceptance");
    let base = root.join(".eplyx");
    let path = base.join("runs").join(HEALTHY).join("metadata.json");
    let mut meta: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    meta["schema_version"] = json!(2);
    meta["run_source"] = json!("ci");
    fs::write(&path, serde_json::to_vec(&meta).unwrap()).unwrap();
    // Legacy source absence remains explicit, even when another run records CI.
    let legacy = base.join("runs").join(UNDERFUNDED).join("metadata.json");
    let mut legacy_meta: Value = serde_json::from_slice(&fs::read(&legacy).unwrap()).unwrap();
    legacy_meta.as_object_mut().unwrap().remove("run_source");
    fs::write(legacy, serde_json::to_vec(&legacy_meta).unwrap()).unwrap();
    // A record whose ID differs from its file name, and garbage, are not counted.
    let records = base.join("reproductions");
    let first = fs::read_dir(&records)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::copy(&first, records.join("repro_20990101000000000_renamed.json")).unwrap();
    fs::write(
        records.join("repro_20990101000000001_garbage.json"),
        b"not json",
    )
    .unwrap();
    let address = serve(&root);
    let runs = get(address, "/api/runs")["runs"].clone();
    let source = |id: &str| {
        runs.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()["run_source"]
            .clone()
    };
    assert_eq!(source(HEALTHY), "ci");
    assert!(source(UNDERFUNDED).is_null());
    let stats = get(address, "/api/project")["stats"].clone();
    assert_eq!(stats["offline_reproductions"], 2);
    assert_eq!(stats["run_sources"]["ci"], 1);
    let comparison = get(
        address,
        &format!("/api/compare?left={HEALTHY}&right={UNDERFUNDED}"),
    );
    let field = comparison["git"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["label"] == "Run source")
        .unwrap()
        .clone();
    assert_eq!(field["left"], "ci");
    assert_eq!(field["changed"], true);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn analytical_kinds_keep_descriptors_separate_and_never_invent_a_gate() {
    let root = project("analytical-kinds");
    let address = serve(&root);
    let runs = get(address, "/api/runs")["runs"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(runs.len(), 2);
    for row in &runs {
        assert_eq!(row["state"], "Complete", "{row:#}");
        assert!(row["gate"]["outcome"].is_null());
        assert!(row.get("candidate_program_sha256").is_none());
        let id = row["id"].as_str().unwrap();
        let detail = get(address, &format!("/api/runs/{id}"));
        let report: Value = serde_json::from_slice(
            &fs::read(root.join(format!(".eplyx/runs/{id}/result/report.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(detail["analysis"]["report"], report);
        if row["kind"] == "lifecycle_change" {
            assert_eq!(report["impact"]["after"]["lifecycle_status"], "Expired");
            assert_eq!(
                report["impact"]["entities"][0]["execution_status"],
                "NotTested"
            );
            assert_eq!(
                detail["analysis"]["change_spec"]["change"]["kind"],
                "lifecycle_change"
            );
        } else {
            assert_eq!(row["kind"], "current_observation");
            assert_eq!(report["execution_performed"], false);
            assert_eq!(report["paths"].as_array().unwrap().len(), 5);
            assert!(report["paths"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["status"] == "NotTested"));
        }
        let file = root.join(format!(".eplyx/runs/{id}/result/report.json"));
        fs::write(&file, b"{}").unwrap();
        let changed = get(address, &format!("/api/runs/{id}"));
        assert_eq!(changed["state"], "Unreadable");
        assert!(changed["gate"]["outcome"].is_null());
    }
    let left = runs[0]["id"].as_str().unwrap();
    let right = runs[1]["id"].as_str().unwrap();
    assert_ne!(
        get_raw(address, &format!("/api/compare?left={left}&right={right}")).0,
        200
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bindings_and_metadata_mismatches_never_render_as_complete() {
    for field in ["kind", "schema_version", "change_spec_id", "gate_policy"] {
        let root = project("transition-acceptance");
        let file = root.join(format!(".eplyx/runs/{HEALTHY}/result/bindings.json"));
        let mut binding: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        binding[field] = json!("substituted");
        fs::write(file, serde_json::to_vec(&binding).unwrap()).unwrap();
        let store = Store::open(&root).unwrap();
        assert_eq!(
            eplyx_engine::dashboard::view::run_summary(&store, HEALTHY)["state"],
            "Unreadable",
            "{field}"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
