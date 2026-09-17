# Eplyx

Upgrade Impact CI for Solana programs.

Eplyx deterministically executes the same transactions and account states
against the current and proposed versions of a Solana program to detect
state-dependent behavioral and economic regressions before deployment.

> **Working name.** *Eplyx* appears only in the CLI binary and the engine's
> package name. It is kept out of the program ID, the wire format, the fixture
> format, the report schema and every core type, so that renaming later stays a
> rename rather than a migration.

It answers one question:

> **What will actually change for users, positions and capital if this new
> program version is deployed?**

Ordinary testing asks whether the new build compiles, passes its unit tests and
keeps its interface. A program can satisfy all of that and still change what
specific existing accounts are worth. This tool executes identical transactions
against identical state under both program versions and reports the difference.

Eplyx reports in two layers, and keeps them distinct:

| Layer | Question | Unit |
| --- | --- | --- |
| **Behavioural regression** | Did anything change? | fixtures, differences, severity |
| **Economic impact aggregation** | How much does it matter? | positions, collateral, debt |
| **Regression clustering** | What triggers it, and what is the smallest example? | groups, conditions, counterexamples |

The first is a statement about code. The second is a statement about capital -
but only about the capital in this corpus. See
[Economic impact aggregation](#economic-impact-aggregation) for exactly what is
and is not being claimed.

**Current scope.** Deterministic V1/V2 execution, structured diffing, economic
interpretation, corpus-wide impact aggregation, regression clustering and
counterexample minimization against a synthetic corpus; exact controlled replay;
and deterministic discovery, classification, clustering and representative
selection for real public-program activity; and one exact slot-archive path for
a bounded real mainnet System-transfer/Memo interaction. Discovery and replay
corpora are separate, provenance and replay eligibility are explicit, and
current account samples remain approximate; and one exact CPI-aware path for a
real stateful protocol interaction, with every dependency binary pinned to the
deployment live at the transaction's slot. No arbitrary mainnet pre-state
reconstruction, universal CPI replay, arbitrary DeFi support, dashboard, GitHub
App, AI, third-party decoding or sequence search. A CI gate *is* implemented —
locally as `eplyx ci check` and as a small hosted API — over exactly the bounded
replay contract described here. See
[Phase 4](docs/phase-4-replay.md) for controlled replay,
[Phase 5](docs/phase-5-mainnet-discovery.md) for discovery,
[Phase 6](docs/phase-6-historical-state.md) for historical state,
[Phase 7](docs/phase-7-production-protocol-upgrade.md) for a real production
protocol upgrade replayed against the binaries mainnet actually ran, and
[Phase 8](docs/phase-8-cpi-mainnet-replay.md) for the same with cross-program
invocation, and
[Phase 9](docs/phase-9-production-corpus.md) for the validated
production-derived corpus a gate runs on, and
[Phase 10](docs/phase-10-hosted-ci.md) for the CI gate and its hosted API
([pilot onboarding](docs/pilot-onboarding.md),
[Railway deployment](docs/railway-deployment.md)).

---

## Quick start

```bash
# 1. Toolchain (once)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"

# 2. Build both program versions and compare them
make
```

`make` compiles V1 and V2 to SBF bytecode and runs the full corpus through both.
For this default synthetic comparison, nothing else is required: no RPC endpoint,
no API key, no network access after
the toolchain is installed, no database, no container.

### Public frontend

The public product site explains the replay and gate model, includes a real
Stake Pool demo report, and provides an `/analyse` form for an existing hosted
project. The form calls the Phase 10 API directly; it does not create projects
or invent an unauthenticated workflow.

```bash
pnpm dev              # http://localhost:4173
pnpm check:frontend   # structure and product-copy checks
pnpm build            # static output in dist/
```

The prepared routes are `/`, `/analyse`, and `/runs/:id`. A static host must
rewrite those application routes to `index.html`.

---

## Controlled Solana replay

```bash
make demo-replay
```

This starts a temporary local validator, executes real SBF transactions, ingests
21 interactions via RPC and selects three exact snapshots. It stops the validator,
then compares V1/V2 offline twice: three exact V1 replays, two newly-liquidatable
regressions, and one unaffected control. No wallet or public-network funds are
needed. [Workflow, format, actual results and limitations](docs/phase-4-replay.md).

[Phase 1–3 audit and all 40 requirements](docs/phase-1-3-audit.md).

## Mainnet discovery

```bash
make demo-discovery
# or
./scripts/demo-mainnet-discovery.sh <PROGRAM_ID>
```

This bounded, non-semantic workflow discovers real activity, distinguishes
direct/CPI interactions, fingerprints and clusters recurring shapes, selects an
explainable representative discovery corpus, then proves byte-identical output
from the same cache. It performs no candidate comparison without sufficient
historical state provenance. [Design, CLI, policy and limitations](docs/phase-5-mainnet-discovery.md).

## Historical mainnet replay

```bash
make demo-mainnet-replay
```

This acquires a fixed real mainnet System-transfer/Memo transaction from a
slot-addressable account archive, proves both account boundaries against
validator metadata, extracts the immutable historical Memo SBF as V1, and then
replays entirely offline. A deliberately regressed Memo-compatible candidate
rejects the historical 88-byte memo and prevents the exact 19,661-lamport
transfer. No fiat estimate is attached. [Provider boundary, proof and measured
result](docs/phase-6-historical-state.md).

## A real production protocol upgrade

```bash
make demo-token2022-upgrade
```

This replays one real historical PYUSD payment — a direct `TransferChecked` of
10.000000 PYUSD at slot 427146982 — against the two Token-2022 binaries actually
deployed to mainnet on either side of a real upgrade. The V1 side is the
deployment that was live at that slot, resolved from ProgramData, so "V1
reproduces the original outcome" is a genuine check rather than a tautology.

```text
V1  deployed at slot 395047597   post-state matches mainnet exactly
V2  deployed at slot 427147035   post-state identical

protocol economics   no field differs
compute              3,800 → 3,848 units (+1.26%), off the pass/fail axis
```

The real upgrade preserved behaviour, which is what a pre-deployment gate should
report for a compatible release. Because that alone cannot show a *changed*
outcome would be caught, the same interaction also runs against a deliberately
regressed candidate, which credits the destination one basis point short:

```text
destination amount   V1 66,675.509183 → V2 66,675.508183   delta -0.001000 PYUSD
CI gate              exit 1 (blocked)
```

[Version resolution, exactness boundary and measured
result](docs/phase-7-production-protocol-upgrade.md).

## A real upgrade, with cross-program invocation

```bash
make demo-cpi-mainnet-replay
```

This replays one real historical SPL Stake Pool `DepositSol` — 0.423 SOL at slot
429878778, 3,339 slots before a real stake-pool upgrade — against the two
stake-pool binaries mainnet actually deployed on either side of it. The deposit
moves lamports through the System program and mints pool tokens through the SPL
Token program, and how many tokens it mints is *computed* from pool state rather
than named in the instruction.

Every program the transaction reaches is pinned to the deployment that was live
at that slot, including the one it reaches only by invocation:

```text
SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy  historical  deployed at slot 370300186
TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA  historical  deployed at slot 419472000
11111111111111111111111111111111             builtin     implemented by the runtime
ComputeBudget111111111111111111111111111111  builtin     implemented by the runtime
```

The gate now checks the invocation graph as well as the fee and the post-state,
and the block is screened so that no other transaction in the slot wrote a
required account:

```text
same-slot screening   6 required accounts against 1,503 transactions; no conflicts
V1 replay             fee, invocation graph and post-state all match mainnet
pool tokens received  0.395906603 under both builds
protocol economics    no field differs
compute               -1.44%, off the pass/fail axis
```

Against a deliberately regressed candidate — one that caches the exchange rate at
four decimal places instead of dividing last — both builds still succeed and make
the same two calls, and the depositor receives less:

```text
pool tokens received  V1 0.395906603 → V2 0.395885700   delta -0.000020903
CI gate               exit 1 (blocked)
```

[CPI graph, dependency versioning, screening and
limitations](docs/phase-8-cpi-mainnet-replay.md).

## What it found

```text
BEHAVIOUR
  Fixtures tested:    141
  Outcome identical:  89   (state, balances, result and CPI shape unchanged)
  Outcome changed:    52
    critical:         11
    high:             41
    warning:          0

  Compute units (tracked separately - any recompilation moves these)
    141 of 141 fixtures differ, range -52.32% .. +133.17%
    above the +30.00% operational-risk threshold: 7

ECONOMIC COVERAGE  (synthetic corpus, valued from fixture state)
  Positions valued:        141
  Collateral represented:     $6,182,370.00
  Debt represented:           $2,531,638.09
  Net represented:            $3,650,731.91

AFFECTED  (positions with a non-compute difference)
  affected                        52   collateral      $444,570.00   debt      $318,298.09
  of which critical               11   collateral      $109,650.00   debt       $85,410.00
  unaffected                      89

BY ECONOMIC CONSEQUENCE
  newly_liquidatable               4   collateral       $39,800.00   debt       $31,780.00
  transaction_now_reverts          4   collateral       $40,000.00   debt       $29,780.00
  transaction_now_succeeds         3   collateral       $29,850.00   debt       $23,850.00
  value_changed                   41   collateral      $334,920.00   debt      $232,888.09

NEWLY LIQUIDATABLE  (healthy under V1, liquidatable under V2)
  Positions:               4
  Collateral:                    $39,800.00
  Debt:                          $31,780.00
```

and at the end:

```text
VERDICT: 11 critical economic regression(s) detected. Do not deploy V2.
         52 of 141 positions affected; 4 newly liquidatable ($39,800.00 collateral).
```

The flagship counterexample - both transactions succeed, and the position
silently becomes liquidatable:

```text
boundary-position-017  [boundary]
  action: refresh position
  CRITICAL  position.liquidatable  false -> true
              health factor 1.003783 -> 0.998738
  HIGH      position.health_factor  1.003783 -> 0.998738  (delta -5045)
              position crosses below the liquidation threshold
  position value: collateral $9,950.00 (99.500000000 SOL), debt $7,930.00, net $2,020.00
  economic consequence: position becomes newly liquidatable
```

The same arithmetic reaches users three different ways. A withdrawal that works
under V1 reverts under V2:

```text
withdraw-boundary-004  [withdraw-boundary]
  action: withdraw 0.5 SOL collateral
  CRITICAL  transaction outcome
              V1: success
              V2: FAILED - instruction 0: LtvExceeded (9)
  HIGH      owner.lamports  5000500000000 -> 5000000000000  (delta -500000000)
```

And a liquidation that V1 rejects as healthy succeeds under V2, moving real
collateral to a third party:

```text
liquidation-boundary-001  [liquidation-boundary]
  action: liquidate, repaying 1000 USD
  CRITICAL  transaction outcome
              V1: FAILED - instruction 0: PositionHealthy (10)
              V2: success
  HIGH      liquidator.lamports  1000000000000 -> 1010500000000  (delta 10500000000)
  HIGH      position.collateral_amount  99500000000 -> 89000000000  (delta -10500000000)
```

---

## How V1 and V2 differ

Everything below is compiled from **the same source tree**, selected by a cargo
feature. The program ID, instruction set, account layouts, error codes, account
ordering and signer sets are byte-for-byte identical. The entire behavioural
delta is one function in
[`programs/fixture-lending/src/math.rs`](programs/fixture-lending/src/math.rs):

```rust
// V1 - correct: widen, multiply, divide last.
(collateral_lamports as u128)
    .checked_mul(price as u128)
    .map(|scaled| scaled / LAMPORTS_PER_SOL)

// V2 - the seeded regression: normalise to whole SOL first.
let whole_sol = (collateral_lamports as u128) / LAMPORTS_PER_SOL;
whole_sol.checked_mul(price as u128)
```

Dividing before multiplying looks like a harmless refactor - it even removes an
overflow concern. It silently discards the **fractional SOL** of every position.

This is what makes it a good test of the thesis, and it is why the corpus is
shaped the way it is:

| Position holds | Effect under V2 |
| --- | --- |
| A whole number of SOL | **Nothing.** `floor(C/1e9)*P` equals `C*P/1e9` exactly. 89 of 141 fixtures. |
| Fractional SOL, comfortable headroom | Health factor shifts by a few thousandths. Visible, not dangerous. |
| Fractional SOL, near a threshold | **Outcome changes.** Liquidatable flips, or a withdrawal reverts. |

At $100/SOL with an 80% liquidation threshold, 99.5 SOL of collateral is worth
$7,960.00 of risk-adjusted value under V1 and $7,920.00 under V2. A position is
healthy under V1 and liquidatable under V2 exactly when its debt falls in
`($7,920.00, $7,960.00]` - a $40 window on an $8,000 position.

`boundary-position-017` carries $7,930.00 of debt and sits inside it:

```text
                     V1            V2
risk-adjusted     $7,960.00     $7,920.00
debt              $7,930.00     $7,930.00
health factor      1.003783      0.998738
liquidatable          false          true
```

None of this is visible from the IDL, the account layout, the authority set or a
bytecode diff. It only appears when the new code executes against that state.

---

## Architecture

```text
                      fixture corpus  (state + transaction, stable IDs)
                              │
              ┌───────────────┴───────────────┐
              ↓                               ↓
      ┌───────────────┐               ┌───────────────┐
      │  fresh VM #1  │               │  fresh VM #2  │
      │  V1 bytecode  │               │  V2 bytecode  │
      │  pinned clock │               │  pinned clock │
      └───────┬───────┘               └───────┬───────┘
              │   real SBF execution          │
              ↓                               ↓
        ExecutionResult                 ExecutionResult
        success / error                 success / error
        account post-state              account post-state
        lamport balances                lamport balances
        CPI sequence                    CPI sequence
        compute units, logs             compute units, logs
              └───────────────┬───────────────┘
                              ↓
                      ┌───────────────┐
                      │  diff engine  │   protocol-agnostic
                      └───────┬───────┘   structural comparison
                              ↓
                      ┌───────────────┐
                      │  interpreter  │   protocol-aware
                      └───────┬───────┘   bytes → health factor,
                              ↓            liquidation status, position value
                      ┌───────────────┐
                      │    impact     │   protocol-aware
                      └───────┬───────┘   per-position economics → corpus totals
                              ↓            (integer fixed-point USD)
                      ┌───────────────┐
                      │    cluster    │   protocol-aware
                      └───────┬───────┘   group by trigger, derive conditions
                              ↓
                      ┌───────────────┐   re-executes candidates through
                      │    shrink     │──▶ the executor above, checking the
                      └───────┬───────┘   regression signature is preserved
                              ↓
                      ┌───────────────┐
                      │    report     │   text / JSON
                      └───────────────┘
```

Execution loads bytecode without calling lending business logic. The adapter
boundary is currently a module convention, not a formal trait: `executor`
resolves lending error names, `diff` calls `interpret` for decoded fields and
liquidation transitions, and reporting renders those findings. `money` remains
protocol independent. Corpus construction, interpretation, impact, clustering
and shrinking contain the lending-specific rules. Generic protocol support
would require extracting this interpretation seam.

### Repository layout

```text
.
├── interface/               wire format shared by the program and the engine
│   └── src/lib.rs           instructions, account layouts, error codes,
│                            and the reference risk math used as a test oracle
├── programs/
│   └── fixture-lending/     the fixture protocol (own cargo workspace)
│       └── src/
│           ├── math.rs      ← the ONLY difference between V1 and V2
│           ├── processor.rs instruction handlers (identical in both builds)
│           ├── error.rs
│           └── lib.rs
├── engine/
│   ├── src/
│   │   ├── types.rs         fixture format: state + transaction + watch list
│   │   ├── corpus.rs        deterministic corpus generation  (protocol-aware)
│   │   ├── executor.rs      one fixture × one build → ExecutionResult
│   │   ├── diff.rs          ExecutionResult × ExecutionResult → StateDiff
│   │   ├── interpret.rs     bytes → named fields → position economics (protocol-aware)
│   │   ├── impact.rs        corpus-wide economic aggregation    (protocol-aware)
│   │   ├── cluster.rs       grouping, conditions, representatives (protocol-aware)
│   │   ├── shrink.rs        bounded counterexample minimization  (protocol-aware)
│   │   ├── money.rs         integer-only fixed-point USD
│   │   ├── report.rs        text and JSON rendering
│   │   └── main.rs          the `eplyx` CLI
│   └── tests/
│       └── upgrade_diff.rs  end-to-end differential suite
├── fixtures/
│   ├── program-id.txt       fixture protocol address
│   └── states/              the 141 generated fixtures, as inspectable JSON
│                            (state and transaction are one unit, so there is
│                             no separate scenarios/ directory)
├── scripts/
│   ├── build-programs.sh    compiles V1 and V2 to artifacts/
│   └── test-programs.sh     host-target unit tests, once per build flavour
└── ts/
    └── check-report.ts      validates the JSON report contract (Node, no deps)
```

---

## Economic impact aggregation

The diff engine answers *did behaviour change?* The impact layer answers *how
much does that change matter?*

### What a position is worth

Every value is normalised to micro-USD by one integer function, so an asset with
different decimals needs no new code path:

```text
value_micro_usd = amount * price_micro_usd / 10^decimals
```

The inputs are read straight out of account state rather than duplicated into
the fixture - `collateral_amount`, `debt_amount` and `collateral_price` are
already on the position. Only the asset metadata (decimals, and the debt asset's
USD peg) is added, as protocol-level constants in the shared interface crate.

```text
boundary-position-017
  collateral   99.5 SOL  @ $100.000000   ->  $9,950.00
  debt         $7,930.00 (USD-pegged)    ->  $7,930.00
  net                                        $2,020.00
```

### Integer arithmetic, end to end

`Usd` and `SignedUsd` are fixed-point integers with six decimal places. There is
no `From<f64>`, no `as f64`, and no path that leaves the integer domain: past
2^53 micro-USD a double can no longer represent every value, and a report that
silently rounded there would be wrong in a way nobody would notice.

JSON encodes them as decimal **strings** (`"6182370.000000"`), not numbers - a
JSON number would be parsed as a double by most consumers, reintroducing exactly
the loss the types exist to prevent. A test enforces that no `f64` appears
anywhere in the valuation or reporting path, and the compute-unit percentage is
carried as integer basis points for the same reason.

### What counts as affected

A position's capital counts as affected when the fixture shows **any
non-compute difference**.

Compute-only changes never count. Every recompilation moves compute units -
including on all 89 fixtures whose state is byte-identical - so letting compute
mark capital as affected would report the entire corpus as economically
impacted, which would be false.

```text
V1 100,000 CU -> V2 105,000 CU, state identical   affected: no
V1 withdraw succeeds -> V2 withdraw reverts       affected: yes
V1 liquidatable=false -> V2 liquidatable=true     affected: yes
```

Each affected position is tagged with the consequences actually observed,
derived from the diff rather than from parsing scenario text:

| Consequence | Meaning |
| --- | --- |
| `newly_liquidatable` | healthy under V1, liquidatable under V2 |
| `no_longer_liquidatable` | the reverse |
| `transaction_now_reverts` | succeeded under V1, reverts under V2 |
| `transaction_now_succeeds` | rejected under V1, succeeds under V2 |
| `value_changed` | balances or fields differ, no threshold crossed |

### What is *not* being claimed

There is no `capital_at_risk` field, and the reporting layer never uses the
phrase. The engine has not proven that any real capital is at risk, and the
vocabulary is deliberately factual:

- **"collateral represented"** - the value the corpus covers.
- **"affected collateral"** - collateral belonging to positions whose behaviour
  changed. Not a loss estimate; most of these positions merely shift a health
  factor by a few thousandths.
- **"newly liquidatable collateral"** - collateral in positions that are healthy
  under V1 and liquidatable under V2. This is the narrowest and strongest claim
  the engine makes, and it is still a claim about the synthetic corpus.

This phase does not touch mainnet. Nothing here says anything about deployed
capital, because the corpus is not derived from deployed state. Making that
possible is a later phase, and it is what would turn "affected collateral in the
corpus" into "affected collateral in production".

Aggregates are computed from the **baseline** valuation - the position as the
corpus holds it before the transaction runs, valued with the reference
arithmetic rather than either candidate build, so the seeded regression cannot
influence the figures that measure it.

## What the engine compares

| Signal | Source | Severity |
| --- | --- | --- |
| Transaction success / failure | VM result, with custom error codes resolved to names | critical |
| Liquidation status | derived from the decoded position | critical |
| Named account fields | borsh-decoded against the shared layout | high / warning |
| Lamport balances | post-execution account state | high |
| CPI sequence | structured inner instructions, not log scraping | high |
| Raw account bytes | fallback when an account does not decode | warning |
| Compute units | VM metering | separate axis (see below) |
| Position value | decoded position + asset metadata | economic layer, not severity |

**Compute is deliberately kept off the pass/fail axis.** Every recompilation
moves compute units - all 141 fixtures differ here, including the 89 whose state
is byte-identical. Folding that in would classify the entire corpus as "changed"
and bury the question the tool exists to answer. Compute is reported on its own,
and still escalates to `high` past 30%, where it becomes an operational risk in
its own right.

One caveat the report does not currently annotate: where a fixture's *outcome*
changed, its compute delta is a *consequence* of that change (a reverting
transaction does less work), not an independent finding.

### Determinism

Both sides are constructed identically and the program bytecode is the only free
variable:

- a **fresh VM per execution**, so "reset to identical initial state" is
  structural rather than a procedure that could drift;
- the **clock sysvar is pinned** to a fixed slot, epoch and timestamp;
- **keypairs are derived from seeds** stored in the fixture, so addresses and
  signatures reproduce on any machine;
- the **fee payer is a separate account** from the position owner, so fee
  deduction never contaminates the economic balance comparison;
- **corpus generation is a pure function** of the fixture index - no RNG, no
  clock, no filesystem;
- the **SBF builds are byte-reproducible**: a clean rebuild from unchanged
  source produces identical bytecode, so a hash difference between two artefacts
  means a source difference and nothing else.

`repeated_execution_of_a_fixture_is_bit_identical` asserts this, and
`checked_in_fixtures_match_the_generator` stops the committed corpus drifting
from the generator.

Execution runs **real SBF bytecode through the real Solana VM**. Nothing in the
comparison path calls the fixture protocol's Rust functions directly - if it
did, the tool would be testing a host build rather than the deployable artefact.

---

## Regression clustering and minimization

Eleven critical findings are not eleven bugs. The clustering layer groups
findings that share a trigger, states the conditions they have in common, and
shrinks one of them into a smaller witness that still reproduces.

### Grouping

Findings are grouped by a signature built from what was actually observed: the
economic consequences, the instruction, the kinds of difference present, the
specific account fields that changed, and the success/failure transition.

Severity is deliberately **not** part of the signature. Two unrelated
regressions that happen to be critical are not the same bug.

The 52 changed fixtures fall into five groups:

```text
newly-liquidatable                    4 fixtures  [CRITICAL]  refresh_position
transaction-now-reverts               4 fixtures  [CRITICAL]  withdraw_collateral
transaction-now-succeeds              3 fixtures  [CRITICAL]  liquidate
value-changed--refresh-position      39 fixtures              refresh_position
value-changed--withdraw-collateral    2 fixtures              withdraw_collateral
```

IDs are short where the consequence is unambiguous and qualified with the action
where that would otherwise collide; distinct field/transition classes sharing an
action receive deterministic numeric suffixes. IDs are stable for the same corpus.

### Common conditions

For each group, categorical values shared by *every* member are reported exactly,
and numeric parameters are reduced to ranges:

```text
1. newly-liquidatable                     [CRITICAL]
   action:         refresh_position
   fixtures:       4  (boundary-position-017, boundary-position-018, ...)
   capital:        collateral $39,800.00   debt $31,780.00
   common conditions:
     collateral_asset = SOL
     fractional_collateral = true
     v1_liquidatable = false
     v2_liquidatable = true
     collateral = 99.500000000 SOL
     debt in $7,930.00 .. $7,960.00
     health_factor_v1 in 1.000000 .. 1.003783
     health_factor_v2 in 0.994974 .. 0.998738
   representative: boundary-position-017
```

A range is promoted to a *condition* only when its members sit within 20% of
each other. The 39-fixture group spans $2,500 to $7,960 of debt, so its ranges
are recorded in JSON but not presented as a trigger: "debt between $2,500 and
$7,960" would describe the corpus rather than the bug, which is worse than
saying nothing.

### Representative selection

Derived from fixture data, never hardcoded, in this order: fewest awkward
properties (whale-scale, dust-scale, zero-debt, or already failing under V1),
then smallest economic magnitude, then closest to the decision boundary, then
fixture ID as a total-order tie-breaker. For the current corpus this picks
`boundary-position-017`, but only because it carries the smallest debt in its
group.

### Counterexample minimization

For each critical group, a bounded delta-debugging search shrinks the
representative fixture while re-executing both builds and checking that the
regression *signature* still matches.

Collateral and debt are coupled near a threshold - neither can move alone
without leaving the regression class - so the move set scales both together
(1/2, then 3/4, then 7/8) before refining each field with a halving descent.
Each candidate is a self-consistent state: the cached health factor is
recomputed with the reference arithmetic and the vault balance is adjusted to
match the collateral it custodies.

```text
   minimized counterexample (23 reductions in 32 probes):
     collateral: 0.001000000 SOL ($0.10)
     debt:       $0.01
     V1: success, health 8.000000, liquidatable false
     V2: success, health 0.000000, liquidatable true
```

The 99.5 SOL position shrinks to 0.001 SOL: under V2 *any* sub-1-SOL collateral
is valued at zero, so the position is instantly liquidatable while V1 reports it
eight times over-collateralised.

```bash
eplyx reproduce newly-liquidatable                  # or regression-group:newly-liquidatable
```

### What minimization does not prove

- **It is not formal verification.** It is a search over a corpus, not a proof
  about the program. It finds a witness; it says nothing about states it never
  tried.
- **It is not minimal.** The search is greedy and bounded (96 probes per
  cluster), stops at a configured granularity (0.001 SOL, $0.01), and varies
  only collateral and debt. A case it cannot shrink further is a local stopping
  point, not a proven boundary.
- **It does not shrink instruction parameters.** The `transaction-now-succeeds`
  group stops at 13 SOL because its liquidation repays a fixed $1,000; below
  that the repayment exceeds the debt and the transaction fails for an unrelated
  reason. The shrinker correctly refuses that candidate rather than reporting a
  different bug as a minimized version of this one.
- **The conditions describe the corpus, not the program.** "debt in $7,930 ..
  $7,960" is the span of the fixtures that happened to be generated, not the
  true mathematical boundary. The boundary is documented separately in
  `corpus.rs`, where it was derived by hand.

Minimization re-executes candidates, so it adds roughly 20 seconds to a full
run. `--no-minimize` skips it.

## Commands

```bash
make                  # build both versions and compare       (the headline command)
make test             # program unit tests + differential suite
make report           # same comparison as JSON, to report.json
make fixtures         # regenerate the checked-in corpus
make fmt lint         # rustfmt and clippy across both workspaces

make demo-token2022-upgrade    # a real Token-2022 upgrade over a real PYUSD transfer
make demo-cpi-mainnet-replay   # a real stake-pool upgrade over a real CPI deposit
```

Mainnet paths, which need an archive endpoint:

```bash
eplyx versions upgrades --program <ID> --start-slot <A> --end-slot <B> --output <DIR>
eplyx versions resolve  --program <ID> --slot <S> --output <DIR> [--out v1.so]
eplyx historical acquire --signature <SIG> --program <ID> --output <DIR> [--offline]
eplyx compare --corpus <DIR>/corpus.json --current v1.so --candidate v2.so \
  [--dependencies <DIR>/dependencies] [--fail-on-critical]
```

Directly:

```bash
cargo run -p eplyx-engine -- compare
cargo run -p eplyx-engine -- compare --format json
cargo run -p eplyx-engine -- compare --category boundary
cargo run -p eplyx-engine -- compare --fail-on-critical      # exit 1 for a CI gate
cargo run -p eplyx-engine -- compare --no-minimize              # skip the shrink search
cargo run -p eplyx-engine -- reproduce boundary-position-017    # one fixture, side by side
cargo run -p eplyx-engine -- reproduce newly-liquidatable       # a whole regression group
cargo run -p eplyx-engine -- list --category withdraw-boundary
```

`reproduce` prints both executions in full - decoded position economics, logs
and every difference - which is the beginning of the "executable evidence"
property the product is aiming at.

### JSON shape

```json
{
  "economics": {
    "positions_valued": 141,
    "total_collateral_value_usd": "6182370.000000",
    "total_debt_value_usd": "2531638.086646",
    "total_net_value_usd": "3650731.913354",
    "affected":           { "positions": 52, "collateral_value_usd": "444570.000000", "debt_value_usd": "318298.086646" },
    "critical":           { "positions": 11, "collateral_value_usd": "109650.000000", "debt_value_usd": "85410.000000" },
    "newly_liquidatable": { "positions": 4,  "collateral_value_usd": "39800.000000",  "debt_value_usd": "31780.000000" },
    "by_consequence": { "newly_liquidatable": { "positions": 4, "...": "..." } }
  },
  "fixture_economics": [
    {
      "fixture_id": "boundary-position-017",
      "baseline": {
        "collateral_amount": 99500000000,
        "collateral_decimals": 9,
        "collateral_price_usd": "100.000000",
        "collateral_value_usd": "9950.000000",
        "debt_value_usd": "7930.000000",
        "net_value_usd": "2020.000000",
        "liquidatable": false
      },
      "affected": true,
      "critical": true,
      "consequences": ["newly_liquidatable"]
    }
  ]
}
```

The report round-trips through `serde_json` exactly, which is asserted by a test:
the schema is a contract, not merely something that happens to serialise.

Optional JSON contract check (requires Node ≥ 22.6; no dependencies to install):

```bash
pnpm install && pnpm verify:report
```

---

## Testing

```bash
make test
```

- **40 unit tests**: 5 in the shared interface crate, 35 in the engine (corpus,
  diff, interpreter, impact aggregation, clustering, shrinking, fixed-point
  money, hex codec).
- **7 program tests per build flavour**, run twice (V1 and V2) - these assert
  the seeded regression exists and is confined to fractional collateral.
- **39 end-to-end differential tests** that execute real bytecode.

The end-to-end suite covers the seven properties this phase had to demonstrate:

1. `v1_matches_the_reference_implementation` - V1's on-chain math agrees with an
   independent host-side implementation for every state the corpus reaches,
   which is what makes that reference usable as an oracle. Its mirror,
   `v2_disagrees_with_the_reference_somewhere`, fails if the seeded regression
   ever goes missing and quietly makes the rest of the suite vacuous.
2. `v2_accepts_the_same_instruction_encoding_and_layout` plus
   `the_two_artifacts_differ` - identical interface, genuinely different builds.
3. `the_majority_of_the_corpus_is_unaffected` and
   `whole_sol_categories_are_completely_unaffected`.
4. `boundary_window_flips_exactly_the_expected_fixtures` - asserts the exact
   set, so a fixture flipping that should not is a failure too.
5. `the_flagship_counterexample_is_reported_precisely` - exact health factors.
6. `repeated_execution_of_a_fixture_is_bit_identical`.
7. `the_text_report_names_the_fixture_and_the_changed_fields` and
   `the_json_report_is_valid_and_carries_the_findings`.

The economic layer adds its own:

8. `no_floating_point_in_the_valuation_or_reporting_path` scans the non-test,
   non-comment source of every module that touches a value and fails on `f64`.
9. `aggregates_equal_the_sum_of_their_member_positions` recomputes every total
   from the per-position records, so a summary cannot drift from its members.
10. `compute_only_and_unchanged_positions_contribute_no_affected_capital`
    pins the definition of "affected" against all 89 unaffected fixtures -
    every one of which *does* differ on compute.
11. `newly_liquidatable_capital_is_counted_from_the_expected_positions` asserts
    the exact fixture set and the exact dollar figures.
12. `the_json_report_round_trips` deserialises the report back into itself.

Clustering and minimization add:

13. `clusters_partition_the_changed_fixtures_exactly` - no overlap, no omission.
14. `unrelated_regression_classes_stay_separate` - two critical groups with the
    same severity must not merge.
15. `minimized_cases_preserve_the_exact_regression_class` - the core safety
    property: a minimized case is re-executed from scratch and its signature
    must equal the original's, so the shrinker cannot wander into a different
    bug and present it as a minimized version of this one.
16. `reproducing_a_minimized_case_is_deterministic` - re-running the search from
    the same fixture lands on exactly the same case.

`ts/check-report.ts` independently recomputes the aggregates and cluster capital
in TypeScript using `BigInt`, and re-checks that clusters partition the changed
set - so the totals are verified by an implementation that shares no code with
the engine.

The expected numbers are derived from the arithmetic documented in `corpus.rs`,
not recorded from a previous run. A change that shifted them fails the suite
rather than silently rewriting the baseline.

---

## Design decisions

**Anchor is not used.** The brief allowed it "where appropriate"; it was not.
Anchor would add a CLI version dependency and a large dependency tree to a
program whose entire job is to be small and to have an unambiguous, hand-checkable
byte layout - and the account layout is exactly what the diff engine decodes. The
value Anchor would bring (IDL-driven decoding) belongs to the later phase that
supports third-party protocols, where the IDL is the only schema available.

**litesvm, not `solana-program-test`.** It executes real SBF bytecode with real
compute metering and structured inner instructions, and constructs fast enough
to give every execution its own VM.

**The program is a separate cargo workspace.** Its `solana-program` tree does not
co-resolve with litesvm's pinned `solana-*` crates. The two are compiled for
different targets by different compilers, so coupling their lockfiles was an
artificial constraint. `make fmt` and `make lint` cover both.

**The wire format is its own crate.** `interface/` has no Solana dependency and
no feature flags, and is linked by both the program and the engine. That is what
makes "V1 and V2 have an identical interface" a structural property rather than
an assertion - neither build owns the definition, so neither can drift.

**The reference math is not shared with the program.** If the program imported
it, V2 could not diverge and the differential test would be vacuous.

**Integration tests live in `engine/tests/`, not a top-level `tests/`.** Cargo
binds integration tests to a package; a top-level directory would belong to no
crate.

---

## Known limitations

These boundaries apply to the default synthetic corpus and the fixture protocol.
The mainnet paths carry their own, narrower boundaries: see
[Phase 7](docs/phase-7-production-protocol-upgrade.md#limitations) and
[Phase 8](docs/phase-8-cpi-mainnet-replay.md#limitations).

**Scope**

- The default corpus is **synthetic**. Other paths ingest real transactions from
  an isolated local validator, and from mainnet through a slot-addressable
  archive. Arbitrary historical account pre-state reconstruction is not
  implemented: each mainnet path states exactly which transaction shapes it can
  reproduce exactly, and rejects everything else.
- The **protocol adapter seam is a trait**, with Token-2022 and SPL Stake Pool
  implementations. `corpus.rs`, `interpret.rs`, `impact.rs`, `cluster.rs` and
  `shrink.rs` remain fixture-lending-specific. They are not reached by adapter
  records because the generic diff takes its decoder as an argument: `replay`
  passes `FieldDecoder::None` for any record an adapter owns. Inferring a layout
  from a leading byte, as it once did, made a real stake-pool account decode as
  a synthetic `Market` and compare as identical over its first 86 of 611 bytes.

**Execution model**

- One fixture is exactly **one instruction in one transaction**. Multi-instruction
  transactions and cross-transaction sequences are not modelled.
- Both versions share a **program ID** and run in separate VMs. This models an
  in-place upgrade but not a migration where the ID changes.
- The runtime **feature set is whatever litesvm defaults to**, and both sides get
  the same one. Differential testing across feature-gate activations is not
  supported. For a historical replay this is an approximation of the set that was
  active at the slot, bounded by V1 reproducing the original outcome.
- **Logs are captured but not diffed.** They are derivative of state and error
  outcome here, and diffing them would duplicate findings. CPI shape is compared
  structurally instead.

**Fixture protocol** (it is a test harness, not a protocol)

- Debt is an **accounted u64**, not an SPL token. Collateral is native SOL, so
  balance comparison is real, but there is no SPL-token or Token-2022 leg.
- No interest accrual, no oracle staleness, no partial-liquidation close factor,
  no multi-asset markets, no account migration paths.
- `LendingError::MathOverflow` is **unreachable** for u64 inputs once widened to
  u128; the `checked_mul` calls are defensive. A test asserts this so the claim
  stays honest if a type is ever narrowed.

**Economic aggregation**

- Figures describe the selected synthetic positions or controlled replay
  observations. Repeated interactions are not deduplicated into unique TVL.
  Nothing here measures deployed capital or expected loss.
- Valuation uses a single static price per market. There is no oracle
  uncertainty, no price path, no slippage, and no liquidation-penalty modelling,
  so "affected collateral" is a measure of exposure, not of expected loss.
- The debt asset is USD-pegged at exactly $1.00, so debt valuation is currently
  an identity. It is routed through the same normalisation as collateral so that
  a non-pegged asset would need no new code path, but that path is untested
  against a real second price.
- `newly_liquidatable` is measured from the *post-execution* liquidation flag.
  A position whose liquidation *eligibility* changed without its stored flag
  flipping is reported under `transaction_now_succeeds` instead - a distinction
  worth understanding before quoting either number.

**Clustering and minimization**

- Grouping is exact-signature based, so a fixture that differs in one extra
  field forms its own group. This errs toward over-fragmentation rather than
  merging unrelated findings, which is the safer failure for a security tool but
  means group counts can be larger than a human would draw them.
- Common conditions are computed over group members only. A condition shared by
  the whole corpus (`collateral_asset = SOL`) is reported even though it
  discriminates nothing.
- Minimization varies only collateral and debt, and only for critical clusters.
  See "What minimization does not prove" above.

**Reporting**

- Compute deltas on fixtures whose outcome changed are consequences of that
  change, and are not annotated as such.
- Severity is a fixed mapping, not configurable per protocol.
- The JSON report embeds full logs for every fixture, so it is large (~MBs).

## Licence

MIT.
