#!/usr/bin/env python3
"""Reconstruct exact historical BPF ProgramData from deterministic archive slices."""

import argparse
import base64
import hashlib
import importlib.machinery
import json
import os
from pathlib import Path
import struct
import time
from urllib.parse import urlsplit

HELPERS = importlib.machinery.SourceFileLoader(
    "u17_2_state", str(Path(__file__).with_name("acquire-u17-2-phoenix-state.py"))
).load_module()
SLOT_PAIR = (HELPERS.PARENT, HELPERS.TARGET)
UPGRADEABLE = "BPFLoaderUpgradeab1e11111111111111111111111"
LEGACY = "BPFLoader2111111111111111111111111111111111"
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
CHUNK = 1_048_576


def b58(data):
    number = int.from_bytes(data, "big")
    result = ""
    while number:
        number, digit = divmod(number, 58)
        result = ALPHABET[digit] + result
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + result


def saved_value(root, receipt):
    return json.loads((root / "raw" / receipt).read_bytes())["result"]["value"]


def slice_account(endpoint, root, address, slot, offset, length):
    name = f"programdata-{slot}-{address}-{offset}-{length}.json"
    params = [address, {"encoding": "base64", "commitment": "finalized", "slot": slot,
                        "dataSlice": {"offset": offset, "length": length}}]
    parsed, raw = HELPERS.fetch(endpoint, root / "raw" / name, "getAccountInfo", params)
    result = parsed["result"]
    if result["context"]["slot"] != slot or result["value"] is None:
        raise RuntimeError(f"{name}: missing exact-slot ProgramData")
    value = result["value"]
    if value["data"][1] != "base64":
        raise RuntimeError(f"{name}: wrong encoding")
    data = base64.b64decode(value["data"][0], validate=True)
    if len(data) != length:
        raise RuntimeError(f"{name}: incomplete slice")
    return value, data, {"offset": offset, "length": length, "receipt": name,
                         "response_sha256": HELPERS.sha(raw), "data_sha256": HELPERS.sha(data)}


def full_programdata(endpoint, root, address, slot):
    header_value, header, header_row = slice_account(endpoint, root, address, slot, 0, 45)
    space = header_value["space"]
    if space < 49 or header_value["owner"] != UPGRADEABLE or struct.unpack_from("<I", header)[0] != 3:
        raise RuntimeError(f"{address}: invalid ProgramData header")
    pieces = []
    rows = []
    for offset in range(0, space, CHUNK):
        length = min(CHUNK, space - offset)
        value, data, row = slice_account(endpoint, root, address, slot, offset, length)
        for key in ("space", "owner", "lamports", "rentEpoch", "executable"):
            if value[key] != header_value[key]:
                raise RuntimeError(f"{address}: metadata changed across slices")
        if offset == 0 and data[:45] != header:
            raise RuntimeError(f"{address}: header slice disagrees")
        pieces.append(data)
        rows.append(row)
        print(json.dumps({"slot": slot, "address": address, "offset": offset,
                          "length": length}), flush=True)
        time.sleep(0.15)
    account = b"".join(pieces)
    if len(account) != space or account[45:49] != b"\x7fELF":
        raise RuntimeError(f"{address}: account or ELF incomplete")
    deploy_slot = struct.unpack_from("<Q", account, 4)[0]
    if deploy_slot > slot:
        raise RuntimeError(f"{address}: future deployment slot")
    authority = b58(account[13:45]) if account[12] == 1 else None
    if account[12] not in (0, 1):
        raise RuntimeError(f"{address}: malformed upgrade authority")
    return account, {"address": address, "slot": slot, "owner": header_value["owner"],
                     "lamports": header_value["lamports"], "rent_epoch": header_value["rentEpoch"],
                     "executable": header_value["executable"], "space": space,
                     "account_sha256": HELPERS.sha(account), "elf_sha256": HELPERS.sha(account[45:]),
                     "elf_length": len(account) - 45, "deployment_slot": deploy_slot,
                     "upgrade_authority": authority, "header_receipt": header_row,
                     "chunks": rows}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    root = args.root
    acquisition = json.loads((root / "acquisition.json").read_text())
    endpoint = os.environ.get("SOLANA_ARCHIVE_RPC_URL") or os.environ.get("SOLANA_RPC_URL") or HELPERS.PUBLIC_ARCHIVE
    if urlsplit(endpoint).hostname != acquisition["provider_host"]:
        raise RuntimeError("binary archive host differs from state archive")
    rows = {r["address"]: r for r in acquisition["receipts"] if r["slot"] == HELPERS.PARENT}
    tx = json.loads(HELPERS.SOURCE.read_bytes())["result"]
    keys = tx["transaction"]["message"]["accountKeys"]
    called = sorted({keys[ix["programIdIndex"]] for ix in tx["transaction"]["message"]["instructions"]}
                    | {keys[ix["programIdIndex"]] for group in tx["meta"]["innerInstructions"]
                       for ix in group["instructions"]})
    binaries = []
    for program in called:
        row = rows[program]
        if not row["present"] or not row["executable"]:
            raise RuntimeError(f"{program}: invoked program not executable at parent")
        data = base64.b64decode(saved_value(root, row["receipt"])["data"][0])
        if row["owner"] == LEGACY:
            if data[:4] != b"\x7fELF":
                raise RuntimeError(f"{program}: legacy BPF account lacks ELF")
            binaries.append({"program": program, "loader": LEGACY, "elf_sha256": HELPERS.sha(data),
                             "elf_length": len(data), "program_receipt": row["receipt"]})
            (root / f"historical-{program}.so").write_bytes(data)
        elif row["owner"] == UPGRADEABLE:
            if len(data) != 36 or struct.unpack_from("<I", data)[0] != 2:
                raise RuntimeError(f"{program}: malformed upgradeable Program account")
            address = b58(data[4:36])
            images = []
            for slot in SLOT_PAIR:
                account, facts = full_programdata(endpoint, root, address, slot)
                images.append(facts)
                if slot == HELPERS.PARENT:
                    (root / f"historical-programdata-{address}.bin").write_bytes(account)
                    (root / f"historical-{program}.so").write_bytes(account[45:])
            if images[0]["account_sha256"] != images[1]["account_sha256"]:
                raise RuntimeError(f"{program}: ProgramData changed across target slot")
            binaries.append({"program": program, "loader": UPGRADEABLE,
                             "programdata": address, "program_receipt": row["receipt"],
                             "parent": images[0], "target": images[1]})
        else:
            binaries.append({"program": program, "loader": row["owner"], "native": True,
                             "program_receipt": row["receipt"]})
    manifest = {"schema": "U17_2PhoenixBinaryAcquisitionV1", "provider_host": acquisition["provider_host"],
                "parent_slot": HELPERS.PARENT, "target_slot": HELPERS.TARGET, "invoked_programs": called,
                "binaries": binaries}
    (root / "binaries.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"complete": True, "programs": called,
                      "elf_hashes": {b["program"]: b.get("elf_sha256") or b.get("parent", {}).get("elf_sha256")
                                     for b in binaries if not b.get("native")}}))


if __name__ == "__main__":
    main()
