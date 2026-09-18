#!/usr/bin/env python3
"""Verify unchanged product reports against preserved U2 hashes, without RPC."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

import kamino_u3_baseline as baseline


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/eplyx").resolve()
    output = Path(sys.argv[2] if len(sys.argv) > 2 else "data/phase-u3a-controls/after")
    output.mkdir(parents=True, exist_ok=True)
    expected = baseline.read(baseline.REPO / "docs/examples/phase-u3-before/controls.json")
    env = {key: value for key, value in os.environ.items() if not any(word in key for word in ("RPC", "API_KEY", "ARCHIVE"))}
    cases = []
    for case in expected["cases"]:
        name = case["case"]
        if name == "verify":
            args = ["bundle", "verify", "--bundle", "deploy/bundle"]
        else:
            candidate = "artifacts/fixture_stake_pool_v2.so" if name in ("regression", "bounded") else "deploy/bundle/binaries/current.so"
            args = ["ci", "check", "--bundle", "deploy/bundle", "--candidate", candidate, "--format", "json"]
            if name in ("bounded", "stale", "unevaluable"):
                args += ["--expectations", f"docs/pilot/expected-changes.{name}.toml"]
        process = subprocess.run([str(binary), *args], cwd=baseline.REPO, env=env, capture_output=True)
        digest = hashlib.sha256(process.stdout).hexdigest()
        baseline.require((process.returncode, digest) == (case["exit_code"], case["stdout_sha256"]), f"product control changed: {name}")
        (output / f"{name}.stdout").write_bytes(process.stdout)
        cases.append({"case": name, "exit_code": process.returncode, "stdout_sha256": digest, "matches_preserved_u2_bytes": True})
        print(f"{name}: exit {process.returncode}, unchanged {digest}", flush=True)
    report = {"control_commit": "8855edb", "rpc_environment_removed": True, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "cases": cases}
    (output / "controls.json").write_bytes(baseline.canonical(report))


if __name__ == "__main__":
    main()
