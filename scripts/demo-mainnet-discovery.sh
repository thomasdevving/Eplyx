#!/usr/bin/env bash
set -euo pipefail

# The compute-budget program is active, structurally varied, and requires no
# protocol-specific decoder. It is only a default discovery target; callers may
# pass any public program address.
program="${1:-ComputeBudget111111111111111111111111111111}"
parent="${2:-./data/mainnet-discovery}"
rpc_url="${SOLANA_RPC_URL:-https://api.mainnet-beta.solana.com}"
limit="${EPLYX_DISCOVERY_LIMIT:-10}"
corpus_size="${EPLYX_DISCOVERY_CORPUS_SIZE:-8}"
retries="${EPLYX_DISCOVERY_RETRIES:-5}"
backoff_ms="${EPLYX_DISCOVERY_BACKOFF_MS:-2000}"

mkdir -p "$parent"
run_dir="$(mktemp -d "$parent/run.XXXXXX")"

cargo run -q -p eplyx-engine -- discover \
  --program "$program" \
  --rpc-url "$rpc_url" \
  --limit "$limit" \
  --corpus-size "$corpus_size" \
  --retries "$retries" \
  --backoff-ms "$backoff_ms" \
  --output "$run_dir"

cp "$run_dir/discovery-corpus.json" "$run_dir/discovery-corpus-1.json"

# Every RPC response needed by the second run is cache-first. Its deterministic
# corpus must be byte-identical even though wall-clock timings are printed apart.
cargo run -q -p eplyx-engine -- discover \
  --program "$program" \
  --rpc-url "$rpc_url" \
  --limit "$limit" \
  --corpus-size "$corpus_size" \
  --retries "$retries" \
  --backoff-ms "$backoff_ms" \
  --output "$run_dir" >/dev/null

cmp "$run_dir/discovery-corpus-1.json" "$run_dir/discovery-corpus.json"
echo "Deterministic cache replay verified: $run_dir/discovery-corpus.json"
echo "Discovery complete."
echo "No trusted V1/V2 comparison performed because exact historical pre-state is unavailable."
