mod common;
use axum::{body::Body, http::StatusCode};
use common::*;
use eplyx_engine::{
    change::{Change, ChangeMetadata, ChangeSpec},
    parameter_change::{self as p, stake_pool as s, ConfigTarget, Operation, ParameterChange},
    replay::hash_bytes,
};
use eplyx_server::{
    hosted::{self, Input},
    registry::RunStatus,
};
use serde_json::{json, Value};
const RECORD: &str = "mainnet-spl-stake-pool-151010f709e113e7";
fn spec(input: &s::Input) -> ChangeSpec {
    let pool = input
        .record
        .accounts
        .iter()
        .find(|a| a.label == "stake-pool")
        .unwrap();
    let state = eplyx_engine::protocol::stake_pool::StakePool::decode(&pool.account.data).unwrap();
    ChangeSpec {
        schema_version: 1,
        change_spec_id: None,
        activation: None,
        metadata: ChangeMetadata::default(),
        change: Change::ProtocolParameterChange(Box::new(ParameterChange {
            target: ConfigTarget {
                program_id: input.record.program_id.clone(),
                config_account: pool.address.clone(),
            },
            operation: Operation::SplStakePoolSolDepositFeeV1 {
                expected_current: s::ExpectedCurrent {
                    account_data_sha256: hash_bytes(&pool.account.data),
                    numerator: state.sol_deposit_fee.numerator,
                    denominator: state.sol_deposit_fee.denominator,
                    sol_referral_fee_percent: state.sol_referral_fee,
                    last_update_epoch: state.last_update_epoch,
                },
                proposed_fee: s::RationalFee {
                    numerator: 1,
                    denominator: 100,
                },
            },
        })),
    }
}
async fn parent() -> (Harness, String, String, ChangeSpec) {
    let h = Harness::new(1);
    let (code,body)=h.post_json("/v1/projects",OPERATOR,json!({"name":"Stake Pool retained parameter","program_id":STAKE_POOL_PROGRAM,"adapter_id":eplyx_server::project::AdapterId::for_program(STAKE_POOL_PROGRAM).to_string()})).await;
    assert_eq!(code, StatusCode::CREATED, "{body}");
    let project = body["project_id"].as_str().unwrap().to_string();
    let bundle = h
        .upload_bundle_from(&project, std::path::Path::new("../deploy/bundle"))
        .await;
    let (code, body) = h
        .post_json(
            &format!("/v1/projects/{project}/bundles/{bundle}/activate"),
            OPERATOR,
            json!({}),
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    // A different candidate is intentional: it must never become the parameter
    // analysis's historical baseline executable.
    let bytes = std::fs::read("../artifacts/fixture_stake_pool_v2.so").unwrap();
    let (content, body) = candidate_multipart(&bytes, None);
    let (code, body) = h
        .send(
            authed("POST", &format!("/v1/projects/{project}/checks"), OPERATOR)
                .header("content-type", content)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{body}");
    let run = body["run_id"].as_str().unwrap().to_string();
    assert_eq!(
        h.wait_until(&run, RunStatus::is_terminal).await,
        RunStatus::Failed
    );
    let input = hosted::parameter::historical_input(
        &h.state.registry,
        &h.state.registry.load_run(&run).unwrap(),
        RECORD,
    )
    .unwrap();
    let s = spec(&input);
    (h, project, run, s)
}
async fn submit(
    h: &Harness,
    project: &str,
    parent: &str,
    s: &ChangeSpec,
    record: Option<&str>,
    key: &str,
) -> (StatusCode, Value) {
    let mut request = json!({"request_key":format!("stake-parameter-{key}"),"change_spec":s});
    if let Some(record) = record {
        request["record_id"] = record.into();
    }
    h.post_json(
        &format!("/v1/projects/{project}/runs/{parent}/parameter-changes"),
        OPERATOR,
        request,
    )
    .await
}
#[tokio::test]
async fn retained_parent_explicit_selection_authorization_and_wrong_inputs() {
    let (h, project, parent, s) = parent().await;
    for record in [
        None,
        Some("not-a-record"),
        Some("mainnet-spl-stake-pool-0c4fe6c80dd53827"),
    ] {
        let (code, body) = submit(&h, &project, &parent, &s, record, "selection-refused").await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    }
    let other = h.create_project("Other project").await;
    let (code, _) = submit(&h, &other, &parent, &s, Some(RECORD), "cross-project").await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    let token = h.create_token(&other, "other").await;
    let (code, _) = h
        .post_json(
            &format!("/v1/projects/{project}/runs/{parent}/parameter-changes"),
            &token,
            json!({"request_key":"denied","change_spec":s,"record_id":RECORD}),
        )
        .await;
    assert_eq!(code, StatusCode::UNAUTHORIZED);
    let mut wrong = s.clone();
    if let Change::ProtocolParameterChange(c) = &mut wrong.change {
        c.operation = Operation::Token2022ActiveNewerTransferFeeBasisPointsV1 {
            expected_current: p::ExpectedCurrent {
                account_data_sha256: "f".repeat(64),
                basis_points: 0,
                schedule_epoch: 0,
                maximum_fee_raw: 0,
            },
            proposed_basis_points: 1,
        };
        c.target.program_id = eplyx_engine::standard_programs::token2022::PROGRAM_ID.into();
    }
    let (code, _) = submit(
        &h,
        &project,
        &parent,
        &wrong,
        Some(RECORD),
        "wrong-operation",
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let (code, accepted) = submit(&h, &project, &parent, &s, Some(RECORD), "real-parameter").await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let run = accepted["run_id"].as_str().unwrap();
    assert_eq!(
        h.wait_until(run, RunStatus::is_terminal).await,
        RunStatus::Completed
    );
    let metadata = h.state.registry.load_run(run).unwrap();
    let projection = h.state.registry.analytical_projection(&metadata).unwrap();
    let report: Value = serde_json::from_str(&projection.report.text).unwrap();
    assert_eq!(report["status"], "semantic_consequence_observed");
    assert_eq!(report["change"]["change_spec_id"], s.id().unwrap());
    assert_eq!(
        report["retained_input"]["record"]["current_program_sha256"],
        "ec2dfefaa70d560754a0000f39bd2cabc192b895d36205b3c428f601b6e1d7e1"
    );
    assert!(metadata.candidate_artifact.is_none());
    assert!(h.state.observation.is_none());
    assert_eq!(
        h.reopen()
            .registry
            .analytical_projection(&metadata)
            .unwrap()
            .report
            .text,
        projection.report.text
    );
    let stored = h.state.registry.hosted_input(&metadata).unwrap();
    let Input::ProtocolParameterChange {
        historical: Some(source),
        ..
    } = &stored
    else {
        panic!("historical input expected")
    };
    assert_eq!(source.record_id, RECORD);
    for field in [
        "bundle_sha256",
        "record_sha256",
        "baseline_sha256",
        "record_id",
    ] {
        let mut tampered = serde_json::to_value(&stored).unwrap();
        tampered["historical"][field] = json!("f".repeat(64));
        let tampered: Input = serde_json::from_value(tampered).unwrap();
        assert!(
            hosted::parameter::validate_parent(&h.state.registry, &project, &tampered).is_err(),
            "{field}"
        );
    }
    for field in [
        "proposed_pool_data",
        "program_bytes",
        "config_result",
        "findings",
    ] {
        let mut request = json!({"request_key":format!("refuse-caller-{field}"),"change_spec":s,"record_id":RECORD});
        request[field] = json!("caller-controlled");
        let (code, _) = h
            .post_json(
                &format!("/v1/projects/{project}/runs/{parent}/parameter-changes"),
                OPERATOR,
                request,
            )
            .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
    }
    // The authenticated read must reject an indexed proposal mismatch.
    let mut value = serde_json::to_value(&metadata).unwrap();
    value["change"]["change_spec_id"] = json!("f".repeat(64));
    std::fs::write(
        h.run_file(run, "metadata.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(h
        .state
        .registry
        .analytical_projection(&h.state.registry.load_run(run).unwrap())
        .is_err());
}
#[tokio::test]
async fn queued_historical_input_is_pinned_durable_and_recovered_without_provider() {
    let (h, project, parent, s) = parent().await;
    let guard = h.state.runs.acquire().await.unwrap();
    let (code, accepted) = submit(
        &h,
        &project,
        &parent,
        &s,
        Some(RECORD),
        "durable-historical",
    )
    .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let run = accepted["run_id"].as_str().unwrap();
    assert_eq!(
        h.state.registry.load_run(run).unwrap().status,
        RunStatus::Queued
    );
    let original = h
        .state
        .registry
        .hosted_input(&h.state.registry.load_run(run).unwrap())
        .unwrap();
    let mut project_state = h.state.registry.load_project(&project).unwrap();
    project_state.active_bundle = None;
    h.state.registry.save_project(&project_state).unwrap();
    let reopened = h.reopen();
    assert!(reopened.observation.is_none());
    let recovery = reopened.registry.recover_runs().unwrap();
    assert!(recovery.requeued.iter().any(|id| id == run));
    assert_eq!(
        serde_json::to_value(
            reopened
                .registry
                .hosted_input(&reopened.registry.load_run(run).unwrap())
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    eplyx_server::worker::spawn(reopened.clone(), run.into());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let m = reopened.registry.load_run(run).unwrap();
        if m.status.is_terminal() {
            assert_eq!(m.status, RunStatus::Completed, "{:?}", m.detail);
            let p = reopened.registry.analytical_projection(&m).unwrap();
            let r: Value = serde_json::from_str(&p.report.text).unwrap();
            assert_eq!(r["status"], "semantic_consequence_observed");
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    drop(guard);
}
