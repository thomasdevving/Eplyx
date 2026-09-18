#!/usr/bin/env python3
"""Prospective transaction-only capture. Policy must exist before this starts.

RPC URL and optional Origin are read from the environment and never serialized.
This command cannot finalize a classified sample; the offline rebuild does that.
"""
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import time
from urllib.parse import urlsplit

from kamino_u3_baseline import b58decode


def encoded(value):
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(encoded(value))
    temporary.replace(path)


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else "docs/examples/phase-u3-baseline")
    policy = json.loads((root / "sampling-policy.json").read_bytes())
    if (root / "fetch-membership.json").exists():
        raise ValueError("capture already started; never overwrite or silently resample")
    source_from = Path(sys.argv[2]) if len(sys.argv) > 2 else None
    if source_from is not None:
        for name, expected_hash in policy["frozen_source_hashes"].items():
            body = (source_from / name).read_bytes()
            if digest(body) != expected_hash:
                raise ValueError("fixed source hash differs from predeclared v2 policy")
            destination = root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source_from / name, destination)
    endpoint = os.environ["SOLANA_RPC_URL"]
    parsed = urlsplit(endpoint)
    if parsed.username or parsed.password or parsed.scheme != "https":
        raise ValueError("RPC must use HTTPS without URL userinfo")
    if f"{parsed.scheme}://{parsed.hostname}" != policy["provider"]:
        raise ValueError("RPC host differs from predeclared policy")
    origin = os.environ.get("SOLANA_RPC_ORIGIN", "")
    if any(c in endpoint + origin for c in '\r\n"\\'):
        raise ValueError("invalid curl configuration characters")

    def rpc(method, params):
        payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
        config = f'url = "{endpoint}"\nheader = "Content-Type: application/json"\n'
        if origin:
            config += f'header = "Origin: {origin}"\n'
        result = subprocess.run(
            ["curl", "--config", "-", "--silent", "--show-error", "--max-time",
             str(policy["fetch_rule"]["timeout_seconds"]), "--request", "POST", "--data", payload],
            input=config.encode(), capture_output=True,
            timeout=policy["fetch_rule"]["timeout_seconds"] + 3,
        )
        if result.returncode:
            return result.stdout or None, None, f"transport_exit_{result.returncode}"
        try:
            value = json.loads(result.stdout)
        except (ValueError, UnicodeError):
            return result.stdout, None, "invalid_json_response"
        if not isinstance(value, dict):
            return result.stdout, value, "invalid_rpc_envelope"
        if "error" in value:
            return result.stdout, value, f"rpc_error_{value['error'].get('code', 'unknown')}"
        if value.get("result") is None:
            return result.stdout, value, "null_result"
        return result.stdout, value, None

    requests = {}
    for name, method, params in [
        ("genesis", "getGenesisHash", []),
        ("anchor", "getSlot", [{"commitment": policy["commitment"]}]),
        ("source", "getSignaturesForAddress", [policy["program_id"], {
            "limit": policy["signature_limit"], "commitment": policy["commitment"]}]),
    ]:
        requests[name] = {"method": method, "params": params, "response_file": f"rpc/{name}.json"}
        if source_from is None:
            body, value, error = rpc(method, params)
        else:
            body = (root / f"rpc/{name}.json").read_bytes()
            value, error = json.loads(body), None
        if error:
            raise ValueError(f"{method} failed: {error}; policy unchanged, no sample finalized")
        path = root / requests[name]["response_file"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)
        if name == "genesis" and value["result"] != policy["expected_genesis_hash"]:
            raise ValueError("network genesis differs from pinned mainnet identity")

    anchor = json.loads((root / "rpc/anchor.json").read_bytes())["result"]
    listing = json.loads((root / "rpc/source.json").read_bytes())["result"]
    if not isinstance(listing, list) or len(listing) > policy["signature_limit"]:
        raise ValueError("invalid signature listing")
    signatures = [dict(item, source_index=index) for index, item in enumerate(listing)]
    if any(len(b58decode(item["signature"])) != 64 or type(item["slot"]) is not int or item["slot"] < 0 for item in signatures):
        raise ValueError("invalid source signature or slot; unsafe artifact path rejected")
    if len({item["signature"] for item in signatures}) != len(signatures):
        raise ValueError("duplicate signature in source listing")
    selected = [item["signature"] for item in signatures
                if anchor - 10000 <= item["slot"] <= anchor][:policy["transaction_fetch_limit"]]
    if len(selected) != policy["transaction_fetch_limit"]:
        raise ValueError("not enough source entries in pinned window; no replacement window")
    write(root / "source-signatures.json", signatures)
    membership = [dict(signature=item["signature"], source_index=item["source_index"],
                       selected=item["signature"] in selected, attempted=False,
                       status="pending" if item["signature"] in selected else "not_selected",
                       attempts=[], response_file=None, response_sha256=None, failure_reason=None)
                  for item in signatures]
    write(root / "fetch-membership.json", membership)
    write(root / "checkpoint.json", {"kind": "checkpoint", "complete": False, "resolved": 0})

    def fetch(item):
        item = dict(item, attempted=True)
        params = [item["signature"], policy["get_transaction_options"]]
        for number in range(1, policy["fetch_rule"]["max_attempts"] + 1):
            try:
                body, value, error = rpc("getTransaction", params)
            except subprocess.TimeoutExpired as timeout:
                body, value, error = timeout.stdout or None, None, "transport_timeout"
            reference = None
            if body is not None:
                suffix = "body" if value is None else "json"
                reference = (f"rpc/attempts/{item['signature']}-{number}.{suffix}" if error else
                             f"transactions/{item['signature']}.json")
                path = root / reference
                path.parent.mkdir(parents=True, exist_ok=True)
                if path.exists():
                    raise ValueError("duplicate transaction artifact; never overwrite")
                path.write_bytes(body)
            attempt = {"number": number, "error": error, "response_file": reference,
                       "response_sha256": digest(body) if body is not None else None}
            item["attempts"].append(attempt)
            if error is None:
                item.update(status="success", response_file=reference, response_sha256=digest(body), failure_reason=None)
                return item
            item.update(failure_reason=error, response_file=reference,
                        response_sha256=digest(body) if body is not None else None)
            if number < policy["fetch_rule"]["max_attempts"]:
                time.sleep(policy["fetch_rule"]["retry_delay_seconds"])
        item["status"] = "failure"
        return item

    resolved = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=policy["fetch_rule"]["workers"]) as pool:
        futures = {pool.submit(fetch, item): item["source_index"] for item in membership if item["selected"]}
        for future in concurrent.futures.as_completed(futures):
            membership[futures[future]] = future.result()
            resolved += 1
            write(root / "fetch-membership.json", membership)
            write(root / "checkpoint.json", {"kind": "checkpoint", "complete": False, "resolved": resolved})
            if resolved % 20 == 0:
                print(f"resolved {resolved}/{len(selected)}", flush=True)

    raw_paths = sorted(path for path in root.rglob("*") if path.is_file() and path.name != "checkpoint.json")
    receipt = {
        "kind": "capture_receipt", "fetch_complete": True,
        "policy_sha256": digest((root / "sampling-policy.json").read_bytes()),
        "source_sha256": digest((root / "source-signatures.json").read_bytes()),
        "provider": policy["provider"], "genesis_hash": policy["expected_genesis_hash"],
        "slot_start": anchor - 10000, "slot_end": anchor, "requests": requests,
        "get_transaction_params": ["<source signature>", policy["get_transaction_options"]],
        "raw_artifact_hashes": {str(path.relative_to(root)): digest(path.read_bytes()) for path in raw_paths},
    }
    write(root / "capture-receipt.json", receipt)
    (root / "checkpoint.json").unlink()
    print("Capture resolved; classification/finalization still required. No replay attempted.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, subprocess.TimeoutExpired) as error:
        # Never print transport exceptions containing endpoint credentials.
        print(f"capture stopped: {error}", file=sys.stderr)
        sys.exit(1)
