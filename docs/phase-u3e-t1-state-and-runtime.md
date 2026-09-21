# Phase U3E — T1 historical state and the runtime-context boundary

**T1-D: historical runtime context is not proven. Production replay remains
0/4.** T1 now has complete ordinary account pre-state and post references under
the existing qualified archive boundary, plus a fresh clean same-slot screen.
Historical Clock, Rent and EpochSchedule were acquired. The execution-slot
SlotHashes request returned an explicit null account. No primary transaction was
executed locally, no fidelity comparison ran and no semantic subject was promoted.

The [complete 41-item report](examples/phase-u3e-validation/final-report.json)
answers the requested reporting checklist. [Causal stages](examples/phase-u3e-validation/primary-stage-table.json)
and the [final inventory](examples/phase-u3e-validation/final-account-inventory.json)
retain full identities and account-level evidence.

## Frozen controls and scope

Initial and final HEAD: `84ba410287a4a7ed315a7d779dbaad625fe3bf39`.
Tracked files were initially unchanged; additive U3D/U3D.2 files were already
untracked. No commit was created. The [preimplementation record](examples/phase-u3e-validation/preimplementation.json)
retains the exact worktree inventory and hashes of earlier attempts. Those
attempts were not overwritten.

All 344 U3A checksums, eight U3B LUT proofs, four primary U3C envelope artifacts
and U3D/U3D.2 acquisition artifacts were revalidated. The sample fingerprint
remains `b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af`;
policy SHA-256 remains
`3a95c98305aa69e2b7f0502c6e9472f47aea863c3fdf32c12e3644a06a0180c3`.
All six product reports retain their U2 bytes and exits **0/0/1/1/3/5**.

The current request explicitly gated T2–T4 on T1-A. Consequently this attempt
made no new requests for T2–T4. Their prior binary failures remain intact.
No hosted code, production executor, ReplayRecord schema, semantic subjects or
adapter admission policy changed.

## Transport diagnosis and bounded policy

[`historical_transport.py`](../scripts/historical_transport.py) records each
logical request, exact parameters, account, slot, attempt, UTC start, elapsed
duration, process exit, HTTP status, safe response headers, content type/length,
available trace identifiers, sanitized stderr, JSON-RPC error, body length/hash
and empty-body flag. Provider configuration reaches curl through stdin; retained
provider identity is only `https://solana-mainnet.g.alchemy.com`.

The existing public archive configuration and its earlier qualification were
reused. No private credentials were read or used. Qualification remains
conditional on that existing provider contract: matching a hostname or echoed
slot does not independently establish historical truth.

The [policy](examples/phase-u3e-validation/transport-policy.json) was written
before the first new request: **four attempts maximum per exact context,
sequential execution, delays 0/2/5/10 seconds, 20-second curl timeout and
25-second process timeout**. Redirects, provider switching, slot changes and
current-state fallback are absent. Contradictions, missing accounts and RPC
errors are not retried. Every attempt survives a later success.

DNS, connection, TLS, HTTP, rate-limit, gateway, timeout, empty-body, truncated
body, invalid-JSON, RPC, missing-account, wrong-context, contradictory-account
and unknown failures have distinct classifications. The old zero-byte failures
cannot retrospectively be assigned a provider cause from the new diagnostics.

The exact previously failing mapping request at **448195165** succeeded on its
first attempt: HTTP 200, curl exit 0, **39,866 response bytes**, 29,704 account
bytes, expected Scope owner and validator lamports. Its account-data SHA-256 is
`1d4d40f87f5c82460565a5780e727073cdd787f528636f059628df0a8d55a110`.
The [single-request probe](examples/phase-u3e-mapping-probe/result.json) therefore
did not reproduce the old failure. This does not identify its original cause.

Across all new archive work there were **39 requests: 38 accepted results and
one missing runtime account**. Every context used one attempt. No live retries
were needed and no live response body was empty; retry behavior is demonstrated
by labeled simulations and mutations, not by a claimed live recovery chain.

## T1 ordinary state and references

T1 remains
`5eLacZQNT4qYoCd6w9KyULUmycVFeEMjcuoS9sEZXsUifVKXi6AkGUx93zgmzEGZ2KUDzfB8Z7m8ZoAo3fvSWg97`,
execution slot **448195166**, deposit V2 at outer index **5**. The original eight
instructions, native v0 key order and sealed LUT proof are unchanged.

The [inventory frozen before capture](examples/phase-u3e-validation/state-inventory.json)
was recomputed from the sealed artifacts, original instruction roles and observed
dependency discovery. It contains **23 message accounts, four ProgramData
accounts and one separate LUT input**. Runtime requirements are recorded
separately. No omitted farm/oracle state was invented from a program name.

The [state attempt](examples/phase-u3e-state/result.json) acquired **32 account
boundaries for 16 addresses**, plus one genesis check. Fifteen accounts were
present at both boundaries; the market-authority PDA explicitly returned null
at both, consistent with its zero validator balances and authority role. This
exception cannot satisfy a missing Scope, token, ATA or protocol account.

| Accounts | Addresses | Per-boundary account bytes |
|---|---:|---:|
| Scope prices / mappings / TWAP | 3 | 28,712 / 29,704 / 344,136 |
| KLend reserves / obligation / market | 4 | 8,624 each / 3,344 / 4,664 |
| User ATA / liquidity vault / collateral vault | 3 | 179 / 175 / 165 |
| Collateral mint / liquidity mint | 2 | 82 / 676 |
| Payer | 1 | 0 |
| Market authority PDA | 1 | explicit absence |
| Passed System / Farms program accounts | 2 | 21 / 36 |

Pre-state is S−1; post references are end-of-S. Every returned account retains
owner, lamports, executable flag, rent epoch, complete base64 data, data hash,
canonical account hash and request provenance. Current/post context cannot
substitute for pre-state, and previously captured exact-context hashes must
agree. Owner checks follow instruction roles and validator token metadata.

The real ATA bytes establish its mint, wallet, initialized base state and
existing-account identity. This proves historical inputs for the expected path;
it does not prove that ATA ran locally. Token-2022 extension entries remain
visible in the decoder output, including entries without new semantic support.

The offline Rust [state verifier](../engine/examples/verify_envelope_state.rs)
uses the existing KLend, SPL and Token-2022 decoders for **18 boundary snapshots
of nine accounts**. No new protocol decoder or semantic evaluator was introduced.
Readonly accounts compare identically across the two boundaries; owner and
executable changes are rejected. Raw writable differences are retained in the
[boundary checks](examples/phase-u3e-boundary-proof/result.json), not hidden by
selected typed fields.

Scope prices/mapping/TWAP sizes, discriminators, account relationships and ordered
mapping keys match the pinned interface. The [additional original source files
and license](examples/phase-u3e-interface/manifest.json) extend the existing U3C
source provenance. Source has not been reproducibly matched to the historical ELF.

The selected Scope entries are **344: CappedFloored, 279: ScopeTwap1h,
13: CappedFloored, 456: PythLazerEMA**. Their four remaining accounts all alias
Scope itself. The retained dispatcher reads the captured Scope prices, mappings
and TWAP accounts for these types. All four TWAP-update bitmasks are zero.
The archive TWAP bytes are unchanged; the price account has the already observed
one-byte change at offset 25,592. Neither observation is a local Scope causal test.

The final screen reran the existing Rust screening implementation on a fresh
block response over **all 26 ordinary message/ProgramData addresses**. It found
**712 transactions, T1 at index 63, zero conflicts**, matching the old screen.
The final ordinary set did not expand. Runtime sysvars are bank-managed inputs
and are not proved by this ordinary-account screen.

## Exact runtime stopping point

Stages 0–5 reached ordinary boundary acquisition and reference validation. Stage
6 obtained these execution-slot Clock fields:

| Field | Captured value |
|---|---:|
| slot | 448195166 |
| epoch | 1037 |
| epoch_start_timestamp | 1789708003 |
| leader_schedule_epoch | 1038 |
| unix_timestamp | 1789764316 |

Clock's timestamp agrees with the frozen validator block time. EpochSchedule
has 432,000 slots per epoch and agrees with Clock's epoch. Captured Rent contains
5,080 lamports per byte-year, exemption threshold 1.0 and burn percent 50. These
are historical observations, not wall-clock approximations or runtime defaults.

The next request was `getAccountInfo` for
`SysvarS1otHashes111111111111111111111111111`, finalized/base64, exact slot
**448195166**. It returned **HTTP 200, curl exit 0, 98 nonempty bytes, valid JSON,
the requested context and `value: null`**. It had no JSON-RPC error. Raw body SHA-256:
`5fd858ec68cd6b74c33a120a1aded1d557ebd50b8d09d87fcceaee03fe6ea121`.
This is `missing_account`, not an empty HTTP response, timeout or rate limit.
The [runtime attempt](examples/phase-u3e-runtime/result.json) stopped without retry.

There is an important limit to the blocker: LiteSVM's native-v0 address loader
obtains SlotHashes from the sysvar cache before lookup, but T1's active table has
`deactivation_slot = u64::MAX`, so its status branch does not inspect historical
hash membership. **The null result does not establish that historical SlotHashes
contents materially affect T1.** It establishes missing bank-input evidence in
this acquisition. A safe alternative treatment for the complete historical
envelope has not been proved. Neither an empty synthetic sysvar nor LiteSVM's
default was substituted to advance the stage.

The [runtime model](examples/phase-u3e-validation/runtime-model.json) and
[source pins](examples/phase-u3e-validation/runtime-source-pins.json) record this
boundary. LiteSVM 0.16.0's existing mainnet feature profile remains pinned;
historical validator equivalence is not claimed. Instructions must eventually
be generated from the complete original envelope. The existing local historical
execution mechanism disables signature/blockhash inclusion checks, but no such
execution or message alteration occurred for T1 here.

Stages 7–12 were not reached. The Scope causal variation test, real ATA execution,
KLend execution, outcome reconciliation and raw/typed post-state fidelity are
all outstanding. Runtime mutations 17–28 and multi-action mutations 29–32 were
not run or counted. No broad exclusion, tolerance or fidelity shortcut was added.

## Funnel and decisions

The denominator remains **156 captures, nine recognized-action transactions and
12 action observations** including one failed original. Successful-only counts
are eight transactions and ten actions. Eight LUT transactions are proven,
including the failed original; seven successful LUT transactions contain nine
action observations.

The primary funnel is **4 classified → 4 envelope-admitted → 1 complete observed
binary set → 1 ordinary historical state/reference set → 0 complete runtime
contexts → 0 executions → 0 outcome matches → 0 post-state matches → 0
production semantic evaluations → 0 assured subjects**. Ordinary state completion
does not mean complete execution inputs.

First current experimental blockers across the nine recognized transactions are
**one RuntimeContext, three Transport failures during binary capture, two Envelope
rejections, two SemanticAttribution rejections and one FailedOriginalPolicy**.
The envelope cases are the separate ATA and legacy System lifecycle. The two
multi-action contracts first report unsupported attribution in the experimental
policy, alongside additional envelope dependency gaps. This does not mean their
whole-transaction replay succeeded. [Every signature and its blockers](examples/phase-u3e-validation/terminal-blockers.json)
is retained. T2–T4 retain their exact U3D.2 failures and had zero
new requests. No corpus, production ReplayRecord, candidate differential,
wrapped-SOL execution or multi-action replay was created.

**Adapter #4: B — GENERIC MODERN REPLAY STILL BLOCKS NEW PROTOCOL WORK.**
**Adapter Contract v2: STILL NEED ADAPTER #4**, after the generic blockers are
resolved. The target/dependency distinction is useful evidence, but this
non-executed historical envelope does not justify a contract redesign.

## Reproduction, tests, performance and storage

```bash
python3 scripts/rebuild-kamino-u3e.py
python3 scripts/test_historical_transport.py
python3 scripts/test_kamino_u3e_state.py
python3 scripts/test_kamino_u3e_runtime.py
python3 scripts/mutate-kamino-u3e.py
cargo test --offline -p eplyx-engine
```

Rebuilding removes provider configuration from subprocesses, uses Cargo offline,
reconstructs prior binaries/LUT/envelope evidence, rechecks all retained new
responses and reruns the existing screen. New archive transport is explicitly
forbidden during derivation. This is **offline evidence reproduction, not offline
transaction replay**.

**717 tests passed: 603 Rust and 114 Python**, including 27 new tests. Clippy with
warnings denied, formatting and diff checks pass. **16/16 requested transport/state
mutants** were killed by named assertions with passing unmutated controls;
[results and assertion output](examples/phase-u3e-validation/mutations.json)
distinguish simulated negative controls from production evidence. No compiler-only
failure counts as a kill. Working sources and prior evidence were preserved.

Measured transport/parsing durations were **0.836 s** for the isolated probe,
**16.308 s** for genesis plus 32 boundary reads, **4.957 s** for the final screen
and **1.035 s** for runtime-input acquisition. The full-state request span was
**17.449 s**; it excludes prerequisite derivation and final receipt validation.
No new binary-acquisition, VM-execution, fidelity-comparison or semantic-evaluation
duration exists. [Performance boundaries](examples/phase-u3e-validation/performance.json)
keep those stages separate.

Raw new acquisition responses occupy **4,947,675 bytes**. Decoded account images,
including the repeated probe and paired boundaries, contain **888,078 bytes**;
**479,137 bytes** remain if identical data images are counted once. The **408,941
duplicated data bytes** are reported without deduplicating contexts. Ordinary
post-reference account data totals **429,142 bytes**. Existing historical binaries
are referenced; new binary bytes and replay-record bytes are zero. Runtime-input
data totals 90 bytes, but no complete runtime-context record exists.
[Storage accounting](examples/phase-u3e-validation/storage.json) separates these
categories. [Retained-artifact scanning](examples/phase-u3e-validation/security-scan.json)
found no retained credential-bearing provider URLs or authorization/cookie headers.
The scan includes the older archives. One SHA-pinned, immutable pre-U3A source
file contains the previously documented public demo configuration; that explicit
source exception is recorded separately from new acquisition provider identities.

The next phase should resolve the SlotHashes boundary explicitly: obtain or
reconstruct its historical context, or prove that a narrowly defined runtime
treatment is sufficient for this complete active-LUT envelope. The null response
must remain preserved. Only then should native-v0 execution, the mandatory Scope
causal test and full baseline fidelity proceed. No evidence here answers the
central complete-replay question affirmatively.
