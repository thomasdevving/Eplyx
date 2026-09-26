# T4 — Current-state migration guarantees

T4 folds STA M10, M11, M13 and the observed portion of M14 into the token
migration evaluator. It uses MAIN's ChangeSpec resolver, shared RPC/token/Clock
primitives and the T3 migration VM. No fixed-ratio conversion product or second
candidate executor is introduced. STA remains unchanged at
`ad1897e86ee05ff2255bd5277518a3230ee2340f`.

## Capture, selection and execution

A new mainnet-state analysis freezes the deterministic shape/bucket selection,
exact per-case account sets, authority plan, proposal identity and candidate hash
before any final recapture. `current/current.plan.json` is written and synced first.
Each case checks the final provider’s genesis hash, then records at most three
serial final batches under an
exact monotonic `minContextSlot` chain. Each batch includes Clock; verification
requires its decoded slot to equal the response context. Failure, missing data,
changed source bytes between retries or changed executable bytes is Indeterminate.
No case is replaced and no approximate execution context is used.

Only the accepted final batch supplies the execution world. Program headers and
ProgramData, both mints, source, recorded authority, destination and overlay
collision checks are included. Captured legacy-loader dependencies remain supported;
the proposed migration candidate uses MAIN's upgradeable loader. Final captures
and their hashes are persisted before the empty-environment worker starts.
`ExecutionContext`, current case results and reports have no deserialization
constructor. Replay recomputes selection, request chains, identities, final state
and every local VM outcome before comparing report bytes.

The selected identity, original shape/bucket and discovery amount remain historical
facts. `FullAtFinalCapture` is frozen as the amount policy before capture. An exact
positive final amount is resolved and hash-bound before the VM runs; it is neither
capped nor changed in response to a result. Supported shape or bucket drift is
reported as `SelectionStateChangedButExecutable` and does not receive unqualified
coverage of the discovery shape. Lamport-only changes use the final lamports.
Missing identity, zero/frozen state, changed authority, unsupported extensions or
unclassified data changes receive no execution claim. All token decoding and typed
balance operations use MAIN's shared primitives.

The main analysis includes `current_state_guarantees`, separate from its existing
four analytical readiness axes. An exact final case failure blocks either gate
policy with `CURRENT_CASE_FAILED` (exit 1). An explicit search counterexample also
exits 1, including when the underlying four axes have no known violation. Unavailable selected cases add
`CURRENT_CASE_UNEVALUATED`: block-only warns; strict exits 5 when no known
requirement violation also exists. The original synthetic and frozen-world
reference reports retain their bytes and expectations. Older saved analyses without
this capture binding gain no retrospective coherence claim.

## Authority and search

The bounded second-layer authority plan preserves the original classifications and
reports more specific control facts separately. Exact DLMM pool and vault PDA
relationships establish protocol custody only. The pinned pool decoder lives in
`protocol::meteora_dlmm`; it is not another historical `ProtocolAdapter`, so the
registered semantic-adapter count stays five. SPL multisig resolution retains the
threshold and members, with no private approvals or assumed wallet signer. One
resolved pool never proves its peers. T4 imports only the two additional population
captures its authority reference tests consume, using the existing hash-verified
importer. Their raw bytes remain ignored under the no-provider-URL rule.

Observed search adds three waves of at most 10, 10 and 5 accounts. Ranking uses
untested executable shapes, proximity to earlier failed balances, then descending
balance and address. It reads only earlier wave results. Each wave's exact plan and
parent/input binding are synced before capture. Failed or unavailable identities
stay selected. Replay reconstructs each next wave from recomputed earlier outcomes;
changed plan, capture or freeze records fail. These observed witnesses remain
separate from T3's derived search dimensions and its 64-probe/32-minimization budget.

No exact case, shape or independent reserve rehearsal supplies population-wide
rollout capacity. Current results keep `OfficialTransition = NotTested`, key
possession unknown, and no issuer binding. This is current finalized observation,
not historical bank proof. All testing uses mock providers or frozen local bytes.

## Verification

The port retains seven coherence tests, six frozen authority tests and 21
population/classification/selection tests from STA. Integration tests exercise
exact execution and rollback, provider reordering, amount/shape/bucket/lamport drift,
missing/frozen/changed identities, retry coherence and source drift, capture errors,
immutable plans, capture tampering, refreshed populations, both observed-search
outcomes, offline replay and the combined migration gate.

Verification passed: `make test` ran 1,012 passing tests and the final
`cargo test --no-fail-fast` ran 961; both had zero failures and the same two
existing/source ignores. Final formatting and lint checks pass. Frontend, report
and governance reference checks pass with exact pnpm 11.24.0.
[Command receipts and limitations](phase-t4-verification.json) retain failed
attempts and successful retries separately; the [source test mapping](phase-t4-test-mapping.json)
records adaptations. All 1,633 inventoried STA files retain their hashes and STA
is clean. Live-provider checks remain intentionally not run.
The broader lifecycle assurance-policy evaluator follows in T6; T4 supplies its
exact current-state evidence without inferring lifecycle or issuer policy.
