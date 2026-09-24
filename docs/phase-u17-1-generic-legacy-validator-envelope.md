# Phase U17.1 — generic schema-2 legacy validator envelope

## Result

**U17.1-A: legacy validator envelope qualified.** `ExecutionInput::LegacyV2`
retains an exact content-addressed validator transaction and a native Solana
legacy `Message`. Resolution reconstructs the message from that transaction,
compares the complete native message, checks every retained signature, and
compares the entire normalized `HistoricalTransaction`. The reconstructed
message is the one passed to execution. The ordinary schema-2 resolver and
contract-3 sequence resolver obtain the new variant's validator transaction
through the existing `validator_transaction_ref()` path. No fidelity profile
or replay rule changed.

The old `Legacy` and `LegacyV1Compatibility` tags and their serialized fields
are unchanged. They still have no validator reference. V0 representation is
unchanged. The new serialized tag is `legacy_v2`; it carries `message`,
`transaction`, `signatures`, and `frozen_transaction`. The capture helper stores
the exact RPC transaction result and immediately resolves the new input as a
self-check.

## Envelope checks

The legacy reconstruction checks the message version and absence of lookup
descriptors, parses every base58 account key and recent blockhash, checks the
header and account indexes, and compares native `Message` equality. This binds
header/signers, key order, blockhash, instruction count, program indexes,
account indexes, and instruction bytes. Every signature is decoded as 64 bytes;
the complete vector and signer count must match the retained transaction.

The normalized transaction comparison binds signature, slot, account access
flags, instruction and CPI normalization, success/error, fee, compute units,
logs, pre/post lamports, and pre/post token balance metadata. The existing
generic validator-envelope verifier additionally compares the declared
success/error, fee, logs, CPI groups, return data, and compute units when
declared. It is exposed for an envelope-only qualification control; ordinary
schema-2 resolution calls the same logic. Account-boundary checks remain in
the unchanged fidelity profiles and still require historical account evidence.

## Phoenix production control

The retained U17 [validator transaction](examples/phase-u17-phoenix-qualification/transaction.json)
at slot `450026714` resolves as `legacy_v2` with its complete six-instruction,
18-key message. Its retained signature is
`rR1tBf4kb4xocTPpLZXRPqFFtvXQHn2GwAEgHbU8d83URqJNWnACGcz6cJfq27kGYhuWaTjrce7ksBNAF5NZVQC`.
The test verifies the one CPI group with two inner instructions, return data,
four pre and four post token balances, and the generic expected outcome. The
[U17 block census](examples/phase-u17-phoenix-qualification/analysis.json),
reproduced from the retained account-mode block by the U17 checker, locates
that signature at index `95`. The production control compares signature,
slot, and index with this independent census. Index is a block position, not
part of a Solana message; contract 3 binds it directly to the retained full
block, while this envelope-only control uses the U17 census.

This is **legacy validator envelope qualified** only. No Phoenix historical
account receipts, target PRE/POST, execution replay, order state, fills,
semantic subjects, corpus, or bundle were produced. The retained public RPC
responses are provider observations, not cryptographic validator attestations.

## Negative and compatibility controls

The [new integration tests](../engine/tests/legacy_validator_envelope.rs)
reject changed signatures, blockhash, key order, header, instruction program
index, instruction count, instruction account indexes, instruction data, slot, swapped validator
evidence, pre/post lamports, token amounts, outcome, fee, compute units, logs,
CPI, and return data. Evidence swaps and metadata mutations use new
content-addressed objects. Changing only the retained transaction index still
resolves the message, then fails the production block-census comparison, which
is the appropriate proof layer for an index claim. Old serialized `legacy`
still round-trips with no validator reference.

The [contract-3 tests](../engine/tests/causal_sequence.rs) now execute a
synthetic three-transaction sequence with a legacy middle transaction and V0
neighbors, retaining all existing closure and replay rules. A rebuilt legacy
input with the wrong block index fails the retained-block comparison. The
frozen U13.3 contract-3 witness still passes.

The three frozen CI JSON controls remain byte-identical:

| Bundle | Exit | SHA-256 |
| --- | ---: | --- |
| U14/U15 Drift semantic | 0 | `08cb9ff221228e81e5ac27f7342dae9b60b9902e60924a46f5f78d1a062b4ffc` |
| U13.3 Drift `none@0` | 2, `no_semantic_coverage` | `19d45309e14e529085481a21dba5d47bb03d1b0133105d1811b072960d508b4f` |
| U12.1 Orca semantic | 0 | `07ce6389c6b2b0105e1886db6c15648eece8a586c2fdba93f054bc1b5762e04a` |

The focused library, sequence, adapter, provenance, and no-adapter suites pass
484 tests. Targeted Clippy passes with warnings denied. The U17 block-census
analysis reproduces byte for byte.

The generic source changes are restricted to `universal/model.rs` and
`universal/resolver.rs`; neither contains a Phoenix program ID, target
signature, slot, opcode, market address, or account-role name. No protocol
adapter, semantic evaluator, universal executor, resolver proof contract,
bundle schema, or CI format changed.

## Readiness for U17.2

The legacy envelope blocker is removed. U17.2 still needs exact parent-slot
and target-slot raw account receipts for the complete message and invoked
programs, historical runtime and feature evidence, historical executable
bytes, and a qualified target-boundary replay. Until then, the correct
Phoenix state and semantic status remains unqualified.

## Reproduction

```sh
cargo test -p eplyx-engine --test legacy_validator_envelope --test causal_sequence
python3 scripts/analyze-u17-phoenix-qualification.py > /tmp/u17-phoenix-analysis.json
cmp /tmp/u17-phoenix-analysis.json docs/examples/phase-u17-phoenix-qualification/analysis.json
```
