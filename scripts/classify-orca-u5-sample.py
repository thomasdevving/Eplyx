#!/usr/bin/env python3
"""Classify the frozen Orca signatures using pinned IDL discriminators and raw blocks."""

from collections import Counter
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1] / "docs/examples/phase-u5-sample"
ORCA = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc"
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def decode58(value):
    number = 0
    for char in value:
        number = number * 58 + ALPHABET.index(char)
    encoded = number.to_bytes((number.bit_length() + 7) // 8, "big")
    return b"\0" * (len(value) - len(value.lstrip("1"))) + encoded


def family(name):
    if name in ("swap", "swap_v2", "two_hop_swap", "two_hop_swap_v2",
                "increase_liquidity", "increase_liquidity_v2", "decrease_liquidity",
                "decrease_liquidity_v2", "collect_fees", "collect_fees_v2"):
        return name
    if name and ("open_position" in name or "close_position" in name):
        return "open_close_position"
    return "other"


def classify_instruction(instruction, keys, idl):
    index = instruction.get("programIdIndex")
    program = keys[index] if isinstance(index, int) and 0 <= index < len(keys) else None
    if program != ORCA:
        return None
    data = decode58(instruction.get("data", ""))
    descriptor = idl.get(data[:8].hex(), {}) if len(data) >= 8 else {}
    name = descriptor.get("name")
    return {
        "program_id": program, "instruction_name": name,
        "discriminator_hex": data[:8].hex(), "data_bytes": len(data),
        "account_count": len(instruction.get("accounts", [])),
        "fixed_idl_account_count": descriptor.get("fixed_account_count"),
        "remaining_account_count": max(0, len(instruction.get("accounts", [])) - descriptor["fixed_account_count"])
        if "fixed_account_count" in descriptor else None,
        "family": family(name),
    }


def idl_fixed_arity_is_not_runtime_arity(rows):
    """A real compiled Whirlpool instruction must retain accounts beyond fixed IDL roles."""
    extra = [instruction for row in rows for instruction in row.get("outer_orca", [])
             if instruction["instruction_name"] == "swap_v2"
             and instruction["remaining_account_count"] > 0]
    assert extra, "production remaining accounts disappeared from classification"
    return len(extra)


def main():
    sample = json.loads((ROOT / "sample.json").read_bytes())
    assert hashlib.sha256(canonical(sample["selection"])).hexdigest() == sample["sample_fingerprint"]
    index = json.loads((ROOT / "discriminator-index.json").read_bytes())
    idl = {row["discriminator_hex"]: row for row in index["instructions"]}
    assert len(idl) == 66
    blocks = {}
    for path in (ROOT / "block-receipts").glob("*.json"):
        receipt = json.loads(path.read_bytes())
        if receipt["curl_exit"] != 0 or receipt["http_status"] != "200" or not receipt["block_present"]:
            continue
        body = (ROOT / "blocks" / f"{receipt['slot']}.body").read_bytes()
        assert hashlib.sha256(body).hexdigest() == receipt["body_sha256"]
        blocks[receipt["slot"]] = json.loads(body)["result"]["transactions"]
    rows = []
    for item in sample["selection"]:
        row = {"source_index": item["source_index"], "signature": item["signature"],
               "slot": item["slot"], "block_available": item["slot"] in blocks}
        receipt = json.loads((ROOT / "receipts" / f"{len(rows):03d}.json").read_bytes())
        assert receipt["signature"] == item["signature"]
        row["get_transaction_status"] = receipt["http_status"]
        row["get_transaction_rpc_error"] = receipt["rpc_error"]
        txs = blocks.get(item["slot"])
        if txs is None:
            rows.append(row)
            continue
        found = [(i, tx) for i, tx in enumerate(txs)
                 if tx["transaction"]["signatures"][0] == item["signature"]]
        assert len(found) == 1, f"signature/block mismatch: {item['signature']}"
        tx_index, tx = found[0]
        message = tx["transaction"]["message"]
        loaded = tx["meta"].get("loadedAddresses") or {}
        keys = message["accountKeys"] + loaded.get("writable", []) + loaded.get("readonly", [])
        outer = []
        for outer_index, instruction in enumerate(message.get("instructions", [])):
            detected = classify_instruction(instruction, keys, idl)
            if detected is not None:
                detected["outer_index"] = outer_index
                outer.append(detected)
        inner = []
        for group in tx["meta"].get("innerInstructions") or []:
            for instruction in group.get("instructions", []):
                detected = classify_instruction(instruction, keys, idl)
                if detected is not None:
                    detected["outer_index"] = group["index"]
                    detected["stack_height"] = instruction.get("stackHeight")
                    inner.append(detected)
        row.update({"transaction_index": tx_index, "version": tx.get("version"),
                    "original_error": tx["meta"].get("err"), "outer_orca": outer,
                    "inner_orca": inner, "compiled_instruction_count": len(message.get("instructions", [])),
                    "key_count": len(keys)})
        rows.append(row)
    counts = Counter(inst["family"] for row in rows for inst in row.get("outer_orca", []))
    summary = {
        "sample_fingerprint": sample["sample_fingerprint"],
        "policy_sha256": sample["policy_sha256"],
        "idl_source_sha256": index["source_sha256"],
        "selected_signatures": len(rows), "full_blocks": len(blocks),
        "structurally_classified": sum("outer_orca" in row for row in rows),
        "unclassified_due_to_block_fetch": sum("outer_orca" not in row for row in rows),
        "direct_orca_transactions": sum(bool(row.get("outer_orca")) for row in rows),
        "one_direct_orca_target": sum(len(row.get("outer_orca", [])) == 1 for row in rows),
        "cpi_only_orca_transactions": sum(not row.get("outer_orca") and bool(row.get("inner_orca")) for row in rows),
        "no_observed_orca_instruction": sum("outer_orca" in row and not row["outer_orca"] and not row["inner_orca"] for row in rows),
        "version_counts": dict(Counter(str(row["version"]) for row in rows if "version" in row)),
        "outer_family_counts": dict(sorted(counts.items())),
        "outer_unknown_discriminators": sorted({inst["discriminator_hex"] for row in rows
                                                 for inst in row.get("outer_orca", [])
                                                 if inst["instruction_name"] is None}),
        "swap_v2_with_remaining_accounts": idl_fixed_arity_is_not_runtime_arity(rows),
    }
    result = {"summary": summary, "rows": rows}
    (ROOT / "classification.json").write_bytes(canonical(result) + b"\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
