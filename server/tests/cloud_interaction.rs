//! Real loopback identity/session/project authorization, over isolated Postgres.
mod cloud_common;
use cloud_common::*;
use eplyx_engine::change::ChangeSpec;
use eplyx_server::{
    project::{AdapterId, Project},
    registry::RunStatus,
};
use serde_json::{json, Value};
const PROGRAM: &str = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";
#[test]
fn workspace_sessions_authorize_all_interaction_operations_and_deny_other_workspaces() {
    let s = Server::start();
    let (browser, account) = s.signup("interaction-owner@example.com");
    let (other, _) = s.signup("interaction-other@example.com");
    let project = s.create_project(
        &browser,
        account["workspace_id"].as_str().unwrap(),
        "Interaction session",
    );
    // Seed this local project's ordinary upgrade setup through the same registry
    // used by operator bundle registration, retaining the real pinned bundle.
    let configured = Project::new(
        &project,
        "Interaction session",
        PROGRAM,
        AdapterId::for_program(PROGRAM),
    )
    .unwrap();
    s.state
        .registry
        .storage()
        .write_json(
            &s.state
                .registry
                .storage()
                .project_dir(&project)
                .unwrap()
                .join("project.json"),
            &configured,
        )
        .unwrap();
    let registered = s
        .state
        .registry
        .register_bundle(&configured, std::path::Path::new("../deploy/bundle"), None)
        .unwrap();
    s.state
        .registry
        .activate_bundle(&project, &registered.bundle_id)
        .unwrap();
    let candidate = std::fs::read("../artifacts/fixture_stake_pool_config_v2.so").unwrap();
    let mut body=b"--interaction-boundary\r\nContent-Disposition: form-data; name=\"candidate\"; filename=\"candidate.so\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
    body.extend(candidate);
    body.extend_from_slice(b"\r\n--interaction-boundary--\r\n");
    let response = browser
        .post(s.url(&format!("/v1/projects/{project}/checks")))
        .header(
            "content-type",
            "multipart/form-data; boundary=interaction-boundary",
        )
        .body(body)
        .send()
        .unwrap();
    assert_eq!(response.status(), 202);
    let accepted: Value = response.json().unwrap();
    let parent = accepted["run_id"].as_str().unwrap();
    for _ in 0..200 {
        if s.state
            .registry
            .load_run(parent)
            .unwrap()
            .status
            .is_terminal()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(
        s.state.registry.load_run(parent).unwrap().status,
        RunStatus::Failed
    );
    let path = format!("/v1/projects/{project}/runs/{parent}/interactions");
    let anonymous = reqwest::blocking::Client::new();
    let proposal = ChangeSpec::parse(include_bytes!(
        "../../docs/examples/stake-pool-parameter-change.json"
    ))
    .unwrap();
    let payload = json!({"record_id":"mainnet-spl-stake-pool-151010f709e113e7","parameter_change_spec":proposal});
    for suffix in ["", "/eligibility"] {
        assert_eq!(
            anonymous
                .get(s.url(&format!("{path}{suffix}")))
                .send()
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            other
                .get(s.url(&format!("{path}{suffix}")))
                .send()
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            browser
                .get(s.url(&format!("{path}{suffix}")))
                .send()
                .unwrap()
                .status(),
            200
        );
    }
    assert_eq!(
        other
            .post(s.url(&format!("{path}/preview")))
            .json(&payload)
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        browser
            .post(s.url(&format!("{path}/preview")))
            .json(&payload)
            .send()
            .unwrap()
            .status(),
        200
    );
    let mut payload = payload;
    payload["request_key"] = json!("workspace-interaction-key");
    assert_eq!(
        anonymous
            .post(s.url(&path))
            .json(&payload)
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        other
            .post(s.url(&path))
            .json(&payload)
            .send()
            .unwrap()
            .status(),
        404
    );
    let response = browser.post(s.url(&path)).json(&payload).send().unwrap();
    assert_eq!(response.status(), 202);
    let body: Value = response.json().unwrap();
    let child = body["run_id"].as_str().unwrap();
    for _ in 0..600 {
        if s.state
            .registry
            .load_run(child)
            .unwrap()
            .status
            .is_terminal()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(
        s.state.registry.load_run(child).unwrap().status,
        RunStatus::Completed
    );
    for suffix in ["", "/artifact"] {
        let url = s.url(&format!(
            "/v1/projects/{project}/interactions/{child}{suffix}"
        ));
        assert_eq!(anonymous.get(&url).send().unwrap().status(), 401);
        assert_eq!(other.get(&url).send().unwrap().status(), 404);
        assert_eq!(browser.get(&url).send().unwrap().status(), 200);
    }
}
