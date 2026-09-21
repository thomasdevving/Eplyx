#!/usr/bin/env python3
"""Acquire the bounded generic historical inputs identified by Phase U8."""

from __future__ import annotations

import base64
import hashlib
import json
import subprocess
import time
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "docs/examples/phase-u9-acquisition"
RAW = OUT / "raw"
ASSEMBLED = OUT / "assembled"
ENDPOINT = "https://solana-mainnet.g.alchemy.com/v2/docs-demo"
PROVIDER = "https://solana-mainnet.g.alchemy.com"
ORIGIN = "https://www.alchemy.com"
SLOT = 448760958
CHUNK = 1_048_576
UPGRADEABLE = "BPFLoaderUpgradeab1e11111111111111111111111"
LUT_OWNER = "AddressLookupTab1e1111111111111111111111111"
SYSVAR_OWNER = "Sysvar1111111111111111111111111111111111111"
FEATURE_PROGRAM = "Feature111111111111111111111111111111111111"
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

LUTS = [
    "8bnWtkhsFKXEXTETxMnTsMuyPfyP3JKRzcbRgQYvAqEd",
    "GqBzfLfkXqpJtEYp7VvuFkZ3SZLLQ3ApQThz9tNmdNfZ",
    "8qfVCNSiPkeDGBYXeqG6xUKsmSkKvAChbUC3ogAbPA3j",
    "FZSQhk9KTGz5wuCKTbYD3FWfEpVPXGLU4eeE2Av3YzVX",
]
PROGRAMDATA = {
    "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc": "CtXfPzz36dH5Ws4UYKZvrQ1Xqzn42ecDW6y8NKuiN8nD",
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA": "3gvYRKWyXRR9xKWe1ZjPhLY5ZJRN7KDB4rFZFGoJfFk2",
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb": "DoU57AYuPFu2QU514RktNPG22QhApEjnKxnBcu4BHDTY",
}
SYSVARS = {
    "Clock": "SysvarC1ock11111111111111111111111111111111",
    "Rent": "SysvarRent111111111111111111111111111111111",
    "EpochSchedule": "SysvarEpochSchedu1e111111111111111111111111",
    "RecentBlockhashes": "SysvarRecentB1ockHashes11111111111111111111",
    "SlotHashes": "SysvarS1otHashes111111111111111111111111111",
}


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def b58encode(value: bytes) -> str:
    number = int.from_bytes(value, "big")
    encoded = ""
    while number:
        number, remainder = divmod(number, 58)
        encoded = ALPHABET[remainder] + encoded
    return "1" * (len(value) - len(value.lstrip(b"\0"))) + encoded


def call(method: str, params: list[object], name: str) -> tuple[dict, dict]:
    payload = canonical({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    path = RAW / name
    if path.exists() and path.stat().st_size:
        cached = path.read_bytes()
        try:
            decoded = json.loads(cached)
        except Exception:
            decoded = {}
        if decoded.get("error") is None and "result" in decoded:
            return decoded, {
                "method": method, "params": params, "response_file": str(path.relative_to(REPO)),
                "response_bytes": len(cached), "response_sha256": sha(cached), "http_status": 200,
                "elapsed_seconds": 0.0, "transport_error": None, "rpc_error": None, "reused": True,
            }
    failure = None
    raw = b""
    status = None
    started = time.monotonic()
    for attempt in range(1, 5):
        process = subprocess.run(
            [
                "curl", "--silent", "--show-error", "--max-time", "30",
                ENDPOINT, "-H", "Content-Type: application/json", "-H", f"Origin: {ORIGIN}",
                "--data-binary", "@-", "--write-out", "\nEPSTATUS:%{http_code}",
            ],
            input=payload,
            capture_output=True,
            check=False,
        )
        body, marker, code = process.stdout.rpartition(b"\nEPSTATUS:")
        if marker:
            raw = body
            status = int(code)
        failure = process.stderr.decode(errors="replace").strip() or None
        if process.returncode == 0 and marker and status == 200:
            failure = None
            break
        if attempt < 4:
            time.sleep(5.0 * attempt if status == 429 else 0.5 * attempt)
    if status == 200 and raw:
        path.write_bytes(raw)
    try:
        decoded = json.loads(raw)
    except Exception:
        decoded = {}
    receipt = {
        "method": method,
        "params": params,
        "response_file": str(path.relative_to(REPO)),
        "response_bytes": len(raw),
        "response_sha256": sha(raw),
        "http_status": status,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "transport_error": failure,
        "rpc_error": decoded.get("error"),
        "reused": False,
    }
    if failure or status != 200 or decoded.get("error"):
        raise RuntimeError(json.dumps(receipt, sort_keys=True))
    return decoded, receipt


def account_call(address: str, slot: int, name: str, data_slice: dict | None = None) -> tuple[dict, dict]:
    config: dict[str, object] = {"encoding": "base64", "commitment": "finalized", "slot": slot}
    if data_slice is not None:
        config["dataSlice"] = data_slice
    response, receipt = call("getAccountInfo", [address, config], name)
    result = response["result"]
    receipt.update(address=address, requested_slot=slot, returned_context_slot=result["context"]["slot"])
    if result["context"]["slot"] != slot:
        raise RuntimeError(f"context mismatch for {address}: {result['context']['slot']} != {slot}")
    return result, receipt


def account_facts(result: dict, expected_owner: str) -> tuple[dict, bytes]:
    account = result["value"]
    if account is None or account["owner"] != expected_owner or account["data"][1] != "base64":
        raise RuntimeError(f"missing account or owner mismatch: expected {expected_owner}")
    data = base64.b64decode(account["data"][0], validate=True)
    return ({
        "owner": account["owner"],
        "lamports": account["lamports"],
        "executable": account["executable"],
        "rent_epoch": account["rentEpoch"],
        "space": account.get("space", len(data)),
        "returned_data_bytes": len(data),
        "data_sha256": sha(data),
    }, data)


def main() -> None:
    RAW.mkdir(parents=True, exist_ok=True)
    ASSEMBLED.mkdir(parents=True, exist_ok=True)
    receipts = []
    genesis, receipt = call("getGenesisHash", [], "genesis.json")
    receipts.append(receipt)
    if genesis.get("result") != "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d":
        raise RuntimeError("not Solana mainnet")

    lut_rows = []
    for address in LUTS:
        observations = []
        for slot in (SLOT - 1, SLOT):
            result, receipt = account_call(address, slot, f"lut-{slot}-{address}.json")
            receipts.append(receipt)
            facts, data = account_facts(result, LUT_OWNER)
            observations.append({"slot": slot, **facts})
            (ASSEMBLED / f"lut-{slot}-{address}.bin").write_bytes(data)
        if observations[0]["data_sha256"] != observations[1]["data_sha256"]:
            raise RuntimeError(f"LUT changed inside target slot: {address}")
        lut_rows.append({"address": address, "observations": observations, "s_minus_1_equals_s": True})
        print(json.dumps({"kind": "lut", "address": address, "bytes": observations[0]["space"]}), flush=True)

    program_rows = []
    for program_id, address in PROGRAMDATA.items():
        headers = []
        for slot in (SLOT - 1, SLOT):
            result, receipt = account_call(address, slot, f"programdata-header-{slot}-{address}.json", {"offset": 0, "length": 45})
            receipts.append(receipt)
            facts, header = account_facts(result, UPGRADEABLE)
            if len(header) != 45 or int.from_bytes(header[:4], "little") != 3:
                raise RuntimeError(f"invalid ProgramData header: {address}")
            headers.append({"slot": slot, **facts, "header_sha256": sha(header), "header": header})
        if headers[0]["header_sha256"] != headers[1]["header_sha256"]:
            raise RuntimeError(f"ProgramData header changed inside target slot: {address}")
        size = headers[0]["space"]
        chunks = []
        assembled = bytearray()
        for offset in range(0, size, CHUNK):
            length = min(CHUNK, size - offset)
            result, receipt = account_call(address, SLOT - 1, f"programdata-{SLOT - 1}-{address}-{offset}-{length}.json", {"offset": offset, "length": length})
            receipts.append(receipt)
            facts, data = account_facts(result, UPGRADEABLE)
            if len(data) != length or facts["space"] != size:
                raise RuntimeError(f"ProgramData chunk mismatch: {address} {offset}")
            assembled.extend(data)
            chunks.append({"offset": offset, "length": length, "data_sha256": sha(data), "response_sha256": receipt["response_sha256"], "response_file": receipt["response_file"]})
        raw = bytes(assembled)
        if len(raw) != size or raw[:45] != headers[0]["header"]:
            raise RuntimeError(f"ProgramData reassembly mismatch: {address}")
        deployment_slot = int.from_bytes(raw[4:12], "little")
        authority = None if raw[12] == 0 else b58encode(raw[13:45])
        if raw[12] not in (0, 1):
            raise RuntimeError(f"invalid ProgramData authority flag: {address}")
        elf = raw[45:]
        if not elf.startswith(b"\x7fELF"):
            raise RuntimeError(f"ProgramData ELF magic missing: {address}")
        account_path = ASSEMBLED / f"programdata-{address}.bin"
        elf_path = ASSEMBLED / f"program-{program_id}.so"
        account_path.write_bytes(raw)
        elf_path.write_bytes(elf)
        program_rows.append({
            "program_id": program_id,
            "programdata_address": address,
            "requested_boundary": SLOT - 1,
            "s_minus_1_header_equals_s_header": True,
            "owner": UPGRADEABLE,
            "lamports": headers[0]["lamports"],
            "executable": headers[0]["executable"],
            "rent_epoch": headers[0]["rent_epoch"],
            "allocated_account_length": size,
            "deployment_slot": deployment_slot,
            "upgrade_authority": authority,
            "account_sha256": sha(raw),
            "elf_bytes": len(elf),
            "elf_sha256": sha(elf),
            "account_file": str(account_path.relative_to(REPO)),
            "elf_file": str(elf_path.relative_to(REPO)),
            "chunks": chunks,
        })
        print(json.dumps({"kind": "programdata", "program_id": program_id, "bytes": size, "chunks": len(chunks)}), flush=True)

    sysvar_rows = []
    for name, address in SYSVARS.items():
        result, receipt = account_call(address, SLOT, f"sysvar-{SLOT}-{name}.json")
        receipts.append(receipt)
        if name in ("RecentBlockhashes", "SlotHashes"):
            sysvar_rows.append({"name": name, "address": address, "slot": SLOT, "archive_value": "null" if result["value"] is None else "present"})
            if result["value"] is not None:
                raise RuntimeError(f"unexpected archive value for excluded per-slot sysvar {name}")
        else:
            facts, data = account_facts(result, SYSVAR_OWNER)
            path = ASSEMBLED / f"sysvar-{name}.bin"
            path.write_bytes(data)
            sysvar_rows.append({"name": name, "address": address, "slot": SLOT, **facts, "account_file": str(path.relative_to(REPO))})
        print(json.dumps({"kind": "sysvar", "name": name, "status": sysvar_rows[-1].get("archive_value", "present")}), flush=True)

    feature_response, receipt = call(
        "getProgramAccounts",
        [FEATURE_PROGRAM, {"encoding": "base64", "commitment": "finalized", "withContext": True}],
        "features-current.json",
    )
    receipts.append(receipt)
    feature_result = feature_response["result"]
    feature_rows = []
    for row in feature_result["value"]:
        data = base64.b64decode(row["account"]["data"][0], validate=True)
        if row["account"]["owner"] != FEATURE_PROGRAM:
            raise RuntimeError(f"feature owner mismatch: {row['pubkey']}")
        activation = None
        if data:
            if len(data) != 9 or data[0] not in (0, 1):
                raise RuntimeError(f"feature layout mismatch: {row['pubkey']}")
            activation = int.from_bytes(data[1:9], "little") if data[0] == 1 else None
        feature_rows.append({"feature_id": row["pubkey"], "activation_slot": activation, "data_sha256": sha(data)})
    version_response, receipt = call("getVersion", [], "version-current.json")
    receipts.append(receipt)
    recent_tail_block, receipt = call(
        "getBlock",
        [SLOT - 149, {"commitment": "finalized", "transactionDetails": "none", "rewards": False}],
        f"recent-blockhash-tail-{SLOT - 149}.json",
    )
    receipts.append(receipt)

    retained_responses = {(REPO / receipt["response_file"]).resolve() for receipt in receipts}
    superseded = []
    for path in sorted(RAW.iterdir()):
        if path.is_file() and path.resolve() not in retained_responses:
            superseded.append(str(path.relative_to(REPO)))
            path.unlink()

    acquisition = {
        "schema": "eplyx.phase-u9.acquisition.v1",
        "provider": {
            "scheme_host": PROVIDER,
            "endpoint_class": "public documentation endpoint",
            "credential_retained": False,
            "origin": ORIGIN,
            "genesis_hash": genesis["result"],
            "historical_semantics": "point-in-time state as of requested finalized slot, inclusive",
            "documentation": "https://www.alchemy.com/docs/solana/account-archive",
        },
        "target_slot": SLOT,
        "checkpoint_a": SLOT - 1,
        "luts": lut_rows,
        "programdata": program_rows,
        "sysvars": sysvar_rows,
        "feature_snapshot": {
            "observation_context_slot": feature_result["context"]["slot"],
            "historical_target_slot": SLOT,
            "accounts": sorted(feature_rows, key=lambda row: row["feature_id"]),
            "target_active_count": sum(row["activation_slot"] is not None and row["activation_slot"] <= SLOT for row in feature_rows),
            "activated_after_target_count": sum(row["activation_slot"] is not None and row["activation_slot"] > SLOT for row in feature_rows),
            "limitation": "Current finalized feature accounts expose activation slots but do not prove the validator binary family at the historical slot.",
        },
        "current_rpc_version_observation": version_response["result"],
        "recent_blockhash_tail_anchor": {
            "slot": SLOT - 149,
            "blockhash": recent_tail_block["result"]["blockhash"],
            "previous_blockhash": recent_tail_block["result"]["previousBlockhash"],
            "parent_slot": recent_tail_block["result"]["parentSlot"],
            "response_file": str((RAW / f"recent-blockhash-tail-{SLOT - 149}.json").relative_to(REPO)),
        },
        "receipts": receipts,
        "superseded_raw_responses_removed": len(superseded),
        "complete": True,
    }
    (OUT / "acquisition.json").write_bytes(canonical(acquisition))
    checksums = {
        str(path.relative_to(OUT)): {"bytes": path.stat().st_size, "sha256": sha(path.read_bytes())}
        for path in sorted(OUT.rglob("*")) if path.is_file() and path.name != "checksums.json"
    }
    (OUT / "checksums.json").write_bytes(canonical(checksums))
    print(json.dumps({"complete": True, "luts": len(lut_rows), "programdata": len(program_rows), "sysvars": len(sysvar_rows), "responses": len(receipts)}, indent=2))


if __name__ == "__main__":
    main()
