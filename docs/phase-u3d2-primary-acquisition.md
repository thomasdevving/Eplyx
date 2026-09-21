# Phase U3D.2 — independent T1–T4 historical acquisition

**All four targets were attempted; none qualifies for execution.** T1 reacquired
its complete observed executable set and passed the unchanged same-slot screen.
It acquired one Scope account at both historical boundaries, then exhausted its
bounded attempt for Scope mappings. T2–T4 stopped at historical binary response
failures. **Production Kamino replay matches remain 0/4.**

This supersedes the configuration-only stop as the current result. The earlier
U3D checkpoint is preserved, including its mistaken diagnosis that user setup was
needed. Phase 6 already documented the public archive URL and Origin. Missing
environment exports were not a sufficient reason to stop without checking that
documentation. The resumed run used those documented values; no private
credentials were needed or read.

The user's later instruction explicitly authorized trying T1–T4 independently.
That expanded acquisition breadth beyond the original T1-first gate. Each
target's binary, state, runtime and fidelity prerequisites remain mandatory.

## Control and worktree state

Initial and final HEAD are `84ba410287a4a7ed315a7d779dbaad625fe3bf39`. No commit
was created. The worktree contains additive, uncommitted U3D/U3D.2 scripts,
tests, reports and evidence. Existing tracked files remain unchanged.

[The continuation freeze](examples/phase-u3d2-validation/preimplementation.json)
pins the entire earlier U3D checkpoint and records the revised scope and request
bounds before the new work. The 344 U3A checksums, frozen fingerprint
`b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af`, policy
`3a95c98305aa69e2b7f0502c6e9472f47aea863c3fdf32c12e3644a06a0180c3`, eight
U3B.2 proofs and four U3C envelopes still reproduce offline. All six Stake Pool
product report hashes and exits **0/0/1/1/3/5** remain unchanged.

## Independent target results

Full signatures, requests, response hashes, account inventories and C1–C10
statuses are in the [stage table](examples/phase-u3d2-validation/stage-table.json).

| Target | Historical binaries in fresh attempt | Historical state | Measured stopping request |
|---|---|---|---|
| T1, slot 448195166 | Complete; clean same-slot screen | Scope prices captured at S−1 and S; incomplete set | Scope mappings `4zh6bmb77qX2CL7t5AJYCqa6YqFafbz3QJNeFvZjLowg`, slot 448195165, two zero-byte responses |
| T2, slot 448194517 | Incomplete | Not attempted | Farms ProgramData `5Fz4tY19ihWJAW1j9RNoEQJKSdn5mQHjcVrcZ5BRK5E9`, slot 448194516, chunk offset 3,670,016; zero-byte response |
| T3, slot 448194462 | Incomplete | Not attempted | Scope ProgramData `DuKtB88SQGJbow5jxmUSwgJWmC7zma8mX3Nodz1Gsjs2`, slot 448194461, chunk offset 0; zero-byte response |
| T4, slot 448194435 | Incomplete | Not attempted | Farms program account `FarmsPZpWu9i7Kky8tPN37rs2TpmMrAZrC7S7vJa91Hr`, slot 448194434; zero-byte response |

The new binary attempt has **70 requests: 68 account reads, one genesis and one
block read; 67 successes and three failures**. It uses the existing collector's
one-attempt-per-context policy. Failures retain `invalid_json_response` and
explicit zero-byte body files. T3's original ATA query and T4's original Compute
Budget query now succeeded. T2's original KLend gap was not revisited because the
fresh resolver stopped earlier at Farms. None of this erases the original U3C
attempt or proves unavailable deployments.

The state collector allows at most **two** attempts for transport/invalid-JSON
failures, with no fallback or retry on contradictory account/context evidence.
It made **seven requests: two genesis and five account attempts**, with three
successful responses and four zero-byte failures. Genesis and the price-account
post-state each needed two attempts; the price pre-state needed one. The mappings
pre-state failed twice. No further account requests followed that target failure.

All **77 network requests** were acquisition requests. There was no transaction
submission. The archive trust boundary remains the previously qualified exact
finalized-slot contract, selected using the documented public configuration;
matching provider host or echoed slot alone does not establish archive truth.

## Inventory and boundary evidence

[Cohort inventories](examples/phase-u3d2-validation/account-inventories.json)
retain all **23/25/22/24** original message accounts for T1/T2/T3/T4, in original
order, with signer/writable flags, static/lookup origin, and every ordered
instruction-account reference. The earlier detailed T1 inventory remains intact,
including four ProgramData accounts and its separate historical LUT proof.
Acquired state facts are separate from structural roles and unresolved state.

T1's original instruction sequence remains Scope, ATA, two reserve refreshes,
obligation refresh, deposit V2, compute limit, compute price. No instruction,
account alias or duplicate reference was stripped or reordered. Scope's four
remaining-account positions still repeat its program address. Historical
configuration and derived-price relationships remain unvalidated.

The fresh T1 block screen again finds no conflicting writers among **712**
transactions, with T1 at index **63**, screening **26** non-sysvar message and
ProgramData addresses. The existing conservative before/after interference rule
is unchanged. It supports S−1/S ordinary-account boundaries under the archive
contract; it does not supply state or execution-bank sysvars.

Only Scope prices `3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH` were acquired:

| Boundary | Slot | Data bytes | SHA-256 |
|---|---:|---:|---|
| Pre-state | 448195165 | 28,712 | `c3fe3908e3c8778f0b99f6c83def59cc07c95ca7468e8f4585c5fe3d0caf6884` |
| Post reference | 448195166 | 28,712 | `7b412449f556f471ca5623ec1731f03366675455a1509a349388e89e90d231bc` |

Returned contexts, owner, lamports, executable/rent fields, complete base64 data
and raw response hashes are retained. Owner is Scope; lamports match the frozen
validator boundary metadata. [The raw diff](examples/phase-u3d2-validation/scope-price-boundary-diff.json)
finds one changed byte at offset **25,592**, from **48** to **51**. This is an
archive boundary observation. It is **not** local Scope execution, a typed
Scope-state proof, or the mandatory controlled Scope → KLend causal experiment.

Scope mappings, TWAPs, KLend reserves/obligation/market, token/mint/vault state,
authority/payer and passed System/Farms account state remain incomplete. T1's
ATA PDA/metadata witness still lacks historical token-account byte proof and
actual CreateIdempotent execution. No no-creation runtime result is claimed.

## Runtime, fidelity and semantic stages

T1's terminal decision is **T1-D: historical state could not be proven**. The
earlier configuration gap has been replaced by a measured bounded response
failure. C1–C4 passed; C5 has partial acquisition but no complete state proof;
C6–C10 were not attempted. T2–T4 passed C1–C3, failed fresh C4 acquisition and
did not attempt C5–C10.

Clock, epoch, Rent, feature/bank context, Instructions sysvar construction,
historical blockhash handling and native loader fidelity remain unproved for
these targets. The prior [runtime source review](examples/phase-u3d-validation/runtime-review.json)
still applies: Eplyx uses pinned LiteSVM 0.16.0's mainnet feature snapshot, while
its existing historical executor takes a legacy `Message`. No feature profile,
sysvar, blockhash or native v0 integration changed in this continuation.

No primary Scope, ATA or KLend instruction executed locally. No intermediate
Scope state was seeded, no Scope → KLend effect was measured, and no outcome or
typed/raw post-state fidelity comparison ran. Existing runtime-managed exclusions
were not broadened. No Kamino subject gained production assurance; no subject
was added and no dependency gained semantic support.

No production-derived Kamino record, experimental corpus, corpus self-check or
production candidate differential was created. The separate ATA case, wrapped-SOL
System lifecycle and multi-action replay/attribution were not expanded. Whole-
transaction balances were not assigned to individual actions.

## Funnel and blocker distribution

| Gate | Count |
|---|---:|
| Frozen captures | 156 |
| Recognized U2 action observations, all outcomes | 12 in 9 transactions |
| Successful recognized observations | 10 in 8 transactions |
| Historically LUT-proven transactions | 8, including one failed original |
| Successful LUT-proven observations | 9 in 7 transactions |
| Primary envelopes admitted | 4 |
| Complete observed binary sets | 1 |
| Targets with partial state capture | 1 |
| Complete historical state sets | 0 |
| Execution attempted / baseline outcome matched / post-state matched | 0 / 0 / 0 |
| Production-assured semantic subjects / validated production observations | 0 / 0 |

Primary first blockers are **Binary 3, HistoricalState 1**. Across the nine
recognized-action transactions, the other unchanged categories are **Envelope
4** and **FailedOriginalPolicy 1**. Multi-action cases also retain unresolved
SemanticAttribution. Runtime/Fidelity failures have not been measured. The 87
other top-level KLend captures lack recognized U2 actions; 60 captures have no
top-level KLend. The 44 fetch failures and 800 unselected candidates remain
outside the capture denominator.

## Tests, mutation limits and correctness findings

**690 tests passed:** 603 in the full engine suite and 87 Python tests, including
15 new state tests. Final product hashes, Clippy with warnings denied and
formatting checks passed. Exact results are recorded in
[tests.json](examples/phase-u3d2-validation/tests.json).

New tests cover exact retained T1 responses, current/post-state substitution,
wrong Scope owner, validator balance contradictions, missing/truncated oracle
state, interference and screen coverage, incomplete/wrong-context binaries,
tampered Scope/KLend image bytes, bounded retries, retained empty bodies,
immutable attempts, uncaptured contexts, altered hashes and false proof/execution
flags. ATA byte checks use explicitly synthetic negative controls; no historical
ATA proof is inferred from them.

**21/21 source mutants were killed by named assertions:** eight new acquisition
mutants and the 13 existing envelope mutants rerun separately. Their planning/provenance
coverage is not runtime execution evidence. The requested runtime mutation
campaign—skipping actual Scope, observing KLend read changed Scope output,
native v0 execution and false fidelity/semantic promotion in an executed
pipeline—remains gated. No compiler-only failure counts as a killed mutant.

Two correctness findings are distinguished:

1. The previous configuration-only stop was premature; documented public
   configuration was available. Its checkpoint survives and this report corrects
   the current conclusion.
2. A new verifier cache miss was initially caught as a per-target acquisition
   failure. Verification still rejected the modified artifact, but reported the
   less precise derived-result mismatch. It now raises an explicit offline
   evidence error. A regression test covers the correction. This did not alter
   the captured bytes or any existing product output.

No existing engine replay correctness bug was found. No runtime or semantic
feature work was mixed into these corrections.

## Storage, timing and offline reproduction

The binary attempt took **43.693 s** wall time, including **39.350 s** measured
transport/helper time. The state attempt took **2.516 s**, including **2.283 s**
transport/helper time. Builds, later offline validation and test runs are outside
these intervals. No primary runtime execution overhead was measured.

The binary capture preserves **62,138,141 bytes in 84 files**, losslessly stored
as a **3,783,157-byte** archive. Every extracted file was compared with its
original hash before expanded raw/image copies were removed. Receipt/result
metadata remains accessible alongside the archive. State capture retains the two
raw price-account responses and all four zero-byte failure bodies. Exact total
additive sizes and hashes are recorded in
[storage.json](examples/phase-u3d2-validation/storage.json).

```bash
bash scripts/rebuild-kamino-u3d-inventory.sh --verify
python3 scripts/rebuild-kamino-u3d2.py
python3 scripts/test_kamino_u3d_state.py
python3 scripts/mutate-kamino-u3d-state.py
```

The rebuild verifies the preserved checkpoint, compressed capture identity,
every original binary response, loader image and screen through the existing
Rust resolver, then the partial state capture, raw byte diff, inventories and
stage table. It serves only retained requests; no RPC configuration is needed.
All account bytes stay in their exact historical context.

## Decisions and next phase

**Adapter #4: B — MODERN REPLAY STILL HAS A GENERIC BLOCKER.** The measured
barrier is incomplete historical acquisition. Runtime/bank reconstruction,
native v0 integration and fidelity remain additional unproven requirements, not
diagnosed runtime failures. No evidence justifies starting another adapter.

**Adapter Contract v2: STILL NEED ADAPTER #4**, once generic replay prerequisites
are solved. The dependency/target separation remains useful, but partial account
capture adds no runtime evidence for a declarative contract redesign.

Next: a separately versioned, explicitly bounded acquisition phase focused on
transport reliability and the exact unresolved contexts, preserving these
attempts. Capture HTTP/transport diagnostics to distinguish rate limits, empty
gateway responses and other causes; the present empty bodies do not establish
why the provider failed. Complete state and runtime context before integration;
then require full native v0 execution, the controlled Scope-state experiment and
strict outcome/post-state fidelity before any replay or semantic promotion.
