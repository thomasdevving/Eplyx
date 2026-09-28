# Program upgrade CI pilot

This path checks one team's candidate SBF program against one **already prepared,
active** Eplyx replay bundle. Bundle preparation and qualification remain a
hands-on operator task. An active bundle is evidence for a bounded set of
historical interactions, not a timeless safety statement about a protocol.

## One-time Eplyx setup

1. The operator and team create a project with the intended program upgrade
   target. The operator prepares, verifies, uploads, and activates its bundle.
   Link the project to the team's workspace and grant developers membership so
   the hosted run page is accessible. Record the project ID, target program ID,
   and active bundle SHA-256.
2. Issue a **project token** for this repository in project Settings (or
   `POST /v1/projects/{id}/tokens` with an operator credential). Store the
   once-shown token in GitHub Actions as the `EPLYX_TOKEN` repository secret.
   It can submit and read this project's runs and use other project-scoped check
   APIs, including governance verification if configured. It cannot access other
   projects, activate bundles, or manage credentials. Never put the operator
   token in CI.
3. Set the non-secret repository variables `EPLYX_API_URL` (the hosted service
   origin) and `EPLYX_PROJECT_ID`. The service origin must also serve the
   hosted project UI for the default run link. If the UI has a separate origin,
   pass that origin with `--web-url`.

The project credential can inspect readiness with
`GET /v1/projects/{id}/capabilities`. Its `program_upgrade` entry must be
`ready`. Missing target, active bundle, unavailable bundle, and disabled project
return concrete setup actions. Submission validates the state again.

## Repository setup

1. Copy [the async client](../scripts/eplyx-submit.sh) to
   `scripts/eplyx-submit.sh` and keep it executable. The runner needs Bash,
   curl, Python 3, and `shasum` or `sha256sum`.
2. Know your own candidate build command and output `.so` path. Replace the
   marked build command and path in [the external repository workflow](../examples/github/eplyx-upgrade-impact.yml),
   then copy it to `.github/workflows/eplyx.yml`. The Eplyx repository's
   [dogfood workflow](../.github/workflows/eplyx.yml) builds its own Stake Pool
   fixture and is not a protocol team's build recipe.
3. Start with no declarations or an optional tracked
   `.eplyx/expected-changes.toml` containing `version = 1`. Intentional
   differences should be declared with narrow bounds and reviewed with code.
   The existing Eplyx gate policy is enforced: no pilot threshold is added.

The canonical command, run after your own build, is:

```sh
EPLYX_TOKEN="$PROJECT_TOKEN" scripts/eplyx-submit.sh \
  --api "$EPLYX_API_URL" --project "$EPLYX_PROJECT_ID" \
  --candidate target/deploy/your_program.so \
  --report-json eplyx-report.json --summary eplyx-summary.md
```

Add `--expectations .eplyx/expected-changes.toml` only when the file exists.
Add `--expect-bundle SHA256` if the team wants CI to stop when the operator
rotates the active bundle. The client hashes the candidate before upload and
checks the accepted run, completed run, and report identities. It polls for at
most 30 minutes by default; `EPLYX_POLL_SECONDS` and
`EPLYX_TIMEOUT_SECONDS` configure positive intervals in seconds.

## First pull request

1. Submit a known-good candidate and inspect the job summary and hosted run.
   Confirm the target, candidate SHA-256, bundle SHA-256, and report limitations.
2. Where practical, submit a deliberately regressed test candidate and confirm
   the engine returns a blocking finding. Re-running the same PR creates a new
   durable run; prior runs remain in project history.
3. Open the run link as a project member. The URL is
   `/p/{project_id}/runs/{run_id}` on the public UI origin; normal sign-in is
   still required. CI exposes no project token in the link.

## Read the result

| Exit | Meaning | Next action |
| ---: | --- | --- |
| 0 | Pass within this bundle's evaluated coverage | Read limitations before approving. |
| 1 | Undeclared, over-bound, or undeclarable change | Inspect findings; fix code or narrow declaration. |
| 3 | Stale declaration | Update the declaration and review its intent. |
| 2 | Configuration or fidelity failure; with a report, this can mean missing semantic coverage | Inspect the run to tell input error from evidence gap. |
| 4, 5 | Bundle compatibility or unevaluable evidence | Repair setup, declarations, or evidence. |
| 70 | Input, auth, transport, server, or identity verification failure | Fix the integration; no analytical regression is claimed. |
| 75 | Hosted execution error with no verdict | Open the run and contact the operator. |
| 76 | Capability preflight says project is not ready | Follow the setup action in the job summary; nothing was uploaded. |

The step summary contains the run link, candidate and evidence identities,
engine exit code, finding count and review counts when a report exists. The
stored Eplyx report is authoritative. A network timeout is never converted
into a finding. Subsequent candidate PRs use the same project and active bundle
until the operator deliberately activates another bundle.
