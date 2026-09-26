#!/usr/bin/env python3
"""Append T7 reference encodings without rewriting T0 or T6 records."""
import hashlib
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
PIN = "562fb6969ccdea0cd47014b66ea10ad9f74935ac0fd42b0ef1d8c16045dc3d62"
T6_PIN = "b22b039003289f6a61a4d63c7dd18c8c2b79f12f52b5b6aac9444c686c1da3e8"
base = ROOT / "fixtures/path"
manifest_bytes = (base / "provenance.json").read_bytes()
t6_bytes = (ROOT / "fixtures/lifecycle/encoding-projection.json").read_bytes()
if hashlib.sha256(manifest_bytes).hexdigest() != PIN or hashlib.sha256(t6_bytes).hexdigest() != T6_PIN:
    raise ValueError("Accepted reference contract changed")
manifest = json.loads(manifest_bytes)
t6 = json.loads(t6_bytes)
hashes = {v["source_sha256"]:v["main_sha256"] for v in [*t6["files"], *t6["derived_hashes"]]}
# These two path records contain no other wide quantities encoded as JSON numbers.
fields = {"captured_slot", "slot", "epoch", "epoch_start_timestamp", "leader_schedule_epoch", "unix_timestamp", "deployment_slot", "compute_units", "transaction_fee_lamports", "last_updated_at"}
def convert(value, key=""):
    if isinstance(value, str): return hashes.get(value, value)
    if isinstance(value, bool) or value is None: return value
    if isinstance(value, int): return str(value) if key in fields else value
    if isinstance(value, float): raise ValueError("Unexpected float in path reference")
    if isinstance(value, list): return [convert(v,key) for v in value]
    return {k:convert(v,k) for k,v in value.items()}
outputs = [];records = []
for entry in manifest["files"]:
    name=entry["file"]
    if Path(name).is_absolute() or ".." in Path(name).parts: raise ValueError("Invalid member")
    source=base/"sta"/name
    if any(p.is_symlink() for p in [source,*source.parents]): raise ValueError("Symlink in reference path")
    if source.stat().st_size != entry["bytes"]: raise ValueError("Source size changed")
    with source.open("rb") as f: original=f.read(entry["bytes"]+1)
    if hashlib.sha256(original).hexdigest()!=entry["sha256"]: raise ValueError("Source digest changed")
    data=original if name.startswith("assets/") else (json.dumps(convert(json.loads(original)),ensure_ascii=False,indent=2)+"\n").encode()
    outputs.append((base/"main"/name,data))
    records.append({"file":name,"source_sha256":entry["sha256"],"main_sha256":hashlib.sha256(data).hexdigest(),"source_bytes":len(original),"main_bytes":len(data)})
record=(json.dumps({"schema_version":1,"fields":sorted(fields),"t6_projection_sha256":T6_PIN,"files":records},indent=2)+"\n").encode()
outputs.append((base/"encoding-projection.json",record))
for path,data in outputs:
    if path.exists() and path.read_bytes()!=data: raise ValueError("Existing derivative changed")
for path,data in outputs:
    path.parent.mkdir(parents=True,exist_ok=True)
    if not path.exists():
        with path.open("xb") as f:f.write(data)
print("Verified three T7 reference encodings; no economic findings derived.")
