#!/usr/bin/env python3
"""Capture exact-slot Account Archive responses without retaining credentials."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--key-file", type=Path, required=True)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--slot", type=int, required=True)
    args = parser.parse_args()
    key = args.key_file.read_text().strip()
    assert key and all(c.isalnum() or c in "-_" for c in key)
    endpoint = "https://solana-mainnet.g.alchemy.com/v2/" + key
    args.out.mkdir(parents=True, exist_ok=True)
    addresses = [a["address"] for a in json.loads(args.inventory.read_text())["accounts"]]
    receipts = []
    for slot in (args.slot - 1, args.slot):
        for address in addresses:
            request = {"jsonrpc": "2.0", "id": 1, "method": "getAccountInfo", "params": [address, {"encoding": "base64", "commitment": "finalized", "slot": slot}]}
            body_path = args.out / f"{slot}-{address}.body"
            with tempfile.NamedTemporaryFile(mode="w", prefix="eplyx-u7-rpc-", suffix=".json") as request_file:
                json.dump(request, request_file, separators=(",", ":"))
                request_file.flush()
                config = f'url = "{endpoint}"\nrequest = "POST"\nheader = "Content-Type: application/json"\ndata-binary = "@{request_file.name}"\noutput = "{body_path}"\n'
                started = time.monotonic()
                result = subprocess.run(["curl", "--silent", "--show-error", "--max-time", "25", "--config", "-", "--write-out", "%{http_code}"], input=config, text=True, capture_output=True)
            raw = body_path.read_bytes() if body_path.exists() else b""
            try:
                response = json.loads(raw)
            except Exception:
                response = {}
            context = response.get("result", {}).get("context", {}) if isinstance(response.get("result"), dict) else {}
            value = response.get("result", {}).get("value") if isinstance(response.get("result"), dict) else None
            row = {"address": address, "requested_slot": slot, "http_status": result.stdout.strip(), "curl_exit": result.returncode,
                   "context_slot": context.get("slot"), "present": value is not None if "result" in response else None,
                   "rpc_error": response.get("error"), "body_sha256": hashlib.sha256(raw).hexdigest(), "body_bytes": len(raw),
                   "elapsed_seconds": round(time.monotonic() - started, 3)}
            receipts.append(row)
            print(f"{slot} {address} http={row['http_status']} context={row['context_slot']} present={row['present']} error={bool(row['rpc_error'])}", flush=True)
    (args.out / "archive-receipts.json").write_text(json.dumps(receipts, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
