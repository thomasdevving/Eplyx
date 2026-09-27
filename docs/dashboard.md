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
view and governance card. Synced records contain exact small documents; their
programs and captures stay at the source machine. Hosted jobs retain their inputs
and results in the service's immutable store. Merely viewing either kind performs
no observation or execution.

See [cloud](cloud.md) for workspace access and sync, [token migration](token-migration.md)
for execution commands, and [T8](phase-t8-dashboard.md) for presentation provenance.
