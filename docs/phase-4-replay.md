# Phase 4 — Solana ingestion and controlled historical replay

Implemented: standard JSON-RPC activity discovery, legacy/v0 normalization,
resolved lookup-table keys, account discovery, immutable filesystem caching,
controlled pre-transaction snapshots, offline legacy replay with actual SBF,
V1 fidelity validation, V2 comparison, state hashing and existing economics.

The exact path is deliberately bounded to the fixture lending program, a
single legacy top-level instruction, no CPI, complete existing account snapshots.
The demo captures refresh instructions. Setup includes actual initialize-market,
create-position, deposit (including system-program CPI), borrow, set-price and
warmup-refresh transactions on a local Agave validator. Setup is ingested but
not selected into the replay corpus. Genesis supplies empty program-owned
accounts and native balances; protocol state is produced by the deployed SBF.

No arbitrary mainnet protocol support, guaranteed historical pre-state recovery,
SPL decoding, universal CPI reconstruction, sequence search, UI, AI or CI was added.

## Run it

Prerequisites: the existing Rust/Solana toolchain, `solana-test-validator`, curl,
Python 3 (script JSON extraction only). No wallet, API key or funded public account.

```bash
./scripts/demo-real-replay.sh
# optional output parent directory
./scripts/demo-real-replay.sh ./data/my-demo
```

Each run creates a fresh `run.XXXXXX` directory, with its own immutable cache.
The script starts an isolated validator on port 18899 (override
`EPLYX_DEMO_RPC_PORT`), loads V1 at genesis using the upgradeable loader with
upgrades disabled, waits for confirmed slots, generates three positions through
real transactions, captures pre/post accounts through RPC, ingests the slot
window, and builds the corpus. It stops the validator **before** replay and even
repeats ingestion from cache while the RPC is unavailable. Two JSON comparisons
must be byte-identical (`cmp`); text output follows. Temporary key material and
ledger are deleted on exit; resulting records contain no private keys.

Commands can also be used separately:

```bash
export SOLANA_RPC_URL=http://127.0.0.1:18899
eplyx ingest --program <PROGRAM_ID> --start-slot 3 --end-slot 100 --cache ./data/capture
eplyx corpus build --cache ./data/capture --snapshots ./data/snapshots --out ./data/corpus.json
eplyx compare --corpus ./data/corpus.json --current ./artifacts/fixture_lending_v1.so \
  --candidate ./artifacts/fixture_lending_v2.so --format json
```

`--rpc-url` overrides the environment. Slots are inclusive, discovery paginates
until the beginning of the window, and selected records sort by slot/signature.
`corpus build --limit N` selects the first N matching captures. Time-to-slot
conversion is not implemented. `compare --fail-on-critical` returns 1 on critical
findings; malformed data or fidelity failure returns 2.

## State provenance and fidelity

[Solana getMultipleAccounts](https://solana.com/docs/rpc/http/getmultipleaccounts)
returns an account snapshot at its response context slot. `minContextSlot` is a
lower bound, not an instruction to retrieve an account at an old slot. An archive
endpoint can retain transactions unavailable on a pruning public endpoint;
archive transaction availability does not itself supply historical account bytes.
[Transaction metadata](https://solana.com/docs/rpc/json-structures) supplies keys,
loaded addresses, balances and observed inner instructions, not arbitrary account
pre-state data. This distinction is reflected in the implementation:

| Source | Meaning | Current comparison behavior |
|---|---|---|
| controlled_snapshot | Captured before execution on our isolated validator | Exact only after V1 outcome, fee and complete captured post-state hash match |
| reconstructed | Explicit future/imported reconstruction | Matched if the supplied original outcome/hash match; no reconstruction algorithm implemented |
| historical_archive | Explicit future/imported archive snapshot | Matched if supplied original outcome/hash match; no account archive provider implemented |
| current_approximation | Account RPC sampled now | Approximate; not accepted for candidate comparison |

No original evidence produces `unknown`. An outcome, fee or post-state mismatch
produces `mismatch` and aborts comparison before loading candidate bytecode.
Malformed pre-state hashes, missing accounts, privilege mismatches, unsupported
versions/instructions/CPI and the wrong V1 binary also abort. A successful V1
transaction alone is insufficient to label a replay exact.

“Exact” means account/outcome/fee equality for this supported execution contract.
It does not assert identical validator internals, compute accounting or log text.
The fixture program reads only `Clock.slot`; that is set to the transaction slot.
Block time, epoch assumptions and default remaining sysvars are persisted or
explicitly described. Both VMs use the same pinned LiteSVM dependency/default
feature set. The controlled validator's actual deployed program bytes are checked
against the supplied V1 artifact before captures are accepted. CPI programs,
other sysvars, lookup-table execution and validator-feature reproduction need
explicit future support, not a broader interpretation of EXACT.

## Durable format and hashing

`ReplayRecord` schema version 1 contains:

- stable observation ID, program ID, genesis hash and V1 SHA-256;
- normalized source signature, slot/time, version, payer, original recent blockhash;
- ordered message keys and signer/writable flags, instruction bytes and metas;
- exact pre-state accounts, protocol labels, state source and clock assumptions;
- pre-state hash and independent original outcome/fee/post-state hash.

The original blockhash and full legacy message order are retained. Signature and
blockhash verification are disabled solely inside historical local execution;
message signer privileges remain enforced. Transactions are never resubmitted
by `compare`, and no wallet keys are needed.

State hashes use SHA-256 over a domain-tagged, length-delimited binary encoding:
accounts sorted by base58 address, address bytes, owner bytes, lamports LE,
executable byte, rent_epoch LE, data length LE and data. Labels are excluded.
Program bytecode has its own hash and is the comparison's intentional variable.
All captured non-program message accounts, including payer, are watched. Generated
reports include source/fidelity and pre/V1/V2 hashes. These hashes detect corruption;
they are not signatures certifying an untrusted snapshot provider.

[Example ReplayRecord](examples/replay-record.json) is a real captured interaction,
not a hand-authored fixture and contains no signing seeds.

## Cache and selection

A cache directory is an **immutable capture session**, not a live RPC mirror.
Use a new directory when the validator genesis changes or when you want fresh
activity. To rebuild, remove that session directory and rerun ingestion. The demo
always uses a new session, avoiding reuse after validator resets on the same port.
Do not run concurrent writers against one cache directory.

```text
run.XXXXXX/
  cache/
    <endpoint-sha256>/rpc-v1/<request-sha256>.json
    transactions/<signature>.json
    accounts/<signature>.json
    manifest.json
  snapshots/<signature>.json
  corpus.json
  report-1.json
  report-2.json
  report.txt
```

Endpoint URLs/API keys are not stored; URL hash namespaces transport requests.
Request hashes cover schema version, method and parameters. Cache writes are
atomic replacements; invalid cached JSON errors rather than silently refetching.
Null/unavailable transactions are not cached as permanent misses. Account samples
keep their response context slots and are labelled current_approximation, including
null closed accounts. Raw ingest data and selected replay records remain separate.

Selection accepts captures only when normalized transaction identity and genesis
match discovery. It never invents pre-state from current account samples. Ordinary
successful setup transactions without captures are intentionally not selected.
Once `corpus.json` is built, no RPC code is called during comparison.

## Economics and architecture

Existing `diff`, `interpret`, `impact`, and cluster building consume the replay
results. The synthetic corpus, CLI and minimizer continue working unchanged.
Replay comparison does not minimize captured records: changing their source state
would stop them being the recorded historical interaction. A future derived-witness
format can make this distinction explicit.

Capital totals are sums of interaction observations, not unique positions or TVL.
The demo has three distinct positions; later repeated interactions can count the
same economic exposure more than once. USD precision and serialization remain
integer micro-USD and decimal strings. Native JSON account u64 fields require a
lossless parser in JavaScript consumers; Rust preserves them exactly.

The new seams are `RpcProvider` → normalized transactions/snapshots → `ReplayRecord`
→ existing executor → existing interpretation/report. Future archive snapshots can
produce the same records; corpus selection can evolve separately; adapters still
need to formalize the existing protocol-specific interpretation boundary. CI can
consume the offline JSON/error codes later. None of these future systems is
implemented here.

## Verification and measurements

See the measured results appended below and the committed tests in
`engine/tests/replay.rs`. All ordinary tests run without RPC, keys or wall-clock
assumptions. The local-validator demo is a separate explicit verification script.
Timings are printed separately from deterministic JSON.

Actual controlled run (Agave 4.2.2, local validator, 2026-09-14):

```text
Captured three controlled interactions; slots 4..24
Ingested 21 transactions in 375 ms
Built 3 replay records in 3 ms
Validator stopped: the following analysis is offline.
Ingested 21 transactions in 14 ms  [cache, RPC unavailable]
Replay: total 240679 us; mean V1/V2 pair 80226 us; 6 VM executions
Replay: total 251790 us; mean V1/V2 pair 83930 us; 6 VM executions

controlled-000 | slot 10 | ControlledSnapshot | fidelity Exact
controlled-001 | slot 17 | ControlledSnapshot | fidelity Exact
controlled-002 | slot 24 | ControlledSnapshot | fidelity Exact

Fixtures tested:      3
Outcome identical:    1
Outcome changed:      2
Critical:             2
Collateral represented: $29,900.00
Debt represented:       $23,810.00
Net represented:         $6,090.00
Newly liquidatable: 2 observations, $19,900 collateral / $15,880 debt
```

The second item is the familiar counterexample: both succeed, health factor
1.003783 → 0.998738, liquidatable false → true, collateral 99.5 SOL/$9,950,
debt $7,930. The third has debt $7,950 and health 1.001257 → 0.996226.
The whole-unit control remains unchanged economically.

Both full replay JSON files are byte-identical, SHA-256:
`e7a341656ba41ff8cf97bc1752995131514bd733e7ce1d08bdef8d81bad6c5d9`.
Signatures, genesis, slots and hashes naturally differ when generating a **new**
chain; repeated analysis of the same durable corpus is deterministic.

[Full actual CLI output](examples/replay-report.txt) includes each source signature
and all pre/V1/V2 hashes. The committed example's original execution hash comes
from RPC, not the differential VM; `actual_validator_snapshot_matches_original_offline`
checks it against real V1 SBF and detects V2's regression.

Final verification passed: `make fmt-check`, `make lint` with warnings denied,
`make test`, and `pnpm verify:report`. The suite contains 7 V1 + 7 V2 program
unit tests, 36 engine unit tests, 42 Phase 1–3 integration tests, 12 Phase 4
integration tests and 6 interface tests (110 tests total). The original synthetic
baseline remains 141 / 89 identical / 52 changed / 11 critical, with five clusters
and $6,182,370 collateral represented. Main comparison, fixture reproduction,
critical-cluster reproduction and repeated full synthetic JSON were verified
in the [foundation audit](phase-1-3-audit.md).
