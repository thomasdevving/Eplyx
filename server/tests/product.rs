//! Onboarding: projects, their tokens, their bundles and their history.
//!
//! Everything here goes through the real router, because the questions are
//! about what a caller is allowed to do and what they are told, and both live
//! in the HTTP layer rather than under it.

mod common;

use axum::body::Body;
use axum::http::StatusCode;
use common::*;
use eplyx_server::project::ProjectStatus;
use eplyx_server::registry::RunStatus;
use serde_json::json;

// ---------------------------------------------------------------- projects

#[tokio::test]
async fn a_project_is_created_in_setup_with_an_opaque_id() {
    let harness = Harness::new(0);
    let program = committed_record().program_id;
    let (status, body) = harness
        .post_json(
            "/v1/projects",
            OPERATOR,
            json!({ "name": "  Example Lending  ", "program_id": program, "adapter_id": "none@0" }),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["project_id"].as_str().expect("project id");
    assert!(id.starts_with("proj_"), "{id}");
    // The name is not the identifier: two teams may both call theirs Lending.
    assert!(!id.contains("Example"), "{id}");
    assert_eq!(body["name"], "Example Lending", "the name was not trimmed");
    assert_eq!(body["chain"], "solana");
    assert_eq!(body["status"], "setup");
    assert_eq!(body["active_bundle"], serde_json::Value::Null);
    // A program this build reads no semantics for says so up front.
    assert_eq!(body["speaks_semantics"], false);
}

#[tokio::test]
async fn a_project_refuses_what_it_cannot_honour() {
    let harness = Harness::new(0);
    let program = committed_record().program_id;
    let cases = [
        (
            "empty name",
            json!({ "name": "", "program_id": program, "adapter_id": "none@0" }),
        ),
        (
            "not a pubkey",
            json!({ "name": "X", "program_id": "not-base58-0OIl", "adapter_id": "none@0" }),
        ),
        (
            "short pubkey",
            json!({ "name": "X", "program_id": "SPoo1", "adapter_id": "none@0" }),
        ),
        (
            "unparseable adapter",
            json!({ "name": "X", "program_id": program, "adapter_id": "none" }),
        ),
        (
            "unknown adapter",
            json!({ "name": "X", "program_id": program, "adapter_id": "invented@9" }),
        ),
        // The engine picks the adapter from the program. A project may declare
        // it so the declaration can be checked, never so it can choose.
        (
            "wrong adapter for the program",
            json!({ "name": "X", "program_id": program, "adapter_id": "spl-stake-pool@3" }),
        ),
        (
            "unknown field",
            json!({ "name": "X", "program_id": program, "adapter_id": "none@0", "chain": "ethereum" }),
        ),
    ];
    for (label, request) in cases {
        let (status, body) = harness.post_json("/v1/projects", OPERATOR, request).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{label} was accepted: {body}"
        );
    }
}

#[tokio::test]
async fn projects_are_listed_newest_first_and_survive_a_restart() {
    let harness = Harness::new(0);
    let first = harness.create_project("First").await;
    let second = harness.create_project("Second").await;

    let (status, body) = harness.get("/v1/projects", OPERATOR).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .map(|p| p["project_id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, vec![second.as_str(), first.as_str()], "{body}");

    // A different service over the same volume.
    let restarted = harness.reopen();
    let (status, body) = harness.get_on(restarted, "/v1/projects", OPERATOR).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["projects"].as_array().expect("projects").len(), 2);
}

#[tokio::test]
async fn a_project_reports_cheap_run_statistics() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, accepted) = harness.submit_check(&project_id, &token).await;
    let run_id = accepted["run_id"].as_str().expect("run id");

    let (status, body) = harness
        .get(&format!("/v1/projects/{project_id}"), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["project"]["status"], "ready");
    assert_eq!(body["run_count"], 1);
    assert_eq!(body["last_run_id"], run_id);
    assert_eq!(body["last_run_status"], "queued");
}

#[tokio::test]
async fn listing_projects_needs_the_operator() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;

    for credential in ["", "wrong", &token] {
        let (status, _) = harness.get("/v1/projects", credential).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a project token listed every project"
        );
    }
    // Its own project, though, a project token may read.
    let (status, _) = harness
        .get(&format!("/v1/projects/{project_id}"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
}

// ------------------------------------------------------------------ tokens

#[tokio::test]
async fn a_token_is_shown_once_and_never_again() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;
    let (status, created) = harness
        .post_json(
            &format!("/v1/projects/{project_id}/tokens"),
            OPERATOR,
            json!({ "label": "Local development" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let secret = created["token"].as_str().expect("token").to_string();
    assert!(secret.starts_with("eplyx_proj_"), "{secret}");
    let token_id = created["token_id"].as_str().expect("token id").to_string();
    assert_ne!(token_id, secret, "the id and the secret are the same value");

    let (status, listed) = harness
        .get(&format!("/v1/projects/{project_id}/tokens"), OPERATOR)
        .await;
    assert_eq!(status, StatusCode::OK);
    let text = listed.to_string();
    assert!(!text.contains(&secret), "the secret came back in a listing");
    assert!(text.contains(&token_id));
    assert_eq!(listed["tokens"][0]["label"], "Local development");

    // Nor is it anywhere on the volume.
    let stored = std::fs::read_to_string(
        harness
            .scratch
            .path()
            .join("data/projects")
            .join(&project_id)
            .join("tokens")
            .join(format!("{token_id}.json")),
    )
    .expect("token record");
    assert!(!stored.contains(&secret), "the raw token was persisted");
}

#[tokio::test]
async fn a_revoked_token_stops_working_immediately() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (status, _) = harness.submit_check(&project_id, &token).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the token did not work to begin with"
    );

    let (_, listed) = harness
        .get(&format!("/v1/projects/{project_id}/tokens"), OPERATOR)
        .await;
    let token_id = listed["tokens"][0]["token_id"].as_str().expect("token id");
    let (status, revoked) = harness
        .delete(
            &format!("/v1/projects/{project_id}/tokens/{token_id}"),
            OPERATOR,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{revoked}");
    assert!(revoked["revoked_at_unix_seconds"].as_u64().is_some());

    let (status, _) = harness.submit_check(&project_id, &token).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a revoked token still worked"
    );
    let (status, _) = harness
        .get(&format!("/v1/projects/{project_id}"), &token)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_project_token_cannot_manage_credentials_or_baselines() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (other_id, other_token) = ready_project(&harness, "Other").await;

    // Its own project: a CI credential may not issue more of itself, nor move
    // what future checks are measured against.
    let (status, _) = harness
        .post_json(
            &format!("/v1/projects/{project_id}/tokens"),
            &token,
            json!({ "label": "self-issued" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a CI token issued a token");
    let (status, _) = harness
        .get(&format!("/v1/projects/{project_id}/tokens"), &token)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Another project: not even the existence of it.
    for path in [
        format!("/v1/projects/{other_id}"),
        format!("/v1/projects/{other_id}/tokens"),
        format!("/v1/projects/{other_id}/bundles"),
        format!("/v1/projects/{other_id}/runs"),
    ] {
        let (status, _) = harness.get(&path, &token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
    }
    let (status, _) = harness.submit_check(&other_id, &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "checked another project");
    let _ = other_token;
}

// ----------------------------------------------------------------- bundles

#[tokio::test]
async fn a_bundle_registers_activates_and_makes_the_project_ready() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;

    let built = build_bundle(&harness.scratch.path().join("b1"), &committed_record());
    let (status, bundle) = harness.upload_bundle_result(&project_id, &built).await;
    assert_eq!(status, StatusCode::CREATED, "{bundle}");
    let bundle_id = bundle["bundle_id"].as_str().expect("bundle id").to_string();
    assert!(bundle_id.starts_with("bndl_"), "{bundle_id}");
    assert_eq!(bundle["active"], false, "registering activated it");
    assert!(bundle["bundle_sha256"]
        .as_str()
        .is_some_and(|s| s.len() == 64));
    assert!(bundle["baseline_sha256"]
        .as_str()
        .is_some_and(|s| s.len() == 64));
    assert_eq!(bundle["record_count"], 1);

    // Still setup: registering a bundle is not activating one.
    let (_, body) = harness
        .get(&format!("/v1/projects/{project_id}"), OPERATOR)
        .await;
    assert_eq!(body["project"]["status"], "setup");

    let (status, project) = harness
        .post_json(
            &format!("/v1/projects/{project_id}/bundles/{bundle_id}/activate"),
            OPERATOR,
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{project}");
    assert_eq!(project["status"], "ready");
    assert_eq!(project["active_bundle"]["bundle_id"], bundle_id.as_str());
}

#[tokio::test]
async fn a_bundle_that_does_not_belong_is_refused() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;

    // Another program entirely.
    let foreign = build_unvalidated_bundle(
        &harness.scratch.path().join("foreign"),
        &stake_pool_record(),
    );
    let (status, body) = harness.upload_bundle_result(&project_id, &foreign).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"]
        .as_str()
        .is_some_and(|e| e.contains("program")));

    // Tampered after it was built: the hash no longer describes the bytes.
    let built = build_bundle(
        &harness.scratch.path().join("tampered"),
        &committed_record(),
    );
    let manifest = built.join("bundle.json");
    let text = std::fs::read_to_string(&manifest).expect("manifest");
    std::fs::write(
        &manifest,
        text.replace("\"record_count\": 1", "\"record_count\": 2"),
    )
    .expect("tamper");
    let (status, body) = harness.upload_bundle_result(&project_id, &built).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a tampered bundle registered: {body}"
    );

    // And nothing was activated along the way.
    let (_, body) = harness
        .get(&format!("/v1/projects/{project_id}"), OPERATOR)
        .await;
    assert_eq!(body["project"]["status"], "setup");
}

#[tokio::test]
async fn a_bundle_built_under_another_adapter_version_is_refused() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;
    // As if the engine had moved on: the project declares a vocabulary this
    // bundle was not built under.
    let mut project = harness
        .state
        .registry
        .load_project(&project_id)
        .expect("project");
    project.adapter_id.version += 1;
    harness.state.registry.save_project(&project).expect("save");

    let built = build_bundle(&harness.scratch.path().join("skewed"), &committed_record());
    let (status, body) = harness.upload_bundle_result(&project_id, &built).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"]
        .as_str()
        .is_some_and(|e| e.contains("adapter")));
}

#[tokio::test]
async fn the_same_bundle_twice_is_one_bundle() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;
    let built = build_bundle(&harness.scratch.path().join("same"), &committed_record());

    let first = harness.upload_bundle_from(&project_id, &built).await;
    let second = harness.upload_bundle_from(&project_id, &built).await;
    assert_eq!(first, second, "identical content minted a second identity");

    let (_, body) = harness
        .get(&format!("/v1/projects/{project_id}/bundles"), OPERATOR)
        .await;
    assert_eq!(body["bundles"].as_array().expect("bundles").len(), 1);
}

#[tokio::test]
async fn activating_a_new_bundle_keeps_the_old_one() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;
    let first = harness
        .upload_bundle_from(
            &project_id,
            &build_bundle(&harness.scratch.path().join("a"), &committed_record()),
        )
        .await;
    let mut other = committed_record();
    other.id = format!("{}-b", other.id);
    let second = harness
        .upload_bundle_from(
            &project_id,
            &build_bundle(&harness.scratch.path().join("b"), &other),
        )
        .await;
    assert_ne!(first, second);

    for id in [&first, &second] {
        let (status, _) = harness
            .post_json(
                &format!("/v1/projects/{project_id}/bundles/{id}/activate"),
                OPERATOR,
                json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    let (_, body) = harness
        .get(&format!("/v1/projects/{project_id}/bundles"), OPERATOR)
        .await;
    let bundles = body["bundles"].as_array().expect("bundles");
    assert_eq!(bundles.len(), 2, "activation destroyed history: {body}");
    let active: Vec<&str> = bundles
        .iter()
        .filter(|b| b["active"] == true)
        .map(|b| b["bundle_id"].as_str().expect("id"))
        .collect();
    assert_eq!(
        active,
        vec![second.as_str()],
        "two bundles claimed to be active"
    );
}

// ------------------------------------------------------------------ checks

#[tokio::test]
async fn only_a_ready_project_accepts_checks() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;
    let token = harness.create_token(&project_id, "CI").await;

    // Setup: nothing to measure against.
    let (status, body) = harness.submit_check(&project_id, &token).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body.get("exit_code").is_none(),
        "a setup project produced a gate code"
    );

    harness.activate_bundle(&project_id).await;
    let (status, _) = harness.submit_check(&project_id, &token).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    harness.set_status(&project_id, ProjectStatus::Disabled);
    let (status, body) = harness.submit_check(&project_id, &token).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"]
        .as_str()
        .is_some_and(|e| e.contains("disabled")));
    assert!(body.get("exit_code").is_none());
}

/// The invariant this whole model exists to protect.
#[tokio::test]
async fn a_run_keeps_the_bundle_it_was_accepted_against() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;
    let token = harness.create_token(&project_id, "CI").await;
    let bundle_a = harness.activate_bundle(&project_id).await;

    let (_, accepted) = harness.submit_check(&project_id, &token).await;
    let run_id = accepted["run_id"].as_str().expect("run id").to_string();
    let (_, before) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    assert_eq!(before["bundle_id"], bundle_a.as_str());
    let pinned_sha = before["bundle_sha256"].as_str().expect("sha").to_string();

    // Rotate the baseline while the run is still queued.
    let mut other = committed_record();
    other.id = format!("{}-b", other.id);
    let bundle_b = harness
        .upload_bundle_from(
            &project_id,
            &build_bundle(&harness.scratch.path().join("rotated"), &other),
        )
        .await;
    let (status, _) = harness
        .post_json(
            &format!("/v1/projects/{project_id}/bundles/{bundle_b}/activate"),
            OPERATOR,
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(bundle_a, bundle_b);

    // Let it run, now that the project points somewhere else.
    harness.state.runs.add_permits(1);
    harness.wait_until(&run_id, RunStatus::is_terminal).await;
    let (_, after) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    assert_eq!(
        after["bundle_id"],
        bundle_a.as_str(),
        "an accepted run was re-pointed at a bundle activated after it"
    );
    assert_eq!(after["bundle_sha256"], pinned_sha.as_str());
    let (_, report) = harness
        .get_bytes(&format!("/v1/runs/{run_id}/report.json"), &token)
        .await;
    let report: serde_json::Value = serde_json::from_slice(&report).expect("report");
    assert_eq!(
        report["bundle"]["sha256"],
        pinned_sha.as_str(),
        "the report describes a comparison nobody asked for"
    );

    // The next run is measured against the new one.
    let (_, next) = harness.submit_check(&project_id, &token).await;
    let next_id = next["run_id"].as_str().expect("run id");
    let (_, next_run) = harness.get(&format!("/v1/runs/{next_id}"), &token).await;
    assert_eq!(next_run["bundle_id"], bundle_b.as_str());
}

// ------------------------------------------------------------- run history

#[tokio::test]
async fn history_lists_only_a_project_s_own_runs_newest_first() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (other_id, other_token) = ready_project(&harness, "Other").await;

    let mut mine = Vec::new();
    for _ in 0..3 {
        let (_, body) = harness.submit_check(&project_id, &token).await;
        mine.push(body["run_id"].as_str().expect("run id").to_string());
    }
    harness.submit_check(&other_id, &other_token).await;

    let (status, body) = harness
        .get(&format!("/v1/projects/{project_id}/runs"), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed: Vec<&str> = body["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .map(|r| r["run_id"].as_str().expect("id"))
        .collect();
    mine.reverse();
    assert_eq!(listed, mine, "wrong runs, or wrong order");

    // No report body rides along in a listing.
    let text = body.to_string();
    assert!(
        !text.contains("\"findings\""),
        "a report leaked into the list"
    );
    assert!(
        !text.contains("\"coverage\""),
        "a report leaked into the list"
    );
    assert!(body["runs"][0]["candidate_sha256"].as_str().is_some());
    assert!(body["runs"][0]["report_available"] == false);
}

#[tokio::test]
async fn history_pages_without_repeating_or_skipping() {
    let harness = Harness::new(0);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let mut created = Vec::new();
    for _ in 0..5 {
        let (_, body) = harness.submit_check(&project_id, &token).await;
        created.push(body["run_id"].as_str().expect("run id").to_string());
    }
    created.reverse();

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let path = match &cursor {
            Some(c) => format!("/v1/projects/{project_id}/runs?limit=2&cursor={c}"),
            None => format!("/v1/projects/{project_id}/runs?limit=2"),
        };
        let (status, body) = harness.get(&path, &token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for run in body["runs"].as_array().expect("runs") {
            seen.push(run["run_id"].as_str().expect("id").to_string());
        }
        match body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    assert_eq!(seen, created, "paging repeated or skipped a run");
}

#[tokio::test]
async fn history_can_be_narrowed_by_status() {
    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Example").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    harness.wait_until(&run_id, RunStatus::is_terminal).await;

    let (_, failed) = harness
        .get(
            &format!("/v1/projects/{project_id}/runs?status=failed"),
            &token,
        )
        .await;
    assert_eq!(failed["runs"].as_array().expect("runs").len(), 1);
    let (_, queued) = harness
        .get(
            &format!("/v1/projects/{project_id}/runs?status=queued"),
            &token,
        )
        .await;
    assert!(queued["runs"].as_array().expect("runs").is_empty());
}

// ----------------------------------------------------------------- adapters

#[tokio::test]
async fn the_adapter_list_is_the_engine_s_own() {
    let harness = Harness::new(0);
    let (status, body) = harness.get("/v1/adapters", OPERATOR).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let adapters = body["adapters"].as_array().expect("adapters");
    assert_eq!(adapters.len(), eplyx_engine::protocol::adapters().len() + 1);
    assert!(adapters
        .iter()
        .any(|a| a["adapter_id"] == "none@0" && a["speaks_semantics"] == false));
    for adapter in eplyx_engine::protocol::adapters() {
        assert!(
            adapters
                .iter()
                .any(|a| a["name"] == adapter.name() && a["program_id"] == adapter.program_id()),
            "{} is missing from the list",
            adapter.name()
        );
    }
    assert_eq!(
        body["semantic_schema_version"],
        eplyx_engine::semantics::SEMANTIC_SCHEMA_VERSION
    );
}

#[tokio::test]
async fn an_uploaded_path_cannot_escape_the_bundle() {
    let harness = Harness::new(0);
    let project_id = harness.create_project("Example").await;
    for name in [
        "../escape.json",
        "/etc/passwd",
        "a/../../b",
        "..",
        "nested/a/b/c/d/e.json",
    ] {
        let boundary = "eplyxescape";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; \
             filename=\"x\"\r\nContent-Type: application/octet-stream\r\n\r\n{{}}\r\n--{boundary}--\r\n"
        );
        let (status, response) = harness
            .send(
                authed(
                    "POST",
                    &format!("/v1/projects/{project_id}/bundles"),
                    OPERATOR,
                )
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("request"),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{name} was accepted: {response}"
        );
    }
}
