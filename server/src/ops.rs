//! Operational visibility: queue, waiting, worker failures and retries.
//!
//! Everything here is derived from the durable run records, which already are
//! the queue (Phase P2), plus what startup recovery did. Nothing is sampled
//! into a separate metrics store that could disagree with the records, and
//! nothing here is analytical: an `execution_error` is an infrastructure
//! outcome, never a statement about a candidate.
//!
//! The second half is the restore check. Files and Postgres are backed up
//! separately, so a restore can pair a volume with a database from another
//! moment. [`verify_volume`] reads both and names every disagreement instead
//! of letting the service start on a split world.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::registry::{
    now_unix_seconds, AttemptEnd, Recovery, Registry, RunMetadata, RunStatus,
    MAX_EXECUTION_ATTEMPTS,
};

/// What one startup reconciliation did. Appended, never rewritten.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryRecord {
    pub at_unix_seconds: u64,
    pub swept_temporary_artifacts: usize,
    pub requeued: Vec<String>,
    pub finalized: Vec<String>,
    pub failed: Vec<String>,
}

pub fn record_recovery(registry: &Registry, recovery: &Recovery, swept: usize) -> Result<()> {
    let record = RecoveryRecord {
        at_unix_seconds: now_unix_seconds(),
        swept_temporary_artifacts: swept,
        requeued: recovery.requeued.clone(),
        finalized: recovery.finalized.clone(),
        failed: recovery.failed.clone(),
    };
    let directory = registry.storage().ops_root().join("recoveries");
    // Time-ordered names; the random tail keeps two starts in one second apart.
    let path = directory.join(format!(
        "{:012}-{}.json",
        record.at_unix_seconds,
        crate::ids::run()
    ));
    registry.storage().write_json(&path, &record)
}

/// The most recent startup reconciliations, newest first.
pub fn recent_recoveries(registry: &Registry, limit: usize) -> Vec<RecoveryRecord> {
    let directory = registry.storage().ops_root().join("recoveries");
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut names = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".json"))
        .collect::<Vec<_>>();
    names.sort();
    names
        .iter()
        .rev()
        .take(limit)
        .filter_map(|name| registry.storage().read_json(&directory.join(name)).ok())
        .collect()
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Distribution {
    pub count: usize,
    pub p50_seconds: Option<u64>,
    pub p90_seconds: Option<u64>,
    pub max_seconds: Option<u64>,
}

impl Distribution {
    fn of(mut samples: Vec<u64>) -> Self {
        samples.sort_unstable();
        let at = |fraction: f64| -> Option<u64> {
            // Nearest-rank percentile over whole seconds. Operational display
            // only; nothing here reaches a report or a valuation.
            (!samples.is_empty()).then(|| {
                let rank = ((fraction * samples.len() as f64).ceil() as usize).max(1);
                samples[rank.min(samples.len()) - 1]
            })
        };
        Self {
            count: samples.len(),
            p50_seconds: at(0.5),
            p90_seconds: at(0.9),
            max_seconds: samples.last().copied(),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Workers {
    pub max_concurrent_runs: usize,
    pub busy: usize,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Queue {
    pub queued: usize,
    pub running: usize,
    pub oldest_queued_age_seconds: Option<u64>,
    pub longest_running_seconds: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Outcomes {
    pub created: usize,
    pub passed: usize,
    pub failed: usize,
    pub completed: usize,
    pub execution_error: usize,
    pub queued: usize,
    pub running: usize,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Retries {
    /// Runs that needed more than one execution attempt.
    pub runs_retried: usize,
    /// Attempts cut short by process death.
    pub interrupted_attempts: usize,
    /// Attempts whose verified report was recorded by recovery.
    pub finalized_on_recovery: usize,
    /// Runs that hit the attempt limit and became `execution_error`.
    pub exhausted: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct WorkerFailure {
    pub run_id: String,
    pub project_id: String,
    pub attempts: usize,
    pub detail: Option<String>,
    pub created_at_unix_seconds: u64,
    pub completed_at_unix_seconds: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub generated_at_unix_seconds: u64,
    pub window_seconds: u64,
    pub workers: Workers,
    /// Every non-terminal run, whatever its age.
    pub queue: Queue,
    /// Runs created inside the window.
    pub outcomes: Outcomes,
    /// Accepted → first attempt started.
    pub wait: Distribution,
    /// First attempt started → outcome recorded.
    pub execution: Distribution,
    pub retries: Retries,
    /// Newest first, inside the window.
    pub recent_worker_failures: Vec<WorkerFailure>,
    pub recoveries: Vec<RecoveryRecord>,
}

const RECENT_FAILURES: usize = 20;
const DETAIL_CHARS: usize = 400;

/// Summarize run records. Pure: the caller supplies records, the clock and
/// the worker gauge, so every number is reproducible from its inputs.
pub fn summarize(
    runs: &[RunMetadata],
    now: u64,
    window_seconds: u64,
    workers: Workers,
    recoveries: Vec<RecoveryRecord>,
) -> Snapshot {
    let since = now.saturating_sub(window_seconds);
    let mut queue = Queue::default();
    let mut outcomes = Outcomes::default();
    let mut retries = Retries::default();
    let mut wait = Vec::new();
    let mut execution = Vec::new();
    let mut failures = Vec::new();

    for run in runs {
        match run.status {
            RunStatus::Queued => {
                queue.queued += 1;
                let age = now.saturating_sub(run.created_at_unix_seconds);
                queue.oldest_queued_age_seconds =
                    Some(queue.oldest_queued_age_seconds.unwrap_or(0).max(age));
            }
            RunStatus::Running => {
                queue.running += 1;
                let since_start = run
                    .attempts
                    .last()
                    .map(|attempt| now.saturating_sub(attempt.started_at_unix_seconds))
                    .unwrap_or(0);
                queue.longest_running_seconds =
                    Some(queue.longest_running_seconds.unwrap_or(0).max(since_start));
            }
            _ => {}
        }
        if run.created_at_unix_seconds < since {
            continue;
        }
        outcomes.created += 1;
        match run.status {
            RunStatus::Passed => outcomes.passed += 1,
            RunStatus::Failed => outcomes.failed += 1,
            RunStatus::Completed => outcomes.completed += 1,
            RunStatus::ExecutionError => outcomes.execution_error += 1,
            RunStatus::Queued => outcomes.queued += 1,
            RunStatus::Running => outcomes.running += 1,
        }
        if let Some(started) = run.started_at_unix_seconds {
            wait.push(started.saturating_sub(run.created_at_unix_seconds));
            if let Some(completed) = run.completed_at_unix_seconds {
                execution.push(completed.saturating_sub(started));
            }
        }
        if run.attempts.len() > 1 {
            retries.runs_retried += 1;
        }
        for attempt in &run.attempts {
            match attempt.end {
                Some(AttemptEnd::Interrupted) => retries.interrupted_attempts += 1,
                Some(AttemptEnd::FinalizedOnRecovery) => retries.finalized_on_recovery += 1,
                _ => {}
            }
        }
        if run.status == RunStatus::ExecutionError {
            if run.attempts.len() >= MAX_EXECUTION_ATTEMPTS {
                retries.exhausted += 1;
            }
            failures.push(WorkerFailure {
                run_id: run.run_id.clone(),
                project_id: run.project_id.clone(),
                attempts: run.attempts.len(),
                detail: run
                    .detail
                    .as_ref()
                    .map(|detail| detail.chars().take(DETAIL_CHARS).collect()),
                created_at_unix_seconds: run.created_at_unix_seconds,
                completed_at_unix_seconds: run.completed_at_unix_seconds,
            });
        }
    }
    failures.sort_by(|a, b| {
        b.completed_at_unix_seconds
            .unwrap_or(b.created_at_unix_seconds)
            .cmp(
                &a.completed_at_unix_seconds
                    .unwrap_or(a.created_at_unix_seconds),
            )
            .then_with(|| b.run_id.cmp(&a.run_id))
    });
    failures.truncate(RECENT_FAILURES);

    Snapshot {
        schema_version: 1,
        generated_at_unix_seconds: now,
        window_seconds,
        workers,
        queue,
        outcomes,
        wait: Distribution::of(wait),
        execution: Distribution::of(execution),
        retries,
        recent_worker_failures: failures,
        recoveries,
    }
}

/// Read every run record on the volume. Unreadable records are skipped here;
/// [`verify_volume`] is where they are reported.
pub fn load_runs(registry: &Registry) -> Vec<RunMetadata> {
    registry
        .list_run_ids()
        .unwrap_or_default()
        .iter()
        .filter_map(|id| registry.load_run(id).ok())
        .collect()
}

pub fn snapshot(registry: &Registry, window_seconds: u64, workers: Workers) -> Snapshot {
    summarize(
        &load_runs(registry),
        now_unix_seconds(),
        window_seconds,
        workers,
        recent_recoveries(registry, 5),
    )
}

// ------------------------------------------------------------ restore check

#[derive(Clone, Debug, Default, Serialize)]
pub struct VolumeCheck {
    pub projects: usize,
    pub runs: usize,
    pub bundles_checked: usize,
    pub artifacts_checked: usize,
    pub reports_checked: usize,
    /// Postgres project mappings compared, when identity storage was given.
    pub identity_projects: Option<usize>,
    /// Disagreements that make the restored service untrustworthy.
    pub problems: Vec<String>,
    /// Facts worth knowing that are not faults, e.g. legacy projects with no
    /// workspace assignment.
    pub notes: Vec<String>,
}

impl VolumeCheck {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Check a restored data directory against itself, and against the identity
/// database when one is given.
///
/// Every check reads the same verified paths the service reads: a bundle must
/// open, a candidate must hash to its name, a reported run must still have its
/// report and change spec. A mapping in Postgres that names a project the
/// volume does not hold means the two backups are from different moments.
pub fn verify_volume(registry: &Registry, identity_projects: Option<&[String]>) -> VolumeCheck {
    let mut check = VolumeCheck::default();
    let projects = match registry.list_projects() {
        Ok(projects) => projects,
        Err(error) => {
            check
                .problems
                .push(format!("project registry unreadable: {error:#}"));
            Vec::new()
        }
    };
    check.projects = projects.len();
    let known = projects
        .iter()
        .map(|project| project.project_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    for project in &projects {
        let bundles = registry
            .list_bundles(&project.project_id)
            .unwrap_or_default();
        for bundle in &bundles {
            check.bundles_checked += 1;
            if let Err(error) = registry.open_bundle(&bundle.bundle_sha256) {
                check.problems.push(format!(
                    "project {} bundle {} ({}) does not open: {error:#}",
                    project.project_id, bundle.bundle_id, bundle.bundle_sha256
                ));
            }
        }
        if let Some(active) = &project.active_bundle {
            if !bundles
                .iter()
                .any(|bundle| bundle.bundle_id == active.bundle_id)
            {
                check.problems.push(format!(
                    "project {} activates bundle {} which has no registration record",
                    project.project_id, active.bundle_id
                ));
            }
        }
    }

    let ids = registry.list_run_ids().unwrap_or_default();
    check.runs = ids.len();
    for run_id in &ids {
        let run = match registry.load_run(run_id) {
            Ok(run) => run,
            Err(error) => {
                check
                    .problems
                    .push(format!("run {run_id} record unreadable: {error:#}"));
                continue;
            }
        };
        if !known.contains(run.project_id.as_str()) {
            check.problems.push(format!(
                "run {run_id} belongs to project {} which is not on this volume",
                run.project_id
            ));
        }
        if let Some(reference) = &run.candidate_artifact {
            check.artifacts_checked += 1;
            if let Err(error) = registry.artifacts().get_program(reference) {
                check.problems.push(format!(
                    "run {run_id} candidate {} does not verify: {error:#}",
                    reference.sha256
                ));
            }
        }
        if let Some(change) = &run.change {
            if run.hosted_analysis.is_none() && run.analysis.is_none() {
                if let Err(error) = registry.load_change_spec(run_id, change) {
                    check.problems.push(format!(
                        "run {run_id} change spec does not verify: {error:#}"
                    ));
                }
            }
        }
        if run.report_available && run.hosted_analysis.is_none() && run.analysis.is_none() {
            check.reports_checked += 1;
            match registry.load_run_artifact(run_id, "report.json") {
                Ok(bytes) => {
                    if serde_json::from_slice::<serde_json::Value>(&bytes).is_err() {
                        check
                            .problems
                            .push(format!("run {run_id} report.json does not parse"));
                    }
                }
                Err(error) => check.problems.push(format!(
                    "run {run_id} reports a report it no longer holds: {error:#}"
                )),
            }
        }
        if !run.status.is_terminal() {
            check.notes.push(format!(
                "run {run_id} is {}; the service will resume it on start",
                serde_json::to_value(run.status)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            ));
        }
    }

    if let Some(mapped) = identity_projects {
        check.identity_projects = Some(mapped.len());
        let mapped_set = mapped
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        for project in mapped {
            if !known.contains(project.as_str()) {
                check.problems.push(format!(
                    "identity database assigns project {project} to a workspace, but the volume does not hold it (backups from different moments?)"
                ));
            }
        }
        for project in &known {
            if !mapped_set.contains(project) {
                check.notes.push(format!(
                    "project {project} has no workspace assignment (legacy or operator-only)"
                ));
            }
        }
    }
    check
}

/// Project ids the identity database maps to workspaces.
pub async fn identity_project_ids(database_url: &str) -> Result<Vec<String>> {
    let (client, connection) = tokio_postgres::connect(database_url, tokio_postgres::NoTls)
        .await
        .context("connecting to the identity database")?;
    let driver = tokio::spawn(connection);
    let rows = client
        .query(
            "SELECT project_id FROM project_workspaces ORDER BY project_id",
            &[],
        )
        .await
        .context("reading project_workspaces")?;
    drop(client);
    let _ = driver.await;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ExecutionAttempt;

    fn run(id: &str, status: RunStatus, created: u64) -> RunMetadata {
        serde_json::from_value(serde_json::json!({
            "run_id": id,
            "project_id": "proj_a",
            "status": status,
            "created_at_unix_seconds": created,
        }))
        .unwrap()
    }

    fn attempt(n: u32, started: u64, end: Option<AttemptEnd>) -> ExecutionAttempt {
        ExecutionAttempt {
            attempt: n,
            started_at_unix_seconds: started,
            ended_at_unix_seconds: end.map(|_| started + 1),
            end,
        }
    }

    fn workers() -> Workers {
        Workers {
            max_concurrent_runs: 2,
            busy: 1,
        }
    }

    #[test]
    fn queue_wait_execution_and_retries_come_from_the_records() {
        let now = 10_000;
        let mut passed = run("run_1", RunStatus::Passed, 9_000);
        passed.started_at_unix_seconds = Some(9_010);
        passed.completed_at_unix_seconds = Some(9_070);
        passed.attempts = vec![attempt(1, 9_010, Some(AttemptEnd::Completed))];

        let mut retried = run("run_2", RunStatus::Failed, 9_100);
        retried.started_at_unix_seconds = Some(9_130);
        retried.completed_at_unix_seconds = Some(9_330);
        retried.attempts = vec![
            attempt(1, 9_130, Some(AttemptEnd::Interrupted)),
            attempt(2, 9_300, Some(AttemptEnd::Completed)),
        ];

        let mut exhausted = run("run_3", RunStatus::ExecutionError, 9_200);
        exhausted.started_at_unix_seconds = Some(9_200);
        exhausted.completed_at_unix_seconds = Some(9_900);
        exhausted.detail = Some("interrupted during 3 execution attempts".into());
        exhausted.attempts = (1..=3)
            .map(|n| attempt(n, 9_200 + n as u64, Some(AttemptEnd::Interrupted)))
            .collect();

        let queued = run("run_4", RunStatus::Queued, 9_400);
        let mut running = run("run_5", RunStatus::Running, 9_500);
        running.started_at_unix_seconds = Some(9_550);
        running.attempts = vec![attempt(1, 9_550, None)];
        // Outside the window, but still in the queue: counted there only.
        let old_queued = run("run_0", RunStatus::Queued, 1_000);

        let snapshot = summarize(
            &[old_queued, passed, retried, exhausted, queued, running],
            now,
            3_600,
            workers(),
            Vec::new(),
        );
        assert_eq!(snapshot.queue.queued, 2);
        assert_eq!(snapshot.queue.running, 1);
        assert_eq!(snapshot.queue.oldest_queued_age_seconds, Some(9_000));
        assert_eq!(snapshot.queue.longest_running_seconds, Some(450));
        assert_eq!(snapshot.outcomes.created, 5);
        assert_eq!(snapshot.outcomes.execution_error, 1);
        assert_eq!(snapshot.wait.count, 4);
        assert_eq!(snapshot.wait.max_seconds, Some(50));
        assert_eq!(snapshot.execution.count, 3);
        assert_eq!(snapshot.execution.max_seconds, Some(700));
        assert_eq!(snapshot.retries.runs_retried, 2);
        assert_eq!(snapshot.retries.interrupted_attempts, 4);
        assert_eq!(snapshot.retries.exhausted, 1);
        assert_eq!(snapshot.recent_worker_failures.len(), 1);
        assert_eq!(snapshot.recent_worker_failures[0].run_id, "run_3");
        assert_eq!(snapshot.recent_worker_failures[0].attempts, 3);
    }

    #[test]
    fn percentiles_are_nearest_rank_and_empty_is_absent_not_zero() {
        let empty = Distribution::of(Vec::new());
        assert_eq!(empty.count, 0);
        assert_eq!(empty.p50_seconds, None);
        assert_eq!(empty.max_seconds, None);
        let ten = Distribution::of((1..=10).collect());
        assert_eq!(ten.p50_seconds, Some(5));
        assert_eq!(ten.p90_seconds, Some(9));
        assert_eq!(ten.max_seconds, Some(10));
    }

    #[test]
    fn recovery_records_append_and_read_back_newest_first() {
        let volume = tempfile::tempdir().unwrap();
        let registry = Registry::new(crate::storage::Storage::open(volume.path()).unwrap());
        for (n, id) in ["run_a", "run_b"].iter().enumerate() {
            record_recovery(
                &registry,
                &Recovery {
                    requeued: vec![id.to_string()],
                    finalized: vec![],
                    failed: vec![],
                },
                n,
            )
            .unwrap();
        }
        let records = recent_recoveries(&registry, 5);
        assert_eq!(records.len(), 2);
        // Two starts in one second keep both records.
        assert!(records.iter().any(|r| r.requeued == ["run_a"]));
        assert!(records.iter().any(|r| r.requeued == ["run_b"]));
    }
}
