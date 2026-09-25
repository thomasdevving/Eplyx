# Phase P2 — durable hosted analyses

## Decision

A hosted run is now reconstructible from three durable identities, and
nothing else:

```text
pinned bundle         bundles/<bundle_sha256>/              (Phase 10, unchanged)
stored ChangeSpec     runs/<id>/change_spec.json            (Phase P1, re-verified on read)
candidate artefact    artifacts/programs/<sha256>           (new: global, immutable, hash-on-read)
```

Every input a run needs is persisted before its record exists. The record
holds its own queue state, so a restart re-enqueues accepted work instead of
failing it, and a run that was executing when the process died gets another
**attempt** of the same run. Replay, semantic evaluation, ChangeSpec identity
and the canonical report are unchanged.

## 1. Inventory — before P2

| Concern | Where it lived (P1) | Durable? |
| --- | --- | --- |
| Candidate upload | request body, in memory | — |
| Candidate after acceptance | `runs/<id>/work/artifacts/programs/<sha>` via `EvidenceStore::put` | **No.** Per-run scratch directory, deleted when the run ended. `put` wrote straight to the final address, so a crash mid-write left a partial object at a valid-looking name. |
| `expected-changes.toml` | `runs/<id>/work/expected-changes.toml` | **No.** Same scratch directory, not hash-pinned. |
| ChangeSpec | `runs/<id>/change_spec.json` | Yes (temp + rename), re-verified on read. |
| Bundle pinning | `metadata.bundle_sha256` → `bundles/<sha>` | Yes, content-addressed and verified on open. |
| Run registry | `runs/<id>/metadata.json` | Temp + rename, **no fsync**: a crash after rename could still lose the write. |
| Queued | `status: queued` plus a `tokio::spawn` task waiting on the semaphore | **The task was in memory only.** |
| Worker pickup | `begin_run` compare-and-set under an in-process mutex | Correct in one process; nothing stopped a second process over the same volume. |
| Running | `status: running` | Yes, but meaningless after a crash. |
| Result | `report.json`, `report.md`, then metadata | A crash between them left a complete report under a `running` record. |
| Restart | `recover_interrupted_runs`: **every** non-terminal run became `execution_error` | Correct but lossy: accepted work was thrown away. |
| Cleanup | `clear_run_work` after every run | Deleted the only copy of the candidate. |

The ephemeral dependencies were therefore: the candidate and the expectations
in the work directory, the in-memory task, the absence of fsync, and the
single-process assumption behind the claim.

## 2. The artefact store

`server/src/artifacts.rs`. The layout is the engine's evidence layout
(`programs/<sha256>`), so the same object is nameable by `EvidenceRef`. What
the per-run scratch store lacked, this store has:

| Property | How |
| --- | --- |
| Identity | `sha256(bytes)`, 64 lowercase hex; `ArtifactRef { sha256, len }` has the same shape as the spec's `candidate`. |
| Atomic creation | `tmp/<random>` → write → `fsync` → re-read and verify → `hard_link` to `programs/<sha256>` → `fsync` the directory → remove the temp file. A link appears whole or not at all. |
| Write-once | `hard_link` fails if the address exists; there is no overwrite path. |
| Deduplication | Existing address + verifies → reused (`created: false`). |
| Concurrent identical uploads | Race only on the link; the loser verifies the winner's object. |
| Damage | An existing object that fails verification is moved to `quarantine/` and the verified bytes take its address. This does not break immutability: the content of an address is fixed by its name. |
| Hash-on-read | existence → length from metadata → SHA-256 of the bytes read. A truncated or padded object is refused before it is read. |
| Paths | Only ever built from a validated canonical hash. No caller string becomes a path. |
| Limits | The configured upload limit (`EPLYX_MAX_CANDIDATE_BYTES`, 8 MiB by default) at the transport, and a hard 32 MiB store ceiling beneath it. |
| Temp cleanup | `tmp/` is swept at startup, under the volume lock. |

The engine's `EvidenceStore::put` was **not** changed. It is not atomic, but it
is used for build-time evidence packages, not by the hosted service, and P2
does not touch the engine's storage.

## 3–4. Upload path and deduplication

```text
candidate-only:  bytes → ChangeSpec::program_upgrade(pinned program, bytes)
explicit:        ChangeSpec::parse → bytes (uploaded, or retained for this project)
both:            ci::bind_change(pinned bundle)      409 + exit 4
                 spec.resolve(Bytes)                  400 + exit 2
                 artifacts.put_program(bytes)          500 on failure, no run
                 projects/<p>/artifacts/<sha> marker
                 runs/<id>/change_spec.json
                 runs/<id>/expected-changes.toml      (hash in metadata)
                 projects/<p>/runs/<id>, projects/<p>/changes/<cid>/<id>
                 runs/<id>/metadata.json               ← the run now exists
                 202
```

The record is written **last**. A crash before it leaves no run (at most an
orphan artefact, retained, and an index entry that every listing skips). A
crash after it leaves a queued run whose inputs are all durable.

One artefact serves every run and every ChangeSpec that names it. A derived
spec and an explicit spec with an activation slot over the same bytes have
different change IDs and one object.

## 5, 14. Hash-on-read and the identity chain

The worker reads nothing by filename. It checks:

```text
metadata.change            == stored ChangeSpec (P1)
metadata.candidate_artifact == spec.candidate               (else execution_error)
sha256(bytes read)          == candidate_artifact           (else execution_error)
report.change               == metadata.change              (finish_run, P1)
report.candidate            == candidate_artifact           (finish_run, new)
```

The bytes handed to `ci::check_change` are the bytes whose hash was just
checked (`CandidateSource::Bytes`), not a path the engine opens again. Any
mismatch is an infrastructure failure, never a verdict.

## 7, 18. Immutability and retention

**Every artefact is retained.** Nothing in the service deletes from
`programs/`. `clear_run_work` deletes only `runs/<id>/work/`, which is now a
scratch cache (the worker writes the verified expectations there because the
engine takes a path) and is never authoritative.

Unreferenced artefacts can exist only when a request fails after
`put_program` and before its run record is written. They are retained. A future
collector must count references from `runs/*/metadata.json`
(`candidate_artifact`) and must never collect on age alone. None is built.

## 8–9. The run record is the queue entry

`RunMetadata` gains:

```json
"candidate_artifact": { "sha256": "…", "len": 96736 },
"expectations_sha256": null,
"attempts": [
  { "attempt": 1, "started_at_unix_seconds": …, "ended_at_unix_seconds": …, "end": "interrupted" },
  { "attempt": 2, "started_at_unix_seconds": …, "ended_at_unix_seconds": …, "end": "completed" }
]
```

A `queued` record is a durable queue entry, and it can answer every question
the worker has: which change, against which pinned bundle, over which artefact,
with which declarations. The in-memory task is only a wake-up for work that is
already on disk. Every registry write is now `fsync`ed before the rename and the
directory after it.

## 10–12. Recovery semantics

Startup (`eplyx-server serve`) does, in order:

1. **Take the volume lock** (`data/.lock`, an exclusive `flock`). A second
   server over the same volume refuses to start, so recovery can never
   re-enqueue a run another live process is executing.
2. Sweep `artifacts/tmp/`.
3. `Registry::recover_runs`, run by run:

| Found | Action | Result |
| --- | --- | --- |
| terminal | left untouched; its scratch cache is cleared | — |
| non-terminal, no `change` or no `candidate_artifact` (accepted before P2) | `execution_error`: "accepted before candidates were stored durably … resubmit" | `failed` |
| `queued` | stays `queued` | `requeued` |
| `running`, and `report.json` is a complete, verified report for exactly this run | recorded as the run's result; `report.md` re-rendered; attempt → `finalized_on_recovery` | `finalized` |
| `running`, anything less | attempt → `interrupted`; any partial or untrusted report deleted; status → `queued` | `requeued` |
| `running`, and this was attempt `MAX_EXECUTION_ATTEMPTS` (3) | attempt → `interrupted`; `execution_error` "interrupted … during 3 execution attempts; it is not retried again" | `failed` |

4. Start the server, then `worker::resume` for every requeued run, oldest
   first, behind the same semaphore as new work.

"Complete, verified report" means that `report.json` parses as `CiReport`,
`report.change` equals the run's change, `report.candidate` equals its
artefact, and `report.bundle.sha256` equals its pinned bundle. `write_bytes` is
temp + fsync + rename, so a torn file cannot appear at `report.json`. A
truncated or foreign file there is still refused and removed, not trusted.

**Why a retry is safe.** Nothing an execution reads is mutated by it: the
bundle, the spec, the artefact and the declarations are immutable and
hash-verified, and the engine is deterministic. An execution writes only its
report and the run's terminal record, both through `finish_run`, which refuses
to finish a terminal run. So a retry produces the same report bytes. The test
compares the report after a restart to a local `eplyx ci check`, byte for byte.

**No duplicate executions.** Within a process, `begin_run` is a
compare-and-set, so a run enqueued three times is still executed once
(`attempts.len() == 1`). Across processes, the volume lock forbids a second
process. **No duplicate runs.** A retry is another attempt of the same run:
same run ID, same history entry, same change.

## 13. Result durability

`report.json` and `report.md` are written through the durable path, before
the metadata that advertises them. A completed run is never touched by recovery
(its metadata bytes are identical before and after a restart). A run that died
after writing its report is finalized from that report rather than recomputed.

## 15. Artefact API

```text
GET /v1/projects/{project_id}/artifacts/{sha256}
→ 200 { "sha256": "…", "len": 96736, "retained": true }   verified on this read
→ 404 if this project never supplied it (whether or not another project did)
→ 400 non-canonical hash;  500 held but does not verify
```

The artefact API returns metadata only: no bytes, no paths, no mutation. The check endpoint also
accepts an explicit `change_spec` **without** a `candidate` part when this project
already supplied that artefact. That is what lets a proposal be analysed again
without re-uploading. Both are scoped to the project. The store is global and
deduplicated, but knowing another project's hash is not enough to detect or
execute its bytes.

## 16–17. Governance and re-analysis

```text
governance proposal → change_spec_id → candidate.sha256
  → GET /v1/projects/{p}/artifacts/{sha256}        Eplyx already holds the bytes
  → POST /checks { change_spec }                   analysis, no upload
```

Re-analysis against a different baseline is already representable: the run is
`(change_spec_id, candidate_artifact, bundle_sha256)`. The spec and the artefact
do not mention a bundle, and a new run against a newly activated bundle reuses
both. The evaluation identity is the run, and the ChangeSpec identity does not
move. Automatic re-analysis is not built.

## 20. The `no_semantic_coverage` headline

A check that fails only because nothing could be evaluated is now shown as
**"Economic impact could not be evaluated for this interaction."**:

- report page: headline, a neutral *Not evaluated* verdict box with
  `Exit code 2 · no_semantic_coverage`, and a new *Why the gate did not pass*
  list carrying the technical reason names. When no-coverage runs also detected
  unnameable changes, the note says so and lists them;
- `eplyx-submit.sh` job summary headline;
- `report.md` subtitle (`ci_markdown`). This changes presentation only: the
  gate still fails with exit 2, and `report.json` is unchanged.

The headline for a real undeclared change is unchanged.

## 21. Frontend

The flow is unchanged. A run resumed after a restart keeps its URL, and its
queued page adds "Resumed after a server restart; this is the same run."

## 22. Failure classification

| Failure | Outcome | Test |
| --- | --- | --- |
| artefact store write fails at acceptance | 500, **no run created** | `no_run_exists_whose_artifact_was_not_stored` |
| artefact deleted after acceptance (incl. across restart) | `execution_error`, no exit code | `a_missing_artifact_after_restart_is_an_execution_error`, `a_missing_candidate_fails_closed` |
| artefact corrupted, same length | `execution_error`; artefact API 500 | `a_corrupted_artifact_is_an_execution_error` |
| run record re-pointed at another held artefact | `execution_error` | `a_run_pointed_at_another_held_artifact_is_an_execution_error` |
| stored ChangeSpec tampered / rewritten | `execution_error` (P1) | `a_tampered_stored_spec_fails_closed`, `a_consistently_rewritten_spec_…` |
| stored expectations altered | `execution_error` | `altered_expectations_are_an_execution_error` |
| pinned bundle unavailable | `execution_error` | `an_unavailable_pinned_bundle_is_an_execution_error` |
| server dies while queued | re-enqueued, completes as the same run | `a_queued_run_survives_a_restart`; `scripts/async-demo.sh` (real `kill -9`) |
| worker dies while running | attempt `interrupted`, retried as attempt 2 of the same run | `an_interrupted_run_is_retried_as_the_same_run` |
| dies repeatedly | `execution_error` after 3 attempts | `a_run_interrupted_too_often_stops_being_retried` |
| dies after report write, before finalize | finalized from the verified report, not recomputed | `a_report_written_before_the_crash_is_recorded_not_recomputed` |
| partial or foreign report on disk | discarded, re-executed | `a_partial_or_foreign_report_is_never_trusted_on_recovery` |
| duplicate worker pickup | one execution, one attempt | `a_run_is_executed_once_however_often_it_is_enqueued`, `a_run_can_only_be_claimed_once` |
| concurrent identical uploads | all accepted, one object, no temp debris | `concurrent_identical_uploads_are_safe`, `artifacts::tests::concurrent_identical_uploads_agree` |
| second server on the same volume | refused at startup | `one_process_holds_the_data_directory`; checked by hand against the release binary |

## 23. Backward compatibility

- Completed runs of any age are readable and never rewritten.
- A pre-P2 run still `queued` or `running` at restart has no durable artefact
  identity. It becomes `execution_error` with an explicit reason, as every
  interrupted run did before P2. Its work-directory candidate, if one exists,
  is **not** migrated into the store. That is the conservative choice: it
  avoids a second, rarely exercised code path that reconstructs inputs, and the
  window it affects is runs that were in flight at the moment of the upgrade.
- The request contract is unchanged, and `candidate_artifact` is added to the
  `202` body. `eplyx-submit.sh`, the workflow and every P1 test pass unchanged,
  except the four tests that pinned the old layout or the old restart policy.
  Those were rewritten to the new semantics.

## 24. Tests

- `server/src/artifacts.rs`: 7 unit tests covering dedup, hash/length on read,
  quarantine, partial writes never addressable, 8-thread concurrent put,
  canonical paths, and limits.
- `server/tests/durable_runs.rs`: 20 integration tests, listed in the table
  above, plus reuse without re-upload scoped to the project, a legacy queued
  run not resumed, the volume lock, and the full identity chain.
- Rewritten: `runs_interrupted_by_a_restart_are_recovered_rather_than_abandoned`,
  `accepted_inputs_are_durable_and_outlive_the_run`, and two P1
  artefact-damage tests pointed at the global store.
- Frontend (`report.test.mjs`): no-coverage headline, no-coverage plus detected
  changes, real undeclared change unchanged, resumed-run note. P1's
  `change.test.mjs` and `console.test.mjs` pass unchanged.
- `ci_markdown`: `no_semantic_coverage_is_not_worded_as_a_finding`.
- Six mutations, each disabling one guard: trust any report on recovery, never
  reconcile a report, the claim always wins, skip the hash on read, skip the
  artefact-equals-spec check, and no retry limit. Each is caught by a test
  that fails for that reason alone.
- Real processes: `scripts/async-demo.sh` kills the release server with
  `kill -9` while a run is queued, and the restarted server completes it under
  the same ID. The P1 live check passes unchanged.

## Known gaps, deliberately left

- A run killed **while executing** is exercised in-process, not with a real
  process kill: the fixture corpus finishes in about 0.2 s, faster than a kill
  can be aimed.
- One process per volume. A queue shared by several workers would need a
  lease, not a lock, and none is built.
- No artefact collection, and no listing of a project's artefacts.
- The engine's `EvidenceStore::put` is still not atomic (see §2).
