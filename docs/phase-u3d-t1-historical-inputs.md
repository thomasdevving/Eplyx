# Phase U3D — T1 historical input inventory and blocked continuation

**T1-D: historical state could not be proven.** Stage 0 passed. Stage 1 produced
an offline inventory but did not satisfy its historical Scope, KLend and ATA
byte requirements. The qualified archive configuration is absent from this
process. No historical-state RPC was attempted. This is a local configuration
blocker, **not a measured provider failure or evidence that history is missing**.

No dependent execution work or broadening to T2–T4 occurred. There are still
**zero validated production Kamino replays** and no production-derived Kamino
ReplayRecord. The user's staged stop rule applies before runtime integration.

## Frozen controls and worktree

Initial and final HEAD: `84ba410287a4a7ed315a7d779dbaad625fe3bf39`. The initial
worktree was clean. No commit was created. Changes are additive, uncommitted
inventory scripts, tests, this report and U3D artifacts; existing tracked files
and engine/runtime/product behavior are unchanged.

[Preimplementation freeze](examples/phase-u3d-validation/preimplementation.json)
records 415 input hashes before code changes. [Stage 0](examples/phase-u3d-validation/stage-0.json)
records the source hashes, 344 U3A checksums, U3A fingerprint
`b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af`, policy
`3a95c98305aa69e2b7f0502c6e9472f47aea863c3fdf32c12e3644a06a0180c3`, eight
reconstructed U3B.2 proofs and four admitted U3C structural envelopes.

All six Stake Pool product report hashes match U2 before and after, with exits
**0/0/1/1/3/5**. Token-2022 and Kamino synthetic controls pass. The initial test
command named nonexistent test targets; the corrected command passed. Both the
original unavailable U3B attempt and the successful U3B.2 attempt were verified;
the former is not counted as eight proofs.

## Stage reach and T1 inventory

| Stage | Result |
|---|---|
| 0 | Passed all required controls before code changes |
| 1 | Message/observed-binary inventory derived; historical byte proof incomplete |
| 2–7 | Not reached; runtime notes below are preliminary source inspection only |
| 8 | Terminal stop decision T1-D |
| 9–15 | Not reached; T2–T4, corpus, separate ATA, wrapped SOL and multi-action work gated |
| 16–19 | Terminal reporting and architecture decisions, without dependent implementation |

T1 is `5eLacZQNT4qYoCd6w9KyULUmycVFeEMjcuoS9sEZXsUifVKXi6AkGUx93zgmzEGZ2KUDzfB8Z7m8ZoAo3fvSWg97`,
slot **448195166**, deposit V2 at outer index **5**.

The [account inventory](examples/phase-u3d-inventory/inventory.json) includes every
address, original message index, static/lookup origin, writable/signer privilege,
instruction-local role and occurrence, historical owner when captured, expected
owner separately, boundary and acquisition status. Roles come from frozen
instruction interfaces and metadata, never from writable status alone.

| Inventory component | Count | Evidence boundary |
|---|---:|---|
| Original message accounts | 23 | Ten static, thirteen LUT-loaded, original order preserved |
| Observed program accounts already captured | 6 | ATA, Compute Budget, Scope, KLend, SPL Token, Token-2022 |
| ProgramData accounts outside message | 4 | Scope, KLend, SPL Token, Token-2022; raw headers and existing complete loader images |
| Historical LUT outside execution account list | 1 | Existing execution-slot proof; not protocol pre-state |
| Instructions sysvar in message | 1 | Runtime must construct it from the original loaded message |
| Message accounts lacking historical bytes | 16 | Fourteen ordinary state/authority/signer accounts plus passed System and Farms accounts |
| Historical protocol-state accounts acquired | 0 | No new acquisition requests |

Scope's exact fixed accounts are prices
`3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH`, mappings
`4zh6bmb77qX2CL7t5AJYCqa6YqFafbz3QJNeFvZjLowg`, TWAP
`6L6vUts9tYqxHVUCEFVc2mzZw6yxMn8C6a44cp5ga7e9` and Instructions.
All four remaining-account positions **4, 5, 6, 7** repeat the Scope program
address. Tokens remain **344, 279, 13, 456**. The inventory retains all aliases;
it does not invent four separate oracle accounts. Historical mapping bytes are
still needed to validate the derived-price/configuration relationships.

KLend state includes reserves `97zoywd8mPZsGTg8q1wdD2Wgkdrs2tqusp1Qqcxbyj7E`
and `UvXjBuC7YZYaGB9Rn1PpBD1GySmjzunXgE8Zev9ua8d`, obligation
`CjYJmMj61rXQUTcrZQRXVWpTfsP7zMiRPHN3K9dZz3Uz`, market
`5wJeMrUYECGq41fxRESKALVcHnNX26TAWy4W98yULsua`, market authority, liquidity and
collateral mints, supply/collateral vaults and user liquidity account. The machine
inventory gives every full address and role. Optional oracle/farm positions that
alias KLend remain aliases. Farms is passed at deposit position 16; no T1 Farms
invocation was observed, and its historical account header remains missing.

ATA `ELTUJbafWtk3AqnNDNzf4KsGS8HEdnrnwqFma8LswThY` is also the user's liquidity
source. The prior structural proof checks its PDA and metadata witness. Historical
Token-2022 account bytes, wallet authority, mint, initialized state and extensions
have **not** been checked. No local CreateIdempotent execution or no-creation
claim is made.

## Historical boundary and acquisition result

U3C's retained block response was replayed through the existing resolver/screen.
T1 is at index **63** among **712** transactions. Its screen has **26** required
accounts: all non-sysvar message accounts plus four ProgramData addresses. There
are no conflicts. The inventory requires coverage of that entire set; missing
coverage, any conflict and the wrong slot are rejected.

For ordinary accounts, an end-of-**448195165** snapshot can represent T1's
pre-state under the qualified archive's exact-bank contract and this unchanged
no-other-writer rule. End-of-**448195166** can supply post-state references under
the same rule. Acquired bytes must still pass owner/layout, lamport, token and
configuration checks. A clean screen alone proves none of those contents.
Runtime-managed bank changes need separate analysis. Sysvars must not be seeded
blindly from S−1. The LUT retains its distinct execution-slot resolution proof.

[Configuration receipt](examples/phase-u3d-validation/environment-check.json):
`SOLANA_ARCHIVE_RPC_URL` and optional `SOLANA_ARCHIVE_RPC_ORIGIN` are absent.
No private configuration files were searched, no credentials were recorded, no
current-state fallback was used, and no new raw acquisition directory was
presented as a provider attempt. Network requests: **0**. State requests: **0**.
Acquisition completeness, Scope/KLend raw-state validation and historical ATA
existence remain unproven. All original U3C success and failure evidence survives.

## Runtime model: inspected, not established

[Runtime review](examples/phase-u3d-validation/runtime-review.json) records local
source hashes and the limits of this inspection.

| Context | Current evidence and outstanding requirement |
|---|---|
| Clock | Frozen Scope and KLend refresh sources call `Clock::get`. T1 slot and transaction block time `1789764316` are known; historical Clock bytes, epoch, epoch-start timestamp and leader-schedule epoch are not acquired. Block time is not a complete Clock proof. |
| Instructions | Must reflect the original native v0 loaded accounts, every instruction and current outer index. No archived or hand-built substitute was seeded. |
| Recent blockhash | Existing historical legacy replay disables local signature/blockhash verification while keeping message bytes. No T1 replacement, native integration or network-inclusion claim. |
| Features | Existing `LiteSVM::new()` uses pinned 0.16.0's mainnet feature snapshot sourced 2026-08-24. It is not a reconstruction of the exact execution bank. |
| Rent | LiteSVM installs feature-dependent rent. Historical applicability and deployed-program requirements remain to be established. |
| SlotHashes | T1's active LUT proof needs no SlotHashes witness. Program-side requirements have not been exhaustively established. |
| Stake history / epoch schedule | No guessed values were added. Relevance and historical values remain part of context closure. |
| Loaders | Existing binary proofs identify legacy/upgradeable/native loaders. Loader relationships must survive later runtime loading. Passed System account metadata is still missing. |
| Native v0 | LiteSVM accepts `Into<VersionedTransaction>`; Eplyx's executor still takes `Option<Message>` and constructs a legacy unsigned transaction. No execution integration changed. |

Public protocol source has not been reproducibly matched to the historical ELF.
This inspection cannot establish every syscall or dynamic configuration branch
of the deployed programs. Stage 2 acceptance is explicitly **not** claimed.

## Execution, fidelity and semantic results

No T1 instruction executed locally. Scope writes were not observed locally, and
Scope → KLend causal influence was not measured. On-chain logs show refresh
activity, including a changed last-updated slot for token 456; those logs are not
a local controlled mutation experiment or raw post-state proof.

Outcome fidelity, strict raw/typed post-state fidelity, runtime-managed exclusions
and semantic evaluation were not attempted. No economic account was excluded to
obtain a match. All seven existing subjects remain unchanged; **zero** gained
production assurance. No durable production-derived Kamino record, experimental
corpus, corpus baseline self-check or production candidate differential exists.

T2–T4 binary retries and replays were not attempted. The separate ATA-first case,
System/wrapped-SOL lifecycle and multi-action execution were not broadened.
Action-local attribution remains unsupported; whole-transaction deltas were not
assigned to individual targets.

## Modern replay funnel and remaining blockers

Counts below distinguish transactions, action observations and assured subjects.

| Gate | Count |
|---|---:|
| Frozen captures | 156 |
| Transactions with recognized U2 actions, all outcomes | 9 |
| Recognized action observations, all outcomes | 12 |
| Successful recognized transactions / action observations | 8 / 10 |
| LUT-proven transactions, including failed original | 8 |
| Successful LUT-proven transactions / action observations | 7 / 9 |
| Admitted primary single-target envelopes | 4 |
| Complete observed executable binary sets | 1 |
| Complete historical state sets | 0 |
| Execution attempted | 0 |
| Baseline outcome matched | 0 |
| Post-state fidelity matched | 0 |
| Production-assured semantic subjects | 0 |

[Per-transaction blockers](examples/phase-u3d-validation/blockers.json) preserve
all nine recognized-action transaction identities. First unresolved categories
are **HistoricalState 1**, **Binary 3**, **Envelope 4**, **FailedOriginalPolicy 1**.
The HistoricalState entry is the configuration/proof gap, not a transport failure.
Envelope entries are the two multi-action cases, separate ATA case and legacy
System lifecycle; multi-action attribution is also independently unresolved.
No Runtime or Fidelity failure was measured because those stages were not run.

The other captures contain 87 top-level KLend transactions without recognized
U2 action families and 60 without top-level KLend. They are outside this action
cohort, not newly diagnosed replay failures. The 44 fetch failures and 800
unselected candidates remain separate from the 156-capture denominator.

## Validation, mutations, performance and storage

**582 tests passed:** the same 574 Rust/Python controls counted by U3C plus eight
new inventory tests. The new tests cover complete account membership, unknown
owner/state handling, Scope aliases, instruction and native-account ordering,
ATA retention, unchanged screening, failed-original policy and distinct
LUT/sysvar/state boundaries. They prove no runtime capability.

**No new source mutation campaign ran.** Seven malformed-input negative cases
pass within the inventory tests; these are not counted as killed source mutants.
[Mutation status](examples/phase-u3d-validation/mutations.json) lists all sixteen
requested mutations as unattempted at the stage gate. U3C's retained 13/13 results
are not recounted as U3D results. The mandatory controlled Scope-state mutation
test remains outstanding until execution works.

All six final product control hashes match. Shell syntax and diff checks pass.
No existing correctness bug was found and no bugfix was mixed with progression.
The offline inventory hash-verification/rederivation measurement took **1.637 s**;
it is a single wall-clock observation, not a performance benchmark. Runtime and
external-program execution overhead were not measured. New state/binary/response
acquisition storage is **zero**. [Storage accounting](examples/phase-u3d-validation/storage.json)
lists exact additive file sizes, separated from reused U3C capture storage.

Reproduce offline from the repository root:

```bash
bash scripts/rebuild-kamino-u3d-inventory.sh --verify
python3 scripts/test_kamino_u3d_inventory.py
```

The wrapper first rederives U3C sealed LUT/envelope proofs, every retained binary
and the unchanged same-slot screen using retained response maps. The inventory
reader has no network path and checks the Stage 0 frozen hashes before deriving.
It is diagnostic and never grants execution admission.

## Architecture decisions and next phase

**Adapter #4: B — modern replay still has a generic blocker.** Historical state
and bank-context closure, full native v0 integration and strict fidelity remain
unproved. The immediate configuration gap prevents measuring deeper blockers;
it does not show a Kamino-specific runtime failure or require architecture revision.

**Adapter Contract v2: STILL NEED ADAPTER #4.** Target roles, ordered execution
dependencies and evidence-to-subject mappings are visible patterns, but this
inventory adds no execution evidence for a contract redesign. Adapter #4 itself
must remain gated until generic replay requirements are closed.

Resume **T1 only** with securely configured access to the previously qualified
archive. Finish historical Scope/KLend/ATA validation, establish the historical
bank context, close acquisition and interference checks, then integrate full
native v0 execution and test outcomes, post-state and causal dependency effects.
Broaden only after **T1-A**. Claims of a reproduced production execution
environment, faithful consequences, production-assured subjects or a Kamino
production record remain prohibited by the missing evidence.

The [machine final report](examples/phase-u3d-validation/final-report.json)
records the requested stage, execution, corpus, control and decision outcomes.
