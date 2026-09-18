//! Guards the production pilot leaned on, kept as tests so they stay true.
//!
//! Two properties were verified against the deployed service by hand and would
//! have been expensive to discover later: that hosted replay consults no RPC
//! environment, and that the CI workflow carries no operator credential.
//! Neither is visible in a type, so neither survives without a test.

mod common;

use common::{ready_project, Harness};
use eplyx_server::registry::RunStatus;

/// The serving path is offline, and "offline" has to mean more than "we did not
/// configure an endpoint". If any part of the hosted check path read these, a
/// run would try to reach an address that does not route and would fail or
/// hang; it passes because nothing looks.
///
/// Its own test binary on purpose: these are process-wide, and a second test
/// running beside it would inherit them.
#[tokio::test]
async fn hosted_replay_consults_no_rpc_environment() {
    // Addresses in TEST-NET-1 (RFC 5737), which is reserved for documentation
    // and routes nowhere. A connection attempt cannot quietly succeed.
    for name in [
        "SOLANA_RPC_URL",
        "SOLANA_ARCHIVE_RPC_URL",
        "SOLANA_BLOCK_RPC_URL",
    ] {
        std::env::set_var(name, "http://192.0.2.1:1/unroutable");
    }

    let harness = Harness::new(1);
    let (project_id, token) = ready_project(&harness, "Offline").await;
    let (_, body) = harness.submit_check(&project_id, &token).await;
    let run_id = body["run_id"].as_str().expect("run id").to_string();
    let status = harness.wait_until(&run_id, RunStatus::is_terminal).await;

    // A verdict, not an execution error: the engine reached an answer without
    // the network. Which verdict is not this test's business.
    assert_ne!(
        status,
        RunStatus::ExecutionError,
        "hosted replay failed while RPC variables pointed at an unroutable \
         address, which means something on the check path read one"
    );

    let (_, run) = harness.get(&format!("/v1/runs/{run_id}"), &token).await;
    assert!(
        run["exit_code"].is_number(),
        "no exit code was produced: {run}"
    );
}

/// A token that lives in a pull request must not be able to change what future
/// pull requests are measured against. The workflow is where that could quietly
/// stop being true — one `secrets.EPLYX_OPERATOR_TOKEN` and every PR could
/// rotate the baseline it is judged by.
#[test]
fn the_ci_workflow_carries_no_operator_credential() {
    let workflow = std::fs::read_to_string("../.github/workflows/eplyx.yml")
        .expect("the CI workflow is part of the product surface and must exist");

    assert!(
        !workflow.contains("EPLYX_OPERATOR_TOKEN"),
        "the CI workflow references an operator credential"
    );
    assert!(
        !workflow.contains("OPERATOR"),
        "the CI workflow mentions an operator secret"
    );
    assert!(
        workflow.contains("secrets.EPLYX_TOKEN"),
        "the CI workflow does not use a project-scoped token"
    );
    // The gate's exit code is the job's exit code. A workflow that swallowed it
    // would report every run as green.
    assert!(
        workflow.contains("exit \"$code\""),
        "the workflow does not exit with the gate's own result"
    );
    // execution_error is not a verdict, and must not be reported as one.
    assert!(
        workflow.contains("75)"),
        "the workflow does not distinguish a run that produced no verdict"
    );
}
