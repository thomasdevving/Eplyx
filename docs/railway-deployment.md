# Deploying the Eplyx service on Railway

This is an operator configuration guide for the existing service. It does not
establish that a remote service has been deployed or is ready. For the fixed
review build, use [its pinned instructions](review-environment.md); for team
onboarding, use [the pilot guide](pilot-onboarding.md).

The upgrade analysis worker runs offline from retained inputs. Historical corpus
preparation is separate. Optional current-state and governance acquisition use
read-only providers in the parent service, never inside the analytical workers.

## Build and persistent storage

```sh
cargo build --locked --release -p eplyx-server
```

`eplyx-server` serves by default; `eplyx-server admin --help` lists local operator
commands. Dashboard/cloud assets are embedded at build time: rebuild the service
after changing those assets.

| Setting | Default / purpose |
| --- | --- |
| `EPLYX_DATA_DIR` | `/data`; mount persistent storage for the authoritative registry, reports, inputs and content-addressed artifacts |
| `EPLYX_BIND` | Overrides the bind address; otherwise the injected `PORT` is honoured, falling back to `0.0.0.0:8080` |
| `EPLYX_OPERATOR_TOKEN` | Explicit operator credential for administrative HTTP actions; no default |
| `EPLYX_MAX_CANDIDATE_BYTES` | 8 MiB |
| `EPLYX_MAX_EXPECTATION_BYTES` | 256 KiB |
| `EPLYX_MAX_BUNDLE_BYTES` | 192 MiB |
| `EPLYX_MAX_CONCURRENT_RUNS` | 2 simultaneous durable queued executions |
| `EPLYX_ALLOWED_ORIGINS` | Empty by default; explicitly allow a separate frontend origin for cross-origin browser API access |

Same-origin workspace pages do not need a CORS allowlist. A separate public
frontend must use the intended `EPLYX_API_URL` and an allowed origin. CI requests
are unaffected by browser CORS rules. Do not put an operator token in CI.

The volume holds project/token/bundle/run indices, report documents and immutable
program/capture/input artifacts. Candidate bytes are retained by content identity
for queued execution and recovery; they are not merely temporary uploads.
Optional Postgres stores identity and authorization mappings. Back up the volume
and database together. See [cloud configuration](cloud.md#operator-configuration)
for identity, public URL, observation provider and registered mechanism settings.

## Provision a project and its evidence

Run these in the configured service environment. Replace each placeholder with
the identifier printed by the previous command:

```sh
eplyx-server admin create-project --name "SPL Stake Pool" \
  --program-id SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy
eplyx-server admin register-bundle --project PROJECT_ID --path ./eplyx-bundle
eplyx-server admin activate-bundle --project PROJECT_ID --bundle BUNDLE_ID
eplyx-server admin create-token --project PROJECT_ID --label ci
```

Project IDs are generated, not supplied to `create-project`. `register-bundle`
verifies and retains the bytes and prints a registered `bndl_...` ID. Activation
uses that ID, not the bundle SHA-256. Registration and activation are separate
because activation changes the evidence used by future checks.

The token is shown once; retain it in the team's secret store. Project tokens
can submit/read their own supported checks and sync. They cannot activate bundles.
The authenticated operator console/API also supports bundle upload and activation;
it is distinct from workspace member access. [Pilot onboarding](pilot-onboarding.md)
explains workspace binding, project capabilities and the async submission client.

## Health, jobs and validation

Use `/ready` for readiness and `/health` for liveness. Readiness checks configured
storage and identity prerequisites; it is not a successful analysis or proof that
every observation provider/path is ready. Inspect project capabilities separately.

Accepted checks return an asynchronous run. The durable queue retains inputs,
attempts and completed outputs and reconciles interrupted work on restart. Follow
the returned run link or poll its state before reading the verdict. Submission,
worker failure and candidate regression are different outcomes.

Run documented local tests with scratch storage before operating a deployment.
The [review guide](review-environment.md) describes the separate explicit remote
smoke procedure and required identities. Remote checks, service restarts, provider
acquisition and infrastructure changes are deliberate operator actions; this
configuration guide does not perform them.

A GitHub App, continuous monitoring and universal protocol coverage remain outside
the current implementation. Durable workers, dashboards and optional user
accounts are implemented.
