# Token migration

Eplyx evaluates a declared token migration against retained state and actual SBF
execution. A proposal, a state observation and a deployment gate are separate
objects. This analysis does not authorize a transaction or establish holder keys,
issuer eligibility or legal entitlement.

## Start with an offline example

Build the programs with `make programs` and the CLI with
`cargo build -p eplyx-engine`. The executable is `target/debug/eplyx`; use that path or add its directory to `PATH`. Use a copy of
`examples/migrations/minimal` as a disposable project, place
`artifacts/eplyx_token_migration.so` at its `target/deploy/migration.so`, then run:

```sh
eplyx doctor
eplyx migration analyse
eplyx runs --json
eplyx migration search --run RUN_ID
eplyx migration gate --run RUN_ID --policy strict
eplyx migration plan --run RUN_ID --out unsigned.json
eplyx dashboard --no-open
```

Replace `RUN_ID` with the saved identifier. A new output path must not already
exist. The minimal example deliberately contains required holder/authority gaps;
its reference stress result is **20/20 cases behaved as specified**, including
nine expected rejections, not twenty successful migrations. The strict gate can
therefore fail even when the reference mechanism behaves correctly.

The captured token-program dependency is not in Git because the original capture
contains provider-origin fields. Import its exact bytes with
`scripts/import-migration-fixtures.py --sta STA_CHECKOUT`, using the pinned archive
identified in [ARCHIVE.md](../ARCHIVE.md). The importer reads a fixed allowlist,
checks sizes/digests and never reads provider configuration. Missing fixtures fail
explicitly. A fixture recipe using LiteSVM's bundled programs needs no capture,
but it is a distinct declared program source and must not replace a pinned test.

## Proposal and state

`eplyx init --migration --fixture` creates editable project templates.
`eplyx change token-migration --spec migration.json --mechanism migration.so
--out change.json` produces MAIN's ChangeSpec. Its identity covers migration terms,
activation and executable identity. Metadata labels do not affect identity.
`state.json` separately binds a synthetic recipe, captured world or bounded
acquisition intent. Runs retain resolved code, inputs and exact report bytes in
`.eplyx/`; changing them invalidates replay.

The migration includes source and replacement mint, ratio, rounding, fees,
eligibility, authorities, reserve and time-window terms. Amounts are canonical
integer strings. Full-balance population rehearsal and stress remain distinct from
a one-account `ExactRaw` candidate check. Proposed reserves and authorities remain
declarations until their relevant evidence is established.

## Read the result

The dashboard presents the proposal, affected accounts, sequential rehearsal,
stress, search, invariants, deployment gate and evidence in that order. Captured,
synthetic, proposed and derived provenance remain visible. Sequential reserve
consumption is not interchangeable with independently executed stress cases.
Source drift or an incoherent final capture is Indeterminate; Eplyx never changes
the amount or substitutes a peer to make a case execute.

A search finding is a bounded counterexample. An empty search is not universal
proof. `eplyx migration reproduce CX_ID` replays the saved witness and appends its
result without overwriting history. Compare only calls a finding resolved when
search conditions and an exact retest establish that conclusion.

Exit 0 means the command completed or the selected gate passed. Known gate
violations use 1, invalid input/processing errors use 2, and strict evidence gaps
use 5. The full mapping and compatibility limits are in
[T5](phase-t5-migration-cli-local-store.md). See [the frozen T3 comparison](phase-t3-migration-reference.json)
for all 14 before/after cases and [T4](phase-t4-current-migration-guarantees.md) for
coherence, authority resolution, final-state rebinding and observed search waves.
