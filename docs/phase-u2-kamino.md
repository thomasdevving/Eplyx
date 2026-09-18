# Phase U2 — Kamino KLend, the third semantic adapter

The question this phase was built to answer is not "can Eplyx support Kamino".
It is:

> Does Phase U1's universal evidence layer let a new protocol adapter consume
> trustworthy Solana evidence without reimplementing token decoding, pairing,
> boundary proof and generic deltas?

The answer is **yes, measurably** — and the phase also produced a second result
nobody asked for, which is that the thing now blocking a third *production*
corpus is not the adapter layer at all.

## What was studied

| | |
|---|---|
| Program | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` |
| Interface | `kamino_lending` 1.25.0, Anchor IDL |
| IDL source | on-chain IDL account `8qLKwp1fk8WyqmzarkuMeZEX3AzL4VDSmA2UZTKT2aCJ` |
| IDL sha256 | `8ac43c0a2f4a927ea0fd0cbd562efa1b61cb95afdde299ad5c0608079cf09164` |
| Surface | 66 instructions, 10 account types, 34 types, 5 events, 186 errors |
| Layout checked against | live mainnet accounts at slot 448,174,973 |

**Provenance is `UpgradeAuthorityPublished`, not a verified build.** The IDL is
published by the KLend upgrade authority, which is precisely the party whose
change a differential gate exists to measure. Nothing in this adapter claims
source-to-bytecode equivalence, and a test enforces that it never starts to.

The layout was therefore not taken on the IDL's word. `docs/examples/mainnet-kamino-accounts.json`
holds a real reserve and a real obligation captured at one slot, and
`the_layout_decodes_real_mainnet_accounts` checks them against each other: the
two name the same lending market, the obligation holds a borrow against that
exact reserve, and the borrowed amounts convert to sane token quantities at the
reserve's own decimal count. An offset table that had drifted would have to
drift consistently across two different structures to keep those agreeing.

## The slice

Two action families, chosen to stress different things:

| family | instructions | what it tests |
|---|---|---|
| **deposit** | `depositReserveLiquidityAndObligationCollateral`, and its `V2` form | almost entirely flow-observable |
| **borrow** | `borrowObligationLiquidity`, and its `V2` form | **not** flow-observable |

`V1` and `V2` share an action id: `V2`'s account list is `V1`'s with three farms
accounts appended, every economic account sits at the same position, and the
economics are identical.

Recognition is by 8-byte Anchor discriminator and exact account arity, never by
name. `every_supported_discriminator_matches_anchors_derivation` recomputes each
one from `sha256("global:<snake_name>")` so the constants are pinned to the
protocol rather than to themselves.

## Semantics are not contained in one instruction

The architecture study predicted this and the first two adapters never exercised
it. Both supported actions **require** `refreshReserve` *and* `refreshObligation`
in the same transaction, naming the same accounts: KLend refuses a stale
reserve, and a borrow computed against an unrefreshed obligation is a different
computation from the one that ran. The adapter requires the prerequisites rather
than tolerating them, and refuses a refresh that names a different reserve,
obligation or market.

Production immediately corrected the interface here. `refreshObligation`
declares **two** accounts in the IDL and carries **three** on mainnet, because
Anchor appends the obligation's reserves as `remaining_accounts`. An adapter
that trusted the IDL's arity would have rejected every production refresh. That
is the kind of gap only real data finds, and it is why "has an IDL" is not a
capability predicate.

## Semantic subjects, and how each is measured

The categories are kept separate because they carry different amounts of trust.

| subject | action | measurement |
|---|---|---|
| `liquidity_deposited` | deposit | `DIRECT_UNIVERSAL` |
| `reserve_liquidity_received` | deposit | `DIRECT_UNIVERSAL` |
| `obligation_collateral_deposited` | deposit | `DIRECT_TYPED_FIELD` |
| `liquidity_borrowed` | borrow | `DIRECT_UNIVERSAL` |
| `reserve_liquidity_drawn` | borrow | `DIRECT_UNIVERSAL` |
| `origination_fee` | borrow | `DIRECT_UNIVERSAL` |
| `debt_increased` | borrow | **`DERIVED_KAMINO`** |

Deliberately absent: health factor, liquidation eligibility, oracle-adjusted
debt, collateral valuation. The seven subjects above are honest without them,
and an evaluable subject the adapter cannot compute is a claim with no
implementation. `a_deposit_declares_exactly_the_subjects_it_can_measure`
asserts no subject name contains "health" or "liquidation".

## `kamino_fraction_v1` — the named evaluator

KLend stores every quantity needing sub-unit precision as a `u128` interpreted
as `U68F60`: 68 integer bits, 60 fractional. Fields carrying one are suffixed
`Sf`.

This is the boundary made concrete. A scaled fraction is **readable
declaratively and meaningless declaratively** — the raw delta on a borrow is a
number in units of 2⁻⁶⁰ tokens, and reporting it as a token amount would be off
by eighteen orders of magnitude. So the conversion lives in a named, versioned,
tested module and nowhere else:

- `to_base_units(v) = v >> 60`, an exact integer floor. No rounding, no scaling
  by mint decimals, no floating point anywhere in the path (enforced by a test).
- `delta_base_units` converts **after** subtracting. Converting each side first
  loses up to a whole base unit whenever the two straddle a unit boundary, which
  on a small borrow is the entire quantity.
- `fractional_remainder` exists so a sub-unit movement is visible rather than
  vanishing into a floored zero.

22 production lines. That is the entire irreducible protocol-math cost of this
slice, and it is the number that matters for a future contract design.

## The test that justifies the slice

Two candidates credit a borrower **identically** — same token flow, same fee,
same reserve draw — and record different debts. Every universal flow primitive
is equal between them.

```
token_flow_alone_cannot_establish_the_borrow_semantic
```

asserts first that the flows really are indistinguishable, then that the
adapter reports exactly one finding:
`kamino-klend/borrow_obligation_liquidity/economic/debt_increased/increased`.

If token flow were sufficient, a candidate that silently over-charges every
borrower would pass. This is the Drift-shaped hazard the U1 `FlowEvidence` API
was designed around, met for real in a protocol Eplyx now decodes.

## Coarse-vocabulary pressure, recorded not resolved

`SemanticAction` has no `Borrow`. The adapter reports
`SemanticAction::Unknown` for a borrow rather than forcing it into `Withdraw`:
value leaves the protocol toward the user, which *looks* like a withdrawal and
is the opposite economically — the user's net position falls rather than rising,
and a corpus selector stratifying the two together would sample two unrelated
things as one.

The exact identity is not lost; it lives in the action id, which is what
expectations key on. The enum was **not** extended —
`no_new_coarse_semantic_action_was_added_for_kamino` asserts it still has 14
variants. A variant with no second protocol behind it is a claim with no test.

## Production reality: the honest result

A bounded census of KLend mainnet activity, 1,000 signatures listed and 200
transactions examined, over slots ≈448,169,000–448,172,000:

| population | count |
|---|---:|
| signatures listed | 1,000 |
| original transaction succeeded | 601 |
| original transaction failed | 399 |
| examined (fetched and classified) | 200 |
| carried a top-level KLend instruction | 110 |
| **used address lookup tables** | **104 (94.5%)** |
| **did not** | **6 (5.5%)** |

By instruction, observed versus without lookup tables:

| instruction | observed | no-LUT |
|---|---:|---:|
| `refreshReserve` | 103 | 5 |
| `refreshObligation` | 87 | 5 |
| `flashBorrowReserveLiquidity` | 44 | 0 |
| `flashRepayReserveLiquidity` | 44 | 0 |
| `liquidateObligationAndRedeemReserveCollateralV2` | 34 | 0 |
| **`depositReserveLiquidityAndObligationCollateralV2`** | 10 | 1 |
| `repayObligationLiquidityV2` | 8 | 0 |
| **`borrowObligationLiquidityV2`** | 7 | **0** |
| `withdrawObligationCollateralAndRedeemReserveCollateralV2` | 6 | 0 |
| `rolloverFixedTermBorrow` | 3 | 3 |
| **`depositReserveLiquidityAndObligationCollateral`** | 1 | 1 |

**Replay-eligible observations for the two supported actions: zero.**

The two no-LUT deposits were examined individually rather than dismissed. Both
are the same shape — a wrapped-SOL deposit:

```
System Transfer → ATA CreateIdempotent → SPL Token SyncNative
  → refreshReserve → refreshObligation → deposit → SPL Token CloseAccount
```

They are excluded because the replay contract does not admit `SyncNative`, and
because `CloseAccount` is account closure, which is on this repository's
explicit not-started list. Widening the adapter's companion rule would not have
helped: the blocker is an **engine capability**, not an adapter decision.

So the three populations, reported side by side and never collapsed:

| population | deposit | borrow |
|---|---:|---:|
| observed (successful, top-level) | 11 | 7 |
| replay-eligible under the current engine contract | **0** | **0** |
| selected | 0 | 0 |

**What this does not say.** It does not say Kamino is unsupportable. It says
that the two things standing between this adapter and a production corpus are
address-lookup-table execution and account-lifecycle replay, both of which are
out of scope for this phase and neither of which is about protocol semantics.

Against the phase's own success criteria: **B is not met.** The adapter is
proved against synthetic observations and a production-pinned layout, not
against replayed production transactions. That is stated rather than papered
over, and no synthetic result in this phase is described as production proof.

## Did U1 pay off? The measurement

Production lines by category, classified by enclosing item
(`scripts/measure-adapter-categories.py`):

| category | Kamino | Stake Pool |
|---|---:|---:|
| A interface/instruction decoding | 83 | 75 |
| B account-role binding | 108 | 89 |
| C universal evidence plumbing | 164 | 184 |
| D protocol invariants | 147 | 166 |
| E protocol-specific math | 22 | 85 |
| F finding/semantic interpretation | 632 | 527 |
| G custom evaluator | 20 | 0 |
| — types, constants, docs | 354 | 153 |
| **total** | **1,530** | **1,279** |

### The machinery tax (§46)

Lines in the Kamino adapter that reimplement token decoding, pairing, boundary
proof, generic deltas or CPI traversal:

> **Zero**, and it is enforced rather than asserted.

`the_kamino_adapter_contains_no_token_layout_of_its_own` and
`the_kamino_adapter_delegates_proof_and_pairing_to_the_shared_layer` read the
adapter's own source. A behavioural test passes just as happily whether a token
account was decoded by the shared decoder or by a private copy of the same
offsets, so the absence of a reimplementation can only be asserted by reading
the code.

The clearest single number is `prove_boundaries`:

| | lines |
|---|---:|
| Stake Pool, before U1 | 248 |
| Stake Pool, after U1 | 162 |
| **Kamino, written against the shared prover from the start** | **93** |

Of Kamino's 93, roughly 22 are the `BoundaryContract` declaration — the actual
delegation cost — and the rest is protocol corroboration (the reserve's
available-liquidity change against its supply vault's observed balance change)
and assumption prose. Before U1 the same adapter would have carried the ~150
lines of index lookup, per-side balance matching and read-only byte identity
that the first two both had.

`interpret` tells the same story: 24 lines, of which the substance is one
closure deciding which fields render against the mint's decimals.

### What is irreducibly Kamino's

22 lines of scaled-fraction conversion, plus the state layout and the
invariants. That is the evidence a future contract design needs: **the protocol
math did not shrink and will not shrink.** It is small here because the slice is
small; a health factor would multiply it.

## Borsh and schema: the §18 answer

Stake Pool needs a **sequential** reader because `StakePool` carries three
`Option<Pubkey>` and three `FutureEpoch<Fee>` fields whose lengths depend on
their contents — a constant table would silently mis-read a differently
configured pool.

Kamino's Anchor structs are **fixed-size in their entirety**: every field is a
scalar, a `publicKey`, or a fixed array. An offset table is exact and a cursor
would buy nothing.

**They are not the same shape, and no generic reader was extracted.** The
resemblance stops at "both read bytes". What *would* help a fourth adapter is
schema-derived decoding — generating the offset table from the IDL rather than
hand-writing it — and that is a different project from a shared cursor.

One primitive was added to the shared layer: `standard_programs::u128_at`. It
sits beside `u64_at` in the same block and is the same thing — a bounds-checked
little-endian read. Leaving it out would have forced the adapter to hand-roll
`from_le_bytes`, which is the tax itself.

## One contract change, forced by the third adapter

`ProtocolAdapter::decoded_source_of` returned `Option<(&'static str, &'static str)>`:
one subject reads exactly one decoded field. Kamino cannot express itself in
that: an obligation holds up to five borrow positions in a fixed-size array, the
action touches whichever one matches its reserve, and the slot index is a
property of the *observation*, not of the subject. Returning one pair could only
ever name the wrong slot.

It is now `decoded_sources_of(&self, subject) -> &'static [(&'static str, &'static str)]`.
An adapter with a one-to-one mapping returns a one-element slice and behaves
exactly as before — verified: all five Stake Pool gate cases remain byte-identical.

This is **not** Adapter Contract v2. It is an expressiveness limit that adapter
number three found, which is exactly what this phase was for.

## What was deliberately not built

No Adapter Contract v2. No adapter DSL, YAML or TOML spec. No IDL
auto-discovery. No AI adapter generation. No semantic census. No new coarse
`SemanticAction` variant. No health factor, liquidation model or oracle pricing.
No hosted, server, frontend or bundle changes. The Stake Pool production bundle
`5e5b67ac…` is untouched and still verifies.

`no_adapter_specification_language_was_introduced` checks the first of these
mechanically.

## Unsupported, and named as such

Everything KLend does beyond the two families: repay, liquidate, withdraw
collateral, redeem, flash borrow and repay, socialize loss, referral and farm
management, withdraw queues, order management, fixed-term rollover, and every
init path. 62 of 66 instructions. `the_adapter_claims_only_the_two_families_it_supports`
holds the line.

Coverage is exact in the other direction too: a supported borrow says nothing
about repaying, and a supported deposit says nothing about withdrawal.
