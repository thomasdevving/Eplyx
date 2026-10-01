# Fixed Step 11A review build

This runbook covers one retained Stake Pool upgrade/fee interaction. The real
review frontend is embedded in `eplyx-server`: `/login`, `/workspaces`, `/p/...`
and `/assets/...` share the API origin. The separate `frontend/Dockerfile` static
operator console is not the ordinary workspace login surface. Route `/p/...`
refreshes must reach the server rather than the static console.

The source is frozen on `codex/step11a-fixed-review`. Resolve its exact tested revision
from the private local `validation-receipt.json` or remote `remote-validation-receipt.json`;
`build-identities.json` also records the source commit, target, lockfile and
server/worker/CLI hashes. The frontend is compiled into that same server binary.
Operational receipts remain outside Git and never change analytical identities.

## Clean local build and validation

Create a fresh worktree of the chosen review commit, never the dirty development
directory. Existing unrelated `media/` is excluded from both source and images.

```sh
git worktree add --detach /tmp/eplyx-review-source codex/step11a-fixed-review
cd /tmp/eplyx-review-source
# Put the existing Rust, PostgreSQL, Node and pinned pnpm executables on PATH.
export LC_ALL=C
pnpm install --offline --frozen-lockfile
EPLYX_BUILD_COMMIT="$(git rev-parse HEAD)" cargo build --locked --offline \
  -p eplyx-server -p eplyx-engine --bin eplyx-server --bin eplyx
```

Use the qualified Darwin arm64 SBF builder from the
[fixture instructions](../programs/fixture-stake-pool-config-candidate/README.md).
Alternatively import the existing `.so` and build receipt into ignored
`artifacts/` after checking the ELF's digest/length and every receipt source,
lockfile, notice/license and script commitment against this clean snapshot.
Do not rebuild it on Linux or admit alternative bytes. Host server/CLI executables
must be built for their actual host from this same snapshot.

```sh
# Optional executable override for an already installed Chromium browser.
export EPLYX_CHROME='/absolute/path/to/chromium'
node scripts/review-local.mjs /tmp/eplyx-review-source
```

This creates a new private scratch PostgreSQL cluster (loopback 55491), registry
and service (loopback 4491). Ports may be selected with
`EPLYX_REVIEW_PG_PORT` and `EPLYX_REVIEW_LOCAL_PORT`. It starts the actual server,
uses supported signup/workspace/member/bundle activation/upload APIs, and submits
a real retained upgrade parent. It creates an owner, a designated test reviewer
with **member** access, and another owner-only workspace/project. Owner/operator
bootstrap credentials never enter the reviewer browser. No email is sent.

The clean browser signs in through the password form, opens the project/parent,
checks eligibility, explicitly selects the record, previews the existing proposal,
submits once, refreshes the accepted occurrence, reads actual six-stage engine
results and downloads the authenticated archive. It checks both presentation
modes and signed-out/cross-project read/list/create/download denials. After one
controlled service restart it signs in anew and reads/downloads the same result
using the same database/registry; no reseeding occurs. The project/baseline,
parent/child/report and retained artifact identities must survive. Tar packaging
may have different timestamps on repeated downloads; each actual archive checksum
is recorded separately, while verification checks its canonical report/CAS bytes. The script stops only
its own server and PostgreSQL processes, preserving private scratch evidence.
There is no broad cleanup step. Delete only that explicitly returned scratch path
when its private evidence is no longer needed.

The downloaded tar is inspected before extraction: only bounded regular files
and directories under `interaction/`, no links or traversal. A fresh standalone
directory contains the copied CLI and downloaded artifact. Separate verify and
reproduce commands run with an empty environment under macOS `sandbox-exec`
`deny network*`; a loopback curl negative control proves denial. The child inherits
that restriction. Reproduce re-executes the matrix and must match the downloaded
report/input identities. PostgreSQL/Python/browser tools are needed for setup and
extraction only, not analytical reproduction. A Linux service needs Linux server/worker
binaries. Offline reproduction uses a compatible same-snapshot native CLI on its
actual host and checks the bound runtime without waiving mismatches; record that
host and its enforced network-denial mechanism separately.

Completed-result persistence and queued/running recovery are separate claims.
This script does not manufacture an in-flight restart or claim to test recovery
unless a later receipt explicitly records it. A host filesystem directory survives
process restart; this is not proof of a remote mounted volume or container restart.

## Required retained input identities

| Input | Identity |
| --- | --- |
| Record | `mainnet-spl-stake-pool-151010f709e113e7`, slot `447850493` |
| Bundle | `5e5b67ac13e4f6b8249348ad81db29885ee8ee897ba78f6de213b55793f4285f` |
| Historical V1 | `ec2dfefaa70d560754a0000f39bd2cabc192b895d36205b3c428f601b6e1d7e1` |
| Constructed V2 | `a664f74b73dedc713f16934829b25f9a0c0c3a06c6ce21f03fdc7869ae5b555d`, 133992 bytes |
| Parameter | `spl_stake_pool_sol_deposit_fee_v1`, observed `0/1000`, proposed `1/100` |

Use `docs/examples/stake-pool-config-upgrade-change.json` and
`docs/examples/stake-pool-parameter-change.json`; identities are derived by the
existing parser/preview, never manually reconstructed. The parent may have a
negative upgrade verdict. The child must report `no_measured_interaction` with
recipient credits `760985008 / 753375157 / 760985008 / 753375157`; both parameter
effects are `-7609851`, code effects and interaction `0`, combined `-7609851`.
Unavailable referral splits include reasons. This is constructed code under
retained historical state, fixed epoch-zero runtime and assumed manager signing,
not an upstream release, current state, rollout simulation or approval.

## Remote deployment and repeatable smoke

No remote environment was present at task start. The owner subsequently authorized
creating one; that authorization does not establish accessible infrastructure,
free capacity, storage isolation or compatibility. Do not deploy until account,
project, environment, service, private PostgreSQL and persistent analytical volume
are positively identified and isolated from production. The owner now authorizes up to EUR 20 total additional usage, including tax/FX,
under the existing plan for at most 14 days. No plan upgrade is authorized. The saved Railway association names production and
must not be reused as a review destination.

Inspect the platform's current repository deployment triggers before pushing the
dedicated review branch. Local workflows alone do not prove a push cannot deploy
production. No main push, force push, tag or release. Deploy the explicit tested
commit, not a moving branch auto-deployment. For Linux, build the server/worker and
reproduction CLI natively from that snapshot; first prove the qualified route
reproduces on that runtime without changing admission or trust rules.

The existing API Dockerfile includes the worker executable (`current_exe`) and
`/usr/bin/tar`. Build it from the clean source, passing
`--build-arg EPLYX_BUILD_COMMIT=<full-review-commit>`, and record the resulting
image digest. Configure `EPLYX_PUBLIC_URL` to the actual HTTPS origin,
`EPLYX_ALLOWED_ORIGINS` explicitly, `EPLYX_DATABASE_URL` on protected private
transport, `EPLYX_DATA_DIR` on a distinct persistent review volume,
`EPLYX_OPERATOR_TOKEN`/`EPLYX_SIGNUP_CODE` as private secrets, and bounded concurrency.
Use `/ready` for readiness. The proxy must preserve API methods, same-origin
session cookies, direct project routes and bounded upload/archive response bodies.
Test the actual mounted stores through a controlled review restart.

Bootstrap through supported operations exactly as the local script demonstrates:
create a labelled owner-controlled workspace/project, register and deliberately
activate the verified bundle, add the already registered designated test account
as member, and submit candidate+ChangeSpec to create a retained parent through the
worker. Create the denial project in a different workspace without reviewer
membership. Do not give the reviewer an operator token or inject a session.

Save a private target JSON with an exact HTTPS origin and
`selection: "isolated-review"`, plus verified `account_id`, `project_id`,
`environment_id`, `service_id`, `identity_storage_id`, `analytical_storage_id`
and `owner_confirmed_isolation: true`. This explicit attestation is required by
the smoke guard; the command cannot infer storage isolation from a hostname.
Supply designated test credentials via safe runtime configuration (not shell
history, Git, command arguments or logs):

```sh
# These variables are required; no origin or credentials have defaults.
# EPLYX_REVIEW_TARGET: private attestation JSON described above
# EPLYX_REVIEW_EMAIL / EPLYX_REVIEW_PASSWORD: designated test account
# EPLYX_REVIEW_PROJECT / EPLYX_REVIEW_PARENT / EPLYX_REVIEW_DENIED_PROJECT
# EPLYX_REVIEW_PARAMETER: path to the existing parameter ChangeSpec
# EPLYX_REVIEW_OUTPUT: private artifact/receipt directory outside source
pnpm test:review
# After the operator restarts only the review service, supply the retained child:
# EPLYX_REVIEW_RUN_ID=<accepted-child> and a fresh EPLYX_REVIEW_OUTPUT
pnpm test:review
```

Remote smoke never bootstraps, restarts or deletes resources. Keep these operator
steps separate. Reproduce the exact remote-downloaded artifact with the compatible
same-snapshot CLI in a fresh offline directory; local success is not remote proof.
Do not save raw HAR/traces/session material in Git. Receipts contain safe IDs and
hashes, never passwords, DSNs, bearer tokens or magic links.

To intentionally update: make only a scoped blocker fix, commit on the dedicated
branch, create a new clean worktree, rebuild every affected component, rerun this
route and record the final revision. Do not automatically advance the review
build on development pushes. Roll back only the review service to its previously
recorded image/source; retain its stores and evidence. Production remains untouched.

Readiness is `ready_for_external_review` only after the actual external HTTPS
login/journey, real worker, persistence, isolation and downloaded-artifact
reproduction pass. Local full-stack success is
`locally_verified_remote_not_validated`; a failing prerequisite is `blocked`.
Neither is a repository-wide verification or security-audit claim.

## Scoped route repair

The real review browser exposed an asset collision: `/assets/migration.js`
served the prepared cloud form instead of the dashboard module that exports
`isMigration`, so every project page failed during module loading. The prepared
form now uses `/assets/prepared-migration.js`; the dashboard module keeps its
existing route. A focused server asset-routing regression check covers both.
No authentication, runtime, candidate admission or analytical conclusion changed.

The actual macOS service download also contained AppleDouble `._` metadata
sidecars from system tar. The packaging subprocess now sets the public
`COPYFILE_DISABLE=1` switch after clearing its environment, without changing
analytical workers or input bytes. A focused archive regression test writes
host metadata and checks that only canonical artifact members enter the tar.
Safe extraction continues to reject unexpected roots, links and traversal;
previous downloads are preserved, never rewritten to make validation pass.


## Bounded Railway review continuation

The original fixed input is `cef17fe3d3c485d925960c4e5aef3a8106206da0`.
A deployment-only revision adds `deploy/review/Dockerfile`, its scoped ignore
file and the absolute-deadline startup. Engine,
frontend, identity checks and qualified fixture sources are unchanged. The image
contains the native Linux server (also its isolated worker) and matching CLI;
the CLI's two existing embedded migration templates are retained in its build
context. A focused source-derived check prevents another omitted CLI include. No SBF
fixture is rebuilt. There is no local Docker runtime; remote build identity and
runtime verification are therefore mandatory, not inferred from the Darwin binary.

The Railway CLI 5.63.1 reuses/refreshed the existing saved sign-in successfully.
Account workspace is `4620b527-2d7b-4731-ac7f-2138933cd64e`
(`thomasdevving's Projects`), on the existing Pro subscription. Unrelated projects
are `eplyx-cloud`, `friendly-bravery`, `loyal-comfort` and `marvelous-sparkle`.
Read-only provider trigger inspection found this repository's existing deploy
triggers on **main**; the dedicated review branch is not a production trigger.
Deploy explicit archived source through CLI upload, with no GitHub auto-deploy.
The current API rejects new legacy Config-as-Code paths; set the Dockerfile,
readiness, replica limits and NEVER restart policy directly on this review service.
The upload context has no root railway.toml; no workspace-wide IaC migration is made.

Billing is USD. The current cycle ends `2026-10-23T23:04:15Z`; neither the four-day
compute period nor fourteen-day storage cleanup window crosses that reset.
The existing workspace compute hard limit is USD 20 and affects unrelated
services; it is not changed or claimed as a review cap. Included Pro usage is
shared, so this estimate counts no included credits. Billing country is NL;
21% VAT is a conservative assumption, not a confirmed invoice rate. The ECB
2026-09-30 reference is USD 1.1355/EUR; allow another 5% for payment/FX variation.
See [pricing](https://docs.railway.com/pricing/plans),
[cost-control scope](https://docs.railway.com/pricing/cost-control), and
[used-volume billing](https://docs.railway.com/volumes/reference).

Planned stack: one app, 2 GB/1 vCPU cap, concurrency one; one private PostgreSQL,
0.5 GB/0.25 vCPU cap; one analytical and one identity volume. An existing offline
reproduction measured 107,282,432 bytes peak RSS. Idle memory is not measured;
the idle estimate assumes a conservative 0.5 GB total. Four days at *full caps*
cost USD 6.66 compute. Include eight GB used storage through day fourteen,
ten GB egress, and USD 2 failed-build/cleanup reserve: about EUR 10.88 all-in.
Even both standard 50 GB volumes fully occupied for fourteen days raise this
risk scenario to about EUR 18.08. Resource caps do not guarantee an invoice
ceiling; egress, billing delay, tax/FX and owner cleanup remain explicit risks.
No external paid build/registry, paid domain, agent or third compute service is used.

Both review services receive one absolute `EPLYX_REVIEW_EXPIRES_UNIX`, chosen no
later than four days after their first billable resource. Standard GNU timeout
terminates the service process group at that UTC deadline; startup rejects an
expired deadline, and Railway restart policy is NEVER. Restart cannot renew it.
This is a scoped runtime shutdown, independent of billing reset; it is not
unattended spend monitoring. Confirm the same startup/deadline and enforced caps
in the actual provider configuration, and test a shortened deadline remotely
before marking the review ready. The local focused check covers missing/expired
values, actual timed shutdown and restart denial.

Storage survives compute shutdown and remains billable. Export the actual remote
artifact, receipts, runbook and both needed stores to the owner's private local
review directory; verify checksums before deleting anything. The owner must remove
only the two recorded review volumes by first billable creation +14 days. At full
100 GB storage, residual storage is USD 0.50/day (about EUR 0.56/day with this
margin); eight GB is USD 0.04/day. Automatic storage cleanup is not claimed.
Exact created IDs, timestamps, configured expiry, validation and cleanup commands
belong in the private remote receipt. Keep the original local receipt unchanged.


## Verified remote review — 2026-10-01

Status: **ready_for_external_review**. Review origin:
<https://review-app-review-80b8.up.railway.app>.
Deployed source is `codex/step11a-fixed-review` at
`b3244996a68ecac81037cdfe997155f76a5b8de4`; later documentation-only commits do
not advance this running image. Explicit CLI upload has no repository auto-deploy.
Application deployment `9238a18f-4bbc-485b-99db-611a62d9401a`, image
`sha256:7b0f8b2e2c570108ffdb9a1c3072a33aeb4189aeb7913c637d4ee49d853b0733`,
built with Rust/Cargo 1.98.1 for `x86_64-unknown-linux-gnu`.
The server and isolated worker share executable SHA-256
`18c0fe84273375a49e45566d20bcdafe36d5689832df2e261526e2dba9c35e3a`;
image CLI SHA-256 is
`987ffd7a60ba04ba59422b7bbb25a1ba19ab5c373c09a61f043f087eb6205a67`.
Cargo.lock SHA-256 is
`4417749a09e246a4fb8c111611fdf5825d295a28a414deeb30cd353eccec992a`.
The first of two bounded app builds failed because an existing CLI migration
JSON include was excluded from the Docker context. The second, after retaining
both template JSONs, passed. The source-derived include check and two real
expiry checks passed (three focused tests); runtime expiry was also tested remotely.
No analytical or fixture bytes changed.

| Review resource | ID | Created UTC |
| --- | --- | --- |
| Project `eplyx-step11a-review-20261001` | `b2660f15-4103-406f-9266-59385cc3fdd1` | `2026-10-01T08:43:29.644Z` |
| Environment `review` | `7b6501b2-2cd8-4188-8556-c903e3f60583` | created with project |
| App | `5e6ba9dc-927b-4ec7-b913-912c517fd6de` | `2026-10-01T08:43:32.296Z` |
| Private PostgreSQL | `274a9491-20a0-4e2f-bd52-f7c9a0d6967a` | `2026-10-01T08:47:07.579Z` |
| Analytical volume `/data` | `8ad3bc74-f2a9-4cf1-8234-d30da4b3de09` | `2026-10-01T08:49:45.354Z` |
| Identity volume `/var/lib/postgresql/data` | `38333286-474d-435b-bc2a-d4cc475cba82` | `2026-10-01T08:49:48.187Z` |

Both mounts were verified as persistent ext4 devices, owned by root, writable and
with about 48 GB available each. The running app resolves
`review-postgres.railway.internal`; provider queries confirm no database public
domain or TCP proxy. Database deployment is
`71e2a84c-c39d-4fad-b9b1-05615a30bb85` (`postgres:17-bookworm`).

The genuine reviewer used a clean password-form session with **member** role.
Parent `run_01M3VBJSJWD98E1FWM13Z4N19C` was produced by the real worker, with
an expected negative upgrade verdict and retained report. Child
`run_01M3VBM18VQSY20H38FNP0DKAK` completed with the four expected credits,
parameter effects `-7609851`, code effects `0`, combined `-7609851`, interaction
`0` and `no_measured_interaction`. All K1/K2/R00/R01/R10/R11 states and handoffs,
original proposal IDs, historical inputs and constructed provenance passed the
existing unmodified browser smoke. Refresh occurred while the child was running.
One controlled app restart preserved the project, active baseline, parent, same
child/report and download. Fresh member login and signed-out/cross-project
read/list/create/download denial passed again. Queued/running restart recovery
was **not exercised**.

Post-restart downloaded tar SHA-256:
`c7a292ad2dd28e404ffe13942ea8758235c68cce5fe0a7d3832899886b24b24a`.
A fresh standalone directory with only the artifact and same-snapshot Darwin
arm64 CLI verified all evidence and actually re-executed the VM matrix, under
empty environment and macOS `sandbox-exec` network denial (HTTPS negative control
also denied). CLI SHA-256:
`1198699bef9bc2fcf34d3142c98e6b1f0f8911bfc9ce954bbd84eee3de876724`.
The compatible bound runtime passed without an override. Remote and reproduced
report SHA-256:
`ed73f7b1895be8dfb74913eb21eea52bfc8932802328dbc855a818bb67673ce3`;
analysis input SHA-256:
`39faccc088197efa811af419693152f4dc4a998b88b6e423f15721b995ea9a1b`.
This validates this remote artifact's portability; it is not a cross-platform
release qualification.

Private owner evidence is retained at
`/Users/thomasnguyen/Downloads/Personal/Eplyx/Eplyx/data/step11a-railway-review/`:
`remote-validation-receipt.json`, original unchanged local receipt, both smoke
receipts/downloads, source-context manifest, runtime/cost evidence, standalone CLI,
verified `exports/identity.dump` and `exports/analytical.tar`, and this runbook.
The database export is a consistent custom-format pg_dump, inspected with
pg_restore --list (including user/member/project table data); a full restore test
was not exercised. Both exports match remote checksums; the analytical archive
contains the actual parent/child evidence and has only safe regular members.
A temporary validation SSH key was revoked and its agent/key files removed.
Credentials stay only in private mode-600 files; no external invitations were sent.
The owner can use the saved owner account, register a designated reviewer with the
private signup code, and add that already-registered account through the existing
workspace-members operation. Provide credentials securely and never operator access.
No additional access/MFA action is required for the tested route.

### Exact deadline and owner cleanup

First billable resource: `2026-10-01T08:49:45.354Z`, or
**1 October 2026 10:49:45.354 Europe/Amsterdam (CEST)**.
Both running PID-1 commands are GNU timeout wrappers using absolute deadline
`1791190185`: **5 October 2026 08:49:45 UTC / 10:49:45 CEST**.
A restart subtracts the current clock from that same deadline; it cannot renew it.
Actual short-deadline testing terminated the command and its child process group,
then refused an expired restart. Railway restart policy is NEVER on both services;
no automatic sleep, renewal or new deployment trigger is configured. TERM begins
at the deadline and KILL follows after at most ten seconds if needed. This stops
compute automatically, not storage billing or arbitrary egress spending.

Authorized final expiry and manual storage removal deadline:
**15 October 2026 08:49:45.354 UTC / 10:49:45.354 CEST**.
Owner cleanup is mandatory by then. Keep the verified local exports/runbook;
export later needed review activity again before removing its only remote copy.
The private `cleanup-review.mjs` verifies the listed local export checksums and
requires an explicit cleanup argument before stopping/deleting only the recorded
review resources. Run it after review ends and before the deadline:

```sh
node /Users/thomasnguyen/Downloads/Personal/Eplyx/Eplyx/data/step11a-railway-review/cleanup-review.mjs --remove-verified-review
```

Equivalent provider actions are `railway down --project <review-project-id>
--environment <review-environment-id> --service <review-service-id> --yes` for
each of the two services, then `railway volume --project <review-project-id>
--environment <review-environment-id> delete --volume <exact-volume-id> --yes` for
the two volumes above. Never rely on a saved linked production target. 2FA, if
enabled later, must be completed by the owner. No storage deletion was performed.

Observed review usage at validation was USD `0.0016614836485498578` before tax/FX,
about EUR 0.002 with the stated margins. Provider billing is delayed: this is not
an invoice or a final accrued-cost ceiling. Plan reserves EUR 10.88 total and
leaves EUR 9.12 inside the EUR 20 allowance; the full-volume risk scenario leaves
EUR 1.92. No shared included credit is subtracted. Sampled app memory peaked at
134.8 MB and PostgreSQL at 78.2 MB during this window; samples are not an exact
instantaneous worker RSS measurement. Current idle app/database were roughly
117/42 MB. Provider used-volume metrics, including filesystem overhead, were
roughly 0.72/0.84 GB, larger than logical file totals. Retained storage after
compute stops costs at most USD 0.50/day for fully used 100 GB (about EUR 0.56/day
with margins); at eight GB it is USD 0.04/day. Manual cleanup, billing delay,
FX/tax variation and unbounded public egress prevent a guaranteed EUR invoice cap.
Production, plan/subscription, shared spending limits and unrelated work were
untouched. No external paid service was created.
