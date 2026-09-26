#!/usr/bin/env python3
"""Import pinned lifecycle reference bytes from a local STA archive, without network."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PIN = "c411ff7226bb515533774ec822829f787fe4bf6910f9816dbbf2e6f2b55baa3e"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sta", type=Path, required=True)
    source = parser.parse_args().sta.resolve(strict=True)
    target = ROOT / "fixtures/lifecycle/sta"
    data = (target.parent / "provenance.json").read_bytes()
    if hashlib.sha256(data).hexdigest() != PIN:
        raise ValueError("Lifecycle fixture allowlist differs from its reviewed pin")
    manifest = json.loads(data)
    pending = []
    for entry in manifest["files"]:
        relative = Path(entry["source"])
        if relative.is_absolute() or ".." in relative.parts or entry["file"] != entry["source"]:
            raise ValueError("Invalid pinned artifact path")
        original = source / relative
        destination = target / relative
        for root, path in [(source, original), (target, destination)]:
            while path != root:
                if path.is_symlink():
                    raise ValueError("Artifact path traverses a symlink")
                path = path.parent
        if original.stat().st_size != entry["bytes"]:
            raise ValueError(f"Fixture size mismatch: {relative}")
        with original.open("rb") as handle:
            content = handle.read(entry["bytes"] + 1)
        if len(content) != entry["bytes"] or hashlib.sha256(content).hexdigest() != entry["sha256"]:
            raise ValueError(f"Fixture digest mismatch: {relative}")
        if destination.exists():
            if destination.stat().st_size != len(content) or hashlib.sha256(destination.read_bytes()).hexdigest() != entry["sha256"]:
                raise ValueError(f"Existing artifact changed: {relative}")
        else:
            pending.append((destination, original, entry))
    # Verify everything before writing. Recheck each bounded read during the copy.
    for destination, original, entry in pending:
        with original.open("rb") as handle:
            content = handle.read(entry["bytes"] + 1)
        if len(content) != entry["bytes"] or hashlib.sha256(content).hexdigest() != entry["sha256"]:
            raise ValueError("Source artifact changed during import")
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as handle:
            handle.write(content)
    print(f"Verified {len(manifest['files'])} exact lifecycle artifacts; imported {len(pending)}.")


if __name__ == "__main__":
    main()
