# Dashboard

Run `eplyx dashboard --no-open` inside a project to browse `.eplyx/` on loopback.
The CLI prints the local address. `--port` chooses a port. No Node runtime or
network fonts are required; assets and licensed fonts are embedded in the binary.
The local service is read-only and cannot start an analysis or upload a record.

Migration results keep eight questions in order: proposal, affected accounts,
sequential rehearsal, stress, search, invariants, gate, and evidence/replay.
Lifecycle and current-state pages display their own analytical scope without an
invented migration gate. Provenance distinguishes captured state, synthetic
fixtures and derived variants. Counterexample pages preserve saved witness and
reproduction history. Comparison never equates an absent finding with a fix.

The Overview/Technical switch persists across MAIN pages. Technical mode exposes
hashes, exact quantities and execution details; it does not change an engine result.
Local artifact links expose only a fixed verified allowlist. Incomplete or corrupt
records are presented as unavailable, never as a successful result.

Hosted project histories use the same analytical projections and label their
source as hosted, local or CI. A program-upgrade run retains MAIN's report, impact
view and, for a bound ChangeSpec, a paginated Squads governance trail. Synced records contain exact small documents; their
programs and captures stay at the source machine. Hosted jobs retain their inputs
and results in the service's immutable store. Merely viewing either kind performs
no observation or execution.

See [cloud](cloud.md) for workspace access and sync, [token migration](token-migration.md)
for execution commands, and [T8](phase-t8-dashboard.md) for presentation provenance.

## Hosted Token-2022 Parameter Change

An authenticated retained hosted current-transfer run exposes **Parameter Change**
when the server verifies eligibility for the active newer Token-2022 fee-bps field.
The guided page is `/p/{project}/runs/{run}/parameter-change`; there is no blank
configuration-account picker. Current rate, mint, cap, schedule/captured epochs and
exact transfer come from retained evidence and stay read-only. Only proposed bps
(`0..=10000`) can be edited. Preview exposes the exact ChangeSpec in Technical mode
without predicting token outputs. Mode switches retain the input; errors preserve it.

The existing hosted submission and run view are reused. Queued/running jobs update
in that view and remain recoverable on refresh. Results distinguish reconciled
consequence, no observed consequence for this transfer, evidence mismatch, pending
schedule, unsupported mutation/action, rejection, unavailable execution and failed
reconciliation. Program bytes stay unchanged. None of these views establishes fee
authority possession, `SetTransferFee` execution or on-chain activation. See the
[parameter contract](protocol-parameter-change.md) for exact scope and API fields.


Bound upgrade review separates **Analysis** from the **Squads governance trail**.
The timeline retains proposal binding/recheck outcomes, host recording times and
chain observation slots; each G2 observation names the exact G1 binding it used.
“More governance observations” retrieves another retained page without triggering
a chain read. “More analysis runs” pages linked analyses of this same bound change.
Expandable Technical evidence exposes full IDs, proposal status, message hashes,
execution and deployment coordinates, and prefix/padding comparison details.

A dated G1 match stays a historical match, not a current claim. `not_executed` says
execution had not been established at that observation. Execution without
attributable deployed bytes stays `unverifiable`; supersession preserves the
earlier execution. Analytical verdicts never inherit a governance outcome.
Legacy G2 time is explicitly unavailable. See the
[governance review contract](governance-review-trail-audit.md#10-implemented-review-layer).

## Hosted migration order review

Authenticated hosted migration run pages add a contained **Order analyses**
section. The server determines parent eligibility; unavailable parents show a
factual reason. Eligible parents expose **Order Analysis**, with two retained
source pickers and a preview of the unchanged proposal, starting world,
Clock/runtime and four scenarios. Changing either source invalidates the preview.

Submission navigates to a child occurrence that follows queued/running status.
The result shows A alone, B alone, A → B and B → A with intermediate/final state
links expressed as identities, observed reserve changes, reconciliation and
failure signatures. It renders the engine's order-effect/no-effect/NotEstablished
classification. Overview and Technical use the same engine result; Technical
adds identities and closure/known-absence evidence. Raw bytes remain in the
project-authenticated portable artifact download, reproducible by the CLI.

The parent lists its child occurrences; each child links to the parent. Derived
analyses are excluded from ordinary migration-proposal history. The UI states
that no refresh occurs and a newer state requires a new migration run. Evidence
and handoff failures show their typed limitation; internal worker failures carry
no analytical conclusion. See [migration-order-case.md](migration-order-case.md).
