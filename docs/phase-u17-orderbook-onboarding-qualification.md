# Phase U17 — Phoenix orderbook onboarding qualification

## Decision

**U17-B: a bounded execution-envelope extension is justified; stop before an
adapter.** A real, successful direct Phoenix limit order exposes a concrete
schema-2 gap: the target is a **legacy** Solana transaction, while all three
current schema-2 fidelity paths require a `validator_transaction_ref` that
`ExecutionInput::Legacy` cannot provide. The ordinary executor can run legacy
messages, but a new legacy observation cannot bind the retained validator
transaction and outcome under `CompleteExecutionV2` or
`CheckpointedExecutionV1` contract 2 or 3. This is an execution-input issue,
not an orderbook exception. The U16 recognition and fixed-role model can
describe this witness as it stands.

The semantic baseline is **not yet qualified**. The public RPC supplied the
complete target transaction, the complete account-access block, and a current
ProgramData account, but no exact parent-slot or target-slot raw account
receipts. No historical execution or target PRE/POST market diff was run.
Consequently this phase cannot claim that the order rested, that any fill
occurred, or that U15 successfully evaluated it. The historical-evidence
gate is also **U17-D** until an exact-slot account archive is available. The
primary B recommendation records the independently demonstrated product gap;
it is not a claim of a completed Phoenix adapter or replay.

The [offline analysis](examples/phase-u17-phoenix-qualification/analysis.json)
is reproduced by [the bounded checker](../scripts/analyze-u17-phoenix-qualification.py).
It verifies the retained transaction, 1,116-transaction block census, decoded
order request, token metadata deltas, current ProgramData bytes, and explicit
absence of historical state receipts. All RPC responses are provider
observations, not cryptographic validator attestations. The exact public RPC
methods and parameters are [retained](examples/phase-u17-phoenix-qualification/requests.json).

## 1–4. Candidate selection, deployment, and source

| Criterion | Phoenix Legacy | OpenBook V2 |
| --- | --- | --- |
| Mainnet program ID | `PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY` | `opnb2LAfJYbRMAHHvqjCwQxanZn7ReEHp1k81EohpZb` |
| Source/interface | [Raw Solana program](https://github.com/Ellipsis-Labs/phoenix-v1/tree/c16694d4f5f8ebd650799ada4853fde1637d4909), Shank instruction definitions, Borsh order packet, zero-copy market/book layout | [Anchor program and versioned release](https://github.com/openbook-dex/openbook-v2), [IDL](https://github.com/openbook-dex/openbook-v2/blob/master/idl/openbook_v2.json), market, bids, asks, event heap, open-orders layouts |
| Historical pin | Commit `c16694d4f5f8ebd650799ada4853fde1637d4909`, 2023-11-21 20:26 UTC; exact blob IDs in [source identity](examples/phase-u17-phoenix-qualification/source-identity.json). Its source tree is unchanged through current HEAD; later commits alter docs/license. | Repository tags mainnet `v1.7`; a deployment-specific source/ELF match was not attempted. |
| Finalized history screened | Direct successful production tx and full slot block acquired below. | A [10-signature public RPC probe](examples/phase-u17-phoenix-qualification/openbook-signatures.json) returned finalized activity; no OpenBook transaction or state was qualified. |
| Historical ELF feasibility | Upgradeable ProgramData address and complete **current** 5,000,045-byte account acquired; last deploy slot precedes target. Exact target-slot receipt and build match absent. | Upgradeable historical ELF appears acquirable through the same archive method, but was not requested. |
| Semantic adversary | Limit order can rest and match several makers inside one market account; requested price and remaining order state are distinct from two-token flow. | Separately stored bids/asks, event heap, and open-orders state could stress optional and collection roles more strongly. |
| Account/CPI shape in chosen witness | Ten fixed Phoenix roles, no LUT; direct outer instruction plus Token and Phoenix-log CPI; earlier SeatManager and ATA companions. | The IDL has optional oracle/admin accounts and an event heap; specific witness LUT/CPI complexity remains unmeasured. |

Phoenix was selected because it offers a **measured** direct, no-LUT mainnet
order with a clean same-slot account-access screen and a source revision within
an hour of the ProgramData deploy slot. OpenBook's independent event heap
would be an interesting later test, but selecting it here would replace a
measured witness with an unmeasured one. Phoenix's [repository README](https://github.com/Ellipsis-Labs/phoenix-v1)
describes a reproducible-build verification command; this phase did **not**
run it. The source timing and current ELF are provenance clues, not an exact
source-to-historical-ELF proof.

The [current Program account receipt](examples/phase-u17-phoenix-qualification/program-current.json)
points to ProgramData `2myyNegEA6pjAHmmEsJC6JdYhW51gwxQW7ZCTWvwaKTk`.
The retained [full current ProgramData receipt](examples/phase-u17-phoenix-qualification/programdata-current.json)
decompresses to 5,000,045 bytes; its 45-byte loader header reports last
deploy slot `231433470` (2023-11-21 21:12 UTC), and its ELF SHA-256 is
`e3d76e5f479989823847fd77b4f96a9c8ce4255dafefcacf920a99643e15420f`.
The earlier [64-byte header read](examples/phase-u17-phoenix-qualification/programdata-header-current.json)
matches the full account prefix. Those reads have **current** contexts
`450039087`, `450039154`, and `450040428`, all later than the target. The
market-header read is likewise current at `450039719`; none is substituted
for historical account state.

## 5–9. Production witness and execution boundary

The [complete finalized transaction](examples/phase-u17-phoenix-qualification/transaction.json)
has signature
`rR1tBf4kb4xocTPpLZXRPqFFtvXQHn2GwAEgHbU8d83URqJNWnACGcz6cJfq27kGYhuWaTjrce7ksBNAF5NZVQC`,
slot `450026714`, **index 95** of 1,116 in block
`9pQm2WPG7PJQrq64gUVV7hy5JsSiFBX2q9RRaoM5ZZsf`, parent slot
`450026713`. It succeeded, paid 40,001 lamports, uses the legacy message
format, and has no LUT. The six outer instructions are two ComputeBudget,
SeatManager `ClaimSeat`, two Associated Token `CreateIdempotent`, and Phoenix
`PlaceLimitOrder` at outer index 5. The target's inner group has one SPL Token
CPI and one Phoenix log CPI. Its log text says two market events were sent;
neither those events nor the return data is treated as economic authority.

The pinned [Phoenix instruction definition](https://github.com/Ellipsis-Labs/phoenix-v1/blob/c16694d4f5f8ebd650799ada4853fde1637d4909/src/program/instruction.rs)
assigns opcode `2` to `PlaceLimitOrder` and the ten roles below. The observed
transaction-wide access flags matter: the seat is writable because the
earlier SeatManager instruction has that privilege, although the Phoenix
instruction builder normally marks it readonly.

| Index | Role | Address | Observed access |
| ---: | --- | --- | --- |
| 0 | Phoenix program | `PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY` | readonly |
| 1 | log authority | `7aDTsspkQNGKmrexAN7FLx9oxU3iPczSSvHNggyuqYkR` | readonly |
| 2 | market/orderbook | `6BVnCYbEQZXkDZuQyaKTQMPs4HjpKcd7poMxKnRNEX32` | writable |
| 3 | trader and payer | `8LzcWniAwV2pa2q3itGmQcWGgbJjU4CVk3qQ9K2tDYKq` | signer, writable |
| 4 | seat | `BAmjhnnpAGCL1uPND16JJxTzUQhEhis6P9h7LTv4wop8` | writable |
| 5 | trader base token | `2tbbNnesHrUJkwaMLMEEDXpjeLb6o7xbieGy7UWhutwA` | writable |
| 6 | trader quote token | `Fnum6HcWtVFtDo21fKJZJZDRKT49Mj8EcxDvnHg1M9ws` | writable |
| 7 | base vault | `DNkx5wNGDkLyDk37HtEdocxayyes9K7dWtg4XsFkuiUV` | writable |
| 8 | quote vault | `2vfxhTAgQYJTojyRELrUEdcP2Frshb4L43rYooR1sZtJ` | writable |
| 9 | SPL Token | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | readonly |

The retained [complete account-mode block](examples/phase-u17-phoenix-qualification/block-accounts.json.gz)
was requested with `maxSupportedTransactionVersion: 1` because an unrelated
transaction in this slot uses v1. The checker finds **zero earlier possible
writers to any of the target's 18 message accounts**, zero later possible
writers to its nine writable accounts, and zero slot writers to Phoenix
ProgramData. The conservative target closure is therefore `[95]`; no suffix
sequence is indicated. This is an access census, not an execution proof.
The block has account metadata, signatures, and balance metadata, not the
full instruction bodies of its other transactions. The complete target
transaction is retained separately.

**Execution/fidelity classification: currently unsupported boundary in the
schema-2 product.** If exact parent/final account receipts were supplied,
the clean one-transaction census would support a checkpoint-derived single
target rather than contract 3 causal suffix replay. It would not make
`CompleteExecutionV2`'s directly observed transaction boundary true. More
immediately, `ExecutionInput::Legacy::validator_transaction_ref()` returns
`None`, and the schema-2 resolver calls `verify_validator_outcome`, which
requires that reference. The contract-3 sequence resolver makes the same
requirement. A narrow generic legacy envelope binding is needed before any
of these fidelity profiles can promote this witness; changing the Phoenix
adapter cannot fix it. The preserved block is sufficient to *screen* closure,
not to replay it.

## 10–13. Bounded semantics, arithmetic, events, and fills

The [Borsh order packet](https://github.com/Ellipsis-Labs/phoenix-v1/blob/c16694d4f5f8ebd650799ada4853fde1637d4909/src/state/order_schema/order_packet.rs)
decodes exactly 41 instruction bytes: opcode `2`, `Limit` variant `1`, `Ask`
side `1`, limit `89,868` ticks, requested `3,950` base lots,
`CancelProvide` self-trade behavior, no match limit, client order ID `73,223`,
and no time-in-force or deposited-funds flags. These are **request facts**.
The validator's pre/post token-balance metadata shows trader base
`−395,000` and base vault `+395,000` base atoms, with zero quote changes.
Those are **whole-transaction flows**; the companions and target cannot be
separated by token metadata alone. The current, later [market header](examples/phase-u17-phoenix-qualification/market-header-current.json)
links the two mints and vaults and reports base lot size 100 atoms, but it
cannot establish the target-time market configuration or final order state.

The useful candidate public subjects for this bounded interaction are:

| Candidate subject | Required target-boundary evidence | Status here |
| --- | --- | --- |
| `order_requested_base_lots` and `order_limit_price_ticks` | Instruction Borsh bytes; market header for human-unit conversion | Exact request values decoded; no historical market header for a target-time price/unit claim |
| `resting_base_lots` or `resting_order_created` | Market book PRE/target POST order ID, price, size, owner, and trader locked balance | **Withheld**; raw historical market bytes unavailable |
| `base_filled`, `quote_filled`, average fill price | Target PRE/POST book and trader state, token account reconciliation, fee math; per-maker attribution needs the changed order nodes | **Withheld**; unchanged quote token metadata alone cannot prove zero fills or a resting order |
| execution/transaction | Frozen successful outcome and future replay proof | Outcome known from RPC; schema-2 proof not built |

This is primarily **order-state decoding plus deterministic lot/tick
arithmetic**, a distinct class from Orca's token-flow-only semantics and
Drift's single-position fixed-point settlement. The pinned
[market layout](https://github.com/Ellipsis-Labs/phoenix-v1/blob/c16694d4f5f8ebd650799ada4853fde1637d4909/src/state/markets/fifo.rs)
stores bids, asks, and trader states in one large market account, using
red-black-tree nodes and price/sequence order IDs. A future evaluator would
need the market header's base/quote lot sizes, tick size, market configuration,
book-node traversal, order ownership, locked/free trader balances, and checked
integer conversion. It should reconcile exact token-account state and fees.
No such decoder or arithmetic was ported in U17.

Phoenix [market events](https://github.com/Ellipsis-Labs/phoenix-v1/blob/c16694d4f5f8ebd650799ada4853fde1637d4909/src/state/markets/market_events.rs)
include per-maker fills, placements, reductions, and fill summaries. In this
architecture they are emitted through a log CPI, not a durable event-queue
account in this instruction's ten roles. The two emitted events in the
validator log may corroborate a future raw-state derivation, but do not
establish `base_filled`, a maker identity, or a resting order. One `Limit`
instruction can match multiple resting orders by source; whether this
particular target did is **unproven** without the market PRE/POST. U15's flat
subject list could express aggregate filled base/quote; an ordered per-fill
collection would need an explicitly designed entity contract if a production
case proves aggregates insufficient. This witness does not yet prove that
extension necessary.

## 14–18. U16/U15 pressure and provenance

**Recognition.** A descriptor with program ID above, discriminator prefix
`[2, 1, 1]` (Phoenix opcode, Limit packet, Ask side), and outer index `5`
selects this direct target. The remaining 38 bytes are decoded locally.
Companion order and exact single-target restrictions are local witness guards.
There is no CPI-originated semantic target or duplicate Phoenix outer
instruction. No U16 recognition extension is required for this shape.

**Roles.** Ten ordered labels and observed signer/writable flags fit
`bind_roles`. All ten addresses are distinct. PDA, mint, vault, token-owner,
market-discriminant, and seat links require adapter-local historical account
decoding. The source's market book is internally collection-valued, but the
transaction has **one fixed market account**, not a variable maker-account
array. There is no event-queue or remaining-accounts role here. A future
OpenBook witness may establish a genuine collection-role requirement; this
Phoenix witness does not.

**Evaluation.** U15's borrowed transaction and target PRE/POST are the right
inputs for aggregate order-state subjects, provided the state is proof-bound.
`Unsupported` should cover another Phoenix opcode/packet shape;
`Unevaluable` should cover a malformed or inconsistent book; `Err` should
identify an internal decoder failure. The SeatManager and ATA companions can
change their own accounts before the Phoenix instruction. The market itself
has no earlier same-transaction Phoenix CPI in the retained instruction and
log trace, but that still needs replay and account checks. U15 explained
ranges must be narrow node/field ranges attached to a measured economic
finding. Its current validator accepts explained ranges only for economic
findings, so a **lifecycle-only** `resting_order_created` finding would leave
all changed book bytes structural. That behavior is safe; whether a small
lifecycle-range extension is useful is unproven here. `SemanticValue` can
encode scalar lot/tick quantities but has no ordered per-fill value.

**SemanticBinding.** The strongest justified tier today is
`RepositorySourceClaim` for the pinned source identity. The current
ProgramData last deploy slot, discriminator logged by execution, exact opcode
and role shape, and current market header corroborate parts of the interface,
but a `SemanticBinding::ExecutionCorroboratedExternalInterface` requires a
historical ELF hash **and independently checked baseline execution evidence**.
Neither has been established for this target. `ExactVerifiedBuild` is false:
the project publishes a verification procedure, but this phase has no
reproducible historical source-to-ELF build witness. `StandardProgramInterface`
does not describe an external orderbook program.

**Structural unknowns.** The 445,536-byte market account contains allocator
links, tree structure, order IDs, trader nodes, counters, and padding. A
future result may explain only exact changed bytes of a measured order amount
or price. It must leave changed sequence metadata, unrelated orders,
allocator nodes, unknown padding, account owner/length changes, and companion
effects visible through U15's structural layer. No entire market range can be
marked explained because one order was decoded.

## 19. Recommendation and next evidence gate

The **B** extension should be a generic schema-2 legacy execution input that
binds the exact frozen validator transaction and outcome, with strict message
comparison and without weakening any existing fidelity profile. It should be
qualified on this retained production legacy envelope before a Phoenix
adapter is written. This proposal is bounded; U17 makes no engine change.

For semantic promotion, obtain exact raw account receipts at parent slot
`450026713` and target slot `450026714` for all message inputs and expected
outputs, including the 445,536-byte market, seat, token accounts, Program and
ProgramData, and the other invoked program binaries. Bind the historical
runtime/feature inputs, execute the complete legacy transaction, compare all
watched outputs and the validator envelope, then decode the market PRE/POST.
If replay or any account receipt fails, remain at **D** and publish no order
subjects. Standard Solana [`getAccountInfo`](https://solana.com/docs/rpc/http/getaccountinfo)
exposes `minContextSlot` as a **minimum current context**, not a selector for
an old account version; the public endpoint used here cannot satisfy this
gate. No archive endpoint was configured in this worktree.

No adapter, ChangeSpec, corpus, bundle, replay path, source-to-ELF assertion,
or existing golden was changed. The answer to the acceptance question for
this exact witness is **no under the current schema-2 product**, for the
measured legacy-envelope reason. The orderbook-specific U16/U15 shapes look
representable for aggregate subjects, but that semantic conclusion remains
conditional on historical account and replay evidence.

## Reproduction

```sh
python3 scripts/analyze-u17-phoenix-qualification.py \
  > /tmp/u17-phoenix-analysis.json
cmp /tmp/u17-phoenix-analysis.json \
  docs/examples/phase-u17-phoenix-qualification/analysis.json
```
