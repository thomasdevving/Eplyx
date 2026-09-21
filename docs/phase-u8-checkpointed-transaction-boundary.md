# Phase U8 — Generic Checkpointed Transaction-Boundary Reconstruction

## Decision

Phase U8 reaches **U8-A only**: a sound bounded replay closure was derived. The closure is the single target transaction at index `259`; it is V0 and successful. Replay was not implemented because the retained evidence fails the Stage 15 acquisition gate:

**D — `HISTORICAL_EVIDENCE_UNAVAILABLE`**

The missing items are four historical LUT accounts, three upgradeable ProgramData/ELF accounts, the historical native/runtime identity, and target-slot sysvar/feature evidence. This is not an Orca limitation and does not justify an expensive per-write provider. Alchemy remains useful for the independent S-1/S checkpoints and the finalized block, but its coalesced account stream is not used as a complete transaction-boundary oracle.

Primary evidence:

- [precise conflict table](examples/phase-u8-analysis/conflict-table.json)
- [backward and forward closure](examples/phase-u8-analysis/closure.json)
- [binary, LUT, account, and runtime census](examples/phase-u8-analysis/evidence-census.json)
- [feasibility decision](examples/phase-u8-analysis/feasibility.json)
- [frozen inputs](examples/phase-u8-freeze/frozen-inputs.json)
- [final manifest](examples/phase-u8-freeze/final-manifest.json)

## Final-report matrix

| # | Required result | U8 result |
|---:|---|---|
| 1 | Initial/final repo state | HEAD unchanged; inherited dirty tree preserved; analysis/evidence/report only |
| 2 | Corrected conflicts | 0 earlier; 8 later, not the U6 transitive upper bound |
| 3 | Failed/successful overlaps | 6 failed; 2 successful |
| 4 | Committed effects | failed ordinary effects rolled back; successful read-only-mint effects unknown |
| 5 | Target accounts | 22 inputs; 12 validation outputs; 10 input-only |
| 6 | Backward closure | fixed point at target; no earlier transaction added |
| 7 | Forward closure | fixed point at target; no later transaction added |
| 8 | Conservative closure | `[259]`, equal to minimal candidate |
| 9 | Exact transaction count | 1 |
| 10 | Message versions | legacy 0 / V0 1 / v1 0 |
| 11 | Failed transactions | 0 in closure |
| 12 | Account count | 22 transaction accounts |
| 13 | Unique programs | 6 executed programs |
| 14 | Historical binaries | 4 BPF ELFs required: 1 present, 3 missing; historical native runtime also missing |
| 15 | Unique LUTs | 4, all missing at checkpoint A |
| 16 | Runtime requirements | sysvars, feature set, fee/compute rules, signature policy, native identity |
| 17 | Feasibility | D — `HISTORICAL_EVIDENCE_UNAVAILABLE` |
| 18 | Missing generic features | historical LUT/ProgramData acquisition and runtime-profile evidence |
| 19 | Segment manifest | not built; gate was not A |
| 20 | Per-transaction replay | not run |
| 21 | First divergence | none measured; execution never started |
| 22 | Target pre-state | not derived |
| 23 | Target post-state | not derived |
| 24 | CompleteExecutionV2 | not evaluated |
| 25 | Terminal reconciliation | not run |
| 26 | Offline reproduction | analysis/controls reproduce offline; replay proof does not exist |
| 27 | Mutations | replay mutations not run after mandatory stop |
| 28 | Performance | closure/census under one second; no replay benchmark |
| 29 | Storage | 516,790 checkpoint-response bytes; missing evidence size unknown |
| 30 | Provenance | checkpointed boundary would be derived, not observed |
| 31 | Fidelity profile | keep CompleteExecutionV2 unchanged; consider `CheckpointedExecutionV1` after U8-F |
| 32 | Protocol-specific core branches | 0 |
| 33 | Prospective capture | recommended default from onboarding onward |
| 34 | Claims justified | bounded single-V0 closure and current evidence gaps |
| 35 | Claims prohibited | no exact boundary, replay, reconciliation, or production primitive |
| 36 | Next phase | bounded generic LUT/ProgramData/runtime acquisition, then re-gate |

## 1–4. Repository, controls, conflicts, and committed effects

HEAD remains `84ba410287a4a7ed315a7d779dbaad625fe3bf39` on `main`. The inherited dirty U4–U7 worktree was preserved; U8 changed no replay core, executor, protocol adapter, corpus, bundle, or CI product source. The [initial state](examples/phase-u8-freeze/initial.json), [final state](examples/phase-u8-freeze/final-state.json), and [prior-artifact check](examples/phase-u8-freeze/prior-artifact-check.json) record this. All 94 files in the U7 manifest rehashed unchanged.

Fresh Stake Pool controls reproduced exits `0/0/1/1/3/5`. Fresh Kamino bundle verification and baseline CI passed in ordinary and `PATH`-only environments with the frozen stdout hashes `529355d…` and `b7f3395…`; see the [Stake Pool](examples/phase-u8-freeze/stake-pool-controls/controls.json) and [Kamino](examples/phase-u8-freeze/kamino-controls.json) results.

The old U6 166-transaction slice was an intentionally conservative writable-overlap upper bound, not the actual closure. The corrected result is:

| Relationship to target | Transactions | Result | Committed effect on target validation outputs |
|---|---:|---|---|
| Earlier writable overlap | 0 | none | none |
| Later successful overlap | 2 (`819`, `846`) | success | only the read-only target input `So111…` was declared writable; actual effect remains unknown and is irrelevant to target-output reconciliation |
| Later failed overlap | 6 (`385`, `857`, `860`, `861`, `896`, `1114`) | `InstructionError/Custom(7)` | normal execution changes rolled back; no overlapped target output is the fee payer or a durable nonce |

This classification follows pinned Agave semantics. `RollbackAccounts` preserves the fee-subtracted payer and, where applicable, the advanced durable nonce; unsuccessful ordinary instruction state is not committed. The exact source revision and hashes are retained in [source provenance](examples/phase-u8-freeze/source/provenance.json), including Agave [`transaction_processor.rs`](https://github.com/anza-xyz/agave/blob/965aee8e55d45ac3ca72e48f15945bfca4a32804/svm/src/transaction_processor.rs) and [`rollback_accounts.rs`](https://github.com/anza-xyz/agave/blob/965aee8e55d45ac3ca72e48f15945bfca4a32804/svm/src/rollback_accounts.rs). Provider stream absence is never used to prove absence of a write.

## 5–11. Reconstruction target and closure

The target has **22 execution inputs**: 12 writable validation outputs and 10 read-only input-only accounts. They include the authority/payer, two transient/nonce-related system accounts, Whirlpool, both vaults, three tick arrays, oracle, both mints, token programs, the associated-token program, system/compute programs, memo, and RecentBlockhashes. Checkpoint requests exist for all 22 at both S-1 and S; 20 values are present and two responses are null. `8gb…` is consistently absent at both boundaries. The RecentBlockhashes null cannot stand in for its runtime image and therefore remains an evidence gap.

The target itself advances durable nonce account `HCy…` as its first instruction. This makes the missing historical RecentBlockhashes/runtime behavior material rather than theoretical.

The backward closure reaches a fixed point immediately: no transaction before index 259 declares any of the 22 target inputs writable. The forward validation closure also reaches a fixed point immediately: no successful later transaction writes one of the 12 validation outputs, and the six failed overlapping transactions can only persist payer/nonce rollback accounts that are not those overlapped outputs.

`MinimalCandidateClosure` and `ConservativeSafeClosure` are therefore both `[259]`. Closure growth is explicitly recorded at iteration zero in the closure artifact. The exact distribution is **1 transaction: 0 legacy, 1 V0, 0 version-1, 0 failed**. The slot as a whole contains 839 legacy, 234 V0, and 58 version-1 transactions, but those unrelated messages do not enter the proof.

## 12–18. Acquisition census and feasibility

The closure requires 22 transaction accounts, four transaction-specific LUT proofs, six executed programs, and one historical runtime profile.

The six executed programs are:

- native System and Compute Budget programs;
- legacy-BPF Associated Token program, whose 105,032-byte ELF is retained with SHA-256 `6804554e…`;
- upgradeable Whirlpool, SPL Token, and Token-2022 programs.

The three upgradeable program accounts identify ProgramData addresses `CtXf…`, `3gvY…`, and `DoU5…`, but the historical ProgramData accounts and ELFs were not captured. Thus four BPF ELF identities are required, only one is present, and three are missing. The two native programs additionally require the pinned historical native/runtime implementation.

All **four** LUT snapshots are absent from checkpoint A. The validator's `loadedAddresses` field corroborates resolution but is not a historical LUT proof and is not promoted into one.

Runtime requirements include canonical ordering and blockhash, RecentBlockhashes, Clock, Rent, EpochSchedule, Instructions synthesis, active features, fee/rent/compute rules, signature policy, and native program identity. Only ordering, the block transaction, and its signature/blockhash evidence are presently frozen sufficiently. The two checkpoint response sets total **516,790 bytes** (258,395 bytes each); this measures retained target-account responses, not the unknown size of the missing LUTs, ELFs, or runtime package.

The gate is consequently **D**, not A. Missing generic features/evidence are historical LUT acquisition, upgradeable ProgramData acquisition, a content-addressed historical runtime/profile contract, and explicit sysvar/native-program reconstruction. A per-write RPC is not among the missing requirements.

## 19–29. Work deliberately not performed after the gate

No experimental segment manifest was built, because Stage 16 is conditional on feasibility A. No generic segment executor or product schema was added. Consequently:

- there are no per-transaction replay results or replay divergence;
- target pre-state and post-state were not derived;
- `CompleteExecutionV2` was not evaluated for this target;
- terminal S reconciliation did not run;
- offline proof reproduction and the 20 replay mutations did not run;
- replay execution time and per-transaction scaling were not measured;
- no 10/100/1000-transaction cost extrapolation is claimed.

The only timing observation is that closure/census derivation completes locally in well under one second on this checkout; it is not an execution benchmark. Storage measured in this phase is limited to the retained 516,790 checkpoint-response bytes, the 105,032-byte available ELF, and the content-addressed analysis/freeze artifacts. Missing evidence size is unknown.

The proof-provenance hierarchy remains:

1. `ObservedTransactionBoundary`: complete direct per-write observations.
2. `CheckpointedReplayBoundary`: derived execution state anchored and reconciled between independent observed checkpoints.
3. `SlotBoundaryOnly`: insufficient for transaction-level fidelity under same-slot contention.

Checkpointed evidence must remain visibly derived. `CompleteExecutionV2` must not silently accept it. Until an offline U8-F proof establishes equivalence requirements, a distinct prospective profile such as `CheckpointedExecutionV1` is the honest integration direction.

## 30–36. Generality, claims, and next phase

There are **zero new protocol-specific branches in core code**. The only Orca/Whirlpool knowledge is in the frozen adversarial evidence and human-readable roles; closure derivation operates on resolved keys, writability, transaction order, outcomes, payer/nonce rules, loaders, LUTs, and runtime dependencies.

For long-term operation, Eplyx should default to prospective capture from protocol onboarding: transactions, program upgrades/ProgramData, LUT versions, runtime/feature checkpoints, and periodic account checkpoints or writes. Complete direct per-write capture is the strongest boundary provenance when demonstrably complete. Checkpointed deterministic replay is the generic historical fallback. Slot-boundary-only evidence remains a qualification signal, not a transaction boundary.

Exact claims justified:

- the target's sound structural closure is one successful V0 transaction;
- no earlier transaction writes a target input;
- no successful later transaction writes a target validation output;
- the six later failed output overlaps cannot commit their ordinary target-account mutations under the pinned rollback semantics;
- existing Stake Pool and Kamino controls are unchanged;
- the current retained evidence is insufficient for deterministic execution;
- no Orca-specific replay logic is necessary or permitted by this result.

Exact claims prohibited:

- exact derived target pre-state or post-state;
- a successful Orca replay or `CompleteExecutionV2` match;
- exact terminal checkpoint reconciliation;
- a production `CheckpointedReplay` primitive;
- offline reproducibility or mutation resistance;
- completeness of Alchemy's per-write stream;
- substituting current LUTs, binaries, sysvars, or native runtime for historical evidence.

The next phase should be a **bounded generic evidence-acquisition phase**, not an Orca implementation and not purchase of a per-write RPC. Acquire the four LUT accounts and three ProgramData accounts at checkpoint A, identify and freeze the historical Agave/runtime feature profile and required sysvars, then rerun the same feasibility gate. Only an A result should authorize a one-transaction experimental manifest and generic executor. No commit was created; the suggested “Prove checkpointed transaction-boundary replay” commit would be false at U8-A.
