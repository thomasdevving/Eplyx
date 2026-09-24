#!/usr/bin/env python3
"""Audit retained U17.2 receipts offline; no replay or semantic claims."""

import argparse
import base64
import gzip
import hashlib
import importlib.machinery
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[1]
U17 = ROOT / "docs/examples/phase-u17-phoenix-qualification"
BINARY = importlib.machinery.SourceFileLoader(
    "u17_2_binary", str(Path(__file__).with_name("acquire-u17-2-phoenix-binaries.py"))
).load_module()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    root = args.root
    acquisition = json.loads((root / "acquisition.json").read_bytes())
    binaries = json.loads((root / "binaries.json").read_bytes())
    closure = json.loads((root / "closure-audit.json").read_bytes())
    partial = json.loads((root / "runtime-partial.json").read_bytes())
    target = json.loads((U17 / "transaction.json").read_bytes())["result"]

    def receipt(name, digest, slot=None):
        raw = (root / "raw" / name).read_bytes()
        assert sha(raw) == digest, name
        response = json.loads(raw)
        assert response.get("error") is None and response.get("result") is not None, name
        if slot is not None:
            assert response["result"]["context"]["slot"] == slot, name
        return response["result"], raw

    genesis = json.loads((root / "raw/genesis.json").read_bytes())
    assert genesis["result"] == "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
    assert sha((root / "raw/genesis.json").read_bytes()) == acquisition["genesis_sha256"]

    account_rows = {}
    for row in acquisition["receipts"]:
        result, _ = receipt(row["receipt"], row["response_sha256"], row["slot"])
        value = result["value"]
        assert (value is not None) == row["present"]
        if value is None:
            account_rows[(row["slot"], row["address"])] = None
            continue
        assert value["data"][1] == "base64"
        data = base64.b64decode(value["data"][0], validate=True)
        assert len(data) == value["space"] == row["space"]
        assert sha(data) == row["data_sha256"]
        for key, field in (("owner", "owner"), ("lamports", "lamports"),
                           ("executable", "executable"), ("rent_epoch", "rentEpoch")):
            assert row[key] == value[field], (row["address"], key)
        account_rows[(row["slot"], row["address"])] = (value, data)
    keys = acquisition["message_keys"]
    writable = acquisition["writable_keys"]
    assert len(keys) == 18 and len(writable) == 9
    assert len(acquisition["receipts"]) == 27
    assert keys == target["transaction"]["message"]["accountKeys"]
    assert sum(account_rows[(acquisition["parent_slot"], key)] is None for key in keys) == 1
    pre_balance_matches = sum(
        (account_rows[(acquisition["parent_slot"], key)][0]["lamports"]
         if account_rows[(acquisition["parent_slot"], key)] else 0)
        == target["meta"]["preBalances"][index] for index, key in enumerate(keys))
    post_balance_matches = sum(
        account_rows[(acquisition["target_slot"], key)][0]["lamports"]
        == target["meta"]["postBalances"][keys.index(key)] for key in writable)
    token_matches = 0
    for side, slot in (("pre", acquisition["parent_slot"]), ("post", acquisition["target_slot"])):
        for row in target["meta"][f"{side}TokenBalances"]:
            key = keys[row["accountIndex"]]
            data = account_rows[(slot, key)][1]
            assert len(data) >= 72
            amount = struct.unpack_from("<Q", data, 64)[0]
            token_matches += amount == int(row["uiTokenAmount"]["amount"])
    assert (pre_balance_matches, post_balance_matches, token_matches) == (18, 9, 8)

    elf_hashes = {}
    for row in binaries["binaries"]:
        program = row["program"]
        program_data = account_rows[(acquisition["parent_slot"], program)][1]
        if row.get("native"):
            continue
        if row["loader"] == BINARY.LEGACY:
            assert sha(program_data) == row["elf_sha256"] and program_data[:4] == b"\x7fELF"
            elf_hashes[program] = row["elf_sha256"]
            continue
        address = row["programdata"]
        assert len(program_data) == 36 and struct.unpack_from("<I", program_data)[0] == 2
        assert BINARY.b58(program_data[4:36]) == address
        images = []
        for boundary in ("parent", "target"):
            facts = row[boundary]
            slices = []
            next_offset = 0
            for part in facts["chunks"]:
                assert part["offset"] == next_offset
                result, _ = receipt(part["receipt"], part["response_sha256"], facts["slot"])
                value = result["value"]
                data = base64.b64decode(value["data"][0], validate=True)
                assert len(data) == part["length"] and sha(data) == part["data_sha256"]
                assert value["space"] == facts["space"] and value["owner"] == BINARY.UPGRADEABLE
                slices.append(data)
                next_offset += len(data)
            account = b"".join(slices)
            header = facts["header_receipt"]
            header_result, _ = receipt(header["receipt"], header["response_sha256"], facts["slot"])
            assert base64.b64decode(header_result["value"]["data"][0]) == account[:45]
            assert len(account) == facts["space"] and sha(account) == facts["account_sha256"]
            assert account[:4] == b"\x03\0\0\0" and account[45:49] == b"\x7fELF"
            assert struct.unpack_from("<Q", account, 4)[0] == facts["deployment_slot"] <= facts["slot"]
            assert sha(account[45:]) == facts["elf_sha256"] and len(account[45:]) == facts["elf_length"]
            images.append(account)
        assert images[0] == images[1], program
        elf_hashes[program] = row["parent"]["elf_sha256"]

    assert len(partial["receipts"]) == partial["acquired_feature_receipts"] == 62
    assert partial["required_backend_known_features"] == 349
    assert partial["missing_feature_count"] == 287
    source = ROOT / partial["feature_source"]
    assert sha(source.read_bytes()) == partial["feature_source_sha256"]
    required = {row["id"] for row in json.loads(source.read_bytes())["observations"]}
    acquired = {row["address"] for row in partial["receipts"]}
    assert required == acquired | set(partial["missing_feature_ids"])
    assert not acquired & set(partial["missing_feature_ids"])
    for row in partial["receipts"]:
        result, _ = receipt(row["receipt"], row["response_sha256"], partial["slot"])
        assert (result["value"] is not None) == row["present"]
    for row in partial["sysvars"]:
        result, _ = receipt(row["receipt"], row["response_sha256"], partial["slot"])
        assert (result["value"] is not None) == row["present"]
    full_block = (root / "raw/block-full.json").read_bytes()
    account_block = gzip.decompress((U17 / "block-accounts.json.gz").read_bytes())
    assert sha(full_block) == closure["full_block_sha256"]
    assert sha(account_block) == closure["account_mode_block_sha256"]
    assert closure["earlier_possible_writers"] == closure["later_possible_writers"] == []
    assert closure["programdata_slot_writers"] == []
    report = {"kind": "u17_2_phoenix_historical_evidence_audit", "outcome": "historical_runtime_feature_gap",
              "parent_accounts": len(keys), "absent_pre": [key for key in keys if account_rows[(acquisition["parent_slot"], key)] is None],
              "terminal_accounts": len(writable), "pre_lamport_matches": pre_balance_matches,
              "post_lamport_matches": post_balance_matches, "token_amount_matches": token_matches,
              "historical_elf_sha256": elf_hashes, "full_block_sha256": sha(full_block),
              "feature_receipts": partial["acquired_feature_receipts"],
              "missing_feature_receipts": partial["missing_feature_count"],
              "historical_replay_attempted": False}
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
