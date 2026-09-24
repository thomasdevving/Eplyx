#!/usr/bin/env python3
"""Acquire exact-slot raw Phoenix transaction account receipts without semantic decoding."""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "docs/examples/phase-u17-phoenix-qualification/transaction.json"
PUBLIC_ARCHIVE = "https://solana-mainnet.g.alchemy.com/v2/docs-demo"
PARENT = 450026713
TARGET = 450026714
GENESIS = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def fetch(endpoint, out, method, params):
    request = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    body = json.dumps(request, separators=(",", ":")).encode()
    if out.exists():
        cached = out.read_bytes()
        try:
            parsed = json.loads(cached)
            if parsed.get("result") is not None and not parsed.get("error"):
                return parsed, cached
        except (ValueError, UnicodeDecodeError):
            pass
    last = None
    for attempt in range(6):
        result = subprocess.run(
            ["curl", "-sS", "--max-time", "40", "-X", "POST", endpoint,
             "-H", "Content-Type: application/json", "-H", "Origin: https://www.alchemy.com",
             "--data-binary", "@-", "-o", str(out), "-w", "%{http_code}"],
            input=body, capture_output=True,
        )
        status = result.stdout.decode(errors="replace").strip()
        raw = out.read_bytes() if out.exists() else b""
        try:
            parsed = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            parsed = None
        if result.returncode == 0 and status == "200" and parsed and not parsed.get("error"):
            return parsed, raw
        last = {"http": status, "rpc": (parsed or {}).get("error"),
                "curl_exit": result.returncode}
        if attempt < 5:
            time.sleep((2, 5, 12, 25, 50)[attempt] if status == "429" else 0.4 * (2 ** attempt))
    raise RuntimeError(f"{out.name}: archive request failed: {last}")


def account(endpoint, out, address, slot):
    params = [address, {"encoding": "base64", "commitment": "finalized", "slot": slot}]
    parsed, raw = fetch(endpoint, out, "getAccountInfo", params)
    result = parsed["result"]
    if result["context"]["slot"] != slot:
        raise RuntimeError(f"{out.name}: context slot differs")
    value = result["value"]
    row = {"address": address, "slot": slot, "receipt": out.name,
           "response_sha256": sha(raw), "present": value is not None}
    if value is None:
        return row
    if value["data"][1] != "base64":
        raise RuntimeError(f"{out.name}: wrong data encoding")
    data = base64.b64decode(value["data"][0], validate=True)
    if len(data) != value["space"]:
        raise RuntimeError(f"{out.name}: incomplete account data")
    row.update({"owner": value["owner"], "lamports": value["lamports"],
                "executable": value["executable"], "rent_epoch": value["rentEpoch"],
                "space": value["space"], "data_sha256": sha(data)})
    return row


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    endpoint = os.environ.get("SOLANA_ARCHIVE_RPC_URL") or os.environ.get("SOLANA_RPC_URL") or PUBLIC_ARCHIVE
    host = urlsplit(endpoint).hostname
    if not host:
        raise RuntimeError("archive endpoint is invalid")
    tx = json.loads(SOURCE.read_bytes())["result"]
    message = tx["transaction"]["message"]
    keys = message["accountKeys"]
    header = message["header"]
    signed = header["numRequiredSignatures"]
    writable_unsigned_end = len(keys) - header["numReadonlyUnsignedAccounts"]
    writable = [key for i, key in enumerate(keys)
                if (i < signed and i < signed - header["numReadonlySignedAccounts"])
                or (i >= signed and i < writable_unsigned_end)]
    if len(keys) != 18 or len(writable) != 9 or tx["slot"] != TARGET:
        raise RuntimeError("frozen transaction shape changed")
    raw_dir = args.out / "raw"
    raw_dir.mkdir(parents=True, exist_ok=True)
    genesis, raw = fetch(endpoint, raw_dir / "genesis.json", "getGenesisHash", [])
    if genesis["result"] != GENESIS:
        raise RuntimeError("archive is not Solana mainnet")
    rows = []
    for slot, addresses in [(PARENT, keys), (TARGET, writable)]:
        for index, address in enumerate(addresses):
            name = f"account-{slot}-{address}.json"
            rows.append(account(endpoint, raw_dir / name, address, slot))
            print(json.dumps({"slot": slot, "index": index, "present": rows[-1]["present"],
                              "space": rows[-1].get("space")}), flush=True)
            time.sleep(0.15)
    manifest = {"schema": "U17_2PhoenixStateAcquisitionV1", "provider_host": host,
                "genesis_sha256": sha(raw), "parent_slot": PARENT, "target_slot": TARGET,
                "message_keys": keys, "writable_keys": writable, "receipts": rows}
    (args.out / "acquisition.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"complete": True, "receipts": len(rows),
                      "absent_pre": [r["address"] for r in rows if r["slot"] == PARENT and not r["present"]]}))


if __name__ == "__main__":
    main()
