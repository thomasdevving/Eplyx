#!/usr/bin/env bash
# Real validator/RPC capture followed by deterministic replay with validator OFF.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
cd "$ROOT"
command -v solana-test-validator >/dev/null || { echo 'solana-test-validator required' >&2; exit 1; }
./scripts/build-programs.sh
cargo build -q -p eplyx-engine
EP="$ROOT/target/debug/eplyx"
TASK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/eplyx-replay.XXXXXX")"
OUT="${1:-$ROOT/data/controlled-demo}"
mkdir -p "$OUT"
# A new validator has a new genesis: never reuse a previous chain cache.
OUT="$(mktemp -d "$OUT/run.XXXXXX")"
PORT="${EPLYX_DEMO_RPC_PORT:-18899}"
RPC="http://127.0.0.1:$PORT"
VALIDATOR_PID=""
cleanup() {
  if [ -n "$VALIDATOR_PID" ]; then kill "$VALIDATOR_PID" 2>/dev/null || true; wait "$VALIDATOR_PID" 2>/dev/null || true; fi
  if [ -f "$TASK_DIR/ledger/validator.log" ]; then cp "$TASK_DIR/ledger/validator.log" "$OUT/runtime.log"; fi
  # This unique mktemp directory contains all temporary key material/ledger.
  rm -rf "$TASK_DIR"
}
trap cleanup EXIT INT TERM
"$EP" controlled prepare --dir "$TASK_DIR"
solana-test-validator --ledger "$TASK_DIR/ledger" --rpc-port "$PORT" \
  --faucet-port "$((PORT+101))" --gossip-port "$((PORT+102))" \
  --dynamic-port-range "$((PORT+200))-$((PORT+300))" \
  --bind-address 127.0.0.1 --upgradeable-program "$(cat fixtures/program-id.txt)" artifacts/fixture_lending_v1.so none \
  --account-dir "$TASK_DIR/genesis" --log > "$OUT/validator.log" 2>&1 &
VALIDATOR_PID=$!
for attempt in $(seq 1 120); do
  if curl --silent --max-time 1 -H 'Content-Type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[{"commitment":"confirmed"}]}' "$RPC" | python3 -c 'import json,sys;sys.exit(json.load(sys.stdin).get("result",0)<3)' 2>/dev/null; then break; fi
  kill -0 "$VALIDATOR_PID" 2>/dev/null || { tail -30 "$OUT/validator.log"; exit 1; }
  sleep 0.5
done
"$EP" controlled capture --dir "$TASK_DIR" --snapshots "$OUT/snapshots" --rpc-url "$RPC" --current artifacts/fixture_lending_v1.so
START="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["start_slot"])' "$TASK_DIR/window.json")"
END="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["end_slot"])' "$TASK_DIR/window.json")"
"$EP" ingest --program "$(cat fixtures/program-id.txt)" --rpc-url "$RPC" --start-slot "$START" --end-slot "$END" --cache "$OUT/cache"
"$EP" corpus build --cache "$OUT/cache" --snapshots "$OUT/snapshots" --out "$OUT/corpus.json"
"$EP" discovery build --cache "$OUT/cache" --snapshots "$OUT/snapshots" --out "$OUT/discovery-corpus-1.json" --corpus-size 12 >/dev/null
kill "$VALIDATOR_PID"; wait "$VALIDATOR_PID" 2>/dev/null || true; VALIDATOR_PID=""
echo 'Validator stopped: the following analysis is offline.'
# Repeat ingestion from cache with RPC unavailable, then run the corpus twice.
"$EP" ingest --program "$(cat fixtures/program-id.txt)" --rpc-url "$RPC" --start-slot "$START" --end-slot "$END" --cache "$OUT/cache"
"$EP" discovery build --cache "$OUT/cache" --snapshots "$OUT/snapshots" --out "$OUT/discovery-corpus-2.json" --corpus-size 12 >/dev/null
cmp "$OUT/discovery-corpus-1.json" "$OUT/discovery-corpus-2.json"
"$EP" compare --corpus "$OUT/corpus.json" --format json --out "$OUT/report-1.json"
"$EP" compare --corpus "$OUT/corpus.json" --format json --out "$OUT/report-2.json"
cmp "$OUT/report-1.json" "$OUT/report-2.json"
"$EP" compare --corpus "$OUT/corpus.json" --out "$OUT/report.txt"
cat "$OUT/report.txt"
echo "Artifacts: $OUT"
