# Phase U9 — Generic Historical Execution Input Acquisition

## Decision

Phase U9 reaches **U9-B**. The four historical address lookup tables and all four historical BPF binaries are now proven. The former U8 classification `HISTORICAL_EVIDENCE_UNAVAILABLE` is therefore resolved for the bounded account, LUT, program, ProgramData, ELF, sysvar, feature, fee, and durable-nonce inputs.

The runtime capability decision is nevertheless:

**B — `SUPPORTED_WITH_GENERIC_RUNTIME_CONFIGURATION`**

and the rerun feasibility gate is:

**B — `BOUNDED_BUT_NEEDS_GENERIC_FEATURE`**

Execution was not attempted. The current universal `ExecutionRequest` cannot inject the exact bank/environment blockhash required by `AdvanceNonceAccount`, while the default LiteSVM build exposes `set_latest_blockhash` only behind its `persistence-internal` feature. The current execution contract also does not bind the acquired feature/native evidence to a content-addressed runtime profile. Supplying current/default values would violate the U9 gate.

This result does **not** require an expensive per-write RPC. Alchemy's public documentation endpoint supplied the four LUT accounts, three complete ProgramData accounts, Clock, Rent, and EpochSchedule at exact historical slots. Alchemy documents point-in-time `getAccountInfo` as inclusive state at the requested finalized slot, and explicitly excludes the per-slot `RecentBlockhashes`, `SlotHashes`, and `SlotHistory` sysvars from that archive. Those two required sysvars were recovered independently from the retained U7 Yellowstone slot stream and finalized block chain instead. See the [official Account Archive contract](https://www.alchemy.com/docs/solana/account-archive).

Primary evidence:

- [acquisition manifest](examples/phase-u9-acquisition/acquisition.json)
- [acquisition checksums](examples/phase-u9-acquisition/checksums.json)
- [exact LUT reconstruction](examples/phase-u9-analysis/lut-proof.json)
- [program census](examples/phase-u9-analysis/program-census.json)
- [historical/default ELF comparison](examples/phase-u9-analysis/backend-default-binary-comparison.json)
- [runtime evidence](examples/phase-u9-analysis/runtime-evidence.json)
- [experimental runtime profile](examples/phase-u9-analysis/runtime-profile.json)
- [runtime capability](examples/phase-u9-analysis/runtime-capability.json)
- [feasibility decision](examples/phase-u9-analysis/feasibility.json)
- [offline qualification](examples/phase-u9-analysis/offline-qualification.json)
- [generality review](examples/phase-u9-analysis/generality-review.json)
- [frozen runtime-source provenance](examples/phase-u9-freeze/runtime-source/provenance.json)
- [final manifest](examples/phase-u9-freeze/final-manifest.json)

## Final-report matrix

| # | Required result | U9 result |
|---:|---|---|
| 1 | Frozen U8 controls | PASS; U8 manifest unchanged, Stake Pool `0/0/1/1/3/5`, Kamino ordinary and PATH-only controls exact |
| 2 | LUT acquisition | 4/4 at S-1 and S; every pair byte-identical |
| 3 | Loaded-address reconstruction | exact 7 writable, 5 readonly, and complete 22-key vector |
| 4 | Program census | 6: 2 runtime-native, 1 legacy-BPF, 3 upgradeable-BPF |
| 5 | Program-account evidence | all six historical S-1 accounts present; CPI-derived program set included |
| 6 | ProgramData evidence | 3/3 complete, exact historical headers and content |
| 7 | ELF hashes | 4/4 proven from historical account bytes |
| 8 | Source/build provenance | Cargo.lock and six crate source packages frozen; exact historical validator build not claimed |
| 9 | Native runtime evidence | target-reachable 4.2-line System/Compute behavior strongly corroborated and source-bound |
| 10 | Feature-set evidence | 311 feature accounts observed; 309 target-active, 0 activated after target; 15 target-reachable features classified |
| 11 | Durable nonce | exact A account, authority, stored nonce, fee field, and derived B nonce proven |
| 12 | RecentBlockhashes | archive null expected; exact 150-entry pre-state reconstructed from stream plus finalized chain |
| 13 | Clock | exact S account; slot/time reconcile to block |
| 14 | Rent | exact S account retained and decoded |
| 15 | EpochSchedule | exact S account retained and decoded |
| 16 | Instructions | generated from complete original V0 message; no fake account |
| 17 | SlotHashes | exact streamed image retained; non-material for four active LUTs |
| 18 | Signature policy | verification disabled; original signature retained |
| 19 | Blockhash policy | inclusion check disabled; original recent blockhash retained; exact environment blockhash required |
| 20 | Fee model | 5,000 base + 750 priority = exact validator fee 5,750 |
| 21 | Compute budget | original limit 150,000 and price 5,000 µ-lamports/CU retained |
| 22 | Runtime capability | B — `SUPPORTED_WITH_GENERIC_RUNTIME_CONFIGURATION` |
| 23 | Rerun feasibility | B — `BOUNDED_BUT_NEEDS_GENERIC_FEATURE` |
| 24 | Missing evidence if stopped | no bounded historical account/binary gap; generic runtime binding/injection capability is missing |
| 25 | Experimental replay manifest | not built; gate was not A |
| 26 | Local execution | not attempted |
| 27 | Target pre-state | structurally checkpoint A by U8 proof, but not promoted into an execution result |
| 28 | Derived target post-state | not derived |
| 29 | 12-account S reconciliation | not run |
| 30 | Validator outcome reconciliation | not run |
| 31 | Offline result | acquisition/qualification reproduce offline; replay proof does not exist |
| 32 | Mutations | not run because execution was not reached |
| 33 | Storage | 40,180,166 acquisition bytes on disk; 12,081,363 historical ELF bytes; 17,248 LUT bytes |
| 34 | Performance | deterministic qualification about 0.4 seconds; no replay benchmark |
| 35 | Protocol-specific core branches | 0 new branches; no core/product source changed in U9 |
| 36 | Proof-profile recommendation | do not productize; reconsider `CheckpointedExecutionV1` only after U9-G-equivalent proof |
| 37 | Prospective capture | retain LUT, ProgramData, runtime/feature, per-slot sysvar, block, and account-checkpoint evidence from onboarding |
| 38 | Exact claims justified | all bounded data inputs acquired; runtime gap isolated to a generic configuration contract |
| 39 | Exact claims prohibited | no successful replay, derived boundary, S reconciliation, offline replay, or production primitive |
| 40 | Next phase | add bounded generic runtime-profile binding and exact environment-blockhash injection, then re-gate |

## 1. Frozen repository and controls

HEAD remains `84ba410287a4a7ed315a7d779dbaad625fe3bf39` on `main`. The inherited U4–U8 dirty worktree was preserved. U9 added acquisition, qualification, frozen evidence, and this report only; it changed no executor, replay core, protocol adapter, product schema, corpus, bundle, or CI source. The [initial state](examples/phase-u9-freeze/initial.json), [final state](examples/phase-u9-freeze/final-state.json), and [U8 artifact check](examples/phase-u9-freeze/prior-artifact-check.json) record that boundary.

All U8 final-manifest entries rehashed unchanged. Fresh Stake Pool controls returned `0/0/1/1/3/5` with their preserved U2 bytes. Fresh Kamino verification and baseline CI passed in both ordinary and credential-free PATH-only environments. The frozen output hashes remained `529355d96e95…` for verification and `b7f33953ca35…` for baseline CI; see [controls.json](examples/phase-u9-freeze/controls.json).

## 2–3. Historical LUTs and exact V0 keys

Each table was requested at checkpoint A (`448760957`) and again at S (`448760958`). S-1 is the pre-slot anchor; the S copy supplies the execution-slot observation expected by the existing U4 reconstruction. Every pair has identical raw account data, so no lookup-table extension or metadata transition occurred across the target slot.

All accounts are owned by `AddressLookupTab1e1111111111111111111111111`, are non-executable, and have `deactivation_slot = u64::MAX`. Consequently all four are active. Their full address vectors, lamports, rent epoch, raw response hashes, and evidence identities are retained in the acquisition and LUT-proof artifacts.

| LUT | Bytes / addresses | Last extended slot / start | Authority | Data SHA-256 |
|---|---:|---:|---|---|
| `8bnW…AqEd` | 888 / 26 | 385149620 / 0 | `Fp4Z…pYWj` | `30e85f35a129…` |
| `GqBz…dNfZ` | 2,552 / 78 | 447632055 / 58 | `Hup7…9A9e` | `d38bc2fb7d21…` |
| `8qfV…PA3j` | 7,608 / 236 | 437321252 / 0 | `9RAu…Cr1g` | `4c36161a4bd3…` |
| `FZSQ…YzVX` | 6,200 / 192 | 446076472 / 190 | `C5EM…Jyqw` | `b41a277e4290…` |

The existing generic U4 resolver parsed the raw accounts and independently produced seven loaded writable addresses, five loaded readonly addresses, and the complete 22-key message vector. Each sequence is exactly equal, including order, to the frozen finalized validator transaction. Validator `loadedAddresses` was used only as the comparison target, never as the LUT proof. Proof ID: `dde611dac39b8a2336414511fed62d6162b5192dd2efd86b702f5cb1d914d00c`.

## 4–8. Program census, ProgramData, and ELF identity

The executed-program set was re-derived from every top-level instruction and every retained inner instruction/CPI program index. It contains exactly:

| Program | Classification | Historical binary result |
|---|---|---|
| System | runtime-native | runtime profile required |
| Compute Budget | runtime-native | runtime profile required |
| Associated Token | legacy-BPF | 105,032 bytes, `6804554e69fd…` |
| SPL Token | upgradeable-BPF | 108,600-byte ELF, `8190d3f7ceb6…` |
| Token-2022 | upgradeable-BPF | 1,382,016-byte ELF, `0999dbf70897…` |
| Whirlpool | upgradeable-BPF | 10,485,715-byte ELF, `610b1e394973…` |

The historical S-1 executable accounts are present in the retained U7 archive. Their owners and account bytes prove the loader classifications. Each upgradeable program header points to the ProgramData address acquired below; no current pointer was substituted.

| ProgramData | Program | Allocated bytes | Deploy slot | Upgrade authority | Full ELF SHA-256 |
|---|---|---:|---:|---|---|
| `3gvY…fFk2` | SPL Token | 108,645 | 419472000 | none | `8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697` |
| `DoU5…HDTY` | Token-2022 | 1,382,061 | 427147035 | `AeLm…twgz` | `0999dbf708971e723b08d1caafc988826a59c6001ed6dc02260da07defbe1469` |
| `CtXf…N8nD` | Whirlpool | 10,485,760 | 440170207 | `GwH3…x4PV` | `610b1e394973bd1b86de8afc983524bd21dea177e87bfae75ec289d4c350f9ae` |

ProgramData was acquired at S-1 in exact, contiguous slices of at most 1 MiB. Every retained proof chunk binds the address, requested and returned slot, offset, length, decoded-content hash, and raw-response hash. A 45-byte header query at S proves no within-slot ProgramData header change. Reassembly verifies the complete account size, header identity, ProgramData discriminant, deployment metadata, authority option, ELF magic, and final ELF hash.

The backend-default comparison is material. LiteSVM's active p-token default (`53e587eee80c…`, 100,312 bytes) does not equal the historical SPL Token ELF. Its Token-2022 default (`495e9d7680dd…`, 615,936 bytes) also differs. Those defaults must be replaced by the captured historical binaries if execution is later authorized. The Associated Token default does hash exactly to the historical 105,032-byte account.

Cargo.lock pins LiteSVM `0.16.0`, the relevant Agave/Solana runtime crates to `4.2.2`, and `solana-fee-structure` to `3.0.0`. U9 froze six package source sets: LiteSVM commit `384675bd…`, the relevant Agave packages at `c9c6f328…`, and fee-structure commit `9d796259…`. They include feature, System, Compute Budget, fee, Instructions construction, loader-default, and embedded ELF sources. These establish the backend implementation used for qualification; they do not prove that the historical validator ran an identical binary.

## 9–12. Runtime, features, nonce, and RecentBlockhashes

The current provider observation at slot `449039760` reported `solana-core 4.2.2` and feature-set `565236538`. Together with Cargo.lock, the complete feature-account census, frozen source, and exact observed transaction behavior, this strongly corroborates the target-reachable 4.2 runtime family. It is deliberately not reported as an exact historical validator build identity.

The feature snapshot contains 311 accounts: 309 activate no later than the target, two are inactive, and zero activate after it. U9 source-bound and cross-checked 15 reachable features. Nine are material: V0 messages; durable-nonce enablement, separation, authorization, advanceability, and RecentBlockhashes fix; compute-unit price; p-token replacement; and prefunded account creation. Six limit/rent/VM/ProgramData guards are classified non-material or defensive for this path. Every named activation slot matches the on-chain account. No target-reachable feature remains `UNKNOWN`; the complete list and provenance are in [runtime-evidence.json](examples/phase-u9-analysis/runtime-evidence.json).

The durable nonce path is exact:

- checkpoint A account `HCyyt…Wd94` is initialized version 1, authority `H6Pj…1rAc`, nonce `3YvB…CHBw`, and 5,000 lamports/signature;
- the stored A nonce equals the original transaction recent blockhash;
- frozen System source requires a non-empty, correctly identified RecentBlockhashes sysvar, then derives the next durable nonce from `environment_config.blockhash`;
- the correct environment blockhash is parent blockhash `A2uV…DjZi`;
- `SHA256("DURABLE_NONCE" || parent_blockhash)` encodes to `GU8h…7cizE`, exactly the nonce stored at checkpoint B.

Alchemy's archive returns null for RecentBlockhashes by documented contract, not because the runtime image was absent. The U7 Yellowstone stream contains the exact 6,008-byte post-slot account event: 150 entries, SHA-256 `5b6e7f56eb69…`. Its first entry is the S blockhash and its second is the parent blockhash. Removing the first, shifting the remaining 149 entries, and appending the independently fetched predecessor of the tail block reconstructs the exact 150-entry pre-transaction serialization. The result is retained as `RecentBlockhashes.pre-transaction.bin`, SHA-256 `8117b81ee222…`. No empty/default sysvar was used.

## 13–21. Remaining sysvars and execution policy

The exact target-slot accounts decode as follows:

| Sysvar | Key values | Data SHA-256 |
|---|---|---|
| Clock | slot 448760958; epoch 1038; Unix time 1789915100 | `022b67aa3ab5…` |
| Rent | 5,080 lamports/byte-year; threshold 1.0; burn 50% | `e6833309a1b7…` |
| EpochSchedule | 432,000 slots/epoch; no warmup | `bc8e8d833121…` |

Clock's slot and Unix timestamp exactly match the retained finalized block. Instructions is not downloaded: it must be generated by the existing universal runtime path from the complete, proven original V0 message.

The exact streamed SlotHashes image is also retained: 20,488 bytes, 512 entries, head slot `448760957`, SHA-256 `230d859ac6fa…`. Each LUT has `deactivation_slot = u64::MAX`; the active-table resolution path returns `Activated` without SlotHashes membership. SlotHashes is therefore non-material for this transaction only. That conclusion is not generalized to deactivating tables.

The intended replay policy disables signature verification and recent-blockhash inclusion checking while retaining the original signature, message, and recent blockhash. This prevents liveness checks from rejecting historical data without changing transaction identity. It does not eliminate the separate requirement to set the exact environment blockhash used by the native nonce transition.

The fee is independently reconstructed: one 5,000-lamport signature plus `ceil(150000 × 5000 / 1,000,000) = 750` priority lamports gives 5,750, exactly the validator fee. The original Compute Budget instructions remain in place: discriminator 2 sets 150,000 units and discriminator 3 sets 5,000 micro-lamports per unit. The source-bound reachable feature state contains no unknown compute-budget or fee behavior for this request.

## 22–24. Capability and feasibility gates

The experimental runtime profile is content-addressed as `eb8ab40820255573c0b2d4534783eda74b1525056afd34696052be9cde734852`. It contains only the target-reachable runtime family evidence, feature activations, native program identities, sysvar treatments, nonce policy, fee and compute-budget policies, signature policy, blockhash policy, and SlotHashes decision. It contains no trusted `historically_equivalent=true` shortcut.

The profile cannot yet be expressed by the current generic execution request. LiteSVM 0.16.0 stores the environment/latest blockhash internally; its setter is compiled only with `persistence-internal`, while Eplyx currently uses LiteSVM's default feature set. Disabling blockhash inclusion checking does not set that value. Executing now would advance the nonce to a backend-generated value and make any later post-state comparison invalid by construction.

Accordingly:

- U9-A: **PASS** — four historical LUTs proven;
- U9-B: **PASS** — all historical BPF binaries proven;
- U9-C: **STOP** — the current backend contract cannot represent the proven runtime profile;
- runtime capability: **B — `SUPPORTED_WITH_GENERIC_RUNTIME_CONFIGURATION`**;
- U8 acquisition gate: **B — `BOUNDED_BUT_NEEDS_GENERIC_FEATURE`**.

This is no longer `HISTORICAL_EVIDENCE_UNAVAILABLE`: the bounded historical inputs exist. The missing piece is a generic, reviewable runtime configuration surface that binds the profile and injects the exact environment blockhash independently of inclusion checking.

## 25–34. Work stopped by the gate

Because only feasibility A may continue, U9 did not build the one-transaction replay manifest and did not execute the target. Consequently no target post-state, 12-account checkpoint-S reconciliation, validator logs/CPI/return-data reconciliation, offline replay result, or 20-case execution mutation campaign exists. U8's structural proof still says target pre-state equals checkpoint A for the relevant accounts, but U9 does not promote that proposition into an execution artifact.

The acquired directory occupies 40,180,166 bytes and contains 55 files including 36 bound raw responses, assembled accounts/ELFs, and checksum metadata. Fifty-one superseded small-chunk retry artifacts were removed before the final checksum set. The authoritative four LUT accounts total 17,248 bytes. The three ProgramData accounts total 11,976,466 bytes; together with Associated Token, the four historical ELFs total 12,081,363 bytes. Frozen qualification/runtime-source evidence adds about 1.6 MB. The deterministic analysis itself runs in roughly 0.4 seconds on this checkout; cached acquisition timings are not used as a service-performance claim. No execution benchmark or 10/100/1000 transaction extrapolation is made.

## 35–40. Generality, claims, and next phase

U9 adds zero protocol-specific branches to core/product code because it changes no core/product code. The acquisition script contains the frozen target's program IDs as bounded evidence selectors; that is test evidence, not execution behavior. No CLMM math, pool layout, tick semantics, swap semantics, or program-ID dispatch was introduced.

Prospective capture would make this acquisition routine: retain LUT versions, upgradeable program/ProgramData versions and ELF content, feature/runtime checkpoints, per-slot sysvars or the block data needed to reconstruct them, and periodic account checkpoints from onboarding. A complete per-write provider remains useful when available, but is not required by this result.

Exact claims justified:

- all four historical LUT accounts are present at S-1 and S and are byte-identical across the slot;
- generic reconstruction exactly matches all loaded and complete V0 keys;
- the complete historical ELF is proven for every executed BPF program;
- System/Compute are correctly treated as native runtime dependencies, not fake ELFs;
- the target-reachable feature, sysvar, fee, compute-budget, and durable-nonce inputs have been bounded and retained;
- the exact expected durable-nonce transition is independently proven against checkpoint B;
- no paid per-write RPC is required for this target's remaining historical-input acquisition;
- the only current stop is a generic runtime-profile/configuration capability.

Exact claims prohibited:

- that the exact historical validator binary/version is known;
- that current LiteSVM defaults are historically equivalent;
- that the default SPL Token or Token-2022 binaries may replace the historical ELFs;
- that the target transaction executed locally or matched validator behavior;
- that a derived target post-state exists or matches the 12 S outputs;
- that replay reproduces offline or survives the mutation campaign;
- that `CompleteExecutionV2` or a production `CheckpointedReplay` primitive is justified;
- that this proves all transactions or all Solana runtime profiles are supported.

The next phase should be narrowly generic: add a content-addressed runtime-profile binding to the universal experimental request and an exact environment/latest-blockhash injection path, with explicit RecentBlockhashes seeding. Then rerun the U9-C and U8 acquisition gates. Only an A result may authorize the single-transaction manifest, execution, exact S reconciliation, offline proof, and mutation campaign. No commit was created; the suggested `Prove generic checkpointed Solana replay` commit would be false before U9-G.
