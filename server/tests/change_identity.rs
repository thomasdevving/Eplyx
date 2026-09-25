//! Phase P1: a hosted run is an analysis of one named proposal.
//!
//! Every property here is about identity rather than verdicts. Which verdict a
//! run reaches is the engine's business and is covered elsewhere; what this
//! file proves is that the proposal a run was accepted for is the proposal it
//! evaluated, the proposal its report names, and the proposal the registry can
//! find it by — and that nothing between acceptance and execution can quietly
//! substitute another.

mod common;

use axum::http::StatusCode;
use common::*;
use eplyx_engine::change::{Activation, Change, ChangeSpec};
use eplyx_server::project::AdapterId;
use eplyx_server::registry::{RunOutcome, RunStatus};
use serde_json::{json, Value};

fn program_id() -> String {
    committed_record().program_id
}

fn derived_spec() -> ChangeSpec {
    ChangeSpec::program_upgrade(&program_id(), &candidate_bytes())
}

async fn accepted(harness: &Harness, project_id: &str, token: &str) -> Value {
    let (status, body) = harness.submit_check(project_id, token).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body
}

fn run_id(body: &Value) -> String {
    body["run_id"].as_str().expect("run id").to_string()
}

async fn report_of(harness: &Harness, run_id: &str, token: &str) -> Value {
    let (status, bytes) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), token)
        .await;
    assert_eq!(status, StatusCode::OK);
    serde_json::from_slice(&bytes).expect("report")
}

fn project_runs(harness: &Harness, project_id: &str) -> Vec<String> {
    harness
        .state
        .registry
        .project_run_ids(project_id)
        .expect("run ids")
}

// ------------------------------------------------------- the simple path

/// 1. A client written before change identity keeps working, unchanged.
#[tokio::test]
async fn a_candidate_only_check_still_succeeds() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let body = accepted(&harness, &project_id, &token).await;

    // Everything an older client read is still there.
    assert_eq!(body["status"], "queued");
    assert!(body["status_url"].as_str().is_some());
    let sha = eplyx_engine::replay::hash_bytes(&candidate_bytes());
    assert_eq!(body["candidate_sha256"], sha.as_str());

    // And the proposal is now named before anything runs.
    let change = &body["change"];
    assert_eq!(change["kind"], "program_upgrade");
    assert_eq!(change["target_program_id"], program_id().as_str());
    assert_eq!(change["candidate_sha256"], sha.as_str());
    assert_eq!(change["candidate_len"], candidate_bytes().len() as u64);
    assert_eq!(change["origin"], "derived_from_candidate");
    assert_eq!(change["label"], Value::Null);

    let id = run_id(&body);
    let status = harness.wait_until(&id, RunStatus::is_terminal).await;
    assert!(matches!(status, RunStatus::Passed | RunStatus::Failed));
    let (_, run) = harness.get(&format!("/v1/runs/{id}"), &token).await;
    assert_eq!(run["candidate_sha256"], sha.as_str());
    assert_eq!(run["change"], *change);
    assert_eq!(run["report_available"], true);
}

/// 2. The server derives exactly the spec `eplyx ci check --candidate` does.
#[tokio::test]
async fn a_candidate_only_check_derives_the_cli_s_spec() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let first = accepted(&harness, &project_id, &token).await;
    let second = accepted(&harness, &project_id, &token).await;

    let expected = derived_spec().id().expect("id");
    assert_eq!(first["change"]["change_spec_id"], expected.as_str());
    // Deterministic: a second submission of the same bytes is the same change.
    assert_eq!(second["change"]["change_spec_id"], expected.as_str());

    // And it is the id the CLI puts in a local report.
    let scratch = tempfile::tempdir().expect("scratch");
    let bundle = build_bundle(&scratch.path().join("built"), &committed_record());
    let candidate = scratch.path().join("candidate.so");
    std::fs::write(&candidate, candidate_bytes()).expect("candidate");
    let local = eplyx_engine::ci::check(&bundle, &candidate, None).expect("local");
    assert_eq!(local.change.expect("change").change_spec_id, expected);

    // A label is display only: it survives, and it does not re-identify.
    let (status, labelled) = harness
        .submit_parts(
            &project_id,
            &token,
            &[
                ("candidate", &candidate_bytes()),
                ("label", b"v2.1 release candidate"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{labelled}");
    assert_eq!(labelled["change"]["change_spec_id"], expected.as_str());
    assert_eq!(labelled["change"]["label"], "v2.1 release candidate");
}

// ----------------------------------------------------- the explicit path

/// 3. A client that already holds a spec gets an analysis of exactly it.
#[tokio::test]
async fn an_explicit_spec_is_analysed_exactly() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let mut spec = derived_spec();
    spec.activation = Some(Activation {
        slot: Some(400_000_000),
        unix_timestamp: None,
    });
    spec.metadata.label = Some("release 2.1".into());
    spec.metadata.source = Some("release tooling".into());
    let document = spec.to_document().expect("document");
    let spec_id = spec.id().expect("id");
    // Activation is identifying, so this is not the derived proposal.
    assert_ne!(spec_id, derived_spec().id().expect("id"));

    let (status, body) = harness
        .submit_parts(
            &project_id,
            &token,
            &[
                ("change_spec", document.as_bytes()),
                ("candidate", &candidate_bytes()),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["change"]["change_spec_id"], spec_id.as_str());
    assert_eq!(body["change"]["origin"], "submitted");
    assert_eq!(body["change"]["label"], "release 2.1");

    let id = run_id(&body);
    harness.wait_until(&id, RunStatus::is_terminal).await;
    let report = report_of(&harness, &id, &token).await;
    assert_eq!(report["change"]["change_spec_id"], spec_id.as_str());

    // The canonical document is served on request, and is the one submitted.
    let (status, stored) = harness
        .get(&format!("/v1/runs/{id}/change_spec.json"), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["change_spec_id"], spec_id.as_str());
    assert_eq!(stored["activation"]["slot"], 400_000_000);
    assert_eq!(stored["metadata"]["source"], "release tooling");
    assert_eq!(
        ChangeSpec::parse(stored.to_string().as_bytes()).expect("parses"),
        ChangeSpec::parse(document.as_bytes()).expect("parses")
    );
}

/// 4. Spec A with artefact B never becomes a run, let alone an execution.
#[tokio::test]
async fn a_spec_for_one_candidate_never_executes_another() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let document = derived_spec().to_document().expect("document");
    let other = std::fs::read(BASELINE).expect("baseline bytes");

    let (status, body) = harness
        .submit_parts(
            &project_id,
            &token,
            &[("change_spec", document.as_bytes()), ("candidate", &other)],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["exit_code"], 2, "{body}");
    assert!(body["error"]
        .as_str()
        .is_some_and(|e| e.contains("describes")));
    assert!(project_runs(&harness, &project_id).is_empty());

    // A spec with no bytes at all has nothing to execute either.
    let (status, body) = harness
        .submit_parts(&project_id, &token, &[("change_spec", document.as_bytes())])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["exit_code"], 2);

    // The spec is authoritative, so a label beside it is ambiguous, not merged.
    let (status, body) = harness
        .submit_parts(
            &project_id,
            &token,
            &[
                ("change_spec", document.as_bytes()),
                ("candidate", &candidate_bytes()),
                ("label", b"elsewhere"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // Two candidates in one request leave "which one" to arrival order.
    let (status, _) = harness
        .submit_parts(
            &project_id,
            &token,
            &[("candidate", &candidate_bytes()), ("candidate", &other)],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A spec whose stated id does not match its fields is malformed.
    let mut forged: Value = serde_json::from_str(&document).expect("json");
    forged["change_spec_id"] = json!("0".repeat(64));
    let (status, body) = harness
        .submit_parts(
            &project_id,
            &token,
            &[
                ("change_spec", forged.to_string().as_bytes()),
                ("candidate", &candidate_bytes()),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["exit_code"], 2);
    assert!(project_runs(&harness, &project_id).is_empty());
}

/// 5. A spec that does not describe a change to this project's baseline is
///    refused with the CLI's exit code, before a run exists.
#[tokio::test]
async fn a_spec_for_another_target_is_refused() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;

    let foreign = ChangeSpec::program_upgrade(STAKE_POOL_PROGRAM, &candidate_bytes());
    let (status, body) = harness
        .submit_parts(
            &project_id,
            &token,
            &[
                (
                    "change_spec",
                    foreign.to_document().expect("doc").as_bytes(),
                ),
                ("candidate", &candidate_bytes()),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["exit_code"], 4, "{body}");
    assert!(body["error"]
        .as_str()
        .is_some_and(|e| e.contains("targets")));

    // An expectation the bundle cannot prove never satisfies itself: a
    // schema-1 bundle carries no ProgramData evidence.
    let mut stated = derived_spec();
    let Change::ProgramUpgrade { target, .. } = &mut stated.change;
    target.programdata_address = Some(STAKE_POOL_PROGRAM.into());
    let (status, body) = harness
        .submit_parts(
            &project_id,
            &token,
            &[
                ("change_spec", stated.to_document().expect("doc").as_bytes()),
                ("candidate", &candidate_bytes()),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["exit_code"], 4);
    assert!(project_runs(&harness, &project_id).is_empty());
}

// ------------------------------------------------------------- identity

/// 6. registry id == stored spec id == report id, and a report about anything
///    else is never recorded as this run's verdict.
#[tokio::test]
async fn registry_spec_and_report_name_one_change() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let body = accepted(&harness, &project_id, &token).await;
    let id = run_id(&body);
    harness.wait_until(&id, RunStatus::is_terminal).await;

    let metadata = harness.state.registry.load_run(&id).expect("run");
    let change = metadata.change.clone().expect("change");
    let stored = harness
        .state
        .registry
        .load_change_spec(&id, &change)
        .expect("stored spec");
    let report = report_of(&harness, &id, &token).await;
    assert_eq!(stored.id().expect("id"), change.change_spec_id);
    assert_eq!(
        report["change"]["change_spec_id"],
        change.change_spec_id.as_str()
    );

    // Found again by the proposal's identity, which is what a governance
    // binding would hold.
    let (_, listed) = harness
        .get(
            &format!(
                "/v1/projects/{project_id}/runs?change_spec_id={}",
                change.change_spec_id
            ),
            &token,
        )
        .await;
    let ids: Vec<&str> = listed["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .map(|run| run["run_id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, vec![id.as_str()]);
    let (_, none) = harness
        .get(
            &format!(
                "/v1/projects/{project_id}/runs?change_spec_id={}",
                "a".repeat(64)
            ),
            &token,
        )
        .await;
    assert!(none["runs"].as_array().expect("runs").is_empty());
    let (status, _) = harness
        .get(
            &format!("/v1/projects/{project_id}/runs?change_spec_id=../x"),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Now hand the registry a genuine report about a different proposal.
    let (_, queued) = harness
        .submit_parts(
            &project_id,
            &token,
            &[("candidate", &candidate_bytes()), ("label", b"second")],
        )
        .await;
    let second = run_id(&queued);
    harness.wait_until(&second, RunStatus::is_terminal).await;
    let scratch = tempfile::tempdir().expect("scratch");
    let bundle = build_bundle(&scratch.path().join("built"), &committed_record());
    let mut spec = derived_spec();
    spec.activation = Some(Activation {
        slot: Some(1),
        unix_timestamp: None,
    });
    let foreign = eplyx_engine::ci::check_change(
        &bundle,
        &eplyx_engine::ci::ChangeInput::Spec {
            spec: &spec,
            source: Some(eplyx_engine::change::CandidateSource::Bytes(
                &candidate_bytes(),
            )),
        },
        None,
    )
    .expect("a real report about another change");

    let registry = &harness.state.registry;
    let mut run = registry.load_run(&second).expect("run");
    run.status = RunStatus::Running;
    run.report_available = false;
    std::fs::write(
        harness
            .state
            .registry
            .storage()
            .run_dir(&second)
            .expect("dir")
            .join("metadata.json"),
        serde_json::to_vec_pretty(&run).expect("json"),
    )
    .expect("reset");
    registry
        .finish_run(
            &second,
            RunOutcome::Reported {
                report: Box::new(foreign),
                markdown: String::new(),
            },
        )
        .expect("finish");
    let recorded = registry.load_run(&second).expect("run");
    assert_eq!(recorded.status, RunStatus::ExecutionError);
    assert_eq!(recorded.exit_code, None);
    assert!(recorded
        .detail
        .as_deref()
        .is_some_and(|d| d.contains("change identity")));
}

/// 7. The same bytes proposed for two programs are two proposals.
#[tokio::test]
async fn the_same_bytes_against_two_targets_are_two_changes() {
    let harness = Harness::new(0);
    let (lending, lending_token) = ready_project(&harness, "Lending").await;

    let (status, created) = harness
        .post_json(
            "/v1/projects",
            OPERATOR,
            json!({
                "name": "Pool",
                "program_id": STAKE_POOL_PROGRAM,
                "adapter_id": AdapterId::for_program(STAKE_POOL_PROGRAM).to_string(),
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let pool = created["project_id"].as_str().expect("id").to_string();
    let pool_token = harness.create_token(&pool, "CI").await;
    let bundle =
        build_unvalidated_bundle(&harness.scratch.path().join("pool"), &stake_pool_record());
    let bundle_id = harness.upload_bundle_from(&pool, &bundle).await;
    let (status, body) = harness
        .post_json(
            &format!("/v1/projects/{pool}/bundles/{bundle_id}/activate"),
            OPERATOR,
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let a = accepted(&harness, &lending, &lending_token).await;
    let b = accepted(&harness, &pool, &pool_token).await;
    assert_eq!(
        a["change"]["candidate_sha256"],
        b["change"]["candidate_sha256"]
    );
    assert_eq!(b["change"]["target_program_id"], STAKE_POOL_PROGRAM);
    assert_ne!(
        a["change"]["change_spec_id"], b["change"]["change_spec_id"],
        "one artefact for two targets collapsed into one proposal"
    );
}

/// 8. A bundle activated while a run waits changes neither what the run
///    measures against nor which proposal it names.
#[tokio::test]
async fn the_worker_cannot_use_a_bundle_activated_later() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;
    let token = harness.create_token(&project_id, "CI").await;
    harness.activate_bundle(&project_id).await;
    let body = accepted(&harness, &project_id, &token).await;
    let id = run_id(&body);
    let pinned = body["bundle_sha256"].as_str().expect("sha").to_string();

    let mut other = committed_record();
    other.id = format!("{}-b", other.id);
    let rotated = harness
        .upload_bundle_from(
            &project_id,
            &build_bundle(&harness.scratch.path().join("rotated"), &other),
        )
        .await;
    let (status, _) = harness
        .post_json(
            &format!("/v1/projects/{project_id}/bundles/{rotated}/activate"),
            OPERATOR,
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    harness.state.runs.add_permits(1);
    harness.wait_until(&id, RunStatus::is_terminal).await;
    let report = report_of(&harness, &id, &token).await;
    assert_eq!(report["bundle"]["sha256"], pinned.as_str());
    assert_eq!(report["change"], {
        let change = &body["change"];
        json!({
            "change_spec_id": change["change_spec_id"],
            "kind": change["kind"],
            "target_program_id": change["target_program_id"],
            "candidate_sha256": change["candidate_sha256"],
        })
    });
}

// ----------------------------------------------------------- fail closed

async fn queued_run(harness: &Harness) -> (String, String) {
    let (project_id, token) = ready_project(harness, "Example").await;
    let body = accepted(harness, &project_id, &token).await;
    (run_id(&body), token)
}

async fn detail_after_running(harness: &Harness, id: &str, token: &str) -> Value {
    harness.state.runs.add_permits(1);
    harness.wait_until(id, RunStatus::is_terminal).await;
    let (_, run) = harness.get(&format!("/v1/runs/{id}"), token).await;
    run
}

/// 9. A candidate that is gone is not silently replaced by anything.
#[tokio::test]
async fn a_missing_candidate_fails_closed() {
    let harness = Harness::new(0);
    let (id, token) = queued_run(&harness).await;
    std::fs::remove_dir_all(harness.work_dir(&id).join("artifacts")).expect("remove");
    let run = detail_after_running(&harness, &id, &token).await;
    assert_eq!(run["status"], "execution_error", "{run}");
    assert_eq!(run["report_available"], false);
    assert!(run["detail"]
        .as_str()
        .is_some_and(|d| d.contains("no longer available")));
}

#[tokio::test]
async fn an_altered_candidate_fails_closed() {
    let harness = Harness::new(0);
    let (id, token) = queued_run(&harness).await;
    let sha = eplyx_engine::replay::hash_bytes(&candidate_bytes());
    let staged = harness.work_dir(&id).join("artifacts/programs").join(&sha);
    std::fs::write(&staged, std::fs::read(BASELINE).expect("other bytes")).expect("swap");
    let run = detail_after_running(&harness, &id, &token).await;
    assert_eq!(run["status"], "execution_error", "{run}");
    assert_eq!(run["report_available"], false);
}

/// 10. A stored spec that no longer verifies is never evaluated or served.
#[tokio::test]
async fn a_tampered_stored_spec_fails_closed() {
    let harness = Harness::new(0);
    let (id, token) = queued_run(&harness).await;
    let path = harness
        .state
        .registry
        .storage()
        .run_dir(&id)
        .expect("dir")
        .join("change_spec.json");
    let mut document: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("spec")).expect("json");
    document["change"]["candidate"]["len"] = json!(1);
    std::fs::write(&path, document.to_string()).expect("tamper");

    let (status, _) = harness
        .get(&format!("/v1/runs/{id}/change_spec.json"), &token)
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let run = detail_after_running(&harness, &id, &token).await;
    assert_eq!(run["status"], "execution_error", "{run}");
    assert!(run["detail"]
        .as_str()
        .is_some_and(|d| d.contains("change spec")));
}

/// A rewrite that is internally consistent — fields and id both changed — is
/// still not the proposal the registry indexed the run under.
#[tokio::test]
async fn a_consistently_rewritten_spec_is_still_not_the_accepted_one() {
    let harness = Harness::new(0);
    let (id, token) = queued_run(&harness).await;
    let path = harness
        .state
        .registry
        .storage()
        .run_dir(&id)
        .expect("dir")
        .join("change_spec.json");
    let mut spec = derived_spec();
    spec.activation = Some(Activation {
        slot: Some(7),
        unix_timestamp: None,
    });
    std::fs::write(&path, spec.to_document().expect("doc")).expect("rewrite");
    // Refused at the read itself, not merely caught later by the report check.
    let (status, _) = harness
        .get(&format!("/v1/runs/{id}/change_spec.json"), &token)
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let run = detail_after_running(&harness, &id, &token).await;
    assert_eq!(run["status"], "execution_error", "{run}");
    assert!(run["detail"]
        .as_str()
        .is_some_and(|d| d.contains("stored change spec") && d.contains("accepted for")));
}

/// The pinned bundle is re-bound at execution, so a run whose indexed target
/// (spec and registry rewritten together) no longer matches its bundle is
/// refused by the engine with exit 4, and nothing executes.
#[tokio::test]
async fn a_target_that_no_longer_matches_the_pinned_bundle_fails_closed() {
    let harness = Harness::new(0);
    let (id, token) = queued_run(&harness).await;
    let registry = &harness.state.registry;
    let spec = ChangeSpec::program_upgrade(STAKE_POOL_PROGRAM, &candidate_bytes());
    let dir = registry.storage().run_dir(&id).expect("dir");
    std::fs::write(
        dir.join("change_spec.json"),
        spec.to_document().expect("doc"),
    )
    .expect("spec");
    let mut run = registry.load_run(&id).expect("run");
    let change = run.change.as_mut().expect("change");
    change.change_spec_id = spec.id().expect("id");
    change.target_program_id = STAKE_POOL_PROGRAM.into();
    std::fs::write(
        dir.join("metadata.json"),
        serde_json::to_vec_pretty(&run).expect("json"),
    )
    .expect("metadata");

    let run = detail_after_running(&harness, &id, &token).await;
    assert_eq!(run["status"], "failed", "{run}");
    assert_eq!(run["exit_code"], 4);
    assert_eq!(run["report_available"], false);
    assert!(run["detail"]
        .as_str()
        .is_some_and(|d| d.contains("targets")));
}

// ---------------------------------------------------------------- legacy

/// 11. A run recorded before change identity stays readable, and is marked as
///     what it is rather than given an identity after the fact.
#[tokio::test]
async fn a_legacy_run_remains_readable() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let body = accepted(&harness, &project_id, &token).await;
    let id = run_id(&body);
    harness.wait_until(&id, RunStatus::is_terminal).await;

    // Rewrite it into the shape a pre-P1 service wrote: no `change`, no spec.
    let dir = harness.state.registry.storage().run_dir(&id).expect("dir");
    let mut metadata: Value =
        serde_json::from_slice(&std::fs::read(dir.join("metadata.json")).expect("read"))
            .expect("json");
    metadata.as_object_mut().expect("object").remove("change");
    std::fs::write(dir.join("metadata.json"), metadata.to_string()).expect("write");
    std::fs::remove_file(dir.join("change_spec.json")).expect("remove");

    let reopened = harness.reopen();
    let (status, run) = harness
        .get_on(reopened.clone(), &format!("/v1/runs/{id}"), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    assert_eq!(run["change"], Value::Null);
    assert!(run["candidate_sha256"].as_str().is_some());
    let (status, listed) = harness
        .get_on(
            reopened.clone(),
            &format!("/v1/projects/{project_id}/runs"),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["runs"][0]["change"], Value::Null);
    let (status, body) = harness
        .get_on(reopened, &format!("/v1/runs/{id}/change_spec.json"), &token)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body["error"]
        .as_str()
        .is_some_and(|e| e.contains("before change identity")));
    // Its report is served exactly as it was stored.
    report_of(&harness, &id, &token).await;
}
