# Phase 7 — Stateful production-protocol upgrade replay

Implemented: a protocol adapter seam as a trait, historical program-version
resolution under the upgradeable loader, upgrade discovery by bisection,
stateful account acquisition proved against validator token balances, an
offline V1 fidelity gate against the binary that actually ran, and protocol
economic interpretation in integer token units.

The phase answers one question end to end:

```text
real production protocol      Token-2022, upgraded in place on mainnet
+ real historical user state  two PYUSD token accounts and PYUSD's mint, at slot S-1
+ real historical transaction a direct TransferChecked of 10.000000 PYUSD
+ actual deployed V1 binary   the deployment live at slot S, read from ProgramData
+ genuine V2 binary           the deployment the real upgrade installed
→ does the upgrade change the economic outcome?
```

It does not. That is the result, and it is a legitimate one: this is what a
pre-deployment gate should report for a behaviour-preserving upgrade. Because a
preserved outcome cannot by itself show that a *changed* outcome would be
caught, the same historical interaction is also run against a deliberately
regressed candidate, which is detected and blocks the gate.

## Why Token-2022

The target had to be a real upgraded production program whose direct
interactions are provable without executing anything else. Token-2022 holds real
balances for real assets, is upgraded in place under the upgradeable loader, and
its direct `TransferChecked` needs no CPI at all.

The economics are genuine but narrow: token amounts and decimals, not shares,
collateral or liquidation state. A protocol whose own accounting is richer -
lending, vaults, staking - would exercise more of the impact layer, at the cost
of CPI execution and PDA state that validator metadata cannot prove. That
trade-off is why this phase stops here.

Most Token-2022 mainnet traffic is *not* replayable by this contract. Sampling
the blocks around the upgrade, 317 transactions in one block referenced
Token-2022 and 5 had a top-level Token-2022 instruction; the rest arrived
through aggregators as versioned transactions with address lookup tables and up
to 26 inner instructions. Selection is the point, not a workaround: the
discovery layer already labels those `unsupported_cpi` and
`unsupported_transaction`.

## Run it

Prerequisites are the existing Rust/Solana toolchain and curl. The default
archive endpoint is Alchemy's public documentation endpoint and needs no API
key, wallet, private key or funded account.

```bash
./scripts/demo-token2022-upgrade.sh
# optional output parent
./scripts/demo-token2022-upgrade.sh ./data/my-token2022-run
```

The individual commands:

```bash
# Find the upgrades in a slot range without scanning a single block.
eplyx versions upgrades --program TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb \
  --start-slot 420000000 --end-slot 430000000 --output ./data/token2022

# Resolve the bytes that were deployed at one slot.
eplyx versions resolve --program TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb \
  --slot 427147035 --output ./data/token2022 --out ./data/token2022/v2.so

# Acquire the transaction, its exact pre-state, and the V1 that ran it.
eplyx historical acquire --signature <SIGNATURE> \
  --program TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb \
  --output ./data/token2022

eplyx compare --corpus ./data/token2022/corpus.json \
  --current ./data/token2022/token-2022-mainnet-v1.so \
  --candidate ./data/token2022/v2.so --fail-on-critical
```

`--offline` disables the transport and turns any cache miss into an error.
Endpoints, origin headers and credentials never reach a record or report.

## Historical version resolution

Phase 6 pinned V1 by reading an immutable legacy-loader program account, where
the executable bytes live in the program account and cannot be replaced. An
upgradeable program breaks both halves of that: the program account holds only a
pointer to a ProgramData account, and that account is rewritten on every upgrade.

`versions::resolve_at` therefore reads ProgramData *at a slot*: it follows
`UpgradeableLoaderState::Program`, reads the 45-byte ProgramData header for the
deployment slot and upgrade authority, and takes the executable buffer after it,
chunking the read because mainnet ProgramData accounts run to 10 MiB. Trailing
padding is kept, because that buffer is what the loader hands the VM.

The header's deployment slot is a monotone non-decreasing step function of the
query slot, so `versions::find_upgrades` bisects instead of scanning. Locating
the upgrade inside a 10,000,000-slot window took 6.1 s and a few dozen 45-byte
reads.

## The exactness boundary

| Evidence | What it establishes |
|---|---|
| Archive account at `S-1` vs `preBalances` | No earlier same-slot write moved this account's lamports |
| Archive account at `S` vs `postBalances` | No later same-slot write left a different boundary state |
| Decoded token amount vs `preTokenBalances`/`postTokenBalances` | The stateful analogue of the lamport proof: the validator recorded the exact base-unit amount on both sides |
| Read-only accounts byte-identical across the boundary | The mint could not have changed during the transaction |
| V1 reproducing success, fee and the full post-state hash | Everything metadata cannot prove directly, including mint bytes and extension data |

That last row carries real weight here. Validator metadata records balances, not
arbitrary account data, so for a mint or an opaque PDA the archive's bytes are
not independently provable. What makes the reconstruction trustworthy is that
V1 is the code that actually produced the recorded outcome: if any input had
been wrong, replaying the original binary would not have reproduced the original
post-state. Where Phase 6's fidelity gate was a consistency check, here it is
the primary evidence.

Same-slot interference is common enough to matter. Of 29 direct zero-CPI
Token-2022 transfers found in 13 blocks, **none** had clean boundaries on both
sides: the senders were bots submitting several transactions per slot, each
writing its own fee payer and token account. Candidates are screened by
requiring that no other transaction in the block writes any of their accounts.
A mismatch is rejected with a message naming which side failed and why, rather
than being approximated.

## Deliberately out of scope

No CPI execution, no address lookup table execution, no account creation or
closure, no failed-original replay, no multisig authorities, no transfer-hook
extra accounts, no unchecked `Transfer` (it carries no mint, so a replay could
not confirm the decimals the original execution validated against), no fiat
valuation, and no expected-versus-unexpected change classification. A protocol
upgrade may change behaviour intentionally; this phase quantifies the change and
declines to judge intent.

The runtime feature set is LiteSVM's mainnet snapshot rather than a
slot-accurate reconstruction. Both builds run under the identical set, so the
differential stays sound, and V1 reproducing the original outcome is what rules
out a material difference. One visible consequence: V1 consumed 3,800 compute
units against the 3,802 the validator recorded — an accounting difference
between runtimes, and a concrete reason compute stays off the pass/fail axis.

## Actual mainnet result

The fixed public transaction is
`3omP6iKrk9jcFfjURFo76biHedNpXVJcK16jBxX3AUyqTQ4zr3ZHW5kpdue18TpVvBhjhkybEFUkhd4ivw4TaN2n`,
slot `427146982`, finalized at `2026-06-17 20:57:35 UTC`. It is a direct
`TransferChecked` of 10.000000 PYUSD, with no CPI and no lookup tables, whose
accounts no other transaction in its block writes.

```text
program              TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb (Token-2022)
ProgramData          DoU57AYuPFu2QU514RktNPG22QhApEjnKxnBcu4BHDTY
upgrade authority    AeLmXCbPaQHGWRLr2saFsEVfmMNuKnxRAbWCT9P5twgz
asset                2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo (PYUSD, 6 decimals)
state source         historical_archive / HistoricalStateReady
fee                  10,000 lamports
pre-state SHA-256    7ea7c0d62622b5c9bfb8241df2adcabeb1e7cb17ec877e692e67e6d969ce3e7c
```

Both binaries are real mainnet deployments, 1,382,016 bytes each, differing in
24% of their bytes:

```text
V1   deployed at slot 395047597   b2a7ce1ea6dfbcbc5ccb0e7f48f7c61dced1a86582d1c7d2e059ac54ed612da4
V2   deployed at slot 427147035   0999dbf708971e723b08d1caafc988826a59c6001ed6dc02260da07defbe1469
```

The upgrade landed at slot `427147035`, 53 slots after the replayed transaction.

### A — the real upgrade

```text
V1  success; fee 10,000; post-state 459306ff…  matches mainnet exactly
V2  success; fee 10,000; post-state 459306ff…  identical

source        10.000000 → 0.000000 PYUSD      under both builds
destination   66,665.509183 → 66,675.509183   under both builds

protocol economics   no field differs
compute              3,800 → 3,848 units (+1.26%), tracked off the pass/fail axis
CI gate              exit 0
```

Eplyx replayed a real production upgrade against real historical state and
quantified the delta as zero.

### B — a deliberately regressed candidate

`programs/fixture-token2022-candidate` implements `TransferChecked` with the
same validation this path applies — mint agreement, declared decimals, a signing
authority, an initialized source, a sufficient balance — and one defect: the
destination is credited one basis point less than the source is debited, as an
unintended truncation would do. It is a locally constructed counterexample, not
a proposed Token-2022 release, and it is byte-compatible with real mainnet
accounts because it leaves extension bytes untouched.

```text
V1  success; post-state 459306ff…  matches mainnet exactly
V2  success; post-state c53f42f4…

destination (token-account) amount
  V1  66,675.509183
  V2  66,675.508183
  delta  -0.001000 PYUSD

CI gate              exit 1 (blocked)
```

The value silently disappears: the transaction still succeeds, the source is
still debited in full, and only the credited amount is short. A protocol-level
economic change fails `--fail-on-critical` even though the protocol-agnostic
classifier, which sees only that a token account's bytes changed, rates it a
warning. That split is deliberate — `diff` is not allowed to learn what a
balance means, so the adapter reports economics separately.

Two complete JSON reports over the same corpus were byte-identical, and a second
acquisition with the transport disabled reproduced the record from cache alone
in 137 ms.

## Architecture

The adapter seam was a module convention through Phase 6, and the mainnet path
had begun bolting a second hard-coded contract next to the fixture one. Two
protocols is where that stops scaling, so `protocol::ProtocolAdapter` is now a
trait: it owns which transactions a program can replay exactly, what its
accounts mean, how a snapshot is proved, and what an execution difference means
economically. Everything it returns is protocol-agnostic.

```text
versions          resolve the deployed bytes at a slot; bisect upgrade history
historical        acquire message accounts at S-1 and S; delegate proof to the adapter
protocol          the adapter trait and its registry
protocol::token2022  Token-2022 account decoding, contract, proof, interpretation
replay            fidelity gate and execution (unchanged contract, adapter-dispatched)
executor / diff   unchanged, and still unable to learn protocol semantics
```

Token amounts are integer base units carried with their mint's decimal count and
serialized as decimal strings, for the same reason `money::Usd` is: a u64 token
amount does not survive a JSON double. The no-floating-point guard now covers
both new protocol modules.

## Verification

`make fmt-check`, `make lint`, `make test` and `pnpm verify:report` all pass, as
does a fresh-cache `demo-token2022-upgrade.sh`. The Phase 1–3 invariants are
untouched: 141 fixtures, 89 outcome-identical, 52 changed, 11 critical,
$6,182,370 collateral represented.

The suite contains 162 tests: 7 V1 + 7 V2 lending-program tests, 3 Memo-candidate
tests, 5 Token-2022-candidate tests, 48 engine unit tests, 42 Phase 1–3
integration tests, 14 replay tests, 16 discovery tests, 4 historical-provider
tests, 10 Token-2022 tests and 6 interface tests. External RPC stays outside the
ordinary suite: the Token-2022 tests run offline against a deterministic archive
stub and against the committed
[mainnet record](examples/mainnet-token2022-record.json), which is the real
archive-derived artefact this phase was measured on rather than a hand-authored
fixture.
