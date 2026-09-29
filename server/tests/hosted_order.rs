//! Hosted order tests reuse the existing router, durable worker and real pinned
//! Phase 7B fixture. No provider or observation service is configured.
mod common;
use axum::http::StatusCode;
use common::*;
use eplyx_engine::{
    migration::{
        adapter, fixture,
        input::{self, Config, StateSource},
        order::{ComparisonStatus, FailureKind},
        order_store,
        planner::RehearsalClockPolicy,
    },
    replay::hash_bytes,
};
use eplyx_server::{
    hosted::{order, worker, Input},
    project::Project,
    registry::{RunOutcome, RunStatus},
};
use serde_json::{json, Value};

async fn make_parent(h: &Harness, reserve: u64) -> (String, String, [String; 2]) {
    parent_with(h, reserve, false).await
}
async fn parent_with(h: &Harness, reserve: u64, defect: bool) -> (String, String, [String; 2]) {
    let p = eplyx_server::ids::project();
    h.state
        .registry
        .create_project(&Project::analytical(&p, "Order parent").unwrap())
        .unwrap();
    let recipe_bytes = std::fs::read(
        eplyx_engine::repo_root().join("fixtures/migration/order/shared-reserve.recipe.json"),
    )
    .unwrap();
    let recipe = fixture::Recipe::parse(&recipe_bytes).unwrap();
    let mut value: Value = serde_json::from_str(include_str!(
        "../../examples/migrations/minimal/migration.json"
    ))
    .unwrap();
    value["source"]["mint"] = json!(fixture::address_of(&recipe, "source"));
    value["destination"]["mint"] = json!(fixture::address_of(&recipe, "destination"));
    value["destination"]["token_program"] =
        json!(eplyx_engine::standard_programs::token::LEGACY_PROGRAM);
    value["destination"]["decimals"] = json!(6);
    value["conversion"]["ratio_basis"] = json!("raw");
    value["conversion"]["denominator"] = json!("2");
    value["window"] = json!({});
    value["authorities"]["expected"] = json!({});
    value["destination_funding"]["reserve"]["funded_raw"] = json!(reserve.to_string());
    if defect {
        value["conversion"]["fee"] = json!({"kind":"source_bps","bps":25});
    }
    let terms: eplyx_engine::migration::spec::TokenMigrationV1 =
        serde_json::from_value(value).unwrap();
    let candidate = std::fs::read(eplyx_engine::repo_root().join(if defect {
        "artifacts/eplyx_token_migration_defect_fee_ceiling.so"
    } else {
        "artifacts/eplyx_token_migration.so"
    }))
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("input");
    input::assemble(
        &root,
        &terms,
        adapter::REFERENCE_PROGRAM_ID,
        &candidate,
        &Config {
            state: StateSource::SyntheticFixture {
                recipe: "fixture.json".into(),
                recipe_sha256: hash_bytes(&recipe_bytes),
            },
            rehearsal_clock: RehearsalClockPolicy::Captured,
            max_rehearsal_units: 100,
            max_captured_holders: 100,
        },
        Some(&recipe_bytes),
        vec![],
    )
    .unwrap();
    let parts = [
        ("candidate", candidate),
        (
            "change_spec",
            std::fs::read(root.join("change.json")).unwrap(),
        ),
        (
            "state_input",
            std::fs::read(root.join("state.json")).unwrap(),
        ),
        ("state_artifact", recipe_bytes),
        (
            "pinned_program_capture",
            std::fs::read(eplyx_engine::repo_root().join(fixture::PROGRAM_CAPTURE)).unwrap(),
        ),
    ];
    let refs: Vec<_> = parts
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    let (code, v) = h.submit_parts(&p, OPERATOR, &refs).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{v}");
    let id = v["run_id"].as_str().unwrap().to_owned();
    let record = h.state.registry.load_run(&id).unwrap();
    h.state.registry.begin_run(&id).unwrap();
    let projection =
        worker::run_isolated(&h.state.registry, &record, &h.state.config.worker_binary).unwrap();
    h.state
        .registry
        .finish_run(
            &id,
            RunOutcome::Analytical {
                projection: Box::new(projection),
            },
        )
        .unwrap();
    (
        p,
        id,
        [
            fixture::address_of(&recipe, "alice-source"),
            fixture::address_of(&recipe, "bob-source"),
        ],
    )
}
fn entry(p: &str, id: &str) -> String {
    format!("/v1/projects/{p}/runs/{id}/migration-order")
}
fn result(p: &str, id: &str) -> String {
    format!("/v1/projects/{p}/migration-orders/{id}")
}
async fn submit(h: &Harness, p: &str, parent: &str, sources: &[String; 2]) -> String {
    let (code, v) = h
        .post_json(
            &entry(p, parent),
            OPERATOR,
            json!({"source_a":sources[0],"source_b":sources[1]}),
        )
        .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{v}");
    v["run_id"].as_str().unwrap().to_owned()
}
#[tokio::test]
async fn eligible_parent_auth_selection_and_narrow_schema() {
    let h = Harness::new(0);
    let (p, id, s) = make_parent(&h, 80).await;
    let (code, e) = h
        .get(&format!("{}/eligibility", entry(&p, &id)), OPERATOR)
        .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(e["eligible"], true, "{e}");
    assert_eq!(e["units"].as_array().unwrap().len(), 2);
    // Both solo controls are eligible despite the population reserve shortfall.
    let other = h.create_project("Other project").await;
    let token = h.create_token(&other, "other").await;
    assert_eq!(
        h.get(&format!("{}/eligibility", entry(&p, &id)), &token)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post_json(
            &entry(&p, &id),
            &token,
            json!({"source_a":s[0],"source_b":s[1]})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post_json(
            &entry(&other, &id),
            OPERATOR,
            json!({"source_a":s[0],"source_b":s[1]})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for body in [
        json!({"source_a":s[0],"source_b":s[0]}),
        json!({"source_a":s[0],"source_b":adapter::REFERENCE_PROGRAM_ID}),
        json!({"source_a":s[0],"source_b":s[1],"world":{}}),
    ] {
        assert!(h
            .post_json(&entry(&p, &id), OPERATOR, body)
            .await
            .0
            .is_client_error());
    }
    let queued = submit(&h, &p, &id, &s).await;
    let record = h.state.registry.load_run(&queued).unwrap();
    assert_eq!(record.status, RunStatus::Queued);
    assert!(record.attempts.is_empty());
    assert!(record
        .hosted_analysis
        .as_ref()
        .unwrap()
        .projection
        .is_none());
    assert!(h.state.registry.hosted_input(&record).is_ok());
    assert_eq!(
        h.get(&result(&p, &queued), &token).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.get(&format!("{}/artifact", result(&p, &queued)), &token)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.get(&format!("{}/eligibility", entry(&p, &queued)), OPERATOR)
            .await
            .1["reason_code"],
        "wrong_analysis_kind"
    );
    assert_eq!(
        h.post_json(
            &entry(&p, &queued),
            OPERATOR,
            json!({"source_a":s[0],"source_b":s[1]})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(h.state.registry.project_run_ids(&p).unwrap().len(), 2);
}
#[tokio::test]
async fn asynchronous_order_effect_portability_occurrences_and_no_rpc() {
    let h = Harness::new(0);
    let (p, parent, s) = make_parent(&h, 80).await;
    assert!(h.state.observation.is_none() && h.state.governance.is_none());
    let first = submit(&h, &p, &parent, &s).await;
    let second = submit(&h, &p, &parent, &s).await;
    assert_ne!(first, second);
    h.state.runs.add_permits(1);
    for id in [&first, &second] {
        assert_eq!(
            h.wait_until_timeout(id, RunStatus::is_terminal, 120).await,
            RunStatus::Completed
        );
    }
    let (code, a) = h.get(&result(&p, &first), OPERATOR).await;
    assert_eq!(code, StatusCode::OK, "{a}");
    let (_, b) = h.get(&result(&p, &second), OPERATOR).await;
    assert_eq!(a["analysis"], b["analysis"]);
    assert_eq!(a["parent_run_id"], parent);
    assert_eq!(
        a["analysis"]["comparison"]["status"],
        "SharedReserveChangesSuccessfulUnit"
    );
    assert!(a["failure"].is_null());
    assert_eq!(a["analysis"]["scenarios"].as_array().unwrap().len(), 4);
    let (_, list) = h.get(&entry(&p, &parent), OPERATOR).await;
    assert_eq!(list["orders"].as_array().unwrap().len(), 2);
    let record = h.state.registry.load_run(&first).unwrap();
    let change = record.change.as_ref().unwrap().change_spec_id.clone();
    assert_eq!(
        h.state.registry.change_run_ids(&p, &change).unwrap(),
        vec![parent.clone()]
    );
    let (_, history) = h.get(&format!("/v1/projects/{p}/runs"), OPERATOR).await;
    assert_eq!(history["runs"].as_array().unwrap().len(), 1);
    let projection = h.state.registry.analytical_projection(&record).unwrap();
    let portable: Value = serde_json::from_str(&projection.bindings.unwrap().text).unwrap();
    // The candidate is shared with the parent's program CAS object.
    let candidate = record.candidate_artifact.unwrap();
    assert_eq!(
        portable["files"][format!("evidence/programs/{}", candidate.sha256)]["sha256"],
        candidate.sha256
    );
    assert!(
        !a["analysis"]["states"].to_string().contains("\"data\":")
            && !a.to_string().contains("/tmp/")
            && !a.to_string().contains("/Users/")
    );
    let (status, archive) = h
        .get_bytes(&format!("{}/artifact", result(&p, &first)), OPERATOR)
        .await;
    assert_eq!(status, StatusCode::OK);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("order.tar");
    std::fs::write(&file, archive).unwrap();
    assert!(std::process::Command::new("/usr/bin/tar")
        .args(["-xf"])
        .arg(&file)
        .arg("-C")
        .arg(temp.path())
        .status()
        .unwrap()
        .success());
    let reproduced =
        order_store::reproduce(&temp.path().canonicalize().unwrap().join("order-case")).unwrap();
    assert_eq!(
        reproduced.comparison.status,
        ComparisonStatus::SharedReserveChangesSuccessfulUnit
    );
    assert_eq!(a["analysis"], order::presentation(&reproduced).unwrap());
    let reopened = h.reopen();
    assert!(reopened
        .registry
        .analytical_projection(&reopened.registry.load_run(&first).unwrap())
        .is_ok());
}
#[tokio::test]
async fn no_effect_and_unestablished_are_completed_analyses() {
    let h = Harness::new(0);
    let (p, parent, s) = make_parent(&h, 110).await;
    let id = submit(&h, &p, &parent, &s).await;
    h.state.runs.add_permits(1);
    assert_eq!(
        h.wait_until_timeout(&id, RunStatus::is_terminal, 120).await,
        RunStatus::Completed
    );
    let (_, a) = h.get(&result(&p, &id), OPERATOR).await;
    assert_eq!(
        a["analysis"]["comparison"]["status"],
        "NoSuccessfulUnitEffect"
    );
    assert!(a["failure"].is_null());
    let h = Harness::new(0);
    let (p, parent, s) = parent_with(&h, 80, true).await;
    let id = submit(&h, &p, &parent, &s).await;
    h.state.runs.add_permits(1);
    assert_eq!(
        h.wait_until_timeout(&id, RunStatus::is_terminal, 120).await,
        RunStatus::Completed
    );
    let (_, a) = h.get(&result(&p, &id), OPERATOR).await;
    assert_eq!(a["analysis"]["comparison"]["status"], "NotEstablished");
    assert!(a["failure"].is_null());
    assert_eq!(
        a["analysis"]["scenarios"][0]["stopped"],
        "ReconciliationMismatch"
    );
}
#[tokio::test]
async fn parent_binding_and_portable_tampering_fail_closed_without_regeneration() {
    let h = Harness::new(0);
    let (p, parent, s) = make_parent(&h, 80).await;
    let id = submit(&h, &p, &parent, &s).await;
    h.state.runs.add_permits(1);
    h.wait_until_timeout(&id, RunStatus::is_terminal, 120).await;
    let record = h.state.registry.load_run(&id).unwrap();
    let projection = h.state.registry.analytical_projection(&record).unwrap();
    let mut bad = projection.clone();
    let mut bindings: Value = serde_json::from_str(&bad.bindings.as_ref().unwrap().text).unwrap();
    bindings["parent_run"] = json!("run_other");
    bad.bindings = Some(
        eplyx_engine::cloud::contract::Artifact::new(serde_json::to_vec(&bindings).unwrap())
            .unwrap(),
    );
    assert!(h
        .state
        .registry
        .verify_hosted_projection(&record, &bad)
        .is_err());
    let portable: Value = serde_json::from_str(&projection.bindings.unwrap().text).unwrap();
    let digest = portable["files"]["case.json"]["sha256"].as_str().unwrap();
    let path = h
        .state
        .registry
        .artifacts()
        .root()
        .join("captures")
        .join(digest);
    let original = std::fs::read(&path).unwrap();
    std::fs::write(&path, b"tampered").unwrap();
    assert_eq!(
        h.get(&result(&p, &id), OPERATOR).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        h.get_bytes(&format!("{}/artifact", result(&p, &id)), OPERATOR)
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"tampered");
    std::fs::write(&path, original).unwrap();
    assert_eq!(h.get(&result(&p, &id), OPERATOR).await.0, StatusCode::OK);
}
#[tokio::test]
async fn evidence_gap_after_acceptance_and_typed_handoff_failure_are_preserved() {
    let h = Harness::new(0);
    let (p, parent, s) = make_parent(&h, 80).await;
    let id = submit(&h, &p, &parent, &s).await;
    let parent_record = h.state.registry.load_run(&parent).unwrap();
    let Input::TokenMigration { state_artifact, .. } =
        h.state.registry.hosted_input(&parent_record).unwrap()
    else {
        panic!()
    };
    std::fs::write(
        h.state
            .registry
            .artifacts()
            .root()
            .join("captures")
            .join(&state_artifact.sha256),
        b"tampered",
    )
    .unwrap();
    h.state.runs.add_permits(1);
    assert_eq!(
        h.wait_until_timeout(&id, RunStatus::is_terminal, 120).await,
        RunStatus::Failed
    );
    let (_, failed) = h.get(&result(&p, &id), OPERATOR).await;
    assert_eq!(failed["failure"]["kind"], "EvidenceGap");
    assert!(failed["analysis"].is_null());
    let h = Harness::new(0);
    let (p, parent, s) = make_parent(&h, 80).await;
    let id = submit(&h, &p, &parent, &s).await;
    h.state.registry.begin_run(&id).unwrap();
    h.state
        .registry
        .finish_run(
            &id,
            RunOutcome::OrderFailure {
                kind: FailureKind::HandoffFailure,
            },
        )
        .unwrap();
    let (_, failed) = h.get(&result(&p, &id), OPERATOR).await;
    assert_eq!(failed["failure"]["kind"], "HandoffFailure");
    assert_eq!(failed["status"], "failed");
    assert!(failed["analysis"].is_null());
}
#[tokio::test]
async fn restart_requeues_child_with_exact_parent_references() {
    let h = Harness::new(0);
    let (p, parent, s) = make_parent(&h, 80).await;
    let id = submit(&h, &p, &parent, &s).await;
    h.state.registry.begin_run(&id).unwrap();
    let restarted = h.reopen();
    assert!(restarted
        .registry
        .recover_runs()
        .unwrap()
        .requeued
        .contains(&id));
    let record = restarted.registry.load_run(&id).unwrap();
    let projection = worker::run_isolated(
        &restarted.registry,
        &record,
        &restarted.config.worker_binary,
    )
    .unwrap();
    restarted
        .registry
        .finish_run(
            &id,
            RunOutcome::Analytical {
                projection: Box::new(projection),
            },
        )
        .unwrap();
    assert_eq!(
        restarted.registry.load_run(&id).unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(
        order::result(
            &restarted.registry,
            &restarted.registry.load_run(&id).unwrap()
        )
        .unwrap()["parent_run_id"],
        parent
    );
}
