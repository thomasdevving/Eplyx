//! Actual VM-backed hosted integration using the existing qualified Step10B ELF.
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use eplyx_engine::{change::ChangeSpec, interaction as engine};
use eplyx_server::{
    hosted::{self, interaction, Input},
    registry::{RunOutcome, RunStatus},
};
use serde_json::{json, Value};
use std::path::Path;
const RECORD: &str = "mainnet-spl-stake-pool-151010f709e113e7";
fn parameter() -> ChangeSpec {
    ChangeSpec::parse(include_bytes!(
        "../../docs/examples/stake-pool-parameter-change.json"
    ))
    .unwrap()
}
async fn parent(h: &Harness, which: &str, authority: bool) -> (String, String) {
    let (_,body)=h.post_json("/v1/projects",OPERATOR,json!({"name":"Interaction parent","program_id":STAKE_POOL_PROGRAM,"adapter_id":eplyx_server::project::AdapterId::for_program(STAKE_POOL_PROGRAM).to_string()})).await;
    let project = body["project_id"].as_str().unwrap().to_owned();
    let bundle = h
        .upload_bundle_from(&project, Path::new("../deploy/bundle"))
        .await;
    assert_eq!(
        h.post_json(
            &format!("/v1/projects/{project}/bundles/{bundle}/activate"),
            OPERATOR,
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    let bytes = if which == "same" {
        std::fs::read(
            h.state
                .registry
                .open_bundle(
                    &h.state
                        .registry
                        .load_bundle_record(&project, &bundle)
                        .unwrap()
                        .bundle_sha256,
                )
                .unwrap()
                .baseline(),
        )
        .unwrap()
    } else {
        std::fs::read(format!("../artifacts/{which}")).unwrap()
    };
    let spec =
        serde_json::to_value(ChangeSpec::program_upgrade(STAKE_POOL_PROGRAM, &bytes)).unwrap();

    let (content, mut body) = candidate_multipart(&bytes, None);
    let end = b"--eplyxtestboundary--\r\n";
    assert!(body.ends_with(end));
    body.truncate(body.len() - end.len());
    body.extend_from_slice(
        b"--eplyxtestboundary\r\nContent-Disposition: form-data; name=\"change_spec\"\r\n\r\n",
    );
    body.extend_from_slice(serde_json::to_string(&spec).unwrap().as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(end);
    let (code, body) = h
        .send(
            authed("POST", &format!("/v1/projects/{project}/checks"), OPERATOR)
                .header("content-type", content)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{body}");
    let run = body["run_id"].as_str().unwrap().to_owned();
    h.wait_until(&run, RunStatus::is_terminal).await;
    assert!(h.state.registry.load_run(&run).unwrap().report_available);
    // Model an evidence-bearing legacy occurrence with an unsupported expectation.
    // This is a storage-contract test, not new VM qualification or a live write.
    if authority {
        let mut metadata = h.state.registry.load_run(&run).unwrap();
        let mut value = serde_json::to_value(
            h.state
                .registry
                .load_change_spec(&run, metadata.change.as_ref().unwrap())
                .unwrap(),
        )
        .unwrap();
        value["change"]["expected_upgrade_authority"] = json!(STAKE_POOL_PROGRAM);
        value.as_object_mut().unwrap().remove("change_spec_id");
        let changed: ChangeSpec = serde_json::from_value(value).unwrap();
        metadata.change = Some(
            eplyx_server::registry::RunChange::of(
                &changed,
                eplyx_server::registry::ChangeOrigin::Submitted,
            )
            .unwrap(),
        );
        std::fs::write(
            h.state
                .registry
                .storage()
                .run_dir(&run)
                .unwrap()
                .join("change_spec.json"),
            changed.to_document().unwrap(),
        )
        .unwrap();
        let root = h.state.registry.storage().run_dir(&run).unwrap();
        std::fs::write(
            root.join("metadata.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let mut report: Value = serde_json::from_slice(
            &h.state
                .registry
                .load_run_artifact(&run, "report.json")
                .unwrap(),
        )
        .unwrap();
        report["change"]["change_spec_id"] = json!(changed.id().unwrap());
        std::fs::write(
            root.join("report.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
    }

    (project, run)
}
fn route(project: &str, parent: &str) -> String {
    format!("/v1/projects/{project}/runs/{parent}/interactions")
}
async fn submit(
    h: &Harness,
    project: &str,
    parent: &str,
    p: &ChangeSpec,
    key: &str,
) -> (StatusCode, Value) {
    h.post_json(
        &route(project, parent),
        OPERATOR,
        json!({"request_key":key,"record_id":RECORD,"parameter_change_spec":p}),
    )
    .await
}
#[tokio::test]
async fn durable_distinct_fixture_recovery_download_and_tampering() {
    let h = Harness::new(1);
    let (project, parent) = parent(&h, "fixture_stake_pool_config_v2.so", false).await;
    let r = h.state.registry.load_run(&parent).unwrap();
    // A negative verdict is evidence-bearing, never an interaction admission gate.
    assert_eq!(r.status, RunStatus::Failed);
    let before = h.state.registry.project_run_ids(&project).unwrap();
    let (code, e) = h
        .get(
            &format!("{}/eligibility", route(&project, &parent)),
            OPERATOR,
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{e}");
    assert_eq!(e["eligible"], true);
    assert!(e["records"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["record_id"] == RECORD));
    assert_eq!(before, h.state.registry.project_run_ids(&project).unwrap());
    let text = serde_json::to_string(&e).unwrap();
    for private in ["pre_accounts", "data_hex", "elf\"", "/Users/", "provider"] {
        assert!(!text.contains(private), "{private}");
    }
    let p = parameter();
    let (code, preview) = h
        .post_json(
            &format!("{}/preview", route(&project, &parent)),
            OPERATOR,
            json!({"record_id":RECORD,"parameter_change_spec":p}),
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{preview}");
    assert_eq!(preview["parameter_change_spec_id"], p.id().unwrap());
    let permit = h.state.runs.acquire().await.unwrap();
    let (code, accepted) = submit(&h, &project, &parent, &p, "interaction-durable-key").await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let child = accepted["run_id"].as_str().unwrap();
    let record = h.state.registry.load_run(child).unwrap();
    assert_eq!(record.status, RunStatus::Queued);
    assert!(record.change.is_none());
    let input = h.state.registry.hosted_input(&record).unwrap();
    let Input::UpgradeParameterInteraction { binding, .. } = &input else {
        panic!("interaction input")
    };
    assert_eq!(binding.parameter_change_spec_id, p.id().unwrap());
    let contract: Value = serde_json::from_slice(
        &h.state
            .registry
            .document_bytes(&binding.input_contract)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        eplyx_engine::canonical::digest(&contract).unwrap(),
        binding.analysis_input_sha256
    );
    assert!(contract["runtime"]["revision"].is_string());
    assert_eq!(binding.bundle_sha256, r.bundle_sha256);
    assert_eq!(
        binding.analysis_input_sha256,
        preview["analysis_input_sha256"]
    );
    let (code, retry) = submit(&h, &project, &parent, &p, "interaction-durable-key").await;
    assert_eq!(code, StatusCode::ACCEPTED, "{retry}");
    assert_eq!(retry["run_id"], child);
    let mut conflict = serde_json::to_value(&p).unwrap();
    conflict["change"]["operation"]["proposed_fee"]["numerator"] = json!("0");
    let conflict: ChangeSpec = serde_json::from_value(conflict).unwrap();
    assert_eq!(
        submit(&h, &project, &parent, &conflict, "interaction-durable-key")
            .await
            .0,
        StatusCode::CONFLICT
    );
    // Clear the active bundle pointer. Recovery still has the exact accepted refs.
    let path = h
        .state
        .registry
        .storage()
        .project_dir(&project)
        .unwrap()
        .join("project.json");
    let mut project_doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    project_doc["active_bundle"] = Value::Null;
    std::fs::write(&path, serde_json::to_vec(&project_doc).unwrap()).unwrap();
    let reopened = h.reopen();
    let recovery = reopened.registry.recover_runs().unwrap();
    assert!(recovery.requeued.contains(&child.to_owned()));
    assert_eq!(
        serde_json::to_value(reopened.registry.hosted_input(&record).unwrap()).unwrap(),
        serde_json::to_value(&input).unwrap()
    );
    assert!(reopened.observation.is_none() && reopened.governance.is_none());
    // Execute the real isolated offline entry point on recovered immutable refs.
    assert!(reopened.registry.begin_run(child).unwrap());
    let projection = hosted::worker::run_isolated(
        &reopened.registry,
        &reopened.registry.load_run(child).unwrap(),
        &reopened.config.worker_binary,
    )
    .unwrap();
    reopened
        .registry
        .finish_run(
            child,
            RunOutcome::Analytical {
                projection: Box::new(projection),
            },
        )
        .unwrap();
    drop(permit);
    h.wait_until(child, RunStatus::is_terminal).await;
    let (code, result) = h
        .get(
            &format!("/v1/projects/{project}/interactions/{child}"),
            OPERATOR,
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{result}");
    assert_eq!(result["status"], "completed");
    let a = &result["analysis"];
    assert_eq!(a["status"], "no_measured_interaction");
    assert_eq!(
        a["effects"]["recipient_account_credit_raw"]["parameter_v1"]["value"],
        "-7609851"
    );
    assert_eq!(
        a["effects"]["recipient_account_credit_raw"]["interaction"]["value"],
        "0"
    );
    assert_eq!(a["r01"]["parent"]["stage"], "K1");
    assert_eq!(a["r11"]["parent"]["stage"], "K2");
    let (_, list) = h.get(&route(&project, &parent), OPERATOR).await;
    assert_eq!(list["interactions"][0]["run_id"], child);
    let artifact_route = format!("/v1/projects/{project}/interactions/{child}/artifact");
    let (code, bytes) = h.get_bytes(&artifact_route, OPERATOR).await;
    assert_eq!(code, StatusCode::OK);
    let isolated = tempfile::tempdir().unwrap();
    let tar = isolated.path().join("download.tar");
    std::fs::write(&tar, bytes).unwrap();
    assert!(std::process::Command::new("/usr/bin/tar")
        .args(["-xf"])
        .arg(&tar)
        .arg("-C")
        .arg(isolated.path())
        .env_clear()
        .status()
        .unwrap()
        .success());
    std::fs::remove_file(tar).unwrap();
    let portable = isolated.path().join("interaction");
    let report = engine::load(&portable).unwrap();
    assert_eq!(report.analysis_input_sha256, binding.analysis_input_sha256);
    assert_eq!(
        serde_json::to_value(interaction::presentation(&report)).unwrap(),
        *a
    );
    let binary = isolated.path().join("eplyx");
    std::fs::copy("../target/debug/eplyx", &binary).unwrap();
    assert_eq!(std::fs::read_dir(isolated.path()).unwrap().count(), 2);
    for op in ["verify", "reproduce"] {
        let output = std::process::Command::new(&binary)
            .current_dir(isolated.path())
            .env_clear()
            .args([
                "interaction",
                op,
                "--artifact",
                "interaction",
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let finished = h.state.registry.load_run(child).unwrap();
    let projection = h.state.registry.analytical_projection(&finished).unwrap();
    let mut corrupt = projection.clone();
    let mut report_value: Value = serde_json::from_str(&corrupt.report.text).unwrap();
    report_value["analysis"]["effects"]["recipient_account_credit_raw"]["interaction"]["value"] =
        json!("1");
    corrupt.report =
        eplyx_engine::cloud::contract::Artifact::new(serde_json::to_vec(&report_value).unwrap())
            .unwrap();
    assert!(h
        .state
        .registry
        .verify_hosted_projection(&finished, &corrupt)
        .is_err());
    for field in [
        "record_id",
        "analysis_input_sha256",
        "upgrade_change_spec_id",
        "parameter_change_spec_id",
        "bundle_sha256",
        "source_slot_range",
    ] {
        let mut altered = serde_json::to_value(&input).unwrap();
        altered["binding"][field] = json!("f".repeat(64));
        assert!(
            interaction::accepted(&h.state.registry, &serde_json::from_value(altered).unwrap())
                .is_err(),
            "{field}"
        );
    }
    // Corrupt actual retained parent/proposal sidecars, then restore the bytes.
    // Reads fail closed without replacing accepted refs or performing a VM run.
    for member in ["change_spec.json", "report.json"] {
        let path = h
            .state
            .registry
            .storage()
            .run_dir(&parent)
            .unwrap()
            .join(member);
        let original = std::fs::read(&path).unwrap();
        std::fs::write(&path, b"{}\n").unwrap();
        let (_, unavailable) = h
            .get(
                &format!("/v1/projects/{project}/interactions/{child}"),
                OPERATOR,
            )
            .await;
        assert!(unavailable["analysis"].is_null());
        assert_eq!(unavailable["failure"]["kind"], "evidence_integrity");
        std::fs::write(path, original).unwrap();
    }
    let mut portable_binding: Value =
        serde_json::from_str(&projection.bindings.unwrap().text).unwrap();
    let first = portable_binding["files"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    let hash = first["sha256"].as_str().unwrap();
    let object = h
        .state
        .registry
        .artifacts()
        .root()
        .join("captures")
        .join(hash);
    std::fs::write(object, b"corrupted CAS").unwrap();
    let (_, failed) = h
        .get(
            &format!("/v1/projects/{project}/interactions/{child}"),
            OPERATOR,
        )
        .await;
    assert!(failed["analysis"].is_null());
    assert_eq!(failed["failure"]["kind"], "evidence_integrity");
    assert_eq!(
        h.get_bytes(&artifact_route, OPERATOR).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(h
        .state
        .registry
        .load_run(child)
        .unwrap()
        .hosted_analysis
        .unwrap()
        .projection
        .is_some());
}
#[tokio::test]
async fn authorization_explicit_selection_unsupported_inputs_and_same_code_partial() {
    let h = Harness::new(1);
    let (project, parent) = parent(&h, "same", false).await;
    let path = route(&project, &parent);
    let p = parameter();
    let other = h.create_project("Other").await;
    let other_token = h.create_token(&other, "other").await;
    let token = h.create_token(&project, "interaction").await;
    for suffix in ["", "/eligibility"] {
        assert_eq!(
            h.get(&format!("{path}{suffix}"), &other_token).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            h.send(
                Request::builder()
                    .uri(format!("{path}{suffix}"))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            h.get(&format!("{path}{suffix}"), &token).await.0,
            StatusCode::OK
        );
    }
    assert_eq!(
        h.get(&format!("{}/eligibility", route(&other, &parent)), OPERATOR)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    for record in ["", "missing", "mainnet-spl-stake-pool-0c4fe6c80dd53827"] {
        assert_eq!(h.post_json(&path,OPERATOR,json!({"request_key":"refuse-selection-key","record_id":record,"parameter_change_spec":p})).await.0,StatusCode::BAD_REQUEST);
    }
    let wrong = json!({"schema_version":1,"change":{"kind":"protocol_parameter_change","target":{"program_id":eplyx_engine::standard_programs::token2022::PROGRAM_ID,"config_account":"CV6bkrUksMwcEC4jfLTJsbHwF3Y2YurZdWWua95Fpbtd"},"operation":{"kind":"token_2022_active_newer_transfer_fee_basis_points_v1","expected_current":{"account_data_sha256":"f".repeat(64),"basis_points":0,"schedule_epoch":"0","maximum_fee_raw":"0"},"proposed_basis_points":1}}});
    let wrong: ChangeSpec = serde_json::from_value(wrong).unwrap();
    assert_eq!(
        submit(&h, &project, &parent, &wrong, "wrong-operation-hosted-key")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    for field in [
        "candidate",
        "upgrade_change_spec",
        "clock",
        "accounts",
        "k1",
        "qualification",
        "effects",
    ] {
        let mut payload = json!({"request_key":"refuse-injection-key","record_id":RECORD,"parameter_change_spec":p});
        payload[field] = json!({});
        assert_eq!(
            h.post_json(&path, OPERATOR, payload).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let mut mismatch = serde_json::to_value(&p).unwrap();
    mismatch["change"]["operation"]["expected_current"]["numerator"] = json!("1");
    assert_eq!(h.post_json(&path,OPERATOR,json!({"request_key":"refuse-mismatch-key","record_id":RECORD,"parameter_change_spec":mismatch})).await.0,StatusCode::BAD_REQUEST);
    let (code,accepted)=h.post_json(&path,&token,json!({"request_key":"same-code-hosted-key","record_id":RECORD,"parameter_change_spec":p})).await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let child = accepted["run_id"].as_str().unwrap();
    assert_eq!(
        h.wait_until(child, RunStatus::is_terminal).await,
        RunStatus::Completed
    );
    let (_, wrong_parent) = h
        .get(&format!("{}/eligibility", route(&project, child)), OPERATOR)
        .await;
    assert_eq!(wrong_parent["eligible"], false);
    assert_eq!(wrong_parent["reason_code"], "wrong_parent_kind");
    let original_child = child.to_owned();
    let childroute = format!("/v1/projects/{project}/interactions/{child}");
    let (_, result) = h.get(&childroute, &token).await;
    assert_eq!(result["analysis"]["status"], "no_measured_interaction");
    assert_eq!(
        result["analysis"]["facts"]["v2"]["profile"]["id"],
        "same-code-historical-v1"
    );
    let original_identity = result["analysis"]["analysis_input_sha256"].clone();
    for suffix in ["", "/artifact"] {
        assert_eq!(
            h.get_bytes(&format!("{childroute}{suffix}"), &other_token)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            h.get_bytes(
                &format!("/v1/projects/{other}/interactions/{child}{suffix}"),
                OPERATOR
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    let mut partial = serde_json::to_value(&p).unwrap();
    partial["change"]["operation"]["proposed_fee"] = json!({"numerator":"2","denominator":"1"});
    let partial: ChangeSpec = serde_json::from_value(partial).unwrap();
    let (code, accepted) =
        submit(&h, &project, &parent, &partial, "partial-hosted-config-key").await;
    assert_eq!(code, StatusCode::ACCEPTED, "{accepted}");
    let child = accepted["run_id"].as_str().unwrap();
    assert_eq!(
        h.wait_until(child, RunStatus::is_terminal).await,
        RunStatus::Completed
    );
    let (_, result) = h
        .get(
            &format!("/v1/projects/{project}/interactions/{child}"),
            OPERATOR,
        )
        .await;
    assert_eq!(result["analysis"]["status"], "not_established");
    assert_eq!(result["analysis"]["k1"]["state"], "rejected");
    assert_eq!(result["analysis"]["r01"]["state"], "not_executed");
    assert!(
        result["analysis"]["effects"]["recipient_account_credit_raw"]["interaction"]["value"]
            .is_null()
    );
    assert_eq!(result["artifact_available"], true);
    let _permit = h.state.runs.acquire().await.unwrap();
    let (code, new) = submit(&h, &project, &parent, &p, "same-code-new-occurrence").await;
    assert_eq!(code, StatusCode::ACCEPTED);
    assert_ne!(new["run_id"], original_child);
    let next = h
        .state
        .registry
        .hosted_input(
            &h.state
                .registry
                .load_run(new["run_id"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
    let Input::UpgradeParameterInteraction { binding, .. } = next else {
        panic!("interaction")
    };
    assert_eq!(binding.analysis_input_sha256, original_identity);
}
#[tokio::test]
async fn unknown_candidate_and_exact_authority_expectation_fail_closed() {
    let h = Harness::new(1);
    for (which, authority, reason) in [
        ("fixture_stake_pool_v2.so", false, "candidate_unqualified"),
        (
            "fixture_stake_pool_config_v2.so",
            true,
            "unsupported_upgrade_expectations",
        ),
    ] {
        let (project, parent) = parent(&h, which, authority).await;
        let (_, e) = h
            .get(
                &format!("{}/eligibility", route(&project, &parent)),
                OPERATOR,
            )
            .await;
        assert_eq!(e["eligible"], false);
        assert!(e.to_string().contains(reason), "{e}");
        assert_eq!(
            submit(
                &h,
                &project,
                &parent,
                &parameter(),
                "unsupported-parent-key"
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn accepted_missing_evidence_is_a_typed_worker_failure_with_refs_retained() {
    let h = Harness::new(1);
    let (project, parent) = parent(&h, "fixture_stake_pool_config_v2.so", false).await;
    let permit = h.state.runs.acquire().await.unwrap();
    let (code, receipt) = submit(
        &h,
        &project,
        &parent,
        &parameter(),
        "missing-evidence-worker-key",
    )
    .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{receipt}");
    let child = receipt["run_id"].as_str().unwrap();
    let record = h.state.registry.load_run(child).unwrap();
    let accepted_input = h.state.registry.hosted_input(&record).unwrap();
    let Input::UpgradeParameterInteraction { capture, .. } = &accepted_input else {
        panic!("interaction")
    };
    std::fs::remove_file(
        h.state
            .registry
            .artifacts()
            .root()
            .join("captures")
            .join(&capture.sha256),
    )
    .unwrap();
    drop(permit);
    assert_eq!(
        h.wait_until(child, RunStatus::is_terminal).await,
        RunStatus::ExecutionError
    );
    let failed = h.state.registry.load_run(child).unwrap();
    assert_eq!(
        failed.hosted_analysis.as_ref().unwrap().interaction_failure,
        Some(interaction::FailureKind::EvidenceIntegrity)
    );
    assert_eq!(
        serde_json::to_value(h.state.registry.hosted_input(&failed).unwrap()).unwrap(),
        serde_json::to_value(&accepted_input).unwrap()
    );
    let (_, result) = h
        .get(
            &format!("/v1/projects/{project}/interactions/{child}"),
            OPERATOR,
        )
        .await;
    assert_eq!(result["failure"]["kind"], "evidence_integrity");
    assert!(result["analysis"].is_null());
    assert_eq!(result["artifact_available"], false);
}
