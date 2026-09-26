#!/usr/bin/env python3
"""Refuse silently re-pinned candidate artifacts. Missing bytes are failures."""
from pathlib import Path
import hashlib

artifacts = Path(__file__).resolve().parent.parent / "artifacts"
expected = {
    "eplyx_token_migration.so": "e5db6948abca1317eb12155f73cdaf619d378c22d1bfc992c977063e10e9c1bb",
    "eplyx_token_migration_defect_deadline_inclusive.so": "c4346736008188befe1d1a41cf167ea6a80c3b61a76cf1d3ba550f916b682275",
    "eplyx_token_migration_defect_fee_ceiling.so": "a93aff2f19f249234950ac6646f97f2c36e492a43ddff073796372a037382c74",
}
for name, digest in expected.items():
    actual = hashlib.sha256((artifacts / name).read_bytes()).hexdigest()
    if actual != digest:
        raise SystemExit(f"{name}: expected {digest}, found {actual}; use platform-tools v1.57, SBF v3")
    print(f"{name}: {actual}")
