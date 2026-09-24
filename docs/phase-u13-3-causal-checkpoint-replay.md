# Phase U13.3 — causal checkpoint replay

Contract 3 productizes the historical sequence `73 → 428 → 431 → 438 → 1245`
at slot `409942000`. It uses the ordinary schema-2 observation, resolver,
CorpusStore, bundle build/verify, and CI paths. There is no execution adapter
or semantic adapter for this witness.

The fidelity profile remains **CheckpointedExecutionV1**. Contract 1 retains its
frozen compatibility meaning; contract 2 retains reconstructive one-target
closure. Contract 3 is a **reconstructive causal sequence proof**, yielding a
**DerivedTargetBoundary** within a **CheckpointedReplayBoundary**. It is not
an ObservedTransactionBoundary and does not claim Exact fidelity.

## Durable evidence

- [New corpus](examples/phase-u13-3-sequence-corpus/manifest.json): observation
  `588b1abd2d2b06689604a81856d3a90493035509c4638a490dea73416eed407e`.
- [New bundle](examples/phase-u13-3-sequence-bundle/bundle.json):
  `7bc43449fd33715ffffdb820de4d339803ed263e6c166e8f0cae33eac1bb956d`.
- Sequence proof:
  `08be90ebefe35bc3b7439cc12a9001fd7a2664c0798e84158c98a546cf33975b`.
- [Ordinary CI report](examples/phase-u13-3-validation/ci.json): replay `matched`,
  profile `checkpointed_execution_v1`, proof contracts `[3]`, boundary proof
  explicitly derived, exit 2 solely for `no_semantic_coverage`.
- [Offline bundle verification](examples/phase-u13-3-validation/bundle-verify.json).

The bundle contains 135 evidence objects, including the complete source block,
all five raw native-v0 transactions, two reconstructed LUTs, 23 observed parent
accounts (including chunk-reconstructed ProgramData), runtime account and
feature receipts, historical ELF, all intermediate frontier commitments, target
execution commitment, and all nine terminal AccountSnapshots. ComputeBudget is
an input supplied by the evidence-bound historical native runtime; it is not
misrepresented as an archived account receipt. The historical RuntimeProfile is
`82d133c01dfdffd1d28d4115f969e7ecf73178c41829ccdc3b882910c270e639`.

The [converter](../engine/examples/convert_u13_3_sequence.rs) reads retained raw
receipts and executes fresh commitments. It never reads `feasibility.json`.
Previously frozen observations, bundles, and U13 research artifacts are unchanged.

## Resolution and candidate isolation

[HistoricalSequenceClosureProofV1](../engine/src/universal/sequence.rs) binds the
slot, distinguished target identity, checkpoints, exact ordered entries,
account-labelled dependency edges, raw block, runtime evidence, exact executable
identities, complete terminal frontier, and target execution commitment. Each
entry binds its message/LUT proof, full key order, expected validator envelope,
runtime profile ID, and intermediate frontier digest.

Resolution recomputes the conservative U13.1 last-writer graph from raw block
messages. It compares exact sequence membership, canonical index ordering,
all 11 edges, parent frontier, and terminal frontier. Matching behavior cannot
substitute for graph validity. Dependencies include readonly message accounts,
LUT accounts, and historical ProgramData. No successful possible writer is
silently discarded. Unanchored sequence outputs fail closed.

The VM is initialized once from observed parent state and historical runtime
inputs. The shared executor runs all sequence messages in that same VM, carrying
fees and writes forward. Every entry must match outcome, fee, retained CU,
ordered logs, inner instructions, and return data. All per-transaction balance
vectors and retained token balances are also checked. All nine terminal accounts
must match presence, lamports, owner, executable, rent epoch, and complete data.
SlotHashes materiality controls execute the same complete historical sequence.

The target post-state is copied immediately after the distinguished transaction.
It is checked against stored content and execution commitments, then returned
with the target pre-state, target message, historical ELF, runtime profile, and
watched outputs. Stored derived bytes cannot authorize themselves. The suffix
is executed during resolution, never by `pipeline::execute` or candidate ELF
substitution. Baseline and candidate execution each run the distinguished target
only. A backend instrumentation test verifies which message and substituted
loader bytes reach candidate execution.

## Bounded contract

This first sequence contract admits native-v0 messages, one slot and runtime,
and an unchanged historical executable environment. It requires evidence-bound
historical features. Relevant failed writable declarations are rejected instead
of assuming rollback effects; this is deliberately stricter than the U13.1
research planner's failed-transaction fee/nonce classification. Contract 2's
existing rollback proof and serialization are unchanged. In-slot LUT writes,
changes to executable/runtime identities, and overlapping parent/runtime sysvar
seeds also fail closed. Extending those cases requires further proof rules.

Provider observations remain provider evidence, not validator attestations.
The historical ELF is observed ProgramData, not a source-to-ELF reproducibility
claim. The runtime has the same compatibility limits recorded in U13.2B/C.
FastRPC prospective evidence is independent of this historical contract.

## Behavioral tests

[causal_sequence.rs](../engine/tests/causal_sequence.rs) contains named tests for
all 18 requested mutations. Rebuilt CAS hashes reach semantic proof checks:
missing entries/order, dependency edges, raw-message reconstruction, validator
envelopes, LUT reconstruction, parent frontier, intermediate state, derived
target commitment, terminal reconciliation/completeness, historical binary and
feature evidence, distinguished identity, and prohibited suffix substitution.

Three controlled standard-program cases protect generality:

1. Target changes X from 10 to 17; suffix changes X to 28. Resolution and
   baseline/candidate comparison preserve 17 while terminal reconciliation
   validates 28.
2. Two later transfers produce the same final state in either order. Reordering
   still fails the canonical dependency proof.
3. Omitting the last transfer leaves terminal X unchanged but leaves Y wrong.
   Exact terminal reconciliation detects it independently of target output X.

## Reproduction

From the repository root, with the Rust toolchain on PATH:

```sh
cargo build -p eplyx-engine --bin eplyx --example convert_u13_3_sequence
cargo test -p eplyx-engine --test causal_sequence
# Use fresh directories; the committed corpus/bundle are immutable controls.
target/debug/examples/convert_u13_3_sequence /tmp/u13-3-corpus
target/debug/eplyx bundle build --corpus /tmp/u13-3-corpus \
  --baseline docs/examples/phase-u13-2a-runtime/historical-drift.so \
  --out /tmp/u13-3-bundle
target/debug/eplyx bundle verify --bundle /tmp/u13-3-bundle --format json
target/debug/eplyx ci check --bundle /tmp/u13-3-bundle \
  --candidate /tmp/u13-3-bundle/binaries/current.so --format json
```

The last command intentionally exits 2 with `NoSemanticCoverage`. No RPC endpoint
is needed for conversion, proof reconstruction, bundle verification, or CI.

## Verification results

[Validation receipt](examples/phase-u13-3-validation/validation.json): 532 tests
passed across the engine library and 12 relevant integration suites, including
26 new sequence tests and all 23 Stake Pool tests. The missing local Stake Pool
SBF fixtures were rebuilt before the final compatibility run. All 15
[compatibility controls](examples/phase-u13-3-validation/compatibility.json)
match their expected exit codes; all 13 controls with frozen stdout hashes
remain byte-identical. U13.1/2A/2B/2C checksums verify all 402 frozen files; the
existing closure and frontier audits also pass.

Strict Clippy passes for the changed library, CLI, converter, and sequence test
targets; workspace formatting and `git diff --check` pass. The all-target Clippy
command on Rust 1.98 reports an existing `cloned_ref_to_slice_refs` warning in
the U11 converter; that frozen converter was left unchanged. The core generality
scan finds zero witness names, signatures, program IDs, slots, or transaction
indexes in the generic sequence, resolver, pipeline, and executor paths.

A [fresh conversion and bundle build](examples/phase-u13-3-validation/reproducibility.json)
reproduces every corpus and bundle file byte-for-byte, including the observation
and bundle identities above.
