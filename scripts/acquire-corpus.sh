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
#   EPLYX_LIMIT              interactions to normalize    (default 500)
#   EPLYX_CORPUS_SIZE        interactions to rank/select  (default 250)
#   EPLYX_TARGET_SIZE        observations to bundle       (default 10)
#   EPLYX_MAX_ACQUIRE        acquisitions to attempt      (default 40)
#
# Expect a narrow funnel. Measured on SPL Stake Pool over 700 blocks: 617
# relevant, 94 in the supported instruction family, 34 boundary-clean, 23
# replay-eligible. A window that yields nothing is ordinary; a window that
# yields nothing twice means scanning further, not relaxing anything.
set -uo pipefail

PROGRAM="${1:?usage: acquire-corpus.sh <program-id> [out-dir]}"
OUT="${2:-data/acquired-$(date +%Y%m%d-%H%M%S)}"
TARGET_SIZE="${EPLYX_TARGET_SIZE:-10}"
LIMIT="${EPLYX_LIMIT:-500}"
CORPUS_SIZE="${EPLYX_CORPUS_SIZE:-250}"
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
"$CLI" discover \
  --program "$PROGRAM" \
  --limit "$LIMIT" \
  --corpus-size "$CORPUS_SIZE" \
  ${WINDOW[@]+"${WINDOW[@]}"} \
  --output "$SESSION" || die "discovery"

# Only interactions whose exact historical state is available can be acquired.
# The rest are real activity that this contract cannot replay, and they stay
# visible in the discovery report rather than disappearing from the count.
# Written to files rather than read through a process substitution: the bash
# macOS ships mis-parses a heredoc inside one, and the funnel is worth keeping
# on disk anyway — it is the honest record of what this window could not offer.
python3 - "$SESSION/discovery-corpus.json" "$MAX_ACQUIRE" \
  >"$OUT/candidates.txt" 2>"$OUT/eligibility.txt" <<'PY'
import json, sys
from collections import Counter

# Named by what cannot be acquired, not by what can.
#
# `historical_state_ready` is not a label this path can produce: discovery
# samples *current* account state, so every shape it accepts comes back as
# `approximate_only`. Only a run given Phase 4 snapshots can claim exact state,
# and acquisition is the step that goes and gets it — asking discovery to have
# already proved it is asking the wrong question of the wrong command.
#
# So the filter excludes what the adapter's contract rules out and attempts the
# rest. Erring toward attempting is the safe direction: acquisition refuses
# loudly when a boundary cannot be proved, while a label added upstream that
# this list did not know about would otherwise be dropped in silence.
CANNOT_ACQUIRE = {"unsupported_transaction", "unsupported_cpi", "missing_state"}
corpus = json.load(open(sys.argv[1]))
selected = corpus["selected"]
ready = [
    s["interaction"]["signature"]
    for s in selected
    if s["interaction"].get("replay_eligibility") not in CANNOT_ACQUIRE
]
breakdown = Counter(s["interaction"].get("replay_eligibility") for s in selected)
for signature in ready[: int(sys.argv[2])]:
    print(signature)
print(f"{len(ready)} of {len(selected)} selected interactions are worth acquiring",
      file=sys.stderr)
for label, count in breakdown.most_common():
    print(f"  {label}: {count}", file=sys.stderr)
PY
sed 's/^/  /' "$OUT/eligibility.txt"

CANDIDATES=()
while IFS= read -r signature; do
  [ -n "$signature" ] && CANDIDATES+=("$signature")
done < "$OUT/candidates.txt"
echo "  ${#CANDIDATES[@]} candidate(s) to acquire"
# Not a failure of the tool: observed-to-replayable yield is a real property of
# the window, and is reported rather than engineered away. Widen the slot range,
# or read the breakdown above for what this contract cannot replay.
if [ "${#CANDIDATES[@]}" -eq 0 ]; then
  echo
  echo "  Nothing in this window is worth acquiring. That is a property of the"
  echo "  window and of the adapter's contract, not a failure here:"
  echo "    · a transaction that failed on mainnet is observed, never replayed"
  echo "    · this adapter replays DepositSol and WithdrawSol, and nothing else"
  echo "  Move the window with EPLYX_START_SLOT / EPLYX_END_SLOT before widening"
  echo "  it: a quiet period yields nothing however far it is scanned."
  die "no interaction in this window is worth acquiring"
fi

step "2. acquire exact historical state, one transaction at a time"
# This is where the funnel narrows for real. Measured on SPL Stake Pool: of 94
# in the supported family, 34 were boundary-clean and 23 reproduced V1. Most of
# what is attempted here is expected to be refused, each for a stated reason.
# Each acquisition reads every message account at S-1 and S from the archive,
# resolves the V1 binary deployed at that slot, and screens the block for
# same-slot interference. A refusal here is the design working: a boundary that
# cannot be proved is an error, never an approximation.
acquired=0
for signature in ${CANDIDATES[@]+"${CANDIDATES[@]}"}; do
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
#
# What discovery counts is *structure* - direct versus CPI - because that is all
# it can see without executing anything. A corpus population is keyed by
# semantic action, which only the adapter can name and only from an acquired
# record. Two taxonomies, no join. So these counts are printed and deliberately
# not passed to --observed: a key that matches nothing is not inert there, it is
# published as "production exercises this and this corpus cannot replay it",
# which would be a false coverage claim sealed inside an immutable bundle. The
# engine now refuses such a map outright. An unmeasured population is reported
# as absent, never as zero, and never as some other taxonomy's number.
python3 - "$SESSION/discovery-corpus.json" <<'PY'
import json, sys
from collections import Counter
corpus = json.load(open(sys.argv[1]))
counts = Counter(
    s["interaction"].get("interaction_type", "unknown") for s in corpus["selected"]
)
print("  observed by structure (not an action population):")
for kind, count in sorted(counts.items()):
    print(f"    {str(kind).lower()}: {count}")
PY
"$CLI" corpus select --corpus "$CORPUS" --target-size "$TARGET_SIZE" || die "selection"

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
