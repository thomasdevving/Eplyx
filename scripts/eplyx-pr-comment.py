#!/usr/bin/env python3
"""Post or update one Eplyx comment on a pull request.

The comment is the step summary `eplyx-submit.sh` already wrote, headed by a
one-paragraph explanation of the gate result and followed by links to the
evidence: the workflow run (whose artefact holds report.json, report.md and
the summary) and the commit it measured. It is a projection of the hosted
result, never a second analysis, and it says nothing the summary does not.

One comment per pull request and project, updated in place on every push,
found by a hidden marker. A token without write access (pull requests from
forks) is a warning, not a failure: the gate result is the job's exit code,
and a comment is only a convenience on top of it.

Usage:
  GITHUB_TOKEN=... scripts/eplyx-pr-comment.py --repo owner/name --pr 12 \\
    --summary eplyx-summary.md --exit-code 1 \\
    [--run-url https://github.com/owner/name/actions/runs/1] [--sha abc123] \\
    [--project proj_...] [--api-url https://api.github.com]

Exit codes: 0 posted, updated or skipped for lack of permission;
1 GitHub refused or could not be reached; 2 bad arguments.
"""
import argparse
import json
import os
import sys
import urllib.error
import urllib.request

# GitHub's limit is 65536 characters; leave room for the header and footer.
MAX_SUMMARY_CHARS = 60000

# What each job result means, in one paragraph. These mirror the workflow's
# own messages, and stay as careful: a pass is bounded by the corpus, and a
# result without a verdict is never described as a verdict.
EXPLANATIONS = {
    "0": "**Passed.** No disallowed difference was observed when this candidate "
         "replayed the project's production-derived corpus. This is bounded by "
         "that corpus's coverage; it is not a statement that the upgrade is safe.",
    "1": "**Unexpected change.** Replaying production transactions against this "
         "candidate produced semantic changes that `expected-changes.toml` does not "
         "declare, or that exceed their declared bounds. Review the findings below; "
         "if a change is intended, declare it narrowly.",
    "2": "**Not evaluated.** Eplyx could not evaluate this candidate (configuration, "
         "fidelity, or no semantic coverage). That says nothing about the candidate "
         "either way.",
    "3": "**Stale declaration.** An entry in `expected-changes.toml` no longer "
         "describes anything this candidate changes. Remove or narrow it.",
    "4": "**Incompatible.** The candidate or its declared change does not fit the "
         "project's active bundle, so nothing was compared.",
    "5": "**Unevaluable declaration.** A declaration cannot be evaluated by this "
         "corpus; the evidence to check it is not in the bundle.",
    "75": "**No verdict.** The hosted run ended in `execution_error`. That is an "
          "infrastructure outcome, not a statement about the candidate; re-run the job.",
    "76": "**Project not ready.** The Eplyx project is missing a prerequisite, so no "
          "candidate was uploaded. The checklist below names the next step.",
}
FALLBACK = ("**Integration error.** The Eplyx client could not trust its own result "
            "(transport, input or identity verification failed). See the job log.")


def marker(project):
    return f"<!-- eplyx-check:{project or 'default'} -->"


def compose(summary, exit_code, run_url, sha, project):
    explanation = EXPLANATIONS.get(str(exit_code), FALLBACK)
    if len(summary) > MAX_SUMMARY_CHARS:
        summary = (summary[:MAX_SUMMARY_CHARS].rsplit("\n", 1)[0]
                   + "\n\n_Summary truncated; the full text is in the job summary and evidence artefact._")
    if not summary.strip():
        summary = "_No summary was produced; see the job log._"
    links = []
    if run_url:
        links.append(f"[Workflow run and evidence artefact]({run_url}) "
                     "(`eplyx-evidence`: report.json, report.md, summary)")
    if sha:
        links.append(f"Measured commit `{sha[:12]}`")
    footer = " · ".join(links)
    parts = [marker(project), explanation, "", summary.rstrip()]
    if footer:
        parts += ["", "---", footer]
    return "\n".join(parts) + "\n"


class GitHub:
    def __init__(self, api, token, repo):
        self.api = api.rstrip("/")
        self.token = token
        self.repo = repo

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(
            self.api + path, data=data, method=method,
            headers={"Authorization": f"Bearer {self.token}",
                     "Accept": "application/vnd.github+json",
                     "X-GitHub-Api-Version": "2022-11-28",
                     "Content-Type": "application/json",
                     "User-Agent": "eplyx-pr-comment"})
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = response.read()
            return json.loads(payload) if payload else None

    def existing(self, pr, tag):
        page = 1
        while page <= 20:
            comments = self.call(
                "GET", f"/repos/{self.repo}/issues/{pr}/comments?per_page=100&page={page}")
            if not comments:
                return None
            for comment in comments:
                if tag in (comment.get("body") or ""):
                    return comment["id"]
            if len(comments) < 100:
                return None
            page += 1
        return None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--repo", required=True)
    parser.add_argument("--pr", required=True, type=int)
    parser.add_argument("--summary", required=True)
    parser.add_argument("--exit-code", required=True)
    parser.add_argument("--run-url", default="")
    parser.add_argument("--sha", default="")
    parser.add_argument("--project", default="")
    parser.add_argument("--api-url", default=os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    args = parser.parse_args(argv)

    token = os.environ.get("GITHUB_TOKEN", "")
    if not token:
        print("GITHUB_TOKEN is not set", file=sys.stderr)
        return 2
    if "/" not in args.repo or args.pr <= 0:
        print("--repo must be owner/name and --pr a positive number", file=sys.stderr)
        return 2
    try:
        with open(args.summary, encoding="utf-8") as handle:
            summary = handle.read()
    except OSError:
        summary = ""

    body = compose(summary, args.exit_code, args.run_url, args.sha, args.project)
    github = GitHub(args.api_url, token, args.repo)
    try:
        comment = github.existing(args.pr, marker(args.project))
        if comment is None:
            github.call("POST", f"/repos/{args.repo}/issues/{args.pr}/comments", {"body": body})
            print(f"posted the Eplyx comment on #{args.pr}")
        else:
            github.call("PATCH", f"/repos/{args.repo}/issues/comments/{comment}", {"body": body})
            print(f"updated the Eplyx comment on #{args.pr}")
        return 0
    except urllib.error.HTTPError as error:
        if error.code in (401, 403, 404):
            # Fork pull requests get a read-only token. The gate result stands.
            print(f"::warning::Eplyx result not commented on #{args.pr}: "
                  f"GitHub answered {error.code} (read-only token?)")
            return 0
        print(f"GitHub refused the comment: HTTP {error.code}", file=sys.stderr)
        return 1
    except (urllib.error.URLError, TimeoutError, ValueError) as error:
        print(f"could not reach GitHub: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
