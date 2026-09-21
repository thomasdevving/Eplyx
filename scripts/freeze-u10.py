#!/usr/bin/env python3
"""Freeze hashes for the bounded Phase U10 implementation and proof."""

from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "docs/examples/phase-u10-freeze"
BASE_COMMIT = "0e862a9"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    prior = REPO / "docs/examples/phase-u9-freeze/final-manifest.json"
    (OUT / "initial.json").write_bytes(
        canonical(
            {
                "schema": "eplyx.phase-u10.initial.v1",
                "base_commit": BASE_COMMIT,
                "base_commit_full": subprocess.check_output(
                    ["git", "rev-parse", BASE_COMMIT], cwd=REPO, text=True
                ).strip(),
                "initial_worktree": "clean after the user-requested pre-U10 commit",
                "prior_u9_manifest_sha256": sha256(prior),
            }
        )
    )
    tracked = [
        "Cargo.lock",
        "Cargo.toml",
        "engine/Cargo.toml",
        "engine/examples/execute_universal_v0.rs",
        "engine/examples/import_universal_observation.rs",
        "engine/examples/measure_universal_replay.rs",
        "engine/examples/qualify_u10_historical_replay.rs",
        "engine/src/replay.rs",
        "engine/src/universal/execution.rs",
        "engine/src/universal/model.rs",
        "engine/src/universal/pipeline.rs",
        "engine/src/universal/resolver.rs",
        "engine/tests/universal_runtime_profile.rs",
        "scripts/freeze-u10.py",
        "scripts/run-u10-controls.py",
        "scripts/run-u10-qualification.py",
        "docs/phase-u10-generic-historical-runtime-binding.md",
    ]
    tracked.extend(
        str(path.relative_to(REPO))
        for root in [
            REPO / "docs/examples/phase-u10-analysis",
            REPO / "docs/examples/phase-u10-freeze",
        ]
        for path in sorted(root.rglob("*"))
        if path.is_file() and path.name not in {"final-manifest.json", "final-state.json"}
    )
    tracked = sorted(set(tracked))
    entries = []
    for relative in tracked:
        path = REPO / relative
        entries.append({"path": relative, "bytes": path.stat().st_size, "sha256": sha256(path)})
    manifest = {
        "schema": "eplyx.phase-u10.final-manifest.v1",
        "base_commit": BASE_COMMIT,
        "files": entries,
        "file_count": len(entries),
        "all_inputs_local": True,
        "feasibility": "BOUNDED_AND_SUPPORTED",
        "strong_success": True,
    }
    (OUT / "final-manifest.json").write_bytes(canonical(manifest))
    status = subprocess.check_output(["git", "status", "--short"], cwd=REPO, text=True)
    (OUT / "final-state.json").write_bytes(
        canonical(
            {
                "schema": "eplyx.phase-u10.final-state.v1",
                "base_commit": BASE_COMMIT,
                "worktree_status_before_u10_commit": status.splitlines(),
                "manifest_sha256": sha256(OUT / "final-manifest.json"),
            }
        )
    )


if __name__ == "__main__":
    main()
