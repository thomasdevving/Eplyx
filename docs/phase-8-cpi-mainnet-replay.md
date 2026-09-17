# Phase 8 — CPI-aware mainnet upgrade replay

Implemented: deterministic discovery and historical resolution of every program
a transaction needs, including those it reaches only through cross-program
invocation; same-slot interference screening that names the conflicting
transaction; account provenance recorded per snapshot; an invocation graph
recorded from validator metadata and compared as part of the fidelity gate; and
an adapter for a stateful protocol whose economics are *computed* from pool
state rather than named in the instruction.

The phase answers one question end to end:

```text
real stateful protocol        SPL Stake Pool, upgraded in place on mainnet
+ real historical state       a pool, its reserve stake account, its mint and
                              two token accounts, at slot S-1
+ real historical transaction a direct DepositSol of 0.423 SOL
+ real CPI dependencies       the System program, and the SPL Token deployment
                              that was live at that slot
+ actual deployed V1 binary   the stake-pool deployment live at slot S
+ genuine V2 binary           the deployment the real upgrade installed
→ does the upgrade change how many pool tokens the depositor receives?
```

It does not. That is the result, and it is a legitimate one. Because a preserved
outcome cannot by itself show that a *changed* outcome would be caught, the same
interaction is also run against a deliberately regressed candidate, which is
detected and blocks the gate.

## The target

| | |
|---|---|
| Program | `SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy` (SPL Stake Pool) |
| Instruction | `DepositSol` (variant 14), 0.423000000 SOL |
| Signature | `58d7oY3zMFRSjcbEZErYYjxEuLEWzNSXphvHBfGNZ78hVznzmLnY1hPNhG8LpV83EYcha8wQn6XifPS53bxR1pgs` |
| Slot | 429,878,778 (block time 2026-06-30T12:30:45Z) |
| Message version | legacy, no address lookup tables |
| Pool | `CV6bkrUksMwcEC4jfLTJsbHwF3Y2YurZdWWua95Fpbtd` |
| Pool token mint | `7cBuurYDdaqxnem7KyMTci6SWhjJKroZ6NUjqH2ewEPB`, 9 decimals |
| Reserve stake account | `uR3aXvCwZ5WnhNfXYYaeWvrg1ZTnF5uuAVpLDeDzFcB`, owned by the stake program |
| Deployed V1 | slot 370,300,186, sha256 `a8571c781982c72c2531f5957aab7d902df35cb3c1ed0d34829f03f9f0166770` |
| Deployed V2 | slot 429,882,117, sha256 `ec2dfefaa70d560754a0000f39bd2cabc192b895d36205b3c428f601b6e1d7e1` |
| Upgrade distance | 3,339 slots after the transaction |

The transaction has four top-level instructions - two compute-budget
instructions, a System transfer funding an ephemeral signer, and the deposit -
twelve message keys, six of which carry state the replay reconstructs, and two
cross-program invocations.

`DepositSol` is worth replaying because the economically meaningful number is
not in the instruction. The instruction says how many lamports go in; how many
pool tokens come back is computed from `pool_token_supply` and `total_lamports`
at the moment of execution. A share calculation is exactly the kind of thing an
upgrade changes by a rounding step, in a way that no byte diff explains and no
fee schedule announces.

## The CPI graph

Recorded from `innerInstructions` in validator metadata, with the depth and the
owning top-level instruction that a flattened list cannot express:

```text
SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy
├─ 11111111111111111111111111111111             (instruction 3, depth 2, 2 accounts, 12 bytes, discriminant 2)
└─ TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA  (instruction 3, depth 2, 3 accounts,  9 bytes, discriminant 7)
```

A System transfer moves the lamports into the reserve stake account; a `MintTo`
signed by the pool's withdraw authority credits the depositor. The stake program
is *not* invoked: a SOL deposit credits the reserve's lamports and leaves its
delegation untouched. The reserve is still required state, because the pool's
`total_lamports` is checked against its balance.

## How dependency binaries are resolved

Programs are discovered from four independent routes and merged, because no one
route is sufficient. The set of top-level instruction *programs* alone would miss
the SPL Token program entirely: it is the program of no top-level instruction in
this transaction, only the target of an invocation from inside one.

| Route | What it contributes here |
|---|---|
| top-level instructions | stake pool, System, compute budget |
| inner instructions | System, SPL Token |
| execution logs (`Program <id> invoke [n]`) | all four, as a cross-check |
| adapter declaration | System, SPL Token, as a floor |

Each discovered program is then read *at the transaction's predecessor slot* and
classified by the owner of its account at that slot, rather than against a
hard-coded list of addresses:

```text
11111111111111111111111111111111             builtin            owned by the native loader; the validator implements it
ComputeBudget111111111111111111111111111111  builtin            owned by the native loader; the validator implements it
SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy  historical-mainnet deployed at slot 370300186
TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA  historical-mainnet deployed at slot 419472000
```

The SPL Token entry is the point of the exercise. SPL Token was migrated to the
upgradeable loader and redeployed at slot 419,472,000 - before this transaction,
but well after many transactions a corpus might contain. Loading "the SPL Token
program" without qualification would silently replay some transactions against a
binary that did not exist when they ran.

### Built-ins

LiteSVM supplies the native programs from `solana_builtins::BUILTINS`, and it
also ships ELF copies of several SPL programs - token, token-2022, memo, the
associated-token-account program and a Core BPF stake program - which it loads by
default. Those defaults are convenience, not history: the bundled SPL Token is
whichever version LiteSVM vendored.

So a dependency classified `historical-mainnet` is always loaded explicitly, from
the bytes read at the transaction's slot, under the loader that owned it at that
slot. That load replaces LiteSVM's default for the same address. A dependency
classified `builtin` is never substituted, because there is nothing to
substitute: its semantics live in the runtime.

### Feature activation

LiteSVM's feature set is a mainnet snapshot, not a slot-accurate reconstruction.
Both builds run under the identical set, so it cannot produce a *difference*
between them - and V1 reproducing the original fee, invocation graph and
post-state is what rules out a material drift. This is recorded as an assumption
on every record rather than claimed as a proof.

## Exact pre-state acquisition

Every message key that is not executed is state the replay has to reproduce. The
account archive is read at exactly `S-1` and `S`, and each snapshot carries its
provenance:

| Account | Role | Discovered by | Source |
|---|---|---|---|
| `CV6bkr…` | stake pool | instruction meta, adapter | historical archive |
| `uR3aXv…` | reserve stake | instruction meta, inner instruction, adapter | historical archive |
| `Epome…` | destination pool token | instruction meta, inner instruction, adapter | historical archive |
| `93eSDR…` | manager fee | instruction meta, adapter | historical archive |
| `7cBuur…` | pool mint | instruction meta, inner instruction, adapter | historical archive |
| `9DtcNy…` | payer | instruction meta | historical archive |
| `4WFPz6…` | depositor (ephemeral signer) | instruction meta, inner instruction, adapter | absent at both boundaries |
| `ECVFhd…` | withdraw authority | instruction meta, inner instruction, adapter | absent at both boundaries |

The two absent accounts held nothing at either boundary and the validator's
balances agree. Recording them as absent is more faithful than inventing a
snapshot: the runtime materializes them, exactly as it did originally.

### What the snapshots are proved against

Validator metadata proves balances, not arbitrary account data. For a token
account that is enough - `preTokenBalances` and `postTokenBalances` pin the exact
base-unit amount. For a pool's internal bytes it is nothing at all, which would
leave the pool resting on the archive alone. It does not have to:

- the pool's `total_lamports` change must equal the validator-observed lamport
  change of the reserve stake account;
- the pool's `pool_token_supply` change must equal the validator-observed change
  in holdings of the pool mint.

Both are checked during acquisition. A mismatch is an error naming which side
failed, never an approximation.

## Same-slot screening

An archive answers "what did this account hold at the end of slot N". A replay
needs "what did it hold immediately before transaction T in slot S". Those are
the same question only when no other transaction in slot S touches the account.

Phase 7 caught violations after the fact, through a boundary proof that failed.
That is adequate for two data-empty System accounts. It is not adequate for a
dozen accounts reached through CPI, because the evidence behind each of them is
uneven. So Phase 8 reads the block and names the conflict:

```text
Same-slot screening: 6 required account(s) against 1503 transactions in slot 429878778;
no conflicts (target at index 67)
```

The criterion is deliberately blunt: any other transaction in the slot that takes
a required account as writable makes the boundary ambiguous, whether or not it is
visible to have changed anything. A writable account can be rewritten with no
lamport movement and nothing in block metadata would show it.

This is why candidate selection is real work rather than a formality. The obvious
first candidates - large-pool deposits routed through an aggregator - all carry a
Jito tip transfer, and tip accounts are written by dozens of transactions per
slot. Screening rejects every one of them, naming the tip account and the
transaction that wrote it first. A record that carries a CPI-admitting adapter
and no screening evidence does not validate at all.

## V1 fidelity

The gate is unchanged in shape and stricter in content:

```text
historical transaction → exact pre-state → actual deployed V1 → replay
  → compare against the original mainnet outcome
      success/failure   ✓
      fee               ✓  14,000 lamports
      invocation graph  ✓  two calls, same programs, same depths, same shapes
      post-state hash   ✓  12b142de86457abc3207f24919c0964d46ffb77a0652c425d41b5f14b7594918
  → MATCH? no → abort, candidate execution withheld
             yes → allow V2 comparison
```

The invocation graph is the new criterion and it is not decorative: a replay can
land on the right post-state by luck far more easily than it can make the same
calls, to the same programs, at the same depths, from the same instruction. A
test removes one recorded invocation while leaving the post-state correct, and
the gate rejects it.

`Exact` is reserved for a controlled snapshot whose pre-state the harness itself
created. An archive-sourced record that satisfies every criterion above reports
`Matched`, which is the strongest verdict historical state admits. Both allow the
candidate to run; anything else aborts.

## Result: the real upgrade

```text
mainnet-spl-stake-pool-e8ff02974848b698 | slot 429878778 | HistoricalArchive | fidelity Matched
  pre 1d548060540eacae2c559d856bd85a6267c3717082a15d40e168e9f1519715ec
  V1  12b142de86457abc3207f24919c0964d46ffb77a0652c425d41b5f14b7594918 (matches original)
  V2  12b142de86457abc3207f24919c0964d46ffb77a0652c425d41b5f14b7594918

PROTOCOL RESULT
    sol_deposited              V1            0.423000000   V2            0.423000000
    pool_tokens_received       V1            0.395906603   V2            0.395906603
    manager_fee_pool_tokens    V1            0.000000000   V2            0.000000000
    pool_tokens_minted         V1            0.395906603   V2            0.395906603
    pool_token_supply_after    V1       291875.877314830   V2       291875.877314830
    pool_total_lamports_after  V1       311850.055457947   V2       311850.055457947
    sol_per_pool_token         V1            1.068433809   V2            1.068433809

PROTOCOL ECONOMICS
  No protocol-level field differs between the two builds.

  Compute units (tracked separately)
    1 of 1 fixtures differ, range -1.44% .. -1.44%
```

0.395906603 pool tokens is what mainnet actually minted: the replayed `MintTo`
carries the same amount the validator recorded. Compute moved by -1.44% and is
off the pass/fail axis, as it has been since Phase 1 - any recompilation moves
it.

Whether this upgrade was *intended* to preserve behaviour is deliberately not
classified. The phase quantifies the change; expected-versus-unexpected
classification is a later phase.

## Result: the deliberately regressed candidate

`programs/fixture-stake-pool-candidate` is a locally constructed counterexample -
not a proposed upstream release, not a claim about any real deployment, and not
code to deploy. It implements `DepositSol` with the same account layout, the same
validation and the same two cross-program invocations, and differs in exactly one
place: instead of `lamports * pool_token_supply / total_lamports` in `u128`, it
precomputes the exchange rate once at four decimal places and multiplies by that.
This is the shape a "cache the rate" refactor takes.

```text
PROTOCOL RESULT
    sol_deposited              V1            0.423000000   V2            0.423000000
    pool_tokens_received       V1            0.395906603   V2            0.395885700   delta -0.000020903
    pool_tokens_minted         V1            0.395906603   V2            0.395885700   delta -0.000020903
    pool_token_supply_after    V1       291875.877314830   V2       291875.877293927   delta -0.000020903
    pool_total_lamports_after  V1       311850.055457947   V2       311850.055457947

PROTOCOL ECONOMICS
  1 of 1 observation(s) changed economically.
    pool-mint (mint) supply                       -0.000020903
    stake-pool (stake-pool) pool_token_supply     -0.000020903
    destination-pool-token (token-account) amount -0.000020903

CI gate exit status: 1 (blocked)
```

Both executions succeed. Both make the same two calls, to the same programs, at
the same depths. The invocation graph is identical. The depositor receives
0.000020903 fewer pool tokens - 53 parts per million of the deposit - and the
difference stays in the pool. The loss is bounded by one part in ten thousand of
the rate, so it scales with the deposit rather than being a fixed skim, which is
what makes it read as a rounding change rather than as theft.

### Two channels, still separate

The generic classifier sees the depositor's token account change and rates it
`RawDataChanged` - a warning, not a critical. It is protocol-agnostic by design
and cannot know those bytes were a balance. The adapter is what knows, and
`--fail-on-critical` trips on either channel. Teaching `diff` about tokens to
"fix" the severity would put protocol semantics in the protocol-agnostic layer,
which is the seam this whole design exists to keep.

## Architecture: is the seam still clean?

`ProtocolAdapter` now has three implementations' worth of pressure on it -
fixture lending (through the pre-trait inline contract), Token-2022, and the
stake pool - and grew four methods in this phase:

| Added | Why it belongs to the adapter |
|---|---|
| `supports_cpi` | whether *this* protocol's contract admits invocation is protocol knowledge; Token-2022 keeps the narrower guarantee it was proved under |
| `dependency_programs` | which programs a protocol reaches is protocol knowledge |
| `required_accounts` | what a program reads is protocol knowledge |
| `summarize` | which quantities a protocol's users would recognize is protocol knowledge |

All four have defaults that make an adapter written before this phase behave
exactly as it did. The generic engine kept execution, raw state capture, generic
diffing, replay fidelity, hashing and report plumbing; `dependencies`,
`screening`, `executor` and `diff` contain no protocol names.

One thing did get narrower rather than wider: `corpus`, `interpret`, `impact`,
`cluster` and `shrink` remain fixture-lending-specific and are still not reached
by adapter records. An adapter record's economics come from `interpret` and
`summarize`, and the report suppresses the position/USD block for them.

## Performance baseline

Measured on the demo, macOS, debug build of the engine. No optimization work has
been done and none is warranted yet.

| Step | Cold (network) | Cached (offline) |
|---|---|---|
| Upgrade search over 5,000,000 slots | — | 100 ms |
| Version resolution, 1,080,464-byte ELF | 2,135 ms | 94 ms |
| Version resolution, 108,600-byte ELF | 693 ms | — |
| Acquisition: transaction, 16 boundary reads, dependency resolution, block screen | 8,446 ms | 507 ms |
| Dependency bundle load and hash check | — | 7 ms |
| V1 replay | — | 57 ms |
| V2 replay | — | 51 ms |
| Full comparison, one record | — | 180 ms |
| Whole demo, second run | — | 4.4 s |

Request and volume counts, which are more durable than wall clock:

| | |
|---|---|
| Account-archive requests | 57 (chunked ELF reads dominate) |
| Transaction requests | 2 |
| Block requests | 2 (8.9 MB of block JSON) |
| Accounts loaded per replay | 6 |
| Program dependencies | 4 — 2 loaded from history, 2 runtime-provided |
| VM executions per comparison | 2 |

Cold timings are a property of the endpoint, not of the engine; the public demo
endpoint rate-limits aggressively. The cache is immutable and keyed by request,
so a second run of the demo needs no network at all.

## Run it

Prerequisites are the existing Rust/Solana toolchain and curl. The default
archive endpoint is Alchemy's public documentation endpoint and needs no API key,
wallet, private key or funded account.

```bash
make demo-cpi-mainnet-replay
# or, with an output parent of your choosing
./scripts/demo-stake-pool-upgrade.sh ./data/my-stake-pool-run
```

The individual commands:

```bash
# Find the upgrades in a slot range without scanning a single block.
eplyx versions upgrades --program SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy \
  --start-slot 425000000 --end-slot 430000000 --output ./data/stake-pool

# Acquire the transaction, its exact pre-state, every dependency binary, and
# the screening evidence.
eplyx historical acquire --program SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy \
  --signature 58d7oY3zMFRSjcbEZErYYjxEuLEWzNSXphvHBfGNZ78hVznzmLnY1hPNhG8LpV83EYcha8wQn6XifPS53bxR1pgs \
  --output ./data/stake-pool

# Compare, offline. Dependency binaries are found beside the corpus.
eplyx compare --corpus ./data/stake-pool/corpus.json \
  --current ./data/stake-pool/spl-stake-pool-mainnet-v1.so \
  --candidate ./artifacts/fixture_stake_pool_v2.so --fail-on-critical
```

The acquired record is committed at
[`docs/examples/mainnet-stake-pool-record.json`](examples/mainnet-stake-pool-record.json).
The binaries are not: they are 1 MB of mainnet bytecode each, and the demo
reproduces them by hash. The one dependency that *is* committed is the SPL Token
deployment the CPI execution tests load, at
[`fixtures/dependencies/`](../fixtures/dependencies/README.md).

## Limitations

What this phase supports is one transaction class against one protocol. Naming
it precisely matters more than the result.

- **Two instructions, one protocol.** `DepositSol` into a pool with no SOL
  deposit authority, reaching the System and SPL Token programs; and
  `WithdrawSol`, reaching SPL Token and the **deployed Stake program** - loaded
  as the BPF deployment live at the transaction's slot, not assumed to be the
  runtime builtin - and admitting a strictly-shaped top-level `Approve`
  companion. One level of invocation. Anything else is rejected.
- **One level of CPI, into known programs.** Invocation at a depth greater than
  two, or into a program outside each instruction's declared set - System and
  SPL Token for `DepositSol`, SPL Token and Stake for `WithdrawSol` - is
  rejected. This is not general CPI support.
- **Legacy messages, or v0 that resolves no lookup addresses.** The reproduced
  `WithdrawSol` is a v0 message with zero resolved lookup addresses, which
  normalized replay executes. A v0 message that *does* resolve addresses through
  a table is rejected. Address lookup tables are normalized but not
  executed, which excludes most aggregator traffic.
- **No account creation or closure.** An associated-token-account
  `CreateIdempotent` is accepted only when the validator recorded a token balance
  for its target on both sides, which proves it had nothing to do.
- **Successful originals only.** Replaying a transaction that failed on mainnet
  is a separate problem with a separate gate.
- **Same-slot cleanliness is a selection constraint, not a solved problem.** Most
  candidates are rejected. The ones that survive are the ones whose required
  accounts nothing else in the slot touched.
- **Feature activation is approximated.** See above; both builds share the set
  and V1's reproduction is what bounds the risk.
- **The reserve stake account's delegation is carried through, not simulated.**
  A SOL deposit does not invoke the stake program. A stake deposit would, and is
  out of scope.
- **No fiat valuation.** Pool tokens and lamports are reported in their own
  units. Nothing here converts them to money.
- **The regressed candidate is a locally constructed counterexample.** It is not
  a real stake-pool release and says nothing about any real deployment. The
  `reference` build of the same crate exists so the CPI execution path can be
  tested locally against the real SPL Token program; it is not a
  reimplementation of the stake pool.
- **Intent is not classified.** A legitimate upgrade may change behaviour on
  purpose. This phase quantifies; it does not judge.

## What Eplyx can and cannot claim

It can claim: Eplyx can exactly replay one supported stateful mainnet interaction
with CPI dependencies, validate historical V1 behaviour against the original
mainnet outcome, compare against a real or candidate upgrade, and quantify
protocol-level economic deltas through the adapter layer.

It cannot claim: universal CPI replay, arbitrary DeFi support, complete protocol
coverage, formal verification, or automatic support for all SPL protocols.
