# T1 — Kind-aware ChangeSpec interfaces

T1 implements the first code phase of the [approved T0 ADR](phase-t0-stock-transition-integration.md).
Only `program_upgrade` is registered. This phase changes Rust interfaces so an
asset change can arrive without inventing a program target or executable.

## Representation

The existing `Change::ProgramUpgrade` fields, schema, compact identity preimage,
optional-field omission and metadata exclusion are unchanged. `candidate()` and
`target_program_id()` are optional, `target()` is typed, and
`as_program_upgrade()` narrows upgrade-only consumers. `with_delivery()` and
baseline binding are fallible kind-specific operations. Governance narrows the
spec before provider reads. `resolve()` remains the sole constructor of the
bytes allowed to execute.

Report bindings contain a flattened `BoundChange`; server run indexes contain
a flattened `IndexedChange`. Both enums currently have one variant. Their old
JSON field order and values remain unchanged, including optional Squads delivery.
The worker still verifies the stored spec against the CAS reference and bytes,
and the registry still checks the report against the accepted run identity,
target, candidate hash, candidate length and delivery.

The frozen C1 identity remains
`b5a894cdbec6251f73b4224a294579fe1af9e316232949fa92da852468900bf3`.
New frozen-wire tests round-trip pre-T1 report-binding and run-index JSON exactly.
Existing governance fixture, ChangeSpec, hosted/local report equality, exit-code,
artifact substitution and durable recovery assertions retain their meaning.

## Verification

The appended `phase-t1-verification.json` records commands, exits and log digests.
The pinned toolchain is the T0 setup: Rust stable, MAIN SBF platform-tools v1.54
with architecture v0, Node 22.23.1 and pnpm 11.24.0. The baseline executable remains
`b307faa27e09fc74fee9e8087263f249bef565a6998f49fd5e562d2932bf1641`.
No fixtures, capture bytes, historical evidence or STA source are changed.
Live-provider checks are intentionally not run. Migration, lifecycle, dashboard
and Postgres checks are pending their implementation phases.
