#!/usr/bin/env python3
"""Compare raw U7 account/transaction events with the frozen validator block.

This is an offline diagnostic, not a replay evidence resolver. It retains hashes
and transaction attribution without copying account data into the report.
"""

import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import struct

import geyser_pb2


ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58(raw):
    n = int.from_bytes(raw, "big")
    out = ""
    while n:
        n, rem = divmod(n, 58)
        out = ALPHABET[rem] + out
    return "1" * (len(raw) - len(raw.lstrip(b"\0"))) + out


def frames(path):
    with path.open("rb") as source:
        while header := source.read(4):
            assert len(header) == 4
            size = struct.unpack("<I", header)[0]
            raw = source.read(size)
            assert len(raw) == size
            update = geyser_pb2.SubscribeUpdate()
            update.ParseFromString(raw)
            yield update, hashlib.sha256(raw).hexdigest()


def analyze(stream, block, inventory, slot):
    frozen = json.loads(block.read_text())["result"]
    expected = [row["transaction"]["signatures"][0] for row in frozen["transactions"]]
    assert len(expected) == 1131
    roles = {row["address"]: row["role"] for row in json.loads(inventory.read_text())["accounts"]}
    transactions = {}
    duplicates = []
    accounts = defaultdict(list)
    all_accounts = defaultdict(list)
    block_metas = []
    slot_statuses = []
    counts = Counter()
    for update, raw_hash in frames(stream):
        kind = update.WhichOneof("update_oneof")
        event = getattr(update, kind)
        if getattr(event, "slot", None) != slot:
            continue
        counts[kind] += 1
        if kind == "transaction":
            info = event.transaction
            row = {"index": info.index, "signature": b58(info.signature), "raw_event_sha256": raw_hash}
            if info.index in transactions:
                duplicates.append(info.index)
            transactions[info.index] = row
        elif kind == "account":
            info = event.account
            address = b58(info.pubkey)
            row = {
                "address": address,
                "owner": b58(info.owner),
                "lamports": info.lamports,
                "executable": info.executable,
                "rent_epoch": info.rent_epoch,
                "data_length": len(info.data),
                "data_sha256": hashlib.sha256(info.data).hexdigest(),
                "write_version": info.write_version,
                "txn_signature": b58(info.txn_signature) if info.HasField("txn_signature") else None,
                "raw_event_sha256": raw_hash,
            }
            all_accounts[address].append(row)
            if address in roles:
                accounts[address].append(row)
        elif kind == "block_meta":
            block_metas.append({"blockhash": event.blockhash, "parent_slot": event.parent_slot,
                                "parent_blockhash": event.parent_blockhash,
                                "executed_transaction_count": event.executed_transaction_count,
                                "raw_event_sha256": raw_hash})
        elif kind == "slot":
            slot_statuses.append({"parent": event.parent, "status": geyser_pb2.SlotStatus.Name(event.status),
                                  "raw_event_sha256": raw_hash})
    mismatch = [{"index": i, "expected": sig, "actual": transactions.get(i, {}).get("signature")}
                for i, sig in enumerate(expected) if transactions.get(i, {}).get("signature") != sig]
    extra = sorted(set(transactions) - set(range(len(expected))))
    signature_index = {row["signature"]: i for i, row in transactions.items()}
    for events in all_accounts.values():
        for event in events:
            event["transaction_index"] = signature_index.get(event["txn_signature"])
    repeated = {address: rows for address, rows in all_accounts.items() if len(rows) > 1}
    version_groups = defaultdict(set)
    for rows in all_accounts.values():
        for row in rows:
            version_groups[row["write_version"]].add(row["transaction_index"])
    summary = {
        "slot": slot,
        "frozen_blockhash": frozen["blockhash"],
        "frozen_parent_slot": frozen["parentSlot"],
        "counts": dict(counts),
        "block_metas": block_metas,
        "slot_statuses": slot_statuses,
        "transaction_index_mismatches": mismatch,
        "transaction_duplicate_indexes": duplicates,
        "transaction_extra_indexes": extra,
        "missing_transaction_indexes": sorted(set(range(len(expected))) - set(transactions)),
        "account_event_count": sum(map(len, all_accounts.values())),
        "unique_account_count": len(all_accounts),
        "account_events_missing_signature": sum(row["txn_signature"] is None for rows in all_accounts.values() for row in rows),
        "account_events_signature_not_in_block": sum(row["txn_signature"] is not None and row["transaction_index"] is None for rows in all_accounts.values() for row in rows),
        "repeated_address_count": len(repeated),
        "repeated_address_examples": [
            {"address": address, "events": [{"write_version": r["write_version"], "transaction_index": r["transaction_index"], "data_sha256": r["data_sha256"]} for r in rows]}
            for address, rows in list(repeated.items())[:15]
        ],
        "write_version_count": len(version_groups),
        "write_versions_spanning_transaction_indexes": {str(v): sorted(i for i in indexes if i is not None) for v, indexes in version_groups.items() if len(indexes) > 1},
        "target_accounts": [
            {"address": address, "role": role, "events": accounts[address]}
            for address, role in roles.items()
        ],
    }
    return summary


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--stream", type=Path, required=True)
    parser.add_argument("--block", type=Path, required=True)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--slot", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = analyze(args.stream, args.block, args.inventory, args.slot)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps({k: result[k] for k in ("counts", "account_event_count", "unique_account_count", "account_events_missing_signature", "account_events_signature_not_in_block", "repeated_address_count", "write_version_count", "transaction_index_mismatches")}, sort_keys=True))


if __name__ == "__main__":
    main()
