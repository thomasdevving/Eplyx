//! Project capability discovery is authenticated, current and explanatory.
//! Submission remains authoritative; this endpoint only reports prerequisites.

mod common;

use std::sync::Arc;

use anyhow::Result;
use axum::{body::Body, http::Request};
use common::*;
use eplyx_engine::ingest::rpc::RpcProvider;
use eplyx_server::{
    hosted::observation::Service,
    project::{Project, ProjectStatus},
};
use serde_json::{json, Value};

fn analysis<'a>(body: &'a Value, kind: &str) -> &'a Value {
    body["analyses"]
        .as_array()
        .expect("analysis list")
        .iter()
        .find(|analysis| analysis["kind"] == kind)
        .expect("hosted analysis kind")
}

fn missing_codes(capability: &Value) -> Vec<&str> {
    capability["missing"]
        .as_array()
        .expect("missing list")
        .iter()
        .map(|reason| reason["code"].as_str().expect("reason code"))
        .collect()
}

#[tokio::test]
async fn capabilities_require_project_access_and_report_independent_current_state() {
    let harness = Harness::new(0);
    let project = harness
        .create_project("FASTRPC_TOKEN=abc123 /Users/private/server.env")
        .await;
    let token = harness.create_token(&project, "browser").await;
    let other = harness.create_project("Other").await;
    let other_token = harness.create_token(&other, "other").await;
    let path = format!("/v1/projects/{project}/capabilities");

    let (status, body) = harness
        .send(
            Request::builder()
                .uri(&path)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED, "{body}");

    let (status, body) = harness.get(&path, &other_token).await;
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED, "{body}");

    let (status, setup) = harness.get(&path, &token).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{setup}");
    assert_eq!(setup["schema_version"], 1);
    assert_eq!(setup["project_id"], project);
    assert_eq!(setup["analyses"].as_array().unwrap().len(), 9);
    assert_eq!(
        setup["analyses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "program_upgrade",
            "token_migration",
            "lifecycle_change",
            "protocol_parameter_change",
            "current_observation",
            "current_path",
            "current_candidate",
            "current_preflight",
            "current_stress",
        ]
    );

    let upgrade = analysis(&setup, "program_upgrade");
    assert_eq!(upgrade["status"], "not_ready");
    assert_eq!(upgrade["supported"], true);
    assert_eq!(upgrade["can_submit"], false);
    assert_eq!(missing_codes(upgrade), vec!["active_bundle_missing"]);

    for kind in [
        "token_migration",
        "lifecycle_change",
        "protocol_parameter_change",
    ] {
        let capability = analysis(&setup, kind);
        assert_eq!(capability["status"], "ready", "{kind}: {capability}");
        assert_eq!(capability["supported"], true);
        assert_eq!(capability["can_submit"], true);
        assert_eq!(capability["missing"], json!([]));
    }
    assert_eq!(
        missing_codes(analysis(&setup, "current_observation")),
        vec!["observation_service_unavailable"]
    );
    assert_eq!(
        missing_codes(analysis(&setup, "current_candidate")),
        vec![
            "observation_service_unavailable",
            "migration_candidate_not_configured"
        ]
    );

    // The response is a deliberately narrow projection, not raw project,
    // provider, filesystem or server configuration.
    let encoded = serde_json::to_string(&setup).unwrap();
    for secret in [
        OPERATOR,
        "abc123",
        "FASTRPC_TOKEN",
        "/Users/private/server.env",
        harness.scratch.path().to_str().unwrap(),
        "postgres://",
        "Bearer ",
    ] {
        assert!(!encoded.contains(secret), "capabilities leaked {secret:?}");
    }

    harness.activate_bundle(&project).await;
    let (status, ready) = harness.get(&path, &token).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{ready}");
    let upgrade = analysis(&ready, "program_upgrade");
    assert_eq!(upgrade["status"], "ready");
    assert_eq!(upgrade["can_submit"], true);
    assert_eq!(upgrade["missing"], json!([]));

    let mut unavailable = harness.state.registry.load_project(&project).unwrap();
    unavailable.active_bundle.as_mut().unwrap().bundle_sha256 = "0".repeat(64);
    harness.state.registry.save_project(&unavailable).unwrap();
    let (_, missing_bundle) = harness.get(&path, &token).await;
    assert_eq!(
        missing_codes(analysis(&missing_bundle, "program_upgrade")),
        vec!["active_bundle_unavailable"]
    );

    // No readiness cache: removing the authoritative pointer changes the next
    // response, and restoring equivalent state restores deterministic bytes.
    let mut record = unavailable;
    record.active_bundle = None;
    record.refresh_status();
    harness.state.registry.save_project(&record).unwrap();
    let (status, after) = harness.get(&path, &token).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{after}");
    assert_eq!(after, setup);
}

struct NeverCalled;
impl RpcProvider for NeverCalled {
    fn call(&self, _: &str, _: Value) -> Result<Value> {
        panic!("capability discovery must not contact the provider")
    }
}

#[tokio::test]
async fn observation_and_disabled_project_requirements_are_derived_per_kind() {
    let mut harness = Harness::new(0);
    Arc::get_mut(&mut harness.state).unwrap().observation =
        Some(Service::new(Arc::new(NeverCalled)));
    let project = harness.create_project("Observed").await;
    let token = harness.create_token(&project, "browser").await;
    let path = format!("/v1/projects/{project}/capabilities");

    let (_, body) = harness.get(&path, &token).await;
    for kind in [
        "current_observation",
        "current_path",
        "current_preflight",
        "current_stress",
    ] {
        let capability = analysis(&body, kind);
        assert_eq!(capability["can_submit"], true, "{kind}: {capability}");
        assert_eq!(capability["missing"], json!([]));
    }
    assert_eq!(
        missing_codes(analysis(&body, "current_candidate")),
        vec!["migration_candidate_not_configured"]
    );

    let analytical = eplyx_server::ids::project();
    harness
        .state
        .registry
        .create_project(&Project::analytical(&analytical, "Prepared inputs").unwrap())
        .unwrap();
    let (_, analytical_body) = harness
        .get(&format!("/v1/projects/{analytical}/capabilities"), OPERATOR)
        .await;
    assert_eq!(
        missing_codes(analysis(&analytical_body, "program_upgrade")),
        vec!["upgrade_target_missing", "active_bundle_missing"]
    );

    harness.set_status(&project, ProjectStatus::Disabled);
    let (_, disabled) = harness.get(&path, &token).await;
    for capability in disabled["analyses"].as_array().unwrap() {
        assert_eq!(capability["status"], "not_ready", "{capability}");
        assert_eq!(capability["can_submit"], false, "{capability}");
        assert!(missing_codes(capability).contains(&"project_disabled"));
    }
    // Prepared inputs have no bundle/provider requirement; disabling the
    // project is their only project-level blocker.
    assert_eq!(
        missing_codes(analysis(&disabled, "token_migration")),
        vec!["project_disabled"]
    );
    assert_eq!(
        missing_codes(analysis(&disabled, "lifecycle_change")),
        vec!["project_disabled"]
    );

    // A capability response can become stale. The authoritative submission
    // boundary reports that readiness change distinctly and creates no run,
    // even before it tries to interpret multipart analytical inputs.
    let (status, refused) = harness.submit_parts(&project, &token, &[]).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{refused}");
    assert_eq!(
        refused["error"],
        "this project is disabled and accepts no checks"
    );
    assert!(harness
        .state
        .registry
        .project_run_ids(&project)
        .unwrap()
        .is_empty());
}
