# Deploying the Eplyx CI API on Railway

The serving path is entirely offline. **No RPC or archive credentials are
required**, and a candidate check that tries to reach one is a bug.

## Build

```bash
cargo build --release -p eplyx-server
```

The binary serves by default; `eplyx-server admin ...` is the operator surface.

## Environment

| Variable | Default | |
|---|---|---|
| `EPLYX_DATA_DIR` | `/data` | must be a **persistent volume** — Railway's deployment filesystem is ephemeral and everything would be lost on redeploy |
| `EPLYX_BIND` | `0.0.0.0:8080` | |
| `EPLYX_MAX_CANDIDATE_BYTES` | 8 MiB | |
| `EPLYX_MAX_EXPECTATION_BYTES` | 256 KiB | |
| `EPLYX_MAX_CONCURRENT_RUNS` | 2 | replay is CPU-bound and synchronous; this bounds how many run at once |
| `EPLYX_ALLOWED_ORIGINS` | *(empty)* | comma-separated browser origins allowed to call the API, e.g. `https://eplyx.dev` |

**`EPLYX_ALLOWED_ORIGINS` is required for the browser flow and for nothing else.**
A request carrying an `Authorization` header triggers a CORS preflight, and with
no origin named the API answers it without `Access-Control-Allow-Origin`, so the
browser blocks the call. CI runners are unaffected — `curl` does not enforce the
same-origin policy — so leaving it empty is the right default and naming a
wildcard never is: these requests are authenticated.

Deliberately absent: `SOLANA_RPC_URL`, `SOLANA_ARCHIVE_RPC_URL`,
`SOLANA_BLOCK_RPC_URL`, `SOLANA_RPC_ORIGIN`. Corpus construction is a separate
workflow that runs elsewhere with its own credentials, and never on the path of
a pull request.

## Volume layout

```text
/data
  projects/<project-id>/project.json
  bundles/<bundle-sha256>/            immutable, content addressed
  runs/<run-id>/{metadata.json,report.json,report.md}
```

Uploaded candidate binaries are **not** stored here. They live in a temporary
directory removed on both success and failure; only the SHA-256 survives.

## Health checks

Point Railway's healthcheck at `/ready`, not `/health`. `/health` means the
process is alive; `/ready` additionally proves the persistent volume is
writable, which is the failure that actually matters after a redeploy.

## Provisioning a project

```bash
eplyx-server admin create-project --id stake-pool --name "SPL Stake Pool" \
  --program-id SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy
```

The CI token is printed **once**. Only a salted SHA-256 verifier is stored, and
there is no endpoint that can hand the token back.

## Installing and activating a bundle

```bash
eplyx-server admin install-bundle   --path ./eplyx-bundle
eplyx-server admin activate-bundle  --project stake-pool --bundle <sha256>
```

Two steps on purpose. Installing verifies and stores; activating changes what
every pull request is measured against, so a human chooses when that happens.
Activation refuses a bundle that fails verification, belongs to another program,
or was built under a different adapter version.

There is no HTTP route to either. A project's CI token can run checks and read
its own reports — it cannot replace the bundle it is measured against.

## Verifying a deployment

```bash
scripts/hosted-demo.sh ./eplyx-bundle
```

Runs the five gate outcomes, proves the hosted report is byte-identical to the
local one, and checks authorization isolation, upload limits, retention and that
no token reaches disk or logs — with every RPC variable unset.

## Not built yet

No queue, no worker pool, no object storage, no dashboard, no user accounts, no
GitHub App. Checks run synchronously, which the current corpus sizes comfortably
allow. The orchestration is shaped so a queue can be introduced later without
touching the engine.
