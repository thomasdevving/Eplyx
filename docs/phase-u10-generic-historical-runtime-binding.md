# Phase U10 — Generic Historical Runtime Binding

## 1. Runtime-flow changes

The universal path is now `ReplayObservationV2 → resolver → RuntimeProfile → ExecutionRequest → LiteSvmBackend → LiteSVM`. `RuntimeContext` may retain separate `HistoricalRuntimeEvidence`; the resolver verifies that evidence, hashes the exact runtime-sysvar snapshot, and derives a content-addressed `RuntimeProfile`. `ExecutionRequest` carries that resolved profile instead of loose signature/blockhash booleans. The backend revalidates the profile and sysvar hash before creating the VM.

## 2. New generic types and APIs

`HistoricalRuntimeEvidence` binds the environment blockhash, sysvar-snapshot hash, feature profile, native-program profile, and provenance under evidence ID `9d8ccbfa241a65840af42ff03b35841f175e43e901f9d879d3b89a9a3a615455`.

`RuntimeProfile` binds the resolved evidence ID, environment blockhash, sysvar hash, feature/native profiles, signature and recent-blockhash policies, Instructions construction, and SlotHashes policy under profile ID `cfc489bf79677dedc6b212b68fe4e12abcd156205b8fe2a252fea14dbe104e4b`. Legacy callers use `RuntimeProfile::legacy`; schema-2 resolution derives the profile from retained evidence.

## 3. LiteSVM configuration mechanism

LiteSVM 0.16.0's `set_latest_blockhash` is insufficient: its transaction processor passes the message recent blockhash into `EnvironmentConfig`, so that setter affects admission but not native nonce execution. U10 therefore enables LiteSVM's small official `invocation-inspect-callback` feature. Immediately before execution, a generic callback sets the public `InvokeContext.environment_config.blockhash` to the profile value. No LiteSVM source was forked or copied, and the native V0 message and signature are unchanged.

## 4. Exact runtime inputs bound

The bound historical configuration contains environment blockhash `A2uVDFYNS3HNaYMus1fM6FztNkanJsV74K66FF4mDjZi`, distinct transaction recent blockhash `3YvBduVCF7V38foZbfs6coHra7BdTCsz2en4mSQWCHBw`, exact Clock, Rent, EpochSchedule, reconstructed pre-transaction RecentBlockhashes, retained SlotHashes, LiteSVM 0.16.0 mainnet features corroborated against the U9 target feature census, Agave 4.2.2 System/Compute native behavior, generated Instructions, disabled signature verification, and disabled recent-blockhash inclusion checking. The sysvar snapshot hash is `1365a82081f95442fccdd60ce795bf264c9491f39aa4edefd4b6e1f3437c7264`.

Execution additionally uses the complete S-1 account set, four exact LUT accounts, and all historical BPF accounts/ProgramData/ELFs. The frozen seed identity is `b0d7b8e5ce6f195ab77c06cdec73efa266dd6bd0c0d319b219272572dc7189cf`; no post-state is seeded.

## 5. Feasibility result

The rerun gate is **A — `BOUNDED_AND_SUPPORTED`**. Every bounded input is representable, the backend honors the historical profile, and the one-transaction closure is executable. This is an experimental `CheckpointedReplayBoundary`: it is derived and independently checkpoint-reconciled, not directly observed per-write evidence.

## 6. Nonce result

The first hard assertion passed. Local `AdvanceNonceAccount` produced `GU8hFD4frrKz5zkwqU4d7wq2Ynor89uNp4dCTZy7cizE`, exactly the independently proven checkpoint-S nonce. A changed environment blockhash produces the canonical changed nonce and stops as `RuntimeConfigurationMismatch`.

## 7. Execution-fidelity result

The validator and local execution both succeed with no error, fee `5,750`, and `58,561` compute units. All 40 log lines match exactly. CPI groups, program indexes, account indexes, instruction bytes, and stack heights match exactly. Return-data representation also matches. Execution evidence hash: `166b16f4f474fade260a70cde32de44f52e1210b602690a126b3f32db35833a0`.

## 8. Twelve-account reconciliation

All 12 U8 validation outputs match the independent S checkpoint exactly for presence, owner, lamports, executable flag, rent epoch, and complete raw data. This includes the absent/closed account and the durable nonce. The frozen S-checkpoint identity is `eb5a9c095e5a32bcaa628d7ff3f2fc06f73bdbc0a7070aa8569fc7c28ce320b5`.

## 9. Offline result

The complete qualification ran again with an empty process environment: no PATH, HOME, RPC URL, provider credential, or provider configuration. It exited `0`, and stdout was byte-identical to the ordinary run with SHA-256 `12635f15e3fb2ffae466cccf7985c2edfaefaffb39b55d28a094892e07864edd`. The proof consumes only checked-in evidence and compiled local code.

## 10. Mutation results

Seven targeted mutations all failed closed: environment blockhash, nonce account, RecentBlockhashes, feature profile, historical BPF/ProgramData, runtime-profile identity, and checkpoint-S bytes. They were rejected respectively by the nonce hard assertion, seed identity, sysvar/profile binding, supported-runtime constraint, seed identity, profile identity, and checkpoint identity. No mutation reached a false exact match.

## 11. Backward-compatibility controls

Stake Pool controls remain `0/0/1/1/3/5`, with all six preserved U2 stdout hashes. Kamino bundle verification and baseline CI pass in ordinary and PATH-only environments with unchanged stdout hashes `529355d96e95bf81c44debd50a007f3624612009cffcf5cee47cc631d05e3b18` and `b7f33953ca35bcdc7949a52a9ea5263739e1be20a1c00ddc1d0d7c89f1a8fe44`. The full workspace suite passes: 423 engine unit tests plus all integration, server, and interface tests; one pre-existing tooling test remains ignored.

## 12. Protocol-specific branch scan

The universal runtime, resolver, pipeline, replay compatibility path, and generic nonce tests contain zero Orca, Whirlpool, target-signature, target-account, or CLMM branches. Target identifiers occur only in the frozen qualification example/evidence. No protocol semantics, pricing math, pool layout, or product replay schema was added.

## 13. Remaining blocker or next phase

There is no remaining blocker for this bounded target, and U10 meets strong success. The next phase should qualify the same generic runtime-profile contract across additional protocols, runtime/feature epochs, durable and ordinary blockhash transactions, failed transactions, deactivating LUTs, and multiple-transaction closures. Only after that broader adversarial evidence should Eplyx productize a distinct checkpoint-derived proof profile; `CompleteExecutionV2` must continue to distinguish derived checkpointed provenance from directly observed transaction boundaries.

Primary artifacts: [replay](examples/phase-u10-analysis/replay.json), [runtime binding](examples/phase-u10-analysis/runtime-binding.json), [feasibility](examples/phase-u10-analysis/feasibility.json), [offline proof](examples/phase-u10-analysis/offline.json), [mutations](examples/phase-u10-analysis/mutations.json), and [controls](examples/phase-u10-freeze/controls.json).
