# Phase U11 — Productize Checkpointed Replay

## Result

The frozen U10 transaction now follows the ordinary schema-2 product path:

`ReplayObservationV2 → CorpusStore → bundle build → bundle verify → ci check`

The replay proof matches under `CheckpointedExecutionV1`. No protocol adapter or protocol semantics were added. Ordinary CI therefore reports `replay_proof.status = matched` and `NoSemanticCoverage`; its exit is 2 because semantic coverage remains a gate failure, not because replay failed.

## Proof-profile design

`CheckpointedExecutionV1` is distinct from `CompleteExecutionV2`. The latter still rejects checkpoint-derived proof fields. The new profile binds the observed predecessor checkpoint, native v0 message and frozen validator envelope, LUT observations and lookup-index proof, historical program binaries, retained historical runtime evidence and resolved runtime profile, transaction closure, deterministic execution evidence, twelve validation outputs, and the independently observed terminal checkpoint.

The observation ID hashes the profile and all evidence references. Revalidation resolves the referenced bytes and executes the transaction. Stored `matched`, `admitted`, or `checkpoint_reconciled` booleans have no authority.

## Closure representation

`TransactionClosureProofV1` retains the target index/signature, canonical block identity, complete target-input and validation-output sets, the account-mode finalized block, the retained conflict census, failed-overlap identities, and the pinned `fee_payer_and_durable_nonce_only` rollback rule. Verification reconstructs writable overlaps from the block. It rejects any earlier writable target-input overlap, any successful later validation-output write, missing failed-overlap evidence, a validation output used as fee payer/durable nonce, or a non-zero failed-overlap lamport delta.

## Durable identities and product results

- Observation ID: `9e2fa3d54661a5981d1b7abf60d0e05de40bc183de7abb0a79c9e5cecbdf2cc6`
- Corpus ID: `ad81e16ce2b49b406d9b629945cb67193bb8b870225e7e61ac151572d0e84ea1`
- Bundle ID: `74342122b62fee24cf95c89e0f6c2b5121379d97c3374a000b913b1a15d80fc9`
- Historical execution evidence: `166b16f4f474fade260a70cde32de44f52e1210b602690a126b3f32db35833a0`
- Runtime profile: `cfc489bf79677dedc6b212b68fe4e12abcd156205b8fe2a252fea14dbe104e4b`
- Bundle verification: exit 0; canonical JSON SHA-256 `a1a05576abb0ee215080890c6d413263205c888217e7a9eb85627707783b27b9`
- CI: replay proof matched; `NoSemanticCoverage`; exit 2; canonical JSON SHA-256 `d2c79c7d00abc8df7ae729f063d43edde089754256b731372ff471c3747c358e`
- Empty-environment bundle verification and CI produced the same two stdout hashes.

The corpus contains exactly one immutable observation. Re-insertion is idempotent, and the same ID with different bytes is rejected. The bundle contains the checkpoints, frozen transaction, LUT evidence, historical program and ProgramData bytes, runtime evidence, closure sources, deterministic execution, and terminal observations needed for offline proof.

## Mutation coverage

The named checkpointed-profile test rejects mutations to the start checkpoint, terminal checkpoint, runtime environment blockhash, closure proof, LUT evidence, historical program binary, validation-output set, stored success-boolean bypass, substitution of `CompleteExecutionV2`, and a missing evidence object. The unchanged U10 seven-case campaign also passes: environment blockhash, nonce account, RecentBlockhashes, feature profile, historical BPF/ProgramData, runtime-profile identity, and terminal-checkpoint bytes all fail closed.

## Backward compatibility

The 423 engine unit tests pass. The focused universal/checkpoint suites pass: 20 tests across universal evidence, mutations, no-adapter replay, runtime profile, and checkpointed replay.

Stake Pool exits remain `0/0/1/1/3/5`. Verify, baseline, stale, and unevaluable outputs remain byte-identical. The regression and bounded candidate-report hash differences remain the two known missing-historical-binary differences and were not promoted to new goldens.

Kamino verify, baseline CI, PATH-only verify, and PATH-only CI remain exit 0 with their frozen stdout hashes `529355d96e95bf81c44debd50a007f3624612009cffcf5cee47cc631d05e3b18` and `b7f33953ca35bcdc7949a52a9ea5263739e1be20a1c00ddc1d0d7c89f1a8fe44`.

The U10 ordinary replay and empty-environment replay remain byte-identical at `12635f15e3fb2ffae466cccf7985c2edfaefaffb39b55d28a094892e07864edd`; all seven U10 mutations still fail closed.

## Generality and limitations

The product/core scan over the universal resolver, evidence, corpus, bundle, CI, and command surfaces finds zero branches containing the protocol name, instruction family, domain terms, target signature, or target program address. Target identifiers occur only in the frozen conversion/test artifacts.

This result proves one successful native-v0, durable-nonce transaction at one runtime/feature epoch with four stable LUTs and a one-transaction closure. The reconstructed RecentBlockhashes and SlotHashes inputs remain explicitly derived runtime evidence rather than direct archive account observations. Failed-overlap closure relies on the retained conflict census plus the pinned validator rollback rule. Multi-transaction closures, failed target transactions, other runtime epochs, deactivating LUTs, and other message versions remain unqualified. The 34 known missing-fixture-binary failures remain evidence-availability limitations and were not changed.

The recommended next phase is a generic checkpoint-profile stress matrix across additional protocols, runtime epochs, ordinary and durable blockhash transactions, failed target executions, deactivating LUTs, and multi-transaction closures—still without adding protocol semantics solely to obtain a green CI result.
