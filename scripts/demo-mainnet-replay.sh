#!/usr/bin/env bash
# Exact public-mainnet acquisition, offline V1 fidelity gate, and V2 comparison.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SIGNATURE="2KQ6LoGoez5dyuQ2VCAZKCMHUiQ6Zzr6Q7V88uSScWdJD9Kbt9vwLEi6QpSbPw6gDwXTQeGUbDNvg3KscqCRWqXU"
RPC_URL="${SOLANA_ARCHIVE_RPC_URL:-https://solana-mainnet.g.alchemy.com/v2/docs-demo}"
RPC_ORIGIN="${SOLANA_RPC_ORIGIN:-https://www.alchemy.com}"
PARENT="${1:-$ROOT/data/mainnet-replay}"

mkdir -p "$PARENT"
RUN="$(mktemp -d "$PARENT/run.XXXXXX")"

cargo build -q -p eplyx-engine
./scripts/build-memo-candidate.sh
EPLYX="$ROOT/target/debug/eplyx"

"$EPLYX" historical acquire \
  --signature "$SIGNATURE" \
  --transaction-rpc-url "$RPC_URL" \
  --archive-rpc-url "$RPC_URL" \
  --rpc-origin "$RPC_ORIGIN" \
  --output "$RUN"

# This second pass has no transport at all. Any missing response would fail.
"$EPLYX" historical acquire \
  --signature "$SIGNATURE" \
  --transaction-rpc-url "$RPC_URL" \
  --archive-rpc-url "$RPC_URL" \
  --rpc-origin "$RPC_ORIGIN" \
  --output "$RUN" \
  --offline

"$EPLYX" compare \
  --corpus "$RUN/corpus.json" \
  --current "$RUN/memo-mainnet-v1.so" \
  --candidate "$ROOT/artifacts/fixture_memo_v2.so" \
  --format json \
  --out "$RUN/report-1.json"
"$EPLYX" compare \
  --corpus "$RUN/corpus.json" \
  --current "$RUN/memo-mainnet-v1.so" \
  --candidate "$ROOT/artifacts/fixture_memo_v2.so" \
  --format json \
  --out "$RUN/report-2.json"
cmp "$RUN/report-1.json" "$RUN/report-2.json"

"$EPLYX" compare \
  --corpus "$RUN/corpus.json" \
  --current "$RUN/memo-mainnet-v1.so" \
  --candidate "$ROOT/artifacts/fixture_memo_v2.so" \
  --format text \
  --out "$RUN/report.txt"
cat "$RUN/report.txt"
echo "Byte-identical offline reports verified."
echo "Artifacts: $RUN"
