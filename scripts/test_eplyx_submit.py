"""Deterministic HTTP contract tests for the hosted CI client; no live service."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]
CLIENT = ROOT / "scripts/eplyx-submit.sh"
TOKEN = "eplyx_proj_" + "a" * 64


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def answer(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        state = self.server.state
        if self.headers.get("Authorization") != f"Bearer {TOKEN}":
            return self.answer(401, {"error": "unauthorized"})
        if self.path == "/v1/projects/proj_test/capabilities":
            if state["mode"] == "server_error":
                return self.answer(503, {"error": "unavailable"})
            ready = state["mode"] != "not_ready"
            return self.answer(200, {"project_id": "proj_test", "analyses": [{
                "kind": "program_upgrade", "status": "ready" if ready else "not_ready",
                "can_submit": ready, "missing": [] if ready else [{
                    "code": "active_bundle_missing", "action": "Activate a replay bundle."}]}]})
        if self.path == "/v1/projects/proj_test/setup":
            return self.answer(200, {"project_id": "proj_test", "steps": [
                {"id": "bundle_registered", "title": "Replay evidence registered", "status": "done",
                 "actions": []},
                {"id": "bundle_active", "title": "Baseline activated", "status": "todo",
                 "actions": [{"label": "Activate the reviewed bundle", "actor": "operator",
                              "command": "eplyx-server admin activate-bundle --project proj_test --bundle bndl_1"}]}]})
        if self.path == "/v1/runs/run_test":
            state["polls"] += 1
            if state["mode"] == "poll_error":
                return self.answer(503, {"error": "unavailable"})
            if state["mode"] == "timeout" or state["polls"] == 1:
                return self.answer(200, {"status": "running"})
            mode = state["mode"]
            code = 1 if mode == "regression" else 2 if mode in ("coverage", "preflight_abort") else 0
            error = mode == "execution_error"
            return self.answer(200, {
                "run_id": "run_test", "project_id": "proj_test",
                "status": "execution_error" if error else "passed" if code == 0 else "failed",
                "exit_code": None if error else code,
                "report_available": not error and mode != "preflight_abort",
                "detail": "Invalid candidate" if error or mode == "preflight_abort" else None,
                "candidate_sha256": "wrong" if mode == "run_mismatch" else state["sha"],
                "bundle_sha256": "other-bundle" if mode == "bundle_mismatch" else "bundle-test",
                "baseline_sha256": "baseline-test",
                "corpus_sha256": "corpus-test", "record_count": 2,
                "change": {"change_spec_id": "change-test", "target_program_id": "target-test"}})
        if self.path == "/v1/runs/run_test/report.json":
            mode = state["mode"]
            code = 1 if mode == "regression" else 2 if mode == "coverage" else 0
            return self.answer(200, {
                "change": {"change_spec_id": "change-test"},
                "candidate": {"sha256": "wrong" if mode == "report_mismatch" else state["sha"]},
                "findings": [{}] if code == 1 else [],
                "summary": {"exit_code": code, "expected": 0, "unexpected": int(code == 1),
                            "failure_reasons": ["undeclared_change"] if code == 1 else
                            ["no_semantic_coverage"] if code == 2 else []},
                "bundle": {"limitations": []}})
        return self.answer(404, {"error": "missing"})

    def do_POST(self):
        state = self.server.state
        state["submits"] += 1
        size = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(size)
        if self.headers.get("Authorization") != f"Bearer {TOKEN}":
            return self.answer(401, {"error": "unauthorized"})
        if self.path != "/v1/projects/proj_test/checks":
            return self.answer(404, {"error": "missing"})
        return self.answer(202, {
            "run_id": "run_test", "project_id": "proj_test",
            "candidate_sha256": "wrong" if state["mode"] == "accepted_mismatch" else state["sha"],
            "change": {"change_spec_id": "change-test"},
            "bundle_sha256": "bundle-test"})


class ClientContract(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name)
        self.candidate = self.dir / "candidate.so"
        self.candidate.write_bytes(b"exact candidate bytes")
        self.state = {"mode": "clean", "submits": 0, "polls": 0,
                      "sha": hashlib.sha256(self.candidate.read_bytes()).hexdigest()}
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.state = self.state
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.url = f"http://127.0.0.1:{self.server.server_port}"

    def run_client(self, mode="clean", token=TOKEN, url=None):
        self.state.update(mode=mode, submits=0, polls=0)
        return subprocess.run(["bash", str(CLIENT), "--api", url or self.url,
                               "--web-url", "https://eplyx.example", "--project", "proj_test",
                               "--candidate", str(self.candidate),
                               "--report-json", str(self.dir / "report.json"),
                               "--summary", str(self.dir / "summary.md")],
                              env={**os.environ, "EPLYX_TOKEN": token,
                                   "EPLYX_POLL_SECONDS": "1", "EPLYX_TIMEOUT_SECONDS": "2"},
                              text=True, capture_output=True, timeout=12)

    def test_clean_and_repeat_runs_keep_exact_identity_and_link(self):
        for _ in range(2):
            result = self.run_client()
            self.assertEqual(result.returncode, 0, result.stderr)
            summary = (self.dir / "summary.md").read_text()
            self.assertIn(self.state["sha"], summary)
            self.assertIn("bundle-test", summary)
            self.assertIn("https://eplyx.example/p/proj_test/runs/run_test", summary)
            self.assertIn("PASS within", summary)
            self.assertIn("### Reproduce", summary)
            self.assertIn("eplyx ci check --bundle", summary)
            self.assertNotIn(TOKEN, result.stdout + result.stderr + summary)
            self.assertEqual(self.state["submits"], 1)

    def test_regression_and_missing_coverage_are_distinct(self):
        regression = self.run_client("regression")
        self.assertEqual(regression.returncode, 1, regression.stderr)
        self.assertIn("Analytical regression", (self.dir / "summary.md").read_text())
        coverage = self.run_client("coverage")
        self.assertEqual(coverage.returncode, 2, coverage.stderr)
        self.assertIn("Evidence or coverage limitation", (self.dir / "summary.md").read_text())

    def test_readiness_and_auth_fail_before_upload(self):
        blocked = self.run_client("not_ready")
        self.assertEqual(blocked.returncode, 76, blocked.stderr)
        self.assertEqual(self.state["submits"], 0)
        summary = (self.dir / "summary.md").read_text()
        self.assertIn("active_bundle_missing", summary)
        self.assertIn("### Setup checklist", summary)
        self.assertIn("| Baseline activated | **to do** | Activate the reviewed bundle "
                      "(`eplyx-server admin activate-bundle --project proj_test --bundle bndl_1`) |", summary)
        unauthorized = self.run_client(token="wrong")
        self.assertEqual(unauthorized.returncode, 70)
        self.assertEqual(self.state["submits"], 0)

    def test_identity_and_execution_fail_closed(self):
        for mode, code in [("accepted_mismatch", 70), ("run_mismatch", 70),
                           ("report_mismatch", 70), ("bundle_mismatch", 70),
                           ("execution_error", 75), ("preflight_abort", 2)]:
            result = self.run_client(mode)
            self.assertEqual(result.returncode, code, (mode, result.stderr))
        self.assertIn("no analytical report", (self.dir / "summary.md").read_text().lower())

    def test_server_poll_and_network_failure_are_not_regression(self):
        for mode in ("server_error", "poll_error", "timeout"):
            result = self.run_client(mode)
            self.assertEqual(result.returncode, 70, (mode, result.stderr))
        network = self.run_client(url="http://127.0.0.1:1")
        self.assertEqual(network.returncode, 70)
        self.assertNotIn("Analytical regression", network.stderr)


if __name__ == "__main__":
    unittest.main()
