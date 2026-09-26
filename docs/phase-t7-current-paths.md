# T7 — Current-state path checks

MAIN now exposes current Transfer and bounded Meteora DLMM market-exit checks,
plus captured PositionV2 principal removal, through one `eplyx path` surface.
`eplyx observe` captures or re-decodes a current mint/owner selection. Current
observations remain separate from lifecycle declarations and historical replay.
No historical `ProtocolAdapter` was added; the pinned count remains five.

## Shared evidence

`evidence::current` pairs watched pre/post accounts by address through the existing
`evidence::account` rules. Token movements come from `evidence::token` and the
standard token decoders. Account creation, closure and absence stay explicit.
The swap adapter alone interprets creation of its selected destination ATA as a
zero opening balance for its conservation equation. Other required accounts must
exist on both sides. Byte-range measurements, Clock, Program/ProgramData headers,
RPC normalization and hashing also use MAIN primitives.

These measurements are across a local execution of captured bytes. They do not
fabricate the validator S-1/S proof required by `evidence::boundary`. Capture
records describe finalized observations across a slot range, not one validator bank.
The final dependency batch, exact deployed programs and decoded runtime clock bind
execution. Discovery is not execution; owner identity is not signing possession.

Current DLMM discovery scans only Token-2022 X to legacy-token Y, at most 64 pool
candidates and four internal-bitmap bin arrays. It chooses the lexicographically
first compatible layout and never substitutes another route after the selected
route fails. The exact user input and minimum output bind the executed instruction.
Transfer proves movement only. Withdrawal checks pool, owner, range and shares,
and reconciles user tokens, reserve debits, withheld fees, bin supply and position
shares. Principal removal does not establish fee collection, closure or conversion.
Proof cannot transfer across scope, bank, authority, account, amount or path.

## Commands and process boundary

- `eplyx observe capture --selection … --out …` reads bounded public state.
- `eplyx observe replay --input … --format json` re-decodes it offline.
- `eplyx path capabilities` and `validate` consume saved wallet bytes.
- `eplyx path capture` records typed current-check dependencies without execution.
- `eplyx path replay` verifies run/check IDs and wallet/capture digests, rebuilds
  the exact message and executes it in the local VM.
- `eplyx path capture-probe` saves a frozen lifecycle probe and its dependency
  capture in one directory; `path probe` replays it offline.
- `eplyx path discover-position` and `probe-withdrawal` use pinned discovery,
  snapshot, scenario and final dependency bytes. Optional `--terms` selects the
  exact removal range and fraction; its absence requests full principal removal.

Only capture parents can construct the read-only RPC client from the environment.
Offline commands run in `env_clear` children; workers reject inherited variables
and all capture commands. Outputs use exclusive creation and structured JSON
errors. Exit 0 means analysis completed, never transaction authorization.

## Reference and verification

[The source mapping](phase-t7-test-mapping.json) retains every assertion from the
five remaining source integration suites (56 tests). T6 already retained the
relevant private position-proof and token-transfer unit suites. T7 additionally
checks generic evidence boundaries, CLI isolation/determinism and route selection.

The [append-only reference package](../fixtures/path/README.md) preserves originals
and independently projects MAIN encodings. Wallet discovery slots and decoded
position timestamps now use decimal strings; raw RPC slot numbers remain exact.
The timestamp also changes linked counterfactual and assurance hashes, so
[an appended lifecycle projection](../fixtures/lifecycle/encoding-projection-t7.json)
records the complete dependency closure. The original T6 projection remains
unchanged. Loader deployment slots use MAIN's ProgramData decoder. No expected economic
quantity, path status, conservation condition or scope binding changed.

Full phase verification passed: `make test` recorded 1,286 passing tests and
`cargo test --no-fail-fast` recorded 1,235, each with the same two pre-existing
ignored tests. Formatting, workspace/program clippy, frontend, report and
governance checks passed. These execution counts include repeated checks and
are not unique-test totals. [The verification record](phase-t7-verification.json)
retains earlier failures, interrupted runs and their successful replacements.

The final source audit matched all 1,633 pinned STA files; STA remains clean.
The delta scan found no private endpoints, machine paths or added binary
payloads. No live provider, live-cluster transaction, deployment or push has
been used. Capture is tested with reconstructed mock providers; execution tests
replay frozen deployed bytes locally.
