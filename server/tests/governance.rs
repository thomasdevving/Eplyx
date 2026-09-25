//! Phase G1: binding a Squads proposal to a hosted analysis.
//!
//! The chain is the simulated Squads world, driven through the real verifier;
//! the analysis is a real run of the real engine over the committed bundle.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use eplyx_engine::change::ChangeSpec;
use eplyx_engine::governance::simulated::{self, World};
use eplyx_server::registry::RunStatus;
use serde_json::{json, Value};

fn fixture_world() -> Arc<World> {
    let program = committed_record().program_id.parse().expect("program id");
    Arc::new(World::upgrading(program, &candidate_bytes()))
}

async fn ready(harness: &Harness) -> (String, String) {
    let project_id = harness.create_project("Governed").await;
    harness.activate_bundle(&project_id).await;
    let token = harness.create_token(&project_id, "ci").await;
    (project_id, token)
}

/// A run that reached a verdict. The fixture V2 candidate is a known
/// regression, so that verdict is `failed`; binding does not depend on it.
async fn finished_run(harness: &Harness, accepted: &Value) -> String {
    let id = accepted["run_id"].as_str().expect("run id").to_string();
    let status = harness.wait_until(&id, RunStatus::is_terminal).await;
    assert!(
        matches!(status, RunStatus::Passed | RunStatus::Failed),
        "{status:?}: {accepted}"
    );
    assert!(
        harness
            .state
            .registry
            .load_run(&id)
            .unwrap()
            .report_available
    );
    id
}

fn verify_body(extra: Value) -> Value {
    let mut body = json!({
        "multisig": simulated::multisig().to_string(),
        "transaction_index": simulated::TRANSACTION_INDEX,
    });
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    body
}

#[tokio::test]
async fn a_proposal_is_bound_to_an_analysis_by_identity_not_by_label() {
    let world = fixture_world();
    let harness = Harness::with_governance(1, world.clone());
    let (project, token) = ready(&harness).await;
    let verify = format!("/v1/projects/{project}/governance/squads/verify");

    // 1. An ordinary analysis of the candidate bytes: unbound.
    let (status, accepted) = harness.submit_check(&project, &token).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let unbound_run = finished_run(&harness, &accepted).await;
    let unbound_id = accepted["change"]["change_spec_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(unbound_id, world.spec().id().unwrap());

    // 2. The proposal matches its content, and the answer names the
    //    governance-bound change — a different id with no analysis yet.
    let (status, body) = harness
        .post_json(
            &verify,
            &token,
            verify_body(json!({ "run_id": unbound_run })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "matched", "{body}");
    assert_eq!(body["exit_code"], 0);
    let bound_id = body["change_spec_id"].as_str().unwrap().to_string();
    assert_eq!(bound_id, world.bound_spec().id().unwrap());
    assert_ne!(bound_id, unbound_id);
    assert_eq!(body["analysed_change_spec_id"], unbound_id.as_str());
    assert_eq!(body["governance_bound"], false);
    assert_eq!(body["analysis"]["runs"], json!([]));
    assert_eq!(body["expected_candidate"]["held_by_project"], true);
    assert_eq!(body["proposal"]["status"], "active");
    let slot = body["observed_slot"].as_u64().expect("slot");
    assert!(body["statement"]
        .as_str()
        .unwrap()
        .contains(&format!("slot {slot}")));
    let bound_document = body["analysis"]["bound_change_spec"].to_string();
    let bound = ChangeSpec::parse(bound_document.as_bytes()).expect("a valid bound spec");
    assert_eq!(bound.id().unwrap(), bound_id);

    // 3. Analyse the bound spec. No upload: the project already holds the bytes.
    let (status, accepted) = harness
        .submit_parts(
            &project,
            &token,
            &[("change_spec", bound_document.as_bytes())],
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    assert_eq!(accepted["change"]["change_spec_id"], bound_id.as_str());
    assert_eq!(accepted["change"]["delivery"]["provider"], "squads_v4");
    let bound_run = finished_run(&harness, &accepted).await;
    let (_, report) = harness
        .get(&format!("/v1/runs/{bound_run}/report.json"), &token)
        .await;
    assert_eq!(report["change"]["change_spec_id"], bound_id.as_str());
    assert_eq!(
        report["change"]["delivery"]["transaction_index"],
        simulated::TRANSACTION_INDEX
    );
    // The unbound run's report is untouched: no Squads label was attached to it.
    let (_, old) = harness
        .get(&format!("/v1/runs/{unbound_run}/report.json"), &token)
        .await;
    assert!(old["change"].get("delivery").is_none());

    // 4. Verify by the bound id: now governance-bound, with its analysis.
    let (status, body) = harness
        .post_json(
            &verify,
            &token,
            verify_body(json!({ "change_spec_id": bound_id })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "matched");
    assert_eq!(body["governance_bound"], true);
    assert_eq!(body["analysis"]["runs"], json!([bound_run]));
    assert_eq!(body["analysis"]["bound_change_spec"], Value::Null);

    // 5. The buffer is rewritten; a re-check re-reads and says so.
    world.edit(|s| s.buffer_bytes = b"\x7fELF\x02\x01\x01 rewritten after review".to_vec());
    let (status, stale) = harness
        .post_json(
            &verify,
            &token,
            verify_body(json!({ "change_spec_id": bound_id })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{stale}");
    assert_eq!(stale["status"], "stale_artifact");
    assert_eq!(stale["exit_code"], 1);
    assert!(stale["observed_slot"].as_u64().unwrap() > slot);

    // 6. History lists every check of the bound change, newest first.
    let (status, listed) = harness
        .get(
            &format!("/v1/projects/{project}/governance/changes/{bound_id}"),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let checks = listed["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 3);
    assert_eq!(checks[0]["status"], "stale_artifact");
    assert_eq!(checks[1]["status"], "matched");
    for check in checks {
        assert!(check["observed_slot"].is_u64(), "{check}");
    }

    // 6b. A re-check that finds another message derives another bound id. It
    //     is still filed under the change it was asked about, so the bound
    //     change's newest check is the failure, not its last match.
    world.edit(|s| {
        let upgrade = s.message().instructions[0].clone();
        s.message().instructions.push(upgrade);
    });
    let (_, altered) = harness
        .post_json(
            &verify,
            &token,
            verify_body(json!({ "change_spec_id": bound_id })),
        )
        .await;
    assert_eq!(altered["status"], "unsupported_proposal", "{altered}");
    assert_ne!(altered["change_spec_id"], bound_id.as_str());
    let (_, relisted) = harness
        .get(
            &format!("/v1/projects/{project}/governance/changes/{bound_id}"),
            &token,
        )
        .await;
    assert_eq!(relisted["checks"].as_array().unwrap().len(), 4);
    assert_eq!(relisted["checks"][0]["status"], "unsupported_proposal");

    // 7. A stored binding edited on disk is refused, never served as matched.
    let directory = harness
        .scratch
        .path()
        .join("data/projects")
        .join(&project)
        .join("governance")
        .join(&bound_id)
        .join("bindings");
    let matched = checks[1]["binding_id"].as_str().unwrap();
    let path = directory.join(format!("{matched}.json"));
    let mut stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stored["observation"]["slot"] = json!(1);
    std::fs::write(&path, stored.to_string()).unwrap();
    let (status, body) = harness
        .get(
            &format!("/v1/projects/{project}/governance/changes/{bound_id}"),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("does not verify"),
        "{body}"
    );
}

#[tokio::test]
async fn governance_requests_are_scoped_validated_and_optional() {
    let world = fixture_world();
    let harness = Harness::with_governance(1, world.clone());
    let (project, token) = ready(&harness).await;
    let (other, other_token) = ready(&harness).await;
    let (_, accepted) = harness.submit_check(&project, &token).await;
    let run = finished_run(&harness, &accepted).await;
    let verify = format!("/v1/projects/{project}/governance/squads/verify");

    // Another project's token cannot bind this project's analysis, and this
    // project's run is invisible from another project.
    let (status, _) = harness
        .post_json(&verify, &other_token, verify_body(json!({ "run_id": run })))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = harness
        .post_json(
            &format!("/v1/projects/{other}/governance/squads/verify"),
            &other_token,
            verify_body(json!({ "run_id": run })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Exactly one analysed reference, and nothing unrecognised.
    for body in [
        verify_body(json!({})),
        verify_body(json!({ "run_id": run, "change_spec_id": "a".repeat(64) })),
        verify_body(json!({ "run_id": run, "proposal_status": "active" })),
        verify_body(json!({ "change_spec_id": "not-a-hash" })),
    ] {
        let (status, response) = harness.post_json(&verify, &token, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    }
    let (status, _) = harness
        .post_json(
            &verify,
            &token,
            verify_body(json!({ "change_spec_id": "b".repeat(64) })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A proposal outside the bounded shape is answered, typed, not refused.
    world.edit(|s| {
        let upgrade = s.message().instructions[0].clone();
        s.message().instructions.push(upgrade);
    });
    let (status, body) = harness
        .post_json(&verify, &token, verify_body(json!({ "run_id": run })))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "unsupported_proposal");
    assert_eq!(body["exit_code"], 4);
    assert_eq!(body["analysis"]["bound_change_spec"], Value::Null);

    // Without a configured chain the endpoint is off, and says why.
    let plain = Harness::new(1);
    let (project, token) = ready(&plain).await;
    let (status, body) = plain
        .post_json(
            &format!("/v1/projects/{project}/governance/squads/verify"),
            &token,
            verify_body(json!({ "change_spec_id": "c".repeat(64) })),
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}
