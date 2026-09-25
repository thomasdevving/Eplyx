# Phase P3 — human-readable impact view

## Decision

The analysis page now answers, in this order: what change was tested, what
happened when it was simulated, what meaningful consequences were found, what
Eplyx could not explain or evaluate, and what proof backs the result. Hashes,
proof profiles and raw findings are still all on the page, one layer down in
**Technical details**.

The page is a projection of the existing report. It is never a second analysis:

```text
CiReport (+ run record) → analysisView() → markup
```

`frontend/src/analysis.js` is a pure, synchronous view model with no DOM and no
arithmetic on protocol values. Whether replay matched, whether semantic coverage
exists, whether a finding is expected, and whether a change is undeclarable are
all read from the report. The view model decides only which sentence leads,
which facts are grouped together, and how identifiers are shortened.

## Result states

| State | From the report | Headline |
| --- | --- | --- |
| `no_changes_found` | passed, coverage > 0, no findings | No changes found in what Eplyx evaluated. |
| `declared_changes_only` | passed, every finding `expected` | Only declared changes were found. |
| `changes_detected` | `undeclared_change` or `undeclarable_change` | A revert, an economic change, or unexplained state (in that precedence) |
| `declarations_attention` | only `stale_expectation` / `unevaluable_expectation` | A declared change no longer happens / cannot be judged. |
| `not_evaluated` | `no_semantic_coverage`, **or** empty coverage | Economic impact could not be evaluated for this interaction. |
| `not_verified` | exit 2 with no report, or a proof status other than `matched` | Eplyx could not verify execution for this change. |
| `incompatible` | exit 4 with no report | This change cannot be checked against the pinned bundle. |
| `analysis_error` | `execution_error` | The analysis could not complete. |
| `identity_mismatch` | run, stored spec and report disagree (P1), or the report's candidate is not the stored one (P2) | No result shown. |

No state says "safe". Zero findings never counts as a clean result by itself:
empty coverage is `not_evaluated` even when a report lacks the
`no_semantic_coverage` reason. A passed gate is described as a statement about
the declarations and the evaluated subjects, never as a deployment approval.

## Separate dimensions

Every result shows four answers side by side instead of one badge:

```text
Execution            Verified          Checkpointed historical replay matched
Economic impact      Not evaluated     No semantic coverage for this interaction.
Named changes        Not applicable
Unexplained changes  None
```

A report exists only after the engine's baseline fidelity gate has passed. A
replay that does not reproduce aborts with exit 2 and produces no report, so
**Execution: Verified** is a fact about the report existing, and the report's
own `replay_proof.status` is still respected. When the proposed build reverts,
Execution stays Verified, because the historical replay reproduced, and its
note says the proposed build fails in *n of m* interactions.

## Page structure

1. **Proposed change** (the P1 change card).
2. **Result**: an execution failure goes first as a primary alert, followed by
   the execution and impact statements and the gate's reasons in plain
   language.
3. **Impact**: subjects grouped by protocol action, each one either *Changed*
   (with a card), *No change measured*, or *Not compared where execution
   failed*. Cards show Before, Proposed and relative change only when the
   engine emitted them. Below the cards is **What is affected**.
4. **Not explained or not evaluated**: "Additional state changed that Eplyx
   could not semantically explain", with the accounts it touches, decoded
   values that no finding speaks for, unmatched declarations, and the bundle's
   limitations.
5. **Verification**: a compact proof statement, plus an expandable **How does
   Eplyx know this?** section covering the deterministic comparison, the
   interpreter, provenance tier, interface source, whether the exact build was
   verified, and the corroborated facts.
6. **Technical details** (collapsed): the full ChangeSpec identity and identity
   check, all bundle hashes, fidelity profile, proof contract, boundary proof,
   the per-observation SemanticBinding with source blobs, raw findings with
   fingerprints, observations, entities and values, raw undeclarable changes,
   gate reasons by technical name, and raw coverage.

Proof wording comes only from report fields and never overstates them:
contract 2 is described as reconstructed and not directly observed; contract 3
or `derived_target_boundary` as derived by replaying earlier transactions;
checkpointed replay as not claiming exact fidelity; and
`execution_corroborated_external_interface` as corroborated, not exact.
`exact_source_to_elf_verified: false` always appears as "Exact build verified:
no".

Names are formatted generically (`pnl_settled` → "PnL settled",
`vault_a_tokens_out` → "Vault A tokens out"). `analysis.js` contains no
protocol names, and a test enforces this.

## The one engine change

`ReviewedFinding` previously dropped the per-observation `baseline` and
`candidate` values that the adapter had already produced on `NamedFinding`, so
the page had nothing to show as Before or Proposed. It now carries them:

```json
"values": [
  { "observation_id": "2292…", "baseline":  { "kind": "signed_quantity", "quantity": "+0.000203" },
                               "candidate": { "kind": "signed_quantity", "quantity": "-0.000203" } }
]
```

The field is additive and is skipped when empty. `relative_delta_bps` appears
per observation only when it is defined; a signed amount has none, and the page
shows "not defined" rather than inventing one. Replay, semantics, the review
and exit codes are unchanged. **Every report without findings is byte-identical
to before.** The four real fixtures below hash to their frozen phase records
once the C1 `change` binding is removed. Reports with findings, such as the
frozen schema-1 stake-pool controls, gain this one field when they are
regenerated.

`ci::structural_account_change` was also extracted, unchanged, from `check_v2`,
so that the controlled fixtures describe unexplained bytes with the gate's own
code.

## Fixtures

`docs/examples/phase-p3-impact-view/` (see its README) holds real reports:
the Drift U14 semantic baseline, U13.3 `none@0` (contract 3), the Orca U12.1
semantic baseline (contract 2), and U11.2 `none@0`. It also holds three
**controlled** Drift reports: a sign reversal, a named change plus an
undecoded byte, and a revert. These are built from the real report, with only
the target post-state changed, then evaluated by the adapter and judged by the
ordinary review. They are not candidate builds. `impact_view_fixtures_are_current`
asserts every file byte-for-byte; regenerate with `make impact-fixtures`.

`frontend/analysis.test.mjs` renders these fixtures, together with the frozen
U14 legacy report and the U4 stake-pool controls, through the real page. It
covers all eighteen required cases. The only synthetic inputs are run records
for states that produce no report (a replay mismatch, an incompatible bundle,
and an execution error).

## History

Run rows show the change and its target, an **Execution** chip, an **Impact**
chip, the short change ID and the exit code. The bundle and candidate hashes
are no longer in the row. The chips are read from the exit-code contract: exit
2 *with* a report is only ever `no_semantic_coverage`, and without one it is a
preflight abort.

## Not done

This phase adds no new adapter, semantic subject, risk score, recommendation
or deploy verdict. `SemanticAction` and the finding vocabulary are unchanged.
"What is affected" shows interactions, entities, accounts and named subjects,
because those are the categories the report actually contains. The Markdown
report (`report.md`) does not show the new `values` field yet.
