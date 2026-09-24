# Phase U14 — bounded Drift `settlePnl` semantics

## Result and exact scope

The frozen direct transaction at slot `409942000`, index `73`, signature
`2BD3UJFPUPJbxoJMAntwyZrzLjzKv3yeuQERTu2rPRZLf4H8Wdjihsh6xChwC4Vxruxy9qyrrknmnvi7UJqCHRSK`
settles **+203 quote base units** (`+0.000203` at six decimals) inside Drift.
No SPL or Token-2022 transfer or token-balance metadata exists for tx73. The
quantity is reconstructed from the User quote spot balance using the pinned
Drift fixed-point rule, then checked against independent User and PerpMarket
state deltas. The only public economic subject is
`drift-settle-pnl/settle_pnl/economic/pnl_settled`; the execution subject is
`drift-settle-pnl/settle_pnl/execution/transaction`.

The adapter accepts only this successful native-v0 transaction: the exact
signature, slot, two ComputeBudget companions, direct ten-byte `settle_pnl`
instruction with `marketIndex = 3`, 14 ordered account addresses and observed
access roles, no LUT, no CPI, and empty pre/post token-balance vectors. The
relevant position is User `perpPositions[1]` for market 3, with zero base and LP
shares; quote deposit is `spotPositions[0]` for SpotMarket 0. PerpMarket 3
settles in SpotMarket 0. No other Drift transaction or instruction is admitted.

The [new semantic corpus](examples/phase-u14-drift-semantic-corpus/manifest.json)
is a separate view of the unchanged U13.3 observation and evidence. Its record
is `229293976795dd2ee8eb88a9849004b25fce536d6540f459fc1848efbff63b49`.
The [semantic bundle](examples/phase-u14-drift-semantic-bundle/bundle.json) is
`c519836bf24a58ec84c6f1511510ada5bca42973abdc57e67c407fdb88df0204`.
The original U13.3 corpus and bundle remain frozen.

## State derivation and provenance

The [target-boundary diagnostic](examples/phase-u14-drift-semantics/target-boundary.json)
was extracted from `resolved.seeds` and `pipeline::baseline` of the contract-3
observation. It contains tx73 **PRE and target POST**, not the suffix terminal
frontier. The [field audit](examples/phase-u14-drift-semantics/field-delta.txt)
is reproduced by [the decoder](../scripts/analyze-u14-drift-fields.py) using
the [retained IDL](examples/phase-u13-drift-witness/source/drift.json). The
auditor computes fixed field offsets recursively and asserts exact account
lengths: User 4,376 bytes, PerpMarket 1,216, SpotMarket 776, each including the
eight-byte Anchor discriminator. The adapter also checks owner, discriminator,
market indices, account pubkeys, and SpotMarket vault linkage before evaluating.

All pre/post values below are raw integers. `A` means a directly decoded state
fact. `B` means a derived quantity requiring the pinned formula. A changed
field does not itself establish a distinct public economic subject.

| Account and field (byte offset) | PRE | tx73 target POST | Layout/source and interpretation | Formula for interpretation? |
| --- | ---: | ---: | --- | --- |
| User `spotPositions[0].scaledBalance` (104) | 5,805,893,709,997 | 5,805,893,878,859 | IDL `User`/`SpotPosition`; quote deposit rises 168,862 scaled units (A) | Yes for token amount (B) |
| User `perpPositions[1].quoteAssetAmount` (536) | 203 | 0 | IDL `PerpPosition`; unsettled perp quote falls 203 (A) | No |
| User `perpPositions[1].settledPnl` (576) | 0 | 203 | IDL `PerpPosition`; market-position settled counter rises 203 (A) | No |
| User `settledPerpPnl` (4296) | 6,839,113,323 | 6,839,113,526 | IDL `User`; aggregate settled counter rises 203 (A) | No |
| PerpMarket `amm.historicalOracleData.lastOracleDelay` (88) | 0 | 5 | IDL `AMM`; oracle maintenance (A), excluded from settlement subject | No for raw fact; yes for valuation |
| PerpMarket `amm.historicalOracleData.lastOraclePriceTwap5min` (104) | 893,826 | 893,832 | IDL `AMM`; oracle TWAP maintenance (A) | No for raw fact; yes for valuation |
| PerpMarket `amm.historicalOracleData.lastOraclePriceTwapTs` (112) | 1,774,902,130 | 1,774,902,132 | IDL `AMM`; oracle timestamp maintenance (A) | No |
| PerpMarket `amm.pegMultiplier` (272) | 892,123 | 892,125 | IDL `AMM`; AMM state change (A), economic valuation excluded | Yes for valuation |
| PerpMarket `amm.quoteAssetAmount` (384) | −555,571,737 | −555,571,940 | IDL `AMM`; aggregate perp quote falls 203 (A) | No |
| PerpMarket `amm.totalFeeMinusDistributions` (560) | 242,399,534,598 | 242,399,530,863 | IDL `AMM`; fee accumulator falls 3,735 (A), not attributed to PnL subject | Yes to explain substep |
| PerpMarket `amm.lastUpdateSlot` (768) | 409,941,995 | 409,942,000 | IDL `AMM`; market update (A) | No |
| PerpMarket `amm.lastOracleConfPct` (776) | 387 | 367 | IDL `AMM`; confidence statistic (A) | Yes for risk use |
| PerpMarket `amm.netRevenueSinceLastFunding` (784) | −1,387,887 | −1,391,622 | IDL `AMM`; revenue accumulator falls 3,735 (A), excluded | Yes to explain substep |
| PerpMarket `amm.oracleStd` (880) | 870 | 867 | IDL `AMM`; oracle statistic (A) | Yes for risk use |
| PerpMarket `pnlPool.scaledBalance` (976) | 103,914,036,187,085 | 103,914,036,018,223 | IDL `PoolBalance`; market pool gives up 168,862 scaled units (A) | Yes for token amount (B) |
| PerpMarket `numberOfUsers` (1156) | 18 | 17 | IDL `PerpMarket`; zeroing the quote-only position drops the count (A) | No |
| SpotMarket `revenuePool.scaledBalance` (256) | 1,825,867,503,137 | 1,825,884,005,077 | IDL `SpotMarket`/`PoolBalance`; interest/revenue maintenance adds 16,501,940 scaled units (A) | Yes to value interest |
| SpotMarket `depositBalance` (432) | 99,194,335,756,554,570 | 99,194,335,773,056,510 | IDL `SpotMarket`; aggregate scaled deposits rise by the same 16,501,940 (A) | Yes for token amount |
| SpotMarket `cumulativeDepositInterest` (464) | 12,021,616,178 | 12,021,616,205 | IDL `SpotMarket`; interest index used at settlement (A) | No for raw index; yes in B |
| SpotMarket `cumulativeBorrowInterest` (480) | 13,986,963,686 | 13,986,963,750 | IDL `SpotMarket`; borrow interest maintenance (A) | Yes for borrow valuation |
| SpotMarket `depositTokenTwap` (544) | 119,158,875,253,004 | 119,158,879,361,717 | IDL `SpotMarket`; TWAP maintenance (A) | Yes to reproduce TWAP |
| SpotMarket `borrowTokenTwap` (552) | 66,685,011,219,246 | 66,684,935,588,705 | IDL `SpotMarket`; TWAP maintenance (A) | Yes to reproduce TWAP |
| SpotMarket `lastInterestTs` (568) | 1,774,902,128 | 1,774,902,132 | IDL `SpotMarket`; interest-update timestamp (A) | No |
| SpotMarket `lastTwapTs` (576) | 1,774,902,128 | 1,774,902,132 | IDL `SpotMarket`; TWAP timestamp (A) | No |

The frozen source interface is commit
[`73d22383e621040cd11b94375b6bd2f728f7537e`](https://github.com/velocity-exchange/protocol-v2/tree/73d22383e621040cd11b94375b6bd2f728f7537e).
The retained [entrypoint](examples/phase-u13-drift-witness/source/lib.rs)
and [IDL](examples/phase-u13-drift-witness/source/drift.json) retain their U13
Git blob IDs. The [U14 source snapshot](examples/phase-u14-drift-semantics/source)
adds the exact pinned `controller/pnl.rs`, `controller/amm.rs`,
`controller/position.rs`, `controller/spot_balance.rs`, `math/spot_balance.rs`,
and `math/constants.rs` blobs; their Git SHA-1 IDs are in the semantic binding
report. These are interface and arithmetic references, not a source-to-ELF
build witness.

## Deterministic evaluator

From pinned [`controller/pnl.rs`](examples/phase-u14-drift-semantics/source/controller/pnl.rs),
the entrypoint updates quote spot interest, calls `update_pool_balances`, credits
the User quote spot deposit, debits perp quote, and increments position/User
settled counters. Pinned [`controller/spot_balance.rs`](examples/phase-u14-drift-semantics/source/controller/spot_balance.rs)
uses `get_spot_balance` on a deposit. Pinned [`math/spot_balance.rs`](examples/phase-u14-drift-semantics/source/math/spot_balance.rs)
defines floor division for deposit balances and token amounts; pinned
[`math/constants.rs`](examples/phase-u14-drift-semantics/source/math/constants.rs)
defines quote precision `10^6`, spot balance precision `10^9`, and cumulative
interest precision `10^10`.

For this six-decimal quote market, `D = 10^(19 − 6) = 10^13`. At target POST
interest `I = 12,021,616,205`, the evaluator computes:

```text
token(scaled) = floor(scaled × I / D)                 [deposit]
pnl_settled   = token(User quote scaled POST)
              − token(User quote scaled PRE)         [same target-post I]
scaled_delta  = floor(abs(pnl_settled) × D / I)        [deposit update]
```

The common post interest isolates the settlement from interest that accrued at
the start of tx73. The observed User token equivalents are `6,979,622,590 →
6,979,622,793` quote base units, hence `+203`. The calculated scaled delta is
`168,862`, matching the User increase and PerpMarket PnL pool decrease. The
PerpMarket pool token equivalents under the same `I` are
`124,921,466,135 → 124,921,465,932`, or `−203`.

The evaluator requires all these independent target-boundary relationships:

```text
Δ User spot scaled                 = −Δ PerpMarket pnlPool scaled
Δ User position settledPnl        = Δ User settledPerpPnl = pnl_settled
−Δ User position quoteAssetAmount = −Δ PerpMarket amm.quoteAssetAmount = pnl_settled
Δ SpotMarket depositBalance       = Δ SpotMarket revenuePool scaledBalance
PerpMarket numberOfUsers          = 18 → 17 when quote-only position reaches zero
```

The last two checks constrain this frozen transition; they do not explain or
publish the interest/revenue and AMM maintenance quantities. All arithmetic is
checked integer arithmetic. No oracle, margin, liquidation, AMM pricing, or
funding formula is ported. Logs/events are never inputs to the evaluator.

## Provenance and behavior

The [CI report](examples/phase-u14-drift-validation/ci.json) shows replay
`matched`, proof contract `[3]`, the derived target boundary, the two covered
subjects, zero findings, zero undeclarable changes, and exit `0`. The bundle
verifies offline. Its canonical CI JSON SHA-256 is
`08cb9ff221228e81e5ac27f7342dae9b60b9902e60924a46f5f78d1a062b4ffc1`.
`SemanticBinding` is
`execution_corroborated_external_interface`: the baseline independently
corroborates instruction/signature roles, exact Anchor account types and
lengths, owner and market-index links, SpotMarket vault address, successful
execution, and internally consistent positive settlement. It binds the
historical ELF and target execution hashes separately from pinned source
blobs. `exact_source_to_elf_verified` is **false**.

The [unchanged U13.3 bundle run](examples/phase-u14-drift-validation/no-adapter-ci.json)
still opens as `none@0`, replays with the same matched contract-3 proof, and
exits `2` solely for `no_semantic_coverage`. Its semantic binding is absent.
The new run's JSON is byte-identical to the frozen U13.3 CI JSON (SHA-256
`19d45309e14e529085481a21dba5d47bb03d1b0133105d1811b072960d508b4f`).
The U12.1 Orca semantic CI remains byte-identical at SHA-256
`07ce6389c6b2b0105e1886db6c15648eece8a586c2fdba93f054bc1b5762e04a`.
The semantic corpus/bundle add an optional layer; no universal execution,
resolver, sequence, runtime, evidence, bundle, CI, or replay logic changed.

Controlled target-result mutations show:

| Target candidate | Economic finding | Other result |
| --- | --- | --- |
| +303 quote base units | `pnl_settled/increased`, signed `+0.000303` | internally consistent |
| +103 quote base units | `pnl_settled/decreased`, signed `+0.000103` | internally consistent |
| −203 quote base units | `pnl_settled/decreased`, signed `−0.000203` | sign reversal preserved |
| Reverted target | `execution/transaction/now_reverts` | no economic zero fabricated |
| Truncated or wrong-owner User, PerpMarket, or SpotMarket | withheld | structural account change remains |
| Wrong market index or inconsistent counters/pool | withheld | structural bytes remain |
| Contradictory event/log text | none | state-derived amount unchanged; changed logs remain structural |
| Unknown User byte 4360 | none | outside every decoded range, thus undeclarable |
| +303 and unknown User byte 4360 | `pnl_settled/increased` | byte 4360 stays outside explained ranges |

The [seven Drift integration tests](../engine/tests/drift_settle_semantics.rs)
exercise these mutations, signed serialization, explicit downgrade visibility,
false exact-build provenance, and both ordinary CI bundle paths. The existing contract-3 tests separately
prove that suffix state is excluded from the semantic target boundary and that
candidate ELF substitution occurs only at tx73. The evaluator reads only the
target pre-state and the baseline/candidate target post-state. The suffix
`428 → 431 → 438 → 1245` remains proof machinery.

A fresh extraction reproduced the target-boundary diagnostic byte-for-byte;
a fresh semantic corpus and bundle reproduced every file byte-for-byte,
including the bundle identity above.

## Size, interface pressure, and next architecture step

Counting nonblank, noncomment lines in
[`engine/src/protocol/drift.rs`](../engine/src/protocol/drift.rs) by contiguous
source spans gives 518 Drift-specific LOC: 52 instruction recognition, 38
account-role binding, 117 layout decoding, **47 deterministic protocol math**,
104 subject mapping/findings, 116 semantic-binding corroboration, and 44
trait/plumbing. The file has 548 physical lines. This is an intentionally
single-witness evaluator, not general Drift support. U12 Orca used **zero CLMM
or other protocol-math LOC** for token flows; the current Orca file has 483
nonblank, noncomment lines after U12.1 provenance additions. The Drift witness
therefore demonstrates a real need for deterministic protocol-specific math,
though the needed cone is small (spot fixed point and cross-account checks),
not a full settlement engine.

The existing `ProtocolAdapter` was sufficient. Two concrete awkwardnesses
remain: `summarize` lacks the transaction needed to read roles and pre/post
state, so this evaluator lives in `named_findings`; and `named_findings`
returns `Vec` rather than `Result`, so malformed candidate state is withheld
and surfaced by the generic structural layer rather than as a typed semantic
decode error. A signed economic value also required an additive
`SemanticValue::SignedQuantity` variant; existing value encodings and schema-2
bundles remain valid. Binding and evaluation share decoding helpers but are
separate methods. For the next protocol-binding architecture phase, preserve
the optional binding + evaluator split and consider a transaction-aware,
fallible evaluation result before broadening to other Drift shapes.

Unsupported: all other Drift instructions or `settlePnl` shapes, other market
indices/positions, borrow or isolated settlement, open base/LP positions,
general positive/negative PnL limits, fee/revenue/funding/AMM derivations,
oracle valuation, health/margin/liquidation, and any source-to-historical-ELF
equivalence claim.

## Reproduction

```sh
cargo test -p eplyx-engine --test drift_settle_semantics
cargo test -p eplyx-engine --test causal_sequence
cargo run -p eplyx-engine --example inspect_u14 -- docs/examples/phase-u14-drift-semantics/target-boundary.json
python3 scripts/analyze-u14-drift-fields.py
cargo run -p eplyx-engine --example promote_u14_semantic_corpus -- /tmp/fresh-u14-corpus
target/debug/eplyx bundle build --corpus /tmp/fresh-u14-corpus \
  --baseline docs/examples/phase-u13-2a-runtime/historical-drift.so --out /tmp/fresh-u14-bundle
target/debug/eplyx bundle verify --bundle /tmp/fresh-u14-bundle --format json
target/debug/eplyx ci check --bundle /tmp/fresh-u14-bundle \
  --candidate /tmp/fresh-u14-bundle/binaries/current.so --format json
```
