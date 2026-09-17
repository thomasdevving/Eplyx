# Phase 9 — Validated production-derived corpus

Implemented: automated generation of a regression corpus from real mainnet
activity, and a deterministic selector that turns validated replay records into
a small corpus a CI gate can afford to run on every upgrade.

The phase produces a **validated historical corpus**: every member is a real
mainnet transaction, replayed against the program version that was live at its
slot, whose V1 post-state matches what mainnet actually produced. It is not a
representative sample of production traffic, and the reporting never claims it
is. What it is and what it is not is the subject of most of this document.

## Three populations, never collapsed

```text
ObservedProduction    interactions seen on mainnet
   ↓  (most are lost here, and the report says so)
ReplayEligible        interactions replayable under the exact historical contract
   ↓  (a deliberate, explainable choice)
SelectedCorpus        interactions the gate actually runs
```

These are three different numbers and the selector prints all three side by
side. Collapsing them is the failure mode this phase exists to avoid: a corpus
with no replayable withdrawals is not evidence that withdrawals do not happen.

Measured on SPL Stake Pool, 700 blocks scanned:

| Stage | Count | |
|---|---:|---|
| Transactions scanned | 700 | |
| Relevant to the program | 617 | |
| Supported instruction family | 94 | `DepositSol`, `WithdrawSol` |
| Structurally eligible | 76 | legacy or LUT-free v0 message |
| Boundary-clean | 34 | no same-slot interference on a required account |
| Acquired | 25 | historical state resolved at slot S-1 |
| **Replay-eligible** | **23** | V1 post-state matches mainnet (`fidelity Matched`) |

The two records lost at the last step share one systemic cause: a Jito tip
account credited while remaining below its rent-exempt minimum. Mainnet accepts
this; the replay runtime refuses it. They are preserved as rejected rather than
admitted by relaxing the gate.

Observed-to-replayable yield is reported per action as a first-class coverage
limitation, not a footnote:

```text
  - deposit_replayability_materially_below_observed
    7 of 681 observed deposit interactions are replayable under the exact
    historical contract (1.02%). The corpus under-represents this action
    relative to production.
  - withdraw_replayability_materially_below_observed
    16 of 200 observed withdraw interactions are replayable under the exact
    historical contract (8.00%).
```

The deposit figure is a property of transaction topology, not of the protocol:
83.3% of observed deposits are Jito-tipped and therefore carry the rent-paying
credit above. An unmeasured population is reported as absent, never as zero.

## Acquired, then validated

"Validated" is earned at a specific step, and it is not acquisition.

```text
historical acquire   publishes what it could read      acquired
corpus select        describes what it was handed      acquired
bundle build         replays V1 for every record       validated
```

Only the last holds the baseline and every dependency, so only the last can
check. It refuses a bundle containing a record whose V1 does not reproduce the
original post-state, naming the record and the mismatch. Until then the artifact
is an acquired corpus, and calling it validated would be a claim nothing had
tested.

## Selection

`corpus select` is deterministic and uses no model, no sampling and no
randomness. Candidates are sorted by observation ID before anything else, so
input order and concurrency cannot reach the result, and every tie breaks on ID.

Rounds, in order:

1. **Semantic action coverage** — one record per action that has any.
2. **Economic entities**, ranked by *compounding* novelty: how much other
   novelty the record also brings. Without this the entity round consumes the
   whole budget and no boundary or tail record is ever reached.
3. **Pools.**
4. **Boundary proximity** — within 2000 bps of a proven boundary.
5. **Amount tails**, per action.
6. **Structural novelty** — an interaction shape nothing selected covers.
7. **Temporal spread.**
8. **Common-path fill.**

Score components are integers on one scale; `tail_bps` saturates at 10,000 so a
whale deposit cannot make the other dimensions decorative.

A record is never duplicated to reach a target. Asking for 25 from a population
of 23 returns 23, a `Shortfall{requested, available, selected}`, and a stated
limitation. There is no forced action balance: at target 10 the selector takes
6 deposits and 4 withdrawals from a population of 7 and 16, because deposits are
the rarer action and rarity is what the score rewards.

The corpus is hashed with `selected_corpus_sha256` empty, so the hash cannot
depend on itself. Three consecutive runs produce byte-identical JSON.

## Interaction shape includes the invocation signature

The account label set alone is not the shape of an interaction. The production
corpus contains one referral-fee deposit that mints twice where an ordinary
deposit mints once, while naming the same accounts. Selecting on labels alone
covered every label set and still missed it.

It is also the only record where the regressed candidate diverges structurally
(`cpi_changed`) rather than numerically. The invocation signature is archive
evidence the record already carries, so the shape key now includes it; a record
with no original outcome is keyed `unknown` rather than sharing a key with one
observed to invoke nothing.

## What the corpus catches

Against the deliberately regressed candidate — a `DepositSol` share calculation
that caches the exchange rate at four decimal places and so mints slightly fewer
pool tokens:

| | Full corpus (23) | Selected (10) |
|---|---:|---:|
| Outcome identical | 0 | 0 |
| critical | 16 | 4 |
| high | 1 | 1 |
| warning | 6 | 5 |

The ten-record corpus reproduces every distinct failure *mode* the full corpus
finds, at 43% of the execution cost. The counts differ because the corpus is
smaller; no class is lost.

Read the classes carefully, because they do not all mean the same thing:

- **warning** — the injected defect. The depositor receives fewer pool tokens;
  the transaction still succeeds and the invocation graph is unchanged. This is
  what the phase set out to detect.
- **high** — the referral deposit above, where the candidate's invocation graph
  also differs.
- **critical** — `WithdrawSol` now fails. The regressed candidate implements
  `DepositSol` only, so this reflects the fixture's instruction surface, not a
  discovered regression. It is reported here rather than filtered out, because
  a corpus that silently drops the records its candidate cannot execute is
  measuring the candidate rather than the upgrade.

The control runs first and must be clean: V1 against itself over the same
corpus reports 10 of 10 outcomes identical and no compute differences.

## Wording

The engine and its reports say **production-derived regression corpus** or
**validated historical corpus**. They do not say *representative production
corpus* unqualified. The distinction is the whole point: these records are
real, and they are not a sample.
