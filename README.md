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

**Phase 1 scope.** Deterministic V1/V2 execution, structured diffing, economic
interpretation and reporting, against a synthetic corpus and a purpose-built
fixture protocol. No mainnet ingestion, no dashboard, no CI integration, no AI,
no third-party protocol support. Those are later phases.

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
Nothing else is required: no RPC endpoint, no API key, no network access after
the toolchain is installed, no database, no container.

---

## What it found

```text
Fixtures tested:    141
Outcome identical:  89   (state, balances, result and CPI shape unchanged)
Outcome changed:    52
  critical:         11
  high:             41
  warning:          0

Compute units (tracked separately - any recompilation moves these)
  141 of 141 fixtures differ, range -52.33% .. +133.17%
  above the 30% operational-risk threshold: 7

By category
  category                tested  identical  changed  critical
  boundary                    20          0       20         4
  fractional                  15          0       15         0
  healthy                     40         40        0         0
  large                       12         12        0         0
  liquidation-boundary         3          0        3         3
  moderate                    25         25        0         0
  near-liquidation             8          0        8         0
  small                       12         12        0         0
  withdraw-boundary            6          0        6         4
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
                              ↓            liquidation status
                      ┌───────────────┐
                      │    report     │   text / JSON
                      └───────────────┘
```

The layering is the point. `executor`, `diff` and `report` know nothing about
lending; they deal in accounts, bytes, balances and compute. Everything that
understands what a *health factor* is lives in `corpus` and `interpret`. That
boundary is where a protocol adapter would plug in later.

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
│   │   ├── interpret.rs     bytes → named fields → economic meaning (protocol-aware)
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

## Commands

```bash
make                  # build both versions and compare       (the headline command)
make test             # program unit tests + differential suite
make report           # same comparison as JSON, to report.json
make fixtures         # regenerate the checked-in corpus
make fmt lint         # rustfmt and clippy across both workspaces
```

Directly:

```bash
cargo run -p eplyx-engine -- compare
cargo run -p eplyx-engine -- compare --format json
cargo run -p eplyx-engine -- compare --category boundary
cargo run -p eplyx-engine -- compare --fail-on-critical      # exit 1 for a CI gate
cargo run -p eplyx-engine -- reproduce boundary-position-017 # full side-by-side
cargo run -p eplyx-engine -- list --category withdraw-boundary
```

`reproduce` prints both executions in full - decoded position economics, logs
and every difference - which is the beginning of the "executable evidence"
property the product is aiming at.

Optional JSON contract check (requires Node ≥ 22.6; no dependencies to install):

```bash
pnpm install && pnpm verify:report
```

---

## Testing

```bash
make test
```

- **15 unit tests**: 3 in the shared interface crate, 12 in the engine (corpus,
  diff, interpreter, hex codec).
- **7 program tests per build flavour**, run twice (V1 and V2) - these assert
  the seeded regression exists and is confined to fractional collateral.
- **18 end-to-end differential tests** that execute real bytecode.

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

These are real and deliberate for Phase 1.

**Scope**

- The corpus is **synthetic**. No mainnet state, no historical transaction
  replay, no archival RPC. Production-state ingestion is a later phase.
- Only the **fixture protocol** is supported. `corpus.rs` and `interpret.rs`
  hardcode its layouts. The adapter seam exists as a module boundary but is not
  yet a trait.
- No CI integration, dashboard, impact aggregation over capital, counterexample
  minimisation, or AI-assisted explanation.

**Execution model**

- One fixture is exactly **one instruction in one transaction**. Multi-instruction
  transactions and cross-transaction sequences are not modelled.
- Both versions share a **program ID** and run in separate VMs. This models an
  in-place upgrade but not a migration where the ID changes.
- The runtime **feature set is whatever litesvm defaults to**, and both sides get
  the same one. Differential testing across feature-gate activations is not
  supported.
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

**Reporting**

- Compute deltas on fixtures whose outcome changed are consequences of that
  change, and are not annotated as such.
- Severity is a fixed mapping, not configurable per protocol.
- The JSON report embeds full logs for every fixture, so it is large (~MBs).

## Licence

MIT.
