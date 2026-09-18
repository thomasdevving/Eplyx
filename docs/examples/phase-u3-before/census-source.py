#!/usr/bin/env python3
"""Bounded KLend activity census: observed vs replay-eligible, by instruction.

Reports the three populations the corpus discipline requires, and never
collapses them. An unmeasured population is reported as absent, not zero.
"""
import json, subprocess, sys, hashlib, re, collections, time

RPC = "https://solana-mainnet.g.alchemy.com/v2/docs-demo"
ORG = "https://www.alchemy.com"
KLEND = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD"
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

def b58decode(s):
    n = 0
    for c in s: n = n*58 + B58.index(c)
    raw = n.to_bytes((n.bit_length()+7)//8, 'big')
    return b'\0'*(len(s)-len(s.lstrip('1'))) + raw

def call(method, params, retries=3):
    body = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":params})
    delay = 0.4
    for _ in range(retries):
        out = subprocess.run(
            ["curl","-sS","--max-time","60","-X","POST",RPC,
             "-H","Content-Type: application/json","-H",f"Origin: {ORG}","-d",body],
            capture_output=True, text=True).stdout
        try: d = json.loads(out)
        except Exception:
            time.sleep(delay); delay=min(delay*2, 6); continue
        if "error" in d:
            time.sleep(delay); delay=min(delay*2, 3); continue
        return d.get("result")
    return None

def disc(name):
    snake = re.sub(r'(?<!^)(?=[A-Z])','_',name).lower()
    return hashlib.sha256(("global:"+snake).encode()).digest()[:8]

IDL = json.load(open(sys.argv[1]))
BY_DISC = {bytes(disc(i["name"])): i["name"] for i in IDL["instructions"]}
TARGET = int(sys.argv[2])
OUT = sys.argv[3]

sigs, before, seen = [], None, set()
while len(sigs) < TARGET:
    p = {"limit":1000}
    if before: p["before"] = before
    r = call("getSignaturesForAddress",[KLEND,p])
    if not r: break
    fresh = [s for s in r if s["signature"] not in seen]
    if not fresh: break
    for s in fresh: seen.add(s["signature"])
    sigs.extend(fresh); before = r[-1]["signature"]

observed = collections.Counter()      # successful top-level KLend ix, all shapes
eligible = collections.Counter()      # ... in a message resolving no LUT entries
detail = collections.defaultdict(list)
totals = collections.Counter()
totals["signatures_listed"] = len(sigs)

for s in sigs:
    if s.get("err"):
        totals["original_failed"] += 1; continue
    totals["original_succeeded"] += 1

for s in sigs:
    if s.get("err"): continue
    tx = call("getTransaction",[s["signature"],{"encoding":"json","maxSupportedTransactionVersion":0}])
    if not tx:
        totals["fetch_failed"] += 1; continue
    totals["examined"] += 1
    if totals["examined"] % 25 == 0:
        print(f"  examined {totals['examined']}...", flush=True)
        json.dump({"totals":dict(totals),"observed":dict(observed),
                   "eligible":dict(eligible),"detail":{k:v for k,v in detail.items()}},
                  open(OUT,"w"), indent=1)
    msg = tx["transaction"]["message"]
    keys = [k["pubkey"] if isinstance(k,dict) else k for k in msg["accountKeys"]]
    meta = tx.get("meta") or {}
    loaded = meta.get("loadedAddresses") or {}
    n_loaded = len(loaded.get("writable",[])) + len(loaded.get("readonly",[]))
    version = str(tx.get("version","legacy"))
    names = []
    for ix in msg["instructions"]:
        if keys[ix["programIdIndex"]] != KLEND: continue
        names.append(BY_DISC.get(b58decode(ix["data"])[:8], "unknown"))
    if not names:
        totals["no_klend_top_level"] += 1; continue
    totals["with_klend_top_level"] += 1
    if n_loaded == 0: totals["no_lookup_tables"] += 1
    else: totals["uses_lookup_tables"] += 1
    for n in set(names):
        observed[n] += 1
        if n_loaded == 0:
            eligible[n] += 1
            top_progs = sorted({keys[i["programIdIndex"]] for i in msg["instructions"]})
            inner = meta.get("innerInstructions") or []
            detail[n].append({
                "signature": s["signature"], "slot": s["slot"], "version": version,
                "top_instructions": len(msg["instructions"]),
                "klend_instructions": names,
                "top_programs": top_progs,
                "inner_groups": len(inner),
                "inner_count": sum(len(g["instructions"]) for g in inner),
                "max_stack": max([i.get("stackHeight") or 2 for g in inner for i in g["instructions"]], default=0),
            })

print("== bounded KLend activity census ==")
if sigs:
    print(f"slot window: {min(s['slot'] for s in sigs)} - {max(s['slot'] for s in sigs)}")
for k,v in totals.most_common(): print(f"  {v:6d}  {k}")
print("\n{:<56}{:>10}{:>12}".format("instruction","observed","no-LUT"))
for n, c in observed.most_common():
    print("  {:<54}{:>10}{:>12}".format(n, c, eligible.get(n,0)))
json.dump({"totals":dict(totals),"observed":dict(observed),"eligible":dict(eligible),
           "detail":{k:v for k,v in detail.items()}}, open(OUT,"w"), indent=1)
print(f"\nwritten: {OUT}")
