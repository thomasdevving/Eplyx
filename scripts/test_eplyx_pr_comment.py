"""PR comment client against a fake GitHub API; no network."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[1]
CLIENT = ROOT / "scripts/eplyx-pr-comment.py"
TOKEN = "ghs_test"


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def answer(self, status, body=None):
        data = json.dumps(body).encode() if body is not None else b""
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def body(self):
        return json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))

    def guard(self):
        state = self.server.state
        if self.headers.get("Authorization") != f"Bearer {TOKEN}":
            self.answer(401, {"message": "Bad credentials"})
            return False
        if state["read_only"] and self.command != "GET":
            self.answer(403, {"message": "Resource not accessible by integration"})
            return False
        return True

    def do_GET(self):
        if not self.guard():
            return
        state = self.server.state
        if self.path.startswith("/repos/o/r/issues/7/comments"):
            page = int(self.path.rsplit("page=", 1)[1])
            # 150 unrelated comments first: the marker has to be found on page 2.
            everything = [{"id": 1000 + n, "body": "unrelated"} for n in range(150)]
            everything += list(state["comments"].values())
            return self.answer(200, everything[(page - 1) * 100:page * 100])
        self.answer(404, {"message": "Not Found"})

    def do_POST(self):
        if not self.guard():
            return
        state = self.server.state
        if self.path == "/repos/o/r/issues/7/comments":
            state["next"] += 1
            state["comments"][state["next"]] = {"id": state["next"], "body": self.body()["body"]}
            state["posts"] += 1
            return self.answer(201, state["comments"][state["next"]])
        self.answer(404, {"message": "Not Found"})

    def do_PATCH(self):
        if not self.guard():
            return
        state = self.server.state
        comment = int(self.path.rsplit("/", 1)[1])
        if self.path.startswith("/repos/o/r/issues/comments/") and comment in state["comments"]:
            state["comments"][comment]["body"] = self.body()["body"]
            state["patches"] += 1
            return self.answer(200, state["comments"][comment])
        self.answer(404, {"message": "Not Found"})


class CommentContract(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.dir = Path(temp.name)
        self.state = {"comments": {}, "next": 0, "posts": 0, "patches": 0, "read_only": False}
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        server.state = self.state
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.api = f"http://127.0.0.1:{server.server_port}"

    def comment(self, summary, code, project="proj_a", token=TOKEN):
        path = self.dir / "summary.md"
        path.write_text(summary)
        return subprocess.run(
            ["python3", str(CLIENT), "--repo", "o/r", "--pr", "7", "--summary", str(path),
             "--exit-code", str(code), "--project", project, "--sha", "0123456789abcdef",
             "--run-url", "https://github.com/o/r/actions/runs/99", "--api-url", self.api],
            env={**os.environ, "GITHUB_TOKEN": token}, text=True, capture_output=True, timeout=20)

    def bodies(self):
        return [c["body"] for c in self.state["comments"].values()]

    def test_one_comment_is_created_then_updated_in_place(self):
        first = self.comment("## Eplyx — Analytical regression\n\n| Run | `run_1` |", 1)
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(self.state["posts"], 1)
        body = self.bodies()[0]
        self.assertTrue(body.startswith("<!-- eplyx-check:proj_a -->"))
        self.assertIn("**Unexpected change.**", body)
        self.assertIn("`run_1`", body)
        self.assertIn("https://github.com/o/r/actions/runs/99", body)
        self.assertIn("`0123456789ab`", body)

        second = self.comment("## Eplyx — PASS within the active bundle's coverage", 0)
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual((self.state["posts"], self.state["patches"]), (1, 1))
        self.assertEqual(len(self.bodies()), 1)
        self.assertIn("**Passed.**", self.bodies()[0])
        self.assertIn("not a statement that the upgrade is safe", self.bodies()[0])
        self.assertNotIn("run_1", self.bodies()[0])

    def test_projects_keep_separate_comments(self):
        self.comment("first", 0, project="proj_a")
        self.comment("second", 0, project="proj_b")
        self.assertEqual(self.state["posts"], 2)

    def test_no_verdict_and_not_ready_are_never_described_as_verdicts(self):
        self.comment("## Eplyx — no analytical verdict", 75)
        self.assertIn("not a statement about the candidate", self.bodies()[0])
        self.comment("## Eplyx — project not ready\n\n### Setup checklist", 76)
        self.assertIn("**Project not ready.**", self.bodies()[0])
        self.comment("", 70)
        self.assertIn("**Integration error.**", self.bodies()[0])
        self.assertIn("No summary was produced", self.bodies()[0])

    def test_oversized_summary_is_truncated_under_the_limit(self):
        self.comment("| row |\n" * 20000, 1)
        body = self.bodies()[0]
        self.assertLess(len(body), 65536)
        self.assertIn("Summary truncated", body)

    def test_read_only_token_is_a_warning_and_bad_credentials_too(self):
        self.state["read_only"] = True
        fork = self.comment("summary", 1)
        self.assertEqual(fork.returncode, 0, fork.stderr)
        self.assertIn("::warning::", fork.stdout)
        self.assertEqual(self.state["posts"], 0)
        missing = self.comment("summary", 1, token="")
        self.assertEqual(missing.returncode, 2)

    def test_unreachable_github_is_an_error(self):
        path = self.dir / "summary.md"
        path.write_text("x")
        result = subprocess.run(
            ["python3", str(CLIENT), "--repo", "o/r", "--pr", "7", "--summary", str(path),
             "--exit-code", "0", "--api-url", "http://127.0.0.1:1"],
            env={**os.environ, "GITHUB_TOKEN": TOKEN}, text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 1)


if __name__ == "__main__":
    unittest.main()
