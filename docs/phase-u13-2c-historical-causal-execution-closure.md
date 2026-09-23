# Phase U13.2C — historical causal execution closure

## Result: U13.2C-A, sequence reconciles

The frozen U13.1 closure is still exactly `73 → 428 → 431 → 438 → 1245`. Its 11 account-labelled edges, 24-identity parent frontier, two LUTs, and nine terminal accounts pass the existing checksum, frontier, and minimality audits. All five validator transactions succeeded. The isolated [sequence feasibility runner](../engine/examples/u13_2c_sequence.rs) constructs one LiteSVM world from the parent-slot frontier and the U13.2B evidence-bound historical feature profile. It executes all five native v0 messages in that world, without reseeding between transactions. Each transaction matches validator success, fee, CU, ordered logs, zero executed inner instructions, and empty return payload. All nine terminal `AccountSnapshot` values, including complete raw data, match independent block-final archive receipts. The [machine-readable receipt](examples/phase-u13-2c-sequence/feasibility.json) contains every intermediate frontier hash, envelope, message resolution, terminal comparison, and control result.

The SpotMarket data after tx73 is `edceea1f1e0ef731ebb9903d5651c5e1bee47ca219a4b13b306598a729cf8f85`. It stays byte-identical through the later four transactions in this execution and equals the observed block-final account in full. This is a **derived target boundary supported by terminal reconciliation**, not a directly observed transaction-index account image or a `ReplayObservationV2`. U13.1's later writers were *possible* writers under the conservative writable-declaration policy; the execution indicates that they did not change SpotMarket bytes in this witness. They remain part of the qualified conservative sequence proof.

## Frozen closure and parent world

The [U13.1 closure](examples/phase-u13-1-causal-closure/closure.json) has these exact signatures and edges:

| Index | Signature | Direct dependency from prior closure transaction |
| ---: | --- | --- |
| 73 | `2BD3UJFPUPJbxoJMAntwyZrzLjzKv3yeuQERTu2rPRZLf4H8Wdjihsh6xChwC4Vxruxy9qyrrknmnvi7UJqCHRSK` | Parent checkpoint |
| 428 | `4MLdGz1Gieizn7Pzs9PujFCAKJ1hzw69KErF4AgMZtW6MANi2K5hE1p6uxLZZBdGJgoYGdtsdozfHeeRaLcLHXPB` | 73 on SpotMarket |
| 431 | `47YKrAiJJQsCbABRwhMFy5G1mqFYzSZZjiVKvyxPZEHbCJg3twhQhW46F6sSZFfaRcMrX8kF1489mKqFywuigRJZ` | 428 on four accounts |
| 438 | `3pjwtrCE8NqPrjakAMyrci4ryk6JAZZAN9FEbf5Yqk2JZcpcu8ybu1AHJ6gtLckYAxJ8AdfydAL2tS8RRNcvmbc1` | 431 on two accounts |
| 1245 | `2QFKnUZWaVUnga2LcXS9dX4Miqe1hWXNnyoR1yBNudodtuuzsZ3num9fXadWoXS1mMMnfBpV3iHqTNrfP4Qe8zVk` | 438 on four accounts |

No transaction was included merely for lying between indexes. The parent frontier contains these 24 identities. Twenty-two ordinary accounts have full parent-slot receipts; ProgramData is assembled from the qualified header and full historical ELF; ComputeBudget is native runtime input:

```text
25Eax9W8SA3wpCQFhJEGyHhQ2NDHEshZEDzyMNtthR8D
2Zsxk1jSe5h2uDVVKKRkv5PevhL3w4SEgyeywgFq2vwd
3e5QUcAj1qWHRjtphaKVguitkZx6Rnun6CSCJibnwxZM
3m6i4RFWEDw2Ft4tFHPJtYgmpPe21k56M3FHeWYrgGBz
3x85u7SWkmmr7YQGYhtjARgxwegTLJgkSLRprfXod6rh
5zpq7DvB6UdFFvpmBPspGPNfUGoBRRCE2HHg5u3gxcsN
6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3
7QAtMC3AaAc91W4XuwYXM1Mtffq9h9Z8dTxcJrKRHu1z
7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP
8qc5mTJKvZVXph8jTjXWhZgP7p8CYYL6wAVtibEDZcbL
93FG52TzNKCnMiasV14Ba34BYcHDb9p4zK4GjZnLwqWR
9VCioxmni2gDLv11qufWzT3RDERhQE4iY5Gf7NTfYyAV
AVHUQjWAxUgRWdMrxf9w2CEwqLBwkjjFtzobadSXNRZ7
CXZhzKePYajrZgZyrzgvHYFKK3c5tNgDrRobAgySo8Nb
ComputeBudget111111111111111111111111111111
EiWSskK5HXnBTptiS5DH6gpAJRVNQ3cAhTKBGaiaysAb
EzxyKMKL6W1MLBDJumJTpARKwf1MXyy6DcebUcWkHjaa
Fpys8GRa5RBWfyeN7AaDUwFGD1zkDCA4z3t4CJLV8dfL
GXWqPpjQpdz7KZw9p7f5PX2eGxHAhvpNXiviFkAB8zXg
HN7qfUNM5Q7gQTwyEucmYdCF4CjwUrspj3DbNQ4V8P52
HpR1bLcW6rsXrBigRhW18WnQNwAVBq7wBPhteRKGBU5z
JE9m89yHHiCGzzL2FAeeZgHKAFwjkW4Qp1GfjegWnojR
dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH
maCYbwXrJDnnP5ft3ySoH4ogwv5yFph8fFacgMa51de
```

The U13.1 full-block census has no committed writer of the Drift Program, ProgramData, or either LUT in this prefix. All five transactions share slot `409942000`. Thus one historical feature/runtime profile and one historical Drift deployment apply. Fee payers `2Zsx…vwd` and `AVHU…NZ7` each occur twice; the single VM carries their fee debit forward. No failed transaction enters the closure, and U13.1 found no failed overlapping frontier writer between the target and final closure transaction. U11.2 rollback semantics therefore require no special case here.

## Message and executable resolution

Each message is independently parsed from the SHA-256-pinned source block `e6e3065b6792dbdebe5ace25333af607cb13e57bc03784ca9697bc132a1af304` by `FrozenV0`, then checked against U13.1's index, signature, full resolved key order, LUT order, programs, fee, and success. The universal `reconstruct` and `validate_proof` machinery derives loaded addresses from raw LUT account bytes and checks them against validator metadata. Tx73 has 16 static keys, three instructions, and no LUT. Each later transaction has five static keys, two instructions, and five loaded keys (one writable, four readonly). Its descriptors reference, in order, `Fpys8GRa5RBWfyeN7AaDUwFGD1zkDCA4z3t4CJLV8dfL` and `EiWSskK5HXnBTptiS5DH6gpAJRVNQ3cAhTKBGaiaysAb`.

The two [new exact execution-slot LUT receipts](examples/phase-u13-2c-sequence/lut-acquisition.json) bind raw response SHA-256, slot, owner and bytes. Their complete account images equal the qualified parent-slot LUT receipts. All eight later table uses resolve through the existing generic proof, and a used-address mutation fails loaded-address comparison at proof stage 4. The [message resolutions](examples/phase-u13-2c-sequence/feasibility.json) retain every static key, descriptor/index list, loaded key, full key order, instruction hash/count, program census, and LUT proof ID. The only invoked IDs across all five are the historical upgradeable Drift BPF program and native ComputeBudget. The ProgramData data hash is `ca89b99a4ce9cd09a35e7fd881b09099397543a9a1122e93f9a68b1349007728`; the ELF SHA-256 is `56bb3c1218ca4cc1158116c008439a48ca942f7e2898227494ce2769a7fdce5d`. No additional BPF binary was loaded.

The resolved U13.2B `RuntimeProfile` ID is `82d133c01dfdffd1d28d4115f969e7ecf73178c41829ccdc3b882910c270e639`, with feature inventory ID `e20728870f96f8aa9d7d491acbdd05eba00c7ba486420c57469a3317e1f38da5` and content-bound feature-set hash `9e20e164cd1299081370bf19eacc592f2990ecdd30d1854a9dfd3ff851a0d18c`. The runner uses the same constructor order as U13.2B: feature set before builtins, sysvars, feature accounts, and default programs; then exact Clock, Rent, EpochSchedule, parent seeds, and the historical environment blockhash callback. Signature and recent-blockhash checks are disabled by this profile, as in U13.2B.

## Execution and derived states

| Index | Validator/local result | Validator/local fee | Validator/local CU | Ordered logs | Inner instructions | Return payload | Derived SpotMarket data SHA-256 |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 73 | success | 6,216 | 97,801 | 11 equal | 0 | empty | `edceea1f1e0ef731ebb9903d5651c5e1bee47ca219a4b13b306598a729cf8f85` |
| 428 | success | 5,000 | 28,231 | 6 equal | 0 | empty | same |
| 431 | success | 5,000 | 28,231 | 6 equal | 0 | empty | same |
| 438 | success | 5,000 | 28,231 | 6 equal | 0 | empty | same |
| 1245 | success | 5,000 | 28,231 | 6 equal | 0 | empty | same |

The receipt retains both validator and local values for each envelope and deterministic account/data hashes for all 23 present parent-frontier accounts after **each** transaction. These are **derived historical sequence states**. They are not provider-observed transaction boundaries. LiteSVM's empty outer inner-instruction groups correspond to zero executed inner instructions and zero validator rows. Validator return data has no row; local return payloads are empty.

## Terminal reconciliation

All nine independently observed terminal accounts match complete snapshots: lamports, owner, executable flag, rent epoch, and raw data. The table gives the common SHA-256 of serialized `AccountSnapshot` for each local/archive pair; the receipt also retains separate actual/expected raw-data hashes and lamports.

| Terminal account | Complete snapshot SHA-256 | Equal |
| --- | --- | --- |
| `2Zsxk1jSe5h2uDVVKKRkv5PevhL3w4SEgyeywgFq2vwd` | `479e0c2ab4d9236aafdc39780ba297f50f6014b6c763b44754ead94e8bd0ab91` | Yes |
| `3e5QUcAj1qWHRjtphaKVguitkZx6Rnun6CSCJibnwxZM` | `b70529712337eca7b09e917dd3d1efd8f098e833fb9b9e8d0eac13552f8bce8c` | Yes |
| `6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3` | `c45057de43e48f747f2a9b11952396762012ccd7eb5bceb2067dc33e1c71b60f` | Yes |
| `7QAtMC3AaAc91W4XuwYXM1Mtffq9h9Z8dTxcJrKRHu1z` | `e867b68a9ccd69731ec6afa574d6ec178e9e04ed2d3379318e102faf783000d4` | Yes |
| `8qc5mTJKvZVXph8jTjXWhZgP7p8CYYL6wAVtibEDZcbL` | `d4b06ee08f1b5261fe76b9eff83df077d6c030eb4156123d668661caa5264ace` | Yes |
| `AVHUQjWAxUgRWdMrxf9w2CEwqLBwkjjFtzobadSXNRZ7` | `0e84b568d318787788d43bebaedf6fb2dd772f2c6f58884743aadd7e2d131c05` | Yes |
| `EzxyKMKL6W1MLBDJumJTpARKwf1MXyy6DcebUcWkHjaa` | `14394261bf19d457a421a0d557e5b837466b035f31818ca2e4c1ea8b48dd89a5` | Yes |
| `JE9m89yHHiCGzzL2FAeeZgHKAFwjkW4Qp1GfjegWnojR` | `de54d743877da319656d0179ad99eb62df5193f9effdac02b11032391b1a15cb` | Yes |
| `maCYbwXrJDnnP5ft3ySoH4ogwv5yFph8fFacgMa51de` | `b417d46a1c235003ffe34a8636258ee3d0daf72945aa4d9e17ba2cc7fc266590` | Yes |

This is three linked claims: **target execution proof** (tx73 envelope and safe User/PerpMarket outputs match), **sequence execution proof** (each later historical transaction envelope matches in the continuing world), and **terminal reconciliation proof** (all nine final accounts equal independent block-final observations). Together they justify the tx73 SpotMarket as a `DerivedTargetBoundary` supported by a generic causal sequence proof, conceptually a `CheckpointedReplayBoundary`. They do not turn it into an `ObservedTransactionBoundary`. The later four transactions are historical baseline proof only. Future candidate comparison must start from the proven tx73 pre-state and execute the candidate **only at tx73**.

## Adversarial controls and limits

| Control | Result |
| --- | --- |
| Omit 428 or 431 | Both leave the shared later fee payer terminal snapshot wrong; included envelopes still match. |
| Omit 438 or 1245 | Both leave the other reused fee payer terminal snapshot wrong; included envelopes still match. |
| Swap 428 and 431 | Execution envelopes and terminal accounts remain equal in this witness, but the order violates four qualified dependency edges. The conservative closure resolver rejects it; behavior alone does not establish order. |
| Change tx428 instruction data | Tx428 envelope diverges; terminal accounts happen to reconcile. This shows why per-transaction comparison is required. |
| Change a LUT address used by tx428 | Universal LUT proof stage 4 rejects the independently resolved loaded keys. |
| Change later-only parent account `3e5Q…wxZM` | Tx428 envelope and that terminal account diverge. |
| Reset SpotMarket to its parent image after tx73 | Terminal SpotMarket complete snapshot diverges, even though later envelopes still match. |
| Substitute LiteSVM current/default feature set | The complete collected execution result is equal across the five transactions. This is a diagnostic equality, not historical authorization: the substituted set lacks the exact-slot active-feature evidence bound by the profile. |

The controls are retained in the receipt and checked by the [offline U13.2C audit](../scripts/check-u13-2c-artifacts.cjs). The default-feature diagnostic cannot be rejected by behavioral comparison on this witness. The reordered pair also commutes behaviorally here; its rejection is based on the independently qualified conservative dependency graph. Neither is presented as a divergent execution fingerprint.

## Assurance, regression, and U13.3 recommendation

U13.1, U13.2A, and U13.2B checksum scripts verify 25, 319, and 53 frozen files. The U13.1 minimality and frontier audits pass 11 cases, 22 parent receipts, nine terminal receipts, eight LUT checks, and 60 balance links. The U13.2C offline audit verifies its five checksummed files, two new LUT receipts, five resolved messages, five envelopes, 23 state hashes per step, nine terminal comparisons, and ten controls. The 28 generic historical-LUT tests and historical-feature mutation test pass. Strict Clippy and workspace formatting pass. A scan of generic execution paths (`engine/src/universal`, `engine/src/replay.rs`, `engine/src/executor.rs`) finds zero Drift name, target program ID, `settlePnl`, or target signature references. Witness-specific IDs occur only in the isolated example and research artifacts. Existing unrelated comments/tests mentioning Drift predate this phase.

The historical feature set is content-bound to exact-slot provider account responses. The historical Drift ELF and ProgramData are provider-observed bytes, without source-to-ELF build proof. The exact target validator binary and native runtime profile are not attested. LiteSVM 0.16.0 and Agave 4.2.2 compatibility is demonstrated by these five matches, not claimed as binary identity or universal equivalence. SlotHashes and RecentBlockhashes remain bounded by U13.2B's target materiality controls; exact historical bytes were not acquired. The new LUT and terminal archive receipts are provider observations, not cryptographic validator attestations. FastRPC F1 remains a separate direct-boundary evidence path.

For U13.3, introduce the smallest **generic auxiliary sequence proof** alongside the existing checkpoint contract. It should bind (1) the independently observed parent frontier and runtime evidence; (2) the frozen source block and exact ordered native messages with universal LUT proofs; (3) historical executable identities and per-transaction validator envelope comparisons; (4) each derived intermediate frontier digest and a distinguished target post-state; and (5) independently observed complete terminal snapshots with exact equality. It should validate the conservative dependency order and distinguish `DerivedTargetBoundary` from a directly observed boundary. Candidate execution should reference the distinguished pre-state and target transaction only. U13.2C implements none of the proposed schema, observation, corpus, bundle, CI, or candidate execution changes.
