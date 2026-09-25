#!/usr/bin/env bash
# Submit a candidate to a hosted Eplyx project and exit with the gate's verdict.
#
# This is the client a CI job runs. It is deliberately the same one the
# production pilot used, so "it worked in the pilot" and "it works in CI" are a
# claim about one piece of code rather than two that resemble each other.
#
# Three kinds of outcome, kept apart on purpose:
#
#   0-5   the engine reached a verdict. These are Eplyx's own exit codes and
#         they mean exactly what `eplyx ci check` means by them.
#   75    the hosted run ended in `execution_error`: no verdict was obtained at
#         all. Never collapsed into 1, which would claim the candidate failed.
#   70    this client could not trust its own result - the candidate the server
#         reports is not the one we uploaded, the change it analysed is not the
#         one we proposed, or the transport broke.
#
# The token is read from the environment and never printed, never passed as an
# argument (argv is world-readable on most systems), and never written to the
# summary.
#
# Usage:
#   EPLYX_TOKEN=<project token> scripts/eplyx-submit.sh \
#     --api <url> --project <id> --candidate <file> [--expectations <file>]
#     [--change-spec <file> | --label <text>]
#     [--report-json <out>] [--report-md <out>] [--summary <out>]
#     [--expect-bundle <sha256>]
#
# Without --change-spec the server derives the minimal program upgrade of the
# project's program to the candidate's bytes, exactly as `eplyx ci check
# --candidate` does. With it, the spec is authoritative and the candidate only
# supplies the bytes it names; a spec that commits to a `change_spec_id` is
# then checked against the change the server analysed and the report names.
set -uo pipefail

API="" PROJECT="" CANDIDATE="" EXPECTATIONS="" REPORT_JSON="" REPORT_MD="" SUMMARY="" EXPECT_BUNDLE=""
CHANGE_SPEC="" LABEL=""
POLL_SECONDS="${EPLYX_POLL_SECONDS:-3}"
TIMEOUT_SECONDS="${EPLYX_TIMEOUT_SECONDS:-1800}"

while [ $# -gt 0 ]; do
  case "$1" in
    --api) API="$2"; shift 2;;
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
[ -f "$CANDIDATE" ] || { echo "--candidate must be a file" >&2; exit 70; }
[ -n "${EPLYX_TOKEN:-}" ] || { echo "EPLYX_TOKEN is not set" >&2; exit 70; }
[ -z "$CHANGE_SPEC" ] || [ -f "$CHANGE_SPEC" ] || { echo "--change-spec must be a file" >&2; exit 70; }
[ -z "$CHANGE_SPEC" ] || [ -z "$LABEL" ] || { echo "--label belongs inside --change-spec's metadata" >&2; exit 70; }
API="${API%/}"

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

CREATED=$(curl -sS -X POST "$API/v1/projects/$PROJECT/checks" \
  -H "Authorization: Bearer $EPLYX_TOKEN" "${FORM[@]}" -w '\n%{http_code}')
HTTP=$(printf '%s' "$CREATED" | tail -1)
BODY=$(printf '%s' "$CREATED" | sed '$d')
if [ "$HTTP" != "202" ]; then
  echo "submission refused: HTTP $HTTP $BODY" >&2
  exit 70
fi
RUN=$(printf '%s' "$BODY" | python3 -c 'import json,sys; print(json.load(sys.stdin)["run_id"])') || exit 70
CHANGE_ID=$(printf '%s' "$BODY" | python3 -c 'import json,sys; print((json.load(sys.stdin).get("change") or {}).get("change_spec_id",""))') || exit 70
echo "run $RUN accepted (HTTP 202)  change ${CHANGE_ID:-<none>}"
if [ -n "$PROPOSED_ID" ] && [ "$CHANGE_ID" != "$PROPOSED_ID" ]; then
  echo "change identity mismatch: proposed $PROPOSED_ID, server accepted $CHANGE_ID" >&2
  exit 70
fi

# Poll. A closed connection does not cancel a run, so a client that dies here
# loses its own result and nothing else.
STARTED=$(date +%s)
STATUS=""
while :; do
  RESPONSE=$(curl -sS "$API/v1/runs/$RUN" -H "Authorization: Bearer $EPLYX_TOKEN")
  STATUS=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("status",""))' 2>/dev/null)
  case "$STATUS" in
    passed|failed|execution_error) break;;
    "") echo "unreadable run status" >&2; exit 70;;
  esac
  NOW=$(date +%s)
  if [ $((NOW - STARTED)) -ge "$TIMEOUT_SECONDS" ]; then
    echo "run $RUN still $STATUS after ${TIMEOUT_SECONDS}s" >&2
    exit 70
  fi
  sleep "$POLL_SECONDS"
done
ELAPSED=$(( $(date +%s) - STARTED ))

# `execution_error` is not a verdict. Reporting it as exit 1 would tell a team
# their candidate failed when nothing was ever measured about it.
if [ "$STATUS" = "execution_error" ]; then
  DETAIL=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("detail") or "")')
  echo "run $RUN ended in execution_error: $DETAIL" >&2
  exit 75
fi

SERVER_SHA=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("candidate_sha256",""))')
if [ "$SERVER_SHA" != "$LOCAL_SHA" ]; then
  echo "candidate identity mismatch: uploaded $LOCAL_SHA, server reports $SERVER_SHA" >&2
  exit 70
fi

# Which evidence the verdict is about. A green run against the wrong bundle -
# a demo corpus, a stale one, a different project's - reads exactly like a
# green run against the right one, so acceptance names the bundle it means.
if [ -n "$EXPECT_BUNDLE" ]; then
  RUN_BUNDLE=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("bundle_sha256",""))')
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

EXIT=$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("exit_code",70))')
if [ -n "$REPORT_JSON" ]; then
  curl -sS "$API/v1/runs/$RUN/report.json" -H "Authorization: Bearer $EPLYX_TOKEN" -o "$REPORT_JSON"
  # A report exists only for a run that reached one. When it does, it must be
  # about the change this job proposed.
  if python3 -c 'import json,sys; json.load(open(sys.argv[1]))["summary"]' "$REPORT_JSON" 2>/dev/null; then
    REPORT_CHANGE=$(python3 -c 'import json,sys; print((json.load(open(sys.argv[1])).get("change") or {}).get("change_spec_id",""))' "$REPORT_JSON")
    if [ "$REPORT_CHANGE" != "$CHANGE_ID" ]; then
      echo "change identity mismatch: accepted $CHANGE_ID, report names $REPORT_CHANGE" >&2
      exit 70
    fi
  fi
fi
[ -n "$REPORT_MD" ] && curl -sS "$API/v1/runs/$RUN/report.md" -H "Authorization: Bearer $EPLYX_TOKEN" -o "$REPORT_MD"

echo "run $RUN  status $STATUS  exit $EXIT  ${ELAPSED}s"

if [ -n "$SUMMARY" ]; then
  python3 - "$RESPONSE" "$RUN" "$ELAPSED" "${REPORT_JSON:-}" >"$SUMMARY" <<'PYEOF'
import json, sys
run = json.loads(sys.argv[1]); run_id, elapsed, report_path = sys.argv[2], sys.argv[3], sys.argv[4]
exit_code = run.get("exit_code")
# Wording is load-bearing. A pass means no disallowed difference was observed
# in the coverage this bundle represents - not that the candidate is safe.
reasons = []
if report_path:
    try:
        reasons = json.load(open(report_path)).get("summary", {}).get("failure_reasons") or []
    except Exception:
        reasons = []
# No semantic coverage is Eplyx saying it did not look. It is neither a pass
# nor an adverse finding, and is never headlined as one.
headline = ("Eplyx check passed against the project's active production-derived replay bundle."
            if exit_code == 0 else
            "Economic impact could not be evaluated for this interaction (no_semantic_coverage)."
            if "no_semantic_coverage" in reasons else
            "Unexpected or over-bound semantic changes detected.")
out = [f"## Eplyx — {headline}", ""]
out += ["| | |", "|---|---|"]
change = run.get("change") or {}
if change:
    out.append(f"| Change | `{change.get('change_spec_id')}` |")
    out.append(f"| Target | `{change.get('target_program_id')}` |")
for label, key in [("Run", "run_id"), ("Candidate", "candidate_sha256"), ("Bundle", "bundle_sha256"),
                   ("Baseline", "baseline_sha256"), ("Corpus", "corpus_sha256"),
                   ("Adapter", "adapter"), ("Records", "record_count"), ("Exit code", "exit_code")]:
    out.append(f"| {label} | `{run.get(key)}` |")
out.append(f"| Duration | {elapsed}s |")
if report_path:
    try:
        report = json.load(open(report_path))
        summary = report.get("summary", {})
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
print("\n".join(out))
PYEOF
fi
exit "$EXIT"
