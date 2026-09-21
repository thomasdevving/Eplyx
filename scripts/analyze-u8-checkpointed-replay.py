#!/usr/bin/env python3
"""Build the deterministic Phase U8 closure and evidence census."""

from __future__ import annotations

import base64
import hashlib
import json
import subprocess
from collections import Counter, defaultdict
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "docs/examples/phase-u8-analysis"
FREEZE = REPO / "docs/examples/phase-u8-freeze"
ARCHIVE = REPO / "docs/examples/phase-u7-archive"
TARGET_INDEX = 259
TARGET_SIGNATURE = "2oBLMEw1UwzVLaFn8Ync4jtMkPtHahRD5jgx4g4ucLpwuADDY9Ew8eDFckS9bFv3dDEiZDsYTS2QUqugzX5zWeFC"
SLOT = 448760958
SYSTEM = "11111111111111111111111111111111"
COMPUTE = "ComputeBudget111111111111111111111111111111"
UPGRADEABLE = "BPFLoaderUpgradeab1e11111111111111111111111"
BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def read(path: str | Path) -> object:
    return json.loads((REPO / path).read_text() if isinstance(path, str) else path.read_text())


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical(value))


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def b58decode(value: str) -> bytes:
    number = 0
    for char in value:
        number = number * 58 + BASE58.index(char)
    raw = number.to_bytes((number.bit_length() + 7) // 8, "big") if number else b""
    return b"\0" * (len(value) - len(value.lstrip("1"))) + raw


def b58encode(value: bytes) -> str:
    number = int.from_bytes(value, "big")
    encoded = ""
    while number:
        number, remainder = divmod(number, 58)
        encoded = BASE58[remainder] + encoded
    return "1" * (len(value) - len(value.lstrip(b"\0"))) + encoded


def resolved_raw_keys(tx: dict) -> list[str]:
    static = tx["transaction"]["message"]["accountKeys"]
    loaded = tx["meta"].get("loadedAddresses") or {"writable": [], "readonly": []}
    return static + loaded["writable"] + loaded["readonly"]


def nonce_accounts(raw_tx: dict) -> list[str]:
    keys = resolved_raw_keys(raw_tx)
    found = []
    for instruction in raw_tx["transaction"]["message"]["instructions"]:
        if keys[instruction["programIdIndex"]] != SYSTEM:
            continue
        data = b58decode(instruction["data"])
        # SystemInstruction::AdvanceNonceAccount is bincode enum variant 4.
        if len(data) >= 4 and int.from_bytes(data[:4], "little") == 4 and instruction["accounts"]:
            found.append(keys[instruction["accounts"][0]])
    return sorted(set(found))


def archive_value(slot: int, address: str) -> dict | None:
    body = read(ARCHIVE / f"{slot}-{address}.body")
    return body["result"]["value"]


def account_digest(value: dict | None) -> str | None:
    if value is None:
        return None
    return hashlib.sha256(base64.b64decode(value["data"][0])).hexdigest()


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    FREEZE.mkdir(parents=True, exist_ok=True)

    candidate = read("docs/examples/phase-u5-sample/first-candidate.json")
    parsed_block = read("docs/examples/phase-u5-sample/first-candidate-screen-block.body")["result"]
    raw_block = read("docs/examples/phase-u5-sample/blocks/448760958.body")["result"]
    stream = read("docs/examples/phase-u7-full-slot/analysis.json")
    parsed_target = parsed_block["transactions"][TARGET_INDEX]
    raw_target = raw_block["transactions"][TARGET_INDEX]
    assert parsed_target["transaction"]["signatures"][0] == TARGET_SIGNATURE
    assert raw_target["transaction"]["signatures"][0] == TARGET_SIGNATURE

    target_keys = parsed_target["transaction"]["accountKeys"]
    addresses = [item["pubkey"] for item in target_keys]
    assert len(addresses) == 22 and len(set(addresses)) == 22
    role_by_address = {item["address"]: item["role"] for item in candidate["fixed_role_accounts"]}
    event_by_address = {item["address"]: item["events"] for item in stream["target_accounts"]}
    output_addresses = [item["pubkey"] for item in target_keys if item["writable"]]
    readonly_addresses = [item["pubkey"] for item in target_keys if not item["writable"]]
    assert len(output_addresses) == 12

    writable_by_tx: list[set[str]] = []
    for tx in parsed_block["transactions"]:
        writable_by_tx.append({key["pubkey"] for key in tx["transaction"]["accountKeys"] if key["writable"]})

    conflict_rows = []
    for key_index, key in enumerate(target_keys):
        address = key["pubkey"]
        before = [index for index in range(TARGET_INDEX) if address in writable_by_tx[index]]
        after = [index for index in range(TARGET_INDEX + 1, len(writable_by_tx)) if address in writable_by_tx[index]]
        events = event_by_address.get(address, [])
        conflict_rows.append({
            "address": address,
            "target_key_index": key_index,
            "role": role_by_address.get(address),
            "target_writable": key["writable"],
            "closure_class": "validation_output" if key["writable"] else "execution_input_only",
            "earlier_writable_transaction_indexes": before,
            "later_writable_transaction_indexes": after,
            "provider_account_events": [
                {"transaction_index": event["transaction_index"], "txn_signature": event["txn_signature"],
                 "write_version": event["write_version"], "data_sha256": event["data_sha256"]}
                for event in events
            ],
        })

    earlier = sorted({i for row in conflict_rows for i in row["earlier_writable_transaction_indexes"]})
    later = sorted({i for row in conflict_rows for i in row["later_writable_transaction_indexes"]})
    assert earlier == []
    assert later == [385, 819, 846, 857, 860, 861, 896, 1114]

    overlap_transactions = []
    for index in later:
        parsed = parsed_block["transactions"][index]
        raw = raw_block["transactions"][index]
        meta = parsed["meta"]
        keys = parsed["transaction"]["accountKeys"]
        tx_addresses = [key["pubkey"] for key in keys]
        overlaps = sorted(set(addresses) & writable_by_tx[index], key=addresses.index)
        nonces = nonce_accounts(raw)
        payer = keys[0]["pubkey"]
        details = []
        for address in overlaps:
            account_index = tx_addresses.index(address)
            lamport_delta = meta["postBalances"][account_index] - meta["preBalances"][account_index]
            if meta["err"] is not None:
                if address == payer:
                    effect = "fee_payer_effect_committed"
                elif address in nonces:
                    effect = "nonce_advance_effect_committed"
                else:
                    effect = "normal_execution_effect_rolled_back"
            else:
                effect = "successful_declared_writable_effect_unknown"
            details.append({
                "address": address,
                "target_class": "validation_output" if address in output_addresses else "execution_input_only",
                "is_fee_payer": address == payer,
                "is_durable_nonce": address in nonces,
                "lamport_delta": lamport_delta,
                "effect_class": effect,
            })
        overlap_transactions.append({
            "transaction_index": index,
            "signature": parsed["transaction"]["signatures"][0],
            "message_version": parsed["version"],
            "success": meta["err"] is None,
            "error": meta["err"],
            "fee": meta["fee"],
            "fee_payer": payer,
            "durable_nonce_accounts": nonces,
            "overlap_accounts": details,
        })

    failed_output_overlaps = [
        tx for tx in overlap_transactions
        if not tx["success"] and any(row["target_class"] == "validation_output" for row in tx["overlap_accounts"])
    ]
    successful_output_overlaps = [
        tx for tx in overlap_transactions
        if tx["success"] and any(row["target_class"] == "validation_output" for row in tx["overlap_accounts"])
    ]
    assert len(failed_output_overlaps) == 6 and not successful_output_overlaps
    assert all(
        row["effect_class"] == "normal_execution_effect_rolled_back"
        for tx in failed_output_overlaps for row in tx["overlap_accounts"]
        if row["target_class"] == "validation_output"
    )

    conflict = {
        "schema": "eplyx.phase-u8.conflicts.v1",
        "slot": SLOT,
        "target_transaction_index": TARGET_INDEX,
        "target_signature": TARGET_SIGNATURE,
        "method": {
            "direct_conflict_source": "jsonParsed getBlock writable account flags",
            "outcome_source": "getBlock transaction meta",
            "failed_transaction_rule": "only the fee-payer and durable-nonce rollback accounts can persist on execution failure",
            "account_stream_role": "corroboration only; absence is not used to prove no write",
        },
        "summary": {
            "target_accounts": len(addresses),
            "validation_outputs": len(output_addresses),
            "execution_input_only_accounts": len(readonly_addresses),
            "earlier_direct_conflict_transactions": len(earlier),
            "later_direct_conflict_transactions": len(later),
            "later_failed_output_conflicts": len(failed_output_overlaps),
            "later_successful_output_conflicts": len(successful_output_overlaps),
            "later_successful_readonly_input_conflicts": 2,
        },
        "accounts": conflict_rows,
        "overlap_transactions": overlap_transactions,
    }
    write(OUT / "conflict-table.json", conflict)

    versions = Counter(str(tx.get("version", "legacy")) for tx in raw_block["transactions"])
    closure = {
        "schema": "eplyx.phase-u8.closure.v1",
        "slot": SLOT,
        "checkpoint_a": SLOT - 1,
        "checkpoint_b": SLOT,
        "target_transaction_index": TARGET_INDEX,
        "execution_inputs": addresses,
        "validation_outputs": output_addresses,
        "backward_closure": {
            "iterations": [{"iteration": 0, "needed_accounts": addresses, "new_transaction_indexes": []}],
            "result_transaction_indexes": [TARGET_INDEX],
            "proof": "No transaction before index 259 declares any target execution input writable.",
        },
        "forward_closure": {
            "iterations": [{"iteration": 0, "tracked_outputs": output_addresses, "new_transaction_indexes": []}],
            "excluded_failed_transaction_indexes": [tx["transaction_index"] for tx in failed_output_overlaps],
            "excluded_readonly_only_transaction_indexes": [819, 846],
            "result_transaction_indexes": [TARGET_INDEX],
            "proof": "No successful later transaction writes a validation output; failed overlaps affect neither fee-payer nor nonce output accounts.",
        },
        "minimal_candidate_segment": [TARGET_INDEX],
        "conservative_safe_segment": [TARGET_INDEX],
        "segments_equal": True,
        "segment_transactions": [{
            "transaction_index": TARGET_INDEX,
            "signature": TARGET_SIGNATURE,
            "message_version": raw_target.get("version", "legacy"),
            "success": raw_target["meta"]["err"] is None,
        }],
        "segment_counts": {"transactions": 1, "legacy": 0, "v0": 1, "v1": 0, "failed": 0},
        "whole_block_version_counts": dict(sorted(versions.items())),
        "soundness_conditions": [
            "writable flags are resolved with the historical LUT state",
            "transaction order is the canonical block order",
            "failed transactions commit only fee-payer and durable-nonce rollback accounts",
            "validation compares only target outputs at checkpoint B while all target inputs are loaded at checkpoint A",
            "runtime and program identities are pinned independently of closure membership",
        ],
    }
    write(OUT / "closure.json", closure)

    raw_keys = resolved_raw_keys(raw_target)
    executed_programs = set()
    for instruction in raw_target["transaction"]["message"]["instructions"]:
        executed_programs.add(raw_keys[instruction["programIdIndex"]])
    for group in raw_target["meta"].get("innerInstructions", []):
        for instruction in group["instructions"]:
            executed_programs.add(raw_keys[instruction["programIdIndex"]])

    programs = []
    for address in sorted(executed_programs):
        value = archive_value(SLOT - 1, address)
        assert value is not None
        owner = value["owner"]
        data = base64.b64decode(value["data"][0])
        if owner == UPGRADEABLE:
            assert int.from_bytes(data[:4], "little") == 2 and len(data) >= 36
            programdata = b58encode(data[4:36])
            programdata_path = ARCHIVE / f"{SLOT - 1}-{programdata}.body"
            programs.append({
                "program_id": address,
                "kind": "upgradeable_bpf",
                "program_account_checkpoint_a": "present",
                "programdata_address": programdata,
                "programdata_checkpoint_a": "present" if programdata_path.exists() else "missing",
                "historical_elf": "present" if programdata_path.exists() else "missing",
            })
        elif owner == "BPFLoader2111111111111111111111111111111111":
            programs.append({
                "program_id": address,
                "kind": "legacy_bpf",
                "program_account_checkpoint_a": "present",
                "historical_elf": "present",
                "elf_bytes": len(data),
                "elf_sha256": hashlib.sha256(data).hexdigest(),
            })
        else:
            programs.append({
                "program_id": address,
                "kind": "native_builtin",
                "program_account_checkpoint_a": "present",
                "historical_implementation": "missing",
            })

    lookups = []
    for descriptor in candidate["lookup_descriptors"]:
        address = descriptor["accountKey"]
        path = ARCHIVE / f"{SLOT - 1}-{address}.body"
        lookups.append({
            "address": address,
            "writable_indexes": descriptor["writableIndexes"],
            "readonly_indexes": descriptor["readonlyIndexes"],
            "checkpoint_a_account": "present" if path.exists() else "missing",
            "resolution_status": "proven" if path.exists() else "unproven",
        })

    receipts = {(item["requested_slot"], item["address"]): item for item in read(ARCHIVE / "archive-receipts.json")}
    account_evidence = []
    for address in addresses:
        a = archive_value(SLOT - 1, address)
        b = archive_value(SLOT, address)
        receipt_a = receipts[(SLOT - 1, address)]
        receipt_b = receipts[(SLOT, address)]
        account_evidence.append({
            "address": address,
            "checkpoint_a_present": a is not None,
            "checkpoint_b_present": b is not None,
            "checkpoint_a_data_sha256": account_digest(a),
            "checkpoint_b_data_sha256": account_digest(b),
            "checkpoint_a_response_sha256": receipt_a["body_sha256"],
            "checkpoint_b_response_sha256": receipt_b["body_sha256"],
            "checkpoint_a_context_slot": receipt_a["context_slot"],
            "checkpoint_b_context_slot": receipt_b["context_slot"],
            "checkpoint_a_response_bytes": receipt_a["body_bytes"],
            "checkpoint_b_response_bytes": receipt_b["body_bytes"],
            "provider": "Alchemy Account Archive",
        })

    census = {
        "schema": "eplyx.phase-u8.evidence-census.v1",
        "programs": programs,
        "program_summary": {
            "executed_programs": len(programs),
            "upgradeable_programs": sum(row["kind"] == "upgradeable_bpf" for row in programs),
            "missing_upgradeable_elfs": sum(row.get("historical_elf") == "missing" for row in programs),
            "native_builtins_without_historical_implementation": sum(row["kind"] == "native_builtin" for row in programs),
        },
        "lookup_tables": lookups,
        "lookup_summary": {"required": len(lookups), "checkpoint_a_present": sum(row["checkpoint_a_account"] == "present" for row in lookups)},
        "accounts": account_evidence,
        "account_summary": {
            "required": len(account_evidence),
            "checkpoint_a_responses": len(account_evidence),
            "checkpoint_b_responses": len(account_evidence),
            "checkpoint_a_present": sum(row["checkpoint_a_present"] for row in account_evidence),
            "checkpoint_b_present": sum(row["checkpoint_b_present"] for row in account_evidence),
            "checkpoint_a_observed_absent": sum(not row["checkpoint_a_present"] for row in account_evidence),
            "checkpoint_b_observed_absent": sum(not row["checkpoint_b_present"] for row in account_evidence),
            "checkpoint_a_response_bytes": sum(row["checkpoint_a_response_bytes"] for row in account_evidence),
            "checkpoint_b_response_bytes": sum(row["checkpoint_b_response_bytes"] for row in account_evidence),
            "both_checkpoint_response_bytes": sum(row["checkpoint_a_response_bytes"] + row["checkpoint_b_response_bytes"] for row in account_evidence),
        },
        "runtime": [
            {"dependency": "canonical transaction order and blockhash", "status": "present", "source": "frozen getBlock"},
            {"dependency": "RecentBlockhashes account image", "status": "missing", "source": "Account Archive returned an exact-slot null; the U7 stream event is not an independent checkpoint"},
            {"dependency": "Clock at target slot", "status": "missing"},
            {"dependency": "Rent and EpochSchedule at target slot", "status": "missing"},
            {"dependency": "active feature set", "status": "missing"},
            {"dependency": "historical native builtin implementation", "status": "missing"},
            {"dependency": "fee/rent/compute-budget rules", "status": "missing"},
            {"dependency": "Instructions sysvar", "status": "runtime-synthesized; implementation missing"},
        ],
    }
    write(OUT / "evidence-census.json", census)

    feasibility = {
        "schema": "eplyx.phase-u8.feasibility.v1",
        "classification": "D",
        "label": "HISTORICAL_EVIDENCE_UNAVAILABLE",
        "closure_qualification": "PASS_BOUNDED_SINGLE_TRANSACTION",
        "replay_qualification": "FAIL_CLOSED",
        "reason": "The dependency closure is bounded and v0-supported, but four LUT snapshots, three upgradeable ProgramData ELFs, historical native/runtime identity, and required sysvar/feature evidence are not frozen.",
        "missing_evidence_counts": {
            "lookup_tables": 4,
            "upgradeable_program_elfs": 3,
            "native_builtin_implementations": 2,
            "runtime_items": 7,
        },
        "implemented_replay": False,
        "product_schema_changed": False,
        "next_acquisition_scope": {
            "accounts_at_checkpoint_a": [row["address"] for row in lookups] + [row["programdata_address"] for row in programs if row["kind"] == "upgradeable_bpf"],
            "runtime": ["Clock", "Rent", "EpochSchedule", "active feature set", "historical validator/runtime identity"],
        },
    }
    write(OUT / "feasibility.json", feasibility)

    u7_manifest_path = REPO / "docs/examples/phase-u7-freeze/final-manifest.json"
    u7_manifest = read(u7_manifest_path)
    changed = []
    for item in u7_manifest["u7_files"]:
        path = REPO / item["path"]
        if not path.exists() or path.stat().st_size != item["bytes"] or sha256(path) != item["sha256"]:
            changed.append(item["path"])
    prior_check = {
        "u7_manifest_sha256": sha256(u7_manifest_path),
        "checked_u7_files": len(u7_manifest["u7_files"]),
        "changed_u7_files": changed,
        "unchanged": not changed,
    }
    write(FREEZE / "prior-artifact-check.json", prior_check)

    status = subprocess.run(["git", "status", "--short"], cwd=REPO, text=True, capture_output=True, check=True).stdout.splitlines()
    pre_u8_status = [line for line in status if "phase-u8" not in line and "u8-" not in line]
    initial = {
        "head": subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, text=True, capture_output=True, check=True).stdout.strip(),
        "branch": subprocess.run(["git", "branch", "--show-current"], cwd=REPO, text=True, capture_output=True, check=True).stdout.strip(),
        "preexisting_status_lines": pre_u8_status,
        "note": "U8 paths are excluded so this records the pre-existing dirty worktree inherited from U4-U7.",
    }
    write(FREEZE / "initial.json", initial)

    frozen_inputs = {
        "head": initial["head"],
        "target": {"slot": SLOT, "transaction_index": TARGET_INDEX, "outer_instruction_index": 4, "signature": TARGET_SIGNATURE},
        "u5_sample_fingerprint": "f77749d7a7992253e15f0df34e3a8f574998cc8868586884ec987f30423712c3",
        "files": [
            {"path": path, "bytes": (REPO / path).stat().st_size, "sha256": sha256(REPO / path)}
            for path in [
                "docs/phase-u4-universal-replay-final-report.md",
                "docs/phase-u5-orca-adversarial-report.md",
                "docs/phase-u6-transaction-boundary-feasibility.md",
                "docs/phase-u7-live-boundary-qualification.md",
                "docs/examples/phase-u5-sample/sample.json",
                "docs/examples/phase-u5-sample/first-candidate.json",
                "docs/examples/phase-u5-sample/blocks/448760958.body",
                "docs/examples/phase-u7-full-slot/analysis.json",
                "docs/examples/phase-u7-archive/archive-receipts.json",
                "docs/examples/phase-u7-archive/target-key-comparison.json",
            ]
        ],
    }
    write(FREEZE / "frozen-inputs.json", frozen_inputs)

    final_state = {
        "head": initial["head"],
        "branch": initial["branch"],
        "head_unchanged": subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, text=True, capture_output=True, check=True).stdout.strip() == initial["head"],
        "u8_product_or_core_source_changes": [],
        "u8_scope": ["analysis scripts", "frozen source semantics", "derived evidence", "qualification report"],
        "commit_created": False,
        "reason_no_commit": "U8 stopped at the acquisition feasibility gate; the suggested commit is reserved for a successful proof.",
    }
    write(FREEZE / "final-state.json", final_state)

    manifest_path = FREEZE / "final-manifest.json"
    candidates = [path for path in OUT.rglob("*") if path.is_file()]
    candidates += [path for path in FREEZE.rglob("*") if path.is_file() and path != manifest_path]
    candidates += [REPO / "scripts/analyze-u8-checkpointed-replay.py", REPO / "scripts/run-u8-controls.py", REPO / "scripts/freeze-u8-agave-sources.py"]
    report_path = REPO / "docs/phase-u8-checkpointed-transaction-boundary.md"
    if report_path.exists():
        candidates.append(report_path)
    manifest_paths = sorted(set(candidates))
    manifest = {
        "schema": "eplyx.phase-u8.manifest.v1",
        "self_excluded": "docs/examples/phase-u8-freeze/final-manifest.json",
        "files": [
            {"path": str(path.relative_to(REPO)), "bytes": path.stat().st_size, "sha256": sha256(path)}
            for path in manifest_paths if path.exists()
        ],
    }
    write(manifest_path, manifest)
    print(json.dumps({
        "classification": feasibility["classification"],
        "closure": closure["minimal_candidate_segment"],
        "later_conflicts": len(later),
        "missing_luts": feasibility["missing_evidence_counts"]["lookup_tables"],
        "missing_elfs": feasibility["missing_evidence_counts"]["upgradeable_program_elfs"],
    }, indent=2))


if __name__ == "__main__":
    main()
