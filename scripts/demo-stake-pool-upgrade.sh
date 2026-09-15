#!/usr/bin/env bash
# Phase 8: replay one real historical SPL Stake Pool DepositSol against the two
# stake-pool binaries mainnet actually deployed on either side of a real upgrade,
# with every dependency binary pinned to the deployment live at that slot, then
# against a deliberately regressed candidate.
#
# Needs no API key, wallet, private key or funded account.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# A direct DepositSol of 0.423 SOL: legacy, no lookup tables, no account
# creation, and no other transaction in its slot writes any required account.
# It sits 3,339 slots before a real stake-pool upgrade.
SIGNATURE="58d7oY3zMFRSjcbEZErYYjxEuLEWzNSXphvHBfGNZ78hVznzmLnY1hPNhG8LpV83EYcha8wQn6XifPS53bxR1pgs"
PROGRAM="SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy"
TRANSACTION_SLOT=429878778
UPGRADE_SLOT=429882117

RPC_URL="${SOLANA_ARCHIVE_RPC_URL:-https://solana-mainnet.g.alchemy.com/v2/docs-demo}"
RPC_ORIGIN="${SOLANA_RPC_ORIGIN:-https://www.alchemy.com}"
PARENT="${1:-$ROOT/data/stake-pool-upgrade}"

# One session directory, reused. The transport cache is immutable and keyed by
# request, so a second run of this script needs no network at all - which is
# also what makes the offline pass below a real check rather than a formality.
RUN="$PARENT/session"
mkdir -p "$RUN"

cargo build -q -p eplyx-engine
./scripts/build-stake-pool-candidate.sh >/dev/null
EPLYX="$ROOT/target/debug/eplyx"

echo "== Upgrade history =================================================="
# Bisecting recorded deployment slots finds the upgrade without scanning blocks.
"$EPLYX" versions upgrades --program "$PROGRAM" \
  --start-slot 425000000 --end-slot 430000000 \
  --archive-rpc-url "$RPC_URL" --rpc-origin "$RPC_ORIGIN" --output "$RUN"

echo
echo "== The two stake-pool binaries mainnet actually ran ================="
# V1 is resolved during acquisition; fetch V2 explicitly for the comparison.
"$EPLYX" versions resolve --program "$PROGRAM" --slot "$UPGRADE_SLOT" \
  --archive-rpc-url "$RPC_URL" --rpc-origin "$RPC_ORIGIN" \
  --output "$RUN" --out "$RUN/stake-pool-v2.so"

echo
echo "== Acquire the transaction, its exact pre-state and its dependencies ="
# Resolves every program the transaction reaches - including the one it only
# reaches by CPI - at the slot before it executed, and screens the whole block
# for another transaction that would make a boundary ambiguous.
ACQUIRE_START=$SECONDS
"$EPLYX" historical acquire --signature "$SIGNATURE" --program "$PROGRAM" \
  --transaction-rpc-url "$RPC_URL" --archive-rpc-url "$RPC_URL" \
  --block-rpc-url "$RPC_URL" --rpc-origin "$RPC_ORIGIN" --output "$RUN"
echo "Acquisition wall clock: $((SECONDS - ACQUIRE_START))s (transaction slot $TRANSACTION_SLOT)"

# This second pass has no transport at all. Any missing response would fail.
"$EPLYX" historical acquire --signature "$SIGNATURE" --program "$PROGRAM" \
  --transaction-rpc-url "$RPC_URL" --archive-rpc-url "$RPC_URL" \
  --block-rpc-url "$RPC_URL" --rpc-origin "$RPC_ORIGIN" \
  --output "$RUN" --offline >/dev/null
echo "Offline re-acquisition reproduced the same record."

V1="$RUN/spl-stake-pool-mainnet-v1.so"
echo "Dependency bundle:"
ls -l "$RUN/dependencies" | tail -n +2

echo
echo "== A: real deployed V1 vs the real deployed upgrade ================="
# Everything from here runs with no transport: corpus, V1, V2 and the pinned
# dependency binaries are all on disk.
"$EPLYX" compare --corpus "$RUN/corpus.json" --current "$V1" \
  --candidate "$RUN/stake-pool-v2.so" --format text --out "$RUN/report-real.txt"
sed -n '1,40p' "$RUN/report-real.txt"

echo
echo "== B: real deployed V1 vs a deliberately regressed candidate ========"
# The regression is locally constructed, not a real stake-pool release. Without
# it, a preserved outcome in A would not show that a changed one is detected.
set +e
"$EPLYX" compare --corpus "$RUN/corpus.json" --current "$V1" \
  --candidate "$ROOT/artifacts/fixture_stake_pool_v2.so" \
  --format text --fail-on-critical --out "$RUN/report-regressed.txt"
GATE=$?
set -e
sed -n '/PROTOCOL RESULT/,/^EPLYX/p' "$RUN/report-regressed.txt" | sed '$d'
echo
echo "CI gate exit status for the regressed candidate: $GATE (1 = blocked)"
[ "$GATE" -eq 1 ] || { echo "expected the gate to block this candidate" >&2; exit 1; }

# Determinism: the same corpus twice must produce byte-identical JSON.
for n in 1 2; do
  "$EPLYX" compare --corpus "$RUN/corpus.json" --current "$V1" \
    --candidate "$RUN/stake-pool-v2.so" --format json --out "$RUN/report-$n.json"
done
cmp "$RUN/report-1.json" "$RUN/report-2.json"
echo "Byte-identical reports verified."
echo "Artifacts: $RUN"
