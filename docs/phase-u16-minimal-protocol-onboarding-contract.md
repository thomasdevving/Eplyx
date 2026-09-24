# Phase U16 — minimal protocol onboarding contract

## Inventory before extraction

The five compiled adapters already share `ProtocolAdapter`, the U15 evaluation
result, and ordinary Rust registration. The table records where each onboarding
responsibility lives before this phase's refactor. “Legacy” means the default
U15 evaluation shim calls the older coverage and finding hooks.

| Adapter | Identity and recognition | Roles and decoding | Provenance | Evaluation, subjects and explained bytes | Legacy reporting and math |
| --- | --- | --- | --- | --- | --- |
| Token-2022 | Program ID, version 2, `TokenOp` opcode/arity and direct-instruction checks | Per-op role arrays; shared Token-2022 layout/extension decoder | Default `ManualOrUnknown` | No promoted named subjects; default U15 shim | Account `decode`/`summarize` path; transfer-fee arithmetic for boundaries |
| Stake Pool | Program ID, version 3, `PoolOp` opcode/arity plus companion checks | Per-op ordered roles; shared SPL Token/System decoders and pool/stake fields | Default `ManualOrUnknown` | Legacy subject/finding hooks and source/range maps | `summarize`; pool-share and fee calculations |
| Kamino | Program ID, version 1, `KlendOp` Anchor discriminators and required refresh companions | Action and farms roles; shared token readers plus reserve/obligation layouts | Default `ManualOrUnknown` | Legacy subject/finding hooks and source/range maps | `summarize`; scaled debt/fraction calculations |
| Orca | Program ID, version 1, direct SwapV2 discriminator and exact-input/companion guards | 15 ordered roles; shared SPL Token/Token-2022 decoding and Whirlpool link | Pinned repository commit/source blobs; independent execution corroboration | U15 evaluator: execution plus three measured flow subjects; no explained account ranges | Older `named_findings` wrapper; **0 CLMM math LOC** |
| Drift | Program ID, version 1, frozen signature/slot/direct `settlePnl` shape | 14 exact addresses; Anchor User/PerpMarket/SpotMarket layouts and links | Pinned repository commit, IDL and source blobs; independent execution corroboration | U15 evaluator: execution plus signed PnL; exact explained settlement ranges | Older `named_findings` wrapper; **47 fixed-point math LOC** |

Repeated machinery is narrow: both bounded adapters repeat immutable interface
identities, direct instruction matching, ordered account-role/access checks,
and construction of `RepositoryInterface` from pinned blobs. Their economic
readers, companion restrictions, historical corroboration facts, and formulas
are different. Shared token decoding already lives in `standard_programs`;
moving it again would add no value. Stake Pool, Kamino, and Token-2022 have
different multi-instruction contracts and stay on their existing paths.

## Extraction

[`protocol/onboarding.rs`](../engine/src/protocol/onboarding.rs) holds the
small compiled contract:

```text
ProtocolDescriptor (name, program ID, adapter version, optional source, interactions)
  → recognize(transaction) → RecognizedInteraction (ID, outer index, matched discriminator, target instruction)
  → bind_roles(...) → BoundInteraction (ordered labels and checked account metas)
  → adapter's independent SemanticBinding and U15 evaluator
```

Recognition first narrows by exact program ID, then discriminator prefix and
optional outer index. Zero matches distinguish unknown program from known
program with unsupported instruction; multiple matches produce a typed
`AmbiguousInteraction` error. `bind_roles` checks exact account count,
deterministic order, unique labels, optional exact addresses, signer/writable
requirements, and optional distinct addresses. A bound role can check a
historical account's owner, exact length, and discriminator. These checks do
not assign economic meaning to a role or treat all account bytes as decoded.

Orca and Drift each declare one immutable descriptor and local role rules.
Their existing companion, transaction-boundary, owner/type, and economic
checks remain in their adapters. Orca uses the bound Whirlpool role in its
independent historical corroboration and shared token decoders in evaluation.
Drift retains its frozen signature/slot and fourteen exact addresses, Anchor
layouts, fixed-point formula, and explicit U15 explained ranges. Neither
adapter calls a generic evaluator or changes replay code. The legacy three
adapters are unchanged.

`SourceIdentity` converts pinned repository/commit/blob data to the existing
`RepositoryInterface`. The descriptor groups that source identity with the
adapter version; it carries no ELF or execution hash. A future adapter may
choose a different descriptor for a deployment era or instruction version,
without automatic source selection. Orca and Drift retain their exact
historical `execution_corroborated_external_interface` checks, including the
separate ELF and execution hashes. A strong replay proof still says nothing
about source-to-ELF equivalence.

## Selection, failure stages, and versioning

Ordinary Rust [`adapters()`](../engine/src/protocol/mod.rs) remains the only
compiled registration list. `adapter_for(program_id)` narrows by program ID;
it now fails on duplicate registered IDs instead of silently picking the
first. The selected adapter recognizes its own interaction. This supports a
new protocol by adding an adapter to that list, without changing universal
execution. Multiple interactions for one program belong in its descriptor;
their recognizers must be unambiguous. No arbitrary cross-adapter guessing or
dynamic loading is introduced.

| Stage | Internal result | U15/CI consequence |
| --- | --- | --- |
| No compiled program | `adapter_for` returns `None` | No adapter semantic coverage |
| Known program, other opcode/shape | `UnsupportedInteraction` | `Unsupported`, no semantic coverage |
| Two matching recognizers | `AmbiguousInteraction` | evaluator error, exit 2 |
| Recognized, wrong account roles/access/type | `RoleBinding` | `Unevaluable` with reason; structural fail-closed |
| Bound, missing or inconsistent economic state | evaluator's `Unevaluable` | no invented zero; structural fail-closed |
| Bound and computed | `Evaluated` | subjects, findings, explicit explained ranges |
| Implementation/configuration failure | `Result::Err` | CI exit 2 |

Recognition, role binding, semantic coverage, deterministic evaluation, and
provenance tier are separate capabilities. Replay support remains a separate
execution property. `ProtocolDescriptor.version` feeds the existing
`adapter_version()`/bundle compatibility check. It remains `1` for Orca and
Drift because their recognized shapes, formulas, layouts, and subject meanings
did not change. A future change to any of those must bump that version; U16
does not reinterpret existing bundles.

## Interface size

Counted as nonblank, noncomment lines (tests excluded from the shared module):

| Code | Before U16 | After U16 | Delta |
| --- | ---: | ---: | ---: |
| Orca adapter | 535 | 535 | 0 |
| Drift adapter | 588 | 610 | +22 |
| Shared onboarding module | 0 | 268 | +268 |

The new module holds 268 common production LOC for recognition, role checks,
source conversion, and typed stage failures. Adapter files grew by 22 LOC in
total because static descriptors make previously implicit roles and source
identity explicit. The net increase is 290 LOC. This is a responsibility
extraction, not a size reduction: no Orca flow logic or Drift settlement math
was generalized. The new path centers on `ProtocolDescriptor`,
`RecognizedInteraction`, and `BoundInteraction`, with `SourceIdentity`, role
rules, and typed failures alongside them. `ProtocolAdapter` gained **zero**
required methods.

`ProtocolAdapter` still requires seven methods for any adapter: name, program
ID, instruction contract, label, decode, boundary proof, and interpret. A new
protocol promoting named semantics additionally overrides `action_id` and
`evaluate_semantics`; it overrides `semantic_binding` when it can corroborate
provenance. Simple flows and custom math use the same two semantic overrides.
A Drift-like adapter adds local account decoders and deterministic math
functions, with **zero** additional framework methods. `summarize` remains
available for older reports. `semantic_action` is optional corpus
classification and defaults to `Unknown`; adapters override `adapter_version`
when their semantic version differs from the trait's default.

## Verification

The U16 unit and integration tests cover deterministic recognition, known
program/unsupported instruction, ambiguous recognizers, role/access/alias and
owner/type failures, version and historical source identity separation,
Orca's simple flow path, Drift's custom math path, independent binding,
unknown raw bytes, the U15 default legacy shim, and `none@0` replay. The
focused library, causal, adapter, provenance, and no-adapter suites passed
**477 tests**. Targeted Clippy passed with warnings denied.

The rebuilt CLI reproduced all three frozen canonical CI JSON files byte for
byte; no report schema or bundle identity changed:

| Bundle | Replay | Coverage | Findings | Exit | SHA-256 |
| --- | --- | ---: | ---: | ---: | --- |
| U14/U15 Drift semantic | matched, contract `[3]` | 2 | 0 | 0 | `08cb9ff221228e81e5ac27f7342dae9b60b9902e60924a46f5f78d1a062b4ffc` |
| U13.3 Drift `none@0` | matched, contract `[3]` | 0 | 0 | 2, `no_semantic_coverage` | `19d45309e14e529085481a21dba5d47bb03d1b0133105d1811b072960d508b4f` |
| U12.1 Orca semantic | matched | 4 | 0 | 0 | `07ce6389c6b2b0105e1886db6c15648eece8a586c2fdba93f054bc1b5762e04a` |

Only files under `engine/src/protocol`, the two adapter test files, and this
report changed. A source scan found no onboarding reference in universal
execution, replay, sequence proof, resolver, evidence, or bundle code.

## Reproduction

```sh
cargo test -p eplyx-engine --lib --test drift_settle_semantics \
  --test orca_swap_semantics --test causal_sequence \
  --test semantic_binding_provenance --test universal_no_adapter
cargo clippy -p eplyx-engine --lib --bin eplyx \
  --test drift_settle_semantics --test orca_swap_semantics \
  --test causal_sequence --test semantic_binding_provenance \
  --test universal_no_adapter -- -D warnings
cargo build -p eplyx-engine --bin eplyx
target/debug/eplyx ci check --bundle docs/examples/phase-u14-drift-semantic-bundle \
  --candidate docs/examples/phase-u14-drift-semantic-bundle/binaries/current.so --format json
```
