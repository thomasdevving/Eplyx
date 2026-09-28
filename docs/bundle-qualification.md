# Schema-1 bundle qualification

`eplyx bundle prepare` is the canonical operator workflow for qualifying a
bounded schema-1 corpus. It discovers production structure, attempts exact
historical acquisition, retains refusals, checks a single evidence cohort,
selects once, builds with authoritative V1 fidelity, reopens the bundle, and
writes a versioned receipt. It does not register or activate anything.

```sh
eplyx bundle prepare --spec qualification.json --out ./qualification-run \
  --cache ./qualification-cache --format text
# Repeat with frozen requests and a fresh output directory:
eplyx bundle prepare --spec qualification.json --out ./qualification-offline \
  --cache ./qualification-cache --offline --format json
```

`--spec` is deliberately separate from the global `--config` project setting.
The output root must not exist. Nothing refreshes or deletes an earlier run.
Parent directories must exist. Relative observed-artifact and prepared-corpus
paths resolve beside the specification. Cache/output paths resolve from the
working directory and never enter canonical receipt facts.

## Explicit input

```json
{
  "schema_version": 1,
  "scope": {
    "program": "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
    "actions": ["deposit"],
    "subjects": [{
      "protocol": "spl-stake-pool",
      "action": "deposit_sol",
      "domain": "economic",
      "subject": "pool_tokens_received"
    }],
    "start_slot": 447850000,
    "end_slot": 447906921,
    "discovery_limit": 500,
    "acquisition_budget": 40,
    "target_size": 10,
    "replay_schema": 1,
    "runtime_profile": "schema1_litesvm_mainnet"
  },
  "providers": {
    "transaction_url_env": "SOLANA_RPC_URL",
    "account_url_env": "SOLANA_ARCHIVE_RPC_URL",
    "block_url_env": "SOLANA_BLOCK_RPC_URL"
  },
  "accept_coverage": false
}
```

Optional keys are `observed` (path to the semantic measurement below),
`prepared_corpus` (diagnostic only; prepared schema 2 is explicitly refused),
and `providers.origin_env` (an environment variable containing an Origin).
No URL or credential is accepted as a specification field. Endpoint values and
Origin remain runtime-only. Error receipts contain typed codes, never raw
provider diagnostics or config paths. Environment values are needed even
`--offline` to locate the existing endpoint-hashed RPC cache. No new cache
format is used; a cache miss in offline mode is a refusal.

All three providers must establish mainnet genesis. Transaction history is
probed by the bounded scan itself. Exact S−1/S account and historical executable
support, and complete block history, are demonstrated by acquisition attempts
at concrete slots. Failed attempts describe this run, not a provider's universal
capabilities. A scan with no normalized interactions does not establish archive
capability. `minContextSlot` or current accounts cannot substitute for history.

Local preflight uses reviewed metadata for the current compiled adapter version,
then every transaction still passes the adapter's existing admission contract.
SPL Stake Pool `deposit` / `deposit_sol` and `withdraw` / `withdraw_sol`, and
Kamino's `deposit` / `deposit_reserve_liquidity_and_obligation_collateral`, have
schema-1 semantic qualification metadata. Subjects are the existing execution
`transaction` and promoted economic subjects. Every action needs at least one
explicit requested subject. Actual acquired records must expose those subjects.

Token-2022 has a schema-1 acquisition path but no promoted evaluable CI subjects;
it returns `missing_evaluable_subject`. Kamino borrow has a precise action ID
but the existing coarse selector vocabulary labels it `unknown`; this command
refuses that action rather than inventing a borrow stratum. Orca and Drift
require universal boundary proofs. They return
`outside_schema1_qualification_contract` before provider calls. Adapter version
changes require qualification metadata review; metadata cannot expand adapter
admission.

## Evidence, selection, and outputs

```text
<output>/
  receipt.json
  discovery/manifest.json, corpus.json, report.txt
  candidate-queue.json
  acquisition-ledger.json
  acquired/records/, manifest.json, corpus.json
  binaries/<sha256>.so
  dependencies/<program>.so
  selection.json
  bundle/bundle.json, corpus/, binaries/current.so, binaries/dependencies/,
         adapters/metadata.json
  cache/                         # only when --cache was omitted
```

Files appear only as their stages establish evidence. Receipts are checkpointed
atomically during the run, including before each acquisition attempt. Completed
runs are never overwritten. Process termination can leave a checkpoint showing
an in-progress attempt; it cannot turn that attempt into acquisition or fidelity.
Malformed JSON and an unwritable/existing output root may prevent a receipt.

Discovery uses the existing bounded normalizer and structural selector. It does
not fetch current account samples: eligible shapes therefore retain
`missing_state` in discovery until acquisition. This label is a missing evidence
claim, not a reason to refuse an exact acquisition attempt. Every normalized
and unreadable signature remains in the ledger. The queue follows the existing
structural selector's order; unsupported shapes, observations outside that
shortlist, and budget exclusions remain explicit. Structural counts are never
passed to the semantic selector as production denominators.

The cohort identity retains genesis, target V1 hash/deployment/loader, the
complete dependency set with deployment/loader/hash identities (including
builtins), and the schema-1 runtime profile. Observation slots and discovery
routes are excluded from compatibility identity. All acquired records must
belong to one cohort before scope filtering. Even different dependency sets
with no conflicting binary fail closed: choose a narrower window or use the
existing manually reviewed corpus/build route. No largest-cohort policy exists.
Baseline bytes come from the record hash and are verified at publication; no
filename search or newest/current executable download occurs.

The existing corpus selector runs exactly once on records in the declared scope.
Its action/entity/boundary/shape/time ranking and shortfall rules remain intact.
Legacy pool/amount features contain Stake Pool assumptions; this limitation is
persisted. The proposed selection is not V1 validation. Its legacy `replay_eligible` counts
mean acquired records, including unselected records whose fidelity has not run;
this distinction is also a persisted limitation. The builder executes
selected V1 records, and its observer writes record-level fidelity outcomes.
A mismatch stops the build without dropping the record or retrying selection.
The bundle builder and the orchestrator both reopen the resulting artifact;
selected IDs, adapter/version, limitations, and authoritative bundle identities
are checked. Bundle verification is not a coverage acceptance decision.

## Semantic production population

Absent `observed`, the receipt contains exactly:

```json
{"observed_semantic_population": {"status": "unknown"}}
```

An optional externally measured JSON artifact must have this contract:

```json
{
  "schema_version": 1,
  "program": "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy",
  "adapter": "spl-stake-pool",
  "adapter_version": 3,
  "classifier": "reviewed-deposit-classifier-v1",
  "actions": ["deposit"],
  "start_slot": 447850000,
  "end_slot": 447906921,
  "counts": {"deposit": 123},
  "completeness": "complete_declared_window",
  "provenance_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "unknown_interactions": 0,
  "unsupported_interactions": 0,
  "failed_interactions": 0
}
```

Counts here are illustrative. Supply real measurements and the hash of their
retained provenance, not these example values. Program, adapter/version, exact
window, and vocabulary must match the scope. Counts must be non-negative JSON
integers, with exactly one count per declared action. Strings, fractions, raw
structural maps, extra fields, and mismatched windows are rejected. Classifier
is a bounded identifier, not a URL. Completeness may instead be
`partial_declared_window`; that claim and the unclassified/unsupported/failed
populations remain visible. The command validates the binding and format, not
the independent classifier's correctness or claimed completeness. It does not
measure semantic production itself. Measured zero is distinct from unknown.
A measured action without acquired records remains a limitation, and a requested
subject absent from selection prevents `ready_for_activation`.

## Receipt and states

Receipt schema 1 is separate from the bundle manifest. Canonical JSON facts
include tool version/commit, validated scope and budgets, adapter/version,
provider capability results, discovery identity and structural counts, the
signature/refusal ledger, candidate cohorts, acquired action counts and
per-record subjects, observed artifact or unknown, selection and limitations,
record-level V1 outcomes (`not_run` for records not reached after a failure), the verified bundle manifest, explicit coverage
acceptance, state history, blockers, next action, and exit code. No timestamp,
absolute output path, provider URL, or credential affects these facts. Invalid
scope strings are not reflected into a receipt.

The evidence states are `scope_unqualified`, `support_blocked`,
`acquisition_blocked`, `evidence_acquired`, `bundle_candidate`,
`bundle_verified`, `coverage_review_required`, and `ready_for_activation`.
State history preserves what was established before a failure. `bundle_verified`
is also an explicit boolean; a verified artifact usually finishes in
`coverage_review_required` because scope and limitations need operator review.
No percentage is inferred.

After reviewing the receipt, rerun with frozen inputs, a fresh output directory,
and `--accept-coverage` (or `accept_coverage: true`). This records the operator's
explicit acceptance of the persisted bounded limitations, including an unknown
production denominator if appropriate. It reaches `ready_for_activation` only
when the bundle is verified and every declared subject is represented. Missing
support or missing selected subjects cannot be waived. Acceptance does not
change the bundle hash or any hosted/project state.

| Exit | Meaning |
| --- | --- |
| 0 | Bundle verified; coverage review required (including visible gaps) |
| 20 | Bundle verified and explicit bounded coverage accepted |
| 21 | Unsupported scope or outside the schema-1 command contract |
| 22 | Acquisition, cohort, fidelity, or bundle evidence blocked |
| 23 | Invalid input/configuration or semantic observed artifact |
| 24 | Internal/tool/output failure |

Automation should handle **both 0 and 20** as verified-artifact outcomes and
inspect the receipt state. These codes are unrelated to candidate-regression CI
exit codes. Common stable blockers include `archive_slot_mismatch`,
`same_slot_conflict`, `creation_closure_unsupported`,
`target_executable_unavailable`, `dependency_resolution_failed`,
`unsupported_lookup_tables`, `adapter_rejected`,
`incompatible_evidence_cohorts`, and `baseline_fidelity_or_build_failed`.
Per-signature rejections remain visible even when other records qualify.

Prepared schema-2 evidence stays on `eplyx bundle build --corpus ... --baseline
... --out ...` followed by `eplyx bundle verify --bundle ...`; no schema-2
collection, selection, or down-conversion is introduced here. Register a reviewed
bundle and deliberately activate it through the existing administrative flow in
[pilot-onboarding.md](pilot-onboarding.md). The qualification command has no
server/client mutation dependency.

`scripts/acquire-corpus.sh` remains a legacy historical reference. It is not the
canonical qualification path and does not issue the new receipt.
