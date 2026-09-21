#!/usr/bin/env python3
"""Re-run frozen product controls for Phase U10 without touching U9 artifacts."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "docs/examples/phase-u10-freeze"
EXPECTED = {
    "verify": "529355d96e95bf81c44debd50a007f3624612009cffcf5cee47cc631d05e3b18",
    "baseline": "b7f33953ca35bcdc7949a52a9ea5263739e1be20a1c00ddc1d0d7c89f1a8fe44",
    "offline_verify": "529355d96e95bf81c44debd50a007f3624612009cffcf5cee47cc631d05e3b18",
    "offline_ci": "b7f33953ca35bcdc7949a52a9ea5263739e1be20a1c00ddc1d0d7c89f1a8fe44",
}


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def run(case: str, args: list[str], path_only: bool) -> dict[str, object]:
    env = {"PATH": os.environ["PATH"]} if path_only else os.environ.copy()
    process = subprocess.run(args, cwd=REPO, env=env, capture_output=True, check=False)
    (OUT / f"kamino-{case}.stdout").write_bytes(process.stdout)
    (OUT / f"kamino-{case}.stderr").write_bytes(process.stderr)
    digest = hashlib.sha256(process.stdout).hexdigest()
    return {
        "case": case,
        "environment": "PATH-only" if path_only else "ordinary",
        "exit": process.returncode,
        "stdout_sha256": digest,
        "expected_stdout_sha256": EXPECTED[case],
        "matches_frozen_control": process.returncode == 0 and digest == EXPECTED[case],
    }


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    stake_out = OUT / "stake-pool-controls"
    stake = subprocess.run(
        ["python3", "scripts/verify-kamino-u3-controls.py", "target/debug/eplyx", str(stake_out)],
        cwd=REPO,
        capture_output=True,
        check=False,
    )
    (OUT / "stake-control-run.stdout").write_bytes(stake.stdout)
    (OUT / "stake-control-run.stderr").write_bytes(stake.stderr)
    cli = str(REPO / "target/debug/eplyx")
    verify = [cli, "bundle", "verify", "--bundle", "docs/examples/phase-u4-kamino-bundle", "--format", "json"]
    baseline = [cli, "ci", "check", "--bundle", "docs/examples/phase-u4-kamino-bundle", "--candidate", "docs/examples/phase-u4-kamino-bundle/binaries/current.so", "--format", "json"]
    kamino = [
        run("verify", verify, False),
        run("baseline", baseline, False),
        run("offline_verify", verify, True),
        run("offline_ci", baseline, True),
    ]
    result = {
        "schema": "eplyx.phase-u10.controls.v1",
        "stake_exit": stake.returncode,
        "stake": json.loads((stake_out / "controls.json").read_text()),
        "kamino": kamino,
        "all_match": stake.returncode == 0 and all(row["matches_frozen_control"] for row in kamino),
    }
    (OUT / "controls.json").write_bytes(canonical(result))
    print(json.dumps(result, indent=2))
    return 0 if result["all_match"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
