//! Milestone 18 CLI against a real test server: `eplyx login` (device flow
//! approved by a browser session), `link`, `sync`, retries, CI environment
//! auth, conflicts and `logout`, with local workflows unaffected throughout.
mod cloud_common;

use cloud_common::*;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

/// The `eplyx` binary from this workspace, built once for the test run.
fn eplyx_bin() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let status = Command::new(env!("CARGO"))
            .args([
                "build",
                "--locked",
                "-q",
                "-p",
                "eplyx-engine",
                "--bin",
                "eplyx",
            ])
            .status()
            .unwrap();
        assert!(status.success(), "could not build the eplyx CLI");
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../target"));
        let bin = target
            .join("debug")
            .join(format!("eplyx{}", std::env::consts::EXE_SUFFIX));
        assert!(bin.is_file(), "missing {}", bin.display());
        bin
    })
}

fn cli(root: &Path, home: &Path) -> Command {
    let mut command = Command::new(eplyx_bin());
    command.current_dir(root).env("EPLYX_CONFIG_DIR", home);
    for name in [
        "EPLYX_TOKEN",
        "EPLYX_PROJECT_ID",
        "EPLYX_URL",
        "SOLANA_RPC_URL",
        "CI",
    ] {
        command.env_remove(name);
    }
    command
}

fn run(mut command: Command) -> (i32, String, String) {
    let output = command.output().unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn login(server: &Server, browser: &reqwest::blocking::Client, root: &Path, home: &Path) {
    let mut child = cli(root, home)
        .args(["login", "--server", &server.base, "--no-open"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut code = None;
    let mut transcript = String::new();
    for line in (&mut reader).lines() {
        let line = line.unwrap();
        transcript.push_str(&line);
        transcript.push('\n');
        if let Some(value) = line.strip_prefix("Code: ") {
            code = Some(value.trim().to_owned());
            break;
        }
    }
    let code = code.unwrap_or_else(|| panic!("no code in {transcript}"));
    assert!(
        !transcript.to_lowercase().contains("password:"),
        "the CLI never prompts for a password"
    );
    let approved = browser
        .post(server.url("/v1/auth/device/approve"))
        .json(&json!({"user_code": code, "approve": true}))
        .send()
        .unwrap();
    assert_eq!(approved.status(), 200);
    let mut rest = String::new();
    for line in reader.lines() {
        rest.push_str(&line.unwrap());
        rest.push('\n');
    }
    assert!(child.wait().unwrap().success(), "{rest}");
    assert!(rest.contains("Signed in as alice@example.com"), "{rest}");
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn stored_token(home: &Path) -> String {
    let credentials: Value =
        serde_json::from_slice(&fs::read(home.join("credentials.json")).unwrap()).unwrap();
    credentials["servers"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn login_link_sync_retry_and_logout_keep_local_workflows_intact() {
    let server = Server::start();
    let (browser, _) = server.signup("alice@example.com");
    let root = fixture_project();
    let home = root.join("home");
    login(&server, &browser, &root, &home);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(home.join("credentials.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let token = stored_token(&home);
    // Browsers cannot relink; the CLI can. Non-interactive link without a choice fails clearly.
    let mut command = cli(&root, &home);
    command.arg("link").stdin(Stdio::null());
    let (code, _, err) = run(command);
    assert_eq!(code, 2);
    assert!(
        err.contains("--project") && err.contains("--create"),
        "{err}"
    );
    let mut command = cli(&root, &home);
    command.args(["link", "--create", "transition-acceptance"]);
    let (code, out, err) = run(command);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Nothing was uploaded"));
    let project: Value =
        serde_json::from_slice(&fs::read(root.join(".eplyx/project.json")).unwrap()).unwrap();
    assert_eq!(
        project["local_id"], LOCAL_ID,
        "the stable local project ID is kept"
    );
    let cloud_project = project["cloud"]["project_id"].as_str().unwrap().to_owned();
    assert_ne!(cloud_project, LOCAL_ID);
    let before: Vec<(PathBuf, Vec<u8>)> = {
        let mut files = Vec::new();
        walk(&root.join(".eplyx/runs"), &mut files);
        files
            .into_iter()
            .map(|p| {
                let b = fs::read(&p).unwrap();
                (p, b)
            })
            .collect()
    };
    let mut command = cli(&root, &home);
    command.arg("sync").env(
        "SOLANA_RPC_URL",
        format!("{}://provider.invalid/cli-secret-key", "https"),
    );
    let (code, out, err) = run(command);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("✓ Run metadata (uploaded)")
            && out.contains("✓ 2 counterexamples")
            && out.contains("✓ 2 reproduction records"),
        "{out}"
    );
    assert!(out.contains(&format!("{}/p/{cloud_project}/runs/", server.base)));
    // Retrying is safe: identical content is a no-op.
    let (code, out, _) = run({
        let mut c = cli(&root, &home);
        c.arg("sync");
        c
    });
    assert_eq!(code, 0);
    assert_eq!(out.matches("already synced, identical").count(), 4, "{out}");
    let (_, runs) = status(
        browser
            .get(server.url(&format!("/v1/projects/{cloud_project}/view/runs")))
            .send()
            .unwrap(),
    );
    assert_eq!(runs["runs"].as_array().unwrap().len(), 4);
    let everything = serde_json::to_string(
        &status(
            browser
                .get(server.url(&format!(
                    "/v1/projects/{cloud_project}/view/runs/{UNDERFUNDED}"
                )))
                .send()
                .unwrap(),
        )
        .1,
    )
    .unwrap();
    assert!(!everything.contains("cli-secret-key") && !everything.contains(&token));
    // Canonical run artifacts are untouched; no token lands in the local store.
    for (path, bytes) in &before {
        assert_eq!(&fs::read(path).unwrap(), bytes, "{}", path.display());
    }
    let mut files = Vec::new();
    walk(&root.join(".eplyx"), &mut files);
    for file in files {
        assert!(
            !String::from_utf8_lossy(&fs::read(&file).unwrap()).contains(&token),
            "{}",
            file.display()
        );
    }
    let state: Value = serde_json::from_slice(
        &fs::read(root.join(format!(".eplyx/sync/runs/{UNDERFUNDED}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(state["status"], "synced");
    // Logout revokes the token on the server and removes it locally.
    let (code, out, _) = run({
        let mut c = cli(&root, &home);
        c.arg("logout");
        c
    });
    assert_eq!(code, 0);
    assert!(out.contains("revoked on the server"), "{out}");
    assert_eq!(
        bearer(&token)
            .get(server.url("/v1/workspaces"))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert!(!fs::read_to_string(home.join("credentials.json"))
        .unwrap()
        .contains(&token));
    let (code, _, err) = run({
        let mut c = cli(&root, &home);
        c.arg("sync");
        c
    });
    assert_eq!(code, 2);
    assert!(err.contains("not signed in"), "{err}");
    // Local workflows never needed the account.
    let (code, out, _) = run({
        let mut c = cli(&root, &home);
        c.args(["runs", "--json"]);
        c
    });
    assert_eq!(code, 0);
    assert!(out.contains(UNDERFUNDED));
    let (code, out, _) = run({
        let mut c = cli(&root, &home);
        c.args(["show", HEALTHY]);
        c
    });
    assert_eq!(code, 0, "{out}");
    fs::remove_dir_all(root).unwrap();
}

/// A copy of the healthy run recorded as a CI run (metadata schema 2).
fn ci_run(root: &Path, run_id: &str) {
    let runs = root.join(".eplyx/runs");
    for entry in fs::read_dir(&runs).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap() != HEALTHY {
            fs::remove_dir_all(path).unwrap();
        }
    }
    fs::rename(runs.join(HEALTHY), runs.join(run_id)).unwrap();
    let file = runs.join(run_id).join("metadata.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    metadata["schema_version"] = json!(2);
    metadata["run_id"] = json!(run_id);
    metadata["run_source"] = json!("ci");
    metadata["git_commit"] = json!("0e511ba6e1a5c0de0000000000000000000000aa");
    metadata["git_branch"] = json!("main");
    metadata["git_dirty"] = json!(false);
    fs::write(&file, serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
    for entry in fs::read_dir(root.join(".eplyx/counterexamples")).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    for entry in fs::read_dir(root.join(".eplyx/reproductions")).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let mut project: Value =
        serde_json::from_slice(&fs::read(root.join(".eplyx/project.json")).unwrap()).unwrap();
    project["local_id"] = json!("local_00000000000000c1c1c1");
    fs::write(
        root.join(".eplyx/project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
}

#[test]
fn ci_environment_sync_joins_local_history_and_conflicts_are_refused() {
    let server = Server::start();
    let (browser, account) = server.signup("alice@example.com");
    let project = server.create_project(
        &browser,
        account["workspace_id"].as_str().unwrap(),
        "shared",
    );
    let user = bearer(&server.device_token(&browser));
    let local = fixture_project();
    sync_all(&server, &user, &project, &documents(&local, LOCAL_ID));
    let (_, created) = status(
        browser
            .post(server.url(&format!("/v1/projects/{project}/tokens")))
            .json(&json!({"label":"ci"}))
            .send()
            .unwrap(),
    );
    let ci_token = created["token"].as_str().unwrap().to_owned();
    let ci_root = fixture_project();
    let ci_id = "run_20260927090000000_e5db6948abca";
    ci_run(&ci_root, ci_id);
    let home = ci_root.join("home");
    let env = |command: &mut Command, token: &str| {
        command
            .env("EPLYX_TOKEN", token)
            .env("EPLYX_PROJECT_ID", &project)
            .env("EPLYX_URL", &server.base)
            .env("CI", "true");
    };
    let mut command = cli(&ci_root, &home);
    command.args(["sync", "--latest"]);
    env(&mut command, &ci_token);
    let (code, out, err) = run(command);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        !home.join("credentials.json").exists(),
        "CI auth comes from the environment only"
    );
    let (_, runs) = status(
        browser
            .get(server.url(&format!("/v1/projects/{project}/view/runs")))
            .send()
            .unwrap(),
    );
    let runs = runs["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 5);
    assert_eq!(runs[0]["id"], ci_id);
    assert_eq!(runs[0]["run_source"], "ci");
    assert_eq!(runs[0]["synced"]["via"], "ci");
    assert!(
        runs[1..].iter().all(|r| r["run_source"] == "local"),
        "transport provenance identifies synced local runs without rewriting their metadata"
    );
    let (_, overview) = status(
        browser
            .get(server.url(&format!("/v1/projects/{project}/view/project")))
            .send()
            .unwrap(),
    );
    assert_eq!(overview["cloud"]["latest_ci"]["id"], ci_id);
    assert_eq!(overview["stats"]["run_sources"]["ci"], 1);
    // The same run ID from another local store with different bytes: conflict.
    let other = fixture_project();
    let mut project_json: Value =
        serde_json::from_slice(&fs::read(other.join(".eplyx/project.json")).unwrap()).unwrap();
    project_json["local_id"] = json!("local_0000000000000000abcd");
    fs::write(
        other.join(".eplyx/project.json"),
        serde_json::to_vec(&project_json).unwrap(),
    )
    .unwrap();
    let mut command = cli(&other, &other.join("home"));
    command.args(["sync", HEALTHY]);
    env(&mut command, &ci_token);
    let (code, _, err) = run(command);
    assert_eq!(code, 2);
    assert!(
        err.contains("conflict") && err.contains("immutable"),
        "{err}"
    );
    let state: Value = serde_json::from_slice(
        &fs::read(other.join(format!(".eplyx/sync/runs/{HEALTHY}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(state["status"], "failed");
    // A CI token for another project, or a revoked one, cannot sync here.
    let (_, created) = status(
        browser
            .post(server.url(&format!("/v1/projects/{project}/tokens")))
            .json(&json!({"label":"revoked"}))
            .send()
            .unwrap(),
    );
    let id = created["id"].as_str().unwrap();
    browser
        .delete(server.url(&format!("/v1/projects/{project}/tokens/{id}")))
        .send()
        .unwrap();
    let mut command = cli(&ci_root, &home);
    command.args(["sync", "--latest"]);
    env(&mut command, created["token"].as_str().unwrap());
    let (code, _, err) = run(command);
    assert_eq!(code, 2);
    assert!(err.contains("401"), "{err}");
    for dir in [local, ci_root, other] {
        fs::remove_dir_all(dir).unwrap();
    }
}
