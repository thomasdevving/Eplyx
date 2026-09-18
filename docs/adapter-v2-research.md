# Universal Protocol Understanding — repo-grounded architecture study

The question: how much of an arbitrary Solana protocol can Eplyx understand
automatically, what can be declarative, what needs deterministic custom code,
and how should `ProtocolAdapter` evolve without weakening trust?

Everything below about Eplyx is measured from the code at commit `029811d`.
Nothing is taken from `CLAUDE.md` or from the phase docs.

## 1. What is actually there

### The generic engine and the protocol layer

```
engine/src/                 18,614 lines
  ├─ generic              ~14,100   executor, diff, money, report, types,
  │                                 dependencies, screening, versions, replay,
  │                                 discovery, select, bundle, ci, review,
  │                                 semantics, expectations
  └─ protocol/             4,503    mod.rs 1,000 · token2022.rs 1,144
                                    stake_pool.rs 2,359
```

`protocol/mod.rs` is not an adapter. It holds the trait, the shared value types
(`TokenQuantity`, `SignedTokenQuantity`, `SemanticField`, `SemanticAccount`,
`EconomicObservation`, `EconomicChange`, `EntityId`, `StateFeature`,
`BoundaryDistance`, `SemanticAction`), and the registry.

### The registry is one adapter per program id

```rust
pub fn adapters() -> &'static [&'static dyn ProtocolAdapter] {
    const TOKEN_2022: token2022::Token2022Adapter = token2022::Token2022Adapter;
    const STAKE_POOL: stake_pool::StakePoolAdapter = stake_pool::StakePoolAdapter;
    &[&TOKEN_2022, &STAKE_POOL]
}

pub fn adapter_for(program_id: &str) -> Option<&'static dyn ProtocolAdapter> {
    adapters().iter().copied().find(|a| a.program_id() == program_id)
}
```

A linear scan on a single static program id. Three consequences, none of them
accidental but all of them limits:

- **one adapter per program**, so a protocol whose economics span several
  programs has no representation;
- **no version dimension** — the adapter that speaks for a program id speaks for
  every deployment of it, past and future;
- adapters are compiled in, which is exactly right for a trust root and exactly
  wrong for scaling to protocol N+1 without a release.

### The trait is 22 methods

`name`, `adapter_version`, `semantic_action`, `economic_entity_id`,
`state_features`, `boundaries`, `program_id`, `accept`, `supports_cpi`,
`dependency_programs`, `label`, `decode`, `required_accounts`,
`prove_boundaries`, `interpret`, `summarize`, `protocol_id`, `action_id`,
`evaluable_subjects`, `decoded_source_of`, `decoded_byte_ranges`,
`named_findings`.

### Only one adapter implements the semantic layer

|  | stake_pool | token2022 |
|---|---|---|
| `action_id` | yes | no |
| `evaluable_subjects` | yes | no |
| `named_findings` | yes | no |
| `summarize` | yes | no |
| `decoded_source_of` | yes | no |
| `decoded_byte_ranges` | yes | no |
| `protocol_id` | default | default |

**This is the single most important fact in the study.** The semantic contract —
the vocabulary, the fingerprints, the capability declaration, the gate's whole
declarable layer — has been exercised by exactly one protocol. Token-2022
participates in replay and historical acquisition only. Any claim that the
current design generalizes rests on n = 1, and this study cannot pretend
otherwise.

### There is no generic token decoder

```rust
pub enum FieldDecoder {
    None,
    FixtureLending,
}
```

The generic diff can decode the synthetic fixture layout or nothing. SPL Token
is not in the generic engine at all. Both adapters define the account length
independently — `TOKEN_ACCOUNT_LEN: usize = 165` in `stake_pool.rs`,
`ACCOUNT_LEN: usize = 165` in `token2022.rs` — and each decodes token accounts
itself.

## 2. Measured decomposition of the Stake Pool adapter

2,359 lines total: 1,774 production, 585 test.

| category | lines | share |
|---|---:|---:|
| F execution-shape validation (`accept`, `prove_boundaries`) | 440 | 24% |
| I state decoding and coverage (`decode`, `summarize`, `state_features`, `boundaries`) | 388 | 21% |
| — types, constants, documentation | 333 | 18% |
| H finding generation (`named_findings`, `interpret`) | 159 | 9% |
| G capability declaration | 104 | 6% |
| C account-role mapping | 103 | 6% |
| A instruction decoding | 97 | 5% |
| E protocol economic math | 79 | 4% |
| D token/SOL measurement | 54 | 3% |
| B action naming | 17 | 1% |

The two largest categories are not protocol economics. They are *admission* and
*decoding*.

### How much of it is accidental

Comparing the methods both adapters implement, line by line, ignoring comments
and blank lines:

| method | stake_pool | token2022 | identical | of token2022 |
|---|---:|---:|---:|---:|
| `label` | 6 | 6 | 6 | 100% |
| `interpret` | 51 | 53 | 42 | 79% |
| `balance_at` | 9 | 9 | 7 | 77% |
| `prove_boundaries` | 248 | 137 | 100 | 72% |
| `labels` | 28 | 35 | 24 | 68% |
| `semantic_action` | 5 | 6 | 4 | 66% |
| `economic_entity_id` | 15 | 10 | 6 | 60% |
| `data_len` | 6 | 10 | 4 | 40% |
| `roles` | 6 | 11 | 4 | 36% |
| `accept` | 153 | 71 | 25 | 35% |
| `decode` | 121 | 125 | 35 | 28% |
| `boundaries` | 43 | 49 | 12 | 24% |
| `state_features` | 43 | 77 | 14 | 18% |
| `from_discriminant` | 7 | 14 | 2 | 14% |
| **total** | **613** | **285** | **46%** |

**46% of the second adapter's shared-method code is line-identical to the
first's.** With two adapters that is 285 duplicated lines. It grows linearly.

The split is sharp and it is informative. Everything above 60% is machinery for
*proving a replay was faithful*: matching snapshots to validator-observed
lamports and token balances, labelling message keys, pairing V1/V2 summaries.
Everything below 40% is the protocol actually saying something: which
instructions it admits, what its accounts mean, which thresholds matter.

Classifying by the study's four buckets, with the evidence:

**ACCIDENTALLY PROTOCOL-SPECIFIC** — universal work that happens to live in
adapters:
- `prove_boundaries`, 72% duplicated. Proving an archived snapshot against
  validator metadata is a property of Solana, not of a protocol. The genuinely
  protocol-specific remainder in Stake Pool is ~118 lines: a `find` closure over
  labelled accounts (64) and a `match self.operation(...)` (44).
- `label` / `labels`, 68–100%.
- `interpret`, 79%. Pairing two executions into observations.
- `balance_at`, token-account decoding, `TOKEN_ACCOUNT_LEN = 165`.
- `economic_entity_id`, 60%: "the account at position N is the user" is a role
  binding, not a computation.

**UNIVERSAL** — already generic, correctly:
- execution result, CPI invocation graph, compute, lamport deltas, byte diff,
  dependency resolution, same-slot screening, version resolution.

**DECLARATIVE** — protocol knowledge that is data, not code:
- which discriminants this adapter admits (`from_discriminant`, 7 lines);
- the action name per discriminant (`action_id`, and 17 lines of `semantic_action`);
- account role per position (`entity_position`, `authority_position`, `roles`);
- the account layout offsets in `decode`;
- the subject list in `evaluable_subjects`.

**PROTOCOL-SPECIFIC (irreducible)**:
- `lamports_for_withdrawal`, `fee`, `apply` — the pool's share/lamport
  arithmetic, which must multiply before dividing and is the whole point;
- `boundaries` — what counts as an economic threshold here;
- the `match self.operation(...)` inside `prove_boundaries` — which accounts
  must be proved depends on which operation ran;
- `named_findings` — deciding that a decrease in pool tokens received is worth a
  HIGH finding.

Measured: of 1,774 production lines, roughly **440 are admission and proof**,
of which the duplicated portion suggests ~120–150 could become one universal
implementation; **~200 are declarative data expressed as code**; and **~250 are
irreducible protocol math and judgement**. The remainder is types, constants and
documentation.

## 3. The four protocols

The comparison framework and its filled-in matrix live in
`adapter-v2-protocol-matrix.md`. The findings that bear on architecture:

### Orca Whirlpools — Anchor, typed, and still partly opaque

`swap` has eleven accounts. Seven are typed in the Anchor context
(`Account<Whirlpool>`, four `Account<TokenAccount>`, `Program<Token>`,
`Signer`). Four are **`UncheckedAccount`**: `tick_array_0`, `tick_array_1`,
`tick_array_2`, `oracle`.

That is the concentrated-liquidity state itself. An IDL-driven decoder gets the
pool, the vaults and the user's token accounts for free, and gets nothing about
the arrays that determine what a swap actually costs.

The instruction emits a `Traded` event. Events are the cleanest semantic source
available anywhere in this study — but they are program-emitted, so they are a
claim by the code under test, and a candidate that changed its economics could
change its event to match.

Structural understanding: complete. Token-flow understanding: complete, both
vault deltas are ordinary SPL transfers. Economic semantic understanding of
*price impact* or *fee attribution*: requires tick math.

### Kamino KLend — the multi-instruction problem

Deposit is two instructions: `deposit_reserve_liquidity` mints cTokens to the
user, then `deposit_obligation_collateral` moves those cTokens into the
obligation. Borrow requires `refresh_reserve` and `refresh_obligation` to have
run first, in the same transaction.

**This breaks the current model at its root.** Stake Pool's adapter identifies an
action from one top-level instruction. A Kamino borrow's economic meaning
depends on state refreshed by a *different instruction earlier in the same
transaction*, and a deposit's user-visible meaning spans two.

`Obligation` carries `deposited_value_sf` and
`borrow_factor_adjusted_debt_value_sf`. The `sf` is scaled fixed point.
Reading the field is declarative; knowing that it is a fraction with a specific
scale, and that comparing two of them is only meaningful after the same
refresh, is not.

Good news for the universal layer: the cToken mint *is* a real token flow. The
Stake Pool primitives — asset deposited, receipt token received — transfer to
Kamino deposit unchanged. Debt does not: it is a field in an account, not a
token.

### Drift — the case that breaks the flow graph

`settlePNL` moves value between a user's position and a per-market P&L pool.
All deposits sit in a **global collateral vault**, and settlement is accounting
across intermediate pool balances rather than SPL transfers. Calling it does not
change the position.

So: **a real economic change with zero token-flow edges, zero lamport movement
on the user's account, and no mint or burn.** The only evidence is a typed field
delta inside a program-owned account.

Any architecture whose semantic layer is built on an economic flow graph reports
this as "nothing happened". That is worse than reporting nothing at all, because
it looks like coverage.

### Phoenix — an IDL that describes no accounts

Phoenix is non-Anchor. An IDL is published, and it contains **28 instructions,
0 account types, 0 events, 31 type definitions**.

This is the cleanest possible demonstration that "has an IDL" is not a usable
capability predicate. Instruction identity: available. Argument types:
available. Account layout: absent. Events: absent.

An architecture that treats `InterfaceSource::Idl` as a single capability will
claim Phoenix account decoding it cannot do.

## 4. What this means for the four assumptions

The study set out four things a universal architecture would need to be true.
Three of them are false as stated.

| assumption | verdict |
|---|---|
| An IDL gives instruction, argument, account layout and events | **False.** Phoenix: no accounts, no events. Orca: four `UncheckedAccount`s covering the state that matters. |
| Economic change implies token flow | **False.** Drift `settlePNL`. |
| An action is one instruction | **False.** Kamino deposit is two; borrow depends on a prior refresh. |
| Universal primitives transfer across protocols | **True, for the measured ones.** Asset-in/receipt-out holds for Stake Pool, Orca liquidity and Kamino deposit. |

The fourth is the one with evidence behind it, and it is also the one that does
not need a new contract language to act on.

## 5. Where the current design is already right

Three properties should survive any v2 and are worth naming so they are not
traded away for expressiveness:

- **`evaluable_subjects` is a capability declaration checked against emitted
  findings.** A test asserts every emitted finding is covered by a declared
  capability. That is the mechanism that makes "supported" mean something, and
  it is already there.
- **Absent evidence is never absence of a problem.** `no_semantic_coverage` is
  exit 2, not a pass. An adapter with nothing to say does not get to say
  "nothing changed".
- **Three evidence layers, only one declarable.** Named findings can be
  declared; decoded economic changes nothing named speaks for, and structural
  changes nothing spoke for at all, cannot. The production pilot demonstrated
  this holding: a bounded declaration moved both findings to `expected` and the
  gate still failed on `undeclarable_change`.

A declarative contract that lets a protocol describe its own subjects must not
become a way to declare the third layer away.
