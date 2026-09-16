# Phase 5 — Mainnet discovery and representative corpus selection

Phase 5 adds deterministic public-program activity discovery without claiming
universal historical replay. It normalizes standard Solana JSON-RPC transaction
data, distinguishes direct and observable CPI invocation, fingerprints target
instructions, clusters recurring shapes, scores useful observations, removes
near-duplicates and emits a provenance-rich discovery corpus.

Discovery is deliberately separate from replay:

```text
RPC activity → normalized interactions → clusters → representative selection
                                                        ↓
                                                DiscoveryCorpus
                                                        ↓ future state provider
                                                  ReplayCorpus
```

An approximate current account sample can make an observation useful for
discovery. It does not make historical V1/V2 comparison trustworthy. Every
selected interaction therefore carries one of `historical_state_ready`,
`reconstructed_ready`, `approximate_only`, `missing_state`,
`unsupported_transaction`, or `unsupported_cpi`. The `compare --corpus` path
still accepts only the Phase 4 `ReplayRecord` contract and its fidelity gate.

## Run it

Use a bounded slot range for a reproducible source window:

```bash
eplyx discover \
  --program <PROGRAM_ID> \
  --rpc-url "$SOLANA_RPC_URL" \
  --start-slot <START> \
  --end-slot <END> \
  --limit 500 \
  --corpus-size 250 \
  --output ./data/mainnet-discovery
```

When slots are omitted, the end slot comes from a cached confirmed `getSlot`
response and the window begins 5,000 slots earlier. Output contains:

```text
mainnet-discovery/
  cache/
    <endpoint-sha256>/rpc-v1/<request-sha256>.json
    transactions/<signature>.json
    accounts/<signature>.json
    manifest.json
  discovery-corpus.json
  discovery-report.txt
```

The endpoint and any API key are not persisted. Retries are bounded with
exponential backoff. `--concurrency` bounds parallel transaction-history fetches
and is recorded in provenance; current-account samples remain serialized for
conservative public-RPC behavior.
Selected current-state observations retain their RPC response context slots;
those slots are never confused with the historical transaction slot.
Timing and cache hit/miss measurements are printed to stderr and excluded from
deterministic JSON.

Selection can also be repeated fully offline from normalized Phase 4 or mainnet
ingestion data:

```bash
eplyx discovery build --cache ./data/capture/cache \
  --out ./data/capture/discovery-corpus.json --corpus-size 250
```

For a controlled Phase 4 session, pass `--snapshots ./data/capture/snapshots`.
Each snapshot is validated against transaction, genesis, program, and the full
ReplayRecord contract before its interaction can be labeled `historical_state_ready`.

Inspect an endpoint without equating transaction history with account archives:

```bash
eplyx rpc inspect --rpc-url "$SOLANA_RPC_URL" --program <PROGRAM_ID>
```

The inspection reports the genesis hash, first available block, node version,
and a sampled signature/transaction capability. Standard Solana RPC has no
method for arbitrary historical account snapshots, so that capability is
reported separately rather than inferred from archive transaction access.

## Selection policy

`representative-v1` first represents as many deterministic clusters as the
corpus limit permits, rarest first. It then fills configurable strata for rare,
high-compute, failed, CPI, high-native-movement, and temporal-edge observations,
followed by the highest remaining scores. A deduplication bucket combines
cluster, compute band, native-movement band, and temporal decile.

Scores are integer-only and include an explicit breakdown:

- rarity of the complete interaction shape;
- compute percentile within the captured activity;
- historical failure (as an edge-case signal, not a bug claim);
- structural novelty;
- observable CPI;
- native lamport movement percentile, explicitly not protocol value or TVL;
- first/last temporal deciles.

The manifest reports cluster-by-cluster selection, failure coverage and observed
versus selected compute deciles. It reports observation count separately from
unique economic entities. Generic programs have no entity adapter, so the
latter is `null`/unknown rather than being equated to transaction count.

## Public-program demo

```bash
./scripts/demo-mainnet-discovery.sh
# or choose a program and output parent
./scripts/demo-mainnet-discovery.sh <PROGRAM_ID> ./data/my-discovery
```

The default is Solana's compute-budget program (`ComputeBudget111...`): it has
real, structurally varied activity, is simple enough for a bounded public-RPC
probe, and needs no semantic protocol decoder. The script discovers 10 recent
interactions by default, repeats the complete
command against the same immutable cache, and requires the two corpus JSON files
to be byte-identical. Use `EPLYX_DISCOVERY_LIMIT` and
`EPLYX_DISCOVERY_CORPUS_SIZE` to change the bounds.

External RPC is intentionally not part of the ordinary test suite. Provider
responses are cached, while deterministic classification, ranking, retry, cache,
serialization, and Phase 4 regression behavior are tested offline.

The Phase 4 controlled-validator demo now also runs `representative-v1` over its
known activity (including setup, warmup and the three captured refreshes), repeats
selection after the validator stops, and requires byte-identical discovery
corpora. This guards against setup/warmup activity dominating simply by order;
shape coverage and rarity drive the selection instead.

## Architecture and remaining coupling

The implemented boundaries are:

```text
activity discovery       ingest::discover_bounded
interaction normalization discovery::ProgramInteraction
clustering               deterministic complete-shape keys
selection                representative-v1 policy
state acquisition        HistoricalStateProvider seam
replay                    replay::ReplayRecord (unchanged)
execution                 executor (unchanged)
diffing                   diff (unchanged)
economic interpretation  fixture-lending adapter modules (unchanged)
```

At the Phase 5 boundary, exact replay supported only the controlled fixture
program's legacy single top-level instruction. Phase 6 subsequently added one
bounded slot-archive System-transfer/Memo path; economic entity identity and
asset valuation still require protocol adapters. CPI execution, v0 lookup
execution and general reconstruction remain outside the discovery layer.

Eplyx can now discover actual public-program activity, normalize and group its
observable execution patterns, select a reproducible representative discovery
corpus, preserve provenance, and state why each item is or is not replayable.
It still cannot universally reconstruct arbitrary historical pre-state, replay
every program/CPI graph, decode arbitrary economics, or guarantee regression
coverage.

## Verification and measured baseline

Actual public-RPC run on 2026-09-15, using the compute-budget program over slots
447180508–447185508:

```text
Transactions discovered/normalized: 5 / 5
Deterministic clusters:              4
Selected interactions:              5
Cluster coverage:                    4 / 4
First network run:                   2011 ms ingestion, 6 ms selection
RPC cache:                           0 hits / 13 transport request groups
Repeated cached run:                 27 ms ingestion, 8 ms selection
RPC cache:                           8 hits / 0 transport request groups
Replay eligibility:                 5 unsupported_cpi
```

All five surrounding transactions contained CPI, so none was promoted to trusted
replay. Repeated JSON was byte-identical, SHA-256
`601b7a6b3d753d5c4d7cbe604ec4b9827c19c59e59b1d8415d4a151eb978325c`.

The controlled Phase 4 cross-check discovered 21 transactions in 8 clusters and
selected 12 with 8/8 cluster coverage. Validated snapshots made one selected
interaction `historical_state_ready`; 8 were approximate-only and 3 retained unsupported
CPI status. Selection repeated byte-identically after the validator stopped.
The original replay remained 3 exact observations, 1 identical and 2 critical
newly-liquidatable outcomes.

Final checks passed: `make fmt-check`, `make lint`, `make test`, and
`pnpm verify:report`. The suite now contains 128 tests: 7 V1 + 7 V2 program
tests, 36 engine unit tests, 42 Phase 1–3 integration tests, 14 replay tests,
16 discovery tests, and 6 interface tests. External RPC remains outside ordinary
tests; the explicit demo supplies that integration check.
