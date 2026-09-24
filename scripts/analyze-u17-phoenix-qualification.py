#!/usr/bin/env python3
"""Recheck the retained U17 Phoenix RPC receipts without an RPC connection.

The current account reads below are deliberately not treated as historical
transaction-boundary account receipts.
"""

import base64
import gzip
import hashlib
import json
from pathlib import Path
import struct
import subprocess


ROOT = Path(__file__).resolve().parents[1] / "docs/examples/phase-u17-phoenix-qualification"
PROGRAM = "PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY"
SIGNATURE = "rR1tBf4kb4xocTPpLZXRPqFFtvXQHn2GwAEgHbU8d83URqJNWnACGcz6cJfq27kGYhuWaTjrce7ksBNAF5NZVQC"
SLOT = 450026714
INDEX = 95
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def b58decode(value):
    n = 0
    for character in value:
        n = n * 58 + ALPHABET.index(character)
    return b"\x00" * (len(value) - len(value.lstrip("1"))) + n.to_bytes(
        (n.bit_length() + 7) // 8, "big"
    )


def b58encode(value):
    n = int.from_bytes(value, "big")
    result = ""
    while n:
        n, remainder = divmod(n, 58)
        result = ALPHABET[remainder] + result
    return "1" * (len(value) - len(value.lstrip(b"\x00"))) + result


def read(name):
    path = ROOT / name
    data = path.read_bytes()
    if name.endswith(".gz"):
        data = gzip.decompress(data)
    response = json.loads(data)
    assert response.get("error") is None and response.get("result") is not None, name
    return response, sha(data)


def account_data(response):
    value = response["result"]["value"]
    assert value is not None
    encoded, encoding = value["data"]
    raw = base64.b64decode(encoded)
    if encoding == "base64+zstd":
        raw = subprocess.run(
            ["zstd", "-d", "-q", "--stdout"], input=raw, capture_output=True, check=True
        ).stdout
    else:
        assert encoding == "base64"
    return value, raw


def main():
    names = (
        "transaction.json",
        "block-accounts.json.gz",
        "openbook-signatures.json",
        "program-current.json",
        "programdata-header-current.json",
        "programdata-current.json",
        "deploy-block-time.json",
        "market-header-current.json",
    )
    responses = {name: read(name) for name in names}
    assert len(responses["openbook-signatures.json"][0]["result"]) == 10
    tx = responses["transaction.json"][0]["result"]
    block = responses["block-accounts.json.gz"][0]["result"]
    assert tx["slot"] == SLOT and tx["version"] == "legacy"
    assert tx["transaction"]["signatures"][0] == SIGNATURE and tx["meta"]["err"] is None
    assert block["parentSlot"] == SLOT - 1
    transactions = block["transactions"]
    assert transactions[INDEX]["transaction"]["signatures"][0] == SIGNATURE

    message = tx["transaction"]["message"]
    keys = message["accountKeys"]
    assert message.get("addressTableLookups", []) == []
    outer = message["instructions"]
    assert len(outer) == 6 and keys[outer[5]["programIdIndex"]] == PROGRAM
    assert all(keys[ix["programIdIndex"]] != PROGRAM for ix in outer[:5])
    role_indices = outer[5]["accounts"]
    assert len(role_indices) == 10
    roles = [keys[index] for index in role_indices]
    assert roles[0] == PROGRAM and roles[9] == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    data = b58decode(outer[5]["data"])
    assert len(data) == 41
    opcode, packet_variant, side, price_ticks, base_lots, self_trade, match_limit = struct.unpack_from(
        "<BBBQQBB", data
    )
    client_order_id = int.from_bytes(data[21:37], "little")
    assert (opcode, packet_variant, side, price_ticks, base_lots, self_trade, match_limit) == (
        2, 1, 1, 89868, 3950, 1, 0
    )
    assert client_order_id == 73223 and data[37:] == bytes(4)

    target_meta = transactions[INDEX]["transaction"]["accountKeys"]
    assert [item["pubkey"] for item in target_meta] == keys
    input_keys = set(keys)
    target_writable = {item["pubkey"] for item in target_meta if item["writable"]}
    before = []
    after = []
    for index, item in enumerate(transactions):
        if index == INDEX:
            continue
        writable = {a["pubkey"] for a in item["transaction"]["accountKeys"] if a["writable"]}
        overlap = writable & (input_keys if index < INDEX else target_writable)
        if overlap:
            (before if index < INDEX else after).append(
                {"index": index, "signature": item["transaction"]["signatures"][0], "accounts": sorted(overlap)}
            )
    assert not before and not after

    program_response = responses["program-current.json"][0]
    program, program_bytes = account_data(program_response)
    assert len(program_bytes) == 36 and struct.unpack_from("<I", program_bytes)[0] == 2
    programdata_address = b58encode(program_bytes[4:36])
    pd_header_response = responses["programdata-header-current.json"][0]
    pd_header_value, pd_header = account_data(pd_header_response)
    pd_response = responses["programdata-current.json"][0]
    pd_value, pd_bytes = account_data(pd_response)
    assert pd_bytes[: len(pd_header)] == pd_header
    assert len(pd_bytes) == pd_value["space"] == 5000045
    assert struct.unpack_from("<I", pd_bytes)[0] == 3
    deploy_slot = struct.unpack_from("<Q", pd_bytes, 4)[0]
    assert deploy_slot == 231433470 and pd_bytes[45:49] == b"\x7fELF"
    assert responses["deploy-block-time.json"][0]["result"] == 1700601143
    assert not any(
        programdata_address in {a["pubkey"] for a in item["transaction"]["accountKeys"] if a["writable"]}
        for item in transactions
    )

    market_response = responses["market-header-current.json"][0]
    market, market_bytes = account_data(market_response)
    assert market["owner"] == PROGRAM and market["space"] == 445536
    assert struct.unpack_from("<Q", market_bytes)[0] == 8167313896524341111
    assert b58encode(market_bytes[48:80]) == "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh"
    assert b58encode(market_bytes[80:112]) == roles[7]
    assert b58encode(market_bytes[128:160]) == "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
    assert b58encode(market_bytes[160:192]) == roles[8]

    token_deltas = {}
    pre = {row["accountIndex"]: row for row in tx["meta"]["preTokenBalances"]}
    post = {row["accountIndex"]: row for row in tx["meta"]["postTokenBalances"]}
    assert pre.keys() == post.keys()
    for index in sorted(pre):
        assert pre[index]["mint"] == post[index]["mint"]
        token_deltas[keys[index]] = int(post[index]["uiTokenAmount"]["amount"]) - int(
            pre[index]["uiTokenAmount"]["amount"]
        )
    assert token_deltas[roles[5]] == -395000 and token_deltas[roles[7]] == 395000
    assert token_deltas[roles[6]] == token_deltas[roles[8]] == 0

    result = {
        "kind": "u17_phoenix_orderbook_qualification",
        "evidence_response_sha256": {name: responses[name][1] for name in names},
        "signature": SIGNATURE,
        "slot": SLOT,
        "index": INDEX,
        "parent_slot": block["parentSlot"],
        "blockhash": block["blockhash"],
        "block_transactions": len(transactions),
        "target_version": tx["version"],
        "target_success": True,
        "outer_instruction_programs": [keys[ix["programIdIndex"]] for ix in outer],
        "phoenix_outer_index": 5,
        "phoenix_roles": roles,
        "phoenix_data_hex": data.hex(),
        "decoded_request": {
            "variant": "limit_ask",
            "price_ticks": price_ticks,
            "base_lots": base_lots,
            "self_trade_behavior": "cancel_provide",
            "client_order_id": client_order_id,
            "use_only_deposited_funds": False,
        },
        "token_base_unit_deltas": token_deltas,
        "earlier_writers_to_target_inputs": before,
        "later_writers_to_target_outputs": after,
        "programdata_address_current": programdata_address,
        "programdata_last_deploy_slot_current": deploy_slot,
        "programdata_account_sha256_current": sha(pd_bytes),
        "elf_sha256_current": sha(pd_bytes[45:]),
        "current_account_contexts": {
            "program": program_response["result"]["context"]["slot"],
            "programdata_header": pd_header_response["result"]["context"]["slot"],
            "programdata": pd_response["result"]["context"]["slot"],
            "market_header": market_response["result"]["context"]["slot"],
        },
        "historical_target_account_receipts_acquired": False,
        "historical_replay_attempted": False,
    }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
