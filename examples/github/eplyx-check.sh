#!/usr/bin/env bash
# Upload a candidate to Eplyx, fetch the reports, and return Eplyx's gate code.
#
# Contains no analysis logic and makes no policy decisions. Every judgement -
# severity, review status, bounds, precedence - belongs to the engine; this
# script moves bytes and propagates a number.
#
# A transport failure is never reported as a gate result. "The API was
# unreachable" and "your upgrade changes user balances" are different facts, and
# collapsing them would teach a team to ignore the check.
set -uo pipefail

: "${EPLYX_TOKEN:?EPLYX_TOKEN is required}"
: "${EPLYX_PROJECT:?EPLYX_PROJECT is required}"
: "${EPLYX_URL:=https://api.eplyx.net}"

CANDIDATE="${1:?usage: eplyx-check.sh <candidate.so> [expected-changes.toml]}"
EXPECTATIONS="${2:-.eplyx/expected-changes.toml}"

[ -f "$CANDIDATE" ] || { echo "candidate not found: $CANDIDATE" >&2; exit 64; }

# Clear artifacts from any previous attempt first. Without this, a request that
# never reaches the server would leave the last run's response in place and be
# read back as a gate result - reporting a verdict for an analysis that did not
# happen. A re-run or a retried job is enough to hit it.
rm -f eplyx-response.json eplyx-report.json eplyx-report.md

args=(--silent --show-error --fail-with-body
      -H "Authorization: Bearer $EPLYX_TOKEN"
      -F "candidate=@$CANDIDATE")
[ -f "$EXPECTATIONS" ] && args+=(-F "expected_changes=@$EXPECTATIONS")

http_status=$(curl "${args[@]}" -o eplyx-response.json -w '%{http_code}' \
  "$EPLYX_URL/v1/projects/$EPLYX_PROJECT/checks")
transport=$?

# A preflight abort still carries a real Eplyx code in the error body (2 for a
# malformed configuration, 4 for a bundle incompatibility). Propagate it rather
# than reporting a generic transport failure.
exit_code=$(python3 -c '
import json, sys
try:
    print(json.load(open("eplyx-response.json")).get("exit_code", ""))
except Exception:
    print("")
' 2>/dev/null)

if [ "$transport" -ne 0 ] && [ -z "$exit_code" ]; then
  echo "Eplyx API request failed (curl $transport, HTTP $http_status)." >&2
  [ -s eplyx-response.json ] && cat eplyx-response.json >&2
  exit 70   # a transport fault, deliberately not an Eplyx gate code
fi

if [ -z "$exit_code" ]; then
  echo "Eplyx response did not contain an exit_code (HTTP $http_status)." >&2
  cat eplyx-response.json >&2
  exit 70
fi

run_id=$(python3 -c '
import json
print(json.load(open("eplyx-response.json")).get("run_id", ""))
' 2>/dev/null)

if [ -n "$run_id" ]; then
  curl --silent --show-error -H "Authorization: Bearer $EPLYX_TOKEN" \
    "$EPLYX_URL/v1/runs/$run_id/report.json" -o eplyx-report.json || true
  curl --silent --show-error -H "Authorization: Bearer $EPLYX_TOKEN" \
    "$EPLYX_URL/v1/runs/$run_id/report.md" -o eplyx-report.md || true
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ] && [ -s eplyx-report.md ]; then
    cat eplyx-report.md >> "$GITHUB_STEP_SUMMARY"
  fi
fi

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  echo "exit_code=$exit_code" >> "$GITHUB_OUTPUT"
  echo "run_id=$run_id" >> "$GITHUB_OUTPUT"
fi

echo "Eplyx gate exit code: $exit_code"
exit "$exit_code"
