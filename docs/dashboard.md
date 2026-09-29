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
