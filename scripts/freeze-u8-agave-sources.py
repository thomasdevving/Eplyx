#!/usr/bin/env python3
"""Freeze the pinned Agave failure/rollback semantics used by Phase U8."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "docs/examples/phase-u8-freeze/source"
COMMIT = "965aee8e55d45ac3ca72e48f15945bfca4a32804"
FILES = {
    "svm-spec.md": (
        Path("/tmp/agave-svm-spec.md"),
        "svm/doc/spec.md",
        "1f8f0926dc0a6f84fdd1c40b867e7a0f87916fd66349d456583c3a3ad0388126",
    ),
    "transaction-processor.rs": (
        Path("/tmp/agave-transaction-processor.rs"),
        "svm/src/transaction_processor.rs",
        "a41d5b466b13c0aa2f63a42d05ded092ad8ef8399109bb85d15546f7cdc09509",
    ),
    "rollback-accounts.rs": (
        Path("/tmp/agave-rollback-accounts.rs"),
        "svm/src/rollback_accounts.rs",
        "05fc22177dd28df5cc1708cb968ca88e77c5ea4bce4e6e23852c2f76ae440c52",
    ),
    "account-loader.rs": (
        Path("/tmp/agave-account-loader.rs"),
        "svm/src/account_loader.rs",
        "7553f764b949344c19f43ccea269a7889617932df4719c980b9ed6a5dd8a851c",
    ),
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    records = []
    for output_name, (source, upstream_path, expected) in FILES.items():
        data = source.read_bytes()
        actual = digest(data)
        if actual != expected:
            raise SystemExit(f"unexpected hash for {source}: {actual}")
        destination = OUT / output_name
        destination.write_bytes(data)
        records.append({
            "path": str(destination.relative_to(REPO)),
            "bytes": len(data),
            "sha256": actual,
            "upstream_commit": COMMIT,
            "upstream_path": upstream_path,
            "url": f"https://github.com/anza-xyz/agave/blob/{COMMIT}/{upstream_path}",
        })
    provenance = {
        "repository": "https://github.com/anza-xyz/agave",
        "commit": COMMIT,
        "tag_observed_during_acquisition": "v4.3.0-rc.0",
        "purpose": "Pin failed-transaction rollback semantics: fee-payer and durable-nonce rollback accounts.",
        "files": records,
    }
    (OUT / "provenance.json").write_text(json.dumps(provenance, sort_keys=True, separators=(",", ":")) + "\n")
    print(json.dumps(provenance, indent=2))


if __name__ == "__main__":
    main()
