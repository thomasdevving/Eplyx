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
try: print(json.load(sys.stdin).get('result',''))
except Exception: print('')
")
  if [ "$got" = "$GENESIS" ]; then note ok "$label genesis matches the record"
  elif [ -z "$got" ]; then note FAIL "$label did not answer getGenesisHash"
  else note FAIL "$label is a different cluster ($got)"; fi
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
import json, subprocess, sys

corpus, url, pre_slot = sys.argv[1], sys.argv[2], int(sys.argv[3])
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

exact = honored = compared = matched = 0
context_slots = set()

for address, label in wanted:
    answer = rpc(address)
    short = f"{address[:8]}… {label:24}"
    if "error" in answer:
        print(f"  FAIL  {short} {answer['error'].get('message')}")
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
            data = value.get("data")
            data = data[0] if isinstance(data, list) else data
            same = (value.get("lamports") == want["lamports"]
                    and value.get("owner") == want["owner"]
                    and (data or "") == (want.get("data") or "")
                    and bool(value.get("executable")) == bool(want["executable"]))
            if same:
                matched += 1
                verdict = "  · bytes match the record"
            else:
                verdict = "  · BYTES DIFFER from the record"
    elif value is None:
        verdict = "  · absent (not in the replay state)"
    else:
        verdict = "  · present, not compared (program or dependency)"

    print(f"  {'ok   ' if context == pre_slot else 'note '} {short} context.slot {mark}{verdict}")

print()
print("== verdict ==")
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
[ $fail -eq 0 ] && [ $rc -eq 0 ] && echo "probe completed" || echo "probe completed with failures"
exit $(( fail || rc ))
