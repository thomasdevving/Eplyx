# T9 — Cloud identity and hosted analysis

MAIN now serves workspace identity, optional CLI sync and durable hosted analyses
from one `eplyx-server`. The approved T0 split is applied: Postgres contains users,
sessions, device codes, membership, tokens and workspace/project mappings. MAIN's
filesystem registry and content-addressed store remain the only authority for
projects, inputs, runs, reports, searches, counterexamples and reproductions.

## Identity and durable storage

Browser sessions use hashed random tokens, HttpOnly/SameSite cookies and same-origin
mutation checks. Passwords use Argon2id. Device codes expire, poll at a bounded
interval and issue a token once after browser approval. Project credentials use
MAIN's `eplyx_proj_` prefix and can submit or sync only their project; they cannot
activate a baseline or administer members. Workspace owners manage membership and
project credentials. An operator explicitly assigns an existing registry project
to a workspace; names and old project credentials never imply ownership.

New project creation writes a filesystem intent before creating the registry entry
and Postgres mapping. Recovery completes a pending intent only with current
membership. Completed intents never restore revoked authorization. Access requires
both the canonical project and the current authorization mapping. No analytical
bytes are stored in the database. Existing operator/project API contracts remain.

The CLI adds `login`, `logout`, `link` and `sync`. Credential files are private,
atomic, bounded and symlink-refusing. Sync uploads exact small analytical documents
with their digests, never source, programs, captures, configuration or environment.
The scanner rejects provider URLs, machine paths and credentials, including escaped
JSON strings and keys. It does not sanitize an artifact into a different artifact.
Reproduction error text that STA sanitized is therefore refused when leaky. This
implements the user's explicit exact-byte privacy rule. Corrupt selected
counterexamples/reproductions fail the sync plan; they are not silently omitted.
Idempotent content is accepted, changed content conflicts, and search appends
without changing a run's immutable core. Sync never executes a result.

## Offline hosted jobs

Uploaded migration and lifecycle inputs are verified and retained before a durable
run is queued. An analytical project needs no upgrade baseline. Program upgrades
keep the same CI evaluator, report bytes, exit codes and baseline activation rules.
All worker kinds execute in a child with an empty environment. The child loads no
identity/provider configuration and verifies its input identities again. Limits
bound concurrency, queue depth, input bytes, execution duration and output bytes.
Timeouts kill and reap the child; missing binaries and nonzero worker exits do not
produce successful evidence.

A completed projection is saved before terminal run metadata. Recovery can finish
that transition without re-execution. Interrupted work otherwise follows MAIN's
bounded durable retry model, preserving inputs and attempts. This deliberately
replaces STA Node's restart-as-error model, as required by the MAIN P2 architecture.
Reports remain exact engine output; display projections use the same engine code
as the local dashboard. Lifecycle/current completion has no invented deployment
gate. Migration policy failures retain their actual gate outcome.

A synthetic recipe using captured token programs must upload
`pinned_program_capture` with the exact T3 capture digest. The dependency is retained
in CAS and staged into the child, so execution needs no source checkout. Bundled
recipes and captured worlds reject that extra dependency. Docker includes the
embedded frontend, curl and the system CA store, and excludes ignored captures
from its context. No Docker executable was available for an image build; the
native staged-worker tests exercise the portable input contract.

## Current-state acquisition and execution

A dedicated operator-configured observation provider is used only by the parent
service. Six read-only methods are allowlisted. A serial acquisition permit,
request/response limits, 15-second calls and a 120-second acquisition budget bound
work. Normal requests permit 32 calls; stress permits 96. No request can supply
providers, commands, code, instructions, metas, signatures, paths or claimed proof.
The offline child receives only stored bytes. Live-provider checks were not run.

The first-party catalogue parses bounded saved Flight JSON without executing
scripts. Versions preserve exact captured bytes and assertions, including the original saved capture envelope on operator import retries. Names are not
issuer verification; arbitrary canonical mints have unconfirmed association.
Owner scope is exact to the selected mint and public owner. One focused account
controls each check, quantities remain decimal strings, and unsupported or zero
balances carry explicit availability reasons. Refresh creates a new observation
and starts untested. Failed acquisition never substitutes historical evidence.

Transfer and Meteora exit rebuild typed messages and reconcile actual VM execution.
Candidate checks resolve MAIN's registered token-migration program and a one-account
ChangeSpec. The new `ExactRaw` amount policy binds a positive selected amount;
existing `full_balance` wire identities and frozen reference results are unchanged.
Source drift is Indeterminate, never an adjusted amount or a substituted account.
A coherent final account set, Clock, loaders and dependencies bind actual execution.
Proven means execution and exact reconciliation; official transition, possession
and authorization remain unestablished.

User-proposed scenarios retain proposed dates and an independent successor
observation. Saved path and candidate checks are replayed against their exact
parent/focus/hash before use. The 17 frozen current-preflight tests are ported.
Pre-event, mobility, candidate execution and full transition remain separate;
deadline passage alone cannot imply Blocked, and candidate success cannot establish
issuer eligibility or population readiness.

Stress requires a completed candidate run belonging to the same project and
observation. It derives a full-balance migration proposal, acquires a new bounded
population, freezes at most ten cases and executes each against its exact coherent
final state. Discovery, population, plan, candidate and final captures are bound.
Coverage counts do not become population readiness, state-shape coverage is not
entity coverage, and non-wallet authority resolution never grants a signer.
The browser chooses only a candidate run, not a budget, program or frozen cases.

## Frontend and verification

The hosted shell uses MAIN's shared dashboard, glossary, fonts and mode setting.
It exposes workspace/member/token/device pages and current-state workflows. One
history reaches every kind. Changing focus clears prior inline results and late
responses cannot attach a different account’s proof. Program upgrades reuse MAIN's report/impact/governance
renderer with scoped styles, while migration, lifecycle and current results reuse
the dashboard projections. Existing public routes remain intact.

[The source mapping](phase-t9-test-mapping.json) retains each source test name and
hash, target coverage and explicit architectural adaptations. Source live-provider
browser cases are intentionally not run. [Verification](phase-t9-verification.json)
records actual commands, failures and retries, full Rust/frontend checks, browser
results, deterministic reference checks and the final source/delta audit. New
capture dependencies are imported as exact ignored files with digest/size manifests;
no raw capture or new SBF bytes are committed. STA and its historical evidence are
unchanged. No deployment, push or live-cluster submission occurred.
