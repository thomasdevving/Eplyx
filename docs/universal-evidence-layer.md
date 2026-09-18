# The universal evidence layer

Phase U1. A refactor and a foundation: every protocol-independent execution and
proof primitive moved out of the protocol adapters and into the engine, with all
existing semantic output byte-identical.

This document says what the layer owns, what it deliberately does not own, and
which of its design choices exist to stop a specific wrong answer.

## Why it exists

Measured at commit `029811d`, before anything moved:

| | |
|---|---|
| Line-identical overlap between the two adapters' shared methods | **46%** (303 of 654 lines) |
| Most duplicated method | `prove_boundaries`, **77%** |
| Second | `interpret`, **84%** |
| Token account layout (`165`) declared in | **two** adapters, independently |
| `u64_at` / `address_at` | 100% identical, twice |

The duplicated half was not protocol economics. "Does this archived snapshot's
lamport balance equal the validator's balance at this account's index in the
message?" is a question about Solana. Adapter number three would have answered it
for a third time, and adapter number four for a fourth.

So the question this phase answers is not "how do we support more protocols" —
that is a later phase, and the architecture study's conclusion that **Adapter
Contract v2 should not be built yet** stands unchanged. It is the narrower
question: *can adapter #3 consume trustworthy Solana evidence without rebuilding
the same account, token and boundary machinery again?*

## The two new module trees

```
engine/src/
  standard_programs/     what the bytes ARE
    mod.rs               Decoded<T>, MalformedReason, SchemaProvenance, readers
    spl_token.rs         the 165-byte account, the 82-byte mint
    token2022.rs         the same base layout, plus the TLV extension list
    system.rs            the System instruction encoding

  evidence/              what was MEASURED
    mod.rs               UniversalEvidence, FlowEvidence, Provenance
    account.rs           pre/post pairing, lifecycle
    boundary.rs          proving snapshots against validator metadata
    token.rs             balance, supply, authority deltas; transfer/mint/burn
    native.rs            lamport deltas, with attribution
    cpi.rs               the invocation graph
    field.rs             typed field deltas and byte-range deltas
    labels.rs            naming a message's account keys
    pairing.rs           comparing two executions field by field

  protocol/              what it MEANS
    stake_pool.rs
    token2022.rs
```

The separation is meant to be visible in the tree, not only in prose. A reviewer
asking "is this protocol knowledge?" can answer it from the path.

## What each layer owns

**`standard_programs`** owns layouts that are not any protocol's private
knowledge: SPL Token, Token-2022, System. It may claim that a byte range holds a
field of a given type under a named layout. It may **not** claim that the layout
is what the deployed program implements — that would need a reproducible build
matching the deployed hash, which this layer does not perform. The claim is
recorded as `SchemaProvenance`, never inferred.

**`evidence`** owns measurement: what an execution did, what moved, and what a
boundary proof established. Nothing in it may name a deposit, a withdrawal, a
borrow, a health factor or a liquidation.

**`protocol`** owns meaning, and after U1 owns *only* meaning: which
instructions a program admits, what its accounts are for, which quantities are
economically real, and what a difference between two builds is worth.

## What the adapters still own, deliberately

This is the half of the phase that matters most, because the tempting failure is
to move too much.

**Admission (`accept`) was not touched.** It is the second-largest block of
duplicated-*looking* code, at 52% line overlap — and the overlap is
`anyhow::ensure!` call syntax, not shared meaning. The predicates inside are the
two protocols' entire replay contracts: which discriminants, which account
counts, which companion instructions, which CPI targets. A shared "instruction
shape validator" would become a place to express one protocol's rules in
another's vocabulary. `accept` measures 153 → 154 lines in the stake-pool adapter
and 71 → 71 in Token-2022; the single added line is the shared System-instruction
decoder replacing a hand-rolled discriminant check.

**Every economic formula stayed.** `Fee::apply`,
`StakePool::pool_tokens_for_deposit`, `lamports_for_withdrawal`, the transfer-fee
cap boundary, `sol_per_pool_token`. None of it moved, and none of it should:
these are protocol math with their own correctness argument.

**The boundary contract's protocol half stayed.** The generic prover proves; the
adapter declares. Three things only the adapter can supply:

- **which token program** a validator-observed balance must be attributed to.
  Stake Pool requires `TokenkegQ…`, Token-2022 requires `TokenzQdB…`, and proving
  a balance against the wrong program proves nothing.
- **which accounts are exempt** from read-only byte identity. Stake Pool exempts
  Clock and StakeHistory because its `WithdrawSol` names them and the runtime
  rewrites them every slot. Token-2022 exempts *nothing*, and must not: no
  instruction in its contract reads a sysvar, so one appearing there is an
  anomaly rather than a known rewrite. A prover that exempted sysvars
  unconditionally would silently weaken the narrower guarantee.
- **the protocol's own accounting identity.** The stake pool's `total_lamports`
  change must equal the reserve's validator-observed lamport change, and its
  `pool_token_supply` change must equal the observed change in holdings of the
  pool mint. That is the strongest evidence in the adapter and is irreducibly
  its own.

**Two decoders, not one permissive one.** `spl_token::decode_account` accepts
exactly 165 bytes; `token2022::decode_account` accepts the extended forms. These
look like a candidate for unification and are not: the stake-pool contract admits
legacy token accounts only, and one lenient decoder would let one program's
extended state be read under another program's narrower claim.

## Universal ≠ complete

There is no `fully_understood` flag, and there must not be. The universal layer
states what it measured. Whether a protocol's semantics are covered is a separate
question with a separate answer — `evaluable_subjects`, checked against emitted
findings, and `no_semantic_coverage` as exit 2 rather than a pass.

## Token flow is not semantic truth

A flow graph is a measurement primitive and never the semantic source.

- A **mint** can be a receipt, a debt, an LP share or a reward. The core does not
  decide.
- A **transfer to a named account** can be a payout or a fee. Stake Pool's
  manager fee account receives a mint on deposit, and naive "the user received
  tokens" attribution counts the protocol's fee as the depositor's proceeds.
- A **`withdraw`-shaped instruction** can be a protocol collecting its own fees.

So `TokenBalanceDelta` is named for what it is, and nothing in `evidence/` is
named `deposit`, `payout` or `fee`.

## No flow does not mean no change

This is the hard invariant, and it is enforced by API shape rather than by
comment.

Drift's `settlePNL` moves value between a user's position and a per-market P&L
pool. All collateral sits in one global vault; settlement is accounting. **Every
flow primitive in this layer measures exactly zero**, and a user's economic
position has genuinely changed.

Therefore `FlowEvidence` has:

- no `is_empty`
- no `unchanged`
- no `is_clean`
- and no method whose name invites the reading "nothing happened"

The only accessor for an empty set is `no_flow_observed()`, whose documentation
says it reports a property of the *measurement*, not of the economics, and that
it may be used to describe coverage but never to conclude safety.

`UniversalEvidence` likewise carries no overall verdict — no severity, no
`changed`, no `passed`.

Guarded by `an_empty_universal_flow_set_is_not_an_empty_change_set`
(`engine/tests/universal_evidence.rs`) and
`a_measured_absence_of_flow_is_not_an_absence_of_change` (`evidence/mod.rs`).
Mutation 6 of the phase's mutation suite injects exactly this mistake — `derive`
discarding account evidence when flows are empty — and both tests fail.

## Error and unknown semantics

`Option` was not enough, because "these bytes are not a token account" and "these
bytes claim to be a token account and are unreadable" are different facts.

```rust
pub enum Decoded<T> {
    Decoded(T),
    NotApplicable,                    // never this kind of account; not an error
    Malformed(MalformedReason),       // present and wrong; never a zero
    Unsupported(&'static str),        // recognised, carrying something unmodelled
}
```

`MalformedReason` distinguishes truncation, an unexpected length, an
out-of-range discriminant, and a TLV entry running past the end of its buffer.

Three consequences that are tested:

- **A truncated buffer never reads as zero.** Seven zero bytes are not a zero
  `u64`. This is the failure mode that makes a malformed token account
  indistinguishable from an empty one.
- **An unknown Token-2022 extension stays visible.** It is listed as
  `unrecognized`, carrying its type number and raw bytes. A mint that gained a
  pause authority this build has never heard of must not read as a mint with
  nothing unusual about it.
- **A malformed extension list records why the walk stopped.** The tolerant walk
  still returns what it read — extension parsing informs the report while the
  economic verdict rests on base fields and proved balances — but `truncated_at`
  now says the list ended badly, where before a short list was
  indistinguishable from a complete one.

## Lamport deltas are attributed, not assumed

Not every lamport delta is an economic transfer. A fee payer loses lamports on a
transaction that does nothing; a rent-exempt account gains them when created; the
runtime rewrites sysvars every slot.

`LamportDelta::attribution` is one of `FeePayer`, `RuntimeManaged`, `Lifecycle`
or `Unattributed`, and `may_be_value_movement()` is false for the first three.
`Unattributed` is the honest default — it means nothing known disqualifies this
delta, not that it is economically meaningful. That judgement is the adapter's.

## Pairing rules, stated

Each has a plausible-looking wrong answer:

| rule | the wrong answer it prevents |
|---|---|
| accounts pair by **address-backed label**, never by position | comparing a mint against a token account and calling the difference economic |
| an account absent before and present after is **created** | attributing a fresh rent deposit to the protocol as a transfer |
| an account present before and absent after is **closed** | the same, inverted |
| an account in neither produces **nothing** | inventing an observation from an absence |
| a token account whose **mint or program changed** is not a balance delta | subtracting two different assets |

## Provenance

Every evidence item carries a `Provenance`: the record, the account label and
address, the `DecoderIdentity` that read it — name, version and
`SchemaProvenance` — and the invocation it is attributable to where one applies.
Evidence divorced from its proof context is an assertion, and this tool's whole
premise is that an assertion is not a measurement.

Evidence types serialize and deliberately **do not deserialize**. A `Deserialize`
on derived evidence would imply a wire format that could be handed to the engine
in place of a measurement.

## Purity

Every function in `evidence/` and `standard_programs/` is a pure function of
already-captured evidence. No RPC. No environment. No IDL fetching. No clock. No
global state. That is what makes derivation replay-safe and a canonical report
reproducible.

## What was explicitly not built

Per the phase brief and the architecture study's conclusion: no Adapter Contract
v2, no adapter DSL, no IDL auto-discovery, no AI adapter generation, no new
protocol adapter, no new `SemanticAction` variants, no economic flow graph as a
semantic source, and no new protocol-specific economics.

`FieldSchema` has exactly the inhabitants `FieldDecoder` had — `Opaque` and
`FixtureLending` — because a schema source with no implementation behind it would
be a claim with no test. The boundary is cleaner; nothing was added behind it.

The generic diff still receives `FieldDecoder::None` for every adapter record,
and must continue to. An adapter record's economics come from
`ProtocolAdapter::interpret`; a token-aware generic diff would start rating
adapter-owned changes itself, which is exactly the under-reading that
`--fail-on-critical`'s `economic_findings` clause exists to correct.

## Results

| | before | after |
|---|---:|---:|
| Adapter shared-method overlap | 46% (303 lines) | **31% (159 lines)** |
| `prove_boundaries` (stake pool / Token-2022) | 248 / 137 | **162 / 49** |
| `interpret` | 51 / 53 | **15 / 18** |
| `labels` | 28 / 35 | **18 / 27** |
| Adapter production LOC | 1452 / 846 | **1303 / 612** |
| Duplicated `u64_at`, `address_at`, `balance_at`, TLV walk | present | **gone** |

The overlap that remains in `prove_boundaries`, `labels` and `interpret` is now
function signatures, braces and call syntax — not logic.

`accept` is unchanged, as intended.

Canonical reports are byte-identical across all six gate cases, the production
bundle still verifies, and the baseline still exits 0 with report sha256
`7be26a66f3e98f9b96ba6f1270003c158d99968d9ed081c336a97f14782d2099`.
