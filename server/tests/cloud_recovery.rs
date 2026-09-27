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
