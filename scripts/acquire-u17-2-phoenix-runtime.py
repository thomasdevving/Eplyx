#!/usr/bin/env python3
"""Acquire the backend-known feature universe and runtime sysvars at the Phoenix slot."""

import argparse
import importlib.machinery
import json
import os
from pathlib import Path
import time
from urllib.parse import urlsplit

HELPERS = importlib.machinery.SourceFileLoader(
    "u17_2_state", str(Path(__file__).with_name("acquire-u17-2-phoenix-state.py"))
).load_module()
SOURCE = HELPERS.ROOT / "docs/examples/phase-u13-2b-feature-universe/audit.json"
SYSVARS = {
    "Clock": "SysvarC1ock11111111111111111111111111111111",
    "Rent": "SysvarRent111111111111111111111111111111111",
    "EpochSchedule": "SysvarEpochSchedu1e111111111111111111111111",
    "RecentBlockhashes": "SysvarRecentB1ockHashes11111111111111111111",
    "SlotHashes": "SysvarS1otHashes111111111111111111111111111",
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    root = args.root
    state = json.loads((root / "acquisition.json").read_text())
    endpoint = os.environ.get("SOLANA_ARCHIVE_RPC_URL") or os.environ.get("SOLANA_RPC_URL") or HELPERS.PUBLIC_ARCHIVE
    if urlsplit(endpoint).hostname != state["provider_host"]:
        raise RuntimeError("runtime archive host differs from state archive")
    audit = json.loads(SOURCE.read_text())
    features = sorted({row["id"] for row in audit["observations"]})
    if len(features) != audit["unionCount"]:
        raise RuntimeError("feature universe differs from pinned U13.2B inventory")
    work = [("sysvar", name, address) for name, address in SYSVARS.items()]
    work += [("feature", address, address) for address in features]
    rows = []
    for index, (kind, name, address) in enumerate(work):
        file = root / "raw" / f"{kind}-{name}.json"
        row = HELPERS.account(endpoint, file, address, HELPERS.TARGET)
        row.update({"kind": kind, "name": name})
        rows.append(row)
        if index % 25 == 0 or index + 1 == len(work):
            print(json.dumps({"acquired": index + 1, "total": len(work),
                              "present": sum(r["present"] for r in rows)}), flush=True)
        time.sleep(1.25)
    manifest = {"schema": "U17_2PhoenixRuntimeAcquisitionV1", "provider_host": state["provider_host"],
                "slot": HELPERS.TARGET, "feature_source": str(SOURCE.relative_to(HELPERS.ROOT)),
                "feature_source_sha256": HELPERS.sha(SOURCE.read_bytes()),
                "feature_count": len(features), "receipts": rows}
    (root / "runtime-acquisition.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"complete": True, "feature_count": len(features),
                      "sysvars": {r["name"]: r["present"] for r in rows if r["kind"] == "sysvar"}}))


if __name__ == "__main__":
    main()
