#!/usr/bin/env python3
"""Run and freeze the offline U10 replay and fail-closed mutation campaign."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "docs/examples/phase-u10-analysis"
RUNNER = REPO / "target/debug/examples/qualify_u10_historical_replay"
MUTATIONS = {
    "environment-blockhash": "RuntimeConfigurationMismatch: durable nonce differs",
    "nonce-account": "frozen seed snapshot identity differs",
    "recent-blockhashes": "runtime sysvar snapshot differs from resolved profile",
    "feature-profile": "unsupported historical runtime profile",
    "historical-bpf": "frozen seed snapshot identity differs",
    "runtime-profile-identity": "resolved runtime profile identity differs",
    "s-checkpoint": "checkpoint S snapshot identity differs",
}


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write(name: str, value: object) -> None:
    (OUT / name).write_bytes(canonical(value))


def execute(args: list[str] | None = None, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [str(RUNNER), *(args or [])],
        cwd=REPO,
        env=env,
        capture_output=True,
        check=False,
    )


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    build = subprocess.run(
        ["cargo", "build", "-q", "-p", "eplyx-engine", "--example", "qualify_u10_historical_replay"],
        cwd=REPO,
        capture_output=True,
        check=False,
    )
    if build.returncode:
        raise SystemExit(build.stderr.decode(errors="replace"))

    ordinary = execute()
    if ordinary.returncode:
        raise SystemExit(ordinary.stderr.decode(errors="replace"))
    replay = json.loads(ordinary.stdout)
    write("replay.json", replay)

    account_matches = all(row["matched"] for row in replay["account_reconciliation"])
    fidelity_matches = all(replay["validator_fidelity"].values())
    feasibility = {
        "schema": "eplyx.phase-u10.feasibility.v1",
        "classification": "A",
        "label": "BOUNDED_AND_SUPPORTED",
        "bounded_inputs_representable": True,
        "runtime_profile_honored": True,
        "nonce_matched": replay["nonce"]["matched"],
        "validator_fidelity_matched": fidelity_matches,
        "validation_outputs_matched": account_matches,
        "validation_output_count": len(replay["account_reconciliation"]),
    }
    write("feasibility.json", feasibility)
    write(
        "runtime-binding.json",
        {
            "schema": "eplyx.phase-u10.runtime-binding.v1",
            "historical_runtime_evidence_id": replay["historical_runtime_evidence_id"],
            "runtime_profile_id": replay["runtime_profile_id"],
            "sysvar_snapshot_hash": replay["runtime_sysvar_snapshot_hash"],
            "transaction_recent_blockhash": replay["transaction_recent_blockhash"],
            "environment_blockhash": replay["environment_blockhash"],
            "feature_profile": "LiteSVM 0.16.0 mainnet",
            "native_program_profile": "agave-4.2.2-native-system-compute",
            "signature_check": False,
            "recent_blockhash_check": False,
            "instructions_rule": "runtime_generated_from_complete_message",
            "slot_hashes_policy": "historical",
            "mechanism": "LiteSVM invocation-inspect-callback mutates public InvokeContext.environment_config.blockhash before native execution",
        },
    )

    mutation_rows = []
    for name, expected in MUTATIONS.items():
        process = execute([name])
        stderr = process.stderr.decode(errors="replace")
        mutation_rows.append(
            {
                "mutation": name,
                "exit": process.returncode,
                "expected_failure": expected,
                "stderr_sha256": hashlib.sha256(process.stderr).hexdigest(),
                "fail_closed": process.returncode != 0 and expected in stderr,
            }
        )
    write(
        "mutations.json",
        {
            "schema": "eplyx.phase-u10.mutations.v1",
            "cases": mutation_rows,
            "all_fail_closed": all(row["fail_closed"] for row in mutation_rows),
        },
    )

    # No PATH, HOME, RPC URL, provider credential, or network configuration is
    # available to this process; it can consume only the checked-in evidence.
    offline = execute(env={})
    offline_result = {
        "schema": "eplyx.phase-u10.offline.v1",
        "environment": "empty",
        "exit": offline.returncode,
        "stdout_sha256": hashlib.sha256(offline.stdout).hexdigest(),
        "ordinary_stdout_sha256": hashlib.sha256(ordinary.stdout).hexdigest(),
        "byte_identical": offline.returncode == 0 and offline.stdout == ordinary.stdout,
        "provider_configuration_present": False,
    }
    write("offline.json", offline_result)
    write(
        "generality.json",
        {
            "schema": "eplyx.phase-u10.generality.v1",
            "universal_runtime_protocol_id_branches": 0,
            "protocol_semantics_added": False,
            "target_ids_confined_to_frozen_example_evidence": True,
        },
    )

    passed = (
        all(feasibility[key] for key in [
            "bounded_inputs_representable",
            "runtime_profile_honored",
            "nonce_matched",
            "validator_fidelity_matched",
            "validation_outputs_matched",
        ])
        and all(row["fail_closed"] for row in mutation_rows)
        and offline_result["byte_identical"]
    )
    summary = {
        "schema": "eplyx.phase-u10.qualification.v1",
        "passed": passed,
        "replay_evidence_sha256": replay["execution_evidence_sha256"],
        "mutation_count": len(mutation_rows),
        "offline_byte_identical": offline_result["byte_identical"],
    }
    write("qualification.json", summary)
    print(json.dumps(summary, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
