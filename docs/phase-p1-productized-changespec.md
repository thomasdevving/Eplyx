# Phase P1 — ChangeSpec as the product identity of an analysis

## Decision

Every hosted analysis is now an analysis **of one named proposal**. A run is
accepted for exactly one `ChangeSpec`, bound to the bundle it pins, stored
canonically, executed from that stored spec, and reported under the same
`change_spec_id` in the registry, the stored spec and the canonical report.
The ordinary experience does not change: upload a candidate `.so`, confirm
what Eplyx identified, analyse. Clients that already hold a spec (CI, release
tooling, a future governance flow) submit it beside the bytes.

No new change kind, no replay change, no new endpoint version. The request
contract is additive, and every frozen engine control is unchanged.

```text
POST /checks ─┬─ candidate only ─→ ChangeSpec::program_upgrade(pinned bundle program, bytes)
              └─ change_spec + candidate ─→ ChangeSpec::parse (authoritative)
                                   │
       ci::bind_change(pinned bundle, spec)            409 + exit 4 on mismatch
       spec.resolve(Bytes(candidate))                  400 + exit 2 on mismatch
                                   │
  runs/<id>/change_spec.json   metadata.change   projects/<p>/changes/<id>/<run>
  work/artifacts/programs/<sha256>                                   → 202
                                   │
  worker: load_change_spec (re-verify) → pinned bundle → resolve(Store) → ci::check_change
                                   │
  finish_run: report.change == metadata.change, or execution_error
```

## 1. API request model

`POST /v1/projects/{project_id}/checks`, multipart. Two new optional parts;
the two existing ones mean what they always meant.

| Part | Required | Meaning |
| --- | --- | --- |
| `candidate` | yes | The candidate ELF. Alone, it stands for the minimal program upgrade. With `change_spec`, it only satisfies the spec's artefact reference. |
| `expected_changes` | no | Unchanged. |
| `change_spec` | no | A ChangeSpec document (≤ 64 KiB). Authoritative. |
| `label` | no | Display name for a derived spec (≤ 120 chars, no control characters). Refused beside `change_spec`, whose own `metadata.label` is the place for it. |

Any other part, or any part sent twice, is `400`. Two candidates in one request
would otherwise leave "which bytes" to the order parts arrived in.

Refusals happen before a run exists and carry the CLI's exit code:

| Case | HTTP | `exit_code` |
| --- | --- | --- |
| spec malformed, unknown field/kind, non-canonical, stated id disagrees with fields | 400 | 2 |
| spec given without `candidate` | 400 | 2 |
| candidate bytes are not the spec's `candidate {sha256, len}` | 400 | 2 |
| spec targets another program; states ProgramData / authority / `replaces` the pinned bundle cannot prove | 409 | 4 |
| project disabled, no active bundle | 409 | none (unchanged: nothing was measured) |

## 2. Backward compatibility

- The multipart `candidate` + `expected_changes` request is unchanged and still
  returns `202`. `eplyx-submit.sh` and `.github/workflows/eplyx.yml` work
  without edits.
- `RunMetadata.candidate_sha256`, `AcceptedResponse` fields and run-list
  fields are all kept. `change` is added beside them.
- The candidate-only path derives exactly the spec `eplyx ci check
  --candidate` derives. The hosted `report.json` is still byte-identical to a
  local `eplyx ci check --format json` (`the_hosted_report_is_byte_identical_to_the_local_one`),
  now with the worker running through `ChangeInput::Spec` + a content store
  rather than `ChangeInput::Candidate` + a file.
- The CLI is untouched. The engine gained one public function,
  `ci::bind_change`, which reuses `check_change`'s own open/preflight path, and
  two read-only accessors on `ChangeSpec` (`candidate()`, `target_program_id()`).

## 3. Run-registry schema

`RunMetadata` gains one optional object. It holds the **index fields only**:

```json
"change": {
  "change_spec_id": "…64 hex…",
  "kind": "program_upgrade",
  "target_program_id": "…",
  "candidate_sha256": "…",
  "candidate_len": 96736,
  "label": null,
  "origin": "derived_from_candidate"      // or "submitted"
}
```

`origin` is provenance, never identity: the same proposal reaches the same ID
by either route. The complete spec is not copied into metadata; it lives once,
canonically, beside the run (§6).

New storage:

```text
runs/<run-id>/change_spec.json                       canonical document
runs/<run-id>/work/artifacts/programs/<sha256>        candidate, until terminal
projects/<project-id>/changes/<change-spec-id>/<run-id>   index marker
```

`GET /v1/projects/{id}/runs?change_spec_id=<id>` lists a project's analyses of
one proposal from that index. Two proposals with identical bytes against
different targets have different IDs and never share an index entry.

## 4. Target and bundle snapshot

The run already pinned its bundle at creation (Phase 10), and that is reused:
`bundle_sha256` and `bundle_id` are written before the `202` and never resolved
again. P1 binds the proposal to that same pinned bundle **at creation**:

- the derived spec's target is the pinned bundle's `program_id` (equal to the
  project's, as registration and activation enforce);
- `ci::bind_change(pinned bundle dir, spec)` runs the same open, preflight and
  `ChangeSpec::bind` the gate runs, so a proposal the bundle cannot honour is
  refused with exit 4 before it is queued.

The worker binds again against the same pinned bundle inside `check_change`.
The first bind is an early answer; the second is the one the verdict rests on.

## 5. Worker execution path

```text
load_run                                                    service fault → execution_error
metadata.change is None → no fallback                       execution_error
registry.load_change_spec (recompute id, match metadata)    execution_error
bundle_path(metadata.bundle_sha256)
spec.resolve(Store(work/artifacts))  (hash-on-read)         execution_error (missing / altered)
ci::check_change(bundle, ChangeInput::Spec { spec, Store })
   Ok(report)  → finish_run (identity check, §7)
   Err(error)  → failed + error.exit_code()  (e.g. 4 if the spec no longer binds)
```

There is no file whose name means "the candidate". A run's own inputs being
untrustworthy is this service failing to keep what it accepted, so it is an
`execution_error` — never a verdict about the candidate. A spec that no longer
binds to its pinned bundle is the engine's exit 4, a genuine preflight answer.

## 6. Canonical ChangeSpec persistence

`runs/<id>/change_spec.json` is `ChangeSpec::to_document()`: the spec with its
`change_spec_id` committed beside the fields, metadata (label, source) included,
candidate bytes never included. Every read (`Registry::load_change_spec`, used
by the worker and by `GET /v1/runs/{id}/change_spec.json`):

1. `ChangeSpec::parse` — refuses unknown fields and a stated id that disagrees
   with the fields;
2. requires the document to commit to an id at all;
3. requires every indexed field (`RunChange::of(spec)`) to equal
   `metadata.change`.

So an edit that forgets the id is refused at (1), and a consistent rewrite —
new fields, new matching id — is refused at (3): it is a valid spec, just not
the one the run was accepted for.

## 7. Change/report consistency invariant

```text
registry change_spec_id == stored ChangeSpec id == report.change.change_spec_id
```

Enforced in `Registry::finish_run`, the only place a run becomes terminal:
a `Reported` outcome whose `report.change` (id, kind, target, candidate) or
`report.candidate {sha256, len}` disagrees with `metadata.change` is recorded
as `execution_error` ("change identity check failed; no verdict recorded"),
and the report is not published. The frontend checks the same three values
(§9) and shows no verdict on disagreement. `eplyx-submit.sh` checks the
accepted id against the run and the report, and against the id a submitted
spec file commits to, and exits 70 otherwise.

## 8. Frontend primary flow

`/analyse` is now **Analyse a program upgrade**:

1. choose the project — it determines the target program;
2. upload the candidate `.so`;
3. a confirmation card appears: *Program upgrade · Target: \<project name\> ·
   SPoo1K…kuHy · Candidate: 60b7e1ac · program.so · 31.5 KB*. The fingerprint is
   SHA-256 computed in the browser; the change ID is not computed there;
4. optional name, optional expected changes, **Analyse change**.

After the `202`, the page compares the server's `change.candidate_sha256` and
`target_program_id` with what it prepared and refuses to follow a run that
disagrees. There is no field for schema, hashes, ProgramData, authority, a
change ID or a JSON document, and only "Program upgrade" is offered.

## 9. Report / change summary UI

Every run state (queued, running, preflight abort, execution error, completed)
opens with a **Proposed change** card:

```text
PROPOSED CHANGE                                  Program upgrade
Release 2.1
Target        Example Pool · SPoo1K…kuHy
Candidate     60b7e1ac   31.5 KB
Change ID     5c1f0e9a
Proposed effective   slot 400000000          (only when the spec states one)
Analysis is counterfactual: the candidate is replayed over validated
historical state, whenever the change would take effect.
```

Full identifiers are in **Proof & identity** (the former Provenance section):
full change ID, kind, full target, candidate hash and size, ProgramData,
`replaces`, upgrade authority, activation slot/time, spec schema, origin,
source, and an **Identity check** row stating whether run record, stored spec
and report name the same change. The existing evidence rows (bundle, corpus,
baseline, candidate, adapter) follow unchanged, as do coverage and limitations.
The metric row's candidate hash became *Cannot be declared*.

`report.change` is the authority for what was analysed. When it, the run's
`change` and the stored spec's id disagree, the page renders *This run's change
identity does not agree with itself* with the conflicting ids and no verdict.

Run history rows read *Release 2.1 → SPoo1K…kuHy* (or *Program upgrade →* when
unlabelled) with `change 5c1f0e9a · candidate 60b7e1ac` beneath. Rendering is
keyed by `kind` in `frontend/src/change.js`, which defines one kind.

## 10. Legacy runs

A run recorded before P1 has no `change` in its metadata and no stored spec.
It is not backfilled: the minimal spec needs the candidate length, which was
never recorded, so any id would be a guess. Such a run:

- reads normally (`change: null`) from `GET /v1/runs/{id}` and the history;
- answers `404` from `change_spec.json`, saying it predates change identity;
- renders as **Legacy run** with only its candidate fingerprint. A run from
  between C1 and P1 does carry the engine's `report.change`; the page shows that
  identity and says it came from the report.

## 11. Advanced explicit-spec API

The same endpoint with a `change_spec` part. The response and run are bound to
exactly that spec (`origin: "submitted"`), and:

- `GET /v1/runs/{id}/change_spec.json` returns the verified canonical document;
- `GET /v1/projects/{id}/runs?change_spec_id=…` finds every analysis of it;
- `scripts/eplyx-submit.sh --change-spec change.json` submits one from CI and
  fails (70) unless the server analysed exactly the id the file commits to.

`eplyx change program-upgrade … --out change.json` produces such a document.
This is the shape a governance binding needs — *the proposal being signed*
names a `change_spec_id`, and an existing analysis is looked up by it — without
anything governance-specific being built.

## 12. Tests

`server/tests/change_identity.rs` (new, 14 tests):

| # | Property | Test |
| ---: | --- | --- |
| 1 | candidate-only request still accepted and completes; old fields kept | `a_candidate_only_check_still_succeeds` |
| 2 | derived spec is deterministic and equals the CLI's; a label does not re-identify | `a_candidate_only_check_derives_the_cli_s_spec` |
| 3 | explicit spec (activation, label, source) analysed and served back exactly | `an_explicit_spec_is_analysed_exactly` |
| 4 | spec A + bytes B, spec without bytes, spec + label, duplicate candidate, forged id → refused, no run | `a_spec_for_one_candidate_never_executes_another` |
| 5 | spec for another program; unprovable ProgramData → 409 exit 4, no run | `a_spec_for_another_target_is_refused` |
| 6 | registry id == stored id == report id; lookup by id; a real report about another change is recorded as `execution_error` | `registry_spec_and_report_name_one_change` |
| 7 | same bytes, two targets → two ids | `the_same_bytes_against_two_targets_are_two_changes` |
| 8 | bundle activated while queued changes neither bundle nor change | `the_worker_cannot_use_a_bundle_activated_later` |
| 9 | missing / altered candidate artefact fails closed | `a_missing_candidate_fails_closed`, `an_altered_candidate_fails_closed` |
| 10 | tampered stored spec; consistently rewritten spec → refused on read and at execution | `a_tampered_stored_spec_fails_closed`, `a_consistently_rewritten_spec_is_still_not_the_accepted_one` |
| — | indexed target no longer binds to pinned bundle → exit 4, nothing executes | `a_target_that_no_longer_matches_the_pinned_bundle_fails_closed` |
| 11 | pre-P1 run readable, `change: null`, spec 404, report served | `a_legacy_run_remains_readable` |

Two mutations were run against the registry guards (disabling the
`finish_run` identity check; disabling the stored-spec comparison); each is
caught by a test that fails for that reason alone.

`frontend/change.test.mjs` (new, 16 checks, in `pnpm check:frontend`): the
card (12), the complete technical identity (13), the analyse page asking for
nothing technical and offering only program upgrade (14), the browser
fingerprint equalling Node's SHA-256, a mismatched accepted change not being
followed, the three-way mismatch showing no verdict, legacy rendering, history
lines, activation wording, and no internal type names in the ordinary view.

Updated: `uploaded_inputs_outlive_the_request_and_are_cleared_at_the_end`
(candidate is now staged by content hash) and `scripts/async-demo.sh`'s
matching check.

## 13. Frozen backend regressions

The engine's replay, bundle, review and report code paths are unchanged.
`check_change` was refactored only to share its bundle-opening code with
`bind_change`. The C1 controls (`engine/tests/change_spec.rs`: pilot baseline
exit 0 and `7be26a66…` minus `change`, stake-pool regression exit 1 with the
same fingerprints, U14 Drift / U12 Orca exit 0, U11.2 / U13.3 exit 2
`no_semantic_coverage`, frozen spec id `b5a894cd…`) pass unchanged, as does
the hosted-equals-local byte identity test.

## Known gaps, deliberately left

- ~~No persistent server-side artefact store.~~ Closed by
  [Phase P2](phase-p2-durable-hosted-analyses.md): candidates are durable,
  immutable and content-addressed, and a spec can name one the project already
  supplied.
- The index is per project. A change ID is global, but a lookup across projects
  would cross the project-token boundary and is not offered.
- The frontend has no explicit-spec upload. It is an API and CI surface on
  purpose.
- `compare --corpus` and the synthetic `compare` still name candidates by label
  or path (unchanged from C1).

## Recommended next step

**A durable, content-addressed artefact store and a durable run queue**, behind
the existing `Storage` abstraction. The two are one problem: today a restart
must fail every queued run because the only copy of its candidate lives in the
run's work directory. With `programs/<sha256>` retained (subject to a retention
policy) the stored spec alone is enough to re-execute a run, restart recovery
can resume rather than fail, a governance flow can submit a spec and point at an
artefact already uploaded, and the frontend could re-run a past proposal against
a newly activated bundle — the "same change, new baseline" comparison teams will
ask for next. It adds no change kind and no replay semantics.
