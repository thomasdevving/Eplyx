# Step 12A: bounded Token-2022 amount search

Development base: `4c521f5` (completed parameter and interaction analysis, frozen
review source). Development branch: `codex/parameter-amount-search`. The review
checkout and Railway environment are outside this task.

## Small implementation contract

One verified successful, reconciled Token-2022 parameter parent, one fixed
ChangeSpec (`token_2022_active_newer_transfer_fee_basis_points_v1`), one unchanged
finalized account fixture, and one positive raw u64 amount interval. Only the
hypothetical `TransferChecked` amount varies. Parent economic findings are allowed.
The existing input validator, typed builder, mint mutation, paired fresh-VM
analyzer, report verifier and reproducer remain authoritative. No live acquisition.

Derived inputs clear the request-bearing source-capture identity; their fixture
identity continues to identify unchanged observed account evidence. An explicit
parent-to-derived-action binding records the original input/capture, original and
hypothetical amounts, exact message, fixed execution commitments and algorithm.
Normalizing the amount and request identity reconstructs the parent byte-for-byte.

`recipient_loss_exceeds` means baseline recipient public credit minus proposed
recipient public credit strictly exceeds a canonical nonnegative u64 threshold.
It is evaluated with checked signed i128 arithmetic only after both executions
succeed and reconcile. It is a declared user condition, not a protocol bug or
safety verdict. Execution rejection, rollback failure, reconciliation failure,
admission refusal and unavailable execution are separate outcomes.

The deterministic queue has endpoints, original amount, neighbors, interior
samples and official pinned fee-helper rounding/cap hints. Refinement samples
smaller amounts without assuming monotonicity. All evaluations, including
refinements, share a ceiling of 64; the hard VM ceiling is 128 (two per admitted
case), with an optional lower explicit VM budget. Verification runs no VM.
Parent verification adds zero VM calls. No confirmation executions are hidden.

The artifact stores complete parent/case evidence in the existing local CAS,
sharing repeated JSON subobjects and large data strings. The completion manifest
is written last; interrupted artifacts are explicitly incomplete. Verification
replays the deterministic scheduler using retained reports, rebuilds derivations,
reverifies each paired report and recomputes every ledger/summary/witness field.
Full reproduction instead uses the same scheduler and actual paired analyzer.
Witness reproduction reruns only its pair and claims only that witness.

## Operator commands and exact schema

```json
{
  "schema_version": 1,
  "dimension": "transfer_amount_raw",
  "min_raw": "1",
  "max_raw": "10000",
  "predicate": {"kind": "recipient_loss_exceeds", "threshold_raw": "100"},
  "budget": {"max_evaluations": 64, "max_refinements": 16, "max_vm_calls": 128}
}
```

`max_vm_calls` defaults to 128 when omitted. Quantities must be canonical u64
decimal strings (no leading zeros, plus signs or JSON numbers). Unknown fields,
other dimensions/predicates, reversed/zero/above-balance domains and budgets above
the hard ceilings fail before search. Supported bounds and the requested bounds
are included in range errors. Parent admission requires both successful reconciled
sides; an ordinary semantic consequence is eligible.

```sh
eplyx parameter search --change change.json --parent-report parameter-report.json \
  --spec search.json --out ./parameter-search --format json
eplyx parameter verify-search --artifact ./parameter-search --format json
eplyx parameter reproduce-search --artifact ./parameter-search --format json
eplyx parameter reproduce-witness --artifact ./parameter-search \
  --witness <witness_sha256> --format json
```

For a parent with `source_capture_sha256`, also pass `--capture` with the exact
original capture bytes. The resolver rechecks that its original request/input is
identical to the parent, before deriving actions. Fixture-only retained parents
need no external capture. The portable artifact then needs neither that path nor
project/repository files, credentials or RPC. The same compatible `eplyx` runtime
is required for full and witness reproduction. Read verification does not execute
the parent or the cases. All CLI operations use the existing empty-environment
worker. Exit 0 means the requested operation completed, including a bounded search
with witnesses or budget exhaustion; invalid evidence/arguments or unavailable
internal execution that prevents an artifact return a nonzero error. No deployment
gate is applied.

The insertion-ordered queue prioritizes minimum, maximum and original amount,
then their neighbors, continuous threshold neighborhood hints, baseline/proposed
official fee neighborhoods and 15 evenly spaced interior neighborhoods. For
intervals of at most 32 amounts it adds every amount. Duplicate amounts preserve
all reasons in deterministic insertion order. Official `calculate_fee` supplies
individual fee-boundary hints with at most 64 integer bisections per target; it
never substitutes for paired VM results. No inverse-fee equality is assumed.

After the first predicate or anomaly witness, one refinement batch samples the
witness's smaller neighbors and interior fractions. At most the requested number
of distinct refinement evaluations run, sharing the total evaluation and VM
budgets, before continuing the seed queue. Already executed amounts are reused.
Refinement preserves the original witness and exact signature relationship.
The public numerical claim is always "smallest matching amount among the executed
cases". Rounded/capped loss can be non-monotone; neither neighboring misses nor
sampling certify a global minimum. Completion of the generated seed plan is
separate from full enumeration of the numeric interval.

`manifest.json` is published atomically only after completed reports verify.
An `incomplete.json` marker without that manifest is an interrupted artifact,
never a completed search. `summary.json` and `report.md` are privacy-scanned public
projections. Exact capture/report CAS bytes stay in a private local directory and
are not uploaded. The summary contains identities, exact quantities, typed
conditions and failure signatures, not provider-bearing capture bytes. Repeated
large JSON subtrees/account strings share the existing `EvidenceStore` CAS;
hydration checks paths, hashes, nesting and a 512 MiB aggregate bound. Each CAS
object uses the existing 128 MiB local read ceiling.

The counted analyzer entry point calls exactly the existing analyzer execution
sites and adds no seed/control/confirmation VMs. Legacy `analyze` calls the same
implementation. The explicit retained-runtime compatibility policy admits the
completed Step 11A source hash
`51643aa9ca1fd8dcba3d21bf8af8b9e45ac53f39b92a66616dade06b3445de74`
with its exact lock hash
`4417749a09e246a4fb8c111611fdf5825d295a28a414deeb30cd353eccec992a`,
as well as the already supported older pair. Every other execution-runtime field
must still agree and reproduction must still match actual canonical outputs.
Frozen receipts and qualified fixture hashes are never rewritten.

Unsupported: another proposal/parameter/protocol, pending fee schedule,
unverified/incomplete/tampered parent, unsupported account extensions or authority,
zero/above-source-balance amount, extra instructions, state/balance/authority/Clock
mutation, live acquisition, population coverage, policy DSL, governance and hosted
productization. Source sufficiency does not establish execution success; actual
recipient/withheld overflow and rejection are retained as VM outcomes.

The CLI supervisor applies a 900-second operational wall-time limit to search
operations. A killed worker cannot publish a partial completed manifest; this
operational limit is outside deterministic analytical facts. Executions remain
serial, using the existing transfer compute ceiling (1,400,000 units per VM),
existing bounded input reads, a maximum of 64 retained paired reports and the
512 MiB artifact ceiling. Completion manifests, public summaries and readable
reports are cross-checked on read; verification never repairs a projection.

## Qualification

The untouched retained source balance is `17621` raw. The `1..10000` domain with
current `50` bps, proposed `200` bps, maximum cap `18446744073709551615` raw and
threshold `100` evaluates 64 distinct amounts with 16 refinements and exactly 128
VM calls. All 64 pairs reconcile. It records 25 declared-condition matches and
zero rejection/invariant anomalies, then stops with `budget_exhausted`.

At `10000`, actual recipient credits are baseline `9950` and proposed `9800`
(loss `150`). At `1`, both actual credits are `0` and loss is `0`; this is an
executed rounding control. The smallest matching amount **among executed cases**
is `6732`: baseline `6698`, proposed `6597`, loss `101`. The domain has 10,000
amounts; 9,936 remain untested. No global minimum or safety verdict is asserted.
The full portable CAS occupies approximately 6.2 MB, sharing account/program
and capture subtrees across the ledger. Public identities and validation results
are retained in `docs/examples/parameter-edge-search-qualification.json`; exact
provider-bearing artifacts remain in ignored local `artifacts/step12a` storage.

Focused tests cover canonical schema, invalid/stale/pending parent evidence,
deterministic huge-domain queues, cap/rounding neighbors, immutable derivation,
actual paired threshold/zero-effect controls, equal-rate controls, shared budgets,
non-monotone capped controls, actual deployed-program overflow rejection and
rollback, unavailable/reconciliation classification, exact full/selected replay,
resealed ledger/amount/threshold/ordering/VM-count/coverage/minimum tampering,
CAS corruption, public-summary tampering, interruption and an empty-environment
CLI worker. Existing parameter tests independently cover zero/maximum rates,
malformed mint shapes, withheld inconsistency and frozen receipt compatibility.
