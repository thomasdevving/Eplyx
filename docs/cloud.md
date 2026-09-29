# Cloud and hosted analysis

Eplyx uses one API service and one authoritative filesystem registry/content-addressed
store. Optional Postgres holds mutable identity and authorization only: users,
sessions, device codes, workspaces, membership, credentials and mappings to MAIN
`proj_...` IDs. It does not store a second analytical project/run/evidence registry.

## Sign in and sync

```sh
eplyx login --server "$EPLYX_URL"
eplyx link --project PROJECT_ID
eplyx sync
eplyx logout
```

Choose an explicit service origin; there is no default production destination.
Login shows a device code for approval in the browser. The CLI does not ask for
the password. `link --workspace WORKSPACE_ID --create NAME` creates a linked
project; creation retries preserve identity. CI can supply `EPLYX_URL`,
`EPLYX_PROJECT_ID` and a project-scoped `EPLYX_TOKEN` through its secret store.
`eplyx sync RUN_ID` selects one run, and `--latest` selects the latest. No cloud
credential is needed for offline analysis or replay.

Sync uploads exact metadata, report, bindings, ChangeSpec/state descriptors, search,
counterexamples and reproduction records with SHA-256 identities. It never uploads
source code, programs, captures, project configuration, environment or provider
credentials. Leaky text is refused, never rewritten. The same content is idempotent;
changed content conflicts. Sync does not execute evidence or change a deployment
gate. Logout attempts server revocation and removes the local credential. If the service is unreachable it reports that revocation could not be confirmed; an environment token remains set until the caller unsets it.

Workspace owners manage members and project credentials in Settings. Project tokens
can submit hosted checks and sync only their project; they cannot activate a
baseline or manage membership. The operator explicitly assigns legacy projects
to a workspace with `POST /v1/projects/{p}/workspace-binding`, naming both the
workspace and its owner. No existing project becomes public through migration.

## Operator configuration

These are configuration references, not deployment instructions executed by this
migration. Adding Postgres, provider credentials or infrastructure remains an owner
operation. Existing API-only deployments can omit identity and observation settings.

| Setting | Purpose |
| --- | --- |
| `EPLYX_DATA_DIR` | Persistent MAIN volume, including CAS and durable run queue |
| `EPLYX_OPERATOR_TOKEN` | Explicit bootstrap/admin credential |
| `EPLYX_DATABASE_URL` | Optional private Postgres identity connection |
| `EPLYX_PUBLIC_URL` | Canonical service origin for browser/device flows |
| `EPLYX_SIGNUP_CODE` | Optional invitation gate for account creation |
| `EPLYX_OBSERVATION_RPC_URL` | Dedicated bounded read-only current-state provider |
| `EPLYX_MIGRATION_CANDIDATE` | Optional path to the exact registered reference SBF |
| `EPLYX_GOVERNANCE_RPC_URL` | Separate existing governance observation provider |

The Postgres connector uses a private transport (`NoTls`); place it on a trusted
private network or behind an appropriately protected tunnel. Browser traffic needs
HTTPS outside loopback. Secrets belong in operator configuration, not a run, report
or request payload. Workers receive no environment variables. `/health` checks
liveness and `/ready` verifies configured storage/identity readiness.

## Project analysis capabilities

`GET /v1/projects/{project_id}/capabilities` is the authenticated,
project-scoped pre-submission contract. It uses the same project token,
operator credential, or workspace access check as hosted submission and project
retrieval. Callers without access receive the normal project authorization
error and no capability details.

The response has `schema_version`, `project_id`, and a deterministic `analyses`
list. Each entry contains the existing hosted job `kind`, an explicit `status`
(`ready`, `not_ready`, or `unsupported`), `supported`, `can_submit`, and a
`missing` list. Every missing prerequisite has stable `code`, user-facing `message`,
and high-level `action` fields. The current hosted kinds are
`program_upgrade`, `token_migration`, `lifecycle_change`,
`current_observation`, `current_path`, `current_candidate`,
`current_preflight`, and `current_stress`.

Readiness is derived on every request. All kinds require an enabled project.
Program upgrades also require an upgrade target and a currently usable active
bundle. Prepared token-migration and lifecycle inputs require neither a bundle
nor an observation provider. Current-state kinds require the configured
read-only observation service; `current_candidate` additionally requires the
server-registered migration mechanism. Observation IDs, selected accounts,
proposal terms, uploaded files, amounts, clocks, and other form inputs remain
submission validation concerns and are not reported here.

Reason codes currently returned are `project_disabled`,
`upgrade_target_missing`, `active_bundle_missing`,
`active_bundle_unavailable`, `observation_service_unavailable`, and
`migration_candidate_not_configured`. Messages never include configuration
values, credentials, provider details, database addresses, or filesystem
paths.

This endpoint is distinct from `/ready`, which reports infrastructure/service
health, and from analysis reports, which contain evidence and verdicts. A
`ready` capability does not mean a proposal is safe, approved, or verified and
does not authorize a later POST. Every submission endpoint revalidates current
state and its own inputs independently.

Hosted project and analysis pages fetch this contract with their existing
authenticated request path. Overview and Technical modes render the same
per-kind response: Overview shows the server message and next action, while
Technical additionally exposes the stable reason code and exact status fields.
Controls remain disabled while capability discovery is loading or has failed.
The display is informational and can become stale immediately; the subsequent
submission remains authoritative.

Authenticated project users can choose **Token migration** from the Analyse
page. The guided flow lives at `/p/{project_id}/analyse/migration` and consumes
the existing `token_migration` capability entry; it does not derive another
readiness model. A disabled project produces a conflict at submission even if
the form was opened while ready, while malformed or mutually inconsistent
prepared inputs remain an input rejection. Both leave the entered proposal in
the browser.

Authenticated project users can also choose **Lifecycle change** from Analyse.
The guided flow lives at `/p/{project_id}/analyse/lifecycle` and consumes the
same server-authored `lifecycle_change` capability entry used by the API. It is
a narrow adapter for one prepared scenario shape: the user declares an
`Active` to `TransitionRequired` policy boundary, may add a successor and
deadline, and uploads an existing immutable lifecycle snapshot. The browser
does not observe chain state, verify an issuer, infer eligibility, or evaluate
consequences.

The current-state candidate registration checks the T3 SHA-256 and requires an
observation service. That current-state workflow cannot choose code; the prepared
migration flow separately uploads the exact mechanism named by its ChangeSpec.
Catalogue import uses
`node scripts/import-catalogue.mjs --server-binary SERVER_BINARY --data-dir DATA_DIR
--capture SAVED_CAPTURE`; omitting `--capture` explicitly fetches the one fixed
public catalogue source. It never executes captured scripts. Import failure leaves
the saved current version usable. No such external fetch was run for this migration.

## Hosted inputs and recovery

`POST /v1/projects/{p}/checks` retains its program-upgrade multipart contract.
Migration inputs add `change_spec`, `state_input`, `state_artifact`, `candidate`
and optional `analysis_options` with the gate policy. A captured-program synthetic
recipe also requires `pinned_program_capture` with the exact T3 digest; it is stored
in CAS and staged into the worker. Captured worlds and bundled recipes need no
checkout dependency. Lifecycle jobs use `change_spec`, `snapshot`, `scenario`,
optional hash-bound `lifecycle_evidence`, and explicit `before`/`at` options.
Current-state browser requests use the typed observation/check endpoints described
in [the current-state guide](current-state-analysis.md).

The guided lifecycle form posts that existing lifecycle multipart contract to
`POST /v1/projects/{project_id}/checks`; it introduces no new analytical model,
job type, or run page. Its generated scenario uses one explicit
`ScenarioAssumption` source and the fixed `browser-prepared/v1` version. The
submitted comparison times are exactly one second before the declared effective
time and the effective time itself. The ChangeSpec, scenario and analysis
options shown in Technical mode are the exact JSON documents submitted, while
the uploaded snapshot is retained byte-for-byte and identified by a browser
SHA-256. The engine still parses and normalizes those documents, binds the
ChangeSpec back to the scenario, validates the snapshot and asset identity, and
derives all lifecycle findings.

The lifecycle form distinguishes project availability from input validity and
job creation. A stale project readiness response is reported separately from a
scenario/snapshot rejection, and both preserve the entered draft and selected
snapshot. After acceptance, the browser verifies the returned lifecycle asset
and optional successor identities before navigating to the existing run route.

The guided browser migration form posts that same multipart contract to
`POST /v1/projects/{project_id}/checks`; it introduces no new job or analytical
record. It builds one narrow ChangeSpec from the visible fields, attaches the
user-selected SBF and already-prepared state files, and supplies the explicit
`block-only` analysis option. The preview and submitted `change_spec` are the
same JSON document. Exact raw amounts and ratio parts stay strings. Candidate
SHA-256 and byte length are previewed and checked against the identity returned
with the accepted run, after which the browser navigates to the existing run
route. Backend parsing, normalization, state/candidate validation, job staging
and analysis remain authoritative.

Inputs are validated and stored before queuing. Durable runs retain attempts;
interrupted work retries within MAIN's limit, while a saved complete projection
can finalize metadata without execution. Sync does not enter that queue. Upload,
acquisition, queue, execution and output bounds reject oversized work. Missing
programs, inconsistent state and failed workers yield honest errors.

Back up the volume and identity database together. Losing the database loses
membership/credential mappings; losing the volume loses authoritative evidence.
Do not rebuild either from display summaries. Tests require
`EPLYX_CLOUD_TEST_DATABASE_URL`; missing Postgres fails loudly. Browser tests require
a loopback scratch database and support `EPLYX_CHROME`. No live provider, deployment,
new infrastructure or release packaging was performed. See [T9](phase-t9-cloud-hosted-analysis.md).


## Squads governance review trail

For a governance-bound program upgrade, read
`GET /v1/projects/{project_id}/governance/changes/{change_spec_id}/trail`.
It uses existing operator, workspace/project member and project-token access.
The bound ChangeSpec is the root, and `runs` contains summaries only of analyses
of that exact change. `events` interleaves retained `governance_check` and
`deployment_attestation` observations while preserving sealed evidence IDs.

`limit` defaults to 20 (maximum 100). Follow `next_cursor` as `cursor` to retrieve
all events; follow `runs_next_cursor` as `run_cursor` for further linked analyses.
Hosted observations are oldest first by durable recording ID. Legacy G2 evidence
has null recording time and follows in deterministic hash order without a claim
about relative chronology. Each G2 row identifies its exact matched G1 binding.
Corrupt identities fail the page with an integrity error.

Each new attest result receives an immutable `gocc_…` occurrence containing the
project, bound ChangeSpec, binding ID, attestation ID and host recording time.
Repeated identical proof content still produces separate occurrences. Existing
sealed JSON and the original governance GET / attest POST responses remain
compatible. Reads perform no RPC or evidence writes and require no observation
provider. See the [review contract](governance-review-trail-audit.md#10-implemented-review-layer)
for schemas, ordering, legacy behavior and the unchanged real-mainnet witness limit.

## Derived hosted migration order analyses

Completed hosted token-migration runs can launch bounded order analysis from the
run page. Run-specific eligibility verifies the retained parent package, world,
Clock/runtime, candidate/dependencies and eligible units. The existing
`token_migration` project capability remains separate from this eligibility.

`POST /v1/projects/{project}/runs/{parent}/migration-order` accepts only
`source_a` and `source_b`, persists a queued occurrence before worker execution,
and returns HTTP 202. Read full structured results at
`GET /v1/projects/{project}/migration-orders/{run}` and list children at the
parent's `migration-order` route. `/artifact` downloads the exact portable
`order-case` directory as tar (the host must provide `/usr/bin/tar`). Existing
project authorization protects create, list, read and download, including
cross-project token rejection. No public demo evidence route is added.

The worker reuses the local Rust `order_store::save` engine, with an empty
environment and no provider/RPC. Same ChangeSpec, two operation instances, same
retained world and Clock: A alone, B alone, A → B, B → A. New state requires a new
parent migration run. Engine IDs remain content identities; hosted `run_...` IDs
are occurrences. Repeated identical requests never overwrite history.

Manifest, report, state/CAS and comparison bytes persist in MAIN's existing
verified CAS; the input/projection references bind each artifact to its immutable
parent. Reads validate the portable evidence without executing or repairing it.
Effects, no effects and valid `NotEstablished` results complete normally;
evidence/handoff failure kinds are retained separately from internal
`execution_error`. Children stay on the parent run page, outside generic proposal
history. See [migration-order-case.md](migration-order-case.md) for the full
contract, reproduction and limitations.
