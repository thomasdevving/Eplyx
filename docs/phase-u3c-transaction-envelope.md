# Phase U3C — transaction envelopes and execution dependencies

**Decision B: primary structural envelope admission proven; production replay
still blocked.** All four frozen successful Scope-companion transactions are
classified and admitted by a sealed experimental envelope path. One has a
complete historical capture of its observed executable dependencies; three hit
bounded provider-response failures. Historical state acquisition and runtime
execution for these four targets were not attempted. This establishes a narrow admission contract,
not execution support, baseline fidelity or Kamino production replay.

Primary results are **N=4/4 classified, M=4/4 envelope-admissible,
K=0/4 complete historical binary-and-state dependencies, J=0/4 baseline attempts,
P=0/4 fidelity matches**. Binary capture alone completed for **1/4**.

## Frozen controls and measured U3B blockers

Initial HEAD was `e88d9bf40b9210ceea9d0501522c41ee4daeefba`, with a clean worktree.
This phase belongs to the commit containing this document and its additive
artifacts. [Preimplementation record](examples/phase-u3c-validation/preimplementation.json)
pins every U3A and U3B.2 input hash before code changes.

All **344 U3A checksums**, normal/reversed baseline rebuilds and the **eight
U3B.2 historical LUT proofs** remain unchanged. The fingerprint is
`b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af`;
policy SHA-256 is
`3a95c98305aa69e2b7f0502c6e9472f47aea863c3fdf32c12e3644a06a0180c3`.
The population remains 156 captures, including seven successful and one failed
recognized-action LUT transaction. No acquisition failure was backfilled.

S8 was rederived from the frozen proof artifacts, with a mechanical assertion
before implementation: **four Scope companions, one ATA companion, two
multiple-KLend-action contracts and one failed original**. Exact signatures,
first rejection text, raw hashes, actions, account metas, instruction bytes and
complete timelines are retained in [the envelope evidence](examples/phase-u3c-envelope/summary.json).
The ninth legacy System-companion transaction remains separate.

## Exact Scope identity and interface provenance

The canonical program is
`HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ`, Kamino Finance's **Scope**
oracle aggregator. Its identity follows the official
[Scope program declaration](https://github.com/Kamino-Finance/scope/blob/fe5352366a7215dda5c6f7b867a6bb5929d52c94/programs/scope/src/program_id.rs)
and [generated instruction interface](https://github.com/Kamino-Finance/scope-sdk/blob/d77ac13bced666e07719bba3e026f628cd5d3e61/src/%40codegen/scope/instructions/refreshPriceList.ts).
Unmodified source files, revisions, SHA-256 hashes and original licenses are
retained under [phase-u3c-interface](examples/phase-u3c-interface/manifest.json).

Every primary Scope instruction is `refresh_price_list` (`refreshPriceList` in
the SDK), discriminator **`53bacf83cbfec682`**. The payload is an exact Borsh
`Vec<u16>`: eight discriminator bytes, four little-endian length bytes and the
ordered token indexes. Duplicate tokens and duplicate account references remain
present. This experimental rule requires exactly four fixed accounts plus one
remaining account per token; official Scope can also consume more remaining
accounts for other oracle shapes, which this phase does not admit.

The fixed accounts are `oracle_prices` (writable), `oracle_mappings` (readonly),
`oracle_twaps` (writable) and the Instructions sysvar (readonly). None is a
signer. Scope validates mapping/price/TWAP relationships at runtime; recognizing
these metas does not prove their historical contents.

The official
[refresh handler](https://github.com/Kamino-Finance/scope/blob/fe5352366a7215dda5c6f7b867a6bb5929d52c94/programs/scope/src/handlers/handler_refresh_prices.rs)
updates price entries and, when configured, TWAP state. Mapping configuration
can select derived prices, frozen entries, error handling and reference-price
validation. Scope also requires a top-level invocation and a preceding
compute-budget-only prefix. Moving it after ATA or reserve refresh violates that
execution contract.

The same `oracle_prices` pubkey occurs at account position five in subsequent
KLend `refreshReserve` instructions. The official
[KLend handler](https://github.com/Kamino-Finance/klend/blob/a08760976f51a3a58c4a0c6ea27b4a0e565bca79/programs/klend/src/handlers/handler_refresh_reserve.rs)
reads Scope prices when its reserve configuration requires a price refresh.
Consequently Scope's writes can affect the target's inputs. Whether each reserve
actually consumes a refreshed entry requires historical configuration and
execution; account overlap alone does not prove that branch was taken.

The public source establishes interface and behavioral provenance. It has not
been reproducibly matched to the historical ELF. The acquired deployed ELF hash,
loader, ProgramData and authority are a separate historical-byte boundary.

## Four primary transaction timelines

Full signatures label immutable transaction artifacts below. All four original
transactions succeeded on chain; no local runtime result is implied.

| Target | Signature | Slot | Target outer index |
|---|---|---:|---:|
| T1 | [`5eLacZQNT4qYoCd6w9KyULUmycVFeEMjcuoS9sEZXsUifVKXi6AkGUx93zgmzEGZ2KUDzfB8Z7m8ZoAo3fvSWg97`](examples/phase-u3c-envelope/transactions/5eLacZQNT4qYoCd6w9KyULUmycVFeEMjcuoS9sEZXsUifVKXi6AkGUx93zgmzEGZ2KUDzfB8Z7m8ZoAo3fvSWg97.json) | 448195166 | 5 |
| T2 | [`55EdbRGcpNETQZ2AerE4KuvKC1xKCFzHkFHxxN98gjz7DWzGCGbgXKSXW6q3SoABZBbvEsEKb6sVBt9eGThj2kvk`](examples/phase-u3c-envelope/transactions/55EdbRGcpNETQZ2AerE4KuvKC1xKCFzHkFHxxN98gjz7DWzGCGbgXKSXW6q3SoABZBbvEsEKb6sVBt9eGThj2kvk.json) | 448194517 | 7 |
| T3 | [`5NKNdj527vBVpCGhNdeGwpj9LdQfpSCxSH3b8oaKkLe3EzKbZdrsc2fR7yCuCkYgt8EmYXFDwJidi15y8ZQNY43V`](examples/phase-u3c-envelope/transactions/5NKNdj527vBVpCGhNdeGwpj9LdQfpSCxSH3b8oaKkLe3EzKbZdrsc2fR7yCuCkYgt8EmYXFDwJidi15y8ZQNY43V.json) | 448194462 | 5 |
| T4 | [`49zi8hsUATfn3oM7x3bx8fBWULY9mN6DGHtFCVWRLcuPJFyBkP54XVdZGWbqVecwHzsSzFkVQdvLKPp8r8929Amx`](examples/phase-u3c-envelope/transactions/49zi8hsUATfn3oM7x3bx8fBWULY9mN6DGHtFCVWRLcuPJFyBkP54XVdZGWbqVecwHzsSzFkVQdvLKPp8r8929Amx.json) | 448194435 | 5 |

| Target | Complete original outer sequence |
|---|---|
| T1 | 0 Scope refresh; 1 ATA CreateIdempotent; 2–3 refreshReserve; 4 refreshObligation; 5 depositReserveLiquidityAndObligationCollateralV2; 6 SetComputeUnitLimit; 7 SetComputeUnitPrice |
| T2 | 0 Scope refresh; 1 ATA CreateIdempotent; 2–5 refreshReserve; 6 refreshObligation; 7 borrowObligationLiquidityV2; 8 SetComputeUnitLimit; 9 SetComputeUnitPrice |
| T3 | 0 Scope refresh; 1 ATA CreateIdempotent; 2–3 refreshReserve; 4 refreshObligation; 5 borrowObligationLiquidityV2; 6 SetComputeUnitLimit; 7 SetComputeUnitPrice |
| T4 | 0 Scope refresh; 1 ATA CreateIdempotent; 2–3 refreshReserve; 4 refreshObligation; 5 borrowObligationLiquidityV2; 6 SetComputeUnitLimit; 7 SetComputeUnitPrice |

| Target | Complete Scope data, hex | Ordered tokens | Accounts |
|---|---|---|---:|
| T1 | `53bacf83cbfec68204000000580117010d00c801` | 344, 279, 13, 456 | 8 |
| T2 | `53bacf83cbfec682080000000300c7010d00c8011900cb019400cd01` | 3, 455, 13, 456, 25, 459, 148, 461 | 12 |
| T3 | `53bacf83cbfec68206000000dd000300dd00c7010d00c801` | 221, 3, 221, 455, 13, 456 | 10 |
| T4 | `53bacf83cbfec68202000000a001f501` | 416, 501 | 6 |

T1–T3 use price account `3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH`,
mapping `4zh6bmb77qX2CL7t5AJYCqa6YqFafbz3QJNeFvZjLowg` and TWAP account
`6L6vUts9tYqxHVUCEFVc2mzZw6yxMn8C6a44cp5ga7e9`. T4 uses
`3NJYftD5sjVfxSnUdZ1wVML8f3aC6mp1CXCL6L7TnU8C`,
`Chpu5ZgfWX5ZzVpUx9Xvv4WPM75Xd7zPJNDPsFnCpLpk` and
`GbpsVomudPRRwmqfTmo3MYQVTikPG6QXxqpzJexA1JRb`, respectively.
Every ordered remaining account, requested privilege, writable set and consuming
refresh index is recorded in the transaction artifacts. T3 retains the repeated
`pSPcvR8GmG9aKDUbn9nbKYjkxt9hxMS7kF1qqKJaPqJ` stake-price account and repeated
token index 221.

## Semantic target and execution-envelope model

[`engine/src/envelope.rs`](../engine/src/envelope.rs) introduces diagnostic
instruction roles, target observations and a sealed `AdmittedEnvelope` carrying
the original `ProvenV0`. Each target retains `(signature, outer_index)`, program,
the existing U2 ActionId and exact instruction identity. V1/V2 names remain
distinct instruction identities while their existing ActionId families stay
unchanged. No semantic graph, new action enum or finding fingerprint is added.

The policy in
[`kamino/envelope.rs`](../engine/src/protocol/kamino/envelope.rs) admits only the
observed Scope → existing-ATA → reserve refreshes → obligation refresh → one
target → compute-limit → compute-price profile. It checks instruction bytes,
arity, fixed privileges/sysvar identity, ATA PDA derivation and metadata witness,
matching reserve/obligation/market prerequisites, distinct reserves, the observed
action/token-count/refresh-count combinations, target signer and positive amount,
and dependency order. Every top-level instruction has a row; unknown
programs/opcodes, unrecognized shapes and extra dependencies fail closed.

Scope and ATA are **execution dependencies** with no semantic ActionIds or
subjects. Reserve/obligation refreshes are target prerequisites. Compute-budget
instructions remain ordered standard companions. Historical dependency discovery
uses existing top-level, CPI and validator-log infrastructure; no Scope-specific
CPI walker exists. Farms appears through CPI/log evidence in T2 and is captured
as another executable dependency, without new semantic coverage.

The envelope retains the original native v0 message and complete instruction
sequence. A planning guard rejects stripping or reordering any instruction.
Binary and seed-context guards reject missing/tampered/current deployments,
current/intermediate/post-state seed records and same-slot interference. Those
guards are acquisition prerequisites, not raw-state proofs or an execution API.
Deserialized diagnostic flags cannot seal an envelope: policy is recomputed from
the sealed LUT reconstruction.

Ordinary adapter admission, ReplayRecord validation and discovery retain their
old LUT rejection. Production eligibility remains zero. ReplayRecord, seven U2
Kamino subjects, normalized JSON, semantic fingerprints, bundles and hosted
behavior are unchanged. No generic executor or LiteSVM transaction path was
modified; full-envelope native v0 runtime integration remains unproven.

## Historical dependency provenance and acquisition stop

The separate collector receives archive configuration through exported environment
variables and reuses the already qualified archive boundary from U3B.2/Phase 6.
Provider identity is `https://solana-mainnet.g.alchemy.com`; genesis is the frozen
mainnet genesis. No private credentials were read or used. Archive truth remains
conditional on that qualification; matching scheme/host or an echoed context slot
does not independently qualify another endpoint.

Existing `versions::resolve_program_at` reads program accounts and upgradeable
ProgramData at **S-1**, checks returned context, loader/deployment metadata and
hashes the complete loader image, including padding. All transport requests,
nonempty bodies, binary images and provenance are losslessly retained in
[capture.tar.gz](examples/phase-u3c-dependencies/capture.tar.gz), bound by
[capture-index.json](examples/phase-u3c-dependencies/capture-index.json).
Offline revalidation reconstructs every complete ELF from the raw responses,
compares it byte for byte and reruns the existing same-slot screen.

The attempt made **103 requests: one genesis, 101 account reads and one block
screen; 100 succeeded and three produced zero response bytes**. Each request
context had one bounded attempt and no fallback/retry chain. The failures retain
`invalid_json_response`, explicit context and bodyless membership; there is no
nonempty failure body to discard. Future collector runs also write explicit
zero-byte body artifacts. This attempt is preserved without overwriting.

T1 captures ATA, Scope, KLend, SPL Token and Token-2022 historical binaries plus
the native Compute Budget program account. Its full original account set and
acquired ProgramData keys passed same-slot screening. A clean screen establishes
an unambiguous boundary prerequisite; it does not acquire account state.

Historical Scope bytes at T1's and T2's predecessor slots have SHA-256
`c1ddc8a903949eafe175dba8a284c636697dcd50f98a346557320b6b06f3bd87`,
deployment slot **443102298**, upgrade authority
`4R33WT7isNgzALNyvpZKiZQARtZNAXarWB3prUbPkXX7`, and loader image length
**10,485,715 bytes**. The equality is measured for those two contexts; other
unrequested Scope contexts are not inferred from it. T1 KLend hash is
`9db16dd4b7bbfe4f13df850bf880bfc4522fcece06717c0626d625746a3cc85b`,
deployment slot **440486775**. Exact ProgramData pubkeys, authorities, loaders,
observed slots, binary lengths/hashes and discovery routes for every acquired
program are in [resolved artifacts](examples/phase-u3c-dependencies/stage-table.json).

The observed failures were T2's KLend ProgramData chunk at offset 1,048,576,
T3's ATA program-account query and T4's Compute Budget program-account query.
They establish acquisition gaps in this attempt, not nonexistent deployments
or permanently unavailable archive history. Acquisition stopped for the cohort
at this measured boundary, following the request's early-stop rule. It did not
move to secondary groups to create replay successes.

**Zero historical protocol/dependency state snapshots were acquired.** S-1
pre-state, end-of-S fidelity references, account owner/data validation and
historical bank/runtime context remain outstanding. Any same-transaction Scope
mutation must arise by executing Scope before KLend; independently acquired
intermediate/post-state may never seed it.

The first collector checkpoint mislabeled its clean-screen pending state as
`dependency_state_unavailable`. No state query occurred. The corrected offline
derivation records C5 as **not attempted**, with no measured state failure for
T1. The original checkpoint is retained in the archive and cannot be used as
evidence that state acquisition failed.

## Causal stage table and execution matrix

C1 LUT proof; C2 structural classification; C3 envelope admission; C4 historical
observed-program binary capture; C5 historical dependency state; C6 executable
transaction; C7 baseline attempt; C8 outcome match; C9 post-state fidelity;
C10 semantic evaluation. “Passed” denotes only its named completed stage.

| Target | C1 | C2 | C3 | C4 | C5–C10 | Next observed blocker |
|---|---|---|---|---|---|---|
| T1 | passed | passed | passed | passed | not attempted | none observed after C4; cohort stopped at other targets' binary acquisition gaps |
| T2 | passed | passed | passed | failed | not attempted | dependency_binary_unavailable: zero-byte KLend ProgramData chunk response at 448194516 |
| T3 | passed | passed | passed | failed | not attempted | dependency_binary_unavailable: zero-byte ATA program-account response at 448194461 |
| T4 | passed | passed | passed | failed | not attempted | dependency_binary_unavailable: zero-byte Compute Budget program-account response at 448194434 |

The [machine stage table](examples/phase-u3c-dependencies/stage-table.json) has
separate C1–C10 statuses. New primary blocker distribution is **three C4 fetch
failures and one unattempted C5 continuation**. That last status is a stopping
decision, not a diagnosed later blocker. All four advance past their old Scope
companion rejection in the experimental structural path; none gains production
ReplayEligibility.

| Program/instruction | Recognized role | Historical binary | State acquired | Executed locally | Semantic support |
|---|---|---|---|---|---|
| KLend deposit/borrow V2 | semantic target | complete T1; partial/unattempted others | no | no | existing U2 subjects conditional on fidelity and attribution |
| KLend refreshReserve/refreshObligation | prerequisite | same KLend image | no | no | no independent finding family |
| Scope refreshPriceList | execution dependency | complete T1/T2 | no | no | none |
| ATA CreateIdempotent, existing-token witness | execution dependency | complete T1/T2/T4 | no | no | none added |
| SPL Token/Token-2022, observed CPI | standard executable dependency | complete T1 | no | no | existing Token-2022 adapter unaffected; no dependency findings |
| Farms, observed CPI | external executable dependency | complete T2 | no | no | none added |
| Compute Budget | standard companion | native owner identified T1/T2; T4 query failed | bank context pending | no | none |
| Unknown external program/opcode | unsupported companion | not admitted | no | no | none |

Only offline decoding, reconstruction, structural admission, historical program
acquisition and one same-slot screen ran for these targets. **No frozen U3C
transaction was submitted to LiteSVM, no U3C baseline outcome/post-state was
compared, no new semantic findings were emitted and no production-derived
Kamino ReplayRecord was created.** Existing product/runtime controls ran normally. The controlled
test varying dependency-produced state and observing KLend's changed input/result
requires runtime execution and remains outstanding. Planning/provenance guards
are not a substitute for that experiment.

## Multi-action observations and attribution

[Structure analysis](examples/phase-u3c-envelope/structure-analysis.json) preserves
every action, account list and instruction-local CPI group. The two successful
multi-action cases are:

| Signature prefix | Ordered targets | Shared obligation | Shared reserve |
|---|---|---|---|
| `3zswY7FX…` at 448195147 | deposit V2 outer 9; borrow V2 outer 13 | yes | no |
| `37XJKQbB…` at 448194959 | borrow V2 outer 8; deposit V2 outer 12 | yes | no |

Each changes a shared obligation before the later action and uses a different
reserve. They preserve distinct outer indexes and existing deposit/borrow
ActionIds. The failed original at 448195003 also retains its two observations;
its successful-original policy is unchanged.

Whole-transaction token balances combine effects. Instruction-local CPI transfer
groups can identify transfers, but do not by themselves prove independent
obligation/reserve/accounting/rounding deltas for all seven existing subjects.
Eplyx has no proved instruction-boundary state snapshots here. Attribution is
therefore **unsupported_multi_action_attribution**; both observations remain
visible and no action-scoped findings are fabricated. No multi-action replay or
semantic evaluator redesign was implemented.

## ATA metadata and separate legacy lifecycle

All primary ATA instructions are exact `01` CreateIdempotent with six accounts,
the correct System/token programs and a derived ATA identity. Each ATA has
positive pre/post lamports, matching pre/post token entries and no inner
instruction group under its ATA instruction. The metadata supports an
existing-token/no-creation path. The real instruction is retained, and historical
token-account bytes still need validation before execution.

The separate ATA-first-rejected LUT transaction, `2dc94E8M…` at 448194341, has
CreateIdempotent at outer 0, an existing token account with **1,488,440 lamports
at both boundaries**, no ATA creation CPIs and a surviving post token entry.
It was inspected diagnostically and receives no new admission/replay claim;
the narrow primary profile requires Scope. Lifecycle creation support was not
added.

The legacy `2bcGD3XP…` at 448195333 begins with a **1,667,810-lamport System
transfer**, ATA CreateIdempotent, and SPL `SyncNative`. Deposit is outer 6,
with farm bookkeeping before/after it; SPL `CloseAccount` is outer 8. Its wrapped
SOL account starts with **1,488,440 lamports** and ends at zero. This requires
funding/synchronization/closure fidelity, a different scope from primary
existing-ATA validation. Its unchanged System first rejection and complete
timeline are retained separately; it never enters the eight-LUT denominator.

## Offline reproduction, checks and storage

```bash
./scripts/rebuild-kamino-u3c-envelopes.sh --verify
./scripts/rebuild-kamino-u3c-envelopes.sh --verify --reverse
./scripts/rebuild-kamino-u3c-envelopes.sh --output /tmp/eplyx-u3c-derived
python3 scripts/test_kamino_u3c_envelopes.py
cargo test --offline -p eplyx-engine --test transaction_envelope
python3 scripts/mutate-kamino-u3c-envelopes.py --output /tmp/eplyx-u3c-mutations.json
```

The rebuild uses `cargo --offline`, strips RPC/archive/API configuration from
derivation processes and serves resolver requests only from the retained
response map. A request for any uncaptured context fails. Reversed input order
preserves canonical artifacts. All original U3B proofs are compared in full;
binary captures and screening are reconstructed rather than trusting success
flags. Compression preserves every original byte without trimming ELF padding
or combining historical contexts. No generalized deduplication was introduced.

**20 new Rust and 10 new Python tests pass.** With the existing 421 library,
4 historical, 27 LUT, 10 Kamino, 16 replay, 12 Token-2022, 24 U3B acquisition
and 30 U3A integrity tests, **574 normal tests passed**. Negative-control
messages and simulated provenance records are labeled separately from production
evidence. They never count as runtime proof.

**13/13 behavioral mutants were killed by named assertions**, covering arbitrary
program admission, Scope ordering/stripping, current state/binary context,
false dependency semantic support, first-only observations, merged ActionIds,
whole-transaction attribution, hidden extras, failed originals, missing ATA
existence evidence and duplicate known dependencies. The ATA creation mutant is
an explicitly transformed negative control; the inspected primary ATA paths did
not create accounts. No compiler-only failure is counted. Isolated sources are
restored and mutation builds use a separate Cargo output directory.
[Exact results](examples/phase-u3c-validation/mutations.json) state the evidence
boundary for each assertion.

All six product report bytes and exits **0/0/1/1/3/5** match U2 before/after.
U3A/U3B evidence hashes and existing semantic controls are unchanged. Clippy with
warnings denied, Rust formatting and diff checks pass. No existing replay
correctness bug was found; the new collector's pending-state label was corrected
as described above.

The measured capture interval took **63.655 s**, from receipt initialization
through final receipt hashing; builds and pre-acquisition envelope checks are
excluded. Measured RPC-helper calls total **59.869 s**, including response JSON
parsing. Expanded capture is **107,368,223 bytes in 115 files**:
63,510,581 response bytes, 43,748,572 loader-image bytes and 109,070 metadata
bytes. Lossless archive storage is **5,130,046 bytes**. External protocol state
storage is **zero**. Offline timing and total envelope/interface/validation
storage are recorded separately in the
[final report](examples/phase-u3c-validation/final-report.json).
No runtime duration or external-program execution overhead was measured; these
are experimental audit sizes, not ReplayRecord overhead.

## Remaining work and adapter decision

Resume the four primary identities with a separately versioned bounded archive
attempt. Close the three binary fetch gaps, acquire and prove complete S-1 seed
state/end-of-S fidelity references under unchanged interference rules, establish
historical runtime/bank context, then integrate full original native v0 execution.
Run the controlled Scope-state-effect test and actual baseline reconciliation
before granting any replay or semantic claim. Keep intermediate state produced
by execution and retain ATA and compute-budget instructions in original order.

Multiple-action attribution, additional Scope oracle shapes, ATA creation,
System/SyncNative/CloseAccount lifecycles and other companion programs remain
unsupported. Contract v2, hosted changes and adapter #4 were not started.
**Adapter #4 should not begin:** full historical state and modern full-envelope
execution remain generic unresolved requirements. A second internal adapter
pattern is now visible—semantic targets and ordered execution dependencies—but
one Scope envelope experiment does not justify a declarative contract redesign.
