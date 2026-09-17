//! Orchestration for a run that outlives the request which created it.
//!
//! This module decides *when* the engine runs and *where its answer is
//! written*. It decides nothing about what the answer means. No finding, no
//! severity, no expectation review, no coverage figure and no exit code
//! originates here: `ci::check` produces all of it, exactly as it does for
//! `eplyx ci check` on a laptop. Moving any of that judgement into this file
//! would create a second implementation, and then two answers.

use std::sync::Arc;

use eplyx_engine::ci;

use crate::api::AppState;
use crate::registry::RunOutcome;

/// Start a run that is not scoped to the HTTP response.
///
/// `tokio::spawn` is the whole point. An axum handler's future is dropped when
/// the client goes away, and anything awaited inside it is cancelled with it. A
/// task spawned here is owned by the runtime instead, so a closed tab, a proxy
/// timeout or an aborted fetch cannot cancel an analysis the server already
/// accepted and promised a run id for.
pub fn spawn(state: Arc<AppState>, run_id: String) {
    tokio::spawn(async move {
        // Queued means precisely this: the run is persisted and is waiting
        // here. Running begins on the far side of this await.
        let permit = match state.runs.acquire().await {
            Ok(permit) => permit,
            Err(_) => {
                fail(
                    &state,
                    &run_id,
                    "the run scheduler shut down before this run started",
                );
                return;
            }
        };

        // Compare-and-set. A run that something else already claimed, or that
        // startup recovery already resolved, is left alone.
        match state.registry.begin_run(&run_id) {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                eprintln!("run {run_id}: could not be claimed: {error}");
                return;
            }
        }

        let outcome = {
            let state = Arc::clone(&state);
            let run_id = run_id.clone();
            tokio::task::spawn_blocking(move || execute(&state, &run_id)).await
        };
        // A panic inside the engine is this service failing to obtain a
        // verdict, not a verdict.
        let outcome = outcome.unwrap_or_else(|error| RunOutcome::ExecutionError {
            detail: format!("the run task did not complete: {error}"),
        });

        if let Err(error) = state.registry.finish_run(&run_id, outcome) {
            eprintln!("run {run_id}: could not be recorded: {error}");
        }
        // The uploaded binary goes away whatever happened. What survives is its
        // hash, the report and the run metadata.
        if let Err(error) = state.registry.clear_run_work(&run_id) {
            eprintln!("run {run_id}: could not clear inputs: {error}");
        }
        drop(permit);
    });
}

/// Everything that touches the engine, on a blocking thread.
///
/// The split between the two failure outcomes is the engine's own: an error
/// from `ci::check` carries a real Eplyx exit code and is a real gate result —
/// a malformed expectation file is 2, an incompatible bundle is 4 — so it is
/// recorded as a failed run with that code and no report, which is what a
/// preflight abort is. Only a fault of this service's own, where no verdict was
/// ever reached, becomes an execution error.
fn execute(state: &AppState, run_id: &str) -> RunOutcome {
    let metadata = match state.registry.load_run(run_id) {
        Ok(metadata) => metadata,
        Err(error) => {
            return RunOutcome::ExecutionError {
                detail: format!("the run record could not be read: {error}"),
            }
        }
    };
    let bundle_dir = match state
        .registry
        .storage()
        .bundle_path(&metadata.bundle_sha256)
    {
        Ok(path) => path,
        Err(error) => {
            return RunOutcome::ExecutionError {
                detail: format!("the bundle path could not be resolved: {error}"),
            }
        }
    };
    let work = match state.registry.run_work_dir(run_id) {
        Ok(path) => path,
        Err(error) => {
            return RunOutcome::ExecutionError {
                detail: format!("the run workspace could not be resolved: {error}"),
            }
        }
    };

    let candidate = work.join("candidate.so");
    if !candidate.exists() {
        return RunOutcome::ExecutionError {
            detail: "the uploaded candidate is no longer present".to_string(),
        };
    }
    let expectations = work.join("expected-changes.toml");
    let expectations = expectations.exists().then_some(expectations);

    // The engine, called directly. There is no second implementation of any of
    // this, and nothing is shelled out to.
    match ci::check(&bundle_dir, &candidate, expectations.as_deref()) {
        Ok(report) => {
            let markdown = eplyx_engine::ci_markdown::render(&report);
            RunOutcome::Reported {
                report: Box::new(report),
                markdown,
            }
        }
        Err(error) => RunOutcome::PreflightAbort {
            exit_code: error.exit_code(),
            detail: format!("{error:#}"),
        },
    }
}

fn fail(state: &AppState, run_id: &str, detail: &str) {
    if let Err(error) = state.registry.finish_run(
        run_id,
        RunOutcome::ExecutionError {
            detail: detail.to_string(),
        },
    ) {
        eprintln!("run {run_id}: could not be recorded: {error}");
    }
}
