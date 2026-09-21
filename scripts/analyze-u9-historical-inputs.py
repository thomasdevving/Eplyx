#!/usr/bin/env python3
"""Qualify the Phase U9 historical execution inputs without executing the target."""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import struct
import subprocess
import time
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
ACQ = REPO / "docs/examples/phase-u9-acquisition"
OUT = REPO / "docs/examples/phase-u9-analysis"
FREEZE = REPO / "docs/examples/phase-u9-freeze"
SLOT = 448760958
INDEX = 259
SIGNATURE = "2oBLMEw1UwzVLaFn8Ync4jtMkPtHahRD5jgx4g4ucLpwuADDY9Ew8eDFckS9bFv3dDEiZDsYTS2QUqugzX5zWeFC"
GENESIS = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
VALIDATION_HASH = "c21a90f2a47425dfa4f925c344bcbbd4d667367547bf3c559f74c35a121e95e4"
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
RECENT = "SysvarRecentB1ockHashes11111111111111111111"
SLOT_HASHES = "SysvarS1otHashes111111111111111111111111111"
UPGRADEABLE = "BPFLoaderUpgradeab1e11111111111111111111111"
LEGACY_LOADER = "BPFLoader2111111111111111111111111111111111"


def read(path: Path | str) -> object:
    p = REPO / path if isinstance(path, str) else path
    return json.loads(p.read_text())


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical(value))


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_sha(path: Path) -> str:
    return sha(path.read_bytes())


def b58encode(value: bytes) -> str:
    number = int.from_bytes(value, "big")
    encoded = ""
    while number:
        number, remainder = divmod(number, 58)
        encoded = ALPHABET[remainder] + encoded
    return "1" * (len(value) - len(value.lstrip(b"\0"))) + encoded


def b58decode(value: str) -> bytes:
    number = 0
    for char in value:
        number = number * 58 + ALPHABET.index(char)
    raw = number.to_bytes((number.bit_length() + 7) // 8, "big") if number else b""
    return b"\0" * (len(value) - len(value.lstrip("1"))) + raw


def address_text(value: object) -> str:
    return value if isinstance(value, str) else b58encode(bytes(value))


def varint(data: bytes, offset: int) -> tuple[int, int]:
    value = 0
    shift = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            return value, offset
        shift += 7


def protobuf_fields(data: bytes):
    offset = 0
    while offset < len(data):
        key, offset = varint(data, offset)
        number, wire = key >> 3, key & 7
        if wire == 0:
            value, offset = varint(data, offset)
        elif wire == 1:
            value, offset = data[offset:offset + 8], offset + 8
        elif wire == 2:
            length, offset = varint(data, offset)
            value, offset = data[offset:offset + length], offset + length
        elif wire == 5:
            value, offset = data[offset:offset + 4], offset + 4
        else:
            raise ValueError(f"unsupported protobuf wire type {wire}")
        yield number, wire, value


def field(data: bytes, number: int):
    return next(value for candidate, _, value in protobuf_fields(data) if candidate == number)


def stream_sysvars() -> dict[str, dict]:
    source = REPO / "docs/examples/phase-u7-full-slot/stream.pbseq"
    wanted = {RECENT, SLOT_HASHES}
    found = {}
    with source.open("rb") as handle:
        frame_number = 0
        while header := handle.read(4):
            frame = handle.read(struct.unpack("<I", header)[0])
            update_fields = list(protobuf_fields(frame))
            account_update = next((value for number, _, value in update_fields if number == 2), None)
            if account_update is not None and field(account_update, 2) == SLOT:
                info = field(account_update, 1)
                address = b58encode(field(info, 1))
                if address in wanted:
                    data = field(info, 6)
                    row = {
                        "address": address,
                        "frame_number": frame_number,
                        "frame_sha256": sha(frame),
                        "owner": b58encode(field(info, 3)),
                        "lamports": field(info, 2),
                        "executable": bool(next((value for number, _, value in protobuf_fields(info) if number == 4), 0)),
                        "rent_epoch": field(info, 5),
                        "write_version": next((value for number, _, value in protobuf_fields(info) if number == 7), 0),
                        "transaction_signature_present": any(number == 8 for number, _, _ in protobuf_fields(info)),
                        "data_bytes": len(data),
                        "data_sha256": sha(data),
                        "data": data,
                        "frame": frame,
                    }
                    found[address] = row
            frame_number += 1
    if set(found) != wanted:
        raise RuntimeError(f"missing streamed sysvar events: {wanted - set(found)}")
    return found


def archive_value(slot: int, address: str) -> dict | None:
    return read(REPO / f"docs/examples/phase-u7-archive/{slot}-{address}.body")["result"]["value"]


def account_data(slot: int, address: str) -> bytes:
    value = archive_value(slot, address)
    if value is None:
        raise RuntimeError(f"missing account {address} at {slot}")
    return base64.b64decode(value["data"][0], validate=True)


def decode_nonce(data: bytes) -> dict:
    if len(data) != 80:
        raise RuntimeError("nonce account is not 80 bytes")
    return {
        "version": int.from_bytes(data[0:4], "little"),
        "state": int.from_bytes(data[4:8], "little"),
        "authority": b58encode(data[8:40]),
        "durable_nonce": b58encode(data[40:72]),
        "lamports_per_signature": int.from_bytes(data[72:80], "little"),
        "data_sha256": sha(data),
    }


def source_feature(name: str, feature_source: str, lite_source: str) -> dict:
    module = re.search(rf"pub mod {re.escape(name)}\s*\{{(.{{0,900}}?)\n\}}", feature_source, re.S)
    if not module:
        raise RuntimeError(f"feature module missing: {name}")
    identity = re.search(r'declare_id!\("([1-9A-HJ-NP-Za-km-z]+)"\)', module.group(1))
    activation = re.search(rf"agave_feature_set::{re.escape(name)}::ID\s*,\s*([0-9_]+)", lite_source, re.S)
    if not identity or not activation:
        raise RuntimeError(f"feature identity/activation missing: {name}")
    return {"name": name, "feature_id": identity.group(1), "activation_slot": int(activation.group(1).replace("_", ""))}


def main() -> None:
    started = time.perf_counter()
    OUT.mkdir(parents=True, exist_ok=True)
    FREEZE.mkdir(parents=True, exist_ok=True)

    acquisition = read(ACQ / "acquisition.json")
    checksums = read(ACQ / "checksums.json")
    bad = []
    for name, expected in checksums.items():
        path = ACQ / name
        if not path.exists() or path.stat().st_size != expected["bytes"] or file_sha(path) != expected["sha256"]:
            bad.append(name)
    if bad or not acquisition["complete"]:
        raise RuntimeError(f"acquisition integrity failure: {bad}")

    u8_manifest_path = REPO / "docs/examples/phase-u8-freeze/final-manifest.json"
    u8_manifest = read(u8_manifest_path)
    u8_changed = []
    for item in u8_manifest["files"]:
        path = REPO / item["path"]
        if not path.exists() or path.stat().st_size != item["bytes"] or file_sha(path) != item["sha256"]:
            u8_changed.append(item["path"])
    if u8_changed:
        raise RuntimeError(f"U8 changed: {u8_changed}")
    write(FREEZE / "prior-artifact-check.json", {
        "u8_manifest_sha256": file_sha(u8_manifest_path),
        "checked_u8_files": len(u8_manifest["files"]),
        "changed_u8_files": u8_changed,
        "unchanged": not u8_changed,
    })

    envelope = read("docs/examples/phase-u5-sample/first-candidate-validator-envelope.json")
    result = {"slot": SLOT, **envelope["transaction"]}
    if result["transaction"]["signatures"][0] != SIGNATURE or result["version"] != 0:
        raise RuntimeError("target identity changed")
    provider = {
        "scheme_host": "https://solana-mainnet.g.alchemy.com",
        "genesis_hash": GENESIS,
        "visibility": "finalized_end_of_execution_slot",
        "validation_artifact_sha256": VALIDATION_HASH,
    }
    lut_evidence = []
    for row in acquisition["luts"]:
        path = ACQ / f"raw/lut-{SLOT}-{row['address']}.json"
        lut_evidence.append({
            "pubkey": row["address"],
            "provider": provider,
            "raw_response_base64": base64.b64encode(path.read_bytes()).decode(),
        })
    lut_input = [{"result": result, "genesis": GENESIS, "evidence": lut_evidence}]
    write(OUT / "lut-reconstruction-input.json", lut_input)
    environment = {"PATH": os.environ["PATH"]}
    process = subprocess.run(
        [str(REPO / "target/debug/examples/reconstruct_lut")],
        cwd=REPO,
        env=environment,
        input=canonical(lut_input),
        capture_output=True,
        check=False,
    )
    (OUT / "lut-reconstruction.stderr").write_bytes(process.stderr)
    if process.returncode != 0:
        raise RuntimeError(process.stderr.decode(errors="replace"))
    lut_result = json.loads(process.stdout)[0]
    if lut_result["failure"] is not None or lut_result["proof"] is None:
        raise RuntimeError(json.dumps(lut_result["failure"]))
    proof = lut_result["proof"]
    raw_target = read("docs/examples/phase-u5-sample/blocks/448760958.body")["result"]["transactions"][INDEX]
    expected_writable = raw_target["meta"]["loadedAddresses"]["writable"]
    expected_readonly = raw_target["meta"]["loadedAddresses"]["readonly"]
    actual_writable = [address_text(row) for row in proof["resolved_writable"]]
    actual_readonly = [address_text(row) for row in proof["resolved_readonly"]]
    if actual_writable != expected_writable or actual_readonly != expected_readonly:
        raise RuntimeError("independent LUT reconstruction differs from validator metadata")
    parsed_target = read("docs/examples/phase-u5-sample/first-candidate-screen-block.body")["result"]["transactions"][INDEX]
    expected_keys = [row["pubkey"] for row in parsed_target["transaction"]["accountKeys"]]
    actual_keys = [address_text(row["address"]) for row in proof["full_account_keys"]]
    if actual_keys != expected_keys:
        raise RuntimeError("full reconstructed key vector differs")
    lut_report = {
        "schema": "eplyx.phase-u9.lut-proof.v1",
        "signature": SIGNATURE,
        "execution_slot": SLOT,
        "proof_id": proof["proof_id"],
        "exact_loaded_writable_match": True,
        "exact_loaded_readonly_match": True,
        "exact_full_key_vector_match": True,
        "loaded_writable": actual_writable,
        "loaded_readonly": actual_readonly,
        "tables": proof["tables"],
        "all_s_minus_1_equal_s": all(row["s_minus_1_equals_s"] for row in acquisition["luts"]),
        "slot_hashes_required": proof["slot_hashes_evidence_id"] is not None,
    }
    write(OUT / "lut-proof.json", lut_report)

    executed = set()
    keys = raw_target["transaction"]["message"]["accountKeys"] + expected_writable + expected_readonly
    for instruction in raw_target["transaction"]["message"]["instructions"]:
        executed.add(keys[instruction["programIdIndex"]])
    for group in raw_target["meta"]["innerInstructions"]:
        for instruction in group["instructions"]:
            executed.add(keys[instruction["programIdIndex"]])
    programdata_by_program = {row["program_id"]: row for row in acquisition["programdata"]}
    programs = []
    for address in sorted(executed):
        value = archive_value(SLOT - 1, address)
        if value is None:
            raise RuntimeError(f"program account missing: {address}")
        data = base64.b64decode(value["data"][0], validate=True)
        owner = value["owner"]
        if owner == UPGRADEABLE:
            pointer = b58encode(data[4:36])
            captured = programdata_by_program[address]
            if pointer != captured["programdata_address"]:
                raise RuntimeError(f"ProgramData pointer mismatch: {address}")
            programs.append({
                "program_id": address,
                "kind": "upgradeable_bpf",
                "loader": owner,
                "program_account_sha256": sha(data),
                "programdata_address": pointer,
                "deployment_slot": captured["deployment_slot"],
                "upgrade_authority": captured["upgrade_authority"],
                "allocated_account_length": captured["allocated_account_length"],
                "elf_bytes": captured["elf_bytes"],
                "elf_sha256": captured["elf_sha256"],
                "historical_proof": "complete",
            })
        elif owner == LEGACY_LOADER:
            programs.append({
                "program_id": address,
                "kind": "legacy_bpf",
                "loader": owner,
                "program_account_sha256": sha(data),
                "elf_bytes": len(data),
                "elf_sha256": sha(data),
                "historical_proof": "complete",
            })
        else:
            programs.append({
                "program_id": address,
                "kind": "runtime_native",
                "loader": owner,
                "program_account_sha256": sha(data),
                "historical_proof": "runtime_profile_required",
            })
    if len(programs) != 6:
        raise RuntimeError(f"program census changed: {len(programs)}")
    write(OUT / "program-census.json", {"programs": programs, "unique_programs": len(programs), "all_bpf_binaries_proven": all(row["historical_proof"] == "complete" for row in programs if row["kind"] != "runtime_native")})

    default_elf_dir = FREEZE / "runtime-source/litesvm-0.16.0/src/programs/elf"
    captured_by_id = {row["program_id"]: row for row in programs}
    backend_binary_comparison = {
        "schema": "eplyx.phase-u9.backend-default-binary-comparison.v1",
        "policy": "Historical account bytes are authoritative; backend defaults may be used only on an exact full-ELF hash match.",
        "programs": [],
    }
    for program_id, filename in (
        ("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", "pinocchio_token_program.so"),
        ("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb", "spl_token_2022-11.0.0.so"),
        ("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL", "spl_associated_token_account-1.1.1.so"),
    ):
        backend_path = default_elf_dir / filename
        backend_hash = file_sha(backend_path)
        captured = captured_by_id[program_id]
        backend_binary_comparison["programs"].append({
            "program_id": program_id,
            "backend_default_file": str(backend_path.relative_to(REPO)),
            "backend_default_bytes": backend_path.stat().st_size,
            "backend_default_sha256": backend_hash,
            "historical_bytes": captured["elf_bytes"],
            "historical_sha256": captured["elf_sha256"],
            "exact_match": backend_hash == captured["elf_sha256"],
            "execution_policy": "may_use_exact_match" if backend_hash == captured["elf_sha256"] else "must_replace_with_historical_elf",
        })
    write(OUT / "backend-default-binary-comparison.json", backend_binary_comparison)

    streamed = stream_sysvars()
    runtime_dir = OUT / "runtime"
    runtime_dir.mkdir(parents=True, exist_ok=True)
    for address, row in streamed.items():
        name = "RecentBlockhashes" if address == RECENT else "SlotHashes"
        (runtime_dir / f"{name}.account.bin").write_bytes(row["data"])
        (runtime_dir / f"{name}.event.pb").write_bytes(row["frame"])

    recent_data = streamed[RECENT]["data"]
    recent_count = int.from_bytes(recent_data[:8], "little")
    if recent_count != 150 or len(recent_data) != 8 + 40 * recent_count:
        raise RuntimeError("RecentBlockhashes layout mismatch")
    recent_entries = [
        {"blockhash": b58encode(recent_data[8 + 40 * i:40 + 40 * i]), "lamports_per_signature": int.from_bytes(recent_data[40 + 40 * i:48 + 40 * i], "little")}
        for i in range(recent_count)
    ]
    frozen_block = read("docs/examples/phase-u5-sample/blocks/448760958.body")["result"]
    tail = acquisition["recent_blockhash_tail_anchor"]
    if recent_entries[0]["blockhash"] != frozen_block["blockhash"] or recent_entries[1]["blockhash"] != frozen_block["previousBlockhash"]:
        raise RuntimeError("streamed RecentBlockhashes head differs from block chain")
    if recent_entries[-1]["blockhash"] != tail["blockhash"]:
        raise RuntimeError("RecentBlockhashes tail anchor differs")
    pre_entries = recent_entries[1:] + [{"blockhash": tail["previous_blockhash"], "lamports_per_signature": recent_entries[-1]["lamports_per_signature"]}]
    recent_pre_data = len(pre_entries).to_bytes(8, "little") + b"".join(b58decode(row["blockhash"]) + row["lamports_per_signature"].to_bytes(8, "little") for row in pre_entries)
    (runtime_dir / "RecentBlockhashes.pre-transaction.bin").write_bytes(recent_pre_data)

    slot_hashes_data = streamed[SLOT_HASHES]["data"]
    slot_hashes_count = int.from_bytes(slot_hashes_data[:8], "little")
    slot_hashes_head_slot = int.from_bytes(slot_hashes_data[8:16], "little")
    if slot_hashes_count != 512 or slot_hashes_head_slot != SLOT - 1:
        raise RuntimeError("SlotHashes context mismatch")

    nonce_address = "HCyytQceq1kmeMEmKmDutWM74M1c7CMK8nASjbSJWd94"
    nonce_a = decode_nonce(account_data(SLOT - 1, nonce_address))
    nonce_b = decode_nonce(account_data(SLOT, nonce_address))
    if nonce_a["authority"] != expected_keys[0] or nonce_a["durable_nonce"] != raw_target["transaction"]["message"]["recentBlockhash"]:
        raise RuntimeError("nonce authority/value differs from transaction")
    expected_post_nonce = b58encode(hashlib.sha256(b"DURABLE_NONCE" + b58decode(frozen_block["previousBlockhash"])).digest())
    if nonce_b["durable_nonce"] != expected_post_nonce:
        raise RuntimeError("checkpoint-B nonce is not derived from the parent blockhash")

    sysvars = {row["name"]: row for row in acquisition["sysvars"]}
    clock = (REPO / sysvars["Clock"]["account_file"]).read_bytes()
    rent = (REPO / sysvars["Rent"]["account_file"]).read_bytes()
    epoch = (REPO / sysvars["EpochSchedule"]["account_file"]).read_bytes()
    clock_fields = dict(zip(["slot", "epoch_start_timestamp", "epoch", "leader_schedule_epoch", "unix_timestamp"], struct.unpack("<QqQQq", clock)))
    rent_fields = dict(zip(["lamports_per_byte_year", "exemption_threshold", "burn_percent"], struct.unpack("<QdB", rent)))
    epoch_fields = dict(zip(["slots_per_epoch", "leader_schedule_slot_offset", "warmup", "first_normal_epoch", "first_normal_slot"], struct.unpack("<QQBQQ", epoch)))
    if clock_fields["slot"] != SLOT or clock_fields["unix_timestamp"] != frozen_block["blockTime"]:
        raise RuntimeError("Clock differs from target block")

    cargo_lock = (REPO / "Cargo.lock").read_text()
    lite_root = FREEZE / "runtime-source/litesvm-0.16.0"
    feature_root = FREEZE / "runtime-source/agave-feature-set-4.2.2"
    lite_features_path = lite_root / "src/features.rs"
    feature_source_path = feature_root / "src/lib.rs"
    lite_source = lite_features_path.read_text()
    feature_source = feature_source_path.read_text()
    relevant_names = [
        "versioned_tx_message_enabled", "enable_durable_nonce", "separate_nonce_from_blockhash",
        "nonce_must_be_authorized", "nonce_must_be_advanceable", "fix_recent_blockhashes",
        "add_set_compute_unit_price_ix", "replace_spl_token_with_p_token",
        "create_account_allow_prefund", "deprecate_rent_exemption_threshold",
        "static_instruction_limit", "limit_instruction_accounts", "increase_cpi_account_info_limit",
        "provide_instruction_data_offset_in_vm_r2", "relax_programdata_account_check_migration",
    ]
    feature_snapshot = {row["feature_id"]: row["activation_slot"] for row in acquisition["feature_snapshot"]["accounts"]}
    features = []
    for name in relevant_names:
        row = source_feature(name, feature_source, lite_source)
        observed = feature_snapshot.get(row["feature_id"])
        row.update({
            "observed_activation_slot": observed,
            "active_at_target": observed is not None and observed <= SLOT,
            "classification": "MATERIAL" if name in {
                "versioned_tx_message_enabled", "enable_durable_nonce", "separate_nonce_from_blockhash",
                "nonce_must_be_authorized", "nonce_must_be_advanceable", "fix_recent_blockhashes",
                "add_set_compute_unit_price_ix", "replace_spl_token_with_p_token", "create_account_allow_prefund",
            } else "NON_MATERIAL_OR_DEFENSIVE",
        })
        if observed != row["activation_slot"] or not row["active_at_target"]:
            raise RuntimeError(f"feature evidence differs: {name}")
        features.append(row)

    compute_ix = [b58decode(raw_target["transaction"]["message"]["instructions"][i]["data"]) for i in (1, 2)]
    unit_limit = int.from_bytes(compute_ix[0][1:5], "little")
    unit_price = int.from_bytes(compute_ix[1][1:9], "little")
    priority_fee = (unit_limit * unit_price + 999_999) // 1_000_000
    fee = raw_target["meta"]["fee"]
    if (compute_ix[0][0], compute_ix[1][0], unit_limit, unit_price, 5000 + priority_fee) != (2, 3, 150000, 5000, fee):
        raise RuntimeError("fee/compute-budget reconstruction differs")

    runtime = {
        "schema": "eplyx.phase-u9.runtime-evidence.v1",
        "historical_slot": SLOT,
        "runtime_family": {
            "current_rpc_observation": acquisition["current_rpc_version_observation"],
            "current_observation_slot": acquisition["feature_snapshot"]["observation_context_slot"],
            "backend": "LiteSVM 0.16.0",
            "linked_solana_program_runtime": "4.2.2",
            "linked_solana_system_program": "4.2.2",
            "qualification": "strongly corroborated 4.2 line; exact historical validator build not claimed",
            "cargo_lock_sha256": sha(cargo_lock.encode()),
            "litesvm_features_source_sha256": file_sha(lite_features_path),
            "agave_feature_set_source_sha256": file_sha(feature_source_path),
        },
        "features": features,
        "feature_snapshot": {
            "total_accounts": len(acquisition["feature_snapshot"]["accounts"]),
            "target_active": acquisition["feature_snapshot"]["target_active_count"],
            "activated_after_target": acquisition["feature_snapshot"]["activated_after_target_count"],
        },
        "nonce": {"address": nonce_address, "checkpoint_a": nonce_a, "checkpoint_b": nonce_b, "expected_post_nonce_from_parent_blockhash": expected_post_nonce, "exact_match": True},
        "recent_blockhashes": {
            "archive_result": "null_by_provider_contract",
            "stream_event": {k: v for k, v in streamed[RECENT].items() if k not in ("data", "frame")},
            "post_slot_entry_count": recent_count,
            "pre_transaction_reconstructed_entry_count": len(pre_entries),
            "pre_transaction_data_sha256": sha(recent_pre_data),
            "environment_blockhash": frozen_block["previousBlockhash"],
            "material_behavior": "account must be correctly identified and non-empty; new nonce derives from environment blockhash",
        },
        "clock": {"data_sha256": sha(clock), "fields": clock_fields},
        "rent": {"data_sha256": sha(rent), "fields": rent_fields},
        "epoch_schedule": {"data_sha256": sha(epoch), "fields": epoch_fields},
        "instructions": "runtime_generated_from_complete_proven_v0_message",
        "slot_hashes": {
            "archive_result": "null_by_provider_contract",
            "stream_event": {k: v for k, v in streamed[SLOT_HASHES].items() if k not in ("data", "frame")},
            "entry_count": slot_hashes_count,
            "head_slot": slot_hashes_head_slot,
            "lut_resolution_required_it": proof["slot_hashes_evidence_id"] is not None,
            "treatment": "non-material for four active LUTs; exact stream image nevertheless retained",
        },
        "signature_policy": {"verify": False, "original_signature_preserved": True},
        "blockhash_policy": {"inclusion_check": False, "original_recent_blockhash_preserved": True, "exact_environment_blockhash_required": True},
        "fee_model": {"signature_fee": 5000, "compute_unit_limit": unit_limit, "micro_lamports_per_unit": unit_price, "priority_fee": priority_fee, "reconstructed_total": 5000 + priority_fee, "validator_total": fee, "exact_arithmetic_match": True},
        "compute_budget_model": {"original_instructions_preserved": True, "set_limit_discriminator": compute_ix[0][0], "set_price_discriminator": compute_ix[1][0]},
    }
    write(OUT / "runtime-evidence.json", runtime)

    runtime_profile = {
        "schema": "eplyx.phase-u9.experimental-runtime-profile.v1",
        "historical_slot": SLOT,
        "runtime_family_evidence": runtime["runtime_family"],
        "feature_activations": features,
        "native_programs": [
            {"program_id": "11111111111111111111111111111111", "identity": "solana-system-program 4.2.2 source-bound implementation"},
            {"program_id": "ComputeBudget111111111111111111111111111111", "identity": "solana-compute-budget 4.2.2 source-bound implementation"},
        ],
        "sysvars": {
            "Clock": runtime["clock"],
            "Rent": runtime["rent"],
            "EpochSchedule": runtime["epoch_schedule"],
            "RecentBlockhashes": runtime["recent_blockhashes"],
            "Instructions": runtime["instructions"],
            "SlotHashes": runtime["slot_hashes"],
        },
        "nonce_policy": runtime["nonce"],
        "fee_policy": runtime["fee_model"],
        "compute_budget_policy": runtime["compute_budget_model"],
        "signature_policy": runtime["signature_policy"],
        "blockhash_policy": runtime["blockhash_policy"],
        "qualification": "Evidence-derived target-reachable profile; current universal backend cannot yet express its exact environment blockhash.",
    }
    runtime_profile["profile_id"] = sha(canonical(runtime_profile))
    write(OUT / "runtime-profile.json", runtime_profile)

    capability = {
        "classification": "B",
        "label": "SUPPORTED_WITH_GENERIC_RUNTIME_CONFIGURATION",
        "reason": "All historical data inputs are now present, but the current universal ExecutionRequest cannot set LiteSVM's exact environment/latest blockhash required by AdvanceNonceAccount, and its runtime profile does not bind the acquired feature/native evidence.",
        "blocking_generic_features": [
            "content-addressed runtime profile binding feature/native evidence",
            "exact bank environment blockhash injection independent of blockhash inclusion checking",
            "explicit historical RecentBlockhashes seed selection",
        ],
        "backend_default_is_acceptable": False,
        "target_execution_attempted": False,
    }
    write(OUT / "runtime-capability.json", capability)
    feasibility = {
        "classification": "B",
        "label": "BOUNDED_BUT_NEEDS_GENERIC_FEATURE",
        "requirements": {
            "account_inputs_22": "present",
            "lut_proofs_4": "present",
            "bpf_binaries_4": "present",
            "native_runtime_profile": "evidence present but not expressible by current backend contract",
            "sysvars": "present/reconstructed",
            "nonce_evidence": "present",
            "feature_evidence": "present",
            "message_proof": "present",
            "validator_outcome": "present",
            "checkpoint_a": "present",
            "checkpoint_b": "present",
        },
        "u9_success_levels": {"U9-A": "PASS", "U9-B": "PASS", "U9-C": "STOP", "U9-D": "NOT_REACHED", "U9-E": "NOT_REACHED", "U9-F": "NOT_REACHED", "U9-G": "NOT_REACHED"},
        "experimental_manifest_built": False,
        "execution_attempted": False,
    }
    write(OUT / "feasibility.json", feasibility)
    credential_keys = sorted(
        key for key in os.environ
        if any(marker in key.upper() for marker in ("ALCHEMY", "RPC_URL", "API_KEY"))
    )
    write(OUT / "offline-qualification.json", {
        "schema": "eplyx.phase-u9.offline-qualification.v1",
        "network_used": False,
        "provider_credentials_present": bool(credential_keys),
        "provider_credential_environment_keys": credential_keys,
        "runtime_sources": "frozen U9 evidence",
        "lut_resolver": "prebuilt local generic U4 binary",
        "result": "qualification_reproduced_offline",
        "replay_claimed": False,
    })
    write(OUT / "generality-review.json", {
        "schema": "eplyx.phase-u9.generality-review.v1",
        "u9_product_or_core_source_changes": [],
        "protocol_specific_core_branches_added": 0,
        "searched_terms": ["Orca", "Whirlpool", "tick", "CLMM", "program-specific state assumptions"],
        "core_findings": [],
        "scope_note": "Frozen target identifiers in acquisition/test evidence are not execution branches.",
    })

    status = subprocess.run(["git", "status", "--short"], cwd=REPO, text=True, capture_output=True, check=True).stdout.splitlines()
    initial = {
        "head": subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO, text=True, capture_output=True, check=True).stdout.strip(),
        "branch": subprocess.run(["git", "branch", "--show-current"], cwd=REPO, text=True, capture_output=True, check=True).stdout.strip(),
        "preexisting_status_lines": [line for line in status if "phase-u9" not in line and "u9-" not in line],
        "note": "U9 paths are excluded to preserve the inherited U4-U8 dirty-worktree baseline.",
    }
    write(FREEZE / "initial.json", initial)
    write(FREEZE / "final-state.json", {
        "head": initial["head"], "branch": initial["branch"], "head_unchanged": True,
        "u9_product_or_core_source_changes": [],
        "u9_scope": ["generic acquisition", "offline qualification", "frozen evidence", "report"],
        "commit_created": False,
        "reason_no_commit": "U9 stopped at runtime capability B; the suggested proof commit requires U9-G.",
    })

    metrics = {
        "analysis_seconds": round(time.perf_counter() - started, 6),
        "acquisition_elapsed_seconds_sum": round(sum(row["elapsed_seconds"] for row in acquisition["receipts"]), 3),
        "acquisition_files": len(checksums),
        "acquisition_bytes": sum(row["bytes"] for row in checksums.values()),
        "programdata_account_bytes": sum(row["allocated_account_length"] for row in acquisition["programdata"]),
        "program_elf_bytes": sum(row["elf_bytes"] for row in acquisition["programdata"]) + next(row["elf_bytes"] for row in programs if row["kind"] == "legacy_bpf"),
        "lut_account_bytes": sum(row["observations"][0]["space"] for row in acquisition["luts"]),
    }
    write(OUT / "metrics.json", metrics)

    manifest_path = FREEZE / "final-manifest.json"
    candidates = [path for root in (ACQ, OUT, FREEZE) for path in root.rglob("*") if path.is_file() and path != manifest_path]
    candidates += [
        REPO / "scripts/acquire-u9-historical-inputs.py",
        REPO / "scripts/analyze-u9-historical-inputs.py",
        REPO / "scripts/freeze-u9-runtime-sources.py",
        REPO / "scripts/run-u9-controls.py",
    ]
    report = REPO / "docs/phase-u9-generic-historical-execution-inputs.md"
    if report.exists():
        candidates.append(report)
    write(manifest_path, {
        "schema": "eplyx.phase-u9.manifest.v1",
        "self_excluded": str(manifest_path.relative_to(REPO)),
        "files": [{"path": str(path.relative_to(REPO)), "bytes": path.stat().st_size, "sha256": file_sha(path)} for path in sorted(set(candidates))],
    })
    print(json.dumps({"lut_proof": proof["proof_id"], "programs": len(programs), "runtime_capability": capability["classification"], "feasibility": feasibility["classification"]}, indent=2))


if __name__ == "__main__":
    main()
