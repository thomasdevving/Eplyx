# Phase U13.1 — causal multi-transaction closure for Drift `settlePnl`

## Result: U13.1-A, bounded conservative closure frontier

The frozen finalized block yields a five-transaction **possible-write closure** for the U13 target at index 73 and its contaminated quote SpotMarket output. The required execution order is `73 → 428 → 431 → 438 → 1245`: **5 of 1,382 transactions (0.3618%)**. All five succeeded. A full-block scan finds no earlier possible writer of a parent seed, no omitted possible writer needed between these transactions, and no later possible writer of the nine terminal outputs. The [machine-readable closure](examples/phase-u13-1-causal-closure/closure.json) records all resolved message keys, implicit LUT/ProgramData inputs, possible committed writes, per-input predecessor, reasons, and edges. The [offline planner](../scripts/analyze-u13-1-causal-closure.cjs) recomputes it from the frozen U13 block.

“Possible write” is deliberate: a successful transaction's writable declaration is conservatively treated as a committed-state dependency even if it may have left a particular account byte-identical. The archive has no transaction-index raw account images to prove a no-op. Thus the five transactions are indispensable **to this sound conservative proof**, while the evidence does not establish that every one changed SpotMarket bytes. This phase proves the graph and obtainable boundary evidence, not historical execution fidelity. No `ReplayObservationV2`, corpus, bundle, candidate result, or Drift semantic interpretation was created.

| Index | Signature | Why it is included | Direct predecessor |
| ---: | --- | --- | ---: |
| 73 | `2BD3UJFPUPJbxoJMAntwyZrzLjzKv3yeuQERTu2rPRZLf4H8Wdjihsh6xChwC4Vxruxy9qyrrknmnvi7UJqCHRSK` | Distinguished `settlePnl` target and first possible SpotMarket writer | Parent checkpoint |
| 428 | `4MLdGz1Gieizn7Pzs9PujFCAKJ1hzw69KErF4AgMZtW6MANi2K5hE1p6uxLZZBdGJgoYGdtsdozfHeeRaLcLHXPB` | Next successful possible SpotMarket writer; required pre-state for 431 | 73 on SpotMarket |
| 431 | `47YKrAiJJQsCbABRwhMFy5G1mqFYzSZZjiVKvyxPZEHbCJg3twhQhW46F6sSZFfaRcMrX8kF1489mKqFywuigRJZ` | Next possible writer; reads SpotMarket, a prior writable account, and the reused fee payer | 428 on four accounts |
| 438 | `3pjwtrCE8NqPrjakAMyrci4ryk6JAZZAN9FEbf5Yqk2JZcpcu8ybu1AHJ6gtLckYAxJ8AdfydAL2tS8RRNcvmbc1` | Next possible SpotMarket writer | 431 on two accounts |
| 1245 | `2QFKnUZWaVUnga2LcXS9dX4Miqe1hWXNnyoR1yBNudodtuuzsZ3num9fXadWoXS1mMMnfBpV3iHqTNrfP4Qe8zVk` | Last possible writer; its SpotMarket output reaches the block-final checkpoint | 438 on four accounts |

The complete graph has 11 account-labelled edges: `73→428` (1), `428→431` (4), `431→438` (2), and `438→1245` (4). The longest dependency depth is four. The fee payer for 428 and 431 is `2Zsxk1jSe5h2uDVVKKRkv5PevhL3w4SEgyeywgFq2vwd`; 438 and 1245 reuse `AVHUQjWAxUgRWdMrxf9w2CEwqLBwkjjFtzobadSXNRZ7`. Their fee-state dependencies are part of the graph, not inferred from timing. The [artifact](examples/phase-u13-1-causal-closure/closure.json) lists each edge's exact account.

## Start and terminal frontiers

The parent-slot frontier has **24 distinct identities**: the 16 static keys of tx 73 (its 14 instruction inputs plus ComputeBudget and Drift program IDs), five additional ordinary accounts from the later transactions, two address lookup table accounts, and one upgradeable ProgramData account. The exact 14 ordered instruction inputs are retained in the [U13 analysis](examples/phase-u13-drift-witness/analysis.json); the additional identities are:

| Role | Address |
| --- | --- |
| Later fee payer | `2Zsxk1jSe5h2uDVVKKRkv5PevhL3w4SEgyeywgFq2vwd` |
| Later account | `3e5QUcAj1qWHRjtphaKVguitkZx6Rnun6CSCJibnwxZM` |
| Later account | `8qc5mTJKvZVXph8jTjXWhZgP7p8CYYL6wAVtibEDZcbL` |
| Later fee payer | `AVHUQjWAxUgRWdMrxf9w2CEwqLBwkjjFtzobadSXNRZ7` |
| Later account | `EzxyKMKL6W1MLBDJumJTpARKwf1MXyy6DcebUcWkHjaa` |
| LUT | `EiWSskK5HXnBTptiS5DH6gpAJRVNQ3cAhTKBGaiaysAb` |
| LUT | `Fpys8GRa5RBWfyeN7AaDUwFGD1zkDCA4z3t4CJLV8dfL` |
| ProgramData | `7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP` |

The exact [parent frontier](examples/phase-u13-1-causal-closure/closure.json) is fully enumerated by address. Its 22 ordinary account identities have full parent-slot archive receipts: 15 already retained in U13 (14 inputs and the Drift Program), and seven [new exact-slot receipts](examples/phase-u13-1-causal-closure/acquisition.json). The other two identities are the assembled ProgramData account and the native ComputeBudget runtime input. The 64-byte U13 ProgramData header was extended by seven exact parent-slot archive slices to its full **6,673,114-byte** account. Its ELF is **6,673,069 bytes**, SHA-256 `56bb3c1218ca4cc1158116c008439a48ca942f7e2898227494ce2769a7fdce5d`. This is historical binary acquisition, not source-to-ELF build provenance.

The terminal frontier is the nine declared outputs below. Four exact block-final receipts were already in U13; five more were acquired at `context.slot = 409942000` [with raw response hashes](examples/phase-u13-1-causal-closure/acquisition.json).

| Terminal account | Observed parent-to-final change |
| --- | --- |
| `2Zsxk1jSe5h2uDVVKKRkv5PevhL3w4SEgyeywgFq2vwd` | Fee payer lamports |
| `3e5QUcAj1qWHRjtphaKVguitkZx6Rnun6CSCJibnwxZM` | Data bytes |
| `6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3` | Data bytes; final after tx 1245 |
| `7QAtMC3AaAc91W4XuwYXM1Mtffq9h9Z8dTxcJrKRHu1z` | Data bytes |
| `8qc5mTJKvZVXph8jTjXWhZgP7p8CYYL6wAVtibEDZcbL` | Data bytes |
| `AVHUQjWAxUgRWdMrxf9w2CEwqLBwkjjFtzobadSXNRZ7` | Fee payer lamports |
| `EzxyKMKL6W1MLBDJumJTpARKwf1MXyy6DcebUcWkHjaa` | Data bytes |
| `JE9m89yHHiCGzzL2FAeeZgHKAFwjkW4Qp1GfjegWnojR` | Data bytes |
| `maCYbwXrJDnnP5ft3ySoH4ogwv5yFph8fFacgMa51de` | Fee payer lamports |

All nine are present at both boundaries; owner, executable flag, allocated space, and rent epoch are stable at those checkpoints. This rules out a terminal creation, deletion, owner change, or realloc for this output set; it does not claim direct observation of every intermediate state. The [frontier audit](examples/phase-u13-1-causal-closure/frontier-audit.json) rehashes all 20 new raw RPC responses, checks 22 ordinary parent and nine terminal receipts, reconstructs ProgramData against the frozen header, resolves the later transactions' two LUTs in eight checks, and verifies 60 pre/post-balance links. It compares complete raw account hashes, without decoding PnL fields.

## Execution scope and runtime

All five messages are native v0. Tx 73 has no LUT; each later transaction references the same two LUTs. The LUT account bytes at the parent slot resolve exactly to the validator metadata's loaded writable and readonly addresses. The complete block has **96 failed transactions**, but zero failed transaction declaring an overlapping frontier key writable between tx 73 and 1245; none enters this closure. The planner still implements the established generic rule that a failed transaction can persist fee-payer and first-instruction durable-nonce changes. All five included transactions have zero inner-instruction groups and zero pre/post token-balance rows. The only invoked program IDs are upgradeable Drift (one historical BPF ELF) and native ComputeBudget (one invoked native program). Loader, fee handling, LUT resolution, sysvars, and feature set remain runtime dependencies, not additional invoked BPF ELFs.

One historical bank/runtime profile can conceptually cover all five transactions because they share slot `409942000`; the full-block writer census finds no Program, ProgramData, or LUT mutation in their causal prefix. Each transaction still has its own message, fee payer, blockhash/signature policy, compute budget, and `Instructions` sysvar image. Agave's [SVM specification](https://github.com/anza-xyz/agave/blob/master/svm/doc/spec.md) distinguishes the processing environment, including feature set and fee structure, from transaction processing configuration. This supports the proposed separation, but does not establish the historical values for this slot. A slot-specific historical feature/native-program/sysvar profile has **not** yet been resolved or executed for this Drift witness. The target's full historical ELF has now been acquired, but matched replay, historical validator build provenance, and field semantics remain unclaimed. Opaque readonly account inputs are covered by the parent frontier; no oracle valuation is inferred.

## Minimality, proof boundary, and recommendation

The [11 named checks](examples/phase-u13-1-causal-closure/minimality.json) validate the frozen sequence and reject omission of each later writer, censorship of a successful SpotMarket writer, and synthetic omission of an earlier writer to a readonly input. Synthetic cases also keep a failed transaction's unrelated writable account rolled back while retaining fee-payer and durable-nonce dependencies, including reused fee-payer state. This is minimality under the conservative possible-write contract; actual byte-level no-op status remains unknown.

The direct transaction-boundary provider path remains unqualified pending separate FastRPC Yellowstone entitlement ([F1-D](phase-f1-fastrpc-qualification.md)). The parent and terminal archive receipts are provider observations, not cryptographic validator attestations. The closure instead proposes a generic **sequence proof under a new checkpoint proof-contract version or auxiliary proof**, while retaining `CheckpointedExecutionV1`'s meaning: derived target state is accepted only after deterministic historical execution reconciles with observed terminal bytes. Such a proof would bind the observed start account checkpoint and native runtime input, exact five-transaction order and source block, historical binary and runtime dependencies, every intermediate transition, distinguished tx 73 result, and terminal checkpoint. The tx-73 post-state must remain separately addressable even though its SpotMarket image is not directly observed.

Candidate evaluation would begin from the same proven tx-73 pre-state and run **only tx 73** with the candidate target ELF. Transactions 428, 431, 438, and 1245 are historical proof machinery and must use historical binaries; they are not automatically rerun with the candidate binary. This requires more than a trivial schema edit, so U13.1 adds no product proof type, resolver branch, replay semantics, fidelity-profile redesign, or Drift-specific closure type. U13.2 is justified as a bounded generic implementation and replay-validation task. Its remaining work is to establish the historical runtime profile, execute all five historical transactions and match the nine terminal raw accounts plus validator envelope, then expose the distinguished target post-state for candidate comparison. If any transition or checkpoint fails, this closure cannot be promoted to an observation.

No Drift adapter, semantic field decoder, PnL formula, margin or oracle interpretation was added. No old bundle, baseline control, or golden was modified. F1 artifacts remain untouched.
