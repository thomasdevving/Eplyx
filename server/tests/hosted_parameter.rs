mod common;
#[path = "common/current.rs"]
mod current_fixture;
use axum::{body::Body, http::StatusCode};
use common::*;
use current_fixture as fixture;
use eplyx_engine::{
    change::{Change, ChangeMetadata, ChangeSpec},
    parameter_change::{self as p, ConfigTarget, ExpectedCurrent, Operation, ParameterChange},
    replay::hash_bytes,
    standard_programs::token2022,
};
use eplyx_server::{
    artifacts::ArtifactClass,
    hosted::{worker, Input},
    project::Project,
    registry::{RunOutcome, RunStatus},
};
use serde_json::{json, Value};
use spl_token_2022_interface::{
    extension::{transfer_fee::TransferFeeConfig, BaseStateWithExtensions, StateWithExtensions},
    state::Mint,
};
use std::sync::Arc;
async fn parent() -> (Harness, Arc<fixture::Rpc>, String, String, ChangeSpec) {
    let mut h = Harness::new(1);
    let rpc = Arc::new(fixture::Rpc::new());
    Arc::get_mut(&mut h.state).unwrap().observation =
        Some(eplyx_server::hosted::observation::Service::new(rpc.clone()));
    let project = eplyx_server::ids::project();
    h.state
        .registry
        .create_project(&Project::analytical(&project, "Parameter changes").unwrap())
        .unwrap();
    let (code,observation)=h.post_json(&format!("/v1/projects/{project}/observations"),OPERATOR,json!({"request_key":"parameter-parent-observation","selection":{"cluster":"solana-mainnet","mint":rpc.mint,"sample_accounts":false,"public_owner":rpc.owner}})).await;
    assert_eq!(code, StatusCode::CREATED, "{observation}");
    let id = observation["observation"]["id"].as_str().unwrap();
    let mut request = rpc.request();
    request.amount_decimal = Some("0.00001".into());
    let (code, accepted) = h
        .post_json(
            &format!("/v1/projects/{project}/observations/{id}/path-checks"),
            OPERATOR,
            json!({"request_key":"parameter-parent-transfer","request":request}),
        )
        .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let run = accepted["run_id"].as_str().unwrap().to_string();
    assert_eq!(
        h.wait_until(&run, RunStatus::is_terminal).await,
        RunStatus::Completed
    );
    let Input::CurrentPath { capture, .. } = h
        .state
        .registry
        .hosted_input(&h.state.registry.load_run(&run).unwrap())
        .unwrap()
    else {
        unreachable!()
    };
    let input = eplyx_engine::path::current::parameter_input(
        &h.state
            .registry
            .artifacts()
            .get(ArtifactClass::Capture, &capture)
            .unwrap(),
    )
    .unwrap();
    let plan = input.validate().unwrap();
    let mint = &plan
        .accounts
        .iter()
        .find(|a| a.address == input.context.mint)
        .unwrap()
        .account;
    let state = StateWithExtensions::<Mint>::unpack(&mint.data).unwrap();
    let fee = state.get_extension::<TransferFeeConfig>().unwrap();
    let spec = ChangeSpec {
        schema_version: 1,
        change_spec_id: None,
        activation: None,
        metadata: ChangeMetadata::default(),
        change: Change::ProtocolParameterChange(Box::new(ParameterChange {
            target: ConfigTarget {
                program_id: token2022::PROGRAM_ID.into(),
                config_account: input.context.mint,
            },
            operation: Operation::Token2022ActiveNewerTransferFeeBasisPointsV1 {
                expected_current: ExpectedCurrent {
                    account_data_sha256: hash_bytes(&mint.data),
                    basis_points: fee.newer_transfer_fee.transfer_fee_basis_points.into(),
                    schedule_epoch: fee.newer_transfer_fee.epoch.into(),
                    maximum_fee_raw: fee.newer_transfer_fee.maximum_fee.into(),
                },
                proposed_basis_points: 200,
            },
        })),
    };
    (h, rpc, project, run, spec)
}
async fn submit(
    h: &Harness,
    project: &str,
    parent: &str,
    key: &str,
    spec: &ChangeSpec,
) -> (StatusCode, Value) {
    h.post_json(
        &format!("/v1/projects/{project}/runs/{parent}/parameter-changes"),
        OPERATOR,
        json!({"request_key":key,"change_spec":spec}),
    )
    .await
}
#[tokio::test]
async fn authenticated_offline_consequence_index_projection_reproduction_and_restart() {
    let (h, rpc, project, parent, spec) = parent().await;
    let before = rpc.calls.lock().unwrap().len();
    // Reopened service has no provider configured; submission and worker need only retained bytes.
    let restarted = h.reopen();
    let url = format!("/v1/projects/{project}/runs/{parent}/parameter-changes");
    let request = authed("POST", &url, OPERATOR)
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"request_key":"parameter-offline-restart","change_spec":spec}).to_string(),
        ))
        .unwrap();
    let (code, accepted) = h.send_on(restarted.clone(), request).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let run = accepted["run_id"].as_str().unwrap();
    for _ in 0..4000 {
        if restarted
            .registry
            .load_run(run)
            .unwrap()
            .status
            .is_terminal()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    let record = restarted.registry.load_run(run).unwrap();
    assert_eq!(record.status, RunStatus::Completed, "{record:?}");
    assert!(record.candidate_artifact.is_none());
    assert!(record.candidate_sha256.is_empty());
    assert_eq!(rpc.calls.lock().unwrap().len(), before);
    assert!(restarted
        .registry
        .change_run_ids(&project, &spec.id().unwrap())
        .unwrap()
        .contains(&run.to_string()));
    let projection = restarted.registry.analytical_projection(&record).unwrap();
    let report: Value = serde_json::from_str(&projection.report.text).unwrap();
    assert_eq!(report["status"], "semantic_consequence_observed");
    p::reproduce(&spec, &report).unwrap();
    assert_eq!(
        restarted
            .registry
            .load_change_spec(run, record.change.as_ref().unwrap())
            .unwrap()
            .id()
            .unwrap(),
        spec.id().unwrap()
    );
    let mut corrupt = projection.clone();
    let mut report = report.clone();
    report["derived_proposed_pre_state"]["proposed_mint_sha256"] = json!("0".repeat(64));
    corrupt.report =
        eplyx_engine::cloud::contract::Artifact::new(serde_json::to_vec(&report).unwrap()).unwrap();
    assert!(restarted
        .registry
        .verify_hosted_projection(&record, &corrupt)
        .is_err());
    let second = h.reopen();
    assert_eq!(
        second
            .registry
            .analytical_projection(&second.registry.load_run(run).unwrap())
            .unwrap()
            .report
            .sha256,
        projection.report.sha256
    );
}
#[tokio::test]
async fn no_consequence_and_stale_expectations_are_durable_factual_results() {
    let (h, _, project, parent, mut spec) = parent().await;
    for (key, stale) in [
        ("parameter-no-op-key", false),
        ("parameter-stale-key", true),
    ] {
        let Change::ProtocolParameterChange(c) = &mut spec.change else {
            unreachable!()
        };
        let Operation::Token2022ActiveNewerTransferFeeBasisPointsV1 {
            expected_current,
            proposed_basis_points,
        } = &mut c.operation;
        *proposed_basis_points = expected_current.basis_points;
        if stale {
            expected_current.account_data_sha256 = "0".repeat(64);
        }
        let (code, a) = submit(&h, &project, &parent, key, &spec).await;
        assert_eq!(code, StatusCode::ACCEPTED, "{a}");
        let id = a["run_id"].as_str().unwrap();
        assert_eq!(
            h.wait_until_timeout(id, RunStatus::is_terminal, 120).await,
            RunStatus::Completed
        );
        let r: Value = serde_json::from_slice(
            &h.state
                .registry
                .load_run_artifact(id, "report.json")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            r["status"],
            if stale {
                "current_state_mismatch"
            } else {
                "no_observed_consequence"
            }
        );
        assert_eq!(r["execution_performed"], !stale);
        p::verify(&spec, &r).unwrap();
    }
}
#[tokio::test]
async fn cross_project_unauthenticated_wrong_kind_and_arbitrary_inputs_are_refused() {
    let (h, rpc, project, parent, spec) = parent().await;
    let other = eplyx_server::ids::project();
    h.state
        .registry
        .create_project(&Project::analytical(&other, "Other").unwrap())
        .unwrap();
    assert_eq!(
        submit(&h, &other, &parent, "cross-project-denied", &spec)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let url = format!("/v1/projects/{project}/runs/{parent}/parameter-changes");
    assert_eq!(
        h.post_json(
            &url,
            "wrong-token",
            json!({"request_key":"unauthenticated-key","change_spec":spec})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let wrong = ChangeSpec::program_upgrade(token2022::PROGRAM_ID, b"different ELF");
    assert_eq!(
        submit(&h, &project, &parent, "wrong-spec-kind-key", &wrong)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    for field in ["proposed_mint_bytes", "execution", "elf", "byte_offset"] {
        let mut v = json!({"request_key":"arbitrary-input-denied","change_spec":spec});
        v[field] = json!("forbidden");
        let status = h.post_json(&url, OPERATOR, v).await.0;
        assert!(status.is_client_error());
    }
    let mut invalid = json!({"request_key":"explicit-activation-denied","change_spec":spec});
    invalid["change_spec"]["activation"] = Value::Null;
    assert!(h
        .post_json(&url, OPERATOR, invalid)
        .await
        .0
        .is_client_error());
    // A valid retained current market case is outside the parameter transfer contract.
    let Input::CurrentPath {
        parent_observation, ..
    } = h
        .state
        .registry
        .hosted_input(&h.state.registry.load_run(&parent).unwrap())
        .unwrap()
    else {
        unreachable!()
    };
    let mut market = rpc.request();
    market.path = eplyx_engine::path::ExitPathType::SecondaryMarketExit;
    market.recipient.clear();
    market.output_mint = Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into());
    market.minimum_output_decimal = Some("0.000001".into());
    let (code, accepted) = h
        .post_json(
            &format!("/v1/projects/{project}/observations/{parent_observation}/path-checks"),
            OPERATOR,
            json!({"request_key":"unsupported-parameter-market-parent","request":market}),
        )
        .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let market_run = accepted["run_id"].as_str().unwrap();
    assert_eq!(
        h.wait_until_timeout(market_run, RunStatus::is_terminal, 120)
            .await,
        RunStatus::Completed
    );
    assert_eq!(
        submit(
            &h,
            &project,
            market_run,
            "unsupported-retained-capture",
            &spec
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn durable_queue_recovery_preserves_exact_inputs_without_provider() {
    let (h, _, project, parent, spec) = parent().await;
    let permit = h.state.runs.acquire().await.unwrap();
    let (code, a) = submit(&h, &project, &parent, "parameter-durable-key", &spec).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    let id = a["run_id"].as_str().unwrap();
    assert!(h.state.registry.begin_run(id).unwrap());
    let restart = h.reopen();
    assert!(restart
        .registry
        .recover_runs()
        .unwrap()
        .requeued
        .contains(&id.to_string()));
    assert!(restart.registry.begin_run(id).unwrap());
    let record = restart.registry.load_run(id).unwrap();
    let projection =
        worker::run_isolated(&restart.registry, &record, &restart.config.worker_binary).unwrap();
    restart
        .registry
        .finish_run(
            id,
            RunOutcome::Analytical {
                projection: Box::new(projection),
            },
        )
        .unwrap();
    assert_eq!(
        restart.registry.load_run(id).unwrap().status,
        RunStatus::Completed
    );
    // Discard the blocked old process task; the durable restart already completed it.
    drop(permit);
}
