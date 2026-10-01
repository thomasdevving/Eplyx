# Fixed Step 11A review build

This runbook covers one retained Stake Pool upgrade/fee interaction. The real
review frontend is embedded in `eplyx-server`: `/login`, `/workspaces`, `/p/...`
and `/assets/...` share the API origin. The separate `frontend/Dockerfile` static
operator console is not the ordinary workspace login surface. Route `/p/...`
refreshes must reach the server rather than the static console.

The source is frozen on `codex/step11a-fixed-review`. Resolve its exact tested revision
from the private `validation-receipt.json` produced by the invocation below;
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
extraction only, not analytical reproduction. Linux deployment reproduction needs
an equivalent explicit network-denial mechanism and a compatible native CLI;
macOS binaries cannot be Linux workers.

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
file, `deploy/review/railway.toml`, and the absolute-deadline startup. Engine,
frontend, identity checks and qualified fixture sources are unchanged. The image
contains the native Linux server (also its isolated worker) and matching CLI;
the CLI's existing embedded template is retained in its build context. No SBF
fixture is rebuilt. There is no local Docker runtime; remote build identity and
runtime verification are therefore mandatory, not inferred from the Darwin binary.

The Railway CLI 5.63.1 reuses/refreshed the existing saved sign-in successfully.
Account workspace is `4620b527-2d7b-4731-ac7f-2138933cd64e`
(`thomasdevving's Projects`), on the existing Pro subscription. Unrelated projects
are `eplyx-cloud`, `friendly-bravery`, `loyal-comfort` and `marvelous-sparkle`.
Read-only provider trigger inspection found this repository's existing deploy
triggers on **main**; the dedicated review branch is not a production trigger.
Deploy explicit archived source through CLI upload, with no GitHub auto-deploy.

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
