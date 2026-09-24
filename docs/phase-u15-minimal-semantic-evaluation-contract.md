# Phase U15 — minimal semantic evaluation contract

## Result

`ProtocolAdapter::evaluate_semantics` now receives a borrowed
`SemanticEvaluationContext`: the historical transaction, named target PRE
accounts, and baseline/candidate **target POST** execution results. Schema-2 CI
calls it once per observation. It does not pass the causal suffix terminal
frontier or bundle internals. Execution, independently recomputed
`SemanticBinding`, and evaluation remain separate.

The result distinguishes `Unsupported`, `Unevaluable { reason }`, and
`Evaluated { subjects, findings, explained }`. A genuine evaluator error uses
`Result::Err` and aborts CI with exit 2. CI validates each result before using
it: every finding must have an advertised subject, and explained byte ranges
must be well formed and tied to an economic finding. `Unevaluable` advertises
no economic value or coverage; CI adds a structural explanation and
`no_semantic_coverage`. `Unsupported` produces ordinary absent coverage. A
candidate revert yields an execution finding only; it does not invent an
economic zero.

The generic structural check now consumes the evaluator's explicit explained
ranges. Only a changed byte inside a range attached to an emitted finding is
suppressed. Different account labels, data length changes, and bytes outside
those ranges remain structural. The Drift User byte at offset 4360 stays
undeclarable even alongside a valid `pnl_settled` finding.

## Bounded adapter migrations

Drift's single frozen `settlePnl` shape still uses the pinned layouts,
fixed-point deposit conversion, and cross-account settlement checks. Baseline
settlement remains signed `+203` quote base units, with two subjects and no
baseline findings. The evaluator now reports malformed or inconsistent target
state as `Unevaluable`. Its old `named_findings` hook delegates to the new
evaluation for callers on the older replay path.

Orca's direct SwapV2 evaluator reads the same shared SPL Token and Token-2022
decoders and computes the same three measured flows. It has four subjects,
zero protocol-specific CLMM math, and no user output claim. Invalid account
state or inconsistent flow is `Unevaluable`; unsupported transaction shapes
are `Unsupported`. Its older finding hook likewise delegates. Neither adapter
replays a transaction to evaluate semantics.

The default trait implementation combines the existing coverage, finding, and
decoded-source hooks for Stake Pool, Kamino, and Token-2022. Their adapters and
serialized formats did not change. `summarize` remains because the older
`replay` report pairs baseline and candidate economic observations through it;
Stake Pool and Kamino also call it within their existing adapters. It remains
transaction-free for that older reporting purpose. The new seam handles
transaction-aware semantics.

The Orca and Drift binding methods still independently corroborate the
historical baseline and retain
`execution_corroborated_external_interface`; evaluation success does not
upgrade either to source-to-ELF verification. U14 signed quantities use the
existing `SemanticValue::SignedQuantity` variant. No schema version or bundle
format changed.

## Frozen controls and tests

The rebuilt CLI reproduced these canonical CI JSON files byte for byte:

| Bundle | Replay | Coverage | Findings | Exit | SHA-256 |
| --- | --- | ---: | ---: | ---: | --- |
| U14 Drift semantic | matched, contract `[3]` | 2 | 0 | 0 | `08cb9ff221228e81e5ac27f7342dae9b60b9902e60924a46f5f78d1a062b4ffc` |
| U13.3 Drift `none@0` | matched, contract `[3]` | 0 | 0 | 2, `no_semantic_coverage` | `19d45309e14e529085481a21dba5d47bb03d1b0133105d1811b072960d508b4f` |
| U12.1 Orca semantic | matched | 4 | 0 | 0 | `07ce6389c6b2b0105e1886db6c15648eece8a586c2fdba93f054bc1b5762e04a` |

Focused tests exercise both new evaluator baselines, unsupported versus
unevaluable states, malformed amounts, signed findings, explained ranges,
unknown bytes, binding independence, and the default legacy shim. Existing
contract-3 causal tests prove target POST is used instead of the suffix
frontier; the no-adapter control still replays under `none@0`. The CI unit
test checks that explained ranges suppress only their own changed bytes and
that a length change remains visible. Result validation rejects an adapter
finding without corresponding coverage.

## Size and interface pressure

Nonblank, noncomment LOC, counted by contiguous source spans in the two
adapter files after U15:

| Responsibility | Drift | Orca |
| --- | ---: | ---: |
| Instruction recognition | 57 | 80 |
| Account role binding | 38 | 61 |
| Layout and token decoding | 116 | 43 |
| Deterministic protocol math | **47** | **0** |
| Subjects and findings, including legacy hooks | 149 | 175 |
| Provenance binding | 116 | 98 |
| Plumbing, imports, and trait methods | 65 | 78 |
| **Total** | **588** | **535** |

The previous totals were 518 Drift and 483 Orca. This contract adds explicit
status and validation, so total LOC grew. The separate non-fallible finding
implementations shrank from 53 to 17 physical lines in Drift and from 66 to
17 in Orca; their remaining wrappers serve the older replay API. Schema-2 CI
now has one adapter evaluation call instead of separate coverage and findings
calls, and it consumes explained ranges from that result. The old coverage
hooks remain for older replay users. The new architecture reduces duplicated
decision paths rather than optimizing total LOC.

The evaluator remains bounded to the previously supported Orca and Drift
shapes. No universal execution, resolver, sequence proof, evidence, bundle,
or runtime-profile code changed.

## Reproduction

```sh
cargo test -p eplyx-engine --lib --test drift_settle_semantics \
  --test orca_swap_semantics --test causal_sequence \
  --test semantic_binding_provenance --test universal_no_adapter
cargo clippy -p eplyx-engine --lib --bin eplyx \
  --test drift_settle_semantics --test orca_swap_semantics -- -D warnings
cargo build -p eplyx-engine --bin eplyx
target/debug/eplyx ci check --bundle docs/examples/phase-u14-drift-semantic-bundle \
  --candidate docs/examples/phase-u14-drift-semantic-bundle/binaries/current.so --format json
```
