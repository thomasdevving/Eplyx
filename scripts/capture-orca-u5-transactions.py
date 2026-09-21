#!/usr/bin/env python3
"""One-attempt capture of the frozen U5 Orca sample; no replay admission."""

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
    assert len(sample["selection"]) == 200
    assert hashlib.sha256(canonical(sample["selection"])).hexdigest() == sample["sample_fingerprint"]
    for subdir in ("requests", "transactions", "receipts"):
        (ROOT / subdir).mkdir(exist_ok=True)
    for index, selected in enumerate(sample["selection"]):
        name = f"{index:03d}"
        request_path = ROOT / "requests" / f"{name}.json"
        body_path = ROOT / "transactions" / f"{name}.body"
        receipt_path = ROOT / "receipts" / f"{name}.json"
        if receipt_path.exists():
            continue
        request = {
            "jsonrpc": "2.0", "id": 1, "method": "getTransaction",
            "params": [selected["signature"], {
                "encoding": "json", "commitment": "finalized",
                "maxSupportedTransactionVersion": 0,
            }],
        }
        request_path.write_bytes(canonical(request) + b"\n")
        started = time.monotonic()
        process = subprocess.run([
            "curl", "--silent", "--show-error", "--max-time", "30",
            "--max-redirs", "0", "--header", "Content-Type: application/json",
            "--request", "POST", "--data", canonical(request).decode(),
            "--output", str(body_path), "--write-out", "%{http_code}", ENDPOINT,
        ], capture_output=True)
        body = body_path.read_bytes() if body_path.exists() else b""
        try:
            response = json.loads(body)
        except (ValueError, UnicodeError):
            response = None
        value = response.get("result") if isinstance(response, dict) else None
        receipt = {
            "index": index, "signature": selected["signature"],
            "requested_slot": selected["slot"], "endpoint": ENDPOINT,
            "curl_exit": process.returncode,
            "http_status": process.stdout.decode(errors="replace").strip(),
            "stderr": process.stderr.decode(errors="replace")[:300],
            "elapsed_seconds": round(time.monotonic() - started, 3),
            "body_bytes": len(body), "body_sha256": hashlib.sha256(body).hexdigest(),
            "rpc_error": response.get("error") if isinstance(response, dict) else None,
            "returned_slot": value.get("slot") if isinstance(value, dict) else None,
            "returned_signature": value.get("transaction", {}).get("signatures", [None])[0]
            if isinstance(value, dict) else None,
            "returned_version": value.get("version") if isinstance(value, dict) else None,
        }
        receipt["valid_envelope"] = (
            receipt["curl_exit"] == 0 and receipt["http_status"] == "200"
            and receipt["rpc_error"] is None
            and receipt["returned_slot"] == receipt["requested_slot"]
            and receipt["returned_signature"] == receipt["signature"]
        )
        receipt_path.write_bytes(canonical(receipt) + b"\n")
        if (index + 1) % 20 == 0:
            print(f"captured {index + 1}/200; valid={receipt['valid_envelope']}", flush=True)
        time.sleep(0.05)


if __name__ == "__main__":
    main()
