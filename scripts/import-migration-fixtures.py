#!/usr/bin/env python3
"""Import exact archived captures without reading credentials or printing endpoints."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_COMMIT = "ad1897e86ee05ff2255bd5277518a3230ee2340f"
DIRECTORIES = ("pinned-programs", "spacex", "current-reference")
# An explicit allowlist prevents a modified manifest from naming credentials.
SOURCES = {
    "reports/milestone8-underfunded/population.capture.json",
    "reports/milestone8-second-asset/population.capture.json",
    "reports/milestone4-validation/live-market.capture.json",
    "reports/milestone8-healthy-worker/population.capture.json",
    "reports/milestone8-healthy-worker/conversion.capture.json",
    "reports/milestone8-healthy-worker/stress.cases.json",
    "reports/milestone8-healthy-worker/bindings.json",
}


def verify(data, entry):
    if len(data) != entry["bytes"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
        raise ValueError(f"Archived fixture does not match pinned bytes: {entry['file']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sta", type=Path, required=True, help="Local checkout of the pinned STA archive")
    args = parser.parse_args()
    source = args.sta.resolve(strict=True)
    pending = []
    for directory in DIRECTORIES:
        target = ROOT / "fixtures" / "migration" / directory
        manifest = json.loads((target / "provenance.json").read_text())
        if manifest["sta_commit"] != SOURCE_COMMIT:
            raise ValueError("Unexpected archive commit in fixture manifest")
        for entry in manifest["files"]:
            if entry["source"] not in SOURCES or Path(entry["file"]).name != entry["file"]:
                raise ValueError("Fixture manifest names an unapproved source or destination")
            original = (source / entry["source"]).resolve(strict=True)
            if not original.is_relative_to(source):
                raise ValueError("Archived fixture escapes its source root")
            # Size check precedes allocation; no arbitrary file or hidden provider
            # configuration is ever opened. Digest verification is authoritative.
            if original.stat().st_size != entry["bytes"]:
                raise ValueError(f"Archived fixture size differs: {entry['file']}")
            with original.open("rb") as handle:
                data = handle.read(entry["bytes"] + 1)
            verify(data, entry)
            destination = target / entry["file"]
            if destination.exists():
                verify(destination.read_bytes(), entry)
            else:
                pending.append((destination, data))
    # Validate the complete import before writing; existing evidence is never replaced.
    for destination, data in pending:
        with destination.open("xb") as handle:
            handle.write(data)
        print(f"Imported {destination.relative_to(ROOT)} ({len(data)} bytes)")
    print("All frozen migration fixture hashes and sizes verified.")


if __name__ == "__main__":
    main()
