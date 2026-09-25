# Phase P3 impact-view fixtures

The CI reports `frontend/analysis.test.mjs` renders. Every file is produced by
the engine and asserted byte-for-byte by
`impact_view_fixtures_are_current` in `engine/tests/drift_settle_semantics.rs`
and `engine/tests/orca_swap_semantics.rs`. Regenerate with
`make impact-fixtures`.

| File | Provenance |
| --- | --- |
| `drift-semantic-baseline.json` | Real `ci::check` of `phase-u14-drift-semantic-bundle` with its historical binary as candidate. Minus `change`, byte-identical to the frozen U14 report (`08cb9ff2…`). |
| `drift-replay-only.json` | Real `ci::check` of `phase-u13-3-sequence-bundle` (`none@0`, contract 3). Minus `change`, byte-identical to the frozen U13.3 report (`19d45309…`). |
| `orca-semantic-baseline.json` | Real `ci::check` of `phase-u12-1-semantic-binding-bundle` (contract 2). Minus `change`, byte-identical to the recorded U12.1 report (`07ce6389…`). |
| `orca-replay-only.json` | Real `ci::check` of `phase-u11-2-checkpointed-bundle` (`none@0`, contract 2). Minus `change`, byte-identical to the frozen U11.2 report (`0114c76d…`). |
| `drift-controlled-sign-reversal.json` | **Controlled.** The real Drift semantic report, with findings from the adapter evaluating the frozen target post-state mutated to settle −0.000203 instead of +0.000203. |
| `drift-controlled-unexplained-bytes.json` | **Controlled.** As above, settling +0.000303 with one undecoded `user` byte (4360) flipped; the structural description comes from the gate's own `structural_account_change`. |
| `drift-controlled-revert.json` | **Controlled.** The target reverts; the program accounts stay at their pre-state, which the structural layer reports against the baseline's settled post-state. |

The controlled reports are locally constructed counterexamples, not candidate
builds: replay proof, semantic binding and change identity are the real
bundle's, and only what a changed target post-state would change is replaced.
They exist because a preserved outcome cannot by itself demonstrate that a
changed outcome is rendered correctly.

Older frozen reports are used as they are: `phase-u14-drift-validation/ci.json`
(written before change identity, so a legacy report) and the schema-1
`phase-u4-validation/product-controls/*.stdout` stake-pool controls.
