# Phase 6 — Exact historical state acquisition

Implemented: one slot-addressable historical-state provider, exact boundary
proofs against validator transaction metadata, immutable historical V1 bytecode
extraction, an `HistoricalStateReady` mainnet replay record, offline V1 fidelity validation,
candidate V2 execution, native-value impact, immutable request caching and a
transport-disabled cache replay.

The path is deliberately narrow: mainnet-beta, the immutable SPL Memo v2
program, one successful legacy transaction containing exactly one System
Program transfer followed by one account-free Memo instruction, no CPI, and two
data-empty System accounts. It is the first public-mainnet execution path, not a
claim of arbitrary protocol support.

## Run it

Prerequisites are the existing Rust/Solana toolchain and curl. The default
archive endpoint is Alchemy's public documentation endpoint and needs no API
key, wallet, private key or funded account.

```bash
./scripts/demo-mainnet-replay.sh
# optional output parent
./scripts/demo-mainnet-replay.sh ./data/my-mainnet-replay
```

Each run creates a fresh `run.XXXXXX` directory, builds the deliberately
regressed Memo-compatible candidate, acquires the fixed mainnet record, and then
repeats acquisition with the transport forcibly disabled. It compares the same
durable corpus twice and requires byte-identical JSON.

The acquisition command can also be used directly:

```bash
eplyx historical acquire \
  --signature 2KQ6LoGoez5dyuQ2VCAZKCMHUiQ6Zzr6Q7V88uSScWdJD9Kbt9vwLEi6QpSbPw6gDwXTQeGUbDNvg3KscqCRWqXU \
  --transaction-rpc-url https://solana-mainnet.g.alchemy.com/v2/docs-demo \
  --archive-rpc-url https://solana-mainnet.g.alchemy.com/v2/docs-demo \
  --rpc-origin https://www.alchemy.com \
  --output ./data/mainnet-replay

eplyx compare \
  --corpus ./data/mainnet-replay/corpus.json \
  --current ./data/mainnet-replay/memo-mainnet-v1.so \
  --candidate ./artifacts/fixture_memo_v2.so
```

`--offline` disables the transport and turns any cache miss into an error. URLs,
origin headers and credentials are excluded from records and reports; endpoint
hashes namespace the cache.

## Source evaluation and exactness boundary

[Standard `getAccountInfo`](https://solana.com/docs/rpc/http/getaccountinfo)
returns an account at the node's selected commitment and has no historical-slot
selector. `minContextSlot` is only a minimum context bound. Transaction archives
can supply old transactions, while [transaction metadata](https://solana.com/docs/rpc/json-structures#transactions)
supplies balances and execution observations rather than arbitrary account data.

| Source class | Decision | Reason |
|---|---|---|
| Standard Solana account RPC | Rejected for exact history | Current committed state is not transaction pre-state |
| Transaction/archive RPC | Supporting evidence only | Supplies message, outcome, fee and pre/post balances, not old account bytes |
| Slot-addressable account archive | Implemented | [Alchemy Account Archive](https://www.alchemy.com/docs/solana/account-archive) extends `getAccountInfo` with an exact finalized `slot` |
| Protocol-specific reconstruction | Not used | Would require a separately proved state transition history |
| Self-operated validator/account index | Future path | Valid in principle, but no such infrastructure is part of this repository |

`HistoricalStateProvider` is provider-neutral. The implemented
`SlotAccountArchiveProvider` keeps transaction and account providers separate,
requires identical genesis hashes, and requests each account at `S-1` and `S`.
Merely receiving bytes from an archive is insufficient.

For this execution contract Eplyx proves:

1. The transaction is successful, legacy, direct, exactly System transfer then
   Memo, and has no CPI or extra keys.
2. Each `S-1` archive balance equals that account's validator-observed
   `preBalances` value. This rejects an earlier same-slot write.
3. Each `S` archive balance equals `postBalances`. This rejects a later
   same-slot write that leaves a different boundary state.
4. Both state accounts are non-executable, data-empty, System-owned accounts;
   owner, data, executable bit and rent epoch are identical at both boundaries.
   The supported instructions can therefore change only the proved lamports.
5. The historical Memo account is executable, owned by the legacy BPF loader,
   and contains an ELF image. Legacy-loader deployment is immutable; the
   archived bytes become the pinned V1 artifact. Solana's deployment model and
   upgrade authority distinction are described in the
   [program deployment documentation](https://solana.com/docs/programs/deploying).
6. V1 must reproduce original success, fee and the complete watched post-state
   hash before candidate bytes are loaded.

The official [Memo program documentation](https://www.solana-program.com/docs/memo)
describes its UTF-8 and signer behavior. The bounded transaction has no Memo
accounts, and neither System transfer nor Memo reads `Clock`; replay persists
that assumption instead of generalizing it to other programs.

Any context-slot mismatch, null account, genesis mismatch, balance-boundary
mismatch, non-System state account, non-legacy program owner, missing ELF,
unsupported version/instruction/CPI, approximate source, wrong V1 bytes or V1
fidelity mismatch aborts. Candidate execution is withheld on V1 failure.

## Actual mainnet result

The fixed public transaction is
`2KQ6LoGoez5dyuQ2VCAZKCMHUiQ6Zzr6Q7V88uSScWdJD9Kbt9vwLEi6QpSbPw6gDwXTQeGUbDNvg3KscqCRWqXU`,
slot `447218873`, finalized at `2026-09-15 09:44:44 UTC`. It transferred
`19,661` lamports and then wrote an 88-byte Memo. Its exact evidence is:

```text
state source        historical_archive / HistoricalStateReady
fee                 5,000 lamports
pre-state SHA-256   5b29e923314147626739e5f1a3af6ce22af68ed43601e77f0d789f31e1182687
post-state SHA-256  c447c3c994cbdc7671153c353d4669b766984d88fd7f4e9d94d768636d58063a
V1 SBF SHA-256      f520eaf096361abbb9639ea4dc3e5388a87b9330e121f476607b87c46ef67954
```

The candidate preserves Memo's UTF-8 and signer checks but deliberately adds a
64-byte ceiling. This is a locally constructed regression candidate, not a
proposed upstream Memo release. On the historical interaction:

```text
V1  success; fee 5,000; post-state matches mainnet
V2  InstructionError(1, InvalidInstructionData)

payer       V1 3,582,692,290   V2 3,582,711,951   delta +19,661
recipient   V1 4,221,400,478   V2 4,221,380,817   delta -19,661

native transfer represented        19,661 lamports
candidate prevented                19,661 lamports
```

The V2 failure rolls back the preceding System transfer while preserving the
fee charge. No fiat value is inferred. This is an exact native-unit economic
diff, distinct from the fixture lending adapter's position/USD aggregation.

Two complete offline JSON reports were byte-identical, SHA-256:
`1fa334486dd918e2d9afaf3f93208091b7e963e65eefda3a568191f28355e85c`.

## Durable output and limitations

```text
run.XXXXXX/
  cache/
    transactions/<endpoint-sha256>/rpc-v1/<request-sha256>.json
    accounts/<endpoint-sha256>/rpc-v1/<request-sha256>.json
  snapshots/<signature>.json
  corpus.json
  memo-mainnet-v1.so
  report-1.json
  report-2.json
  report.txt
```

The transaction metadata now retains exact pre/post balance arrays as provenance
evidence. The record contains snapshots but not the V1 executable bytes; those
remain a separately hashed artifact, matching the existing comparison contract.
The committed [mainnet ReplayRecord](examples/mainnet-replay-record.json) is the
actual archive-derived record used for the measured result, not a hand-authored
fixture.

No upgradeable-loader ProgramData reconstruction, token-account interpretation,
arbitrary historical sysvars, CPI, address lookup-table execution, multiple
transfers, account creation/closure, failed-original replay, provider consensus,
cryptographic provider attestation, public-protocol semantic adapter, or fiat
valuation was added. Account Archive coverage begins in July 2025 according to
the provider documentation. Transactions outside the exact bounded contract are
rejected rather than approximated.

Final verification passed: `make fmt-check`, `make lint`, `make test`,
`pnpm verify:report`, and the fresh-cache `demo-mainnet-replay.sh`. The complete
suite contains 135 tests: 7 V1 + 7 V2 lending-program tests, 3 Memo-candidate
tests, 36 engine unit tests, 42 Phase 1–3 integration tests, 14 replay tests,
16 discovery tests, 4 historical-provider tests and 6 interface tests.
