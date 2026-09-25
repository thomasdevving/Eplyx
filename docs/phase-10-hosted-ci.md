# Phase 10 — Pilot-ready CI

Eplyx runs candidate Solana program upgrades against a pinned corpus of
validated historical production interactions and fails CI on undeclared
economic changes.

Phase 10 turns the engine into something a protocol team can wire into their
pipeline. It adds no analysis: every judgement below — severity, review status,
bounds, precedence, coverage — is made by the same engine the local CLI uses.

## Corpus construction is not CI. Corpus consumption is CI.

This is the architectural decision the whole phase rests on.

```text
PERIODIC / ADMINISTRATIVE            PER PULL REQUEST

mainnet                              candidate.so
  ↓ historical acquisition             + expected-changes.toml
validated records                      + the project's active bundle
  ↓ selection                                ↓
CI bundle                            eplyx ci check
  ↓ an operator activates it                 ↓
project points at it                 pass / fail + report
```

Building a validated corpus needs an archive endpoint, a several-hundred-block
scan and a funnel that discards most of what it sees. On the path of a pull
request that would mean credentials a protocol team should not have to hold,
minutes per run, and — worst — a moving target. A pull request that was green
last week and red today because the corpus changed underneath it teaches a team
to stop reading the check.

So a bundle is built rarely, reviewed when it changes, and activated
deliberately. A bundle is *consumed* on every pull request, offline, with
nothing configured.

## Candidate code is built outside Eplyx

A security boundary, not a convenience. Eplyx never clones a repository, never
runs a build script, never compiles uploaded source and never executes a
Dockerfile. GitHub Actions builds the `.so` in the customer's own runner and
uploads only those bytes, which are executed solely inside the replay VM the
engine already sandboxes.

## The three inputs

| | Owned by | Why |
|---|---|---|
| `candidate.so` | the pull request | it is the thing under test |
| `.eplyx/expected-changes.toml` | the customer's repository | intentional changes must be version controlled, visible in the diff, and attributable to the commit that introduces them |
| the active bundle | Eplyx | megabytes of historical evidence, and a client must not be able to pick a more forgiving baseline for its own check |

Every report names the bundle explicitly — `bundle_sha256`, `corpus_sha256`,
`baseline_sha256`, the production slot range and the record count — so a result
is reproducible without the customer storing the evidence.

## Exit codes

```text
0  passed
1  an undeclared change, one larger than declared, or one that cannot be declared
2  malformed configuration, fidelity or internal analysis failure,
   including a bundle whose adapter produced no semantic coverage at all
3  a stale declaration
4  bundle or baseline incompatibility
5  a declaration this corpus cannot judge
```

## Three layers of evidence, one declarable

```text
named semantic findings     declarable in expected-changes.toml
decoded economic changes    the adapter decoded it but has not promoted it
structural differences      bytes, balances, outcome, invocation shape
```

Only the first can be named by an expectation. The other two still fail the
gate, as `undeclarable_change`. That is deliberate and conservative: the
alternative is reporting a change as absent because the vocabulary could not
name it, which is what an earlier version of this gate did — it consumed only
the named layer, so a protocol with no semantic surface produced empty coverage,
empty findings and a green check over a change the replay had detected.

A consequence worth stating plainly: a legitimate upgrade that moves a quantity
no adapter has promoted **cannot currently be made to pass**. The subject has to
be promoted deliberately first. Failing in that direction is the safe error.

Empty semantic coverage is never a pass. An adapter with no emission surface
yields `no_semantic_coverage` and exit 2, because zero findings there means "we
did not look", not "nothing changed".

Three different actions for a team, so three different codes: 1 is "the
candidate did something you did not approve", 3 is "your approval file contains
something that no longer happens", and 5 is "Eplyx cannot prove whether your
approval still applies". Codes 2 and 4 are preflight aborts and produce no
report.

When several failures coexist the report contains all of them; the exit code is
a deterministic summary by precedence, coverage uncertainty first. Precedence
never removes a failure from the report.

**HTTP status and the Eplyx gate are separate axes.** A candidate that fails
policy is `HTTP 200` with `exit_code: 1` — the request succeeded, and the answer
is that the upgrade should not ship. A preflight abort returns an HTTP error and
still carries its Eplyx code in the body, so a workflow propagates 2 or 4 rather
than a generic failure. A transport fault is never reported as a gate result.

## Severity is descriptive, not policy

The gate runs on review status alone. Every non-`EXPECTED` status fails whatever
its severity, and an `EXPECTED` one passes whatever its severity:

```text
CRITICAL / EXPECTED               may pass — declared, and inside its bounds
WARNING  / UNEXPECTED             fails   — nothing declares it
CRITICAL / EXPECTED_BUT_EXCEEDED  fails   — larger than declared
```

`CRITICAL / EXPECTED` is never rewritten to `INFO`. The change is still
critical; what the declaration adds is that somebody signed for it. Keeping
severity out of the contract is also what lets a better severity model land
later without invalidating a single existing expectation file — the current
mapping is coarse, and rates a 1 bp and a 3,000 bp change the same.

## The hosted API

```text
POST /v1/projects/{project_id}/checks     multipart: candidate, expected_changes,
                                          change_spec?, label?
GET  /v1/runs/{run_id}                    lifecycle state, in every state
GET  /v1/runs/{run_id}/report.json        the canonical report, as stored
GET  /v1/runs/{run_id}/report.md
GET  /v1/runs/{run_id}/change_spec.json   the proposal the run evaluates, re-verified
GET  /health                              process alive
GET  /ready                               persistent storage usable
```

Authentication is a per-project bearer token. Only a salted SHA-256 verifier is
stored, so a leaked data volume does not hand over the ability to run checks. A
token authenticates exactly one project; used on another project's URL it fails
like any other bad token. Reports are served as stored — fetching one never
re-runs an analysis.

Since Phase P1 every run is an analysis of one named proposal: candidate
bytes alone stand for the minimal program upgrade of the pinned bundle's
program, an explicit `change_spec` part is authoritative, and either is bound
to the pinned bundle before the `202`. The run, its stored spec and its report
must name the same `change_spec_id`. See
[`phase-p1-productized-changespec.md`](phase-p1-productized-changespec.md).

Uploaded candidates are ephemeral. They are staged in the run's own work
directory and removed once the run is terminal, whatever the outcome; what
survives is the SHA-256, the report and the run metadata.

### A check outlives the request that created it

`POST /checks` answers `202 Accepted` with a run id and `status: "queued"`. It
does not wait for the analysis, and it never returns a verdict — inventing one
to fill the shape would be a lie a caller could act on.

The run is persisted before any worker is started, so a `202` is a promise that
`GET /v1/runs/{run_id}` already resolves. That is the whole point: a closed tab,
a proxy timeout or an aborted fetch cannot destroy an accepted analysis, and a
browser that reloads recovers the run from the server rather than from its own
memory. The worker is a detached task, not something scoped to the response.

```text
queued  → running → passed | failed
                  → execution_error
```

`queued` means the run exists and is waiting for execution capacity. `running`
means it holds a permit and the engine is going. No terminal state ever returns
to `running`, and claiming a queued run is a compare-and-set, so one run cannot
execute twice.

**`failed` and `execution_error` are not the same failure.** `failed` means the
engine reached a verdict and the verdict is no: a real gate result with a real
exit code, which is the product working. `execution_error` means this service
could not obtain a verdict at all — a disk that filled, a task that panicked, a
restart — so there is no gate result and none is shown. Collapsing them would
tell a team their upgrade was rejected when in fact a volume went away.

A preflight abort stays on the first branch. Exit codes 2 and 4 produce no
report by design, so such a run is `failed`, carries its exit code, and reports
`report_available: false`. Asking for its report is a `409` that names the code
rather than an empty report object.

Reports are refused with `409` until one exists. The engine exposes no durable
per-record progress, so neither the API nor the page invents any: queued,
running and completed is the whole vocabulary.

### Onboarding, and who may do what

A protocol team goes from a program to a running check through the API the
console uses: create a project, upload a bundle, activate it, issue a token.

```text
POST /v1/projects                                    create
GET  /v1/projects                                    list
GET  /v1/projects/{id}                               one, with cheap run stats
POST /v1/projects/{id}/tokens                        issue, shown once
GET  /v1/projects/{id}/tokens                        ids and labels, never secrets
DEL  /v1/projects/{id}/tokens/{token_id}             revoke
POST /v1/projects/{id}/bundles                       register, verified on arrival
GET  /v1/projects/{id}/bundles                       history, with the active one marked
POST /v1/projects/{id}/bundles/{bundle_id}/activate  move the pointer
GET  /v1/projects/{id}/runs                          newest first, cursor paged;
                                                     ?change_spec_id= for one proposal
GET  /v1/adapters                                    what this build speaks
```

A project id is opaque and minted (`proj_01M2RN…`), never a name: two teams may
both call theirs Lending, a rename is not a new project, and a name that became
a path segment would put the caller in charge of where bytes land. Bundle, token
and run ids are the same shape, and lead with their own minting time, so history
is a directory listing and a cursor is "everything after this id".

**Two credentials, no user model.** A *project token* is a CI secret: it submits
checks for its own project and reads that project's runs and reports. An
*operator token* is the console's, configured on the server as
`EPLYX_OPERATOR_TOKEN` rather than issued by it; it creates projects, issues and
revokes their tokens, and registers and activates bundles.

That split is the Phase 10 rule applied to credentials: a token that lives in a
pull request must not be able to change what future pull requests are measured
against. It is also why no endpoint is public — listing projects without a
credential would hand an unauthenticated caller every program this service
watches. A foreign project token is answered `401` uniformly, because it is only
ever matched against the owning project's tokens: the answer is the same whether
the resource exists, belongs to someone else, or never existed.

**Status is the project's, never the run's.** `setup` has no active bundle and
accepts no checks; `ready` has one; `disabled` is an operator's decision and is
never inferred away by activating a bundle. A check refused for any of those is
a hosted configuration answer — `409`, with no exit code — because nothing was
measured, so there is no verdict about the candidate to report.

**A bundle must say what it is.** Registration and activation both check the
bundle's own verified manifest against the project: same program, same declared
adapter, an adapter this build still speaks, and the semantic schema version it
was named under. Reading both sides matters. Checking only the engine's version
left a program with no compiled adapter unable to onboard at all, while a bundle
declaring `none@0` for such a program is telling the exact truth — every check
against it reports `no_semantic_coverage` and fails, which is the engine saying
it did not look rather than saying nothing is wrong. The console says so too,
before a team reads that red gate as a finding.

**A run pins its bundle at creation.** The active bundle is resolved once, when
the check is accepted, and its id and hash are written to the run before the
`202`. The worker never resolves it again. Activating a new bundle while a run
is queued belongs to the next run: otherwise a result would quietly describe a
comparison nobody asked for. Activation moves a pointer and destroys nothing, so
the bundle an old run names is still there to re-read.

### What a restart costs

The task queue is in-process. A restart drops whatever it was holding, and runs
left `queued` or `running` have no worker behind them any more. On startup they
are resolved as `execution_error` with `"Run interrupted by server restart;
resubmit the check."`, and their inputs are cleared.

This is a real limitation, stated rather than hidden. Resuming interrupted runs
needs a durable queue, which a pilot does not need and which the single-volume
deployment cannot honestly provide. What is not acceptable is leaving a run
`running` forever for a client to poll.

`scripts/async-demo.sh <bundle-dir>` drives a real server over a real socket
through all of it: acceptance without execution, a queued run that has provably
not started, refusal of its report, restart recovery, completion, and the
canonical report refetched byte-identical.

### The hosted report is the local report

For the same bundle, candidate and expectations, `report.json` from the API is
byte-for-byte what `eplyx ci check --format json` produces locally. The HTTP
layer is not permitted to reinterpret severity, review status, expectations,
bounds, failure precedence or coverage. Hosted metadata — `run_id`,
`created_at_unix_seconds` — lives beside the canonical report, never inside it,
so it cannot reach a determinism hash.

`scripts/hosted-demo.sh <bundle-dir>` proves this end to end, along with the
five gate outcomes, authorization isolation and retention, with every RPC
variable explicitly unset.

## Bundle lifecycle

A bundle is immutable and content addressed. Activation is a pointer change, and
a freshly installed bundle is never activated automatically.

```text
Bundle A (baseline = V1)  ← pull requests test V2 against this
        ↓ V2 is deployed to mainnet
Bundle B (baseline = V2)  ← built, reviewed, then activated
```

Bundle A is never edited, so runs recorded against it stay reproducible. Phase 10
does not automate detecting that a deployment happened; preparing and activating
the successor bundle is an operator step.

Activation refuses a bundle that fails verification, is for another program, or
was built under a different adapter version. That last check is what stops a
pull request going green last week and red today because the interpretation
moved underneath it.

## Wording

A passing run says:

> No unexpected economic changes were detected across the tested validated
> historical corpus.

or, where declarations matched:

> All observed changes matched the declared expectations and remained within
> their configured bounds.

It does not say *safe*, *fully verified*, *complete production coverage* or *all
users unaffected*. The bundle's coverage limitations are carried into every
report, including passing ones — a green result must never hide them.
