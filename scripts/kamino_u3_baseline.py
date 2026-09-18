#!/usr/bin/env python3
"""Offline U3A derivation and strict finalization; contains no RPC client.

The Rust example calls the unchanged engine. Python only preserves diagnostic
evidence, derives counts, and checks capture/classification provenance.
"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlsplit

REPO = Path(__file__).resolve().parents[1]
DEFAULT_SAMPLE = REPO / "docs/examples/phase-u3-baseline"
SCHEMA = 1
KLEND = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD"
SYSTEM = "11111111111111111111111111111111"
TOKENS = {"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
          "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"}
ACTION_NAMES = {
    bytes([129, 199, 4, 2, 222, 39, 26, 46]): ("depositReserveLiquidityAndObligationCollateral", "deposit"),
    bytes([216, 224, 191, 27, 204, 151, 102, 175]): ("depositReserveLiquidityAndObligationCollateralV2", "deposit"),
    bytes([121, 127, 18, 204, 73, 245, 225, 65]): ("borrowObligationLiquidity", "borrow"),
    bytes([161, 128, 143, 245, 171, 199, 194, 6]): ("borrowObligationLiquidityV2", "borrow"),
}
DERIVED = ("classifications.json", "supported-observations.json", "before-rejections.json",
           "summary.json", "manifest.json", "checksums.sha256")


def canonical(value):
    return (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=False) + "\n").encode()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return json.loads(path.read_bytes())


def unique(rows, field):
    result = {}
    for row in rows:
        key = row[field]
        require(key not in result, f"duplicate {field}: {key}")
        result[key] = row
    return result


def hygiene(value):
    text = canonical(value).decode()
    require(not re.search(r'"(?:api[_-]?key|rpc_url|access_token|secret)"\s*:', text, re.I),
            "credential field in artifact")
    for url in re.findall(r'https?://[^\s"<>]+', text):
        parsed = urlsplit(url)
        require(not parsed.username and not parsed.password and not parsed.query and
                not parsed.fragment and parsed.path in ("", "/"), "credential-bearing or full provider URL")


def b58decode(text):
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    number = 0
    for character in text:
        number = number * 58 + alphabet.index(character)
    return b"\0" * (len(text) - len(text.lstrip("1"))) + (
        number.to_bytes((number.bit_length() + 7) // 8, "big") if number else b"")


def instruction(raw, keys, outer_index, inner_index=None):
    def address(index):
        return keys[index] if isinstance(index, int) and 0 <= index < len(keys) else None
    try:
        data = b58decode(raw["data"])
        decode_error = None
    except (KeyError, ValueError, TypeError):
        data, decode_error = b"", "invalid_or_missing_base58_data"
    return {"outer_index": outer_index, "inner_index": inner_index,
            "program": address(raw.get("programIdIndex")), "data_base58": raw.get("data"),
            "data_hex": data.hex() if decode_error is None else None,
            "data_decode_error": decode_error, "account_indices": raw.get("accounts"),
            "accounts": [address(i) for i in raw.get("accounts", [])],
            "stack_height": raw.get("stackHeight")}


def transient_cases(result, keys, sequence):
    """Evidence of creation/closure, never historical account-byte proof."""
    meta = result.get("meta") or {}
    pre, post = meta.get("preBalances"), meta.get("postBalances")
    if meta.get("err") is not None or not isinstance(pre, list) or not isinstance(post, list):
        return []
    creations, closures = {}, {}
    for ix in sequence:
        data = bytes.fromhex(ix["data_hex"]) if ix["data_hex"] is not None else b""
        if ix["program"] == SYSTEM and len(data) == 52 and data[:4] == bytes(4) and len(ix["accounts"]) >= 2:
            creations.setdefault(ix["accounts"][1], []).append(ix)
        if ix["program"] in TOKENS and data == b"\x09" and len(ix["accounts"]) >= 3:
            closures.setdefault(ix["accounts"][0], []).append(ix)
    cases = []
    before_tokens = {item["accountIndex"] for item in meta.get("preTokenBalances") or []}
    after_tokens = {item["accountIndex"] for item in meta.get("postTokenBalances") or []}
    for account in sorted(creations.keys() & closures.keys()):
        if account not in keys:
            continue
        index = keys.index(account)
        if index >= min(len(pre), len(post)) or pre[index] != 0 or post[index] != 0:
            continue
        if index in before_tokens or index in after_tokens:
            continue
        position = lambda ix: (ix["outer_index"], -1 if ix["inner_index"] is None else ix["inner_index"])
        events = sorted([(position(ix), "create", ix) for ix in creations[account]] +
                        [(position(ix), "close", ix) for ix in closures[account]])
        create = None
        for _, kind, ix in events:
            if kind == "create":
                create = ix
            elif create is not None:
                cases.append({"account": account, "account_index": index,
                              "boundary_lamports": [0, 0], "boundary_token_entries": [False, False],
                              "create_instruction": create, "close_instruction": ix,
                              "diagnosis": "apparent_absent_created_closed_absent",
                              "historical_account_bytes_verified": False})
                create = None
    return cases


def classify(result, source, audit):
    message = result.get("transaction", {}).get("message", {})
    meta = result.get("meta") or {}
    static = message.get("accountKeys", [])
    loaded = meta.get("loadedAddresses") or {}
    writable, readonly = loaded.get("writable", []), loaded.get("readonly", [])
    keys = static + writable + readonly
    outer = [instruction(ix, keys, index) for index, ix in enumerate(message.get("instructions", []))]
    sequence = []
    groups = {group["index"]: group["instructions"] for group in meta.get("innerInstructions", []) or []}
    for ix in outer:
        sequence.append(ix)
        sequence.extend(instruction(raw, keys, ix["outer_index"], i)
                        for i, raw in enumerate(groups.get(ix["outer_index"], [])))
    klend = []
    for ix in outer:
        if ix["program"] != KLEND:
            continue
        tag = bytes.fromhex(ix["data_hex"] or "")[:8]
        known = ACTION_NAMES.get(tag)
        klend.append({"outer_index": ix["outer_index"], "discriminator_hex": tag.hex(),
                      "instruction_name": known[0] if known else None,
                      "supported_action_family": known[1] if known else None,
                      "data_length": len(bytes.fromhex(ix["data_hex"] or "")),
                      "account_count": len(ix["accounts"])})
    version = result.get("version", "legacy")
    if audit["normalization"] == "rejected" and version in (None, "legacy", 0):
        distribution = "unreadable"
    elif version in (None, "legacy"):
        distribution = "legacy"
    elif version == 0:
        distribution = "v0_with_lut_resolution" if len(writable) + len(readonly) else "v0_without_lut_resolution"
    else:
        distribution = "newer_unsupported"
    preserved_meta = {key: meta[key] for key in (
        "err", "loadedAddresses", "innerInstructions", "preBalances", "postBalances",
        "preTokenBalances", "postTokenBalances") if key in meta}
    return {"signature": source["signature"], "slot": source["slot"], "source_index": source["source_index"],
            "success": meta.get("err") is None, "transaction_error": meta.get("err"),
            "message_version": version, "message_distribution": distribution,
            "static_key_count": len(static), "lookup_table_count": len(message.get("addressTableLookups", [])),
            "loaded_writable_count": len(writable), "loaded_readonly_count": len(readonly),
            "top_level_programs": [ix["program"] for ix in outer],
            "contains_klend_top_level": any(ix["program"] == KLEND for ix in outer),
            "klend_instructions": klend, "top_level_instructions": outer,
            "companion_programs": sorted({ix["program"] for ix in outer if ix["program"] not in (KLEND, None)}),
            "rpc_evidence": {"message": message, "meta": preserved_meta},
            "transient_accounts": transient_cases(result, keys, sequence), "old_engine": audit}


def supported_rows(rows):
    return [{"signature": row["signature"], "slot": row["slot"], "source_index": row["source_index"],
             "outer_index": ix["outer_index"], "instruction_name": ix["instruction_name"],
             "family": ix["supported_action_family"], "success": row["success"],
             "message_distribution": row["message_distribution"],
             "raw_response_sha256": row["old_engine"]["capture_sha256"]}
            for row in rows for ix in row["klend_instructions"] if ix["supported_action_family"]]


def before_rows(rows):
    return [{"signature": row["signature"], "slot": row["slot"], "source_index": row["source_index"],
             "action_id": row["old_engine"]["action_id"], "success": row["success"],
             "recognized_actions": [ix for ix in row["klend_instructions"] if ix["supported_action_family"]],
             "message_version": row["message_version"], "lookup_table_count": row["lookup_table_count"],
             "loaded_address_count": row["loaded_writable_count"] + row["loaded_readonly_count"],
             "companion_programs": row["companion_programs"], "old_engine": row["old_engine"]}
            for row in rows if any(ix["supported_action_family"] for ix in row["klend_instructions"])]


def summarize(source, membership, rows):
    observations = supported_rows(rows)
    successful = [item for item in observations if item["success"]]
    before = before_rows(rows)
    counts = Counter(row["message_distribution"] for row in rows)
    return {"source_count": len(source), "fetch_selected": sum(item["selected"] for item in membership),
            "not_fetch_selected": sum(not item["selected"] for item in membership),
            "fetch_success": sum(item["status"] == "success" for item in membership),
            "fetch_failure": sum(item["status"] == "failure" for item in membership),
            "classified_rows": len(rows), "klend_top_level": sum(row["contains_klend_top_level"] for row in rows),
            "no_klend_top_level": sum(not row["contains_klend_top_level"] for row in rows),
            "successful_supported_actions": {family: sum(item["family"] == family for item in successful)
                                             for family in ("deposit", "borrow")},
            "all_status_recognized_actions": {family: sum(item["family"] == family for item in observations)
                                              for family in ("deposit", "borrow")},
            "supported_transactions_all_status": len(before),
            "supported_transactions_successful": sum(row["success"] for row in before),
            "successful_supported_lut_observations": sum(item["message_distribution"] == "v0_with_lut_resolution" for item in successful),
            "successful_supported_no_lut_observations": sum(item["message_distribution"] in ("legacy", "v0_without_lut_resolution") for item in successful),
            "message_versions": {key: counts[key] for key in ("legacy", "v0_without_lut_resolution", "v0_with_lut_resolution", "newer_unsupported", "unreadable")},
            "companion_program_transactions": dict(sorted(Counter(program for row in before for program in row["companion_programs"]).items())),
            "transient_account_cases": sum(len(row["transient_accounts"]) for row in rows),
            "transient_account_transactions": sum(bool(row["transient_accounts"]) for row in rows),
            "supported_first_rejections": dict(sorted(Counter(
                row["old_engine"]["first_blocker"]["detail"] if row["old_engine"]["first_blocker"] else "admitted; missing state (not acquired)"
                for row in before).items())),
            "supported_admission_accepted": sum(row["old_engine"]["admission"] == "accepted" for row in before),
            "production_replay_validated": 0}


def fingerprint(source, membership, raw_hashes, policy_hash):
    # Enumeration is irrelevant; explicit source_index retains original RPC order.
    inputs = {"classification_schema": SCHEMA, "policy_sha256": policy_hash,
              "source": sorted(source, key=lambda item: item["source_index"]),
              "fetch_membership": sorted(membership, key=lambda item: item["source_index"]),
              "raw_rpc_hashes": {name: raw_hashes[name] for name in sorted(raw_hashes)
                                 if name.startswith(("transactions/", "rpc/"))}}
    return sha(canonical(inputs))


def validate_tables(source, membership, rows, supported, before, summary, complete=True):
    require(complete, "checkpoint cannot finalize")
    sources, members, classified = unique(source, "signature"), unique(membership, "signature"), unique(rows, "signature")
    require(set(sources) == set(members), "source/fetch membership mismatch")
    selected = [item for item in membership if item["selected"]]
    require(all(item["attempted"] and item["status"] in ("success", "failure") for item in selected), "unresolved selected fetch")
    success = {item["signature"] for item in selected if item["status"] == "success"}
    require(len(rows) == len(success), "successful fetch/classification count mismatch")
    require(set(classified) == success, "classification membership mismatch")
    for row in rows:
        member, origin = members[row["signature"]], sources[row["signature"]]
        require(row["source_index"] == origin["source_index"] and row["slot"] == origin["slot"], "row/source provenance mismatch")
        require(row["old_engine"]["capture_sha256"] == member["response_sha256"], "row/raw hash mismatch")
        blocker = row["old_engine"].get("first_blocker")
        require(row["old_engine"]["admission"] == "accepted" or
                bool(blocker and blocker.get("detail") and blocker.get("code") and blocker.get("class")),
                "missing old-engine rejection reason")
    require(supported == supported_rows(rows), "supported observation identity/count mismatch")
    require(before == before_rows(rows), "BEFORE table incomplete or altered")
    require(summary == summarize(source, membership, rows), "aggregate count mismatch")
    for family in ("deposit", "borrow"):
        derived_count = sum(row["success"] and ix["supported_action_family"] == family
                            for row in rows for ix in row["klend_instructions"])
        require(summary["successful_supported_actions"][family] == derived_count, "action count not derivable from rows")
    require(summary["source_count"] == summary["fetch_selected"] + summary["not_fetch_selected"], "source invariant")
    require(summary["fetch_selected"] == summary["fetch_success"] + summary["fetch_failure"], "fetch invariant")
    require(summary["classified_rows"] == summary["klend_top_level"] + summary["no_klend_top_level"], "classification invariant")


def load_raw(root):
    receipt = read(root / "capture-receipt.json")
    require(receipt.get("kind") == "capture_receipt" and receipt.get("fetch_complete") is True,
            "checkpoint cannot finalize")
    policy = read(root / "sampling-policy.json")
    require(policy["program_id"] == KLEND and policy["control_commit"] == "8855edb" and
            policy["classification_schema"] == SCHEMA and policy["commitment"] == "finalized" and
            policy["get_transaction_options"] == {"encoding": "json", "commitment": "finalized", "maxSupportedTransactionVersion": 0},
            "unsupported capture/control policy")
    hygiene(policy)
    hygiene(receipt)
    paths = {"sampling-policy.json", "source-signatures.json", "fetch-membership.json"}
    paths.update(str(path.relative_to(root)) for folder in ("rpc", "transactions")
                 for path in (root / folder).rglob("*") if path.is_file())
    hashes = receipt["raw_artifact_hashes"]
    require(set(hashes) == paths, "raw artifact hash inventory incomplete or duplicated")
    for name in sorted(paths):
        require((root / name).is_file(), f"raw response missing: {name}")
        require(sha((root / name).read_bytes()) == hashes[name], f"raw hash mismatch: {name}")
    require(receipt["policy_sha256"] == hashes["sampling-policy.json"] and
            receipt["source_sha256"] == hashes["source-signatures.json"], "explicit source/policy linkage mismatch")
    for name, expected_hash in policy.get("frozen_source_hashes", {}).items():
        require(hashes.get(name) == expected_hash, "fixed source provenance mismatch")
    require(receipt["provider"] == policy["provider"], "provider provenance mismatch")
    require(read(root / "rpc/genesis.json")["result"] == policy["expected_genesis_hash"] == receipt["genesis_hash"], "genesis mismatch")
    anchor = read(root / "rpc/anchor.json")["result"]
    require((receipt["slot_start"], receipt["slot_end"]) == (anchor - 10000, anchor), "slot window mismatch")
    for name, method, params in (
        ("genesis", "getGenesisHash", []),
        ("anchor", "getSlot", [{"commitment": policy["commitment"]}]),
        ("source", "getSignaturesForAddress", [policy["program_id"], {"limit": policy["signature_limit"], "commitment": policy["commitment"]}]),
    ):
        require(receipt["requests"][name] == {"method": method, "params": params, "response_file": f"rpc/{name}.json"}, "RPC request provenance mismatch")
    require(receipt["get_transaction_params"] == ["<source signature>", policy["get_transaction_options"]], "transaction request provenance mismatch")
    source = sorted(read(root / "source-signatures.json"), key=lambda item: item["source_index"])
    members = sorted(read(root / "fetch-membership.json"), key=lambda item: item["source_index"])
    unique(source, "signature")
    unique(source, "source_index")
    unique(members, "signature")
    expected = [dict(item, source_index=index) for index, item in enumerate(read(root / "rpc/source.json")["result"])]
    require(source == expected and len(source) <= policy["signature_limit"], "source listing not linked to captured RPC listing")
    for item in source:
        require(len(b58decode(item["signature"])) == 64 and type(item["slot"]) is int and item["slot"] >= 0,
                "invalid source signature or slot")
    require(len(members) == len(source), "fetch membership denominator mismatch")
    selected = [item["signature"] for item in source if receipt["slot_start"] <= item["slot"] <= receipt["slot_end"]][:policy["transaction_fetch_limit"]]
    require(len(selected) == policy["transaction_fetch_limit"], "selection incomplete")
    references = []
    for origin, member in zip(source, members):
        require((member["signature"], member["source_index"]) == (origin["signature"], origin["source_index"]), "membership/source linkage mismatch")
        require(member["selected"] == (origin["signature"] in selected), "selection rule changed")
        if not member["selected"]:
            require(member["status"] == "not_selected" and not member["attempted"] and not member["attempts"] and
                    member["response_file"] is None and member["response_sha256"] is None, "invalid unselected membership")
            continue
        require(member["attempted"] and member["status"] in ("success", "failure"), "unresolved selected fetch")
        attempts = member["attempts"]
        require(0 < len(attempts) <= policy["fetch_rule"]["max_attempts"], "invalid fetch attempts")
        for index, attempt in enumerate(attempts, 1):
            require(attempt["number"] == index, "attempt sequence mismatch")
            if attempt["response_file"] is not None:
                require(hashes.get(attempt["response_file"]) == attempt["response_sha256"], "attempt/raw hash mismatch")
                references.append(attempt["response_file"])
        last = attempts[-1]
        require((member["response_file"], member["response_sha256"]) == (last["response_file"], last["response_sha256"]), "final response linkage mismatch")
        if member["status"] == "failure":
            require(bool(member["failure_reason"]) and all(attempt["error"] for attempt in attempts) and
                    len(attempts) == policy["fetch_rule"]["max_attempts"] and member["failure_reason"] == last["error"],
                    "failed fetch lacks reason or bounded attempts")
        else:
            require(last["error"] is None, "successful fetch has error")
            require(member["response_file"] == f"transactions/{member['signature']}.json", "wrong transaction artifact path")
            result = read(root / member["response_file"])["result"]
            require(result["slot"] == origin["slot"] and result["transaction"]["signatures"][0] == origin["signature"], "transaction/source identity mismatch")
            require(result["meta"]["err"] == origin["err"], "transaction/source status mismatch")
    require(len(references) == len(set(references)), "duplicate transaction artifact")
    require(set(references) == {name for name in paths if name.startswith(("transactions/", "rpc/attempts/"))}, "unlinked transaction artifact")
    return policy, receipt, source, members


def audit_files(root, members, reverse=False):
    paths = [str(root / member["response_file"]) for member in members if member["status"] == "success"]
    if reverse:
        paths.reverse()
    require(paths, "no successful production transaction captures")
    binary = REPO / "target/debug/examples/classify_kamino_baseline"
    env = {key: value for key, value in os.environ.items() if not re.search(r'RPC|API_KEY|ARCHIVE', key)}
    env["CARGO_NET_OFFLINE"] = "true"
    result = subprocess.run([str(binary), *paths], capture_output=True, check=True, env=env)
    return unique(json.loads(result.stdout), "signature")


def derive(root, reverse=False):
    policy, receipt, source, members = load_raw(root)
    audits = audit_files(root, members, reverse)
    origins = unique(source, "signature")
    enumeration = list(reversed(members)) if reverse else members
    rows = [classify(read(root / member["response_file"])["result"], origins[member["signature"]], audits[member["signature"]])
            for member in enumeration if member["status"] == "success"]
    rows.sort(key=lambda row: row["source_index"])
    supported, before, summary = supported_rows(rows), before_rows(rows), summarize(source, members, rows)
    validate_tables(source, members, rows, supported, before, summary)
    outputs = {"classifications.json": canonical(rows), "supported-observations.json": canonical(supported),
               "before-rejections.json": canonical(before), "summary.json": canonical(summary)}
    raw_hashes = dict(receipt["raw_artifact_hashes"], **{"capture-receipt.json": sha((root / "capture-receipt.json").read_bytes())})
    manifest = {"kind": "frozen_production_baseline", "complete": True, "sample_version": policy["sample_version"],
                "classification_schema": SCHEMA, "control_commit": policy["control_commit"],
                "sample_kind": "U3 pre-change production baseline collected before modern replay implementation",
                "sample_fingerprint": fingerprint(source, members, raw_hashes, receipt["policy_sha256"]),
                "fingerprint_definition": "SHA256 of canonical classification schema, policy hash, source rows sorted by source_index, fetch membership sorted by source_index, sorted raw RPC response hashes",
                "collection_rule": policy, "slot_start": receipt["slot_start"], "slot_end": receipt["slot_end"],
                "source_slot_min": min(item["slot"] for item in source), "source_slot_max": max(item["slot"] for item in source),
                "provider": receipt["provider"], "genesis_hash": receipt["genesis_hash"], "summary": summary,
                "raw_artifact_hashes": dict(sorted(raw_hashes.items())),
                "derived_artifact_hashes": {name: sha(data) for name, data in sorted(outputs.items())},
                "known_limits": ["New prospective sample; original U2 classified denominator remains unrecoverable.",
                                 "Fetch failures remain in the selected denominator; the classified subset may reflect provider availability. No failed signature was replaced.",
                                 "RPC transaction metadata only; no historical account or lookup-table account state acquired.",
                                 "Unchanged U2 admission and discovery(state=None); no baseline execution or production replay eligibility earned.",
                                 "Transient-account diagnosis uses transaction metadata, not verified historical account bytes.",
                                 "Provider receipt records request parameters; transaction evidence is provider-attested, not independently ledger-verified."]}
    hygiene(manifest)
    outputs["manifest.json"] = canonical(manifest)
    checksums = dict(raw_hashes, **{name: sha(data) for name, data in outputs.items()})
    outputs["checksums.sha256"] = "".join(f"{digest}  {name}\n" for name, digest in sorted(checksums.items())).encode()
    return outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sample", type=Path, default=DEFAULT_SAMPLE)
    parser.add_argument("--output", type=Path, help="write to a separate directory; frozen raw sample stays unchanged")
    parser.add_argument("--verify", action="store_true", help="compare all regenerated bytes with committed artifacts")
    parser.add_argument("--reverse", action="store_true", help="reverse classifier/input enumeration to check determinism")
    args = parser.parse_args()
    outputs = derive(args.sample.resolve(), args.reverse)
    require(not args.verify or args.output is None, "verify cannot write")
    for name, data in outputs.items():
        if args.verify:
            require((args.sample / name).read_bytes() == data, f"derived bytes differ: {name}")
        else:
            destination = args.output or args.sample
            destination.mkdir(parents=True, exist_ok=True)
            (destination / name).write_bytes(data)
    print(json.loads(outputs["manifest.json"])["sample_fingerprint"])


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"baseline finalization failed: {error}", file=sys.stderr)
        sys.exit(1)
