# Phase U3A — reproducible production baseline

**Decision B: NEW PROSPECTIVE U3 SAMPLE FROZEN.** This commit freezes a
**U3 pre-change production baseline collected before modern replay implementation**.
The original U2 classified sample was not recovered. This is a separate dataset;
it does not retroactively reproduce U2 and does not establish Kamino production replay.

## Control and recovery

Pre-freeze HEAD and semantic/replay control: `8855edb`
(`8855edb` is resolved in [unchanged-source.json](examples/phase-u3a-validation/unchanged-source.json)).
The freeze is the commit containing this document, its tooling, historical
preflight and frozen artifacts. No modern replay changes are included.
All 66 tracked engine/interface/server source and pinned Cargo files checked
against that control are byte-identical. The original U3 preflight files were
untracked before this phase and are committed with their original bytes.

**Recovery outcome C: ORIGINAL SAMPLE NOT RECOVERABLE ENOUGH FOR A/B TESTING.**
The bounded project-only search inspected 7,537 workspace JSON artifacts,
discovery/acquisition/cache outputs, tracked/untracked/ignored project evidence,
the exact U2 task scratchpad and task-linked session history. A related archived
Eplyx worktree at `3787a697bbdd9ed08d87e5b63800724d4de2d55c` had no candidate
Kamino/census/signature files. No unrelated personal directories or credentials
were searched. [Recovery record](examples/phase-u3a-validation/recovery.json).

The authentic census, source script and two deposit captures remain under
[phase-u3-before](examples/phase-u3-before/manifest.json). The other 16 supported
observation identities and the original classified denominator are absent.
The earlier 100-signature listing is explicitly excluded: slots
448163393–448164098 and its source/session timing do not link it to the final
1,000-signature census. The census source saved neither its source listing nor
fetch membership/raw transactions; it only retained no-LUT detail. The session
confirms a progress snapshot written before classification was copied to final.
No aggregate-matching reconstruction was attempted.

The historical census SHA-256 remains
`a179c254b8ada5888d76a8544d9bf9e2eed562f713aec0505442322834f49ba5`.
Its count/naming defects and the partial BEFORE results are retained in the
[unchanged preflight report](phase-u3-modern-replay-surface.md).

## Prospective rule fixed before acquisition

[Sampling policy](examples/phase-u3-baseline/sampling-policy.json), version 2:

- Program: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`.
- One finalized `getSlot` anchor: **448195486**. Inclusive selection window:
  **448185486–448195486** (anchor minus 10,000 through anchor).
- One `getSignaturesForAddress` listing, `limit=1000`, `commitment=finalized`.
  Preserve every returned entry, its original `source_index`, signature, slot,
  on-chain error, memo, block time and confirmation status.
- Select the first **200** listing entries in that window, regardless of
  on-chain success. Selected original indexes are **8–207**. No failed fetch
  is replaced. The complete source listing spans **448190016–448195516**;
  entries above the anchor remain in the source denominator but are unselected.
- `getTransaction`: `encoding=json`, `commitment=finalized`,
  `maxSupportedTransactionVersion=0`. Version 2 uses one worker, a 15-second
  timeout, at most three attempts and three seconds between retries.
- Provider identity: `https://solana-mainnet.g.alchemy.com`.
  Mainnet genesis: `5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d`.
  Collection used public documentation access and an Alchemy Origin header;
  no private API key or historical account archive was used.
- Classify every successful RPC fetch, including on-chain failures. Recognize
  the four U2 action discriminators diagnostically, retaining data length and
  account arity; recognition is not adapter admission. Derive successful action
  counts from rows, separately from all-status recognized instruction counts.
  Call unchanged `normalize`, Kamino `accept` and `replay_eligibility(state=None)`.

The [version-1 policy](examples/phase-u3a-validation/capture-v1-policy.json)
was pinned before the initial RPC calls. Its candidate was not frozen: a collector check found that non-JSON
failure bodies were discarded despite the policy requiring their preservation.
That candidate remains locally under ignored `data/phase-u3a-capture-v1` with
an explicit invalidation note. Version 2 was pinned before re-fetching and
uses **the exact same anchor, genesis, raw source listing and selected 200
signatures**, with their source hashes pinned in the new policy. It preserves
all received failure bodies and fetches serially. This is a tooling correction,
not a new convenient window or a recovered U2 sample. The 91 transaction results
successfully captured in both candidates are structurally identical.
[Revision and exact-membership checks](examples/phase-u3a-validation/capture-revision.json).

Policy SHA-256: `3a95c98305aa69e2b7f0502c6e9472f47aea863c3fdf32c12e3644a06a0180c3`.
Pinned original source RPC SHA-256:
`70d1fb93d69513bef928ab5f34489be4b569f4475477d93ed92cbe8f3776f772`.

## Complete frozen population

| Membership | Count |
|---|---:|
| Source signatures | 1000 |
| Fetch selected / not selected | 200 / 800 |
| Fetch success / failure | 156 / 44 |
| Classification rows | 156 |
| KLend top-level / no KLend top-level | 96 / 60 |
| Successful deposit / borrow instruction observations | 5 / 5 |
| All-status recognized deposit / borrow observations | 6 / 6 |
| Recognized-action transactions, all-status / successful | 9 / 8 |
| Successful supported LUT / no-LUT instruction observations | 9 / 1 |
| All-status recognized LUT / no-LUT instruction observations | 11 / 1 |
| Production baseline-validated replays | 0 |

Counts of observations count instructions with durable `(signature, outer_index)`
identities. Three transactions contain two recognized actions. The BEFORE table
has one row per transaction, retains every recognized action, and includes the
one failed on-chain transaction. No instruction family count proves its complete
shape is supported or its execution reproducible.

All completeness invariants hold mechanically:

```text
1000 = 200 + 800
200 = 156 + 44
156 = 156 classification rows
156 = 96 + 60
```

The 44 failures consist of 39 non-JSON provider responses and five JSON-RPC
errors with code `-32015`; their raw bodies/envelopes are retained. These are
fetch failures, not classified irrelevant transactions. Across 332 transaction
requests, all 332 received responses were preserved: 156 successful transaction
captures and 176 error responses, including failed attempts preceding success.
No failed signature was backfilled with another signature.

Message distribution applies to the **156 captured transactions**, not to
unreadable fetch failures:

| Message | Count |
|---|---:|
| Legacy | 1 |
| v0 without LUT resolution | 1 |
| v0 with LUT resolution | 154 |
| Newer/unsupported captured version | 0 |
| Unreadable captured transaction | 0 |

## Complete old-engine BEFORE table

[before-rejections.json](examples/phase-u3-baseline/before-rejections.json)
contains exact signatures, action indexes, old action IDs, versions, LUT/key
counts, companion programs, first rejection code/detail and discovery result.
`D1`/`D2` denote deposit V1/V2; `B2` denotes borrow V2.

| Signature | Slot | Actions | On-chain success | Message | LUTs / loaded | First blocker |
|---|---:|---|---|---|---:|---|
| `2bcGD3XPZoBaQ9VL1oL4HtJ45sSatCYwPycGuLCNh9NxqhHtcahXYFYsYfZNvYk5HgXGaCYZAJrn256ZLgoHMjv6` | 448195333 | D1 | yes | legacy | 0 / 0 | B: System companion |
| `5eLacZQNT4qYoCd6w9KyULUmycVFeEMjcuoS9sEZXsUifVKXi6AkGUx93zgmzEGZ2KUDzfB8Z7m8ZoAo3fvSWg97` | 448195166 | D2 | yes | v0 | 1 / 13 | B: loaded LUT addresses |
| `3zswY7FXGLsVCVJTg8LawCQZp27xBza4M4Cp7mtBm2LxP97kn8eZrXPkCCJv3dt3qMb9fvRXKT3qZTXiucgR6y4Q` | 448195147 | D2+B2 | yes | v0 | 5 / 52 | B: loaded LUT addresses |
| `47XZ8dNPUBxMTnJ3zrxeCKt37PRsckvhKzPSutdUp2NavU1UTTNYRd7rnvEP3pSmHQszrLW3cdB56zTw3kyrwZUf` | 448195003 | D2+B2 | no | v0 | 5 / 55 | B: loaded LUT addresses |
| `37XJKQbB9fRX1R76cKKGSENNNA2RSY1WvjkCx2Q7joN2irKfTTZ5Z6bHgH4rfgDPJvhXf3peTe4mwxisfpSEmimu` | 448194959 | B2+D2 | yes | v0 | 1 / 30 | B: loaded LUT addresses |
| `55EdbRGcpNETQZ2AerE4KuvKC1xKCFzHkFHxxN98gjz7DWzGCGbgXKSXW6q3SoABZBbvEsEKb6sVBt9eGThj2kvk` | 448194517 | B2 | yes | v0 | 1 / 11 | B: loaded LUT addresses |
| `5NKNdj527vBVpCGhNdeGwpj9LdQfpSCxSH3b8oaKkLe3EzKbZdrsc2fR7yCuCkYgt8EmYXFDwJidi15y8ZQNY43V` | 448194462 | B2 | yes | v0 | 1 / 9 | B: loaded LUT addresses |
| `49zi8hsUATfn3oM7x3bx8fBWULY9mN6DGHtFCVWRLcuPJFyBkP54XVdZGWbqVecwHzsSzFkVQdvLKPp8r8929Amx` | 448194435 | B2 | yes | v0 | 1 / 14 | B: loaded LUT addresses |
| `2dc94E8Mvh83c8MiecgRekd7Vj8YE6aexfVeCJK58K46LQcTRjpDomii6pYmwbdeYmdTM3bVbHw6rhFdtwRtSTnT` | 448194341 | D2 | yes | v0 | 1 / 13 | B: loaded LUT addresses |

All nine rows have `admission=rejected`, discovery result
`unsupported_transaction`, and first observed blocker class **B**. Eight are
rejected by the unchanged executable-message gate for loaded LUT addresses;
one is first rejected for a System companion. Exact rejection distribution:

| First rejection | Transactions |
|---|---:|
| message resolves 11 address lookup table entries; lookup tables are normalized but not executed | 1 |
| message resolves 13 address lookup table entries; lookup tables are normalized but not executed | 2 |
| message resolves 14 address lookup table entries; lookup tables are normalized but not executed | 1 |
| message resolves 30 address lookup table entries; lookup tables are normalized but not executed | 1 |
| message resolves 52 address lookup table entries; lookup tables are normalized but not executed | 1 |
| message resolves 55 address lookup table entries; lookup tables are normalized but not executed | 1 |
| message resolves 9 address lookup table entries; lookup tables are normalized but not executed | 1 |
| unsupported program 11111111111111111111111111111111 in a Kamino KLend replay; the supported contract admits KLend and compute-budget instructions only | 1 |

No historical state acquisition, message reconstruction, runtime, reconciliation
or semantic stage was attempted. Those are not diagnosed as later blockers.
Discovery with `state=None` is not a baseline-fidelity test. No production replay
eligibility is claimed. Multiple actions and companion structure are retained
as features; they are not substituted for the actual first rejection.

## Companion and lifecycle evidence

[classifications.json](examples/phase-u3-baseline/classifications.json) retains
every original message, lookup references/indexes, loaded addresses, complete
compiled top-level sequence, decoded diagnostic bytes/account identities, inner
groups, native balances and token balances. Each row points to an exact raw
capture hash; the complete original RPC response remains in `transactions/`.

Companion distribution counts distinct program presence per recognized-action
transaction, including the failed transaction; categories overlap:

| Companion program | Transactions |
|---|---:|
| `11111111111111111111111111111111` | 3 |
| `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL` | 8 |
| `ComputeBudget111111111111111111111111111111` | 9 |
| `DF1ow4tspfHX9JwWJsAb9epbkA8hmpSEAtxXy1V27QBH` | 1 |
| `HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ` | 4 |
| `MNFSTqtC93rEfYHB6hF82sKdZpUDFWkViLByLd1k1Ms` | 1 |
| `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | 3 |
| `proVF4pMXVaYqmy4NjniPh4pqKNfMmsihgd4wdkCX3u` | 1 |

Transient diagnostics detected **three creation/closure lifecycles in two
transactions**, involving two distinct accounts:

- `3Qe9H5SDwKLoN6zaa9ibuGZfJvdqyBhHoMJU69nA9nQwFcu2sauG7W4r59WqAVUPVMh6n94pPpgoDAHFzZdZmPJx`
  at slot 448194560: `GGndpdVwG4g4AegawJQ87aRs4nTERouaaGgowv36AyKR`
  is created/closed twice (outer instructions 5→7 and 8→13).
- `53CcVTKZ8tq4TyxMPUggzmT4T5og2m2CdPv11rydHx9rZi8wtGWigAwpTkwRiUjq2oTjRmFpypXShMBNfRy8eckd`
  at slot 448194240: `8YdFnrdKroUgfBFZaXfu6paRg6SgmXB1LUZa2XPaAAUD`
  is created and closed through inner instructions under outer instruction 4.

These accounts have zero boundary lamports, no boundary token entries, explicit
System CreateAccount and subsequent SPL closure evidence. Neither transaction
is among the nine top-level recognized-action transactions. This is an apparent
lifecycle diagnosis from metadata, not proof of historical account bytes or
replay support. A test prevents cross-pairing two lifecycles into three cases.

## Frozen identity and hashes

Fingerprint:

```text
b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af
```

It is SHA-256 of canonical schema version, policy hash, source rows sorted by
explicit source index, fetch membership sorted by source index, and sorted raw
RPC response hashes. Directory names and wall-clock timestamps do not contribute.
Original source order is meaningful through `source_index`; file/input
enumeration order is irrelevant.

[manifest.json](examples/phase-u3-baseline/manifest.json) has 339 raw artifact
hashes (3,408,069 bytes including receipt) and four primary derived hashes.
[checksums.sha256](examples/phase-u3-baseline/checksums.sha256) hashes all 344
raw/derived files, including the final manifest. The complete chosen directory
is 9,874,707 bytes. This is transaction-audit storage, not ReplayRecord overhead.

| Artifact | SHA-256 |
|---|---|
| `source-signatures.json` | `8b24adc076b0804781823853878b87d4e9bbc56a41838bfd34c61528adc71a3b` |
| `fetch-membership.json` | `a04c9d28a9130b91da61ad985efd7838541aac60ed62e98bf95453b12359599b` |
| `capture-receipt.json` | `00daa0b08b1540433e29eeaee027bf16ea12511d62878a054ce59bb14d47ab4f` |
| `classifications.json` | `dabafe084c4e91168e022d1691037db68f52882893567d5d190b1d88e0ac40e9` |
| `supported-observations.json` | `21d72d1713640dbc0b037557cfb1ffb73b60a8ae75740bec14282d759011fe57` |
| `before-rejections.json` | `0c7bb8085801eec1ab7bb1fb4a00e18e766d64e47f5d83ba6a8dec4d0bfd7548` |
| `summary.json` | `3946f9a03abff5d41824b05959972c58b98868b3ca1579af57ce54ec8855e00a` |
| `manifest.json` | `a9f8b9e78cc49484c2002ea1d614745b49dc36bea221ca7ed9a1703752e79674` |
| `checksums.sha256` | `55eef5e2ba0e4cadbdaef6322b21b2daa6b5fdd0f404f58882a0c6cbb03c6ad9` |

## Offline reproduction and integrity checks

```bash
./scripts/rebuild-kamino-u3-baseline.sh --verify
./scripts/rebuild-kamino-u3-baseline.sh --verify --reverse
./scripts/rebuild-kamino-u3-baseline.sh --output /tmp/eplyx-u3a-derived
python3 scripts/test_kamino_u3_baseline.py
python3 scripts/mutate-kamino-u3-baseline.py --output /tmp/eplyx-u3a-mutations.json
```

The rebuild builds only a read-only example using `cargo --offline`, removes
RPC/archive/API-key variables from the classifier environment, and calls no RPC.
It validates source-to-RPC linkage, immutable source pins, every raw hash, unique
identities/artifacts, deterministic selection, resolved membership and per-row
counts before emitting `manifest.complete=true`. Checkpoints are explicitly
`complete=false` and cannot serve as capture receipts or finalized samples.
The network collector is a separate `scripts/capture-kamino-u3-sample.py`
command and refuses to overwrite an existing capture.

Repeated runs, reversed input/classifier order, reversed filesystem enumeration
and relocating the sample into a differently named directory preserve all
canonical derived bytes and fingerprint. All 344 checksums were verified.
[Determinism evidence](examples/phase-u3a-validation/determinism.json).

**30 integrity tests passed**, including an actual 200-success fixture with
one classification removed; pending fetches, missing/duplicate artifacts,
aggregate/supported membership defects, earlier overlapping source lists,
normalization/newer-version first-stage reporting, raw preservation, no-network
derivation and secret-bearing URL/non-JSON capture tests. Synthetic fixtures
are explicitly separate from production evidence.

**10/10 real source mutants killed by named assertion tests.** Each mutant runs
in an isolated source copy, is restored byte for byte, and must fail via an
assertion rather than a syntax/compile/runtime error. The production source
hash before/after is identical. [Exact mutation/test results](examples/phase-u3a-validation/mutations.json).

Artifacts were scanned for credential fields and credential-bearing URLs;
no matches were found. Tests deliberately use synthetic secret strings, which
are never serialized into production artifacts. No private RPC credentials
were inspected or committed.

## Existing product controls unchanged

[controls.json](examples/phase-u3a-validation/controls.json) records the before
and after results. Canonical outputs are retained in
[reports/](examples/phase-u3a-validation/reports/baseline.json):

| Case | Exit before / after | Canonical stdout SHA-256 |
|---|---:|---|
| verify | 0 / 0 | `d395a058064535cbeb7547943f34ad3c3a2b109e849e5ecf859c694b54acbcf0` |
| baseline | 0 / 0 | `7be26a66f3e98f9b96ba6f1270003c158d99968d9ed081c336a97f14782d2099` |
| regression | 1 / 1 | `e5e6a4b39ce377d8573099dc646fb37928bd3aa1172ceea4b4fa6c903cd52ee4` |
| bounded | 1 / 1 | `139fd95c2899d54b765511d20bc70dfc63cb7a9532d5b3b5ae62b7626df68d16` |
| stale | 3 / 3 | `8d6a4dc867c39655b58f91395630aac48d8e865b1aa1fee3a36a6ac0bcb9bc63` |
| unevaluable | 5 / 5 | `0c7a3b5231f84b99e58d6ddba6fe76e595cc212ce13a81fac24d9be283ca57d7` |

All reports are byte-identical to retained U2 reports. Token-2022 integration
controls passed **12/12**, and Kamino synthetic integration controls **10/10**,
before and after tooling. The seven U2 Kamino subjects, ReplayRecord schema
and replay/admission/normalization/semantic rules are unchanged. Both read-only
examples pass Clippy with warnings denied; Rust formatting passes.

## Limitations and the next permitted experiment

The classified subset may reflect provider availability: 44 selected fetches
failed and remain explicit failures. No action count or version distribution
is inferred for them. Reacquiring missing signatures later requires a separate
capture/version; it cannot silently enlarge this frozen denominator.

This is provider-attested transaction metadata, not independent ledger
verification, historical account bytes, LUT bank context or historically pinned
dependency ELF evidence. No replay baseline, mutation of replay behavior, new
eligible signature, AFTER count, execution timing comparison or production
ReplayRecord storage overhead exists yet. It is not an A/B/C conclusion for U3.

After this freeze commit, **future U3 may compare the old engine with a
modernized engine on these exact 156 captures and nine recognized-action
transactions**, preserving all twelve instruction identities and the ten
successful observations. It may acquire historical LUT/account/dependency
evidence for those identities, then test normalization/admission, message
identity, lifecycle execution, baseline fidelity and reconciliation. Any new
eligibility must be earned on the same frozen sample through actual baseline
proof; later blockers must be measured after the first blocker is reached.

U3 implementation and adapter #4 have not started. No standard-program
companion admission, SyncNative, CloseAccount, ATA replay, LUT acquisition,
contract v2 or hosted changes are part of U3A.
