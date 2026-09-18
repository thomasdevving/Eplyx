# Phase U1 — duplication map

Written before any code was edited, as the phase brief requires. Every number
comes from `scripts/measure-adapter-duplication.py` run against the two adapters
at commit `029811d`; every classification names the specific lines behind it.

## Measured overlap, reproduced

Line-identical overlap between `stake_pool.rs` (A) and `token2022.rs` (B),
production code only, comments and blank lines excluded, counted as a multiset
so a line repeated N and M times contributes `min(N, M)`:

| method | A | B | identical | of B |
|---|---:|---:|---:|---:|
| `address_at` | 3 | 3 | 3 | 100% |
| `label` | 6 | 6 | 6 | 100% |
| `program_id` | 3 | 3 | 3 | 100% |
| `u64_at` | 5 | 5 | 5 | 100% |
| `interpret` | 51 | 53 | 45 | 84% |
| `mint_decimals` | 5 | 5 | 4 | 80% |
| `balance_at` | 9 | 9 | 7 | 77% |
| `prove_boundaries` | 248 | 137 | 106 | 77% |
| `labels` | 28 | 35 | 24 | 68% |
| `token_account_amount` | 3 | 3 | 2 | 66% |
| `token_account_mint` | 3 | 3 | 2 | 66% |
| `economic_entity_id` | 15 | 10 | 6 | 60% |
| `accept` | 153 | 71 | 36 | 50% |
| `boundaries` | 43 | 49 | 13 | 26% |
| `state_features` | 43 | 77 | 17 | 22% |
| `decode` | 52 | 125 | 4 | 3% |
| **total** | **704** | **654** | **303** | **46%** |

This reproduces the research phase's headline: **46% of the second adapter's
shared-method code is line-identical to the first's.** The figure is the control
for the re-measurement the phase requires at completion.

## Classification

The brief's four buckets, applied to the blocks it names. The rule applied
throughout: *move it only if the semantics are protocol-independent* — similarity
alone is not evidence of that.

### UNIVERSAL — move to `evidence/`

**Snapshot-to-message index lookup** (`prove_boundaries`, both). Finding a
snapshot's position among the message's account keys is a property of a Solana
message. Identical in both, 8 lines each.

**Lamport boundary proof** (`prove_boundaries`, both). "Every archived snapshot's
lamports equal the validator's pre/post balance at its index" is a statement
about validator metadata, not about a protocol. ~35 identical lines, differing
only in the wording of the failure.

**Read-only byte identity** (`prove_boundaries`, both). "An account the message
marks read-only must be byte-identical across the boundary" is a runtime
property. ~20 lines, near-identical; the *exemption list* is not universal (see
below).

**Pre/post account pairing** (`interpret`, both). Looking up two executions'
post-states by label, decoding each, walking the economic fields of one against
the other, skipping equal values. 45 of 53 lines identical. The decode and the
rescale are protocol hooks; the walk is not.

**Label assignment** (`labels`, both). Default `key-{n}`, bind roles by
instruction position, first role wins, name key 0 `payer` if unclaimed. 24 of 35
lines identical.

**`balance_at`** — find a validator token balance by account index. 7 of 9 lines
identical, and the two differences are formatting.

**Little-endian and base58 readers** (`u64_at`, `u16_at`, `address_at`) — 100%
identical, and not even protocol-adjacent.

**CPI traversal.** Already generic in `executor::CpiCall`, but flat: there is no
parent link and no typed view. Formalising it is universal work that neither
adapter should own.

### STANDARD-PROGRAM-SPECIFIC — move to `standard_programs/`

**SPL Token account layout.** `TOKEN_ACCOUNT_LEN = 165` in `stake_pool.rs`,
`ACCOUNT_LEN = 165` in `token2022.rs`; `token_account_amount`, `token_account_mint`,
`mint_decimals`, `mint_supply` defined twice. This is the SPL Token program's
layout, published and stable. Neither adapter owns it.

**Token-2022 TLV extensions.** `extensions()`, `extension_name()` and the
transfer-fee-config offsets exist only in `token2022.rs` today — so this is not
duplication yet, it is *pre-duplication*: the third adapter that touches a
Token-2022 mint reimplements it. Moving it now is the point of the phase.

An important non-obvious distinction the shared decoder must preserve rather
than smooth over: **the two adapters accept different things on purpose.**
`stake_pool` decodes a token account only at exactly 165 bytes, because it deals
with legacy SPL Token accounts; `token2022` accepts 165, 82, or a longer buffer
tagged by its account-type byte. Collapsing these into one permissive decoder
would silently widen what the stake-pool adapter claims to read. Two entry
points, one layout.

### PROTOCOL-SPECIFIC — stays in the adapter

**Which accounts must be proved, and against which token program.** Stake Pool
requires `TokenkegQ…`; Token-2022 requires `TokenzQdB…`. Not a detail — proving
a balance against the wrong program proves nothing.

**The sysvar exemption.** Stake Pool exempts Clock and StakeHistory from
read-only byte identity because its `WithdrawSol` names them and the runtime
rewrites them every slot. Token-2022 exempts nothing, and must not: no
instruction in its contract reads a sysvar, so a sysvar appearing there is a
real anomaly. A generic prover that exempted sysvars unconditionally would
weaken Token-2022's guarantee.

**Pool corroboration.** `total_lamports` against the reserve's observed lamport
change, `pool_token_supply` against observed holdings of the pool mint. This is
the stake pool's own accounting identity and is the strongest evidence in the
adapter. It is irreducibly protocol-specific.

**Admission** (`accept`). 50% line-identical, and the identical half is
`anyhow::ensure!` scaffolding rather than shared meaning. The predicates —
which discriminants, which account counts, which companion instructions, which
CPI targets — are the protocol's contract. Deliberately **not moved**.

**Every economic formula.** `Fee::apply`, `pool_tokens_for_deposit`,
`lamports_for_withdrawal`, the transfer-fee cap boundary, `basis_points`,
`sol_per_pool_token`. Untouched.

**`decode`** at 3% overlap. The two decode different protocols. Untouched except
where it calls the shared layout readers.

### ACCIDENTALLY-DUPLICATED — deduplicate without moving semantics

**Assumption strings.** Both adapters emit prose describing what the proof
established. The sentences differ and *should* differ — they describe different
contracts. What is accidental is that each adapter rebuilds the list from
scratch. The generic prover returns the universal sentences; the adapter appends
its own.

**`label`** (100% identical, 6 lines): `self.labels(tx).get(i).cloned()`
defaulting to `key-{i}`. Pure boilerplate over the real method.

## What this phase must not do

`accept` is the largest single block of duplicated-looking code after
`prove_boundaries`, and it is the one most tempting to unify. It must not be
unified. 36 of its 71 shared lines are `anyhow::ensure!` and `anyhow::bail!`
call syntax; the conditions inside them are the two protocols' entire admission
contracts, and a shared "instruction shape validator" would become a place to
express one protocol's rules in another's vocabulary. Left alone deliberately,
and the completion measurement will show `accept` unchanged.

Equally: the generic diff must not gain a token decoder. `FieldDecoder::None`
for adapter records is load-bearing — an adapter record's economics come from
`ProtocolAdapter::interpret`, and a token-aware generic diff would start rating
adapter-owned changes itself, which is precisely the failure
`--fail-on-critical`'s `economic_findings` clause exists to correct.
