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

The candidate registration checks the T3 SHA-256 and requires an observation
service. The browser cannot choose code. Catalogue import uses
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
