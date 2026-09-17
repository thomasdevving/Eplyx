# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**Eplyx — Upgrade Impact CI**: a pre-deployment safety system for Solana programs. It executes identical transactions against two builds of the same program over a corpus of account states, and reports what changed *solely because the program version changed* — with an emphasis on economic consequences, not byte diffs.

`Eplyx` is a working name. It appears only in the CLI binary and the engine package name; it is deliberately kept out of the program ID, wire format, fixture schema, report schema and every core type, so renaming stays a rename rather than a migration.

## Toolchain

Both must be on `PATH`, and `cargo-build-sbf` shells out to `cargo`, so the Rust bin dir is required even for SBF builds:

```bash
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
```

Install: `rustup` (stable) and `sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"`. Node ≥ 22.6 and pnpm for the JSON contract check only.

## Commands

```bash
make                  # build V1+V2 to SBF, then run the full comparison
make test             # program tests (both flavours) + engine unit + end-to-end
make lint             # clippy -D warnings across both workspaces and both features
make fmt-check        # rustfmt across both workspaces
make fixtures         # regenerate fixtures/states/ after changing corpus.rs
make report           # JSON to report.json
make demo-token2022-upgrade   # Phase 7: real Token-2022 upgrade over a real PYUSD transfer
make demo-cpi-mainnet-replay  # Phase 8: real stake-pool upgrade over a real CPI deposit
```

Single tests:

```bash
cargo test --test upgrade_diff <name>                  # one end-to-end test
cargo test -p eplyx-engine --lib <module>::<name>      # one unit test
cargo test --manifest-path programs/fixture-lending/Cargo.toml \
  --no-default-features --features v2 <name>           # program test, one flavour
cargo test -p eplyx-engine --test stake_pool <name>    # one Phase 8 test
```

CLI:

```bash
cargo run -p eplyx-engine -- compare [--format json] [--no-minimize] [--fail-on-critical]
cargo run -p eplyx-engine -- reproduce boundary-position-017   # one fixture
cargo run -p eplyx-engine -- reproduce newly-liquidatable      # a regression group
pnpm install && pnpm verify:report                             # JSON contract check
```

Mainnet paths (need an archive endpoint; see `docs/phase-7-production-protocol-upgrade.md`):

```bash
eplyx versions resolve  --program <ID> --slot <S> --output <DIR> [--out v1.so]
eplyx versions upgrades --program <ID> --start-slot <A> --end-slot <B> --output <DIR>
eplyx historical acquire --signature <SIG> --program <ID> --output <DIR> [--offline]
eplyx compare --corpus <DIR>/corpus.json --current v1.so --candidate v2.so \
  [--dependencies <DIR>/dependencies]
```

`historical acquire` writes dependency binaries to `<DIR>/dependencies/<program
id>.so`; `compare` finds them there by default and refuses a file whose hash
differs from the one the record pins.

The CI path is offline and needs no endpoint at all:

```bash
eplyx bundle build --corpus <DIR> --baseline v1.so --dependencies <DIR>/dependencies \
  --target-size 10 [--observed '{"deposit":681,"withdraw":200}'] --out .eplyx/bundle
eplyx bundle verify --bundle .eplyx/bundle
eplyx ci check --bundle .eplyx/bundle --candidate target/deploy/program.so \
  [--expectations .eplyx/expected-changes.toml] [--format json]
```

**Invoke the built binary, not `cargo run`, wherever the exit code matters.**
`cargo run` replaces the child's exit status, which silently turns every gate
result into the same code. Exit codes: 0 passed, 1 undeclared or out-of-bounds
change, 2 configuration/fidelity error, 3 stale declaration, 4 bundle or
baseline incompatibility, 5 unevaluable declaration. Codes 2 and 4 are preflight aborts: they produce
no *analysis*, but under `--format json` they still emit a structured error
carrying the same exit code.

`artifacts/*.so` is gitignored. Anything that executes requires `./scripts/build-programs.sh` (or `make`) first; tests fail with an explanatory message rather than skipping.

## Architecture

### Two cargo workspaces, deliberately

`programs/fixture-lending/` is **not** a workspace member, and neither is any candidate program under `programs/`. They compile to SBF via `cargo-build-sbf` with the platform-tools toolchain, and their `solana-program` tree does not co-resolve with litesvm's pinned `solana-*` crates. Separate lockfiles keep both graphs free. `make fmt`, `make lint` and `scripts/test-programs.sh` cover each one explicitly — add a new candidate to all three.

### The V1/V2 mechanism

One source tree, two cargo features (`v1` / `v2`, mutually exclusive, `compile_error!` otherwise), producing two `.so` files with the **same program ID** — they only ever load into separate VM instances. The entire behavioural delta is one function, `collateral_value` in `programs/fixture-lending/src/math.rs`: V2 divides before multiplying, silently truncating fractional SOL.

`interface/` holds the wire format (instructions, account layouts, error codes) with no Solana dependency and no feature flags, linked by both the program and the engine. That is what makes "V1 and V2 have an identical interface" structural rather than asserted — neither build owns the definition.

### Layering, and the seam that matters

```
corpus → executor → diff → interpret → impact → cluster → shrink → report
```

- **Protocol-agnostic**: `executor`, `diff`, `money`, `report`, `hexfmt`, `types`, `dependencies`, `screening`, `versions`. These deal in accounts, bytes, balances, compute, programs and USD. They must not learn what a health factor is, and must contain no protocol names.
- **Protocol-aware**: `corpus`, `interpret`, `impact`, `cluster`, `shrink`. Everything that understands positions, health factors and liquidation lives here.

As of Phase 7 the seam **is a trait**: `protocol::ProtocolAdapter` owns which transactions a program can replay exactly, what its accounts mean, which programs and accounts it reaches, how an archived snapshot is proved against validator metadata, and what an execution difference means economically. `protocol::token2022` and `protocol::stake_pool` are the implementations. Phase 8 added `supports_cpi`, `dependency_programs`, `required_accounts` and `summarize`, all with defaults that leave an older adapter behaving exactly as before — `supports_cpi` defaults to `false` on purpose, so an adapter proved without CPI keeps the narrower guarantee. The fixture protocol and the bounded Phase 6 Memo path predate the trait and keep their inline contracts in `replay::validate`; new protocols arrive as adapters, not as another branch there. `corpus`, `interpret`, `impact`, `cluster` and `shrink` stay fixture-lending-specific. They are not reached by adapter records **because the generic diff takes its decoder as an argument**: `replay` passes `FieldDecoder::None` for any record an adapter owns, and `FieldDecoder::FixtureLending` only for the fixture and pre-adapter paths that define that layout. This was not always true — the diff used to dispatch on a leading discriminator byte, and a real stake-pool account (first byte `1`, 611 bytes) decoded as a synthetic `Market`, was compared over 86 bytes, and reported identical while `total_lamports` at offset 258 changed. Never infer a layout from bytes alone: an adapter record's economics come from `ProtocolAdapter::interpret`, and the report suppresses the position/USD block for them.

`shrink` re-enters `executor` to run candidate states — it sits above execution, never inside it.

### Historical mainnet layer

```
versions → dependencies ┐
screening ──────────────┼→ historical → replay → executor → diff → protocol::interpret
                        ┘
```

- `versions` resolves the program bytes deployed at a slot. Under the upgradeable loader that means following `UpgradeableLoaderState::Program` to ProgramData and reading it *at that slot*; the 45-byte header's deployment slot is monotone in the query slot, so `find_upgrades` bisects rather than scanning blocks. Large ProgramData accounts are read in 512 KiB chunks. Trailing padding is kept: that buffer is what the loader hands the VM.
- `historical::ProtocolArchiveProvider` acquires every non-program message account at `S-1` and `S` and delegates the proof to the adapter. A key absent at both boundaries whose validator balances are zero is recorded as absent, not invented.
- **V1 is the binary that actually ran**, so the fidelity gate is real evidence rather than a consistency check. This matters because validator metadata proves balances, not arbitrary account data: for a mint or an opaque PDA, V1 reproducing the original post-state is the *only* proof the reconstruction was right.
- `dependencies` answers *which programs a transaction actually needs*, from four merged routes: top-level instruction programs, inner-instruction programs, `Program <id> invoke [n]` log lines, and adapter declaration. Each is then classified by the owner of its account **at the transaction's slot** - `NativeLoader` means the runtime implements it and nothing may be substituted; anything else is resolved through `versions` and loaded explicitly, under the loader it had at that slot. LiteSVM's bundled SPL ELFs are convenience, not history, and are overridden.
- `screening` reads the block and rejects a candidate when any other transaction in the slot takes a required account as writable, naming the account, the conflicting transaction and which side of the boundary it spoils. Required for any adapter whose contract admits CPI: `validate()` refuses a CPI record with no screening evidence.
- **The invocation graph is part of the fidelity gate.** `OriginalExecution::cpi_invocations` is recorded from validator `innerInstructions` with depth and owning instruction, and the replay must reproduce it. A post-state can match by luck; a call sequence is much harder to match by luck.

### Execution model

`executor::execute` builds a **fresh `LiteSVM` per execution**, so "reset to identical initial state" is structural rather than a procedure that can drift. The clock sysvar is pinned; keypairs derive from seeds stored in the fixture; the fee payer is a separate account from the position owner so fee deduction never contaminates economic comparison. Execution runs real SBF bytecode — never call the program's Rust functions directly, or the tool tests a host build instead of the deployable artefact.

## Invariants — do not break these casually

- **The corpus is 141 fixtures producing 89 outcome-identical / 52 changed / 11 critical, $6,182,370 collateral represented.** These numbers are asserted in tests and are *derived from the arithmetic documented in `corpus.rs`*, not recorded from a previous run. If a change shifts them, the suite fails on purpose.
- **`fixtures/states/` must match the generator.** A test enforces it; run `make fixtures` after touching `corpus.rs`.
- **The reference math in `interface::reference` must never be imported by the program.** If it were, V2 could not diverge and the whole differential suite would be vacuous. `v2_disagrees_with_the_reference_somewhere` guards this.
- **No `f64` in the valuation or reporting path.** Enforced by `no_floating_point_in_the_valuation_or_reporting_path`, which scans non-test, non-comment source of `interface/src/lib.rs`, `money.rs`, `interpret.rs`, `impact.rs`, `diff.rs`, `report.rs`. Money is integer micro-USD; the compute percentage is integer basis points.
- **Account lamports and balance differences serialize as decimal strings**, via `numfmt::{u64_string, i64_string}`, alongside money and token quantities. A pool holding 15 million SOL is 1.5e16 lamports, and most JSON parsers — every JavaScript one — round past 2^53. Deserialization still accepts a number, so records written earlier keep parsing. Counts, basis points, slots and byte offsets stay plain numbers because they cannot plausibly get there. **Not yet converted:** transaction token amounts, the transaction balance arrays and post-account digests are still JSON numbers, so the rule is not uniform across every surface.
- **Monetary values serialize as decimal strings** (`"6182370.000000"`), never JSON numbers — a number becomes a double in most consumers.
- **`Difference` is an internally-tagged serde enum**, so it cannot carry `i128`/`u128` fields: serde's buffering for internally-tagged enums has no 128-bit variant, and the report would serialize but refuse to deserialize. Use `i64`.
- **Compute is off the pass/fail axis.** All 141 fixtures differ on compute, including the 89 whose state is byte-identical. Folding it in would classify the whole corpus as changed.
- **"Affected" means any non-compute difference.** Compute-only never counts as affected capital.
- **`newly_liquidatable` is narrower than it sounds**: the post-execution liquidation flag flipping false→true. The liquidation-boundary fixtures, where V1 rejects a liquidation and V2 permits it, are `transaction_now_succeeds`. Keep these distinct.
- **Dependency pins**: engine `solana-*` versions match litesvm's requirements exactly, because those types appear in litesvm's public API. A second semver-incompatible copy produces two distinct `Address`/`Instruction` types. `cargo tree -d` is the guard.
- **Token amounts are integer base units plus the mint's decimal count**, serialized as decimal strings, for the same reason `Usd` is. There is no cross-mint arithmetic: `TokenQuantity::delta` returns `None` when decimals differ, because subtracting two different assets is meaningless rather than merely imprecise.
- **Absent evidence is never read as absence of a problem.** A block account key that reports no `writable` flag fails screening rather than counting as read-only; a screening verdict is checked against the archive-sourced accounts the replay actually depends on rather than the set the record claims; acquisition and dependency slots must describe the transaction's own boundary; and a bundle manifest's `record_count`, `record_ids` and slot window are reconciled against the corpus rather than merely hashed. A self-hash proves bytes were not edited, not that they are described truthfully.
- **A corpus is *acquired* until something reproduces it.** `historical acquire` publishes what it could read and `corpus select` describes what it was handed; neither executes anything. `bundle build` is the first step holding the baseline and every dependency, so it is where V1 is replayed for every record and a non-reproducing one is refused. `bundle::Validation` is an explicit parameter with no default and `Skip` carries a reason, so assembling without validating is visible in a diff. The replay gate still re-checks at comparison time; building checks so a corpus is never published under a name it has not earned.
- **Discovery does not answer questions the adapter owns.** `replay_eligibility` delegates to `ProtocolAdapter::accept` and `supports_cpi` for any program with an adapter. The blanket "one top-level instruction, no inner instructions" rule is the pre-adapter contract, and applying it to an adapter program labelled both committed stake-pool records `unsupported_cpi` while replay accepted and reproduced them.
- **A bundle is built into an empty directory.** The corpus store is append-only, so building over an existing bundle keeps its records while the new manifest describes only the new ones — the result reopens happily and is wrong. Build fresh, review, then activate.
- **A boundary mismatch is an error, never an approximation.** Same-slot interference is common: of 29 direct zero-CPI Token-2022 transfers sampled across 13 blocks, none had clean boundaries on both sides. Candidate selection screens for it, and the error names which side failed.
- **`--fail-on-critical` also trips on `ReplayReport::economic_findings`.** `diff` is protocol-agnostic and can only rate a changed token account as `RawDataChanged`/warning; the adapter is what knows those bytes were a balance. Do not "fix" this by teaching `diff` about tokens.
- **An invocation's shape is program, depth, owning instruction, discriminant, account count and data length** — all six. Comparing `program@depth` alone reported a candidate that changed which instruction it called, or how many accounts it passed, as identical.
- **A CPI graph difference between V1 and V2 is reported, not scored.** `cpi_graph_changed` is a fact about the candidate; whether it matters is a semantic question this layer does not answer.
- **Dependency binaries are addressed by program ID and verified by hash.** A bundle that pairs one program's bytes with another's manifest entry, or a stale artefact, is an error - never a different replay.
- **`ReplayReport::timings` is `#[serde(skip)]`.** The determinism check compares report bytes, and a wall-clock number never repeats.
- - **Evidence is suppressed only where it is demonstrably accounted for.** A decoded change is dropped from the undeclarable list only when a finding emitted *for that observation* names it — never from a static promoted list, because `pool-mint/supply` is the burn on a withdrawal and a by-product of the mint on a deposit. A raw byte change is dropped only when every differing offset lies inside a range the adapter decodes *and* reported for that account (`ProtocolAdapter::decoded_byte_ranges`). Suppressing all structural evidence because something was named let a candidate rewrite a manager key behind one declared share change.
- **The CI gate consumes three layers of evidence, not one.** Named semantic findings are the only declarable layer, but a decoded economic change the adapter has not promoted, and a structural change on an observation nothing semantic spoke for, both fail the gate as `undeclarable_change`. Empty semantic coverage is `no_semantic_coverage` and exit 2, never a pass: zero findings from an adapter with no surface means "we did not look". `ProtocolAdapter::promoted_economic_fields` is how an adapter says which decoded fields its named findings already speak for.
- **Whether a change is *intended* is classified by `expected-changes.toml`,** not by the engine. A legitimate upgrade may change behaviour on purpose; the team declares it narrowly and the review reports expected / unexpected / exceeded / stale / unevaluable.

## Conventions and prior decisions

- **Anchor is not used.** It would add a CLI version dependency and a large tree to a program whose job is to be small with a hand-checkable byte layout — and that layout is what the diff engine decodes. Anchor's value (IDL-driven decoding) belongs to the phase supporting third-party protocols.
- **Integration tests live in `engine/tests/`**, not a top-level `tests/` — cargo binds integration tests to a package.
- **Fixtures carry state and transaction as one unit**, so there is no separate `fixtures/scenarios/` directory.
- Reporting vocabulary is deliberately conservative: "collateral represented", "affected collateral", "newly liquidatable collateral". There is no `capital_at_risk` field and the phrase is not used, because nothing here measures deployed capital.
- **The regression candidates are locally constructed counterexamples**, not proposed upstream releases. `fixture-memo-candidate` and `fixture-token2022-candidate` exist because a preserved outcome cannot by itself demonstrate that a changed outcome would be caught. Say so wherever their results are quoted.
- Minimization is a **witness, not a proof of minimality**: greedy, 96-probe budget, 0.001 SOL / $0.01 granularity, varies only collateral and debt. Say so in any output that quotes it.

## Cost notes

Minimization re-executes candidates: `compare` takes ~28s with it, ~6s with `--no-minimize`. The end-to-end suite is ~32s, mostly minimization; the shared minimized report is computed once via `OnceLock`. Avoid adding tests that call `compare_default_corpus_minimized()` a second time — test the shrinker directly instead.

## Scope

Done, on a synthetic corpus and a purpose-built fixture protocol (Phases 1-4): deterministic V1/V2 execution, structured diffing, economic impact aggregation, regression clustering, counterexample minimization, and durable offline replay of captured transactions.

Done, on real mainnet (Phases 5-8): bounded activity discovery and representative selection; slot-addressable historical state acquisition; historical program-version resolution and upgrade discovery under the upgradeable loader; one stateful production protocol replayed through the adapter trait against the real deployed V1 and V2 binaries; and one CPI-bearing stake-pool deposit replayed with every dependency binary pinned to the deployment live at its slot, screened against same-slot interference, and gated on the invocation graph as well as the post-state.

Done, on a validated production-derived corpus (Phase 9): automated generation of a regression corpus from real mainnet activity, and a deterministic selector (`corpus select`) that turns validated replay records into a corpus a gate can afford. Three populations — observed, replay-eligible, selected — are reported side by side and never collapsed; observed-to-replayable yield is a first-class coverage limitation; an unmeasured population is reported as absent, never as zero. A record is never duplicated to reach a target. The structural shape key includes the original CPI invocation signature, because a referral deposit mints twice where an ordinary one mints once while naming the same accounts. See `docs/phase-9-production-corpus.md`.

Done, as a pilot-ready product surface (Phase 10): an immutable, hash-addressed offline CI bundle; a versioned semantic finding vocabulary (`protocol/action/domain/subject/change`); `expected-changes.toml` and the expected / unexpected / exceeded / stale / unevaluable review; `eplyx ci check` with stable exit codes; and `eplyx-server`, a thin hosted API over the same engine. Two rules hold the design together: **corpus construction is not CI, corpus consumption is CI**, and **candidate code is built outside Eplyx** — the service never clones a repository or runs a build. Severity is descriptive metadata, never expectation identity and never gate policy. The hosted `report.json` is byte-identical to local `eplyx ci check --format json`. See `docs/phase-10-hosted-ci.md`; `scripts/hosted-demo.sh` proves it end to end.

**Bounded, not general.** The CPI path is one protocol (`SPoo1Ku8…`) and two instructions: `DepositSol` into a pool with no SOL deposit authority, reaching the System and SPL Token programs; and `WithdrawSol`, reaching SPL Token and the deployed Stake program, admitting a strictly-shaped top-level `Approve` companion. One level of invocation, into known programs. Do not describe it as CPI support.

Explicitly **not** started, and not to be begun without being asked: general CPI execution beyond that contract, address lookup table execution, account creation/closure in replay, failed-original replay, multi-instruction sequence search, protocols beyond Token-2022 and SPL Stake Pool (lending, vaults, AMMs), a GitHub App, frontend UI, AI analysis, and fiat valuation of protocol assets.

Expected-vs-unexpected classification and CI integration *are* done — see Phase 10 above. What remains unbuilt there is a GitHub App, a dashboard, and a queue.
