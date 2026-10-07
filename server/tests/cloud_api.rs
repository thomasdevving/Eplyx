//! Milestone 18 hosted API: authentication, workspace isolation, strict sync
//! validation, immutability, idempotency, binding checks and view fidelity.
//! Requires EPLYX_CLOUD_TEST_DATABASE_URL. Missing storage is a failure.
mod cloud_common;

use cloud_common::*;
use eplyx_engine::{
    cloud::contract::Artifact,
    dashboard::{store::Store, view},
};
use serde_json::{json, Value};

fn workspace_of(signup: &Value) -> String {
    signup["workspace_id"].as_str().unwrap().to_owned()
}

#[test]
fn signup_and_login_release_the_database_connection_before_starting_a_session() {
    let server = Server::start();
    // A busy pool can leave just one connection available. Neither endpoint
    // should hold that connection while requesting another for the session.
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _busy_connections = runtime.block_on(async {
        let pool = &server.state.identity.as_ref().unwrap().db;
        let mut connections = Vec::new();
        for _ in 0..15 {
            connections.push(pool.get().await.unwrap());
        }
        connections
    });
    let browser = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    for (path, request, status) in [
        (
            "/v1/auth/signup",
            json!({"email":"pool@example.com","password":"correct horse battery","name":"Pool test"}),
            201,
        ),
        (
            "/v1/auth/login",
            json!({"email":"pool@example.com","password":"correct horse battery"}),
            200,
        ),
    ] {
        let response = browser
            .post(server.url(path))
            .header("origin", &server.base)
            .json(&request)
            .send()
            .unwrap();
        assert_eq!(response.status(), status, "{path}");
    }
}

#[test]
fn unauthenticated_and_forged_credentials_are_rejected() {
    let server = Server::start();
    let anonymous = reqwest::blocking::Client::new();
    let (browser, account) = server.signup("alice@example.com");
    let project = server.create_project(&browser, &workspace_of(&account), "demo");
    for path in [
        "/v1/workspaces".to_owned(),
        "/v1/auth/me".to_owned(),
        format!("/v1/projects/{project}/workspace"),
        format!("/v1/projects/{project}/view/project"),
        format!("/v1/projects/{project}/view/runs"),
        format!("/v1/projects/{project}/runs"),
    ] {
        assert_eq!(
            anonymous.get(server.url(&path)).send().unwrap().status(),
            401,
            "{path}"
        );
    }
    let root = fixture_project();
    let docs = documents(&root, LOCAL_ID);
    let (code, _) = status(
        anonymous
            .post(server.url(&format!("/v1/projects/{project}/sync/runs")))
            .json(&docs.runs[0])
            .send()
            .unwrap(),
    );
    assert_eq!(code, 401);
    let forged = bearer("eplyx_u_not-a-real-token");
    assert_eq!(
        forged
            .get(server.url("/v1/workspaces"))
            .send()
            .unwrap()
            .status(),
        401
    );
    // No run was stored by any rejected request.
    let (_, runs) = status(
        browser
            .get(server.url(&format!("/v1/projects/{project}/view/runs")))
            .send()
            .unwrap(),
    );
    assert_eq!(runs["runs"].as_array().unwrap().len(), 0);
    // Wrong password and unknown account look the same.
    let (a, x) = (
        status(
            server
                .browser()
                .post(server.url("/v1/auth/login"))
                .json(&json!({"email":"alice@example.com","password":"wrong password!"}))
                .send()
                .unwrap(),
        ),
        status(
            server
                .browser()
                .post(server.url("/v1/auth/login"))
                .json(&json!({"email":"nobody@example.com","password":"wrong password!"}))
                .send()
                .unwrap(),
        ),
    );
    assert_eq!((a.0, x.0), (401, 401));
    assert_eq!(a.1, x.1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn projects_are_private_to_their_workspace() {
    let server = Server::start();
    let (alice, account) = server.signup("alice@example.com");
    let (bob, _) = server.signup("bob@example.com");
    let workspace = workspace_of(&account);
    let project = server.create_project(&alice, &workspace, "private-project");
    let root = fixture_project();
    let docs = documents(&root, LOCAL_ID);
    let alice_token = bearer(&server.device_token(&alice));
    sync_all(&server, &alice_token, &project, &docs);
    let (_, detail) = status(
        alice
            .get(server.url(&format!("/v1/projects/{project}/workspace")))
            .send()
            .unwrap(),
    );
    assert_eq!(detail["project"]["visibility"], "workspace");
    assert_eq!(detail["project"]["demo"], false);
    let bob_token = bearer(&server.device_token(&bob));
    for client in [&bob, &bob_token] {
        for path in [
            format!("/v1/projects/{project}/workspace"),
            format!("/v1/projects/{project}/view/project"),
            format!("/v1/projects/{project}/view/runs/{UNDERFUNDED}"),
            format!("/v1/projects/{project}/view/counterexamples/{RESERVE_BOUNDARY}"),
            format!("/v1/projects/{project}/view/compare?left={HEALTHY}&right={UNDERFUNDED}"),
            format!("/v1/workspaces/{workspace}/members"),
        ] {
            let code = client.get(server.url(&path)).send().unwrap().status();
            assert!(code == 404, "{path} answered {code}");
        }
    }
    let (code, _) = status(
        bob_token
            .post(server.url(&format!("/v1/projects/{project}/sync/runs")))
            .json(&docs.runs[0])
            .send()
            .unwrap(),
    );
    assert_eq!(code, 404, "no sync into another workspace");
    let (_, list) = status(bob.get(server.url("/v1/workspaces")).send().unwrap());
    assert!(!list.to_string().contains(&project));
    // No public demo unless the operator names one.
    assert_eq!(
        reqwest::blocking::get(server.url("/v1/demo/view/project"))
            .unwrap()
            .status(),
        404
    );
    // Membership grants access; removal takes it away.
    let (code, _) = status(
        alice
            .post(server.url(&format!("/v1/workspaces/{workspace}/members")))
            .json(&json!({"email":"bob@example.com"}))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 200);
    assert_eq!(
        bob.get(server.url(&format!("/v1/projects/{project}/view/project")))
            .send()
            .unwrap()
            .status(),
        200
    );
    let (_, members) = status(
        alice
            .get(server.url(&format!("/v1/workspaces/{workspace}/members")))
            .send()
            .unwrap(),
    );
    let bob_id = members["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["email"] == "bob@example.com")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // A member cannot manage the workspace.
    let (code, _) = status(
        bob.post(server.url(&format!("/v1/workspaces/{workspace}/members")))
            .json(&json!({"email":"alice@example.com"}))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 403);
    assert_eq!(
        alice
            .delete(server.url(&format!("/v1/workspaces/{workspace}/members/{bob_id}")))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        bob.get(server.url(&format!("/v1/projects/{project}/view/project")))
            .send()
            .unwrap()
            .status(),
        404
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sync_is_idempotent_immutable_and_rejects_conflicts() {
    let server = Server::start();
    let (alice, account) = server.signup("alice@example.com");
    let project = server.create_project(&alice, &workspace_of(&account), "p");
    let token = bearer(&server.device_token(&alice));
    let root = fixture_project();
    let docs = documents(&root, LOCAL_ID);
    let runs = server.url(&format!("/v1/projects/{project}/sync/runs"));
    let underfunded = docs
        .runs
        .iter()
        .find(|r| r.run_id == UNDERFUNDED)
        .unwrap()
        .clone();
    // Preflight synced before its search: the search attaches exactly once.
    let mut before_search = underfunded.clone();
    before_search.search = None;
    assert_eq!(
        status(token.post(&runs).json(&before_search).send().unwrap()).1["status"],
        "created"
    );
    assert_eq!(
        status(token.post(&runs).json(&before_search).send().unwrap()).1["status"],
        "unchanged"
    );
    let (code, body) = status(token.post(&runs).json(&underfunded).send().unwrap());
    assert_eq!(
        (code, body["status"].as_str()),
        (200, Some("search_attached"))
    );
    assert_eq!(
        status(token.post(&runs).json(&underfunded).send().unwrap()).1["status"],
        "unchanged"
    );
    // Same run ID, different bound digest: conflict, never an overwrite.
    let mut other_search = underfunded.clone();
    // Different exact bytes with identical analytical values still conflict.
    let text = format!("{}\n", underfunded.search.as_ref().unwrap().text);
    other_search.search = Some(Artifact::new(text.into_bytes()).unwrap());
    let (code, body) = status(token.post(&runs).json(&other_search).send().unwrap());
    assert_eq!(code, 409, "{body}");
    let mut other_project = underfunded.clone();
    other_project.local_project_id = "local_0000000000000000abcd".into();
    let (code, body) = status(token.post(&runs).json(&other_project).send().unwrap());
    assert_eq!(code, 409, "{body}");
    assert!(body["error"].as_str().unwrap().contains("immutable"));
    // Exactly one run row, still bound to the original digest.
    let (_, listed) = status(
        alice
            .get(server.url(&format!("/v1/projects/{project}/view/runs")))
            .send()
            .unwrap(),
    );
    assert_eq!(listed["runs"].as_array().unwrap().len(), 1);
    assert_eq!(
        listed["runs"][0]["search"]["sha256"].as_str(),
        underfunded.search.as_ref().map(|s| s.sha256.as_str())
    );
    // MAIN owns immutable analytical bytes in its filesystem registry and CAS.
    // Postgres has no analytical table whose mutation could change a result.
    let metadata = server
        .state
        .registry
        .source_run(&project, UNDERFUNDED)
        .unwrap()
        .unwrap();
    let retained = server
        .state
        .registry
        .analytical_document(&metadata)
        .unwrap();
    assert_eq!(retained.report, underfunded.report);
    assert_eq!(retained.search, underfunded.search);
    assert!(server.sql("SELECT * FROM runs").is_err());
    assert!(server.sql("SELECT * FROM reports").is_err());
    assert_eq!(
        server
            .state
            .registry
            .project_run_ids(&project)
            .unwrap()
            .len(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_leaky_and_oversized_documents_are_rejected() {
    let server = Server::start();
    let (alice, account) = server.signup("alice@example.com");
    let project = server.create_project(&alice, &workspace_of(&account), "p");
    let token = bearer(&server.device_token(&alice));
    let root = fixture_project();
    let docs = documents(&root, LOCAL_ID);
    let runs = server.url(&format!("/v1/projects/{project}/sync/runs"));
    let post = |body: String| {
        status(
            token
                .post(&runs)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .unwrap(),
        )
    };
    assert_eq!(post("{not json".into()).0, 422);
    assert_eq!(
        post(json!({"schema": "eplyx.cloud.run.v1"}).to_string()).0,
        422
    );
    let mut extra = serde_json::to_value(&docs.runs[0]).unwrap();
    extra["uploaded_program"] = json!("AAAA");
    assert_eq!(post(extra.to_string()).0, 422, "unknown fields are refused");
    let mut digest = docs.runs[0].clone();
    digest.report.sha256 = "0".repeat(64);
    assert_eq!(post(serde_json::to_string(&digest).unwrap()).0, 422);
    // A document carrying an RPC URL is refused and not stored.
    let mut leaky = docs.runs[0].clone();
    let mut report: Value = serde_json::from_str(&leaky.report.text).unwrap();
    report["provider"] = json!(format!("{}://provider.invalid/example", "https"));
    leaky.report = Artifact::new(serde_json::to_vec(&report).unwrap()).unwrap();
    let (code, body) = post(serde_json::to_string(&leaky).unwrap());
    assert_eq!(code, 422);
    assert!(body["error"].as_str().unwrap().contains("URL"), "{body}");
    let mut path = docs.runs[0].clone();
    path.metadata = Artifact::new(
        path.metadata
            .text
            .replacen(
                "\"eplyx_version\"",
                "\"home\": \"/home/runner/work\", \"eplyx_version\"",
                1,
            )
            .into_bytes(),
    )
    .unwrap();
    assert_eq!(post(serde_json::to_string(&path).unwrap()).0, 422);
    // Beyond the body bound: 413 before parsing.
    let huge = format!("{{\"pad\":\"{}\"}}", "x".repeat(13 * 1024 * 1024));
    assert_eq!(post(huge).0, 413);
    let small = server.url(&format!("/v1/projects/{project}/sync/reproductions"));
    let (code, _) = status(
        token
            .post(&small)
            .header("content-type", "application/json")
            .body("x".repeat(70 * 1024))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 413);
    let (_, listed) = status(
        alice
            .get(server.url(&format!("/v1/projects/{project}/view/runs")))
            .send()
            .unwrap(),
    );
    assert_eq!(listed["runs"].as_array().unwrap().len(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn counterexamples_and_reproductions_require_their_synced_parents() {
    let server = Server::start();
    let (alice, account) = server.signup("alice@example.com");
    let project = server.create_project(&alice, &workspace_of(&account), "p");
    let token = bearer(&server.device_token(&alice));
    let root = fixture_project();
    let docs = documents(&root, LOCAL_ID);
    let url = |kind: &str| server.url(&format!("/v1/projects/{project}/sync/{kind}"));
    let boundary = docs
        .counterexamples
        .iter()
        .find(|c| c.counterexample_id == RESERVE_BOUNDARY)
        .unwrap();
    let repro = docs
        .reproductions
        .iter()
        .find(|r| r.counterexample_id == RESERVE_BOUNDARY)
        .unwrap();
    let (code, body) = status(
        token
            .post(url("counterexamples"))
            .json(boundary)
            .send()
            .unwrap(),
    );
    assert_eq!(code, 422, "no parent run yet: {body}");
    let (code, _) = status(token.post(url("reproductions")).json(repro).send().unwrap());
    assert_eq!(code, 422, "no counterexample yet");
    // A run synced without its search cannot accept its counterexamples.
    let mut run = docs
        .runs
        .iter()
        .find(|r| r.run_id == UNDERFUNDED)
        .unwrap()
        .clone();
    let search = run.search.take();
    status(token.post(url("runs")).json(&run).send().unwrap());
    let (code, body) = status(
        token
            .post(url("counterexamples"))
            .json(boundary)
            .send()
            .unwrap(),
    );
    assert_eq!(code, 422, "{body}");
    run.search = search;
    status(token.post(url("runs")).json(&run).send().unwrap());
    // Bound to a different local project than its parent run.
    let mut foreign = boundary.clone();
    foreign.local_project_id = "local_0000000000000000abcd".into();
    let (code, body) = status(
        token
            .post(url("counterexamples"))
            .json(&foreign)
            .send()
            .unwrap(),
    );
    assert_eq!(code, 422, "{body}");
    assert_eq!(
        status(
            token
                .post(url("counterexamples"))
                .json(boundary)
                .send()
                .unwrap()
        )
        .1["status"],
        "created"
    );
    assert_eq!(
        status(
            token
                .post(url("counterexamples"))
                .json(boundary)
                .send()
                .unwrap()
        )
        .1["status"],
        "unchanged"
    );
    let mut foreign = repro.clone();
    foreign.local_project_id = "local_0000000000000000abcd".into();
    assert_eq!(
        status(
            token
                .post(url("reproductions"))
                .json(&foreign)
                .send()
                .unwrap()
        )
        .0,
        422
    );
    // A reproduction naming the right counterexample but another run is refused.
    let mut wrong: Value = serde_json::from_str(&repro.file.text).unwrap();
    wrong["parent_run"] = json!(HEALTHY);
    let mut moved = repro.clone();
    moved.file = Artifact::new(serde_json::to_vec_pretty(&wrong).unwrap()).unwrap();
    assert_eq!(
        status(
            token
                .post(url("reproductions"))
                .json(&moved)
                .send()
                .unwrap()
        )
        .0,
        422
    );
    assert_eq!(
        status(token.post(url("reproductions")).json(repro).send().unwrap()).1["status"],
        "created"
    );
    let (_, cx) = status(
        alice
            .get(server.url(&format!(
                "/v1/projects/{project}/view/counterexamples/{RESERVE_BOUNDARY}"
            )))
            .send()
            .unwrap(),
    );
    assert_eq!(cx["reproductions"]["count"], 1);
    assert_eq!(
        cx["reproduce"],
        format!("eplyx migration reproduce {RESERVE_BOUNDARY}")
    );
    assert_eq!(cx["minimized"], true);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ci_tokens_can_only_sync_their_project() {
    let server = Server::start();
    let (alice, account) = server.signup("alice@example.com");
    let (bob, _) = server.signup("bob@example.com");
    let workspace = workspace_of(&account);
    let project = server.create_project(&alice, &workspace, "ci");
    let other = server.create_project(&alice, &workspace, "other");
    alice
        .post(server.url(&format!("/v1/workspaces/{workspace}/members")))
        .json(&json!({"email":"bob@example.com"}))
        .send()
        .unwrap();
    // Only owners mint CI tokens.
    let (code, _) = status(
        bob.post(server.url(&format!("/v1/projects/{project}/tokens")))
            .json(&json!({"label":"x"}))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 403);
    let (code, created) = status(
        alice
            .post(server.url(&format!("/v1/projects/{project}/tokens")))
            .json(&json!({"label":"github main"}))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 201);
    let secret = created["token"].as_str().unwrap().to_owned();
    assert!(secret.starts_with("eplyx_proj_"));
    let ci = bearer(&secret);
    let root = fixture_project();
    let docs = documents(&root, "local_00000000000000c1c1c1");
    let healthy = docs.runs.iter().find(|r| r.run_id == HEALTHY).unwrap();
    let (code, body) = status(
        ci.post(server.url(&format!("/v1/projects/{project}/sync/runs")))
            .json(healthy)
            .send()
            .unwrap(),
    );
    assert_eq!(code, 201, "{body}");
    let (code, _) = status(
        ci.post(server.url(&format!("/v1/projects/{other}/sync/runs")))
            .json(healthy)
            .send()
            .unwrap(),
    );
    assert_eq!(code, 404);
    // MAIN project tokens can also read their own results; never another project.
    assert_eq!(
        ci.get(server.url(&format!("/v1/projects/{project}/view/runs")))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        ci.get(server.url(&format!("/v1/projects/{other}/view/runs")))
            .send()
            .unwrap()
            .status(),
        404
    );
    for path in [
        "/v1/workspaces".to_owned(),
        format!("/v1/projects/{project}/tokens"),
    ] {
        assert_eq!(
            ci.get(server.url(&path)).send().unwrap().status(),
            403,
            "{path}"
        );
    }
    let (_, detail) = status(
        alice
            .get(server.url(&format!("/v1/projects/{project}/workspace")))
            .send()
            .unwrap(),
    );
    let link = &detail["links"][0];
    assert_eq!(
        (
            link["linked_via"].as_str(),
            link["local_project_id"].as_str()
        ),
        (Some("ci"), Some("local_00000000000000c1c1c1"))
    );
    let (_, runs) = status(
        alice
            .get(server.url(&format!("/v1/projects/{project}/view/runs")))
            .send()
            .unwrap(),
    );
    assert_eq!(runs["runs"][0]["synced"]["via"], "ci");
    // The listing never shows the secret; revocation takes effect at once.
    let (_, tokens) = status(
        alice
            .get(server.url(&format!("/v1/projects/{project}/tokens")))
            .send()
            .unwrap(),
    );
    assert!(!tokens.to_string().contains(&secret));
    let id = created["id"].as_str().unwrap();
    assert_eq!(
        alice
            .delete(server.url(&format!("/v1/projects/{project}/tokens/{id}")))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        ci.post(server.url(&format!("/v1/projects/{project}/sync/runs")))
            .json(healthy)
            .send()
            .unwrap()
            .status(),
        401
    );
    // The database holds only a digest of each token.
    assert!(server.sql(&format!("DO $$ BEGIN IF EXISTS (SELECT 1 FROM api_tokens WHERE token_sha256 = '{secret}') THEN RAISE EXCEPTION 'plaintext'; END IF; END $$")).is_ok());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn browser_sessions_are_same_origin_and_cannot_relink() {
    let server = Server::start();
    let (alice, account) = server.signup("alice@example.com");
    let workspace = workspace_of(&account);
    // Reuse alice's session cookie from a hostile origin.
    let session = {
        let response = server
            .browser()
            .post(server.url("/v1/auth/login"))
            .json(&json!({"email":"alice@example.com","password":"correct horse battery"}))
            .send()
            .unwrap();
        response
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    };
    let hostile = reqwest::blocking::Client::new();
    let (code, _) = status(
        hostile
            .post(server.url(&format!("/v1/workspaces/{workspace}/projects")))
            .header("cookie", &session)
            .header("origin", "https://evil.example")
            .json(&json!({"name":"x"}))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 403);
    let (code, _) = status(
        hostile
            .post(server.url("/v1/auth/login"))
            .header("origin", "https://evil.example")
            .json(&json!({"email":"alice@example.com","password":"correct horse battery"}))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 403);
    let project = server.create_project(&alice, &workspace, "p");
    let (code, _) = status(
        alice
            .post(server.url(&format!("/v1/projects/{project}/links")))
            .json(&json!({"local_project_id": LOCAL_ID}))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 403, "linking belongs to the CLI");
    let cookie = server
        .browser()
        .post(server.url("/v1/auth/login"))
        .json(&json!({"email":"alice@example.com","password":"correct horse battery"}))
        .send()
        .unwrap();
    let flags = cookie
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        flags.contains("HttpOnly") && flags.contains("SameSite=Strict"),
        "{flags}"
    );
    let page = reqwest::blocking::get(server.url("/")).unwrap();
    let csp = page
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.contains("frame-ancestors 'none'") && csp.contains("script-src 'self'"));
    assert_eq!(page.headers().get("x-frame-options").unwrap(), "DENY");
}

#[test]
fn device_flow_is_approved_in_the_browser_and_single_use() {
    let server = Server::start();
    let (alice, _) = server.signup("alice@example.com");
    let anonymous = reqwest::blocking::Client::new();
    let start: Value = anonymous
        .post(server.url("/v1/auth/device"))
        .json(&json!({"client":"eplyx CLI test"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert!(start["verification_uri"]
        .as_str()
        .unwrap()
        .starts_with(&server.base));
    let poll = || {
        status(
            anonymous
                .post(server.url("/v1/auth/device/token"))
                .json(&json!({"device_code": start["device_code"]}))
                .send()
                .unwrap(),
        )
    };
    assert_eq!(poll().1["error"], "authorization_pending");
    let code = start["user_code"].as_str().unwrap();
    assert_eq!(
        anonymous
            .get(server.url(&format!("/v1/auth/device/lookup?code={code}")))
            .send()
            .unwrap()
            .status(),
        401
    );
    let token = bearer(&server.device_token(&alice));
    // A CLI token cannot approve further codes; only a browser session can.
    let (code2, _) = status(
        token
            .post(server.url("/v1/auth/device/approve"))
            .json(&json!({"user_code": code, "approve": true}))
            .send()
            .unwrap(),
    );
    assert_eq!(code2, 403);
    let (_, lookup) = status(
        alice
            .get(server.url(&format!(
                "/v1/auth/device/lookup?code={}",
                code.to_lowercase()
            )))
            .send()
            .unwrap(),
    );
    assert_eq!(lookup["state"], "pending");
    alice
        .post(server.url("/v1/auth/device/approve"))
        .json(&json!({"user_code": code, "approve": true}))
        .send()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let (ok, issued) = poll();
    assert_eq!(ok, 200);
    assert_eq!(issued["user"]["email"], "alice@example.com");
    assert_eq!(
        poll().1["error"],
        "expired_token",
        "a device code is single use"
    );
    let user = bearer(issued["access_token"].as_str().unwrap());
    assert_eq!(
        user.get(server.url("/v1/workspaces"))
            .send()
            .unwrap()
            .status(),
        200
    );
    // Denied codes never yield a token; revoked tokens stop working.
    let denied: Value = anonymous
        .post(server.url("/v1/auth/device"))
        .json(&json!({"client":"x"}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    alice
        .post(server.url("/v1/auth/device/approve"))
        .json(&json!({"user_code": denied["user_code"], "approve": false}))
        .send()
        .unwrap();
    let (_, body) = status(
        anonymous
            .post(server.url("/v1/auth/device/token"))
            .json(&json!({"device_code": denied["device_code"]}))
            .send()
            .unwrap(),
    );
    assert_eq!(body["error"], "access_denied");
    assert_eq!(
        user.delete(server.url("/v1/auth/token"))
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        user.get(server.url("/v1/workspaces"))
            .send()
            .unwrap()
            .status(),
        401
    );
}

#[test]
fn hosted_views_equal_the_local_dashboard_engine_views() {
    let server = Server::start();
    let (alice, account) = server.signup("alice@example.com");
    let project = server.create_project(&alice, &workspace_of(&account), "transition-acceptance");
    let token = bearer(&server.device_token(&alice));
    let root = fixture_project();
    let docs = documents(&root, LOCAL_ID);
    sync_all(&server, &token, &project, &docs);
    sync_all(&server, &token, &project, &docs);
    let store = Store::open(&root).unwrap();
    let get = |path: String| status(alice.get(server.url(&path)).send().unwrap()).1;
    let strip = |mut value: Value| {
        for side in ["left", "right"] {
            value[side].as_object_mut().unwrap().remove("synced");
            value[side].as_object_mut().unwrap().remove("number");
        }
        value
    };
    for (a, b) in [(HEALTHY, UNDERFUNDED), (UNDERFUNDED, HEALTHY)] {
        let hosted = get(format!(
            "/v1/projects/{project}/view/compare?left={a}&right={b}"
        ));
        let local = view::compare(&store, a, b).unwrap();
        assert_eq!(strip(hosted.clone()), strip(local), "{a} → {b}");
        assert_eq!(hosted["counterexamples"]["comparable"], false);
    }
    let overview = get(format!("/v1/projects/{project}/view/project"));
    assert_eq!(overview["stats"]["runs"], 4);
    assert_eq!(overview["stats"]["counterexamples_saved"], 4);
    assert_eq!(overview["stats"]["offline_reproductions"], 2);
    assert_eq!(overview["latest"]["id"], UNDERFUNDED);
    assert_eq!(overview["latest"]["gate"]["outcome"], "Block");
    assert_eq!(
        overview["context"]["cloud"]["project"]["visibility"],
        "workspace"
    );
    let detail = get(format!("/v1/projects/{project}/view/runs/{HEALTHY}"));
    assert_eq!(detail["kind"], "token_migration");
    assert_eq!(detail["gate_detail"]["consistent_with_engine"], true);
    assert!(
        detail["production"]["provider"].is_null(),
        "provider origin never syncs"
    );
    assert!(detail["evidence"]["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["name"] != "program.so"));
    let local_detail = view::run_detail(&store, HEALTHY, &[]).unwrap();
    assert_eq!(detail["gate_detail"], local_detail["gate_detail"]);
    assert_eq!(detail["stress_detail"], local_detail["stress_detail"]);
    assert_eq!(
        detail["invariant_results"],
        local_detail["invariant_results"]
    );
    let raw = alice
        .get(server.url(&format!(
            "/v1/projects/{project}/view/counterexamples/{RESERVE_BOUNDARY}/raw"
        )))
        .send()
        .unwrap()
        .text()
        .unwrap();
    assert_eq!(
        raw,
        std::fs::read_to_string(
            root.join(format!(".eplyx/counterexamples/{RESERVE_BOUNDARY}.json"))
        )
        .unwrap()
    );
    let page = alice
        .get(server.url(&format!("/p/{project}/runs/{HEALTHY}")))
        .send()
        .unwrap()
        .text()
        .unwrap();
    assert!(page.contains(&format!("data-api=\"/v1/projects/{project}/view\"")));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_single_demo_project_is_read_only_and_others_stay_private() {
    const DEMO: &str = "proj_00000000000000000000000001";
    let server = Server::start_with(|config| config.demo_project = Some(DEMO.into()));
    let (alice, account) = server.signup("alice@example.com");
    let workspace = workspace_of(&account);
    let user = account["user"]["id"].as_str().unwrap();
    server
        .state
        .registry
        .create_project(&eplyx_server::project::Project::analytical(DEMO, "demo").unwrap())
        .unwrap();
    server.sql(&format!("INSERT INTO project_workspaces (project_id,workspace_id,linked_by) VALUES ('{DEMO}','{workspace}','{user}')")).unwrap();
    let private = server.create_project(&alice, &workspace, "private");
    let token = bearer(&server.device_token(&alice));
    let root = fixture_project();
    let docs = documents(&root, LOCAL_ID);
    sync_all(&server, &token, DEMO, &docs);
    sync_all(&server, &token, &private, &docs);
    let anonymous = reqwest::blocking::Client::new();
    let (code, overview) = status(
        anonymous
            .get(server.url("/v1/demo/view/project"))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 200);
    assert_eq!(overview["project"]["id"], DEMO);
    assert_eq!(overview["context"]["cloud"]["demo"], true);
    assert!(
        overview["context"]["cloud"]["links"].is_null(),
        "demo hides who linked what"
    );
    let (code, compare) = status(
        anonymous
            .get(server.url(&format!(
                "/v1/demo/view/compare?left={HEALTHY}&right={UNDERFUNDED}"
            )))
            .send()
            .unwrap(),
    );
    assert_eq!(code, 200);
    assert_eq!(compare["counterexamples"]["comparable"], false);
    assert_eq!(
        anonymous
            .get(server.url(&format!("/v1/projects/{private}/view/project")))
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        anonymous
            .get(server.url(&format!("/v1/projects/{DEMO}/view/project")))
            .send()
            .unwrap()
            .status(),
        401,
        "the demo is reachable only through /demo"
    );
    let (code, _) = status(
        anonymous
            .post(server.url("/v1/demo/view/project"))
            .json(&json!({}))
            .send()
            .unwrap(),
    );
    assert!(code == 404 || code == 405, "demo is read-only: {code}");
    let (code, _) = status(
        anonymous
            .post(server.url(&format!("/v1/projects/{DEMO}/sync/runs")))
            .json(&docs.runs[0])
            .send()
            .unwrap(),
    );
    assert_eq!(code, 401);
    std::fs::remove_dir_all(root).unwrap();
}
