# Production Pilot: Real Bundle Hosted CI

A candidate binary travelled from a developer workflow to a hosted Eplyx
deployment, was evaluated against an immutable bundle of validated historical
production-derived interactions, and produced the same deterministic gate result
as local execution. This is the record of that, including what it broke.

Deployment commit: `340f66b` on `main`.
Acceptance date: 2026-09-18.

## Lead with the flaws

Three things were wrong in production. Two of them would have destroyed or
compromised the pilot, and neither was visible from the fact that the service
answered `200`.

**There was no volume.** The service had been deployed, provisioned with a
project, a bundle, a token and a passing run — and `/data` was the container's
own filesystem. `/`, `/data` and `/opt` shared device id `40894568`; `railway
volume list` reported none in the environment. Every redeploy would have
silently discarded the project a pilot was using. A deployment succeeding is not
evidence that its storage is durable, and the healthcheck cannot tell the
difference. Fixed by attaching a volume at `/data` and re-provisioning; the
device ids now differ and `lost+found` is present.

**The operator token was the string `openssl rand -hex 32`.** The command had
been pasted where its output belonged, so a publicly guessable twenty-character
value guarded the credential that creates projects, issues tokens and activates
bundles — the credential that decides what every future pull request is measured
against. Rotated to 256 bits of `openssl rand`; the old value now answers `401`
and the new one `200`.

**One service's build configuration was imposed on another.** `railway.toml`
named `dockerfilePath` and `startCommand`, and config-as-code applies to every
service built from a repository and always overrides the dashboard. Adding a
frontend service made it visible: it began compiling the Solana tree on its way
to launching `eplyx-server` to serve static files. Both fields were redundant —
each image carries the right `CMD`, and the API Dockerfile sits at the default
path — so removing them let `RAILWAY_DOCKERFILE_PATH` select per service.

Two earlier defects were found on the way in and are recorded for completeness:
the Docker build stubbed `engine`'s declared binary but not `server`'s declared
library, so the dependency-caching layer failed target resolution before
compiling anything; and `deploy/README.md` instructed an operator to run
`admin install-bundle`, which is not a command.

## What is deployed

| | |
|---|---|
| API | `https://upgrade-impactreport-check-production.up.railway.app` |
| Frontend | `https://eplyx-frontend-production.up.railway.app` |
| Volume | `/data`, its own device, holding `projects/`, `bundles/`, `runs/` |
| Replicas | 1 — the registry is a directory, so a second would fork the state |
| API variables | `EPLYX_OPERATOR_TOKEN`, `EPLYX_ALLOWED_ORIGINS`, `EPLYX_DATA_DIR`, `PORT` |
| Frontend variables | `EPLYX_API_URL`, `RAILWAY_DOCKERFILE_PATH`, `PORT` |
| Concurrency | 2, as the startup line reports |

No RPC or archive credential is present. `SOLANA_RPC_URL`,
`SOLANA_ARCHIVE_RPC_URL`, `SOLANA_BLOCK_RPC_URL`, `SOLANA_RPC_ORIGIN`,
`ALCHEMY_API_KEY`, `HELIUS_API_KEY`, `ANCHOR_WALLET` and `SOLANA_KEYPAIR` are all
unset, and hosted replay succeeds anyway.

## The bundle

| | |
|---|---|
| Program | `SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy` |
| Adapter | `spl-stake-pool@3`, cross-program invocation |
| Bundle | `5e5b67ac13e4f6b8249348ad81db29885ee8ee897ba78f6de213b55793f4285f` |
| Baseline | `ec2dfefaa70d560754a0000f39bd2cabc192b895d36205b3c428f601b6e1d7e1` |
| Corpus | `a4b667316b972685743bf0724b5596b7fbb64dde46468e159f44d773894f69b1` |
| Records | 10 validated historical observations |
| Window | slots 447850493 – 447904974 |

Registering copied it onto the volume and did not change it: all 18 files hash
identically to the copy baked at `/opt/eplyx/bundle`. Being in the image did not
activate it — the project read back `status: setup`, `active_bundle: null` after
registration, and became `ready` only after an explicit operator activation.

## The pilot project

`proj_01M2SSV2YR9MM1RX4PHP31Z42W`, "Solana Stake Pool Pilot", `spl-stake-pool@3`,
active bundle `bndl_01M2SSVAD4X3ZJRRT1DM0KJC60` = `5e5b67ac…`, read back from the
registry rather than inferred from a filename. One project token,
"Production Pilot CI", printed once; only a hash is stored.

A second project, "Isolation Test Project" (`none@0`, no bundle), exists to test
boundaries without touching the pilot.

## Acceptance matrices

Local, against `deploy/bundle`:

| case | candidate | exit |
|---|---|---:|
| A baseline as its own candidate | `ec2dfef…` | 0 |
| B known regression | `3193eabd…` | 1 |
| C B with a bounded declaration | `3193eabd…` | 1 |
| C′ C with the bound tightened to 0 bps | `3193eabd…` | 1 |
| D stale declaration | `ec2dfef…` | 3 |
| E unevaluable declaration | `ec2dfef…` | 5 |
| F corrupted bundle copy | `ec2dfef…` | 4 |

Hosted, through the real API, and byte-identical to local in every case:

| case | hosted exit | canonical report sha256 |
|---|---:|---|
| baseline | 0 | `7be26a66f3e98f9b96ba6f1270003c158d99968d9ed081c336a97f14782d2099` |
| regression | 1 | `e5e6a4b39ce377d8…` |
| bounded | 1 | `139fd95c2899d54b…` |
| stale | 3 | `8d6a4dc867c39655…` |
| unevaluable | 5 | `0c7a3b5231f84b99…` |

Since Phase C1 every report also names the change it evaluated (`change`), so
the baseline report is now `cb0e9d12290191ea7fd6a8c5ab02f226fe56ab35b20070a253f4e6e760014bef`.
With that object removed it is still exactly `7be26a66…`; the U1 guard asserts
both. See [phase-c1-changespec.md](phase-c1-changespec.md).

Case F was verified locally only. Producing it hosted would mean corrupting
bundle bytes in content-addressed shared storage, which the pilot project reads
from too; manufacturing that danger to demonstrate an exit code was not worth
it.

### What the regression actually found

`fixture_stake_pool_v2.so` is a locally constructed counterexample, not a
proposed release. Two findings, both real:

- `spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased`, 1 of 1
  observation, largest change −1 bps. This is the deliberate defect: the
  candidate caches the exchange rate at four decimal places instead of
  multiplying before dividing, so a depositor receives slightly fewer pool
  tokens. The reference build of the same candidate produces no such finding,
  which isolates it.
- `spl-stake-pool/withdraw_sol/execution/transaction/now_reverts`, 9 of 9. The
  candidate implements `DepositSol` and nothing else.

### A bounded declaration does not buy a pass here

Declaring both fingerprints with bounds moved both to `expected` — 2 expected, 0
unexpected, 0 exceeded — and the gate still failed, on `undeclarable_change`.
The decoded economic evidence the named findings do not speak for (manager-fee
amount, pool-mint supply, reserve-stake lamports, across 9–10 observations) is a
separate layer, and declaration cannot reach it. That is the design working:
a team cannot declare its way past evidence nothing semantic accounts for.

Tightening the deposit bound to 0 bps reclassified that finding as
`expected_but_exceeded`, which is how the bound was shown to be live rather than
decorative.

## Durability

The service was restarted three times during the pilot, once by attaching the
volume, once by `railway redeploy`, once by a variable change. After each:
the project survives, the active bundle pointer survives, the token still
authenticates, passed and failed runs survive, run-history order is unchanged,
and stored `report.json` bytes are identical to before the restart and still
identical to the local report.

## Concurrency

Four checks submitted in 0.49 s were all accepted `202 queued`. Each kept its own
candidate hash, each pinned bundle `5e5b67ac…`, no report crossed, and the
project token retrieved all four. Capacity is 2; runs complete in well under a
second, so a queued run was observed at acceptance but no sustained queue formed.

## Boundaries

A project token gets `404` for operator resources and `401` for project
creation. Across projects, every attempt — read the other project, list its
runs, read one of its runs, fetch its report, submit a check to it — answers
`401`, in both directions, while each token reads its own project `200`.

Malformed input: a non-ELF candidate is accepted, then terminates as `failed`
with exit 2, `report_available: false`, and a stated reason. It is a preflight
abort, not a gate verdict — the candidate is never reported as having failed a
comparison it was never eligible for. A missing candidate field, an unparseable
upload field, an invalid token and an unknown run id answer 400, 400, 401 and
404. The server stayed healthy throughout.

Logs contain three lines and no credential: a volume mount path, the listening
address with the concurrency, and "Starting Container". There is also no
per-request logging, so there is no audit trail.

## Frontend

Thirty checks against the deployed frontend, driven through Chrome DevTools
Protocol: the landing page loads with the deployed API baked in and no
localhost; the console refuses to list anything before a credential and leaks no
project data; the pilot project renders with the right program, adapter and
active bundle hash; the analyse form has a project selector and neither a
backend URL field nor a project id field; a candidate submits; the app navigates
straight to `/runs/{id}`; the run reaches `PASSED` with `exit code 0`; a hard
refresh keeps access; the report shows bundle, baseline, candidate and corpus
hashes and the record count; coverage limitations remain visible on a pass; no
"safe" or "secure" language appears; and the history lists the run with its
verdict, exit code and bundle, linking to it.

## GitHub CI

`.github/workflows/eplyx.yml` checks out, builds or accepts a candidate, hashes
it before submission, submits through `scripts/eplyx-submit.sh`, polls, fetches
both report formats, writes a step summary carrying run id, candidate, bundle,
baseline and corpus hashes, adapter, record count, exit code, review counts and
the corpus's known limitations, uploads them as an artifact, and exits with the
gate's own code. It uses `secrets.EPLYX_TOKEN`, a project token, and no operator
credential.

**No GitHub Actions run occurred.** The `gh` CLI is not available in this
environment, so repository secrets could not be set and no workflow could be
triggered. What was run is the exact client the workflow invokes, with
GitHub-shaped environment variables, for both required cases: the
baseline-compatible candidate exited 0 and the regressed candidate exited 1,
each with the server-reported candidate hash matching the hash taken before
upload.

## Timings

Client-side wall clock, including upload and polling: baseline accepted in
0.31 s and terminal at 0.52 s; regression accepted in 0.20 s and terminal at
0.40 s. Server-recorded queue and execution times are 0 s at second
granularity. Bundle 2.5 MB on disk, baseline candidate 1,080,464 bytes,
regression candidate 33,864 bytes, canonical report 2,609 bytes passing and
9,722 bytes failing.

## Mutation testing

Ten deliberate breakages, each caught by the guard that should catch it, all
restored:

| # | mutation | caught by |
|---|---|---|
| 1 | worker re-resolves the active bundle after run creation | `a_run_keeps_the_bundle_it_was_accepted_against` |
| 2 | project token may activate a bundle | `a_project_token_cannot_manage_credentials_or_baselines` |
| 3 | acceptance runs against a different bundle | `--expect-bundle`, exit 70 |
| 4 | canonical hosted report gains a timestamp | `the_hosted_report_is_byte_identical_to_the_local_one` |
| 5 | reported candidate hash differs from uploaded bytes | client identity check, exit 70 |
| 6 | HTTP 500 for an ordinary exit 1 | `a_completed_run_reads_the_same_on_every_later_fetch` |
| 7 | `execution_error` mapped to exit 1 | `an_execution_error_is_not_a_failed_gate` |
| 8 | workflow uses the operator token | `the_ci_workflow_carries_no_operator_credential` |
| 9 | frontend hides limitations on a pass | frontend acceptance |
| 10 | RPC environment required for hosted replay | `hosted_replay_consults_no_rpc_environment` |

Mutation 9 was not caught at first, twice, and both failures were in the guard
rather than the product. The limitations check matched the word "limitation",
which also appears in the passing headline "the coverage limitations below still
apply", so a page that rendered no limitations at all still passed. And the
static host serves app files with `max-age=300` while the browser profile
persisted between runs, so the control and the mutated build were the same
bytes. The assertion now matches the limitations' own sentences and the harness
disables the browser cache. A third assertion, "the run reached `passed` or
`failed`", was accepting either verdict for a candidate that can only pass; it
now requires `PASSED` and `exit code 0`.

## What may now be claimed

Eplyx can evaluate a Solana program candidate in hosted CI against an immutable
bundle of validated historical production-derived interactions, using the same
deterministic engine locally and in CI, with byte-identical canonical reports.

## What may not

- That all mainnet behaviour is covered. This corpus is 10 observations from one
  window of roughly 57,000 slots.
- That a passing candidate is safe. A pass means no disallowed difference was
  observed in the replay coverage this bundle represents.
- That the corpus is statistically representative. The observed semantic action
  population is **not measured**. "1 deposit / 9 withdraw" describes the selected
  replay corpus, not production frequency, and the bundle says so rather than
  implying a distribution it never counted.
- That CPI is covered. 41 CPI interactions were observed in the window and none
  were replayable under the exact historical contract. The corpus says nothing
  about them, which is not the same as their being unaffected.

## Still open

- No real GitHub Actions run.
- Exit 4 verified locally only.
- Run-history rows show no run id; they identify a run by candidate, bundle,
  verdict and time. Adequate, but a link to copy would help.
- No per-request logging, so no audit trail of who submitted what.
- `railway.toml` config-as-code is deprecated with a hard cutoff of 2026-12-01.
