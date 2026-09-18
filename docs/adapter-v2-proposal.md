# Adapter Contract v2 — proposal, and what the evidence does not support

This is the design half of the study. `adapter-v2-research.md` measures the
repository; `adapter-v2-protocol-matrix.md` compares four protocols. Read those
first: the recommendation here follows from them and contradicts the shape the
phase brief assumed.

## The recommendation, first

**Do not build Adapter Contract v2 yet. Build the universal primitive layer,
which the evidence does support, and let the contract follow from a second
semantic adapter written against it.**

The reasoning is not caution for its own sake:

- The duplication finding is real and measured. 46% of the second adapter's
  shared-method code is line-identical to the first's, and the most duplicated
  method — `prove_boundaries`, 72% — is Solana machinery, not protocol
  knowledge. That is worth fixing now and needs no new schema.
- The semantic contract has **one** implementation. Designing a declarative
  language for protocol semantics against n = 1 would encode Stake Pool's
  shape as though it were the general case, and three of the four protocols
  examined already violate an assumption that shape carries.
- Three of the brief's own failure criteria (§34) are *already met* by a
  contract designed on today's evidence: it would assume economic change implies
  token flow (Drift disproves), it would assume an action is one instruction
  (Kamino disproves), and it would assume an IDL describes accounts (Phoenix
  disproves).

A contract designed now would have to be redesigned after the second protocol.
A primitive layer designed now is testable against the two adapters that exist.

## What to build now: the universal primitive layer

Owned by the engine, not by adapters. Every item is justified by measured
duplication or by a measured absence.

**Execution** — already generic. Success/revert, compute, CPI invocation graph.
No change.

**Boundary proof.** Extract the ~100 duplicated lines of `prove_boundaries` into
one implementation that matches snapshots against validator lamport and token
balances. What stays in the adapter is the part that differs: which accounts
must be proved for which operation (~118 lines in Stake Pool). This is the
single largest measured win and it changes no semantics.

**SPL Token and Token-2022 decoding.** `FieldDecoder` currently offers `None` or
`FixtureLending`. Both adapters define `165` and decode token accounts
themselves. The engine should own: token account (mint, owner, amount,
delegate, state), mint (supply, decimals, authorities), and the Token-2022
extensions — transfer fee config and withheld amounts, pause, permanent
delegate, freeze, transfer hook presence. An adapter that must reimplement
Token-2022 extension semantics to notice a withheld fee will get it wrong.

**Native.** Lamport delta, account create/close, owner change, executable
change. Partly present in the diff; not exposed as named primitives.

**Labelling and pairing.** `label`/`labels` (68–100% duplicated) and `interpret`
(79%) are mechanism, not meaning.

**Typed field delta.** Where a layout is known, a field delta; where it is not,
a byte-range delta. The engine already has the byte half.

Estimated effect on the Stake Pool adapter, from the measured categories: of
1,774 production lines, roughly 120–150 lines of proof machinery and 54 lines of
token measurement move out, and ~200 lines of declarative-shaped code stay as
code until there is a contract to hold it. That is a real but modest reduction —
**and the point is not the reduction, it is that adapter number three does not
pay for it again.**

## The contract, when it is justified

Sketched so the primitive layer can be built without closing doors. Not to be
implemented in this phase.

```
AdapterContract
  schema_version        u32
  protocol              ProtocolId
  adapter_version       u32
  interface             Interface
  actions               [Action]
  capabilities          [SemanticSubject]        # the declared surface
  evaluators            [EvaluatorRef]           # stable ids, compiled in

Interface
  source                InterfaceSource          # taxonomy, see the matrix
  provenance            Provenance               # ExactVerifiedBuild … ManualAssociation
  interface_hash        sha256
  program_binary_hash   Option<sha256>           # what it was proved against

Action
  match                 InstructionMatch         # program, discriminant, arity
  context               Option<[InstructionMatch]>  # Kamino: a required prior refresh
  action_id             ActionId
  semantic_action       Option<SemanticAction>
  roles                 [RoleBinding]            # position -> role
  measurements          [Measurement]
  invariants            [Invariant]

Measurement
  subject               SemanticSubject
  source                MeasurementSource
  quantity              QuantityKind             # token(mint_role) | lamports | raw

MeasurementSource
  TokenBalanceDelta(role)
  LamportDelta(role)
  MintSupplyDelta(role)
  TypedFieldDelta(role, field_path)
  Evaluator(evaluator_id)                        # the escape hatch, named

Invariant
  MintsEqual(role, role)
  SignIs(measurement, Positive | Negative)
  OwnedBy(role, role)
  FlowMatches(measurement, from_role, to_role)
  FieldEquals(role, field_path, role)
```

Three deliberate refusals in that sketch:

- **No expression language.** `DerivedArithmetic` was considered and rejected.
  `post - pre` is already `TokenBalanceDelta`. Anything beyond it — Kamino's
  scaled-fraction normalisation, Orca's tick math, Drift's margin — is not
  "slightly more arithmetic", it is protocol math with its own correctness
  argument, and it belongs in a named compiled evaluator that can be tested and
  versioned. A declarative language that can express a health factor is a
  programming language, and §34 rejects the design at that point.
- **`context` on an action, not a flat instruction match.** Kamino forces this.
  Without it the contract cannot express "this borrow's numbers are only
  meaningful because `refresh_obligation` ran first", and a contract that cannot
  express a precondition will silently measure a stale one.
- **`capabilities` is a closed list checked against emitted findings.** This
  already exists and already has a test. It is the mechanism that stops an
  evaluator returning a subject nobody declared.

## Evidence levels

Not a confidence percentage. Five states, each meaning one specific thing:

| state | means |
|---|---|
| `Structural` | the instruction was seen; nothing is claimed about it |
| `InterfaceDescribed` | an interface names it, with recorded provenance |
| `ExecutionObserved` | it was replayed and primitives measured |
| `SemanticallyVerified` | measurements bound to subjects and all invariants held |
| `ProductionAssured` | additionally, validated production-derived observations reproduce |

"Instruction decoded, semantics not verified" is a useful, reportable state, and
today's architecture cannot say it. Only the last two may drive a gate finding.

`Supported` is then definable: instruction identity deterministic, roles known,
execution reproducible, measurements deterministic, invariants hold, findings
emitted without guessing. Execution without semantics is `ReplaySupported`, not
`SemanticallySupported` — which is exactly what Token-2022 is today, and the
current model has no word for it.

## The economic flow graph: build it, do not believe it

A graph of accounts and mints with transfer, mint, burn and lamport edges is
worth having. It expresses the one assumption that survived all four protocols:
what a user hands over and what they receive.

It fails, specifically and importantly, on Drift `settlePNL`: value moves
between a position and a P&L pool with no transfer, no mint, no burn and no
lamport delta, because all collateral sits in one global vault and settlement is
accounting. An architecture that derives semantics *from* the graph reports this
as nothing happened — which is worse than reporting nothing, because it looks
like coverage.

So: the graph is a **measurement primitive**, never the semantic source. An
action whose subjects cannot be measured from it is not an unsupported action;
it is an action that needs typed field deltas or an evaluator. The contract must
be able to say so, and the capability model must show it as unsupported rather
than as clean.

## Roles and actions

**Roles should be namespaced strings, not an enum.** The measured naming across
four protocols does not converge: `user_source` is meaningful for Stake Pool,
Orca and Kamino deposits, and meaningless for Drift, where the user's economic
position is a field in a program-owned account. A fixed enum would either grow
without bound or force protocols into wrong roles. Namespaced —
`spl-stake-pool/depositor_pool_token_account` — keeps fingerprint stability
without pretending to universality. A small set of *well-known* roles may be
recognised by the primitive layer for flow-graph binding; the rest stay opaque
strings.

**`SemanticAction` is insufficient and should not be stretched.** Orca
`increase_liquidity` is not `Deposit`: two assets go in, no receipt token comes
back, and the position is an NFT whose value depends on a range. Forcing it into
`Deposit` would let a corpus selector stratify two economically unrelated things
together. The vocabulary needs at least `LiquidityAdd`, `LiquidityRemove`,
`Borrow`, `Repay`, `Liquidate`, `Settle`, and `PositionModify` before a second
protocol arrives. **Do not add them yet** — an enum variant with no adapter
behind it is a claim with no test. Add each one with the adapter that needs it.

## Where AI may and may not be used

Acceptable, as a proposer: suggest role names from account context, map
instruction names to coarse actions, propose candidate subjects, draft invariant
templates, summarise source and documentation.

Not acceptable, ever: deciding pass or fail, producing a finding without
deterministic evidence, or moving a contract into `SemanticallyVerified`.

A generated contract enters as `Generated`, is reviewed to `Reviewed`, and
becomes `Verified` only when its invariants hold against validated
production-derived observations. The gate consumes `Verified` and above. This is
the same discipline the corpus already has — *a corpus is acquired until
something reproduces it* — applied to semantics.

## Threats, and the guard each needs

| threat | guard |
|---|---|
| malicious or stale IDL | provenance recorded; only `ExactVerifiedBuild` or a standard program drives findings |
| IDL for a different program version | interface bound to the deployed binary hash, which Eplyx already resolves per slot |
| misleading account names | names are hints; roles are bound by position and checked by invariants |
| malformed layout | typed field read is bounds-checked; failure is an error, never a zero |
| contract matching too broadly | discriminant plus arity plus invariants; ambiguity is a contract error |
| role misbinding | `MintsEqual`, `OwnedBy`, `FlowMatches` must hold or the action is unevaluable |
| measurement reading the wrong mint | quantity carries its mint role; cross-mint arithmetic already returns `None` |
| AI-generated false semantics | cannot reach `Verified` without invariants holding on real observations |
| evaluator returning an undeclared subject | `capabilities` is closed and already tested |
| contract claiming what it cannot evaluate | capability matrix per subject, not per protocol |

## Five ways naive inference gets it wrong

1. **`withdraw` that withdraws protocol fees.** Kamino and Orca both have
   fee-collection instructions whose names read like user withdrawals. Flow
   direction is out; the recipient is the protocol.
2. **A transfer that is a fee, not a payout.** Stake Pool's manager fee account
   receives a mint on deposit. Naive "user received tokens" attribution counts
   the protocol's fee as the depositor's proceeds — and this is not theoretical:
   Eplyx's own report suppresses `pool-mint/supply` for exactly this reason,
   because it is the burn on a withdrawal and a by-product of the mint on a
   deposit.
3. **A mint that is debt, not receipt.** Kamino's cToken is a receipt; a debt
   position is a field. A protocol that tokenised debt would present an
   identical mint-to-user shape with the opposite meaning.
4. **Internal position change with no token movement.** Drift `settlePNL`.
5. **A writable account that looks like the user's and is not.** Orca's
   `token_vault_a` and the user's `token_owner_account_a` are both writable
   `Account<TokenAccount>` of the same mint. Position alone distinguishes them,
   which is why roles must be bound and then checked with `OwnedBy`, not
   inferred from type.

And the converse — conservatism losing real coverage:

- no IDL but verified source exists (Stake Pool today): layout is knowable,
  just not automatically;
- no event but flows are deterministic: Orca swap needs no event;
- a generic instruction name with a decisive account layout;
- a protocol documented only in an SDK.

Each is a reason the ladder has rungs: `InterfaceDescribed` without
`SemanticallyVerified` is still worth recording, and a protocol can climb.

## Failure criteria, checked honestly

| §34 criterion | v2-as-briefed | primitive layer |
|---|---|---|
| only works for Anchor | **fails** — Phoenix has no account types | passes; primitives are execution-level |
| assumes token flow | **fails** — Drift | passes; flow is one primitive among several |
| adversarial case bypasses architecture | **fails** — Drift needs an evaluator for everything | passes; it is honestly unsupported |
| schema becomes a programming language | at risk, if arithmetic is admitted | n/a |
| adapters emit undeclared subjects | already guarded | already guarded |
| provenance ignored | at risk | n/a |
| generated semantics enter CI | guarded by evidence levels | n/a |
| protocol math leaks into generic engine | at risk | guarded by keeping evaluators named |

Four criteria are met by the contract as briefed. That is the answer to the
research question, and it is not the answer the brief expected.

## Unresolved

- Can the boundary-proof extraction keep Stake Pool's `prove_boundaries`
  behaviour byte-identical? The 118 protocol-specific lines are entangled with
  the shared ones; this needs a real refactor with the existing tests as the
  control, not an estimate.
- Does a contract hash belong in bundle identity? It would make a semantic
  change invalidate a corpus, which is correct, and would also invalidate every
  existing bundle. Deferred with the contract.
- Is `settlePNL` even in scope? A protocol whose economics are entirely internal
  state may be better served by a different tool than a differential replayer.
  Worth asking before building for it.

## Success criterion, answered

*What does Eplyx need to know to support protocol N+1, how much can be
discovered automatically, how much declared, and where is bespoke code
unavoidable?*

- **Discovered automatically:** instruction identity and arguments, nearly
  everywhere. Account roles by position. Token and lamport flows. Execution
  shape, CPI graph, compute. This is the primitive layer and it is protocol-
  independent.
- **Declarable, once the primitive layer exists:** which discriminants are
  admitted, action names, role bindings, which subjects are measured from which
  primitive, and the invariants that must hold. Measured at roughly 200 of
  Stake Pool's 1,774 production lines.
- **Bespoke and unavoidable:** any quantity that is a function of protocol state
  rather than of observed flow. Exchange rates, tick and liquidity math,
  index-normalised debt, health factors, margin. Roughly 250 lines for Stake
  Pool; more for every other protocol examined.
- **Not supportable by this architecture at all, today:** actions whose entire
  economic content is internal accounting with no flow and no stable typed
  field an outsider can bind to. Drift `settlePNL` is the example, and the
  correct response is to report it unsupported rather than to widen the
  architecture until it appears covered.

The honest one-line answer: **Eplyx can automate recognition, can declare
binding, and cannot avoid writing protocol math. A universal adapter
architecture is feasible for the first two and is not feasible for the third,
and the value of v2 is in making that boundary explicit rather than in removing
it.**

## Four pseudo-contracts, annotated line by line

Illustrative. `G` = generic primitive the engine owns, `D` = declarative data in
the contract, `C` = compiled evaluator. Nothing here is implemented.

### A. Stake Pool `DepositSol` — the baseline

```
action                                                         # D
  match: program=SPoo1Ku8…, discriminant=14, accounts=11       # D
  action_id: deposit_sol                                       # D
  semantic_action: Deposit                                     # D
  roles:
    0 -> pool                                                  # D
    1 -> withdraw_authority                                    # D
    2 -> reserve                                               # D
    3 -> user_source          (lamport source, signer)         # D
    4 -> user_destination     (pool token account)             # D
    5 -> manager_fee_account                                   # D
    6 -> pool_mint                                             # D
  measurements:
    sol_deposited        = LamportDelta(user_source)           # G via D
    pool_tokens_received = TokenBalanceDelta(user_destination) # G via D
    manager_fee          = TokenBalanceDelta(manager_fee_account)   # G via D
    pool_mint_supply     = MintSupplyDelta(pool_mint)          # G via D
  invariants:
    MintsEqual(user_destination, pool_mint)                    # G
    SignIs(sol_deposited, Negative)                            # G
    SignIs(pool_tokens_received, Positive)                     # G
    OwnedBy(reserve, pool)                                     # G
    FlowMatches(sol_deposited, user_source, reserve)           # G
  evaluators:
    expected_shares = spl_stake_pool_share_math_v1             # C
```

Everything the current adapter spends ~200 lines on becomes the `roles` and
`measurements` blocks. The ~79 lines of share arithmetic stay, as one named
evaluator. The ~440 lines of admission and proof become the `match` line plus
the generic boundary prover.

### B. Orca `swap` — where declarative stops

```
action                                                         # D
  match: program=whirLb…, discriminant=anchor("swap"), accounts=11   # D
  action_id: swap                                              # D
  semantic_action: Swap                                        # D
  roles:
    1 -> token_authority (signer)                              # D
    2 -> pool                                                  # D
    3 -> user_a                                                # D
    4 -> vault_a                                               # D
    5 -> user_b                                                # D
    6 -> vault_b                                               # D
    7,8,9 -> tick_array                    # D, but layout UNKNOWN from IDL
    10 -> oracle                           # D, layout UNKNOWN from IDL
  measurements:
    input_spent      = TokenBalanceDelta(user_a | user_b)      # G via D
    output_received  = TokenBalanceDelta(user_b | user_a)      # G via D
    vault_a_delta    = TokenBalanceDelta(vault_a)              # G via D
    vault_b_delta    = TokenBalanceDelta(vault_b)              # G via D
  invariants:
    MintsEqual(user_a, vault_a)                                # G
    MintsEqual(user_b, vault_b)                                # G
    FlowMatches(input_spent, user_a, vault_a)                  # G
    OwnedBy(vault_a, pool)                                     # G
  evaluators:
    price_impact   = orca_whirlpool_price_impact_v1            # C
    fees_attributed = orca_whirlpool_fee_split_v1              # C
```

The whole user-visible trade is declarative. Everything about *why* that was the
price is custom, and the accounts carrying the answer — the tick arrays — are
`UncheckedAccount` in the IDL, so the evaluator must own their layout too.

`a_to_b` is an instruction *argument*, so which of `user_a`/`user_b` is the
input is known before execution. That is a case for argument-conditioned role
binding, which the sketch above does not express and would need.

### C. Kamino `borrow_obligation_liquidity` — the precondition problem

```
action                                                         # D
  match: program=KLend2g…, discriminant=anchor("borrow_obligation_liquidity")  # D
  context:                                                     # D
    requires_prior: refresh_reserve                            # D
    requires_prior: refresh_obligation                         # D
  action_id: borrow_obligation_liquidity                       # D
  semantic_action: Borrow            # not in today's vocabulary
  roles:
    obligation, lending_market, reserve, user_destination,
    reserve_source_liquidity, fee_receiver                     # D
  measurements:
    liquidity_received = TokenBalanceDelta(user_destination)   # G via D
    reserve_drawn      = TokenBalanceDelta(reserve_source_liquidity)  # G via D
    origination_fee    = TokenBalanceDelta(fee_receiver)       # G via D
    debt_value_sf      = TypedFieldDelta(obligation,
                           borrow_factor_adjusted_debt_value_sf)     # G via D
  invariants:
    SignIs(liquidity_received, Positive)                       # G
    SignIs(reserve_drawn, Negative)                            # G
    FlowMatches(liquidity_received, reserve_source_liquidity,
                user_destination)                              # G
    FieldEquals(obligation, lending_market, lending_market)    # G
  evaluators:
    debt_increased = kamino_debt_index_normalise_v1            # C
    health_factor  = kamino_health_factor_v1                   # C
```

`debt_value_sf` is readable declaratively and **meaningless declaratively**: it
is a scaled fraction whose two ends were computed against a refresh. A contract
that let this delta drive a finding without the evaluator would report debt
changes that are units, not economics. The `context` block is what stops that,
and today's model cannot express it.

### D. Drift `settlePNL` — the case that does not fit

```
action                                                         # D
  match: program=dRiftyHA…, discriminant=anchor("settle_pnl")  # D
  action_id: settle_pnl                                        # D
  semantic_action: Settle            # not in today's vocabulary
  roles:
    user (program-owned), state, spot_market_vault, perp_market # D
  measurements:
    # No token flow. No lamport delta. No mint or burn.
    # Every generic flow primitive measures exactly zero here.
    quote_settled = TypedFieldDelta(user,
                      perp_positions[i].quote_asset_amount)    # G via D, layout from IDL
    pnl_pool      = TypedFieldDelta(perp_market,
                      pnl_pool.scaled_balance)                 # G via D
  invariants:
    # There is no flow to check the field against.
    # The only honest invariant is conservation, and proving it
    # needs the protocol's own accounting rules.
    ConservationHolds(quote_settled, pnl_pool)                 # C, not G
  evaluators:
    unrealized_pnl      = drift_unrealized_pnl_v1              # C
    margin_requirement  = drift_margin_v1                      # C
```

This is the one to read carefully. Every generic measurement returns zero. The
declarative part reduces to two typed field paths. The invariant that would
bind them — that what left one balance arrived in the other — is itself protocol
math, so it cannot be a generic invariant.

Drift does not extend the architecture; it exits it. The correct outcome is a
capability matrix entry saying `settle_pnl: unsupported, no deterministic
binding`, and coverage reporting that names it. A design that instead widens
`Invariant` until `ConservationHolds` is generic has begun writing Drift's
accounting into the engine, which §34 rejects.
