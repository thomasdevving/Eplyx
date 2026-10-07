//! MAIN-specific storage split, restart and legacy authorization regressions.
mod cloud_common;
use cloud_common::*;
use eplyx_server::{
    cloud::workspaces::ProjectIntent,
    project::{Project, ProjectToken},
};
use serde_json::json;

#[test]
fn pending_project_creation_is_recoverable_and_completed_intents_never_restore_authorization() {
    let server = Server::start();
    let (browser, account) = server.signup("owner@example.com");
    let workspace = account["workspace_id"].as_str().unwrap();
    let user = account["user"]["id"].as_str().unwrap();
    let id = eplyx_server::ids::project();
    let intent = ProjectIntent {
        project: Project::analytical(&id, "recover me").unwrap(),
        workspace_id: workspace.into(),
        user_id: user.into(),
        request_key: "stable-request".into(),
        completed: false,
    };
    let path = server
        .state
        .registry
        .storage()
        .project_dir(&id)
        .unwrap()
        .join("creation-intent.json");
    server
        .state
        .registry
        .storage()
        .write_json(&path, &intent)
        .unwrap();
    assert!(
        server.state.registry.load_project(&id).is_err(),
        "crash happened before canonical project creation"
    );
    server.recover_projects();
    assert_eq!(
        server.state.registry.load_project(&id).unwrap().name,
        "recover me"
    );
    let saved: ProjectIntent = server.state.registry.storage().read_json(&path).unwrap();
    assert!(saved.completed);
    let request = json!({"name":"recover me","request_key":"stable-request"});
    for _ in 0..2 {
        let (code, body) = status(
            browser
                .post(server.url(&format!("/v1/workspaces/{workspace}/projects")))
                .json(&request)
                .send()
                .unwrap(),
        );
        assert_eq!(code, 201, "{body}");
        assert_eq!(body["project"]["id"], id);
    }
    assert_eq!(server.state.registry.list_projects().unwrap().len(), 1);
    let changed = json!({"name":"different","request_key":"stable-request"});
    assert_eq!(
        browser
            .post(server.url(&format!("/v1/workspaces/{workspace}/projects")))
            .json(&changed)
            .send()
            .unwrap()
            .status(),
        409
    );
    server
        .sql(&format!(
            "DELETE FROM project_workspaces WHERE project_id='{id}'"
        ))
        .unwrap();
    server.recover_projects();
    assert_eq!(
        browser
            .get(server.url(&format!("/v1/projects/{id}/workspace")))
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        browser
            .post(server.url(&format!("/v1/workspaces/{workspace}/projects")))
            .json(&request)
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(server.state.registry.list_projects().unwrap().len(), 1);
}

#[test]
fn legacy_projects_require_operator_assignment_and_both_token_stores_share_owner_revocation() {
    let server = Server::start();
    let (browser, account) = server.signup("owner@example.com");
    let workspace = account["workspace_id"].as_str().unwrap();
    let user = account["user"]["id"].as_str().unwrap();
    let id = eplyx_server::ids::project();
    server
        .state
        .registry
        .create_project(&Project::analytical(&id, "legacy").unwrap())
        .unwrap();
    let secret = eplyx_server::project::generate_token();
    let token_id = eplyx_server::ids::token();
    server
        .state
        .registry
        .create_token(&ProjectToken::new(&token_id, &id, "old CI", &secret).unwrap())
        .unwrap();
    let legacy = bearer(&secret);
    let operator = bearer("test-operator");
    let url = server.url(&format!("/v1/projects/{id}/workspace-binding"));
    let request = json!({"workspace_id":workspace,"owner_user_id":user});
    assert_eq!(
        browser
            .get(server.url(&format!("/v1/projects/{id}")))
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        browser.post(&url).json(&request).send().unwrap().status(),
        401
    );
    assert_eq!(
        legacy.post(&url).json(&request).send().unwrap().status(),
        401
    );
    assert_eq!(
        operator.post(&url).json(&request).send().unwrap().status(),
        200
    );
    assert_eq!(
        operator.post(&url).json(&request).send().unwrap().status(),
        200,
        "assignment is idempotent"
    );
    assert_eq!(
        browser
            .get(server.url(&format!("/v1/projects/{id}")))
            .send()
            .unwrap()
            .status(),
        200
    );
    let tokens = server.url(&format!("/v1/projects/{id}/tokens"));
    let (_, created) = status(
        browser
            .post(&tokens)
            .json(&json!({"label":"new CI"}))
            .send()
            .unwrap(),
    );
    let pg_secret = created["token"].as_str().unwrap();
    let pg_id = created["id"].as_str().unwrap();
    for client in [&operator, &browser] {
        let (_, list) = status(client.get(&tokens).send().unwrap());
        assert_eq!(list["tokens"].as_array().unwrap().len(), 2);
        assert!(!list.to_string().contains(&secret));
        assert!(!list.to_string().contains(pg_secret));
    }
    assert_eq!(
        legacy
            .get(server.url(&format!("/v1/projects/{id}")))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        browser
            .delete(format!("{tokens}/{token_id}"))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        legacy
            .get(server.url(&format!("/v1/projects/{id}")))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        operator
            .delete(format!("{tokens}/{pg_id}"))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        bearer(pg_secret)
            .get(server.url(&format!("/v1/projects/{id}")))
            .send()
            .unwrap()
            .status(),
        401
    );
    // Neither generation of CI token can move the baseline or activate a bundle.
    let active_secret = eplyx_server::project::generate_token();
    let active_id = eplyx_server::ids::token();
    server
        .state
        .registry
        .create_token(&ProjectToken::new(&active_id, &id, "CI", &active_secret).unwrap())
        .unwrap();
    assert_eq!(
        bearer(&active_secret)
            .post(server.url(&format!(
                "/v1/projects/{id}/bundles/bundle_unknown/activate"
            )))
            .send()
            .unwrap()
            .status(),
        404
    );
}

/// Backing up the volume and Postgres together, and restoring them together,
/// gives back the same service: sign-in, workspace access, the project token
/// and the byte-identical report. Restoring a database newer than the volume
/// is detected and named rather than served.
#[test]
fn files_and_postgres_restore_together_and_a_mismatched_pair_is_refused() {
    use std::process::Command;
    const PROGRAM: &str = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";
    let script = eplyx_engine::repo_root().join("scripts/eplyx-backup.sh");
    let server_binary = env!("CARGO_BIN_EXE_eplyx-server");
    let scratch = tempfile::tempdir().unwrap();

    // ---- a service with real history
    let original = Server::start();
    let (browser, account) = original.signup("restore-owner@example.com");
    let workspace = account["workspace_id"].as_str().unwrap().to_owned();
    let project = original.create_project(&browser, &workspace, "Restore me");
    let configured = Project::new(
        &project,
        "Restore me",
        PROGRAM,
        eplyx_server::project::AdapterId::for_program(PROGRAM),
    )
    .unwrap();
    let storage = original.state.registry.storage();
    storage
        .write_json(
            &storage.project_dir(&project).unwrap().join("project.json"),
            &configured,
        )
        .unwrap();
    let registered = original
        .state
        .registry
        .register_bundle(&configured, std::path::Path::new("../deploy/bundle"), None)
        .unwrap();
    original
        .state
        .registry
        .activate_bundle(&project, &registered.bundle_id)
        .unwrap();
    let (code, issued) = status(
        browser
            .post(original.url(&format!("/v1/projects/{project}/tokens")))
            .json(&json!({"label": "ci"}))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 201, "{issued}");
    let ci_token = issued["token"].as_str().unwrap().to_owned();
    let candidate = std::fs::read("../deploy/bundle/binaries/current.so").unwrap();
    let mut body = b"--restore-boundary\r\nContent-Disposition: form-data; name=\"candidate\"; filename=\"candidate.so\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
    body.extend(candidate);
    body.extend_from_slice(b"\r\n--restore-boundary--\r\n");
    let (code, accepted) = status(
        bearer(&ci_token)
            .post(original.url(&format!("/v1/projects/{project}/checks")))
            .header(
                "content-type",
                "multipart/form-data; boundary=restore-boundary",
            )
            .body(body)
            .send()
            .unwrap(),
    );
    assert_eq!(code, 202, "{accepted}");
    let run_id = accepted["run_id"].as_str().unwrap().to_owned();
    for _ in 0..600 {
        if original
            .state
            .registry
            .load_run(&run_id)
            .unwrap()
            .status
            .is_terminal()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let report = bearer(&ci_token)
        .get(original.url(&format!("/v1/runs/{run_id}/report.json")))
        .send()
        .unwrap();
    assert_eq!(report.status(), 200);
    let report = report.bytes().unwrap();

    // ---- back up both halves
    let backup = scratch.path().join("backup");
    let out = Command::new("bash")
        .arg(&script)
        .args(["backup", "--data-dir"])
        .arg(original.volume())
        .args(["--database-url", &original.database_url(), "--out"])
        .arg(&backup)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The world moves on after the backup: a second project, mapped in Postgres.
    let later = original.create_project(&browser, &workspace, "After the backup");
    let later_dump = scratch.path().join("later.dump");
    let dumped = Command::new("pg_dump")
        .args(["--format=custom", "--no-owner", "--no-privileges", "--file"])
        .arg(&later_dump)
        .arg(original.database_url())
        .status()
        .unwrap();
    assert!(dumped.success());

    // ---- restore both into fresh stores
    let volume = tempfile::tempdir().unwrap();
    let restored_database = create_database();
    let out = Command::new("bash")
        .arg(&script)
        .args(["restore", "--from"])
        .arg(&backup)
        .arg("--data-dir")
        .arg(volume.path())
        .args(["--database-url", &database_url(&restored_database)])
        .env("EPLYX_SERVER", server_binary)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let check: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(check["problems"], json!([]));
    assert_eq!(check["identity_projects"], 1);
    assert_eq!(check["reports_checked"], 1);

    // A restore never runs over the store it would replace.
    let refused = Command::new("bash")
        .arg(&script)
        .args(["restore", "--from"])
        .arg(&backup)
        .arg("--data-dir")
        .arg(volume.path())
        .args(["--database-url", &database_url(&restored_database)])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));

    let restored = Server::start_over(volume, restored_database, |_| {});
    let session = restored.browser();
    let login = session
        .post(restored.url("/v1/auth/login"))
        .json(&json!({"email": "restore-owner@example.com", "password": "correct horse battery"}))
        .send()
        .unwrap();
    assert_eq!(login.status(), 200);
    let (code, listed) = status(session.get(restored.url("/v1/projects")).send().unwrap());
    assert_eq!(code, 200, "{listed}");
    let listed = listed.to_string();
    assert!(listed.contains(&project), "{listed}");
    assert!(!listed.contains(&later), "{listed}");
    let again = bearer(&ci_token)
        .get(restored.url(&format!("/v1/runs/{run_id}/report.json")))
        .send()
        .unwrap();
    assert_eq!(again.status(), 200);
    assert_eq!(
        again.bytes().unwrap(),
        report,
        "the report survives byte for byte"
    );
    let (code, setup) = status(
        bearer(&ci_token)
            .get(restored.url(&format!("/v1/projects/{project}/setup")))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 200, "{setup}");
    assert_eq!(setup["ready_for_first_check"], true);

    // ---- a database newer than the volume is named, not served
    let mismatched_volume = tempfile::tempdir().unwrap();
    let untar = Command::new("tar")
        .arg("-C")
        .arg(mismatched_volume.path())
        .arg("-xf")
        .arg(backup.join("volume.tar"))
        .status()
        .unwrap();
    assert!(untar.success());
    let newer = create_database();
    let loaded = Command::new("pg_restore")
        .args([
            "--no-owner",
            "--no-privileges",
            "--exit-on-error",
            "--dbname",
        ])
        .arg(database_url(&newer))
        .arg(&later_dump)
        .status()
        .unwrap();
    assert!(loaded.success());
    let verdict = Command::new(server_binary)
        .args(["admin", "verify-volume"])
        .env("EPLYX_DATA_DIR", mismatched_volume.path())
        .env("EPLYX_DATABASE_URL", database_url(&newer))
        .output()
        .unwrap();
    drop_database(&newer);
    assert_eq!(verdict.status.code(), Some(1));
    let verdict: serde_json::Value = serde_json::from_slice(&verdict.stdout).unwrap();
    assert!(
        verdict["problems"].to_string().contains(&later),
        "{verdict}"
    );
}
