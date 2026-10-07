# Operating Eplyx: regression CI, first checks, pull requests and recovery

This guide covers five operational surfaces added together: the repository's own
regression workflow, the guided path to a project's first upgrade check, the
pull-request comment, the operator view of the run queue, and joint backup and
restore of files and Postgres. None of them adds analysis. Each one projects
records the service already holds.

## Repository regression CI

`.github/workflows/ci.yml` runs on every pull request and every push to `main`.
Nothing in it skips. The cloud suites need Postgres, so the workflow starts a
loopback scratch cluster instead of leaving them out.

| Job | Runner | What it runs |
| --- | --- | --- |
| `programs` | macos-15 | `make test-artifacts`: every SBF artefact, hash-checked, uploaded once for the jobs below |
| `rust-lint` | macos-15 | `make fmt-check`, `make lint` |
| `rust-test` | macos-15 | `make test-programs`, `cargo test --locked --workspace` (engine, server, cloud/Postgres) |
| `browser` | macos-15 | `pnpm verify:report`, then the dashboard, cloud (Postgres), public-site and governance-trail browser suites |
| `frontend` | ubuntu | `check:frontend`, `test:frontend-runtime`, `verify:governance`, `check:legal`, the submit and PR-comment client tests |

The Rust jobs run on macOS because the pinned stake-pool candidates are only
qualified on Darwin arm64.

To get the same Postgres locally:

```bash
eval "$(scripts/scratch-postgres.sh start)"
```

```bash
cargo test --workspace
```

```bash
scripts/scratch-postgres.sh stop
```

The script creates a trust-auth cluster under `$TMPDIR/eplyx-scratch-postgres`.
It listens on `127.0.0.1:54329` only, with no Unix socket. It exports
`EPLYX_CLOUD_TEST_DATABASE_URL`, which `make test-cloud` and the cloud browser
suite read.

## Guided first upgrade check

`GET /v1/projects/{project_id}/setup` returns the ordered checklist that takes a
project to its first upgrade check. It accepts the same callers as
`/capabilities`: the operator, a workspace member or the project's own token.

| Step | Required | Satisfied by |
| --- | --- | --- |
| `project_enabled` | yes | project status is not `disabled` |
| `upgrade_target` | yes | a program id. `attention` when this build has no semantic adapter for it: every check would then exit 2 (`no_semantic_coverage`) |
| `bundle_registered` | yes | at least one registered bundle (evidence: newest bundle, record count) |
| `bundle_active` | yes | an active bundle that still opens on this volume. `blocked` until a bundle is registered |
| `ci_token` | yes | a live project token in either token store |
| `expectations` | no | a recent check that carried `expected-changes.toml` |
| `first_check` | yes | an upgrade run that reached `passed` or `failed`. `in_progress` while one is queued or running, `attention` when only `execution_error` runs exist |

Statuses are `done`, `attention`, `in_progress`, `todo`, `blocked` and
`optional`. The response also carries:

- `ready_for_first_check`: every required step before `first_check` is satisfied.
- `first_check_complete`
- `next_step`: the first unsatisfied required step.
- `repository`: the variables and secret `.github/workflows/eplyx.yml` reads.

Each step's `actions` name who can perform them: `operator`, `workspace_member`
or `repository`. Commands are filled in with the project and bundle ids and never
contain a credential. A project token can read operator commands; reading them
does not let it run them.

The checklist appears in three places:

- the operator console's project page;
- the cloud workspace overview of a project with an upgrade target, until the
  first check has a verdict;
- the CI step summary when `scripts/eplyx-submit.sh` exits 76.

A finished checklist means a check can run. It says nothing about any candidate.

## Pull-request comments

`.github/workflows/eplyx.yml` and `examples/github/eplyx-upgrade-impact.yml` run
`scripts/eplyx-pr-comment.py` after every check. The script keeps one comment per
pull request and project, found by the hidden marker
`<!-- eplyx-check:<project> -->` and updated in place on each push. The comment
contains:

1. One paragraph explaining the job result. The wording follows the workflow's
   exit-code messages: a pass is bounded by the corpus, and 75 and 76 are never
   described as verdicts.
2. The step summary from `eplyx-submit.sh`, which now includes a **Reproduce**
   section: the bundle and candidate hashes, and the local `eplyx ci check`
   command that reproduces the hosted `report.json` byte for byte.
3. Links to the workflow run, whose `eplyx-evidence` artefact holds
   report.json, report.md and the summary, and to the measured commit.

The workflow grants `pull-requests: write` to the job token only. On pull
requests from forks the token is read-only. The script then prints a warning,
the step is `continue-on-error`, and the gate result (the job's exit code) is
unchanged. No GitHub App is involved.

## Operator view of the queue

`GET /v1/ops?window_hours=24` (operator token only, 1–168 hours) and
`eplyx-server admin ops --window-hours 24` read the durable run records and
return:

- `workers`: configured concurrency and permits currently held. The admin
  command runs outside the server, so it reports `busy: 0`.
- `queue`: queued and running counts, the oldest queued age and the longest
  running attempt. These count every non-terminal run, whatever the window.
- `outcomes`: runs created in the window, by status.
- `wait` and `execution`: nearest-rank p50, p90 and max in whole seconds.
  `wait` runs from acceptance to the first attempt starting; `execution` from
  that start to the recorded outcome. With no samples these are `null`, never 0.
- `retries`: runs that needed more than one attempt, interrupted attempts,
  attempts finalized on recovery, and runs that hit the attempt limit.
- `recent_worker_failures`: the newest `execution_error` runs with attempt count
  and a truncated detail.
- `recoveries`: the last five startup reconciliations. `serve` appends one
  record under `ops/recoveries/` on every start.

The operator console's project list renders the same view. An `execution_error`
is an infrastructure outcome and describes no candidate.

## Backing up and restoring files and Postgres together

The data volume owns projects, runs, reports, bundles and artefacts. Postgres
owns users, sessions, workspaces, workspace→project assignments and user-issued
tokens. Back them up together:

```bash
scripts/eplyx-backup.sh backup --data-dir /data --database-url "$EPLYX_DATABASE_URL" --out backup-2026-10-07
```

Postgres is dumped first and the volume copied second. The volume only grows,
so every project the dump assigns is already on disk when the copy is taken.
Scratch work directories, temporary artefacts and the lock file are excluded.
The manifest pins both parts by SHA-256.

Restore into an empty directory and an empty database:

```bash
scripts/eplyx-backup.sh restore --from backup-2026-10-07 --data-dir /data-restored --database-url "$RESTORED_DATABASE_URL"
```

Restore refuses a non-empty directory, a database that already has tables, or a
part whose hash differs from its manifest. It then runs
`eplyx-server admin verify-volume`, and that command's exit code is the
script's. You can also run the check alone:

```bash
EPLYX_DATA_DIR=/data-restored EPLYX_DATABASE_URL="$RESTORED_DATABASE_URL" eplyx-server admin verify-volume
```

`verify-volume` reads the same verified paths the service reads, and reports a
problem (exit 1) when:

- a bundle does not open, or an activation has no registration record;
- a run's candidate artefact no longer hashes to its name, or its change spec no
  longer verifies;
- a reported run lacks its report;
- a run names a project the volume does not hold;
- Postgres assigns a workspace to a project the volume does not hold. This is
  the signature of a database newer than the files.

Non-terminal runs, and projects without a workspace assignment, are reported as
notes: the service resumes the first, and the second are legacy or
operator-only projects.

`files_and_postgres_restore_together_and_a_mismatched_pair_is_refused` in
`server/tests/cloud_recovery.rs` proves the round trip against real Postgres.
After the restore, a user signs in and sees their project, the project token
still authenticates, and the run's report is byte-identical. Pairing the backed-up
volume with a later database dump fails, and the failure names the missing
project.

Revocations and sessions written after the dump are not in the backup. After a
restore, rotate project tokens and expect users to sign in again.
