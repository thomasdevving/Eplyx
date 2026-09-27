mod common;
#[path = "common/current.rs"]
mod current_fixture;
use axum::{body::Body, http::StatusCode};
use common::*;
use current_fixture::Rpc;
use eplyx_server::{project::Project, registry::RunStatus};
use serde_json::{json, Value};
use std::sync::Arc;
fn setup() -> (Harness, Arc<Rpc>, String) {
    let mut h = Harness::new(1);
    let rpc = Arc::new(Rpc::new());
    Arc::get_mut(&mut h.state).unwrap().observation = Some(
        eplyx_server::hosted::observation::Service::new(rpc.clone())
            .with_candidate(
                std::fs::read(eplyx_engine::repo_root().join("artifacts/eplyx_token_migration.so"))
                    .unwrap(),
            )
            .unwrap(),
    );
    let id = eplyx_server::ids::project();
    h.state
        .registry
        .create_project(&Project::analytical(&id, "Current checks").unwrap())
        .unwrap();
    (h, rpc, id)
}
async fn post(h: &Harness, url: &str, value: Value) -> (StatusCode, Value) {
    h.send(
        authed("POST", url, OPERATOR)
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .unwrap(),
    )
    .await
}
fn selection(rpc: &Rpc) -> Value {
    json!({"cluster":"solana-mainnet","mint":rpc.mint,"catalogue_version":null,"sample_accounts":false,"public_owner":rpc.owner})
}
async fn observe(h: &Harness, rpc: &Rpc, p: &str, key: &str) -> Value {
    let (code, v) = post(
        h,
        &format!("/v1/projects/{p}/observations"),
        json!({"request_key":key,"selection":selection(rpc)}),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "{v}");
    v["observation"].clone()
}
#[tokio::test]
async fn observation_is_immutable_project_scoped_and_refresh_never_inherits_execution() {
    let (h, rpc, p) = setup();
    let one = observe(&h, &rpc, &p, "first-observation-key").await;
    let id = one["id"].as_str().unwrap();
    let count = rpc.calls.lock().unwrap().len();
    assert_eq!(observe(&h, &rpc, &p, "first-observation-key").await, one);
    assert_eq!(rpc.calls.lock().unwrap().len(), count);
    assert!(h.state.registry.project_run_ids(&p).unwrap().is_empty());
    let (_, view) = h
        .get(&format!("/v1/projects/{p}/observations/{id}"), OPERATOR)
        .await;
    assert_eq!(view["result"]["selection"]["mint"], rpc.mint);
    assert_eq!(view["result"]["signer_possession_known"], false);
    assert_eq!(view["result"]["authorization"], false);
    assert!(view["result"]["paths"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["status"] == "NotTested"));
    let (code, accepted) = post(
        &h,
        &format!("/v1/projects/{p}/observations/{id}/analyse"),
        json!({"request_key":"analyse-observation-key"}),
    )
    .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let run = accepted["run_id"].as_str().unwrap();
    assert_eq!(
        h.wait_until(run, RunStatus::is_terminal).await,
        RunStatus::Completed
    );
    assert_eq!(rpc.calls.lock().unwrap().len(), count);
    let (_, again) = post(
        &h,
        &format!("/v1/projects/{p}/observations/{id}/analyse"),
        json!({"request_key":"analyse-observation-key"}),
    )
    .await;
    assert_eq!(again["run_id"], run);
    let fresh = observe(&h, &rpc, &p, "refresh-observation-key").await;
    assert_ne!(fresh["id"], id);
    assert_eq!(
        one,
        h.state
            .registry
            .load_observation(&p, id)
            .map(|v| serde_json::to_value(v).unwrap())
            .unwrap()
    );
    let other = eplyx_server::ids::project();
    h.state
        .registry
        .create_project(&Project::analytical(&other, "Other").unwrap())
        .unwrap();
    assert_eq!(
        h.get(&format!("/v1/projects/{other}/observations/{id}"), OPERATOR)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn typed_transfer_executes_exact_frozen_bytes_in_offline_child() {
    let (h, rpc, p) = setup();
    let observation = observe(&h, &rpc, &p, "transfer-observation").await;
    let id = observation["id"].as_str().unwrap();
    let request = json!({"request_key":"transfer-request-key","request":rpc.request()});
    let (code, accepted) = post(
        &h,
        &format!("/v1/projects/{p}/observations/{id}/path-checks"),
        request.clone(),
    )
    .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let run = accepted["run_id"].as_str().unwrap();
    let count = rpc.calls.lock().unwrap().len();
    assert_eq!(
        h.wait_until(run, RunStatus::is_terminal).await,
        RunStatus::Completed
    );
    let report: Value = serde_json::from_slice(
        &h.state
            .registry
            .load_run_artifact(run, "report.json")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["status"], "Proven");
    assert_eq!(report["execution_performed"], true);
    assert_eq!(report["funds_moved"], false);
    assert_eq!(report["signer_possession_known"], false);
    assert_eq!(report["authorization"], false);
    assert_eq!(
        report["wallet_capture_sha256"],
        observation["capture"]["sha256"]
    );
    assert_eq!(rpc.calls.lock().unwrap().len(), count);
    let (_, retry) = post(
        &h,
        &format!("/v1/projects/{p}/observations/{id}/path-checks"),
        request,
    )
    .await;
    assert_eq!(retry["run_id"], run);
    assert_eq!(rpc.calls.lock().unwrap().len(), count);
    let fresh = observe(&h, &rpc, &p, "refresh-after-transfer").await;
    let (_, view) = h
        .get(
            &format!(
                "/v1/projects/{p}/observations/{}",
                fresh["id"].as_str().unwrap()
            ),
            OPERATOR,
        )
        .await;
    assert!(view["result"]["paths"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["status"] == "NotTested"));
    assert!(rpc
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|m| !m.to_lowercase().contains("transaction")));
}
#[tokio::test]
async fn forged_program_status_provider_and_cross_focus_are_refused_before_observation() {
    let (h, rpc, p) = setup();
    let url = format!("/v1/projects/{p}/observations");
    for field in ["rpc_url", "status", "program", "instructions", "source_url"] {
        let mut s = selection(&rpc);
        s[field] = json!("forged");
        assert_eq!(
            post(
                &h,
                &url,
                json!({"request_key":"invalid-observation-key","selection":s})
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let mut s = selection(&rpc);
    s["mint"] = json!("O".repeat(44));
    assert_eq!(
        post(
            &h,
            &url,
            json!({"request_key":"invalid-observation-key","selection":s})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert!(rpc.calls.lock().unwrap().is_empty());
    let observation = observe(&h, &rpc, &p, "safe-observation-key").await;
    let count = rpc.calls.lock().unwrap().len();
    let mut request = rpc.request();
    request.source = "11111111111111111111111111111111".into();
    assert_eq!(
        post(
            &h,
            &format!("{url}/{}/path-checks", observation["id"].as_str().unwrap()),
            json!({"request_key":"bad-focus-check-key","request":request})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(rpc.calls.lock().unwrap().len(), count);
}

#[tokio::test]
async fn candidate_and_proposed_scenario_keep_exact_evidence_separate_and_retry_without_rpc() {
    let (h, rpc, p) = setup();
    let permit = h.state.runs.acquire().await.unwrap();
    let observation = observe(&h, &rpc, &p, "candidate-observation-key").await;
    let id = observation["id"].as_str().unwrap();
    let request = json!({"source":rpc.source,"replacement_mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","amount_mode":"Custom","amount_decimal":"0.0000001","numerator":"1","denominator":"1","rounding":"floor","fee_bps":0,"reserve_raw":"1000"});
    let url = format!("/v1/projects/{p}/observations/{id}/candidate-checks");
    let body = json!({"request_key":"candidate-execution-key","request":request});
    let (code, accepted) = post(&h, &url, body.clone()).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let run = accepted["run_id"].as_str().unwrap();
    let before = rpc.calls.lock().unwrap().len();
    let pending = post(
        &h,
        &format!("/v1/projects/{p}/observations/{id}/stress-checks"),
        json!({"request_key":"pending-candidate-stress","candidate_run_id":run}),
    )
    .await;
    assert_eq!(pending.0, StatusCode::BAD_REQUEST);
    assert_eq!(rpc.calls.lock().unwrap().len(), before);
    drop(permit);
    let status = h.wait_until(run, RunStatus::is_terminal).await;
    if status != RunStatus::Completed {
        let work = tempfile::tempdir().unwrap();
        let input = h
            .state
            .registry
            .hosted_input(&h.state.registry.load_run(run).unwrap())
            .unwrap();
        eplyx_server::hosted::worker::stage(&h.state.registry, &input, work.path()).unwrap();
        std::fs::write(
            work.path().join("request.json"),
            serde_json::to_vec(&eplyx_server::hosted::worker::Request {
                run_id: run.into(),
                input,
            })
            .unwrap(),
        )
        .unwrap();
        let diagnostic = std::process::Command::new(env!("CARGO_BIN_EXE_eplyx-server"))
            .arg("offline-analysis")
            .arg(work.path())
            .env_clear()
            .output()
            .unwrap();
        let projection = std::fs::read(work.path().join("projection.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<eplyx_server::projection::Projection>(&b).ok());
        let verification = projection.map(|p| {
            h.state
                .registry
                .verify_hosted_projection(&h.state.registry.load_run(run).unwrap(), &p)
        });
        panic!(
            "candidate worker {:?}: {}, verification {:?}",
            status,
            String::from_utf8_lossy(&diagnostic.stderr),
            verification
        );
    }
    let count = rpc.calls.lock().unwrap().len();
    let (_, again) = post(&h, &url, body).await;
    assert_eq!(again["run_id"], run);
    assert_eq!(rpc.calls.lock().unwrap().len(), count);
    let (_, report) = h
        .get(&format!("/v1/runs/{run}/report.json"), OPERATOR)
        .await;
    assert_eq!(report["status"], "Proven", "{report}");
    assert_eq!(report["amount_raw"], "100");
    assert_eq!(report["official_transition"], "NotTested");
    let proposal = json!({"request_key":"proposed-scenario-key","request":{"source":rpc.source,"successor_mint":request["replacement_mint"],"effective_at":"2031-01-01T00:00:00Z","deadline":"2032-01-01T00:00:00Z","post_deadline":"TransitionStillRequired","assurance":"FullTransition","check_ids":[],"conversion_check_id":run}});
    let endpoint = format!("/v1/projects/{p}/observations/{id}/preflights");
    let (code, accepted) = post(&h, &endpoint, proposal.clone()).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let preflight = accepted["run_id"].as_str().unwrap();
    assert_eq!(
        h.wait_until(preflight, RunStatus::is_terminal).await,
        RunStatus::Completed
    );
    let (_, report) = h
        .get(&format!("/v1/runs/{preflight}/report.json"), OPERATOR)
        .await;
    assert_eq!(report["replacement_conversion"]["status"], "Proven");
    assert_eq!(
        report["views"][1]["readiness"]["candidate_plan"]["status"],
        "Ready"
    );
    assert_eq!(
        report["views"][1]["readiness"]["full_transition"]["status"],
        "Incomplete"
    );
    assert!(report["views"][0]["readiness"].is_null());
    assert!(report["population_readiness"].is_null());
    assert_eq!(report["authorization"], false);
    let count = rpc.calls.lock().unwrap().len();
    let (_, again) = post(&h, &endpoint, proposal.clone()).await;
    assert_eq!(again["run_id"], preflight);
    assert_eq!(rpc.calls.lock().unwrap().len(), count);
    let stress_body = json!({"request_key":"candidate-stress-key","candidate_run_id":run});
    let stress_url = format!("/v1/projects/{p}/observations/{id}/stress-checks");
    let (code, saved) = post(&h, &stress_url, stress_body.clone()).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{saved}");
    let stress = saved["run_id"].as_str().unwrap();
    assert_eq!(
        h.wait_until(stress, RunStatus::is_terminal).await,
        RunStatus::Completed
    );
    let count = rpc.calls.lock().unwrap().len();
    let (_, repeated) = post(&h, &stress_url, stress_body).await;
    assert_eq!(repeated["run_id"], stress);
    assert_eq!(rpc.calls.lock().unwrap().len(), count);
    let (_, stress_report) = h
        .get(&format!("/v1/runs/{stress}/report.json"), OPERATOR)
        .await;
    assert_eq!(stress_report["kind"], "current-stress");
    assert_eq!(stress_report["report"]["official_transition"], "NotTested");
    assert!(!stress_report["report"]["results"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        stress_report["report"]["coverage"]["population_rollout_readiness"],
        "Incomplete"
    );
    let fresh = observe(&h, &rpc, &p, "candidate-refresh-key").await;
    let other = fresh["id"].as_str().unwrap();
    let (code, _) = post(
        &h,
        &format!("/v1/projects/{p}/observations/{other}/preflights"),
        proposal,
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn candidate_scenario_and_stress_reject_injected_execution_and_invalid_scope_before_rpc() {
    let (h, rpc, p) = setup();
    let observation = observe(&h, &rpc, &p, "typed-proposal-observation").await;
    let id = observation["id"].as_str().unwrap();
    let count = rpc.calls.lock().unwrap().len();
    let candidate = json!({"source":rpc.source,"replacement_mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","amount_mode":"Custom","amount_decimal":"0.0000001","numerator":"1","denominator":"1","rounding":"floor","fee_bps":0,"reserve_raw":"1000"});
    let proposal = json!({"source":rpc.source,"successor_mint":null,"effective_at":"2031-01-01T00:00:00Z","deadline":null,"post_deadline":null,"assurance":"FullTransition","check_ids":[],"conversion_check_id":null});
    for (endpoint, terms) in [
        ("candidate-checks", candidate.clone()),
        ("preflights", proposal.clone()),
    ] {
        for field in [
            "program",
            "instructions",
            "account_metas",
            "transaction",
            "rpc_url",
            "path",
            "status",
            "proof",
            "captured_state",
        ] {
            let mut injected = terms.clone();
            injected[field] = json!("forged");
            let (code, _) = post(
                &h,
                &format!("/v1/projects/{p}/observations/{id}/{endpoint}"),
                json!({"request_key":"injected-proposal-key","request":injected}),
            )
            .await;
            assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{endpoint}/{field}");
        }
    }
    for (field, value) in [
        ("numerator", json!("0")),
        ("denominator", json!("0")),
        ("rounding", json!("Bankers")),
        ("fee_bps", json!(10001)),
        ("fee_bps", json!(-1)),
        ("amount_mode", json!("Everything")),
        ("amount_decimal", json!("1e9")),
        ("amount_decimal", json!("-1")),
        ("reserve_raw", json!("1.5")),
        ("source", json!("11111111111111111111111111111111")),
    ] {
        let mut terms = candidate.clone();
        terms[field] = value;
        let (code, _) = post(
            &h,
            &format!("/v1/projects/{p}/observations/{id}/candidate-checks"),
            json!({"request_key":"invalid-candidate-key","request":terms}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::UNPROCESSABLE_ENTITY,
            "{field}: {code}"
        );
    }
    for (field, value) in [
        ("effective_at", json!("2000-01-01T00:00:00Z")),
        ("deadline", json!("2030-01-01T00:00:00Z")),
        ("post_deadline", json!("Blocked")),
        ("assurance", json!("Population")),
        ("source", json!("11111111111111111111111111111111")),
    ] {
        let mut terms = proposal.clone();
        terms[field] = value;
        let (code, _) = post(
            &h,
            &format!("/v1/projects/{p}/observations/{id}/preflights"),
            json!({"request_key":"invalid-preflight-key","request":terms}),
        )
        .await;
        assert!(
            code == StatusCode::BAD_REQUEST || code == StatusCode::UNPROCESSABLE_ENTITY,
            "{field}: {code}"
        );
    }
    for field in [
        "max_selected_cases",
        "population",
        "program",
        "status",
        "case_ids",
    ] {
        let mut terms =
            json!({"request_key":"injected-stress-key","candidate_run_id":"run_unknown"});
        terms[field] = json!(1);
        let (code, _) = post(
            &h,
            &format!("/v1/projects/{p}/observations/{id}/stress-checks"),
            terms,
        )
        .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
    }
    let (code, _) = post(
        &h,
        &format!("/v1/projects/{p}/observations/{id}/preflights"),
        json!({"request_key":"oversized-proposal-key","request":{"source":"x".repeat(70*1024)}}),
    )
    .await;
    assert_eq!(code, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(rpc.calls.lock().unwrap().len(), count);
    assert!(h.state.registry.project_run_ids(&p).unwrap().is_empty());
}
