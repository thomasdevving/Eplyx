#!/usr/bin/env python3
"""Run ten real source mutations in isolated copies, with assertion-only kills.

The production source is never edited. Each mutated copy is restored byte for
byte, and a surviving mutant or non-assertion failure makes this command fail.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

import kamino_u3_baseline as baseline
from test_kamino_u3_baseline import make_fixture

MUTATIONS = [
    (1, "199 classifications accepted for 200 successful fetches", "test_199_of_200_classifications_rejected", [
        ('    require(len(rows) == len(success), "successful fetch/classification count mismatch")\n', ''),
        ('    require(set(classified) == success, "classification membership mismatch")\n', ''),
    ]),
    (2, "LUT-supported identity omitted from detail", "test_lut_supported_identity_preserved", [
        ('for row in rows for ix in row["klend_instructions"] if ix["supported_action_family"]]',
         'for row in rows for ix in row["klend_instructions"] if ix["supported_action_family"] and row["lookup_table_count"] == 0]'),
    ]),
    (3, "independent extra deposit aggregate", "test_action_counts_from_rows", [
        ('sum(item["family"] == family for item in successful)',
         '(sum(item["family"] == family for item in successful) + int(family == "deposit"))'),
    ]),
    (4, "failed fetch admitted as classified irrelevant row", "test_failed_fetch_cannot_be_classified_irrelevant", [
        ('success = {item["signature"] for item in selected if item["status"] == "success"}',
         'success = {item["signature"] for item in selected}'),
    ]),
    (5, "raw response hash omitted from manifest", "test_raw_hash_inventory_in_final_manifest", [
        ('"raw_artifact_hashes": dict(sorted(raw_hashes.items())),',
         '"raw_artifact_hashes": {name: value for name, value in raw_hashes.items() if name != "rpc/source.json"},'),
    ]),
    (6, "duplicate signature silently overwrites", "test_duplicate_signature_rejected", [
        ('        require(key not in result, f"duplicate {field}: {key}")\n', ''),
    ]),
    (7, "API key written into final manifest", "test_final_manifest_has_no_credentials", [
        ('    hygiene(manifest)\n', '    manifest["api_key"] = "mutation-secret"\n'),
    ]),
    (8, "irrelevant source enumeration alters fingerprint", "test_fingerprint_ignores_input_enumeration", [
        ('"source": sorted(source, key=lambda item: item["source_index"]),', '"source": source,'),
    ]),
    (9, "checkpoint treated as complete", "test_checkpoint_cannot_finalize", [
        ('    require(complete, "checkpoint cannot finalize")\n', ''),
    ]),
    (10, "old-engine reason omitted from supported BEFORE row", "test_before_rejection_reason_retained", [
        ('"companion_programs": row["companion_programs"], "old_engine": row["old_engine"]}',
         '"companion_programs": row["companion_programs"], "old_engine": {key: value for key, value in row["old_engine"].items() if key != "first_blocker"}}'),
    ]),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    production = baseline.REPO / "scripts/kamino_u3_baseline.py"
    original_bytes = production.read_bytes()
    original_hash = hashlib.sha256(original_bytes).hexdigest()
    results = []
    with tempfile.TemporaryDirectory(prefix="eplyx-u3a-mutants-") as directory:
        root = Path(directory)
        control = root / "control"
        make_fixture(control / "fixture")
        outputs = baseline.derive(control / "fixture")
        (control / "derived").mkdir()
        for name, data in outputs.items():
            (control / "derived" / name).write_bytes(data)
        scripts = root / "scripts"
        scripts.mkdir()
        test_bytes = (baseline.REPO / "scripts/test_kamino_u3_baseline.py").read_bytes()
        (scripts / "test_kamino_u3_baseline.py").write_bytes(test_bytes)
        # Only bootstrap the isolated copy to read the real unchanged engine.
        source = original_bytes.decode().replace('REPO = Path(__file__).resolve().parents[1]',
                                                f'REPO = Path({str(baseline.REPO)!r})')
        copied = scripts / "kamino_u3_baseline.py"
        copied.write_text(source)
        env = dict(os.environ, KAMINO_TEST_CONTROL_DIR=str(control), PYTHONDONTWRITEBYTECODE="1")
        env.pop("PYTHONPATH", None)
        for number, description, name, replacements in MUTATIONS:
            changed = source
            for old, new in replacements:
                if changed.count(old) != 1:
                    raise ValueError(f"mutation {number} target must occur exactly once")
                changed = changed.replace(old, new)
            copied.write_text(changed)
            try:
                process = subprocess.run(["python3", str(scripts / "test_kamino_u3_baseline.py"),
                                          f"IntegrityTests.{name}"], cwd=root, env=env, capture_output=True)
                stderr = process.stderr.decode()
                killed = process.returncode != 0 and "AssertionError" in stderr and "FAIL:" in stderr and "ERROR:" not in stderr
                results.append({"mutation": number, "description": description, "named_test": name,
                                "result": "killed_by_assertion" if killed else "FAILED",
                                "test_exit": process.returncode, "mutated_source_sha256": hashlib.sha256(changed.encode()).hexdigest()})
                print(f"M{number:02}: {results[-1]['result']} — {name}", flush=True)
                if not killed:
                    raise ValueError(f"mutation {number} survived or failed outside an assertion:\n{stderr}")
            finally:
                copied.write_text(source)
                if copied.read_bytes() != source.encode():
                    raise ValueError("mutated source restoration failed")
    if production.read_bytes() != original_bytes:
        raise ValueError("production source changed during isolated mutations")
    report = {"mutations_executed": len(results), "killed_by_named_assertions": len(results),
              "production_source_sha256_before": original_hash,
              "production_source_sha256_after": hashlib.sha256(production.read_bytes()).hexdigest(),
              "all_isolated_sources_restored": True, "results": results}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_bytes(baseline.canonical(report))


if __name__ == "__main__":
    main()
