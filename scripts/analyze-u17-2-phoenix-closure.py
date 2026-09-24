#!/usr/bin/env python3
"""Audit Phoenix tx95 closure against retained full and account-mode blocks."""

import argparse
import gzip
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
U17 = ROOT / "docs/examples/phase-u17-phoenix-qualification"
SIGNATURE = "rR1tBf4kb4xocTPpLZXRPqFFtvXQHn2GwAEgHbU8d83URqJNWnACGcz6cJfq27kGYhuWaTjrce7ksBNAF5NZVQC"
SLOT = 450026714
INDEX = 95


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    root = args.root
    account_bytes = gzip.decompress((U17 / "block-accounts.json.gz").read_bytes())
    full_bytes = (root / "raw/block-full.json").read_bytes()
    account_block = json.loads(account_bytes)["result"]
    full_block = json.loads(full_bytes)["result"]
    target = json.loads((U17 / "transaction.json").read_bytes())["result"]
    account_rows = account_block["transactions"]
    full_rows = full_block["transactions"]
    if not (len(account_rows) == len(full_rows) == 1116
            and account_block["parentSlot"] == full_block["parentSlot"] == SLOT - 1
            and account_block["blockhash"] == full_block["blockhash"]):
        raise RuntimeError("two block views differ in parent, hash, or transaction count")
    for index, (a, f) in enumerate(zip(account_rows, full_rows)):
        if a["transaction"]["signatures"] != f["transaction"]["signatures"]:
            raise RuntimeError(f"block signature order differs at {index}")
    if full_rows[INDEX]["transaction"]["signatures"][0] != SIGNATURE:
        raise RuntimeError("target index/signature differs")
    frozen = target.copy()
    for key in ("slot", "transactionIndex", "blockTime"):
        frozen.pop(key, None)
    frozen["meta"] = frozen["meta"].copy()
    frozen["meta"].pop("rewards", None)
    full_target = full_rows[INDEX].copy()
    full_target["meta"] = full_target["meta"].copy()
    full_target["meta"].pop("rewards", None)
    if frozen != full_target:
        raise RuntimeError("full block target differs from frozen transaction")
    message_keys = target["transaction"]["message"]["accountKeys"]
    account_target = account_rows[INDEX]["transaction"]["accountKeys"]
    if [row["pubkey"] for row in account_target] != message_keys:
        raise RuntimeError("account-mode key order differs from target message")
    inputs = set(message_keys)
    outputs = {row["pubkey"] for row in account_target if row["writable"]}
    before = []
    after = []
    for index, row in enumerate(account_rows):
        if index == INDEX:
            continue
        writable = {key["pubkey"] for key in row["transaction"]["accountKeys"] if key["writable"]}
        overlap = writable & (inputs if index < INDEX else outputs)
        if overlap:
            (before if index < INDEX else after).append({
                "index": index, "signature": row["transaction"]["signatures"][0],
                "success": row["meta"]["err"] is None, "accounts": sorted(overlap)})
    binaries = json.loads((root / "binaries.json").read_text())
    programdata = {row["programdata"] for row in binaries["binaries"] if row.get("programdata")}
    programdata_writers = [index for index, row in enumerate(account_rows) if
                           programdata & {key["pubkey"] for key in row["transaction"]["accountKeys"]
                                          if key["writable"]}]
    if before or after or programdata_writers:
        raise RuntimeError("single-target closure has a conflicting slot writer")
    outer = target["transaction"]["message"]["instructions"]
    earlier = [{"outer_index": index,
                "program": message_keys[ix["programIdIndex"]],
                "accounts": [message_keys[key] for key in ix["accounts"]]}
               for index, ix in enumerate(outer[:5])]
    companion_accounts = set().union(*(set(row["accounts"]) for row in earlier))
    phoenix_ix = outer[5]
    roles = [message_keys[index] for index in phoenix_ix["accounts"]]
    audit = {"schema": "U17_2PhoenixClosureAuditV1", "slot": SLOT, "parent_slot": SLOT - 1,
             "index": INDEX, "signature": SIGNATURE, "blockhash": full_block["blockhash"],
             "account_mode_block_sha256": sha(account_bytes), "full_block_sha256": sha(full_bytes),
             "transaction_count": len(full_rows), "input_keys": message_keys,
             "validation_outputs": sorted(outputs), "earlier_possible_writers": before,
             "later_possible_writers": after, "programdata_slot_writers": programdata_writers,
             "blockhash_model": "ordinary_recent_blockhash",
             "companion_instructions": earlier,
             "phoenix_role_prior_touch": {
                 "market": roles[2] in companion_accounts,
                 "seat": roles[4] in companion_accounts,
                 "trader_base_token": roles[5] in companion_accounts,
                 "trader_quote_token": roles[6] in companion_accounts,
                 "base_vault": roles[7] in companion_accounts,
                 "quote_vault": roles[8] in companion_accounts}}
    (root / "closure-audit.json").write_text(json.dumps(audit, indent=2) + "\n")
    print(json.dumps({"closure": "single_target", "full_block_sha256": audit["full_block_sha256"],
                      "companions_touch": audit["phoenix_role_prior_touch"]}))


if __name__ == "__main__":
    main()
