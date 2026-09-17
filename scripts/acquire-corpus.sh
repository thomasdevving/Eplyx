#!/usr/bin/env bash
# Build a validated production corpus, and a CI bundle from it.
#
# This is the administrative half of Eplyx: it needs archive credentials, takes
# minutes, and runs rarely. Nothing on the pull-request path touches any of it —
# `eplyx ci check` and the hosted server read the bundle this produces and
# execute offline.
#
#   discover ──> historical acquire ──> corpus select ──> bundle build ──> verify
#      │                 │                    │                 │
#   Helius          Helius + Alchemy       offline           offline
#
# Two transports, deliberately. Transaction history is ordinary archival RPC;
# account state at an exact slot is not, and the endpoint that serves it is
# named separately so a standard one cannot quietly stand in. Prove yours first:
#
#   scripts/probe-archive.sh "$SOLANA_ARCHIVE_RPC_URL" "$SOLANA_RPC_URL"
#
# Usage:
#   scripts/acquire-corpus.sh <program-id> [out-dir]
#
# Environment:
#   SOLANA_RPC_URL           transaction history          (required)
#   SOLANA_ARCHIVE_RPC_URL   account state at exact slot  (required)
#   SOLANA_BLOCK_RPC_URL     blocks for same-slot screening (defaults to the above)
#   EPLYX_START_SLOT         window start (default: a bounded recent window)
#   EPLYX_END_SLOT           window end
#   EPLYX_TARGET_SIZE        observations to bundle       (default 10)
#   EPLYX_MAX_ACQUIRE        acquisitions to attempt      (default 40)
set -uo pipefail

PROGRAM="${1:?usage: acquire-corpus.sh <program-id> [out-dir]}"
OUT="${2:-data/acquired-$(date +%Y%m%d-%H%M%S)}"
TARGET_SIZE="${EPLYX_TARGET_SIZE:-10}"
MAX_ACQUIRE="${EPLYX_MAX_ACQUIRE:-40}"
CLI=./target/release/eplyx

[ -x "$CLI" ] || { echo "build first: cargo build --release -p eplyx-engine"; exit 2; }
: "${SOLANA_RPC_URL:?set SOLANA_RPC_URL to a transaction-history endpoint}"
: "${SOLANA_ARCHIVE_RPC_URL:?set SOLANA_ARCHIVE_RPC_URL to a slot-addressable account archive}"
export SOLANA_BLOCK_RPC_URL="${SOLANA_BLOCK_RPC_URL:-$SOLANA_RPC_URL}"

SESSION="$OUT/session"
CORPUS="$OUT/corpus"
mkdir -p "$SESSION" "$CORPUS"
step() { printf '\n== %s ==\n' "$1"; }
die() { echo "  FAILED: $1"; exit 1; }

# Left to the CLI's own bounded default unless a window is named: 5,000 slots
# ending at the current one, which is the shape discovery was proved under.
WINDOW=()
[ -n "${EPLYX_START_SLOT:-}" ] && WINDOW+=(--start-slot "$EPLYX_START_SLOT")
[ -n "${EPLYX_END_SLOT:-}" ] && WINDOW+=(--end-slot "$EPLYX_END_SLOT")
step "1. discover activity${EPLYX_START_SLOT:+ in slots $EPLYX_START_SLOT..${EPLYX_END_SLOT:-latest}}"
"$CLI" discover --program "$PROGRAM" "${WINDOW[@]+"${WINDOW[@]}"}" --output "$SESSION" \
  || die "discovery"

# Only interactions whose exact historical state is available can be acquired.
# The rest are real activity that this contract cannot replay, and they stay
# visible in the discovery report rather than disappearing from the count.
CANDIDATES=()
while IFS= read -r signature; do
  [ -n "$signature" ] && CANDIDATES+=("$signature")
done < <(python3 - "$SESSION/discovery-corpus.json" "$MAX_ACQUIRE" <<'PY'
import json, sys
from collections import Counter

# The serialized spelling is snake_case, and `exact_ready` is the pre-rename
# name still accepted on read. Matching the Rust variant name instead finds
# nothing, and finding nothing here looks exactly like a window with no
# eligible activity — so the breakdown is printed either way.
READY = {"historical_state_ready", "exact_ready"}
corpus = json.load(open(sys.argv[1]))
selected = corpus["selected"]
ready = [
    s["interaction"]["signature"]
    for s in selected
    if s["interaction"].get("replay_eligibility") in READY
]
breakdown = Counter(s["interaction"].get("replay_eligibility") for s in selected)
print("\n".join(ready[: int(sys.argv[2])]))
print(f"# {len(ready)} of {len(selected)} selected interactions have exact historical state",
      file=sys.stderr)
for label, count in breakdown.most_common():
    print(f"#   {label}: {count}", file=sys.stderr)
PY
)
echo "  ${#CANDIDATES[@]} candidate(s) to acquire"
# Not a failure of the tool: observed-to-replayable yield is a real property of
# the window, and is reported rather than engineered away. Widen the slot range,
# or read the breakdown above for what this contract cannot replay.
[ "${#CANDIDATES[@]}" -gt 0 ] || die "no interaction in this window has exact historical state"

step "2. acquire exact historical state, one transaction at a time"
# Each acquisition reads every message account at S-1 and S from the archive,
# resolves the V1 binary deployed at that slot, and screens the block for
# same-slot interference. A refusal here is the design working: a boundary that
# cannot be proved is an error, never an approximation.
acquired=0
for signature in "${CANDIDATES[@]}"; do
  if "$CLI" historical acquire \
      --signature "$signature" \
      --program "$PROGRAM" \
      --output "$CORPUS" >"$OUT/acquire-$signature.log" 2>&1; then
    acquired=$((acquired + 1))
    printf '  ok   %s\n' "${signature:0:24}…"
  else
    printf '  skip %s  %s\n' "${signature:0:24}…" \
      "$(grep -m1 -oE 'error: .*' "$OUT/acquire-$signature.log" | cut -c1-96)"
  fi
done
echo "  $acquired of ${#CANDIDATES[@]} acquired"
[ "$acquired" -gt 0 ] || die "nothing could be acquired; see $OUT/acquire-*.log"

step "3. report the three populations"
# Observed, replay-eligible and selected are never collapsed into one number.
OBSERVED=$(python3 - "$SESSION/discovery-corpus.json" <<'PY'
import json, sys
from collections import Counter
corpus = json.load(open(sys.argv[1]))
counts = Counter(
    s["interaction"].get("interaction_type", "unknown") for s in corpus["selected"]
)
print(json.dumps({str(k).lower(): v for k, v in counts.items()}))
PY
)
echo "  observed: $OBSERVED"
"$CLI" corpus select --corpus "$CORPUS" --target-size "$TARGET_SIZE" --observed "$OBSERVED" \
  || die "selection"

step "4. build the bundle"
# The baseline is the V1 binary acquisition resolved for these records. Every
# record must pin the same one: if the program was upgraded inside the window,
# this refuses rather than bundling two histories under one name.
BASELINE=$(ls "$CORPUS"/*-mainnet-v1.so 2>/dev/null | head -1)
[ -n "$BASELINE" ] || die "no V1 artifact in $CORPUS"
echo "  baseline $(basename "$BASELINE")"
rm -rf "$OUT/bundle"
"$CLI" bundle build \
  --corpus "$CORPUS" \
  --baseline "$BASELINE" \
  --dependencies "$CORPUS/dependencies" \
  --target-size "$TARGET_SIZE" \
  --observed "$OBSERVED" \
  --out "$OUT/bundle" || die "bundle build"

step "5. verify it the way the gate will"
"$CLI" bundle verify --bundle "$OUT/bundle" || die "bundle verify"

cat <<SUMMARY

Bundle at $OUT/bundle

Next:
  cp -R "$OUT/bundle" deploy/bundle          # so the image carries it
  eplyx ci check --bundle "$OUT/bundle" --candidate <candidate.so>

Nothing beyond this point needs an endpoint. The bundle holds the baseline,
every dependency binary and the validated records, and the gate runs offline.
SUMMARY
