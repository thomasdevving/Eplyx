#!/usr/bin/env python3
"""Apply the existing Eplyx accounts-mode same-slot rule to frozen direct swaps."""

from collections import Counter
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1] / "docs/examples/phase-u5-sample"
CORE_ROLES = {"whirlpool", "token_vault_a", "token_vault_b", "tick_array_0",
              "tick_array_1", "tick_array_2", "oracle"}


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def transaction_keys(tx):
    keys = tx["transaction"].get("accountKeys")
    assert isinstance(keys, list) and keys
    for key in keys:
        assert isinstance(key.get("pubkey"), str)
        assert isinstance(key.get("writable"), bool), "missing writable flag cannot prove a clean boundary"
    return keys


def same_slot_conflict_rejected(summary):
    """A real later writer prevents this bounded cohort from claiming clean post-state."""
    assert summary["direct_swaps_screened"] > 0
    assert summary["post_dirty"] == summary["direct_swaps_screened"]
    assert summary["both_clean"] == 0


def main():
    report = json.loads((ROOT / "classification.json").read_bytes())
    idl = json.loads((ROOT / "discriminator-index.json").read_bytes())
    names = {x["name"]: x["fixed_account_names"] for x in idl["instructions"]}
    results = []
    for row_index, row in enumerate(report["rows"]):
        if not row.get("outer_orca"):
            continue
        slot = row["slot"]
        receipt = json.loads((ROOT / "screen-receipts" / f"{slot}.json").read_bytes())
        if receipt["http_status"] != "200" or receipt["transaction_count"] is None:
            results.append({"sample_index": row_index, "slot": slot, "signature": row["signature"],
                            "status": "screen_block_unavailable"})
            continue
        body = (ROOT / "screen-blocks" / f"{slot}.body").read_bytes()
        assert hashlib.sha256(body).hexdigest() == receipt["body_sha256"]
        txs = json.loads(body)["result"]["transactions"]
        assert len(txs) == receipt["transaction_count"]
        target_index = next(i for i, tx in enumerate(txs)
                            if tx["transaction"]["signatures"][0] == row["signature"])
        assert target_index == row["transaction_index"]
        target_keys = transaction_keys(txs[target_index])
        required = {key["pubkey"] for key in target_keys}
        assert len(required) == len(target_keys)
        full_tx = json.loads((ROOT / "blocks" / f"{slot}.body").read_bytes())["result"]["transactions"][target_index]
        message = full_tx["transaction"]["message"]
        loaded = full_tx["meta"].get("loadedAddresses") or {}
        ordered = message["accountKeys"] + loaded.get("writable", []) + loaded.get("readonly", [])
        assert ordered == [key["pubkey"] for key in target_keys]
        instruction = message["instructions"][row["outer_orca"][0]["outer_index"]]
        role_names = names[row["outer_orca"][0]["instruction_name"]]
        role_map = {ordered[account_index]: role_names[i]
                    for i, account_index in enumerate(instruction["accounts"][:len(role_names)])}
        conflicts = []
        for index, tx in enumerate(txs):
            if index == target_index:
                continue
            writable = {key["pubkey"] for key in transaction_keys(tx) if key["writable"]}
            shared = sorted(writable & required)
            if shared:
                conflicts.append({"transaction_index": index,
                                  "signature": tx["transaction"]["signatures"][0],
                                  "position": "before" if index < target_index else "after",
                                  "accounts": [{"address": address,
                                                "orca_role": role_map.get(address)} for address in shared]})
        before = [x for x in conflicts if x["position"] == "before"]
        after = [x for x in conflicts if x["position"] == "after"]
        core_before = [x for x in before if any(a["orca_role"] in CORE_ROLES for a in x["accounts"])]
        core_after = [x for x in after if any(a["orca_role"] in CORE_ROLES for a in x["accounts"])]
        results.append({"sample_index": row_index, "slot": slot, "signature": row["signature"],
                        "transaction_index": target_index, "version": row["version"],
                        "instruction": row["outer_orca"][0]["instruction_name"],
                        "required_key_count": len(required), "before_conflict_count": len(before),
                        "after_conflict_count": len(after), "core_before_conflict_count": len(core_before),
                        "core_after_conflict_count": len(core_after),
                        "pre_boundary_clean": not before, "post_boundary_clean": not after,
                        "status": "TransactionBoundaryStateRequired" if conflicts else "clean",
                        "conflicts": conflicts})
    counts = Counter(x["status"] for x in results)
    summary = {"sample_fingerprint": report["summary"]["sample_fingerprint"],
               "direct_swaps_screened": len(results), "statuses": dict(counts),
               "pre_dirty": sum(x.get("pre_boundary_clean") is False for x in results),
               "post_dirty": sum(x.get("post_boundary_clean") is False for x in results),
               "both_clean": sum(x.get("status") == "clean" for x in results),
               "first_direct_sample_index": results[0]["sample_index"],
               "first_direct_status": results[0]["status"]}
    output = {"summary": summary, "results": results}
    (ROOT / "same-slot-screen.json").write_bytes(canonical(output) + b"\n")
    same_slot_conflict_rejected(summary)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
