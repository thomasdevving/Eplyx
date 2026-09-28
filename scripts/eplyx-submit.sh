#!/usr/bin/env bash
# Submit a candidate to a hosted Eplyx project and exit with the gate's verdict.
#
# This is the client a CI job runs. It is deliberately the same one the
# production pilot used, so "it worked in the pilot" and "it works in CI" are a
# claim about one piece of code rather than two that resemble each other.
#
# Three kinds of outcome, kept apart on purpose:
#
#   0-5   Eplyx engine exit codes. Codes 2 and 4 can also be preflight aborts
#         with no analytical report; they are never candidate regressions.
#   75    the hosted run ended in `execution_error`: no verdict was obtained at
#         all. Never collapsed into 1, which would claim the candidate failed.
#   76    project readiness prevented submission; no candidate was uploaded.
#   70    this client could not trust its own result - the candidate the server
#         reports is not the one we uploaded, the change it analysed is not the
#         one we proposed, or the transport broke.
#
# The token is read from the environment and passed to curl through stdin,
# never argv or the summary. Project tokens contain only ASCII letters,
# digits and underscores, so they are safe in this curl config value.
#
# Usage:
#   EPLYX_TOKEN=<project token> scripts/eplyx-submit.sh \
#     --api <url> --project <id> --candidate <file> [--expectations <file>]
#     [--change-spec <file> | --label <text>]
#     [--report-json <out>] [--report-md <out>] [--summary <out>]
#     [--expect-bundle <sha256>] [--web-url <public origin>]
#
# Without --change-spec the server derives the minimal program upgrade of the
# project's program to the candidate's bytes, exactly as `eplyx ci check
# --candidate` does. With it, the spec is authoritative and the candidate only
# supplies the bytes it names; a spec that commits to a `change_spec_id` is
# then checked against the change the server analysed and the report names.
set -uo pipefail

API="" WEB_URL="" PROJECT="" CANDIDATE="" EXPECTATIONS="" REPORT_JSON="" REPORT_MD="" SUMMARY="" EXPECT_BUNDLE=""
CHANGE_SPEC="" LABEL=""
POLL_SECONDS="${EPLYX_POLL_SECONDS:-3}"
TIMEOUT_SECONDS="${EPLYX_TIMEOUT_SECONDS:-1800}"

while [ $# -gt 0 ]; do
  case "$1" in
    --api) API="$2"; shift 2;;
    --web-url) WEB_URL="$2"; shift 2;;
    --project) PROJECT="$2"; shift 2;;
    --candidate) CANDIDATE="$2"; shift 2;;
    --expectations) EXPECTATIONS="$2"; shift 2;;
    --report-json) REPORT_JSON="$2"; shift 2;;
    --report-md) REPORT_MD="$2"; shift 2;;
    --summary) SUMMARY="$2"; shift 2;;
    --expect-bundle) EXPECT_BUNDLE="$2"; shift 2;;
    --change-spec) CHANGE_SPEC="$2"; shift 2;;
    --label) LABEL="$2"; shift 2;;
    *) echo "unknown argument: $1" >&2; exit 70;;
  esac
done

[ -n "$API" ] || { echo "--api is required" >&2; exit 70; }
[ -n "$PROJECT" ] || { echo "--project is required" >&2; exit 70; }
[[ "$PROJECT" =~ ^[A-Za-z0-9_-]+$ ]] || { echo "--project is not a valid project ID" >&2; exit 70; }
[ -f "$CANDIDATE" ] || { echo "--candidate must be a file" >&2; exit 70; }
[ -n "${EPLYX_TOKEN:-}" ] || { echo "EPLYX_TOKEN is not set" >&2; exit 70; }
case "$EPLYX_TOKEN" in *[!A-Za-z0-9_]* ) echo "EPLYX_TOKEN has invalid characters" >&2; exit 70;; esac
[ -z "$CHANGE_SPEC" ] || [ -f "$CHANGE_SPEC" ] || { echo "--change-spec must be a file" >&2; exit 70; }
[ -z "$CHANGE_SPEC" ] || [ -z "$LABEL" ] || { echo "--label belongs inside --change-spec's metadata" >&2; exit 70; }
API="${API%/}"
WEB_URL="${WEB_URL%/}"
[ -n "$WEB_URL" ] || WEB_URL="$API"
case "$API" in http://*|https://*) ;; *) echo "--api must be an HTTP origin" >&2; exit 70;; esac
case "$WEB_URL" in http://*|https://*) ;; *) echo "--web-url must be an HTTP origin" >&2; exit 70;; esac
python3 - "$API" "$WEB_URL" <<'PYEOF' || { echo "API and web URLs must be origins without paths or credentials" >&2; exit 70; }
import sys
from urllib.parse import urlsplit
for value in sys.argv[1:]:
    parsed = urlsplit(value)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password or parsed.path or parsed.query or parsed.fragment:
        sys.exit(1)
PYEOF
[[ "$POLL_SECONDS" =~ ^[1-9][0-9]*$ && "$TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] || {
  echo "poll and timeout must be positive seconds" >&2; exit 70
}
if [ -n "$SUMMARY" ]; then : > "$SUMMARY" || exit 70; fi
curl_auth() {
  printf 'header = "Authorization: Bearer %s"\n' "$EPLYX_TOKEN" | curl -K - "$@"
}

# This view uses the same project credential as submission. The POST still
# validates authoritative state after upload; this check only saves work when
# the project is already known to be unready.
CAPABILITIES=$(curl_auth -fsS --connect-timeout 10 --max-time 30 \
  "$API/v1/projects/$PROJECT/capabilities") || {
  echo "capability preflight failed (authorization, service, or network); no candidate uploaded" >&2
  exit 70
}
PREFLIGHT=$(printf '%s' "$CAPABILITIES" | python3 -c '
import json,sys
try:
    body=json.load(sys.stdin)
    item=next(a for a in body["analyses"] if a["kind"]=="program_upgrade")
    assert body["project_id"] == sys.argv[1]
    if item["status"] == "ready" and item["can_submit"] is True:
        print("ready")
    else:
        print("; ".join("{}: {}".format(m["code"], m["action"]) for m in item["missing"]) or item["status"])
except (ValueError, KeyError, StopIteration, AssertionError, TypeError):
    sys.exit(1)
' "$PROJECT") || { echo "unreadable capability response" >&2; exit 70; }
if [ "$PREFLIGHT" != ready ]; then
  echo "program_upgrade is not ready: $PREFLIGHT; no candidate uploaded" >&2
  if [ -n "$SUMMARY" ]; then printf '## Eplyx — project not ready\n\n%s\n\nNo candidate was uploaded.\n' "$PREFLIGHT" >"$SUMMARY"; fi
  exit 76
fi

# Hash before submission, and never rebuild between here and the upload. This
# is the value the server's answer is checked against: without it, "the gate
# passed" says nothing about which bytes it passed on.
LOCAL_SHA=$(shasum -a 256 "$CANDIDATE" 2>/dev/null | cut -d' ' -f1)
[ -n "$LOCAL_SHA" ] || LOCAL_SHA=$(sha256sum "$CANDIDATE" | cut -d' ' -f1)
echo "candidate $(basename "$CANDIDATE")  sha256 $LOCAL_SHA"

FORM=(-F "candidate=@$CANDIDATE")
[ -n "$EXPECTATIONS" ] && FORM+=(-F "expected_changes=@$EXPECTATIONS")
[ -n "$CHANGE_SPEC" ] && FORM+=(-F "change_spec=@$CHANGE_SPEC")
[ -n "$LABEL" ] && FORM+=(-F "label=$LABEL")
# The id a submitted spec commits to, if it commits to one. The server
# recomputes it and refuses a spec whose stated id disagrees with its fields,
# so a match here means the analysis is of exactly the proposal in that file.
PROPOSED_ID=""
if [ -n "$CHANGE_SPEC" ]; then
  PROPOSED_ID=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("change_spec_id") or "")' "$CHANGE_SPEC") || exit 70
fi

CREATED=$(curl_auth -sS --connect-timeout 10 --max-time 180 -X POST "$API/v1/projects/$PROJECT/checks" \
  "${FORM[@]}" -w '\n%{http_code}') || {
  echo "submission transport failed; the server may still have accepted a run" >&2; exit 70
}
HTTP=$(printf '%s' "$CREATED" | tail -1)
BODY=$(printf '%s' "$CREATED" | sed '$d')
if [ "$HTTP" != "202" ]; then
  echo "submission refused: HTTP $HTTP $BODY" >&2
  exit 70
fi
RUN=$(printf '%s' "$BODY" | python3 -c 'import json,sys; print(json.load(sys.stdin)["run_id"])') || exit 70
CHANGE_ID=$(printf '%s' "$BODY" | python3 -c 'import json,sys; print((json.load(sys.stdin).get("change") or {}).get("change_spec_id",""))') || exit 70
ACCEPTED_SHA=$(printf '%s' "$BODY" | python3 -c 'import json,sys; print(json.load(sys.stdin)["candidate_sha256"])') || exit 70
ACCEPTED_BUNDLE=$(printf '%s' "$BODY" | python3 -c 'import json,sys; print(json.load(sys.stdin)["bundle_sha256"])') || exit 70
if [ "$ACCEPTED_SHA" != "$LOCAL_SHA" ]; then
  echo "candidate identity mismatch: local $LOCAL_SHA, server accepted $ACCEPTED_SHA" >&2
  exit 70
fi
[[ "$RUN" =~ ^run_[A-Za-z0-9_-]+$ ]] || { echo "invalid accepted run ID" >&2; exit 70; }
RUN_URL="$WEB_URL/p/$PROJECT/runs/$RUN"
echo "run $RUN accepted (HTTP 202)  change ${CHANGE_ID:-<none>}"
echo "run URL: $RUN_URL (sign-in required)"
if [ -n "$PROPOSED_ID" ] && [ "$CHANGE_ID" != "$PROPOSED_ID" ]; then
  echo "change identity mismatch: proposed $PROPOSED_ID, server accepted $CHANGE_ID" >&2
  exit 70
fi

# Poll. A closed connection does not cancel a run, so a client that dies here
# loses its own result and nothing else.
STARTED=$(date +%s)
STATUS=""
while :; do
  RESPONSE=$(curl_auth -fsS --connect-timeout 10 --max-time 30 "$API/v1/runs/$RUN") || {
    echo "run $RUN polling failed; result unknown. Open $RUN_URL" >&2; exit 70
  }
  STATUS=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("status",""))' 2>/dev/null)
  case "$STATUS" in
    passed|failed|execution_error) break;;
    queued|running) ;;
    "") echo "unreadable run status" >&2; exit 70;;
    *) echo "unexpected run status: $STATUS" >&2; exit 70;;
  esac
  NOW=$(date +%s)
  if [ $((NOW - STARTED)) -ge "$TIMEOUT_SECONDS" ]; then
    echo "run $RUN still $STATUS after ${TIMEOUT_SECONDS}s" >&2
    exit 70
  fi
  sleep "$POLL_SECONDS"
done
ELAPSED=$(( $(date +%s) - STARTED ))

RUN_ID=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("run_id",""))') || exit 70
RUN_PROJECT=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("project_id",""))') || exit 70
if [ "$RUN_ID" != "$RUN" ] || [ "$RUN_PROJECT" != "$PROJECT" ]; then
  echo "run identity mismatch" >&2; exit 70
fi

SERVER_SHA=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("candidate_sha256",""))')
if [ "$SERVER_SHA" != "$LOCAL_SHA" ]; then
  echo "candidate identity mismatch: uploaded $LOCAL_SHA, server reports $SERVER_SHA" >&2
  exit 70
fi
RUN_BUNDLE=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("bundle_sha256",""))')
if [ "$RUN_BUNDLE" != "$ACCEPTED_BUNDLE" ]; then
  echo "bundle identity mismatch: accepted $ACCEPTED_BUNDLE, run used $RUN_BUNDLE" >&2
  exit 70
fi

# Which evidence the verdict is about. A green run against the wrong bundle -
# a demo corpus, a stale one, a different project's - reads exactly like a
# green run against the right one, so acceptance names the bundle it means.
if [ -n "$EXPECT_BUNDLE" ]; then
  if [ "$RUN_BUNDLE" != "$EXPECT_BUNDLE" ]; then
    echo "bundle mismatch: expected $EXPECT_BUNDLE, run used $RUN_BUNDLE" >&2
    exit 70
  fi
fi

RUN_CHANGE=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print((json.load(sys.stdin).get("change") or {}).get("change_spec_id",""))')
if [ "$RUN_CHANGE" != "$CHANGE_ID" ]; then
  echo "change identity mismatch: accepted $CHANGE_ID, run reports $RUN_CHANGE" >&2
  exit 70
fi

# `execution_error` is not a verdict. Its run identity is still checked before
# linking to the record or describing the outcome.
if [ "$STATUS" = "execution_error" ]; then
  DETAIL=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("detail") or "")')
  echo "run $RUN ended in execution_error: $DETAIL; open $RUN_URL" >&2
  if [ -n "$SUMMARY" ]; then printf '## Eplyx — no analytical verdict\n\nRun: [%s](%s)  \nStatus: execution_error  \nDetail: %s\n' "$RUN" "$RUN_URL" "$DETAIL" >"$SUMMARY"; fi
  exit 75
fi

EXIT=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("exit_code",70))')
case "$EXIT" in 0|1|2|3|4|5) ;; *) echo "invalid terminal exit code: $EXIT" >&2; exit 70;; esac
if { [ "$STATUS" = passed ] && [ "$EXIT" != 0 ]; } || { [ "$STATUS" = failed ] && [ "$EXIT" = 0 ]; }; then
  echo "run status and exit code disagree" >&2; exit 70
fi
REPORT_AVAILABLE=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(str(json.load(sys.stdin).get("report_available",False)).lower())')
case "$REPORT_AVAILABLE" in true|false) ;; *) echo "invalid report availability" >&2; exit 70;; esac
if [ "$REPORT_AVAILABLE" = false ]; then
  case "$EXIT" in 0|1|3|5) echo "run has a verdict code but no authoritative report" >&2; exit 70;; esac
fi
if [ -n "$REPORT_JSON" ]; then
  if [ "$REPORT_AVAILABLE" = true ]; then
    curl_auth -fsS --connect-timeout 10 --max-time 60 "$API/v1/runs/$RUN/report.json" -o "$REPORT_JSON" || exit 70
  fi
  # A report exists only for a run that reached one. When it does, it must be
  # about the change this job proposed.
  if [ "$REPORT_AVAILABLE" = true ]; then
    python3 -c 'import json,sys; json.load(open(sys.argv[1]))["summary"]' "$REPORT_JSON" || { echo "invalid authoritative report" >&2; exit 70; }
    REPORT_CHANGE=$(python3 -c 'import json,sys; print((json.load(open(sys.argv[1])).get("change") or {}).get("change_spec_id",""))' "$REPORT_JSON")
    if [ "$REPORT_CHANGE" != "$CHANGE_ID" ]; then
      echo "change identity mismatch: accepted $CHANGE_ID, report names $REPORT_CHANGE" >&2
      exit 70
    fi
    REPORT_SHA=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("candidate",{}).get("sha256",""))' "$REPORT_JSON")
    if [ "$REPORT_SHA" != "$LOCAL_SHA" ]; then
      echo "candidate identity mismatch: local $LOCAL_SHA, report names $REPORT_SHA" >&2
      exit 70
    fi
    REPORT_EXIT=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["summary"]["exit_code"])' "$REPORT_JSON") || exit 70
    if [ "$REPORT_EXIT" != "$EXIT" ]; then
      echo "report and run exit codes disagree" >&2
      exit 70
    fi
  fi
fi
if [ -n "$REPORT_MD" ] && [ "$REPORT_AVAILABLE" = true ]; then
  curl_auth -fsS --connect-timeout 10 --max-time 60 "$API/v1/runs/$RUN/report.md" -o "$REPORT_MD" || exit 70
fi

echo "run $RUN  status $STATUS  exit $EXIT  ${ELAPSED}s"

if [ -n "$SUMMARY" ]; then
  python3 - "$RESPONSE" "$RUN" "$ELAPSED" "${REPORT_JSON:-}" "$RUN_URL" >"$SUMMARY" <<'PYEOF'
import json, sys
run = json.loads(sys.argv[1]); run_id, elapsed, report_path, run_url = sys.argv[2:]
exit_code = run.get("exit_code")
# Wording is load-bearing. A pass means no disallowed difference was observed
# in the coverage this bundle represents - not that the candidate is safe.
reasons = []
if report_path and run.get("report_available"):
    try:
        reasons = json.load(open(report_path)).get("summary", {}).get("failure_reasons") or []
    except Exception:
        reasons = []
# No semantic coverage is Eplyx saying it did not look. It is neither a pass
# nor an adverse finding, and is never headlined as one.
headline = ("PASS within the active bundle's coverage" if exit_code == 0 else
            "Analytical regression" if exit_code == 1 else
            "Stale expectation" if exit_code == 3 else
            "Evidence or coverage limitation" if exit_code in (4, 5) or "no_semantic_coverage" in reasons else
            "Analysis could not complete")
out = [f"## Eplyx — {headline}", ""]
out.append(f"[Open authoritative run]({run_url}) (sign-in required)")
out.append("")
out += ["| | |", "|---|---|"]
change = run.get("change") or {}
if change:
    out.append(f"| Change | `{change.get('change_spec_id')}` |")
    out.append(f"| Target | `{change.get('target_program_id')}` |")
for label, key in [("Run", "run_id"), ("Candidate", "candidate_sha256"), ("Bundle", "bundle_sha256"),
                   ("Baseline", "baseline_sha256"), ("Corpus", "corpus_sha256"),
                   ("Adapter", "adapter"), ("Records", "record_count"), ("Exit code", "exit_code")]:
    out.append(f"| {label} | `{run.get(key)}` |")
out.append(f"| Run status | `{run.get('status')}` |")
out.append(f"| Duration | {elapsed}s |")
if report_path and run.get("report_available"):
    try:
        report = json.load(open(report_path))
        summary = report.get("summary", {})
        out.append(f"| Findings | {len(report.get('findings') or [])} |")
        # Only the five review outcomes belong under a "count" heading.
        # `passed` is a verdict and `exit_code` is already in the table above;
        # listing them here rendered "passed | False" as though it were a count.
        out += ["", "### Review", "", "| outcome | count |", "|---|---:|"]
        for key in ("expected", "unexpected", "expected_but_exceeded", "stale", "unevaluable"):
            if isinstance(summary.get(key), int):
                out.append(f"| {key.replace('_', ' ')} | {summary[key]} |")
        reasons = summary.get("failure_reasons") or []
        if reasons:
            out += ["", "Gate failed on: " + ", ".join(f"`{r}`" for r in reasons)]
        limits = report.get("bundle", {}).get("limitations") or []
        if limits:
            out += ["", "### Known limitations of this corpus", ""]
            for limit in limits:
                out.append(f"- **{limit.get('code')}** — {limit.get('detail')}")
        out += ["", "A pass means no disallowed difference was observed in the replay coverage "
                    "this bundle represents. It is not a statement that the candidate is safe, "
                    "nor that the corpus is representative of production traffic."]
    except Exception as error:
        out.append(f"\n_report detail unavailable: {error}_")
elif not run.get("report_available"):
    out += ["", "No analytical report was produced. " + str(run.get("detail") or "Check the run for the failure reason.")]
print("\n".join(out))
PYEOF
fi
exit "$EXIT"
