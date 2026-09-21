#!/usr/bin/env python3
"""Supplemental full-block diagnostic for the fixed U5 signature set."""

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
    sample = json.loads((ROOT / "sample.json").read_bytes())
    assert sample["sample_fingerprint"] == "f77749d7a7992253e15f0df34e3a8f574998cc8868586884ec987f30423712c3"
    slots = sorted({row["slot"] for row in sample["selection"]}, reverse=True)
    assert len(slots) == 22
    (ROOT / "blocks").mkdir(exist_ok=True)
    (ROOT / "block-receipts").mkdir(exist_ok=True)
    for slot in slots:
        body_path = ROOT / "blocks" / f"{slot}.body"
        receipt_path = ROOT / "block-receipts" / f"{slot}.json"
        if receipt_path.exists():
            continue
        request = {"jsonrpc": "2.0", "id": 1, "method": "getBlock", "params": [slot, {
            "commitment": "finalized", "encoding": "json", "transactionDetails": "full",
            "maxSupportedTransactionVersion": 1, "rewards": False,
        }]}
        process = subprocess.run([
            "curl", "--silent", "--show-error", "--max-time", "60",
            "--max-redirs", "0", "--header", "Content-Type: application/json",
            "--request", "POST", "--data", canonical(request).decode(),
            "--output", str(body_path), "--write-out", "%{http_code}", ENDPOINT,
        ], capture_output=True)
        body = body_path.read_bytes() if body_path.exists() else b""
        try:
            response = json.loads(body)
        except (ValueError, UnicodeError):
            response = None
        result = response.get("result") if isinstance(response, dict) else None
        receipt = {
            "slot": slot, "endpoint": ENDPOINT, "request": request,
            "curl_exit": process.returncode,
            "http_status": process.stdout.decode(errors="replace").strip(),
            "stderr": process.stderr.decode(errors="replace")[:300],
            "body_bytes": len(body), "body_sha256": hashlib.sha256(body).hexdigest(),
            "rpc_error": response.get("error") if isinstance(response, dict) else None,
            "block_present": isinstance(result, dict),
            "transaction_count": len(result.get("transactions", [])) if isinstance(result, dict) else None,
        }
        receipt_path.write_bytes(canonical(receipt) + b"\n")
        print(f"slot={slot} http={receipt['http_status']} tx={receipt['transaction_count']}", flush=True)
        time.sleep(1.0)


if __name__ == "__main__":
    main()
