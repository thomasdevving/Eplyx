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
POST /v1/projects/{project_id}/checks     multipart: candidate, expected_changes
GET  /v1/runs/{run_id}
GET  /v1/runs/{run_id}/report.json        the canonical report, as stored
GET  /v1/runs/{run_id}/report.md
GET  /health                              process alive
GET  /ready                               persistent storage usable
```

Authentication is a per-project bearer token. Only a salted SHA-256 verifier is
stored, so a leaked data volume does not hand over the ability to run checks. A
token authenticates exactly one project; used on another project's URL it fails
like any other bad token. Reports are served as stored — fetching one never
re-runs an analysis.

Uploaded candidates are ephemeral. They are written to a temporary directory
removed on both success and failure; what survives is the SHA-256, the report
and the run metadata.

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
