#!/usr/bin/env python3
"""Capture accounts-mode blocks for every observed direct swap in the frozen sample."""

import hashlib
import json
from pathlib import Path
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1] / "docs/examples/phase-u5-sample"
ENDPOINT = "https://api.mainnet-beta.solana.com"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def main():
    report = json.loads((ROOT / "classification.json").read_bytes())
    assert report["summary"]["sample_fingerprint"] == "f77749d7a7992253e15f0df34e3a8f574998cc8868586884ec987f30423712c3"
    slots = sorted({row["slot"] for row in report["rows"] if row.get("outer_orca")}, reverse=True)
    assert slots == [448760958, 448760957, 448760956, 448760955, 448760954, 448760952]
    (ROOT / "screen-blocks").mkdir(exist_ok=True)
    (ROOT / "screen-receipts").mkdir(exist_ok=True)
    for slot in slots:
        body_path = ROOT / "screen-blocks" / f"{slot}.body"
        receipt_path = ROOT / "screen-receipts" / f"{slot}.json"
        if receipt_path.exists():
            continue
        if slot == 448760958:
            body = (ROOT / "first-candidate-screen-block.body").read_bytes()
            body_path.write_bytes(body)
            status, process_exit, stderr = "200", 0, ""
        else:
            request = {"jsonrpc": "2.0", "id": 1, "method": "getBlock", "params": [slot, {
                "encoding": "json", "transactionDetails": "accounts", "rewards": False,
                "commitment": "finalized", "maxSupportedTransactionVersion": 1,
            }]}
            process = subprocess.run([
                "curl", "--silent", "--show-error", "--max-time", "60", "--max-redirs", "0",
                "--header", "Content-Type: application/json", "--request", "POST",
                "--data", canonical(request).decode(), "--output", str(body_path),
                "--write-out", "%{http_code}", ENDPOINT,
            ], capture_output=True)
            body = body_path.read_bytes() if body_path.exists() else b""
            status = process.stdout.decode(errors="replace").strip()
            process_exit = process.returncode
            stderr = process.stderr.decode(errors="replace")[:300]
        try:
            response = json.loads(body)
        except (ValueError, UnicodeError):
            response = None
        result = response.get("result") if isinstance(response, dict) else None
        receipt = {"slot": slot, "endpoint": ENDPOINT, "curl_exit": process_exit,
                   "http_status": status, "stderr": stderr, "body_bytes": len(body),
                   "body_sha256": hashlib.sha256(body).hexdigest(),
                   "rpc_error": response.get("error") if isinstance(response, dict) else None,
                   "transaction_count": len(result.get("transactions", [])) if isinstance(result, dict) else None}
        receipt_path.write_bytes(canonical(receipt) + b"\n")
        print(f"slot={slot} http={status} tx={receipt['transaction_count']}", flush=True)
        time.sleep(1.0)


if __name__ == "__main__":
    main()
