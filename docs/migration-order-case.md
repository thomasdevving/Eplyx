# Bounded migration order case v1

This local analysis selects two source accounts under one unchanged TokenMigration
ChangeSpec. It executes A alone, B alone, A → B and B → A at the world's fixed
Clock, using the existing instruction builder, Session, transaction execution and
per-unit economic reconciliation. It does not introduce a ChangePlan, a new
proposal identity, search, governance status or hosted workflow.

## Use

Starting with an existing local migration run:

```sh
eplyx migration order --run RUN_ID \
  --source-a SOURCE_ACCOUNT_A --source-b SOURCE_ACCOUNT_B \
  --out ./order-case --format json

eplyx migration reproduce-order ./order-case --format json
```

`order` validates the saved run's package and bindings, reconstructs its retained
world offline and writes a new immutable directory. It does not read the current
project's candidate or fetch fresh state. `reproduce-order` needs only that
directory, without the original project, run, fixture recipe or program capture.
Both CLI operations run in the existing empty-environment worker. Neither engine
entry point accepts an RPC/provider. Output directories must be new; use canonical
paths when an OS temporary directory is reached through a symlink.

Only shared reserve-transfer funding is supported. Both sources must resolve to
distinct, individually eligible units with an available authority path and enough
initial reserve for each solo control. The existing authority model establishes
which signer privileges are assumed, not private-key possession or authorization.
Every external signer used by the local transaction must also have retained
wallet-compatible account evidence through the existing authority classifier.
Mint-to funding is explicitly unsupported. A future activation is refused; the
normal population rehearsal's optional activation-time advance is not used.
Activation remains inclusive and deadline exclusive, using existing window rules.

The minimal migration example normally declares activation after its fixture
Clock. Such a saved run is correctly refused by order analysis. Prepare a world
and proposal whose fixed Clock is actually inside the window; order analysis never
rewrites either to make it executable.

## Models and identity

`migration::order` supplies `PairBinding`, `OrderCase`, `StateEvidence`, `Step`,
`Scenario`, `Comparison`, `Analysis`, and typed `OrderError`/`FailureKind` values.
`migration::order_store` saves and reproduces these using the existing
`universal::evidence::EvidenceStore`.

All new digests use `canonical::digest`: SHA-256 over deterministic pretty JSON
with one trailing newline. Maps/sets are sorted; version/domain strings are part
of each identifying body.

- **Pair binding ID:** `eplyx-migration-order-case/v1`, unchanged ChangeSpec ID,
  candidate SHA-256 and length, existing world ID plus a full world-content digest
  (including parent-independent bytes for derived worlds), world provenance kind,
  planner version, adapter/version, runtime ID, all five Clock fields, ordered
  program identities (program address, loader, ELF SHA-256/length), reserve address
  and initial raw amount, both focused `MigrationUnit` records, bounded closure,
  and `same_bank_separate_transactions_no_time_advance`.
- **Runtime ID:** `eplyx-migration-order-runtime/v1`, executor version, Cargo.lock
  SHA-256, workspace/engine manifest digests, execution-module source SHA-256, and the pinned LiteSVM initialization
  contract. This binds the locked runtime/features/defaults, fee behavior,
  signature/blockhash checks disabled, transaction-history capacity zero and no
  warp. A different runtime contract is refused during restoration/reproduction.
- **State ID:** `eplyx-migration-order-state/v1`, pair binding ID and the sorted
  account census, including each existence tag and every AccountSnapshot field.
- **Order-case ID:** version, pair binding ID, exact initial state ID and ordered
  unit IDs. A → B and B → A have different IDs under the same pair binding.
  The solos have their own one-unit case IDs.
- **Scenario/run ID:** `eplyx-migration-order-run/v1`, order-case ID, complete steps
  and stop status. It identifies deterministic execution content, not wall time.

Migration units are always reconstructed by the existing planner from the supplied
ChangeSpec and world, never accepted as mutable imported units from another
proposal. Their existing unit IDs are retained, with source, amount, quote,
expected economics, authority and signer records bound alongside them. Proposal
metadata labels/source prose and world limitation prose are excluded from these
analytical identities. No output path or host timestamp enters them. The existing
world identity retains its own provenance rules.

## Account closure and handoff

Before execution, the closure is the union of both units' actual adapter
instruction accounts/programs, the relayer, source owners and required signers,
loaded program/ProgramData addresses, and the fixed runtime sysvars (Clock, Rent,
EpochRewards, EpochSchedule, LastRestartSlot, SlotHashes, SlotHistory,
StakeHistory, Fees and RecentBlockhashes). This covers the source/destination
accounts, mints, reserve, proposed configuration, authority and escrow when the
existing contract uses one, ATA creation, fee payer and token/ATA dependencies.
The migration source-basis-point fee is reconciled by the existing contract; this
feature does not invent a fee-destination account.

Only this closure from the world is seeded, plus the existing deterministic
proposed overlay/relayer. Named runtime builtins/sysvars and exact loader outputs
have explicit local-runtime provenance rather than observed-chain provenance.
Every other missing required account must have an absence proof. Synthetic
fixtures are closed worlds under the existing World contract; observed worlds
require `inspected_absent`. An implicit VM account outside these named defaults
cannot substitute for missing evidence.

Snapshots represent `Present { account }` or `KnownAbsent`. A missing map entry
means unknown and fails handoff. Present rows retain exact data bytes, lamports,
owner, executable and rent epoch. Every snapshot binds the same candidate,
dependencies, runtime, Clock and signer assumptions through its pair binding.
The initial state includes the declared proposed overlay and local relayer; it is
not described as wholly observed chain state.

Each scenario restores the exact same initial snapshot into a **new Session**,
rechecking every present and absent row after loading programs/sysvars. Only the
two transactions within an order share a Session. The second transaction reads
the actual first transaction's state. Source and signer evidence is checked again
at handoff; destination creation uses the existing idempotent ATA instruction.
The current pre-state is used by existing reconciliation. `restore` also supports
loading a saved intermediate snapshot into a fresh Session; the tests execute the
next transaction from those bytes and compare its complete UnitExecution.

The engine compares the full local VM account census before/after every transaction
against the declared closure, as well as checking compiled transaction keys.
An outside write is explicit and stops the scenario before another step. This
verification census is not exported or claimed as a complete Solana bank.

## Pair economics and result

Focused planner invocations evaluate A and B independently against the initial
reserve. Population `InsufficientReserve` classifications and funding order are
never copied. Each scenario recomputes its own expected reserve sequence from
those quotes. Actual reserve balances and transaction results come from the VM;
arithmetic does not produce the second outcome.

Each step retains the existing UnitExecution: instructions/message identity,
logs/compute identity, authority assumptions, source burn/escrow, destination
release, fees, supplies, reserve/balance deltas, reconciliation and failure
signature. It also retains before/after state IDs, expected reserve/funding,
observed reserve before/after and any outside writes. First-step output is the
intermediate state; last-step output is the final state. Solos retain the same
complete evidence.

`SharedReserveChangesSuccessfulUnit` and finding
`migration/order/shared_reserve/order_changes_successful_holder` require both
successful solo controls, complete ordered scenarios, changed per-unit outcomes,
reconciled successes, reserve-shortage rejection signatures with verified rollback,
and observed reserve sequencing matching the pair-specific model. The comparison
references all four scenario IDs and both order-case IDs and retains affected
units' outcomes, failure signatures and economic deltas.

`NoSuccessfulUnitEffect` records equivalent per-unit outcomes/economics with valid
controls. It does not require byte-identical final snapshots (transaction fees or
account creation can differ). `NotEstablished` avoids an ordering claim when
controls, reconciliation or the narrow reserve explanation do not establish it.
The full scenario evidence remains available in every case.

A supported rejected transaction is `Outcome::Rejected`, not unsupported or an
evidence gap. Existing rollback checks allow only the actual fee debit from the
payer; rent epoch is also checked. A failed rollback or successful execution with
unexpected economics is a reconciliation mismatch. Unsupported composition,
evidence gap, handoff failure and unexpected writes have separate typed statuses.

## Durable layout and reproduction

```text
order-case/
  case.json                         # manifest, written last
  report.md                         # contained human-readable order report
  evidence/
    closure/<hash>                  # original ChangeSpec
    checkpoints/<hash>              # retained initial World
    programs/<hash>                 # candidate ELF
    execution/<hash>                # Analysis, scenarios, comparison, states
    accounts/content/<hash>         # deduplicated exact AccountSnapshots
```

Snapshots in the analysis map are addressed by state ID. Their present account
rows reference shared CAS objects on disk; explicit absence remains in the
snapshot. The world uses the same account store, avoiding repeated account bytes
across world, initial, intermediate and final snapshots. Program dependencies are
retained in world account/ProgramData content; no external capture is needed.

Reproduction verifies CAS bytes, ChangeSpec/candidate/world/runtime identities,
manifest/binding/state IDs, then re-plans and executes all four fresh scenarios.
It compares the entire Analysis, including order-case IDs, closure, every
intermediate/final state, UnitExecution, reconciliation and pair comparison, and
the contained Markdown report. Rehashing a tampered intermediate CAS account does
not evade its state ID or the execution comparison. Missing bytes are errors;
there is no live lookup, repair or historical-report rewrite.

## Fixture and tests

`fixtures/migration/order/shared-reserve.recipe.json` is built through the real
fixture instruction/VM infrastructure with the pinned captured token programs and
the real migration candidate. The test terms quote 100 source → 50 destination
raw for A, and 120 → 60 for B. At reserve 80, both succeed alone; A → B leaves 30
and rejects B; B → A leaves 20 and rejects A. At reserve 110 both orders succeed.
Both destination ATAs start known absent. Tests verify exact creation bytes,
intermediate restoration, rollback and fully offline saved reproduction.

The focused suite also covers population-order independence, deterministic IDs,
metadata exclusion, order changes, proposal identity preservation, fixed Clock
boundaries, candidate/world/runtime/dependency/unit/order tampering, changed
intermediate bytes/existence even after CAS rehashing, missing authorities,
unknown absence, foreign sources, duplicate units, unsupported funding and inputs
outside the handoff closure. An internal census test covers writes, creations
and deletions outside the closure. CLI tests verify saved-run selection, typed
errors, empty-environment execution and reproduction without project context.

Hosted persistence/indexing and frontend work are not included. The portable
local artifact is the completed product boundary for this phase. Any subsequent
hosted exposure is a separate possible phase. This result establishes no
production fairness, complete-holder coverage, future ordering, issuer permission,
atomicity, universal safety, population behavior or governance/deployment status.

## Verification record (2026-09-29)

Final focused checks:

| Check | Result |
| --- | --- |
| `cargo test -p eplyx-engine --test migration_order` | 11 passed |
| `cargo test -p eplyx-engine --test migration_cli` (also run alongside order tests) | 10 passed |
| `cargo test -p eplyx-engine --lib migration::order::tests` | Account-census creation/deletion/write check passed |
| `cargo clippy -p eplyx-engine --lib --bin eplyx --test migration_order --test migration_cli -- -D warnings` | Passed |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |

The broader `cargo test -p eplyx-engine --test 'migration_*' --no-fail-fast`
attempt ran all 15 migration integration targets. Account, capture, current,
exact-amount, identity, package, rehearsal, search and stress targets passed.
Its initial CLI failure was the new test using the existing minimal example's
future activation; the test now explicitly prepares a proposal active at the
fixture Clock, and the complete 10-test CLI target passed afterward. The engine's
refusal of future-only activation was correct and was preserved.

The broad run was not green. Nine `migration_cases` tests and one
`migration_reference` test fail frozen T0 comparisons: the existing local
reference candidate has SHA-256
`d1df86d276582640a5585e774cb002cafe5eeef4c69b28941886590373efdc98`, while the
frozen records expect
`e5db6948abca1317eb12155f73cdaf619d378c22d1bfc992c977063e10e9c1bb`.
No candidate or frozen record was changed. `migration_demo` lacks its archived
population capture, and `migration_dashboard` cannot bind its loopback listener
in this sandbox. Its performance test remains ignored.

The migration library selection ran 47 passing tests (including the new census
check) and five authority-resolution failures caused by absent archived fixture
files. An initial `--all-targets` Clippy attempt was blocked compiling an unrelated
lifecycle test's missing `fixtures/lifecycle/sta/fixtures/token-2022-spacex-mint.json`;
the relevant library, binary and changed tests passed targeted Clippy.

The canonical order fixture initially lacked its pinned program capture. The
repository importer verified and restored that exact ignored capture from the
local frozen archive, restricted to its `pinned-programs` directory because
other legacy captures are unavailable. No provider configuration was read and no
captured bytes were replaced with synthetic program bytes.

The bounded engine/local capability has no known remaining implementation blocker.
A completely green repository-wide migration baseline still requires the pinned
legacy artifacts/binary and a test environment permitting the dashboard listener.
Those existing verification limitations and any future hosted productization are
outside this phase. No commit was made.
