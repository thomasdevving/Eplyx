# Upgrade × parameter interaction

The bounded analyzer accepts one existing `program_upgrade` ChangeSpec and one
existing `protocol_parameter_change` ChangeSpec for
`spl_stake_pool_sol_deposit_fee_v1`. It answers whether the measured fee-change
effect differs between historical V1 and explicitly qualified V2 for one retained
DepositSol. It does not execute a loader Upgrade.

For the user workflow, see the [fee-analysis walkthrough](parameter-analysis-guide.md).
The CLI and hosted flow below share the same bounded analyzer.

Build the host CLI with `cargo build -p eplyx-engine --bin eplyx --offline --locked`.
The already qualified fixture ELF must be present for analysis; its reproducible
build instructions remain in
[the fixture README](../programs/fixture-stake-pool-config-candidate/README.md).

```sh
target/debug/eplyx interaction analyse \
  --upgrade docs/examples/stake-pool-config-upgrade-change.json \
  --parameter docs/examples/stake-pool-parameter-change.json \
  --bundle deploy/bundle \
  --record-id mainnet-spl-stake-pool-151010f709e113e7 \
  --candidate artifacts/fixture_stake_pool_config_v2.so \
  --out /tmp/eplyx-interaction-new \
  --format json

target/debug/eplyx interaction verify \
  --artifact /tmp/eplyx-interaction-new --format json

target/debug/eplyx interaction reproduce \
  --artifact /tmp/eplyx-interaction-new --format json
```

`--out` must name a fresh directory whose parent exists. All three commands run
through the existing empty-environment local worker. Verification never starts a
VM. Reproduction verifies first, then actually repeats every admitted execution
in fresh banks and compares the complete canonical report. Copying the executable
and artifact directory is sufficient: no original checkout, bundle, recipe,
provider, credentials or qualification JSON participates in reproduction.
Commands return success when an internally valid artifact is produced or checked;
a completed analysis can report `not_established`. Inspect status and per-stage
availability rather than interpreting process success as economic establishment.

The reusable Rust API is `interaction::Input::new`, `analyse`, `verify`,
`reproduce`, `save` and `load`. The analyzer shares the original parameter
implementation's typed configuration preservation, pool-only state substitution,
historical execution and unique-account reconciliation checks.

## Admission and isolation

The compiled, versioned interpretation profile admits only exact historical
same-code V2=V1, or the constructed Step 10B fixture
`a664f74b73dedc713f16934829b25f9a0c0c3a06c6ce21f03fdc7869ae5b555d`
(133992 bytes). That fixture is test code, not an upstream release. It references
the retained qualification identity
`2a5247b4dcad7d8c49ef0f7e0a3763d2960e0ddaef9a78fb8763c59220552cc9`.
Caller-supplied qualification claims cannot extend admission. Unknown candidates
produce a factual unqualified result with no invented executions or contrast.

Historical V1 retains its original ELF/layout/manager guard. The scope requires
complete official StakePool layout, zero referral, ungated nonzero ten-account
DepositSol, legacy Token, the reviewed Token deployment and loaders, only the
reviewed target/Token/System/ComputeBudget dependency set, and fixed epoch zero.
Every account, instruction, compiled message, Clock, loader, dependency and
simulation manager/payer assumption is bound. `replaces`, when supplied, must
exactly match V1. Activation, delivery, authority and ProgramData expectations
are explicitly rejected because this experiment cannot evaluate a loader world.
Stale pool/current-fee expectations and mismatched targets or candidate bytes
also prevent admission.

Execution order is R00, K1, K2, available R01, R10, available R11. R00 must have
historical fidelity **Matched** and successful reconciliation. K1 and K2 each
start from S0 and independently execute SetFee(C1) under their own code. R01
receives only K1's verified pool; R11 receives only K2's verified pool. The parent
binding contains the stage name, stage input, execution and full pool commitment;
equal pool bytes cannot substitute K1 for K2. Restoring the original pool makes
all action commitments equal except the explicit effective-target code overlay.
The original historical record and dependency manifest are never rewritten.

Manager signer privilege and its account envelope are simulation assumptions.
The configuration payer is separate, and its fee/account changes never enter
an action cell. No externally signable authority or key possession is asserted.

## Metrics, failures and identities

Each metric has a value or a precise unavailable reason in each cell. Actual
instruction rejection, preservation failure, historical mismatch, unreconciled
output, infrastructure unavailability and not-executed stages remain distinct.
Independent completed cells and computable effects survive partial failures.
Failed, absent or unreconciled cells never become zero. Token-role aliases are
counted once; an independent manager/referral split stays undefined when aliased.

The checked signed integer contrasts use canonical decimal strings:

- Parameter effects: R01−R00 and R11−R10.
- Code effects: R10−R00 and R11−R01.
- Combined: R11−R00.
- Interaction: (R11−R10)−(R01−R00).
- Cross-check: combined = code C0 + parameter V1 + interaction.

The two ChangeSpec IDs remain independent and retain their original identity
contract. Display metadata is retained in the report but excluded from analysis
input identity. The latter commits both proposals, original record/bundle and
account bytes, V1/V2/dependency bytes and loaders, versioned runtime/profile,
configuration and action messages, Clock, assumptions and metric/cell semantics.
It excludes measured results, output paths and display metadata. Report identity
additionally binds all outcomes and evidence.

## Portable artifact

`manifest.json` is written last. `objects/<sha256>` contains bounded CAS objects
for the input, both specs, full report, candidate and historical program bytes,
and readable report. `report.md` presents the question, both proposals,
configuration assumptions/handoffs, matrix, effects, availability and limitations.
The machine report contains full pre/post account bytes, instructions/messages,
execution hashes, logs and CPI traces. The artifact is complete without external
references. Existing local bounds are retained: 64 KiB proposal, 2 MiB executable,
128 MiB artifact object; this profile uses a small fixed dependency set. Hosted
and lifecycle limits are unchanged.

Loading verifies every reference and byte hash, strict canonical documents,
proposal/candidate/record/runtime/profile bindings, stage inputs, K1/K2 handoffs,
configuration preservation or rollback, action reconciliation, all quantities,
contrasts and seals. It neither repairs evidence nor executes a VM. This proves
internal consistency, not independent authentication of VM execution or historical
provider claims. Reproduction supplies the additional fresh local execution check.

## Compatibility and limits

The shared helper seam preserves the old parameter evaluator. A reviewed
compatibility adapter accepts only its specific prior operation-source commitment
`63aa307015b17f79d6b07e71f024f624eb2388ad15e6ba8264a5b3fa2a9a9a87`,
with every other runtime pin and receipt field checked. Old receipts remain
unchanged; verification normalizes only in memory, and reproduction restores that
retained source identity before comparing seals. Arbitrary source/runtime
mismatches remain errors. New interaction receipts bind the current source and
versioned analyzer revision. Archived Step 10A/10B qualification/build records and
both fixture ELF identities are preserved; reruns with the extracted source
naturally have new runtime-bound receipt identities.

There is no slot-accurate validator reconstruction, time advancement, installed
loader world, real Upgrade, rollout-order or atomicity proof, signing, governance,
live acquisition, broad candidate qualification, valuation, population-wide
impact or deployment policy. The hosted flow below submits this same bounded experiment;
it does not expand candidate qualification or its analytical scope. A measured zero interaction covers
only available reconciled metrics in this bounded experiment.

## Hosted project flow (Step 10D)

From a private project's retained **program-upgrade run**, use **Compare code and
fee**. Select a historical record explicitly and paste/import an existing Stake
Pool parameter ChangeSpec (for example
[the existing declaration](examples/stake-pool-parameter-change.json)). Preview
shows exact proposal documents/IDs, the current/proposed rational fee, pool and
retained slot, V1/V2 provenance, qualified profile, six-execution scope and fixed
Clock. It predicts no result. Edits invalidate the preview; errors preserve input.
Project readiness and parent eligibility are checked separately.

All routes use existing project authorization (project token, authorized workspace
session or operator). There is no public-demo interaction evidence route.

| Method | Route | Contract |
| --- | --- | --- |
| GET | `/v1/projects/{project}/runs/{parent}/interactions/eligibility` | Safe eligible-record facts/unavailable reasons; read-only admission, no VM/RPC/run creation |
| POST | `/v1/projects/{project}/runs/{parent}/interactions/preview` | `{ "record_id": "...", "parameter_change_spec": { ... } }`; authoritative input facts, no execution |
| POST | `/v1/projects/{project}/runs/{parent}/interactions` | Preview fields plus required `request_key`; durable HTTP 202 |
| GET | `/v1/projects/{project}/runs/{parent}/interactions` | Derived occurrences linked to this parent |
| GET | `/v1/projects/{project}/interactions/{run}` | Lifecycle, verified six-stage presentation or factual failure |
| GET | `/v1/projects/{project}/interactions/{run}/artifact` | Verified original portable artifact as `interaction.tar` |

Unknown request fields are rejected. Only the existing
`spl_stake_pool_sol_deposit_fee_v1` parameter operation is admitted. Candidate,
upgrade proposal, accounts, dependencies, Clock, qualifications, K outputs and
claimed results are never caller inputs on this route.

A parent must be project-owned, ordinary (not another analytical job), completed
with a retained report, exact ChangeSpec and candidate, and a verified pinned
historical bundle/baseline/corpus. A completed **negative upgrade verdict is
eligible evidence**; analytical PASS is not required. Parent activation, delivery,
authority and ProgramData expectations are preserved verbatim and rejected by
existing engine admission if unsupported. Unknown candidate bytes remain
unqualified; only same-code historical V1 and the existing constructed Step 10B
profile are admitted. No qualification upload or trust flag exists.

Before acceptance, MAIN retains the self-contained engine Input and exact engine
input-contract CAS reference (including runtime/dependency commitments), both proposal
CAS references and IDs, project/parent references, exact parent input snapshot and
report references, selected record/bundle commitments, executable/dependency/
runtime commitments and `analysis_input_sha256`. The parent snapshot includes its
expected-changes commitment where present. The existing request-key service returns
the same job for an identical retry, conflicts on different accepted input, and
allows a new key to create another occurrence with the same engine content ID.
There is no global deduplication or independent parameter run prerequisite.

The existing durable queue, semaphore, compare-and-set claim, isolated
credential-free worker and restart recovery are reused. The worker reloads and
verifies accepted references, calls `interaction::analyse`, saves with
`interaction::save`, and ingests the original manifest, CAS objects and report
into MAIN's existing CAS. It never resolves a newer active bundle or contacts a
provider. Minimal parent-child indexing scans existing project occurrence IDs and
immutable interaction bindings; ordinary one-proposal histories remain unchanged.
Both proposal IDs are explicit, and neither is selected as the generic run's single
`change_spec_id`.

`interaction_observed`, `no_measured_interaction`, and a valid partial
`not_established` engine report are **completed analyses**. Rejected SetFee/action
cells retain their actual states; missing cells/quantities remain unavailable with
reasons. Evidence integrity failures are typed factual failures; internal worker
failures establish no analytical conclusion. Read/download verifies both hosted
bindings and the engine artifact with no VM, observation, regeneration or repair.
Missing/corrupt evidence leaves the occurrence and references retained and hides
analytical conclusions.

The review shows K1 and K2 separately, then R00/R01/R10/R11 and engine-provided
ledgers/effects. Signed integer strings are preserved without JavaScript
arithmetic. For the qualified retained case, recipient credit falls by 7609851 raw
under both versions and interaction is zero: the fee effect is equal in this case,
with no additional interaction measured for this quantity. This does not mean no
parameter effect, rollout compatibility or approval. Overview and Technical use
the same result; Technical exposes both proposals, analysis/report/program/
dependency/runtime IDs, K1→R01/K2→R11 handoffs, stage commitments and manager/payer
assumptions.

Download requires project access. Extract in a fresh directory:

```sh
tar -xf interaction.tar
eplyx interaction verify --artifact interaction --format json
eplyx interaction reproduce --artifact interaction --format json
```

The archive contains controlled `manifest.json`, `report.md` and `objects/<sha256>`
paths under `interaction/`, at most 34 files and 128 MiB total member bytes, without
symlinks or caller paths. It uses the existing migration-order tar packaging
helper. Hosted occurrence provenance stays outside engine canonical bytes. The
extracted artifact plus a matching CLI executable suffice without the hosted
project, original bundle, provider, credentials or source checkout.

The only engine API additions are pure `Input::retained_control` and `prepare`
wrappers around existing decoding/admission/commitment logic. Candidate profiles,
VM runtime settings, six-execution behavior, fixtures and numerical semantics are
unchanged. Compiled analyzer source commitments continue to identify the exact
source build; hosted and local analysis with that build have identical content IDs.

Qualification boundaries: Step 10B/10C VM-backed numerical evidence remains local
analyzer qualification. `hosted_interaction` tests exercise the actual hosted
router, durable/recovered isolated worker, original artifact, same-code/distinct
profile, partial rejection and standalone downloaded CLI reproduction.
`cloud_interaction` tests real workspace sessions using isolated local Postgres.
Frontend unit/browser tests mock hosted HTTP at the established report boundary;
they qualify presentation and interaction flow, not new VM execution or candidate
profiles. No actual deployment or live hosted project mutation is part of Step 10D.
