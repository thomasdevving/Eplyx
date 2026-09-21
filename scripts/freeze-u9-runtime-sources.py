#!/usr/bin/env python3
"""Freeze the exact crate sources used for the Phase U9 runtime decision."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "docs/examples/phase-u9-freeze/runtime-source"
REGISTRY = Path.home() / ".cargo/registry/src"
PACKAGES = {
    "litesvm-0.16.0": [
        "src/lib.rs",
        "src/features.rs",
        "src/utils/mod.rs",
        "src/programs/mod.rs",
        "src/programs/elf/pinocchio_token_program.so",
        "src/programs/elf/spl_token_2022-11.0.0.so",
        "src/programs/elf/spl_associated_token_account-1.1.1.so",
        "Cargo.toml",
        ".cargo_vcs_info.json",
    ],
    "agave-feature-set-4.2.2": ["src/lib.rs", "Cargo.toml", ".cargo_vcs_info.json"],
    "solana-system-program-4.2.2": ["src/system_instruction.rs", "src/system_processor.rs", "Cargo.toml", ".cargo_vcs_info.json"],
    "solana-compute-budget-4.2.2": ["src/lib.rs", "src/compute_budget.rs", "src/compute_budget_limits.rs", "Cargo.toml", ".cargo_vcs_info.json"],
    "solana-fee-4.2.2": ["src/lib.rs", "Cargo.toml", ".cargo_vcs_info.json"],
    "solana-fee-structure-3.0.0": ["src/lib.rs", "Cargo.toml", ".cargo_vcs_info.json"],
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def locate(package: str) -> Path:
    matches = sorted(REGISTRY.glob(f"*/{package}"))
    if len(matches) != 1:
        raise SystemExit(f"expected one registry source for {package}, found {len(matches)}")
    return matches[0]


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    records = []
    packages = []
    for package, relative_files in PACKAGES.items():
        source_root = locate(package)
        package_name, version = package.rsplit("-", 1)
        vcs = json.loads((source_root / ".cargo_vcs_info.json").read_text())
        package_records = []
        for relative in relative_files:
            source = source_root / relative
            data = source.read_bytes()
            destination = OUT / package / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
            row = {
                "path": str(destination.relative_to(REPO)),
                "source_relative_path": relative,
                "bytes": len(data),
                "sha256": digest(data),
            }
            records.append(row)
            package_records.append(row)
        packages.append({
            "package": package_name,
            "version": version,
            "vcs_commit": vcs["git"]["sha1"],
            "path_in_vcs": vcs.get("path_in_vcs"),
            "files": package_records,
        })
    provenance = {
        "schema": "eplyx.phase-u9.runtime-source-provenance.v1",
        "purpose": "Pin the backend, feature-set, System nonce, and Compute Budget sources used for the U9 capability decision.",
        "authority": "Cargo.lock-pinned crates.io packages; VCS commits are package metadata, not a claim about the historical validator build.",
        "packages": packages,
        "files": records,
    }
    (OUT / "provenance.json").write_text(json.dumps(provenance, sort_keys=True, separators=(",", ":")) + "\n")
    print(json.dumps({"packages": len(packages), "files": len(records)}, indent=2))


if __name__ == "__main__":
    main()
