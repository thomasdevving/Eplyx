#!/usr/bin/env python3
"""Re-run the frozen U3/U4 product controls for Phase U8."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "docs/examples/phase-u8-freeze/kamino-controls"
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
    (OUT / f"{case}.stdout").write_bytes(process.stdout)
    (OUT / f"{case}.stderr").write_bytes(process.stderr)
    digest = hashlib.sha256(process.stdout).hexdigest()
    return {
        "case": case,
        "command": args,
        "environment": "PATH-only" if path_only else "ordinary",
        "exit": process.returncode,
        "stdout_sha256": digest,
        "expected_stdout_sha256": EXPECTED[case],
        "matches_frozen_control": process.returncode == 0 and digest == EXPECTED[case],
    }


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    cli = str(REPO / "target/debug/eplyx")
    verify = [
        cli,
        "bundle",
        "verify",
        "--bundle",
        "docs/examples/phase-u4-kamino-bundle",
        "--format",
        "json",
    ]
    baseline = [
        cli,
        "ci",
        "check",
        "--bundle",
        "docs/examples/phase-u4-kamino-bundle",
        "--candidate",
        "docs/examples/phase-u4-kamino-bundle/binaries/current.so",
        "--format",
        "json",
    ]
    results = [
        run("verify", verify, False),
        run("baseline", baseline, False),
        run("offline_verify", verify, True),
        run("offline_ci", baseline, True),
    ]
    (OUT.parent / "kamino-controls.json").write_bytes(canonical(results))
    print(json.dumps(results, indent=2))
    return 0 if all(row["matches_frozen_control"] for row in results) else 1


if __name__ == "__main__":
    sys.exit(main())
