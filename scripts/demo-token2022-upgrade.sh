#!/usr/bin/env bash
# Phase 7: replay one real historical PYUSD transfer against the two Token-2022
# binaries that were actually deployed to mainnet on either side of a real
# upgrade, then against a deliberately regressed candidate.
#
# Needs no API key, wallet, private key or funded account.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# A direct TransferChecked of 10.000000 PYUSD: legacy, no lookup tables, no CPI,
# and no other transaction in its slot writes any of its accounts.
SIGNATURE="3omP6iKrk9jcFfjURFo76biHedNpXVJcK16jBxX3AUyqTQ4zr3ZHW5kpdue18TpVvBhjhkybEFUkhd4ivw4TaN2n"
PROGRAM="TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
TRANSACTION_SLOT=427146982
UPGRADE_SLOT=427147035

RPC_URL="${SOLANA_ARCHIVE_RPC_URL:-https://solana-mainnet.g.alchemy.com/v2/docs-demo}"
RPC_ORIGIN="${SOLANA_RPC_ORIGIN:-https://www.alchemy.com}"
PARENT="${1:-$ROOT/data/token2022-upgrade}"

mkdir -p "$PARENT"
RUN="$(mktemp -d "$PARENT/run.XXXXXX")"

cargo build -q -p eplyx-engine
./scripts/build-token2022-candidate.sh >/dev/null
EPLYX="$ROOT/target/debug/eplyx"

echo "== Upgrade history =================================================="
# Bisecting deployment slots finds the upgrade without scanning any blocks.
"$EPLYX" versions upgrades --program "$PROGRAM" \
  --start-slot 420000000 --end-slot 430000000 \
  --archive-rpc-url "$RPC_URL" --rpc-origin "$RPC_ORIGIN" --output "$RUN"

echo
echo "== The two binaries mainnet actually ran ============================"
# V1 is resolved during acquisition; fetch V2 explicitly for the comparison.
"$EPLYX" versions resolve --program "$PROGRAM" --slot "$UPGRADE_SLOT" \
  --archive-rpc-url "$RPC_URL" --rpc-origin "$RPC_ORIGIN" \
  --output "$RUN" --out "$RUN/token2022-v2.so"

echo
echo "== Acquire the historical transaction and its exact pre-state ======="
"$EPLYX" historical acquire --signature "$SIGNATURE" --program "$PROGRAM" \
  --transaction-rpc-url "$RPC_URL" --archive-rpc-url "$RPC_URL" \
  --rpc-origin "$RPC_ORIGIN" --output "$RUN"

# This second pass has no transport at all. Any missing response would fail.
"$EPLYX" historical acquire --signature "$SIGNATURE" --program "$PROGRAM" \
  --transaction-rpc-url "$RPC_URL" --archive-rpc-url "$RPC_URL" \
  --rpc-origin "$RPC_ORIGIN" --output "$RUN" --offline >/dev/null
echo "Offline re-acquisition reproduced the same record."

V1="$RUN/token-2022-mainnet-v1.so"

echo
echo "== A: real deployed V1 vs the real deployed upgrade ================="
"$EPLYX" compare --corpus "$RUN/corpus.json" --current "$V1" \
  --candidate "$RUN/token2022-v2.so" --format text --out "$RUN/report-real.txt"
sed -n '1,14p' "$RUN/report-real.txt"

echo
echo "== B: real deployed V1 vs a deliberately regressed candidate ========"
# The regression is locally constructed, not a real Token-2022 release. Without
# it, a preserved outcome in A would not show that a changed one is detected.
set +e
"$EPLYX" compare --corpus "$RUN/corpus.json" --current "$V1" \
  --candidate "$ROOT/artifacts/fixture_token2022_v2.so" \
  --format text --fail-on-critical --out "$RUN/report-regressed.txt"
GATE=$?
set -e
sed -n '1,16p' "$RUN/report-regressed.txt"
echo
echo "CI gate exit status for the regressed candidate: $GATE (1 = blocked)"
[ "$GATE" -eq 1 ] || { echo "expected the gate to block this candidate" >&2; exit 1; }

# Determinism: the same corpus twice must produce byte-identical JSON.
for n in 1 2; do
  "$EPLYX" compare --corpus "$RUN/corpus.json" --current "$V1" \
    --candidate "$RUN/token2022-v2.so" --format json --out "$RUN/report-$n.json"
done
cmp "$RUN/report-1.json" "$RUN/report-2.json"
echo "Byte-identical reports verified."
echo "Artifacts: $RUN"
