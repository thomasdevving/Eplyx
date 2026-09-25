//! Orchestration for a run that outlives the request which created it.
//!
//! This module decides *when* the engine runs and *where its answer is
//! written*. It decides nothing about what the answer means. No finding, no
//! severity, no expectation review, no coverage figure and no exit code
//! originates here: `ci::check_change` produces all of it, exactly as it does for
//! `eplyx ci check` on a laptop. Moving any of that judgement into this file
//! would create a second implementation, and then two answers.

use std::sync::Arc;

use eplyx_engine::change::CandidateSource;
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
        // Only the scratch cache goes. The candidate artefact is shared and
        // immutable, and stays; so do the spec, the report and the record.
        if let Err(error) = state.registry.clear_run_work(&run_id) {
            eprintln!("run {run_id}: could not clear its scratch cache: {error}");
        }
        drop(permit);
    });
}

/// Start a worker for every run startup recovery put back in the queue.
///
/// Oldest first, so recovered work keeps its place; each still waits for a
/// permit like any new run, and each is claimed by compare-and-set, so a run
/// that is somehow enqueued twice still executes once.
pub fn resume(state: &Arc<AppState>, run_ids: &[String]) {
    for run_id in run_ids {
        spawn(Arc::clone(state), run_id.clone());
    }
}

/// Everything that touches the engine, on a blocking thread.
///
/// Every input is rebuilt from a durable identity; nothing comes from a
/// filename, from memory, or from the project's current state:
///
/// ```text
/// metadata.change + candidate_artifact           the registry's record
/// runs/<id>/change_spec.json   re-verified       == metadata.change
/// artifacts/programs/<sha256>  hash-on-read      == spec candidate == metadata artifact
/// bundles/<bundle_sha256>      verified open     the bundle pinned at creation
/// runs/<id>/expected-changes.toml  hash-pinned   the declarations it was accepted with
///   → ci::check_change(bundle, Spec { spec, Bytes(verified bytes) })
/// ```
///
/// The bytes handed to the engine are the bytes whose hash was just checked,
/// not a path the engine opens again later.
///
/// The split between the two failure outcomes is the engine's own: an error
/// from `ci::check_change` carries a real Eplyx exit code and is a real gate
/// result — a malformed expectation file is 2, a spec that no longer binds to
/// its bundle is 4 — so it is recorded as a failed run with that code and no
/// report. A run whose own durable inputs cannot be trusted (no spec, a spec
/// that no longer verifies, an artefact that is gone or altered, a pinned
/// bundle that is missing or fails verification) is this service failing to
/// keep what it accepted, and becomes an execution error: that is never a
/// statement about the candidate.
fn execute(state: &AppState, run_id: &str) -> RunOutcome {
    let service_fault = |detail: String| RunOutcome::ExecutionError { detail };
    let registry = &state.registry;
    let metadata = match registry.load_run(run_id) {
        Ok(metadata) => metadata,
        Err(error) => return service_fault(format!("the run record could not be read: {error}")),
    };
    // No fallback to "the uploaded bytes against whatever bundle is active":
    // a run without a proposal has nothing it was accepted to evaluate.
    let Some(change) = metadata.change.as_ref() else {
        return service_fault(
            "the run records no change spec, so there is nothing to evaluate".into(),
        );
    };
    let Some(artifact) = metadata.candidate_artifact.as_ref() else {
        return service_fault(
            "the run records no durable candidate artifact, so there is nothing to execute".into(),
        );
    };
    let spec = match registry.load_change_spec(run_id, change) {
        Ok(spec) => spec,
        Err(error) => {
            return service_fault(format!(
                "the stored change spec is not trustworthy: {error:#}"
            ))
        }
    };
    if !artifact.matches(spec.candidate()) {
        return service_fault(format!(
            "the run's artifact {} is not the candidate its change spec names ({})",
            artifact.sha256,
            spec.candidate().sha256
        ));
    }
    let bytes = match registry.artifacts().get_program(artifact) {
        Ok(bytes) => bytes,
        Err(error) => {
            return service_fault(format!(
                "the accepted candidate is no longer available: {error:#}"
            ))
        }
    };
    if let Err(error) = registry.open_bundle(&metadata.bundle_sha256) {
        return service_fault(format!("the pinned bundle is unavailable: {error:#}"));
    }
    let bundle_dir = match registry.storage().bundle_path(&metadata.bundle_sha256) {
        Ok(path) => path,
        Err(error) => {
            return service_fault(format!("the bundle path could not be resolved: {error}"))
        }
    };

    // The declarations are hash-verified from their durable copy and written to
    // the scratch cache, because the engine takes a path. The cache copy is
    // derived; losing it loses nothing.
    let expectations = match registry.load_expectations(&metadata) {
        Ok(None) => None,
        Ok(Some(bytes)) => {
            let staged = registry.run_work_dir(run_id).and_then(|work| {
                std::fs::create_dir_all(&work)?;
                let path = work.join("expected-changes.toml");
                std::fs::write(&path, &bytes)?;
                Ok(path)
            });
            match staged {
                Ok(path) => Some(path),
                Err(error) => {
                    return service_fault(format!("staging the expected changes: {error:#}"))
                }
            }
        }
        Err(error) => {
            return service_fault(format!(
                "the run's expected changes are not trustworthy: {error:#}"
            ))
        }
    };

    // The engine, called directly. There is no second implementation of any of
    // this, and nothing is shelled out to. It binds the spec to the pinned
    // bundle again and verifies the bytes against the spec again: the checks at
    // acceptance were early answers, not substitutes for these.
    let input = ci::ChangeInput::Spec {
        spec: &spec,
        source: Some(CandidateSource::Bytes(&bytes)),
    };
    match ci::check_change(&bundle_dir, &input, expectations.as_deref()) {
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
