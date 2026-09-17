#!/usr/bin/env bash
# The onboarding path, end to end against a real server over a real socket.
#
# A protocol team arrives with a Solana program and leaves with Eplyx running
# checks for it: project, bundle, activation, token, check, history. Everything
# below goes through the HTTP API a browser uses — the admin CLI only creates
# the operator's first credential, because that one cannot be issued by the
# thing it authenticates.
#
# Usage: scripts/product-demo.sh <bundle-dir> <second-bundle-dir> [work-dir]
set -uo pipefail

BUNDLE="${1:?usage: product-demo.sh <bundle-dir> <second-bundle-dir> [work-dir]}"
BUNDLE_B="${2:?a second bundle is needed to show rotation}"
WORK="${3:-$(mktemp -d)}"
PORT="${EPLYX_DEMO_PORT:-8896}"
URL="http://127.0.0.1:$PORT"
SERVER=./target/release/eplyx-server
CANDIDATE="${EPLYX_DEMO_CANDIDATE:-artifacts/fixture_lending_v2.so}"
OPERATOR="operator-$(date +%s)-secret"

[ -x "$SERVER" ] || { echo "build first: cargo build --release -p eplyx-server"; exit 2; }
[ -r "$CANDIDATE" ] || { echo "no candidate at $CANDIDATE"; exit 2; }

export EPLYX_DATA_DIR="$WORK/data"
export EPLYX_BIND="127.0.0.1:$PORT"
export EPLYX_OPERATOR_TOKEN="$OPERATOR"
mkdir -p "$EPLYX_DATA_DIR"

fail=0
check() { # label actual expected
  if [ "$2" = "$3" ]; then printf '  ok   %-50s %s\n' "$1" "$2"
  else printf '  FAIL %-50s %s (want %s)\n' "$1" "$2" "$3"; fail=1; fi
}
note() { printf '  %-5s %-50s %s\n' "$1" "$2" "${3:-}"; [ "$1" = "FAIL" ] && fail=1; return 0; }
field() { python3 -c "import json,sys; v=json.load(sys.stdin); [v:=v.get(k) if isinstance(v,dict) else None for k in sys.argv[1].split('.')]; print('' if v is None else v)" "$1" 2>/dev/null; }
op() { curl -s -H "Authorization: Bearer $OPERATOR" "$@"; }
opcode() { curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $OPERATOR" "$@"; }
# One multipart part per bundle file, named by its path inside the bundle.
bundle_parts() { find "$1" -type f | while read -r f; do printf -- '-F\n%s=@%s\n' "${f#"$1"/}" "$f"; done; }
upload_bundle() { # project dir
  local args=(); while IFS= read -r line; do args+=("$line"); done < <(bundle_parts "$2")
  op -X POST "$URL/v1/projects/$1/bundles" "${args[@]}"
}
start_server() {
  "$SERVER" serve >>"$WORK/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 40); do curl -sf "$URL/health" >/dev/null 2>&1 && return 0; sleep .25; done
  echo "  FAIL server did not start"; exit 1
}

start_server
trap 'kill $SERVER_PID 2>/dev/null' EXIT

echo "== 1. a team arrives with a program =="
PROGRAM=$(python3 -c "import json;print(json.load(open('$BUNDLE/bundle.json'))['program_id'])")
ADAPTER=$(op "$URL/v1/adapters" | python3 -c "
import json,sys
adapters = json.load(sys.stdin)['adapters']
match = next((a for a in adapters if a.get('program_id') == '$PROGRAM'), None)
print((match or next(a for a in adapters if not a['speaks_semantics']))['adapter_id'])
")
note info "adapter this build speaks" "$ADAPTER"
CREATED=$(op -X POST "$URL/v1/projects" -H 'Content-Type: application/json' \
  -d "{\"name\":\"Example Lending\",\"program_id\":\"$PROGRAM\",\"adapter_id\":\"$ADAPTER\"}")
PROJECT=$(echo "$CREATED" | field project_id)
check "project created" "$(echo "$CREATED" | field status)" "setup"
case "$PROJECT" in proj_*) note ok "opaque project id" "$PROJECT" ;; *) note FAIL "opaque project id" "$PROJECT" ;; esac
check "a name is not an identifier" "$(echo "$PROJECT" | grep -c Example)" "0"

echo
echo "== 2. a project in setup cannot be checked =="
check "check refused" "$(opcode -X POST "$URL/v1/projects/$PROJECT/checks" -F "candidate=@$CANDIDATE")" "409"

echo
echo "== 3. a bundle is uploaded, verified, and left inactive =="
REGISTERED=$(upload_bundle "$PROJECT" "$BUNDLE")
BUNDLE_A=$(echo "$REGISTERED" | field bundle_id)
case "$BUNDLE_A" in bndl_*) note ok "bundle registered" "$BUNDLE_A" ;; *) note FAIL "bundle registered" "$REGISTERED" ;; esac
check "verification reported its own hash" "$(echo "$REGISTERED" | field bundle_sha256 | wc -c | tr -d ' ')" "65"
check "and its baseline" "$(echo "$REGISTERED" | field baseline_sha256 | wc -c | tr -d ' ')" "65"
check "not active yet" "$(echo "$REGISTERED" | field active)" "False"
check "project still in setup" "$(op "$URL/v1/projects/$PROJECT" | field project.status)" "setup"

echo
echo "== 4. activation is a separate, deliberate step =="
ACTIVATED=$(op -X POST "$URL/v1/projects/$PROJECT/bundles/$BUNDLE_A/activate")
check "project is ready" "$(echo "$ACTIVATED" | field status)" "ready"
check "pointing at the bundle" "$(echo "$ACTIVATED" | field active_bundle.bundle_id)" "$BUNDLE_A"

echo
echo "== 5. a project token is issued once =="
ISSUED=$(op -X POST "$URL/v1/projects/$PROJECT/tokens" -H 'Content-Type: application/json' -d '{"label":"GitHub Actions"}')
TOKEN=$(echo "$ISSUED" | field token)
TOKEN_ID=$(echo "$ISSUED" | field token_id)
case "$TOKEN" in eplyx_proj_*) note ok "token issued" "${TOKEN:0:18}…" ;; *) note FAIL "token issued" ;; esac
LISTED=$(op "$URL/v1/projects/$PROJECT/tokens")
check "the secret is never listed again" "$(echo "$LISTED" | grep -c "$TOKEN")" "0"
check "but the token is" "$(echo "$LISTED" | grep -c "$TOKEN_ID")" "1"
check "nor is it on the volume" "$(grep -rl "$TOKEN" "$EPLYX_DATA_DIR" 2>/dev/null | wc -l | tr -d ' ')" "0"

echo
echo "== 6. the team's own token runs a check =="
ACCEPTED=$(curl -s -X POST "$URL/v1/projects/$PROJECT/checks" -H "Authorization: Bearer $TOKEN" -F "candidate=@$CANDIDATE")
RUN_A=$(echo "$ACCEPTED" | field run_id)
check "accepted" "$(echo "$ACCEPTED" | field status)" "queued"
check "pinned to the active bundle" "$(curl -s "$URL/v1/runs/$RUN_A" -H "Authorization: Bearer $TOKEN" | field bundle_id)" "$BUNDLE_A"
for _ in $(seq 1 120); do
  STATUS=$(curl -s "$URL/v1/runs/$RUN_A" -H "Authorization: Bearer $TOKEN" | field status)
  case "$STATUS" in passed|failed|execution_error) break ;; esac
  sleep .2
done
case "$STATUS" in passed|failed) note ok "run completed" "$STATUS" ;; *) note FAIL "run completed" "$STATUS" ;; esac
check "report served" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/runs/$RUN_A/report.json" -H "Authorization: Bearer $TOKEN")" "200"

echo
echo "== 7. it appears in the project's history =="
HISTORY=$(op "$URL/v1/projects/$PROJECT/runs")
check "one run listed" "$(echo "$HISTORY" | python3 -c "import json,sys;print(len(json.load(sys.stdin)['runs']))")" "1"
check "and it is that run" "$(echo "$HISTORY" | python3 -c "import json,sys;print(json.load(sys.stdin)['runs'][0]['run_id'])")" "$RUN_A"
check "no report body in the list" "$(echo "$HISTORY" | grep -c '"findings"')" "0"

echo
echo "== 8. everything survives a restart =="
kill -9 $SERVER_PID 2>/dev/null; wait $SERVER_PID 2>/dev/null
start_server
check "project still there" "$(op "$URL/v1/projects/$PROJECT" | field project.status)" "ready"
check "active bundle unchanged" "$(op "$URL/v1/projects/$PROJECT" | field project.active_bundle.bundle_id)" "$BUNDLE_A"
check "the completed run is readable" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/runs/$RUN_A/report.json" -H "Authorization: Bearer $TOKEN")" "200"
check "the issued token still authenticates" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/projects/$PROJECT" -H "Authorization: Bearer $TOKEN")" "200"

echo
echo "== 9. rotating the baseline keeps history honest =="
BUNDLE_B_ID=$(upload_bundle "$PROJECT" "$BUNDLE_B" | field bundle_id)
check "a second bundle" "$(echo "$BUNDLE_B_ID" | cut -c1-5)" "bndl_"
op -X POST "$URL/v1/projects/$PROJECT/bundles/$BUNDLE_B_ID/activate" >/dev/null
BUNDLES=$(op "$URL/v1/projects/$PROJECT/bundles")
check "both bundles kept" "$(echo "$BUNDLES" | python3 -c "import json,sys;print(len(json.load(sys.stdin)['bundles']))")" "2"
check "exactly one is active" "$(echo "$BUNDLES" | python3 -c "import json,sys;print(sum(1 for b in json.load(sys.stdin)['bundles'] if b['active']))")" "1"
check "the earlier run still names bundle A" "$(curl -s "$URL/v1/runs/$RUN_A" -H "Authorization: Bearer $TOKEN" | field bundle_id)" "$BUNDLE_A"
RUN_B=$(curl -s -X POST "$URL/v1/projects/$PROJECT/checks" -H "Authorization: Bearer $TOKEN" -F "candidate=@$CANDIDATE" | field run_id)
check "the next run names bundle B" "$(curl -s "$URL/v1/runs/$RUN_B" -H "Authorization: Bearer $TOKEN" | field bundle_id)" "$BUNDLE_B_ID"

echo
echo "== 10. isolation holds =="
OTHER=$(op -X POST "$URL/v1/projects" -H 'Content-Type: application/json' \
  -d "{\"name\":\"Other\",\"program_id\":\"$PROGRAM\",\"adapter_id\":\"$ADAPTER\"}" | field project_id)
OTHER_TOKEN=$(op -X POST "$URL/v1/projects/$OTHER/tokens" -H 'Content-Type: application/json' -d '{"label":"x"}' | field token)
check "another project's run" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/runs/$RUN_A" -H "Authorization: Bearer $OTHER_TOKEN")" "401"
check "another project's metadata" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/projects/$PROJECT" -H "Authorization: Bearer $OTHER_TOKEN")" "401"
check "a CI token cannot list projects" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/projects" -H "Authorization: Bearer $TOKEN")" "401"
check "a CI token cannot issue tokens" "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$URL/v1/projects/$PROJECT/tokens" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"label":"x"}')" "404"
check "and nothing is public" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/projects")" "401"

echo
echo "== 11. a revoked token stops working =="
op -X DELETE "$URL/v1/projects/$PROJECT/tokens/$TOKEN_ID" >/dev/null
check "revoked" "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$URL/v1/projects/$PROJECT/checks" -H "Authorization: Bearer $TOKEN" -F "candidate=@$CANDIDATE")" "401"

echo
[ $fail -eq 0 ] && echo "product acceptance passed" || echo "product acceptance FAILED"
exit $fail
