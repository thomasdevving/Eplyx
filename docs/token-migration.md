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

## Prepare one migration in the browser

An authenticated cloud project exposes **Analyse → Token migration** at
`/p/{project_id}/analyse/migration` when the project capability response says
`token_migration.can_submit = true`. If it is unavailable, the page shows the
server's reason and action and keeps the form disabled. The capability is only
pre-submission guidance; the POST checks current project state again.

The first browser workflow intentionally supports one prepared shape: a
full-balance, whole-token UI ratio; owner authorization for wallet and multisig
owners; source burn; proposed reserve transfer; program-derived migration
authority; relayer fee payer; and the block-only gate policy. The form exposes
source and replacement mint/program/decimals, exact ratio integers, rounding,
an optional source-basis-point fee, minimum raw output, proposed raw reserve,
UTC effective/deadline times, and the mechanism program. The exact candidate
SBF, an existing migration `state.json`, and the fixture or captured-world JSON
named by that state descriptor are imported as prepared files. A state recipe
that uses `pinnedMainnetCapture` remains a CLI/API workflow because it also needs
the separately retained pinned program capture.

Raw amounts and ratio parts remain canonical decimal strings; the browser does
not pass them through JavaScript `Number`. Effective and deadline fields are
explicitly UTC and the preview shows the corresponding Unix seconds. Browser
checks are structural only: required values, canonical u64 syntax, public-key
syntax, positive ratio parts, bounded fee/decimals and deadline ordering. The
engine still decides whether proposal terms, state, reserve, authorities and
mechanism validate and what the rehearsal establishes.

Before submission, Overview shows the source-to-replacement summary, exact
ratio, canonical timing, fee/rounding, reserve and authority declarations, plus
which optional declarations are omitted. Technical mode shows the exact
ChangeSpec bytes sent in the multipart request and SHA-256 identities for the
prepared files. “Ready to submit for analysis” means only that the browser's
structural checks passed; it is not a safe, valid, approved or verified
migration verdict. Submission uses the existing hosted check route and then
opens the existing asynchronous run page.

The browser does not expose bounded search settings, stress budgets,
reproduction IDs, deployment-gate policy selection, unsigned plans, arbitrary
account capture, provider configuration or execution/signing. The CLI remains
the interface for those advanced workflows. There is no generic proposal JSON
editor in this phase; the JSON file inputs are the already-prepared state
descriptor and its named state artifact.

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

## One selected ordering case

`eplyx migration order --run RUN_ID --source-a SOURCE_A --source-b SOURCE_B --out
./order-case` compares both solo controls and both transaction orders under one
unchanged migration proposal and fixed world Clock. Shared reserve-transfer
funding is the supported shape. `eplyx migration reproduce-order ./order-case`
verifies and re-executes the portable evidence completely offline. See the
[order-case contract](migration-order-case.md) for account closure, known absence,
identities, reproduction and the precise limits of the ordering finding.

## Hosted order analysis

A completed hosted migration run with a verified retained package can expose
**Order Analysis**. Choose two retained eligible sources, preview the bounded
comparison, then submit an asynchronous derived child run. The parent owns the
ChangeSpec, candidate/dependencies, world and fixed Clock/runtime; the request
supplies only Source A and Source B. No state refresh occurs, and the ordinary
migration history remains proposal history with children listed on the parent.

The exact local Phase 7B engine evaluates A alone, B alone, A → B and B → A. The
web UI presents its comparison and Technical evidence without another planner or
verdict implementation. Ordering effects are completed analytical results, not
worker errors or deployment authorization. Portable evidence can be downloaded
and reproduced with `eplyx migration reproduce-order`. See
[migration-order-case.md](migration-order-case.md) for eligibility, endpoints,
legacy runtime verification, typed failures and bounded limitations.
