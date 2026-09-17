#!/usr/bin/env bash
# Prove an account archive against a record Eplyx already validated.
#
# A generic probe can only answer "did the endpoint reply". This one answers
# the two questions that decide whether a corpus run is possible at all:
#
#   1. Does the archive honor `"slot": S` on getAccountInfo, and what does it
#      put in context.slot? historical.rs asserts context.slot == S, so an
#      archive that returns state-as-of-S while reporting the account's last
#      write slot is usable but needs that assertion changed rather than a
#      different provider.
#
#   2. Are the bytes right? The record on disk carries the exact account state
#      the original acquisition captured at S-1, so every field can be compared
#      rather than merely fetched. Matching bytes prove the archive; a reply
#      alone proves nothing.
#
# Usage:
#   scripts/probe-archive.sh <archive-url> [transaction-url] [corpus.json]
#
# Only the archive URL is required. Keys stay in the arguments and are never
# echoed: this prints endpoints as scheme://host, with query strings removed.
set -uo pipefail

ARCHIVE="${1:?usage: probe-archive.sh <archive-url> [transaction-url] [corpus.json]}"
TRANSACTION="${2:-}"
case "$TRANSACTION" in ""|http://*|https://*) ;; *)
  echo "second argument is not a URL: ${TRANSACTION}"; exit 2 ;;
esac
CORPUS="${3:-data/stake-pool-upgrade/session/corpus.json}"

[ -r "$CORPUS" ] || { echo "no corpus at $CORPUS"; exit 2; }
command -v python3 >/dev/null || { echo "python3 is required"; exit 2; }

call() { # url method params-json
  curl -sS --max-time 45 -X POST "$1" \
    -H 'Content-Type: application/json' \
    --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}" 2>&1
}
export -f call

redact() { python3 -c "
import sys,urllib.parse as u
p = u.urlsplit(sys.argv[1])
print(f'{p.scheme}://{p.netloc}{p.path[:24]}…' if p.path or p.query else f'{p.scheme}://{p.netloc}')
" "$1"; }

echo "== inputs =="
echo "  corpus       $CORPUS"
echo "  archive      $(redact "$ARCHIVE")"
[ -n "$TRANSACTION" ] && echo "  transaction  $(redact "$TRANSACTION")"

# The record names its own program, signature, slot and the state each account
# held. Nothing below is hard-coded to one protocol or one run.
read -r SIGNATURE SLOT PRE_SLOT GENESIS PROGRAM < <(python3 -c "
import json,sys
r = json.load(open(sys.argv[1]))[0]
t = r['transaction']
print(t['signature'], t['slot'], t['slot'] - 1, r['genesis_hash'], r['program_id'])
" "$CORPUS")

echo "  record       $SIGNATURE"
echo "  program      $PROGRAM"
echo "  slots        S-1 $PRE_SLOT  ·  S $SLOT"

fail=0
note() { printf '  %-5s %s\n' "$1" "$2"; [ "$1" = "FAIL" ] && fail=1; return 0; }

echo
echo "== identity =="
for pair in "archive:$ARCHIVE" "transaction:${TRANSACTION:-}"; do
  label="${pair%%:*}"; url="${pair#*:}"
  [ -z "$url" ] && continue
  got=$(call "$url" getGenesisHash '[]' | python3 -c "
import json,sys
try: d = json.load(sys.stdin)
except Exception: print('unparseable:' + sys.stdin.read()[:90]); raise SystemExit
if 'error' in d: print('refused:' + str(d['error'].get('message'))[:110])
else: print('hash:' + str(d.get('result','')))
")
  case "$got" in
    "hash:$GENESIS") note ok "$label genesis matches the record" ;;
    refused:*)       note FAIL "$label refused the call" "${got#refused:}" ;;
    unparseable:*)   note FAIL "$label gave no JSON" "${got#unparseable:}" ;;
    hash:)           note FAIL "$label did not answer" ;;
    *)               note FAIL "$label is a different cluster" "${got#hash:}" ;;
  esac
done

if [ -n "$TRANSACTION" ]; then
  echo
  echo "== transaction history =="
  first=$(call "$TRANSACTION" getFirstAvailableBlock '[]' | python3 -c "
import json,sys
try: print(json.load(sys.stdin).get('result','?'))
except Exception: print('?')
")
  note info "first available block $first (record needs $SLOT)"
  call "$TRANSACTION" getTransaction "[\"$SIGNATURE\",{\"encoding\":\"json\",\"maxSupportedTransactionVersion\":0}]" \
    | python3 -c "
import json,sys
try: d = json.load(sys.stdin)
except Exception: print('  FAIL  getTransaction returned no JSON'); sys.exit()
if 'error' in d: print(f\"  FAIL  getTransaction: {d['error'].get('message')}\")
elif d.get('result') is None: print('  FAIL  the archive no longer has this transaction')
else:
    r = d['result']
    inner = len(r.get('meta',{}).get('innerInstructions') or [])
    print(f\"  ok    getTransaction returned slot {r.get('slot')}, {inner} inner-instruction group(s)\")
"
fi

echo
echo "== account archive at slot $PRE_SLOT =="
python3 - "$CORPUS" "$ARCHIVE" "$PRE_SLOT" <<'PY'
import base64, json, subprocess, sys

corpus, url, pre_slot = sys.argv[1], sys.argv[2], int(sys.argv[3])

# The record and the wire disagree about how to write the same bytes. A record
# stores account data as hex and lamports as a decimal string; getAccountInfo
# answers base64 and a JSON number. Comparing the two representations directly
# reports every account with any data as different, which is the most alarming
# possible wrong answer: it condemns a correct archive.
def record_bytes(value):
    return bytes.fromhex(value or "")

def wire_bytes(value):
    if isinstance(value, list):
        value = value[0] if value else ""
    return base64.b64decode(value or "")

def lamports(value):
    return int(value) if value is not None else None
record = json.load(open(corpus))[0]
recorded = {a['address']: a['account'] for a in record['accounts']}
wanted = [(a['address'], a.get('label', '')) for a in record['acquisitions']]

def rpc(address):
    body = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "getAccountInfo",
        "params": [address, {"encoding": "base64", "commitment": "finalized", "slot": pre_slot}],
    })
    out = subprocess.run(
        ["curl", "-sS", "--max-time", "45", "-X", "POST", url,
         "-H", "Content-Type: application/json", "--data", body],
        capture_output=True, text=True)
    try:
        return json.loads(out.stdout)
    except Exception:
        return {"error": {"message": (out.stdout or out.stderr).strip()[:120]}}

exact = honored = compared = matched = gated = 0
context_slots = set()

for address, label in wanted:
    answer = rpc(address)
    short = f"{address[:8]}… {label:24}"
    if "error" in answer:
        message = str(answer["error"].get("message", ""))
        # An endpoint that does not implement the slot selector ignores it and
        # answers with current state. One that names a plan is telling us the
        # opposite: it implements it, and this key may not use it.
        if any(word in message.lower() for word in
               ("tier", "upgrade to", "plan", "not entitled", "subscription")):
            gated += 1
            print(f"  gated {short} {message[:96]}")
        else:
            print(f"  FAIL  {short} {message[:96]}")
        continue
    result = answer.get("result") or {}
    context = (result.get("context") or {}).get("slot")
    value = result.get("value")
    context_slots.add(context)
    honored += 1
    mark = "exact" if context == pre_slot else f"reports {context}"
    if context == pre_slot:
        exact += 1

    # The decisive comparison: does the returned state equal what the original
    # acquisition recorded at this slot?
    verdict = ""
    if address in recorded:
        compared += 1
        want = recorded[address]
        if value is None:
            verdict = "  · archive says absent, record has state"
        else:
            differences = []
            if lamports(value.get("lamports")) != lamports(want.get("lamports")):
                differences.append(
                    f"lamports {lamports(value.get('lamports'))} vs {lamports(want.get('lamports'))}")
            if value.get("owner") != want.get("owner"):
                differences.append(f"owner {value.get('owner')} vs {want.get('owner')}")
            got, expected = wire_bytes(value.get("data")), record_bytes(want.get("data"))
            if got != expected:
                if len(got) != len(expected):
                    differences.append(f"data {len(got)} bytes vs {len(expected)}")
                else:
                    at = next(i for i, (a, b) in enumerate(zip(got, expected)) if a != b)
                    differences.append(f"data differs at byte {at} of {len(got)}")
            if bool(value.get("executable")) != bool(want.get("executable")):
                differences.append("executable")
            if differences:
                # Naming the field is the difference between "this archive is
                # wrong" and "one of these two is an off-by-one, and here is
                # which number moved".
                verdict = "  · DIFFERS: " + ", ".join(differences)
            else:
                matched += 1
                verdict = "  · bytes match the record"
    elif value is None:
        verdict = "  · absent (not in the replay state)"
    else:
        verdict = "  · present, not compared (program or dependency)"

    print(f"  {'ok   ' if context == pre_slot else 'note '} {short} context.slot {mark}{verdict}")

print()
print("== verdict ==")
if honored == 0 and gated:
    # The decisive distinction. A provider that lacks the feature ignores the
    # slot and hands back today's state; this one refuses by name, which means
    # the API Eplyx needs is there and this key cannot reach it.
    print(f"  note  the slot selector is implemented but not enabled for this key ({gated}/{len(wanted)} accounts)")
    print("  ok    this is the right endpoint on the wrong plan, not the wrong endpoint")
    print("        Re-run on a plan that includes historical slot parameters.")
    sys.exit(3)
if honored == 0:
    print("  FAIL  the archive answered nothing usable; it cannot serve as account_archive_rpc")
    sys.exit(1)

# The bytes decide whether the archive is usable at all. The slot only decides
# whether one assertion has to change, so it is never the headline.
state_ok = bool(compared) and matched == compared
slot_ok = exact == honored

if not compared:
    print("  note  no account in this record could be compared; the bytes are unproven")
elif state_ok:
    print(f"  ok    state matches the validated record for {matched}/{compared} compared accounts")
else:
    print(f"  FAIL  only {matched}/{compared} compared accounts match the record")
    print("        This is not state as of S-1, whatever context.slot says.")

if not slot_ok:
    print(f"  note  context.slot equals the requested slot for {exact}/{honored} accounts")
    print(f"        observed: {sorted(s for s in context_slots if s is not None)}")

if state_ok and slot_ok:
    print("  ok    SlotAccountArchiveProvider works against this endpoint unchanged")
elif state_ok:
    print("  ok    the archive is correct; the assertion in engine/src/historical.rs:57")
    print("        is what needs changing, not the provider")
else:
    print("  FAIL  do not build a corpus against this endpoint")

sys.exit(0 if state_ok else 1)
PY
rc=$?

echo
# Bash's `||` yields 1, which would collapse "not entitled" into "unusable".
if [ $fail -ne 0 ]; then
  echo "probe completed with failures"
  exit 1
fi
case $rc in
  0) echo "probe completed" ;;
  3) echo "probe completed: the endpoint is right, the plan is not" ;;
  *) echo "probe completed with failures" ;;
esac
exit $rc
