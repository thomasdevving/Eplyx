#!/usr/bin/env bash
# End-to-end proof that a hosted check survives its own request.
#
# The integration suite drives the router in-process, which is where the
# lifecycle is actually pinned down. This drives a real server over a real
# socket, because the thing being claimed — that an accepted run outlives the
# connection that created it — is only fully true across a transport.
#
# Usage: scripts/async-demo.sh <bundle-dir> [work-dir]
set -uo pipefail

BUNDLE="${1:?usage: async-demo.sh <bundle-dir> [work-dir]}"
WORK="${2:-$(mktemp -d)}"
PORT="${EPLYX_DEMO_PORT:-8897}"
URL="http://127.0.0.1:$PORT"
SERVER=./target/release/eplyx-server
CANDIDATE="${EPLYX_DEMO_CANDIDATE:-artifacts/fixture_lending_v2.so}"

[ -x "$SERVER" ] || { echo "build first: cargo build --release -p eplyx-server"; exit 2; }
[ -r "$CANDIDATE" ] || { echo "no candidate at $CANDIDATE"; exit 2; }

export EPLYX_DATA_DIR="$WORK/data"
export EPLYX_BIND="127.0.0.1:$PORT"
mkdir -p "$EPLYX_DATA_DIR"

# The engine finishes this corpus in under a fifth of a second, so a run that
# is allowed to start is terminal before a second request can ask about it.
# Phase one therefore runs with no execution capacity at all: every claim about
# the queued state is then a fact rather than a race the script usually wins.
start_server() { # capacity logfile
  EPLYX_MAX_CONCURRENT_RUNS="$1" "$SERVER" serve >"$2" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 40); do
    curl -sf "$URL/health" >/dev/null 2>&1 && return 0
    sleep .25
  done
  echo "  FAIL server did not come up"; exit 1
}
submit() { # -> run id on stdout
  curl -s -o "$WORK/accepted.json" -X POST "$URL/v1/projects/$PROJECT/checks" \
    -H "Authorization: Bearer $TOKEN" -F "candidate=@$CANDIDATE" >/dev/null
  field run_id < "$WORK/accepted.json"
}
run_field() { # run-id field
  curl -s "$URL/v1/runs/$1" -H "Authorization: Bearer $TOKEN" | field "$2"
}

fail=0
check() { # label actual expected
  if [ "$2" = "$3" ]; then printf '  ok   %-52s %s\n' "$1" "$2"
  else printf '  FAIL %-52s %s (want %s)\n' "$1" "$2" "$3"; fail=1; fi
}
note() { printf '  %-5s %-52s %s\n' "$1" "$2" "${3:-}"; [ "$1" = "FAIL" ] && fail=1; return 0; }
field() { python3 -c "import json,sys; v=json.load(sys.stdin).get(sys.argv[1]); print('' if v is None else v)" "$1" 2>/dev/null; }

echo "== provisioning =="
PROG=$(python3 -c "import json;print(json.load(open('$BUNDLE/bundle.json'))['program_id'])")
PROJECT=$("$SERVER" admin create-project --name Demo --program-id "$PROG" 2>&1 | head -1 | awk '{print $2}')
OTHER=$("$SERVER" admin create-project --name Other --program-id "$PROG" 2>&1 | head -1 | awk '{print $2}')
TOKEN=$("$SERVER" admin create-token --project "$PROJECT" 2>&1 | grep -oE 'eplyx_proj_[0-9a-f]+')
TOKEN2=$("$SERVER" admin create-token --project "$OTHER" 2>&1 | grep -oE 'eplyx_proj_[0-9a-f]+')
BUNDLE_ID=$("$SERVER" admin register-bundle --project "$PROJECT" --path "$BUNDLE" 2>&1 | head -1 | awk '{print $3}')
"$SERVER" admin activate-bundle --project "$PROJECT" --bundle "$BUNDLE_ID" >/dev/null 2>&1 \
  && note ok "bundle activated" "$BUNDLE_ID" || note FAIL "bundle activated"

start_server 0 "$WORK/server.log"
trap 'kill $SERVER_PID 2>/dev/null' EXIT

echo
echo "== a check returns before it runs =="
START=$(python3 -c 'import time;print(time.time())')
RESPONSE=$(curl -s -o "$WORK/accepted.json" -w '%{http_code}' -X POST "$URL/v1/projects/$PROJECT/checks" \
  -H "Authorization: Bearer $TOKEN" -F "candidate=@$CANDIDATE")
ELAPSED=$(python3 -c "import time;print(f'{time.time()-$START:.2f}s')")
check "POST /checks" "$RESPONSE" "202"
GATED=$(field run_id < "$WORK/accepted.json")
check "status at creation" "$(field status < "$WORK/accepted.json")" "queued"
note info "accepted in" "$ELAPSED"
[ -n "$GATED" ] || { echo "  FAIL no run id"; exit 1; }

echo
echo "== with no capacity, nothing has started =="
# A different connection entirely: the POST is over.
check "the id already resolves" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/runs/$GATED" -H "Authorization: Bearer $TOKEN")" "200"
check "still queued" "$(run_field "$GATED" status)" "queued"
check "no exit code invented" "$(run_field "$GATED" exit_code)" ""
check "report refused" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/runs/$GATED/report.json" -H "Authorization: Bearer $TOKEN")" "409"
check "markdown refused" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/runs/$GATED/report.md" -H "Authorization: Bearer $TOKEN")" "409"
sleep 1
check "still queued a second later" "$(run_field "$GATED" status)" "queued"
check "never started" "$(run_field "$GATED" started_at_unix_seconds)" ""
GATED_SHA=$(run_field "$GATED" candidate_sha256)
check "candidate stored durably" "$([ -f "$EPLYX_DATA_DIR/artifacts/programs/$GATED_SHA" ] && echo yes || echo no)" "yes"
check "change spec stored" "$([ -f "$EPLYX_DATA_DIR/runs/$GATED/change_spec.json" ] && echo yes || echo no)" "yes"

echo
echo "== a restart resumes what it was holding =="
kill -9 $SERVER_PID 2>/dev/null; wait $SERVER_PID 2>/dev/null
start_server 1 "$WORK/server2.log"
grep -q "re-enqueued: $GATED" "$WORK/server2.log" && note ok "startup re-enqueued the queued run" || note FAIL "startup re-enqueued the queued run"
for _ in $(seq 1 100); do
  case "$(run_field "$GATED" status)" in passed|failed|execution_error) break ;; esac
  sleep .2
done
case "$(run_field "$GATED" status)" in passed|failed) note ok "the same run completed" "$(run_field "$GATED" status)" ;; *) note FAIL "the same run completed" "$(run_field "$GATED" status)" ;; esac
check "with a report" "$(run_field "$GATED" report_available)" "True"
check "its candidate is still held" "$([ -f "$EPLYX_DATA_DIR/artifacts/programs/$GATED_SHA" ] && echo yes || echo no)" "yes"
check "no scratch left behind" "$([ -e "$EPLYX_DATA_DIR/runs/$GATED/work" ] && echo yes || echo no)" "no"

echo
echo "== with capacity, a run completes on its own =="
RUN=$(submit)
[ -n "$RUN" ] || { echo "  FAIL no run id"; exit 1; }
SEEN=""
for _ in $(seq 1 160); do
  STATUS=$(run_field "$RUN" status)
  case "$SEEN" in *"$STATUS"*) ;; *) SEEN="$SEEN $STATUS" ;; esac
  case "$STATUS" in passed|failed|execution_error) break ;; esac
  sleep .2
done
note info "states observed" "$(echo "$SEEN" | xargs)"
case "$STATUS" in passed|failed) note ok "terminal state" "$STATUS" ;; *) note FAIL "terminal state" "$STATUS" ;; esac
check "report now available" "$(run_field "$RUN" report_available)" "True"
check "it passed through running" "$([ -n "$(run_field "$RUN" started_at_unix_seconds)" ] && echo yes || echo no)" "yes"
check "scratch cleared afterwards" "$([ -e "$EPLYX_DATA_DIR/runs/$RUN/work" ] && echo yes || echo no)" "no"

echo
echo "== the canonical report =="
check "GET report.json" "$(curl -s -o "$WORK/report.json" -w '%{http_code}' "$URL/v1/runs/$RUN/report.json" -H "Authorization: Bearer $TOKEN")" "200"
REPORT_EXIT=$(python3 -c "import json;print(json.load(open('$WORK/report.json'))['summary']['exit_code'])")
check "exit code agrees with the run" "$REPORT_EXIT" "$(run_field "$RUN" exit_code)"
check "bundle hash agrees" "$(python3 -c "import json;print(json.load(open('$WORK/report.json'))['bundle']['sha256'])")" "$(run_field "$RUN" bundle_sha256)"
curl -s -o "$WORK/report-again.json" "$URL/v1/runs/$RUN/report.json" -H "Authorization: Bearer $TOKEN"
if cmp -s "$WORK/report.json" "$WORK/report-again.json"; then note ok "refetch is byte-identical"; else note FAIL "refetch is byte-identical"; fi
check "GET report.md" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/runs/$RUN/report.md" -H "Authorization: Bearer $TOKEN")" "200"

echo
echo "== another project's token =="
check "run metadata" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/runs/$RUN" -H "Authorization: Bearer $TOKEN2")" "401"
check "report" "$(curl -s -o /dev/null -w '%{http_code}' "$URL/v1/runs/$RUN/report.json" -H "Authorization: Bearer $TOKEN2")" "401"

echo
echo "== capacity is shared, and nothing accepted is dropped =="
IDS=""
for _ in 1 2 3 4 5 6; do
  curl -s -o "$WORK/a.json" -X POST "$URL/v1/projects/$PROJECT/checks" \
    -H "Authorization: Bearer $TOKEN" -F "candidate=@$CANDIDATE" >/dev/null
  IDS="$IDS $(field run_id < "$WORK/a.json")"
done
QUEUED_SEEN=0
for _ in $(seq 1 60); do
  STATES=""
  for id in $IDS; do
    STATES="$STATES $(curl -s "$URL/v1/runs/$id" -H "Authorization: Bearer $TOKEN" | field status)"
  done
  case "$STATES" in *queued*) QUEUED_SEEN=1 ;; esac
  case "$STATES" in *queued*|*running*) sleep .2 ;; *) break ;; esac
done
[ "$QUEUED_SEEN" = "1" ] && note ok "a run waited for a slot" || note note "queueing not observed" "the engine finished faster than a poll"
for id in $IDS; do
  STATE=$(curl -s "$URL/v1/runs/$id" -H "Authorization: Bearer $TOKEN" | field status)
  case "$STATE" in passed|failed) ;; *) note FAIL "run $id" "$STATE" ;; esac
done
note ok "every accepted run reached a terminal state"

echo
[ $fail -eq 0 ] && echo "acceptance passed" || echo "acceptance FAILED"
exit $fail
