# Protocol comparison matrix

The same eight questions asked of four materially different protocols. Sources
are the Eplyx code for Stake Pool, and upstream program source, IDL and
documentation for the rest. Where a claim rests on documentation rather than on
bytes Eplyx has executed, it says so — that distinction is the whole subject of
section H below.

## A. Interface availability

| | Stake Pool | Orca Whirlpools | Kamino KLend | Drift v2 | Phoenix v1 |
|---|---|---|---|---|---|
| Anchor program | no | yes | yes | yes | **no** |
| Published IDL | no | yes | yes | yes | yes |
| IDL has account types | — | yes, partly | yes | yes | **no (0)** |
| IDL has events | — | yes | yes | yes | **no (0)** |
| Generated SDK | yes (SPL) | yes (Codama) | yes (klend-sdk) | yes | yes |
| Open source | yes | yes | yes | yes | yes |
| Eplyx has executed it | **yes** | no | no | no | no |

Only the first column describes a protocol Eplyx has actually replayed. Every
other column is a claim about an interface, not about behaviour.

## B. Instruction identification

| | Stake Pool | Orca | Kamino | Drift | Phoenix |
|---|---|---|---|---|---|
| Discriminator scheme | 1-byte enum tag | 8-byte Anchor | 8-byte Anchor | 8-byte Anchor | 1-byte enum |
| Decodable generically | yes, once told | yes | yes | yes | yes, from IDL |
| Names available | from source | from IDL | from IDL | from IDL | from IDL |
| Arguments typed | from source | yes | yes | yes | yes |

Instruction identity is the one thing available everywhere. It is also the
cheapest and least meaningful: knowing an instruction is called `withdraw` says
nothing about whose money moves.

## C. Account understanding

| | Stake Pool | Orca | Kamino | Drift | Phoenix |
|---|---|---|---|---|---|
| Account names in context | source | yes | yes | yes | IDL args only |
| writable/signer roles | from message | yes | yes | yes | from message |
| Account *layout* | hand-written in adapter | partial — `tick_array_*` and `oracle` are `UncheckedAccount` | yes | yes | **none** |
| PDA seeds | source | yes | yes | yes | source |
| Ownership derivable | yes | yes | yes | yes | yes |

Two different failures hide under "account understanding". Phoenix gives no
layout at all. Orca gives layout for the accounts a swap moves and withholds it
for the accounts a swap *depends on*.

## D. Event understanding

| | Stake Pool | Orca | Kamino | Drift | Phoenix |
|---|---|---|---|---|---|
| Events in IDL | no | yes (`Traded`) | yes | yes | no |
| Logs | yes | yes | yes | yes | yes |
| Return data | no | no | no | no | no |
| Custom decoder needed | n/a | no | no | no | yes |

An event is a statement by the program under test. For a differential gate this
is a specific hazard: the candidate build emits the event. A finding sourced
only from an event cannot detect a candidate that changed both the economics and
the event consistently. Events are usable as corroboration, never as the sole
measurement.

## E. Universal execution primitives present

| primitive | Stake Pool | Orca | Kamino | Drift | Phoenix |
|---|---|---|---|---|---|
| SPL token transfer | yes | yes | yes | at deposit/withdraw only | yes |
| Token-2022 | no | yes | yes | yes | no |
| Mint / burn | yes (pool token) | yes (position NFT) | yes (cToken) | no | no |
| SOL transfer | yes | no | no | no | no |
| Stake movement | yes | no | no | no | no |
| Account create/close | outside contract | yes (position) | yes (obligation) | yes | yes (seat) |
| Authority/delegate mutation | no | no | no | no | no |
| **Change with no token flow** | no | no | partly (refresh) | **yes (`settlePNL`)** | yes (order placement) |

## F. Semantic action inference

| | from interface alone | from execution alone | from both | safely inferable? |
|---|---|---|---|---|
| Stake Pool `DepositSol` | name is suggestive | SOL in, pool token out | yes | **yes** |
| Orca `swap` | name is decisive | token A out, token B in | yes | **yes** |
| Orca `increase_liquidity` | name is decisive | two tokens in, no receipt token | yes | **yes, but not as Deposit** |
| Kamino `deposit_reserve_liquidity` | name is decisive | token in, cToken out | yes | **yes** |
| Kamino `borrow_obligation_liquidity` | name is decisive | token out, no token in | depends on prior refresh | **partly** |
| Drift `settlePNL` | name is opaque | **nothing observable** | no | **no** |
| Phoenix `PlaceLimitOrder` | name is decisive | token moves to seat | yes | **yes** |
| Phoenix `Swap` | name is decisive | token flow | yes | yes |

Seven of eight are inferable from name plus flow. The eighth is the one that
matters, because it is the one where a wrong answer is silent.

## G. Semantic subjects

| protocol | directly measurable | needs protocol formula |
|---|---|---|
| Stake Pool | `pool_tokens_received`, `pool_tokens_burned`, `sol_received_by_user` | exchange rate, fee share |
| Orca | `input_token_spent`, `output_token_received`, `vault_a_delta`, `vault_b_delta` | `position_liquidity`, `fees_owed`, price impact, tick crossing |
| Kamino | `liquidity_deposited`, `ctokens_received`, `debt_token_received` | `debt_increased` (index-normalised), `health_factor`, `borrow_factor_adjusted_debt_value` |
| Drift | (none from flow) | `base_asset_amount`, `quote_asset_amount`, `margin_requirement`, `unrealized_pnl` |
| Phoenix | `base_lots_filled`, `quote_lots_paid` | `order_book_depth`, queue position |

The pattern is consistent and it is the useful result of this matrix: **what a
user hands over and what they receive is measurable almost everywhere; what it
is worth, and whether they are now at risk, is protocol math almost everywhere.**

## H. Required custom logic

| protocol | verdict |
|---|---|
| Stake Pool | small deterministic hook — share/lamport arithmetic, ~79 measured lines |
| Orca swap | none for flow; significant evaluator for price impact and fee attribution |
| Orca liquidity | significant — tick/liquidity math |
| Kamino deposit/withdraw | small — cToken exchange rate |
| Kamino borrow/liquidate | significant — index normalisation, oracle, borrow factor |
| Drift | significant, and it is the *only* source of meaning |
| Phoenix | significant — no layout, so the layout itself is custom |

## Interface source, and what may be trusted from it

| source | instruction identity | argument types | account layout | events | proves it matches deployed bytes |
|---|---|---|---|---|---|
| `CanonicalOnchainIdl` | trust | trust | trust if present | trust | **no** — the upgrade authority published it, which is the same authority that can deploy something else |
| `LegacyAnchorIdl` | hint | hint | hint | hint | no |
| `PublishedRepositoryIdl` | hint | hint | hint | hint | no |
| `GeneratedSdkInterface` | hint | hint | hint | hint | no |
| `VerifiedSourceDerived` | trust | trust | trust | trust | **only with a reproducible build matching the deployed hash** |
| `StandardProgramInterface` (SPL Token, System, Stake) | trust | trust | trust | n/a | yes — the runtime or a pinned dependency binary |
| `ManualInterface` | trust as far as it was reviewed | same | same | same | no, but the association is recorded |
| `Unknown` | nothing | nothing | nothing | nothing | n/a |

The row that matters is the first. **A canonical on-chain IDL is published by the
upgrade authority, and the upgrade authority is precisely the party whose change
Eplyx exists to measure.** An IDL is evidence about intent, never about bytes.
Eplyx already resolves the program bytes deployed at a slot and pins their hash;
an interface is only as trustworthy as its binding to *that* hash.

This gives a provenance ladder that should be recorded, never inferred:

```
ExactVerifiedBuild            source compiles to the deployed hash
UpgradeAuthorityPublished     on-chain IDL at the deployed slot
ProgramIdOnly                 an interface claims this program id
RepositoryClaim               a repository says it is this program
ManualAssociation             a person asserted it, and is named
```

Only `ExactVerifiedBuild` and `StandardProgramInterface` justify a layout being
trusted enough to drive a hard finding. Everything below is a hint that may
propose a candidate mapping for deterministic verification.
