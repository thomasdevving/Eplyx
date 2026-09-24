#!/usr/bin/env python3
"""Decode fixed Drift IDL fields from the frozen U13.3 target boundary."""
import functools
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
IDL = json.loads((ROOT / "docs/examples/phase-u13-drift-witness/source/drift.json").read_text())
TYPES = {entry["name"]: entry["type"] for entry in IDL["types"] + IDL["accounts"]}
ACCOUNTS_PATH = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "docs/examples/phase-u14-drift-semantics/target-boundary.json"
ACCOUNTS = json.loads(ACCOUNTS_PATH.read_text())


@functools.lru_cache(None)
def named_size(name):
    kind = TYPES[name]["kind"]
    if kind == "enum":
        return 1 + max((sum(size(x.get("type", x)) for x in variant.get("fields", [])) for variant in TYPES[name]["variants"]), default=0)
    return sum(size(field["type"]) for field in TYPES[name]["fields"])


def size(typ):
    if isinstance(typ, str):
        return {"publicKey": 32, "bool": 1, "u8": 1, "i8": 1, "u16": 2, "i16": 2,
                "u32": 4, "i32": 4, "u64": 8, "i64": 8, "u128": 16, "i128": 16}[typ]
    if "array" in typ:
        element, count = typ["array"]
        return size(element) * count
    if "defined" in typ:
        return named_size(typ["defined"])
    if "option" in typ:
        return 1 + size(typ["option"])
    raise ValueError(typ)


def leaves(typ, offset, path):
    if isinstance(typ, str):
        yield path, offset, size(typ), typ
    elif "defined" in typ:
        definition = TYPES[typ["defined"]]
        if definition["kind"] == "enum":
            yield path, offset, size(typ), "enum"
        else:
            for field in definition["fields"]:
                yield from leaves(field["type"], offset, path + "." + field["name"])
                offset += size(field["type"])
    elif "array" in typ:
        element, count = typ["array"]
        if element == "u8":
            yield path, offset, count, "bytes"
        else:
            for index in range(count):
                yield from leaves(element, offset + index * size(element), f"{path}[{index}]")
    else:
        raise ValueError(typ)


for role, entry in ACCOUNTS.items():
    name = {"user": "User", "perp_market": "PerpMarket", "spot_market": "SpotMarket"}[role]
    pre = bytes.fromhex(entry["pre"]["data"])
    post = bytes.fromhex(entry["post"]["data"])
    assert len(pre) == len(post) == named_size(name) + 8
    print(f"\n{name}, {len(pre)} bytes")
    offset = 8
    for field in TYPES[name]["fields"]:
        for path, start, count, typ in leaves(field["type"], offset, field["name"]):
            before, after = pre[start:start + count], post[start:start + count]
            if before != after:
                if typ == "bytes" or typ == "publicKey":
                    old, new = before.hex(), after.hex()
                else:
                    signed = typ.startswith("i")
                    old, new = int.from_bytes(before, "little", signed=signed), int.from_bytes(after, "little", signed=signed)
                print(f"{start:4} {path:65} {old} -> {new}")
        offset += size(field["type"])
