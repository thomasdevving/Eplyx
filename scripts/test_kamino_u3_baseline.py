#!/usr/bin/env python3
"""Executable integrity tests with synthetic copies of real preflight captures.

The fixture is explicitly not production evidence. Admission is obtained from
the actual unchanged engine example, including a diagnostic LUT-bearing copy.
"""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import kamino_u3_baseline as baseline


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(baseline.canonical(value))


def b58encode(data):
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    number = int.from_bytes(data, "big")
    text = ""
    while number:
        number, remainder = divmod(number, 58)
        text = alphabet[remainder] + text
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + text


def make_fixture(root, count=3, failed=1):
    policy = baseline.read(baseline.DEFAULT_SAMPLE / "sampling-policy.json")
    policy.pop("frozen_source_hashes", None)
    policy.update(sample_version=0, signature_limit=count, transaction_fetch_limit=count)
    write(root / "sampling-policy.json", policy)
    originals = [baseline.read(baseline.REPO / f"docs/examples/phase-u3-before/tx-dep{i}.json") for i in (1, 2)]
    # Add one resolved key without changing any existing compiled instruction.
    lut = originals[1]["result"]
    message = lut["transaction"]["message"]
    key = message["accountKeys"][1]
    message["addressTableLookups"] = [{"accountKey": key, "writableIndexes": [], "readonlyIndexes": [0]}]
    lut["meta"]["loadedAddresses"] = {"writable": [], "readonly": [key]}
    for name in ("preBalances", "postBalances"):
        lut["meta"][name].append(0)
    source, members = [], []
    for index in range(count):
        envelope = copy.deepcopy(originals[index % 2])
        result = envelope["result"]
        signature = result["transaction"]["signatures"][0] if index < 2 else b58encode(hashlib.sha512(str(index).encode()).digest())
        result["transaction"]["signatures"][0] = signature
        origin = {"signature": signature, "slot": result["slot"], "err": result["meta"]["err"],
                  "blockTime": result.get("blockTime"), "memo": None, "confirmationStatus": "finalized", "source_index": index}
        source.append(origin)
        member = {"signature": signature, "source_index": index, "selected": True, "attempted": True,
                  "status": "success", "attempts": [], "response_file": None, "response_sha256": None, "failure_reason": None}
        if index >= count - failed:
            member.update(status="failure", failure_reason="transport_exit_28")
            member["attempts"] = [{"number": n, "error": "transport_exit_28", "response_file": None, "response_sha256": None} for n in range(1, 4)]
        else:
            reference = f"transactions/{signature}.json"
            write(root / reference, envelope)
            digest = baseline.sha((root / reference).read_bytes())
            member.update(response_file=reference, response_sha256=digest)
            member["attempts"] = [{"number": 1, "error": None, "response_file": reference, "response_sha256": digest}]
        members.append(member)
    write(root / "source-signatures.json", source)
    write(root / "fetch-membership.json", members)
    write(root / "rpc/source.json", {"jsonrpc": "2.0", "id": 1, "result": [{key: value for key, value in item.items() if key != "source_index"} for item in source]})
    write(root / "rpc/anchor.json", {"jsonrpc": "2.0", "id": 1, "result": 448171000})
    write(root / "rpc/genesis.json", {"jsonrpc": "2.0", "id": 1, "result": policy["expected_genesis_hash"]})
    receipt = {"kind": "capture_receipt", "fetch_complete": True,
               "provider": policy["provider"], "genesis_hash": policy["expected_genesis_hash"],
               "slot_start": 448161000, "slot_end": 448171000,
               "requests": {name: {"method": method, "params": params, "response_file": f"rpc/{name}.json"} for name, method, params in [
                   ("genesis", "getGenesisHash", []), ("anchor", "getSlot", [{"commitment": "finalized"}]),
                   ("source", "getSignaturesForAddress", [baseline.KLEND, {"limit": count, "commitment": "finalized"}])]},
               "get_transaction_params": ["<source signature>", policy["get_transaction_options"]]}
    write(root / "capture-receipt.json", receipt)
    relink(root)


def relink(root):
    receipt = baseline.read(root / "capture-receipt.json")
    receipt["raw_artifact_hashes"] = {str(path.relative_to(root)): baseline.sha(path.read_bytes()) for path in sorted(root.rglob("*")) if path.is_file() and path.name != "capture-receipt.json"}
    receipt["policy_sha256"] = receipt["raw_artifact_hashes"]["sampling-policy.json"]
    receipt["source_sha256"] = receipt["raw_artifact_hashes"]["source-signatures.json"]
    write(root / "capture-receipt.json", receipt)


class IntegrityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.fixture = Path(cls.temporary.name) / "fixture"
        control = os.environ.get("KAMINO_TEST_CONTROL_DIR")
        if control:
            shutil.copytree(Path(control) / "fixture", cls.fixture)
            cls.outputs = {name: (Path(control) / "derived" / name).read_bytes() for name in baseline.DERIVED}
        else:
            make_fixture(cls.fixture)
            cls.outputs = baseline.derive(cls.fixture)
        cls.source = baseline.read(cls.fixture / "source-signatures.json")
        cls.members = baseline.read(cls.fixture / "fetch-membership.json")

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def tables(self):
        return [copy.deepcopy(self.source), copy.deepcopy(self.members),
                json.loads(self.outputs["classifications.json"]), json.loads(self.outputs["supported-observations.json"]),
                json.loads(self.outputs["before-rejections.json"]), json.loads(self.outputs["summary.json"])]

    def fixture_copy(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name) / "sample"
        shutil.copytree(self.fixture, root)
        return root

    def test_valid_fixture_finalizes(self):
        baseline.validate_tables(*self.tables())
        summary = json.loads(self.outputs["summary.json"])
        self.assertEqual((summary["fetch_selected"], summary["fetch_success"], summary["fetch_failure"], summary["classified_rows"]), (3, 2, 1, 2))

    def test_unresolved_selected_fetch_rejected(self):
        tables = self.tables()
        tables[1][0]["status"] = "pending"
        with self.assertRaisesRegex(ValueError, "unresolved"):
            baseline.validate_tables(*tables)

    def test_199_of_200_classifications_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            make_fixture(root, count=200, failed=0)
            _, _, source, members = baseline.load_raw(root)
            outputs = baseline.derive(root)
            rows = json.loads(outputs["classifications.json"])[:-1]
            with self.assertRaisesRegex(ValueError, "classification count mismatch|classification membership mismatch"):
                baseline.validate_tables(source, members, rows, baseline.supported_rows(rows), baseline.before_rows(rows), baseline.summarize(source, members, rows))

    def test_progress_snapshot_before_classification_rejected(self):
        source, members, rows, _, _, _ = self.tables()
        rows = rows[:-1]
        with self.assertRaises(ValueError):
            baseline.validate_tables(source, members, rows, baseline.supported_rows(rows), baseline.before_rows(rows), baseline.summarize(source, members, rows))

    def test_duplicate_signature_rejected(self):
        duplicate = [self.source[0], copy.deepcopy(self.source[0])]
        with self.assertRaisesRegex(ValueError, "duplicate signature"):
            baseline.unique(duplicate, "signature")

    def test_duplicate_transaction_artifact_rejected(self):
        root = self.fixture_copy()
        member = baseline.read(root / "fetch-membership.json")[0]
        shutil.copyfile(root / member["response_file"], root / "transactions/duplicate.json")
        relink(root)
        with self.assertRaisesRegex(ValueError, "unlinked transaction artifact"):
            baseline.load_raw(root)

    def test_missing_raw_response_rejected(self):
        root = self.fixture_copy()
        (root / self.members[0]["response_file"]).unlink()
        with self.assertRaises(ValueError):
            baseline.load_raw(root)

    def test_raw_hash_missing_from_receipt_rejected(self):
        root = self.fixture_copy()
        receipt = baseline.read(root / "capture-receipt.json")
        receipt["raw_artifact_hashes"].pop(self.members[0]["response_file"])
        write(root / "capture-receipt.json", receipt)
        with self.assertRaisesRegex(ValueError, "hash inventory"):
            baseline.load_raw(root)

    def test_raw_hash_inventory_in_final_manifest(self):
        manifest = json.loads(baseline.derive(self.fixture)["manifest.json"])
        expected = set(baseline.read(self.fixture / "capture-receipt.json")["raw_artifact_hashes"]) | {"capture-receipt.json"}
        self.assertEqual(set(manifest["raw_artifact_hashes"]), expected)

    def test_supported_count_must_be_derivable(self):
        tables = self.tables()
        tables[3].pop()
        with self.assertRaisesRegex(ValueError, "supported observation"):
            baseline.validate_tables(*tables)

    def test_lut_supported_identity_preserved(self):
        rows = json.loads(self.outputs["classifications.json"])
        expected = {(row["signature"], ix["outer_index"]) for row in rows for ix in row["klend_instructions"] if ix["supported_action_family"]}
        actual = {(item["signature"], item["outer_index"]) for item in baseline.supported_rows(rows)}
        self.assertTrue(any(row["lookup_table_count"] for row in rows))
        self.assertEqual(actual, expected)

    def test_aggregate_mismatch_rejected(self):
        tables = self.tables()
        tables[5]["klend_top_level"] += 1
        with self.assertRaisesRegex(ValueError, "aggregate"):
            baseline.validate_tables(*tables)

    def test_action_counts_from_rows(self):
        source, members, rows, _, _, _ = self.tables()
        summary = baseline.summarize(source, members, rows)
        for family in ("deposit", "borrow"):
            expected = sum(row["success"] and ix["supported_action_family"] == family for row in rows for ix in row["klend_instructions"])
            self.assertEqual(summary["successful_supported_actions"][family], expected)

    def test_failed_fetch_cannot_be_classified_irrelevant(self):
        source, members, rows, _, _, _ = self.tables()
        fake = copy.deepcopy(rows[0])
        origin = source[-1]
        fake.update(signature=origin["signature"], slot=origin["slot"], source_index=origin["source_index"], klend_instructions=[], contains_klend_top_level=False)
        fake["old_engine"]["capture_sha256"] = None
        rows.append(fake)
        with self.assertRaises(ValueError):
            baseline.validate_tables(source, members, rows, baseline.supported_rows(rows), baseline.before_rows(rows), baseline.summarize(source, members, rows))

    def test_checkpoint_cannot_finalize(self):
        with self.assertRaisesRegex(ValueError, "checkpoint"):
            baseline.validate_tables(*self.tables(), complete=False)

    def test_checkpoint_cannot_be_capture_receipt(self):
        root = self.fixture_copy()
        receipt = baseline.read(root / "capture-receipt.json")
        receipt.update(kind="checkpoint", complete=False, fetch_complete=False)
        write(root / "capture-receipt.json", receipt)
        with self.assertRaisesRegex(ValueError, "checkpoint"):
            baseline.load_raw(root)

    def test_unrelated_source_even_overlapping_slots_rejected(self):
        root = self.fixture_copy()
        source = baseline.read(root / "source-signatures.json")
        source[0]["signature"] = source[-1]["signature"] + "1"
        write(root / "source-signatures.json", source)
        relink(root)  # Self-consistent hashes alone do not establish source linkage.
        with self.assertRaisesRegex(ValueError, "not linked"):
            baseline.load_raw(root)

    def test_explicit_source_link_required(self):
        root = self.fixture_copy()
        receipt = baseline.read(root / "capture-receipt.json")
        receipt["source_sha256"] = "0" * 64
        write(root / "capture-receipt.json", receipt)
        with self.assertRaisesRegex(ValueError, "explicit source"):
            baseline.load_raw(root)

    def test_raw_evidence_preserved(self):
        rows = json.loads(self.outputs["classifications.json"])
        for row in rows:
            origin = next(item for item in self.members if item["signature"] == row["signature"])
            result = baseline.read(self.fixture / origin["response_file"])["result"]
            self.assertEqual(row["rpc_evidence"]["message"], result["transaction"]["message"])
            for key in ("loadedAddresses", "innerInstructions", "preBalances", "postBalances", "preTokenBalances", "postTokenBalances"):
                if key in result["meta"]:
                    self.assertEqual(row["rpc_evidence"]["meta"][key], result["meta"][key])

    def test_before_rejection_reason_retained(self):
        rows = json.loads(self.outputs["classifications.json"])
        for row in baseline.before_rows(rows):
            self.assertTrue(row["old_engine"].get("first_blocker", {}).get("detail"))
            self.assertEqual(row["old_engine"]["first_blocker"]["class"], "B")

    def test_engine_actual_first_rejections(self):
        rows = json.loads(self.outputs["classifications.json"])
        self.assertIn("unsupported program 11111111111111111111111111111111", rows[0]["old_engine"]["first_blocker"]["detail"])
        self.assertEqual(rows[1]["old_engine"]["first_blocker"]["detail"], "message resolves 1 address lookup table entries; lookup tables are normalized but not executed")
        self.assertTrue(all(not row["old_engine"]["execution_attempted"] for row in rows))

    def test_fingerprint_ignores_input_enumeration(self):
        receipt = baseline.read(self.fixture / "capture-receipt.json")
        args = (receipt["raw_artifact_hashes"], receipt["policy_sha256"])
        self.assertEqual(baseline.fingerprint(self.source, self.members, *args), baseline.fingerprint(list(reversed(self.source)), list(reversed(self.members)), *args))

    def test_byte_determinism_reverse_enumeration(self):
        with patch.object(Path, "rglob", lambda path, pattern: iter(reversed(list(original_rglob(path, pattern))))):
            reverse = baseline.derive(self.fixture, reverse=True)
        self.assertEqual(reverse, self.outputs)

    def test_final_manifest_has_no_credentials(self):
        manifest = json.loads(baseline.derive(self.fixture)["manifest.json"])
        self.assertNotIn("api_key", manifest)
        baseline.hygiene(manifest)

    def test_secret_and_url_hygiene_rejects_keys(self):
        for bad in ({"api_key": "mutation-secret"}, {"provider": "https://host/v2/mutation-secret"}, {"provider": "https://host/?api-key=mutation-secret"}, {"provider": "https://user:mutation-secret@host/"}):
            with self.assertRaises(ValueError):
                baseline.hygiene(bad)

    def test_fixture_transient_account_detection(self):
        rows = json.loads(self.outputs["classifications.json"])
        self.assertEqual(len(rows[1]["transient_accounts"]), 1)
        self.assertFalse(rows[1]["transient_accounts"][0]["historical_account_bytes_verified"])

    def test_two_lifecycle_cycles_do_not_cross_pair(self):
        create = {"program": baseline.SYSTEM, "data_hex": (bytes(4) + bytes(48)).hex(), "accounts": ["payer", "target"], "outer_index": 1, "inner_index": 0}
        close = {"program": next(iter(baseline.TOKENS)), "data_hex": "09", "accounts": ["target", "payer", "payer"], "outer_index": 2, "inner_index": None}
        create2, close2 = copy.deepcopy(create), copy.deepcopy(close)
        create2["outer_index"], close2["outer_index"] = 3, 4
        result = {"meta": {"err": None, "preBalances": [0], "postBalances": [0]}}
        self.assertEqual(len(baseline.transient_cases(result, ["target"], [create, close, create2, close2])), 2)

    def test_normalization_and_newer_versions_record_actual_stage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            make_fixture(root, count=2, failed=0)
            members = baseline.read(root / "fetch-membership.json")
            for index, member in enumerate(members):
                path = root / member["response_file"]
                envelope = baseline.read(path)
                if index == 0:
                    del envelope["result"]["transaction"]["message"]["recentBlockhash"]
                else:
                    envelope["result"]["version"] = 1
                write(path, envelope)
                member["response_sha256"] = member["attempts"][0]["response_sha256"] = baseline.sha(path.read_bytes())
            write(root / "fetch-membership.json", members)
            relink(root)
            rows = json.loads(baseline.derive(root)["classifications.json"])
            self.assertEqual([row["message_distribution"] for row in rows], ["unreadable", "newer_unsupported"])
            self.assertTrue(all(row["old_engine"]["first_blocker"]["class"] == "A" for row in rows))
            self.assertTrue(all(row["old_engine"]["admission"] == "not_reached" for row in rows))

    def test_offline_derive_removes_rpc_and_secret_environment(self):
        actual_run = subprocess.run
        invocations = []
        def offline_run(command, **kwargs):
            self.assertEqual(command[0], str(baseline.REPO / "target/debug/examples/classify_kamino_baseline"))
            self.assertTrue(all(Path(path).is_file() for path in command[1:]))
            self.assertNotIn("SOLANA_RPC_URL", kwargs["env"])
            self.assertNotIn("ALCHEMY_API_KEY", kwargs["env"])
            invocations.append(command)
            return actual_run(command, **kwargs)
        with patch.dict(os.environ, {"SOLANA_RPC_URL": "https://invalid.test/?api-key=synthetic-secret", "ALCHEMY_API_KEY": "synthetic-secret"}), patch.object(subprocess, "run", offline_run):
            self.assertEqual(baseline.derive(self.fixture), self.outputs)
        self.assertEqual(len(invocations), 1)

    def test_capture_preserves_non_json_failures_and_hides_url_key(self):
        spec = importlib.util.spec_from_file_location("capture", baseline.REPO / "scripts/capture-kamino-u3-sample.py")
        capture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(capture)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            policy = baseline.read(self.fixture / "sampling-policy.json")
            write(root / "sampling-policy.json", policy)
            responses = {member["signature"]: (self.fixture / member["response_file"]).read_bytes() for member in self.members if member["status"] == "success"}
            def mock_rpc(command, **kwargs):
                request = json.loads(command[command.index("--data") + 1])
                if request["method"] == "getTransaction":
                    body = responses.get(request["params"][0], b"synthetic non-JSON provider failure")
                else:
                    name = {"getGenesisHash": "genesis", "getSlot": "anchor", "getSignaturesForAddress": "source"}[request["method"]]
                    body = (self.fixture / f"rpc/{name}.json").read_bytes()
                return subprocess.CompletedProcess(command, 0, body, b"")
            with patch.object(sys, "argv", ["capture", str(root)]), patch.dict(os.environ, {"SOLANA_RPC_URL": policy["provider"] + "/v2/synthetic-secret", "SOLANA_RPC_ORIGIN": "https://www.alchemy.com"}), patch.object(subprocess, "run", mock_rpc), patch.object(capture.time, "sleep"), patch("builtins.print"):
                capture.main()
            _, receipt, _, membership = baseline.load_raw(root)
            failed = next(member for member in membership if member["status"] == "failure")
            self.assertEqual(len(failed["attempts"]), 3)
            for attempt in failed["attempts"]:
                self.assertEqual((root / attempt["response_file"]).read_bytes(), b"synthetic non-JSON provider failure")
            self.assertFalse(any(b"synthetic-secret" in path.read_bytes() for path in root.rglob("*") if path.is_file()))
            self.assertEqual(receipt["provider"], policy["provider"])


original_rglob = Path.rglob

if __name__ == "__main__":
    unittest.main()
