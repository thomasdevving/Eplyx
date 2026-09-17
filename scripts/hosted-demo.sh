#!/usr/bin/env bash
# End-to-end proof of the hosted CI API, entirely offline.
#
# Every RPC variable is explicitly unset for the whole run: if a hosted
# candidate check ever needs an endpoint, this script fails rather than quietly
# depending on one.
#
# Usage: scripts/hosted-demo.sh <bundle-dir> [work-dir]
set -uo pipefail

BUNDLE="${1:?usage: hosted-demo.sh <bundle-dir> [work-dir]}"
WORK="${2:-$(mktemp -d)}"
PORT="${EPLYX_DEMO_PORT:-8899}"
URL="http://127.0.0.1:$PORT"
CANDIDATE_REGRESSED="artifacts/fixture_stake_pool_v2.so"
PROGRAM=SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy

SERVER=./target/release/eplyx-server
CLI=./target/release/eplyx
[ -x "$SERVER" ] || { echo "build first: cargo build --release -p eplyx-server"; exit 2; }
[ -x "$CLI" ] || { echo "build first: cargo build --release -p eplyx-engine"; exit 2; }

export EPLYX_DATA_DIR="$WORK/data"
export EPLYX_BIND="127.0.0.1:$PORT"
mkdir -p "$EPLYX_DATA_DIR" "$WORK/expect"

fail=0
check() { # label actual expected
  if [ "$2" = "$3" ]; then printf '  ok   %-46s %s\n' "$1" "$2"
  else printf '  FAIL %-46s %s (want %s)\n' "$1" "$2" "$3"; fail=1; fi
}

echo "== provisioning =="
"$SERVER" admin create-project --id demo --name Demo --program-id "$PROGRAM" > "$WORK/p.txt" 2>&1
TOKEN=$(grep -oE 'eplyx_[0-9a-f]+' "$WORK/p.txt")
"$SERVER" admin create-project --id other --name Other --program-id "$PROGRAM" > "$WORK/p2.txt" 2>&1
TOKEN2=$(grep -oE 'eplyx_[0-9a-f]+' "$WORK/p2.txt")
BSHA=$("$SERVER" admin install-bundle --path "$BUNDLE" 2>&1 | head -1 | awk '{print $3}')
"$SERVER" admin activate-bundle --project demo --bundle "$BSHA" > /dev/null 2>&1
echo "  bundle $BSHA"

# Declarations used by the cases below.
cat > "$WORK/expect/bounded.toml" <<'EOF'
version = 1
semantic_schema_version = 2
[[change]]
protocol = "spl-stake-pool"
action   = "deposit_sol"
domain   = "economic"
subject  = "pool_tokens_received"
change   = "decreased"
max_delta_bps             = 25
max_affected_observations = 6
reason = "Approved deposit fee increase from 0.10% to 0.25%"
[[change]]
protocol = "spl-stake-pool"
action   = "deposit_sol"
domain   = "economic"
subject  = "pool_tokens_received"
change   = "increased"
max_delta_bps             = 25
max_affected_observations = 6
reason = "Referral path no longer splits the mint; depositor keeps the fee"
[[change]]
protocol = "spl-stake-pool"
action   = "withdraw_sol"
domain   = "execution"
subject  = "transaction"
change   = "now_reverts"
max_affected_observations = 4
reason = "WithdrawSol is intentionally removed in upgrade v3"
EOF
cat > "$WORK/expect/stale.toml" <<'EOF'
version = 1
semantic_schema_version = 2
[[change]]
protocol = "spl-stake-pool"
action   = "withdraw_sol"
domain   = "execution"
subject  = "transaction"
change   = "now_succeeds"
max_affected_observations = 4
reason = "We restored WithdrawSol in this release"
EOF
cat > "$WORK/expect/unevaluable.toml" <<'EOF'
version = 1
semantic_schema_version = 2
[[change]]
protocol = "spl-stake-pool"
action   = "deposit_sol"
domain   = "economic"
subject  = "sol_received_by_user"
change   = "decreased"
reason = "A deposit never pays SOL out, so nothing in this corpus can measure it"
EOF

echo "== starting the service with no RPC configured =="
env -u SOLANA_RPC_URL -u SOLANA_ARCHIVE_RPC_URL -u SOLANA_BLOCK_RPC_URL -u SOLANA_RPC_ORIGIN \
  "$SERVER" serve > "$WORK/server.log" 2>&1 &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null' EXIT
for _ in $(seq 1 60); do curl -sf "$URL/health" >/dev/null 2>&1 && break; sleep 0.25; done
check "health" "$(curl -s "$URL/health" | tr -d ' ')" '{"status":"ok"}'
check "ready" "$(curl -s "$URL/ready" | tr -d ' ')" '{"status":"ready"}'

post() { # candidate [expectations] -> body on stdout
  local args=(-s -H "Authorization: Bearer $TOKEN" -F "candidate=@$1")
  [ -n "${2:-}" ] && args+=(-F "expected_changes=@$2")
  curl "${args[@]}" "$URL/v1/projects/demo/checks"
}
code_of() { python3 -c "import json,sys;print(json.loads(sys.stdin.read()).get('exit_code','?'))"; }
run_of()  { python3 -c "import json,sys;print(json.loads(sys.stdin.read()).get('run_id','-'))"; }

echo "== gate outcomes =="
BODY=$(post "$BUNDLE/binaries/current.so");                         check "A baseline candidate"        "$(echo "$BODY" | code_of)" 0
BODY=$(post "$CANDIDATE_REGRESSED");                                check "B undeclared regression"     "$(echo "$BODY" | code_of)" 1
# C: every *named* finding is declared and inside its bounds, and the gate still
# fails - because this candidate also moves the manager fee, the reserve and the
# pool's own totals, none of which any expectation can name. Reporting that as a
# pass is exactly the false green an audit found in an earlier build.
BODY=$(post "$CANDIDATE_REGRESSED" "$WORK/expect/bounded.toml")
RUN_C=$(echo "$BODY" | run_of);                                     check "C named findings declared, rest undeclarable" "$(echo "$BODY" | code_of)" 1
BODY=$(post "$BUNDLE/binaries/current.so" "$WORK/expect/stale.toml");        check "D stale declaration" "$(echo "$BODY" | code_of)" 3
BODY=$(post "$BUNDLE/binaries/current.so" "$WORK/expect/unevaluable.toml");  check "E unevaluable"       "$(echo "$BODY" | code_of)" 5

echo "== hosted report == local report =="
curl -s -H "Authorization: Bearer $TOKEN" "$URL/v1/runs/$RUN_C/report.json" > "$WORK/hosted.json"
env -u SOLANA_RPC_URL "$CLI" ci check --bundle "$BUNDLE" --candidate "$CANDIDATE_REGRESSED" \
  --expectations "$WORK/expect/bounded.toml" --format json > "$WORK/local.json" 2>/dev/null
if cmp -s "$WORK/local.json" "$WORK/hosted.json"; then
  check "canonical report byte-identical" yes yes
else
  check "canonical report byte-identical" no yes
  diff "$WORK/local.json" "$WORK/hosted.json" | head -20
fi

echo "== authorization =="
st() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
check "no token"            "$(st -F "candidate=@$BUNDLE/binaries/current.so" "$URL/v1/projects/demo/checks")" 401
check "another project's token" "$(st -H "Authorization: Bearer $TOKEN2" -F "candidate=@$BUNDLE/binaries/current.so" "$URL/v1/projects/demo/checks")" 401
check "report needs the owner"  "$(st -H "Authorization: Bearer $TOKEN2" "$URL/v1/runs/$RUN_C/report.json")" 401
check "owner reads the report"  "$(st -H "Authorization: Bearer $TOKEN"  "$URL/v1/runs/$RUN_C/report.md")" 200

echo "== retention and secrecy =="
check "no candidate binaries retained" "$(find "$EPLYX_DATA_DIR/runs" -name '*.so' 2>/dev/null | wc -l | tr -d ' ')" 0
grep -rq "$TOKEN" "$EPLYX_DATA_DIR" 2>/dev/null && { check "raw token on disk" yes no; } || check "raw token absent from disk" yes yes
grep -q "$TOKEN" "$WORK/server.log" 2>/dev/null && { check "token in logs" yes no; } || check "raw token absent from logs" yes yes

echo
if [ "$fail" -eq 0 ]; then echo "hosted demo: all checks passed"; else echo "hosted demo: FAILURES above"; fi
exit "$fail"
