# Phase U3F — runtime minimality and first native v0 execution

**T1 and T2 reproduce real historical Kamino transactions with complete watched-account fidelity.** Their original native v0 envelopes run with historical deployed binaries and historical pre-state. Both match the original outcome, fee, logs, CPI instructions and raw watched post-state. The existing U2 evaluator then measures three deposit subjects and four borrow subjects, with no baseline self-findings.

These are **two experimental production-derived replay records**, not ordinary product `ReplayRecord` admission. The existing production adapter and hosted behavior remain unchanged. T3 stops at a bounded historical Rent fetch failure; T4 has genuine same-slot interference. No result for either is inferred from T1/T2.

The [two-record corpus](examples/phase-u3f-record/corpus.json), [41-part measured report](examples/phase-u3f-validation/final-report.json) and [causal stage table](examples/phase-u3f-validation/stage-table.json) distinguish acquisition, execution, fidelity and product admission.

## Frozen evidence and independent outcomes

Initial HEAD was `84ba410287a4a7ed315a7d779dbaad625fe3bf39`. Earlier additive U3D/U3E work was already present. This phase adds separate files; it does not rewrite their captures, failed requests or reports. [Preimplementation hashes](examples/phase-u3f-validation/preimplementation.json) preserve U3E. All 344 U3A checksums, eight U3B.2 LUT proofs and four U3C envelope identities remain unchanged. The sample fingerprint remains `b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af`, and the sampling-policy hash remains `3a95c98305aa69e2b7f0502c6e9472f47aea863c3fdf32c12e3644a06a0180c3`.

| Target | Slot / target index | Binaries | Paired ordinary state | Native baseline and fidelity | Current result |
|---|---|---|---|---|---|
| T1 `5eLacZQNT…` | 448195166 / 5 | complete | proven | matched | T1-A, experimental deposit record |
| T2 `55EdbRGcp…` | 448194517 / 7 | complete | proven independently | matched | T2-A, experimental borrow record |
| T3 `5NKNdj527…` | 448194462 / 5 | complete | proven independently | not attempted | Rent at S: four rate-limit responses; EpochSchedule not attempted |
| T4 `49zi8hsUA…` | 448194435 / 5 | complete | not attempted | not attempted | two earlier same-slot writes invalidate S−1 as transaction pre-state |

Thus N=4 classified, M=4 envelope-admitted, binary capture=4, paired ordinary-state proof=3, demonstrated sufficient runtime inputs=2, J=2 baselines attempted, P=2 fidelity matches, experimental records=2, ordinary product eligibility=0.

## What SlotHashes does and what was proved

Pinned LiteSVM 0.16.0 obtains a SlotHashes cache object before resolving a v0 lookup. Its [account loader](examples/phase-u3f-interface/runtime/litesvm-0.16.0/src/accounts_db.rs) therefore requires the object. In the pinned [lookup-table implementation](examples/phase-u3f-interface/runtime/solana-address-lookup-table-interface-3.1.0/src/state.rs), `deactivation_slot == u64::MAX` returns `Activated` before consulting SlotHashes membership. Both admitted experiments require this active-table condition. Original historical table bytes, extension metadata, lookup indexes and the complete resolved account vector are independently checked.

For each of T1 and T2, three real executions used:

1. LiteSVM's default SlotHashes;
2. an empty valid SlotHashes;
3. a valid synthetic list with different hashes at S−1 and S−7.

All three produced identical canonical execution evidence within each target: native message, account vector, outcome, fee, compute units, logs, full CPI data and raw watched post-state. T1's execution-evidence SHA-256 is `83f479faf1fe0f809c7c9c5c1db16047bb6c94d4cf3e1d549fbb3b19bc7f7990`; T2's is `00dd0b8a5b1c2a3bacc43f1660c323ad206de4ce0c16dc5787f51a9f71c3e4cb`.

This establishes non-materiality of SlotHashes contents **for the two exact active-LUT replay shapes and the tested valid profiles**, supported by the active-table code path. It does not reconstruct historical SlotHashes or prove arbitrary programs cannot read that sysvar. The old historical null response is still preserved as a null response. No synthetic value is labeled historical.

Clock, Rent and EpochSchedule use exact historical account bytes. Instructions is generated from the sanitized original native message by LiteSVM's [instruction-account constructor](examples/phase-u3f-interface/runtime/litesvm-0.16.0/src/utils/mod.rs). It is not an independently downloaded post-state account. Source revisions, hashes and licenses are in the [interface manifest](examples/phase-u3f-interface/manifest.json).

The pinned LiteSVM mainnet feature/default native-program profile is an implementation context, not a reconstruction of every historical validator-bank field. Signature verification and recent-blockhash checking are disabled for historical execution; the original signatures and message remain unchanged. Exact historical outcomes and states substantiate these two replays, not universal bank equivalence.

## Actual execution and strict reconciliation

The additive [native-v0 runner](../engine/examples/execute_envelope_v0.rs) reconstructs the sealed LUT proof, recomputes envelope admission and submits the complete original `VersionedTransaction::V0`. It checks historical seed contexts, original validator balances, raw-response provenance, loader-image hashes and deployment metadata. ProgramData is seeded before executable headers so the VM loads the acquired historical images. It preserves loader padding, authority and original account metadata. No legacy flattening, dependency stripping or post-Scope seed occurs in a baseline.

T1 runs all eight original instructions; T2 runs all ten. Scope, ATA, reserve refreshes, obligation refresh, target and both Compute Budget instructions retain their original positions. Scope and ATA gain no semantic ActionIds. T2's observed Farms CPI executes through its independently acquired historical binary.

T1 produces the exact Scope price-byte change at offset 25,592, from 48 to 51. Its TWAP account is unchanged as observed historically. Both genuine ATA CreateIdempotent instructions follow the existing-token path. All 22 T1 watched accounts match, and T2 independently matches its entire watched set. The only message account without an ordinary archive post-reference is the runtime-generated Instructions sysvar. No additional state ignore-list or byte tolerance was introduced.

The [strict comparator](../scripts/kamino_u3f_fidelity.py) checks success/error, fee, exact logs, CPI outer indexes, program/account indexes, instruction bytes and stack heights. It checks every watched account's presence, owner, lamports, executable flag, rent epoch and complete raw data. Validator post-balances independently constrain references. Readonly executable post-references derive from captured pre-state plus original readonly privileges and a clean complete same-slot screen. Compute-unit equality is recorded separately and also matches.

An initial reconciliation rejected absent RPC `returnData` versus LiteSVM's empty payload with the last Compute Budget program ID. The retained [Agave source](examples/phase-u3f-interface/agave/transaction_processor.rs) explicitly omits return-data objects with empty payloads. The comparer now uses that representation rule while retaining the local program ID diagnostically. Nonempty unexpected return data still fails. The [initial failed reconciliation](examples/phase-u3f-validation/initial-reconciliation.json) remains visible; no account tolerance was added.

## Scope causal controls

Each separate control starts with the same historical pre-state and binaries, removes only Scope outer instruction zero, and preserves every later compiled instruction's bytes and accounts. It is explicitly nonhistorical and cannot pass the baseline gate. No intermediate Scope post-state is supplied to either control.

| Target | Control outcome | Changed final accounts versus real baseline | Supported conclusion |
|---|---|---|---|
| T1 | success | Scope prices only | This specific refresh produced no observable change to later KLend/token/vault state. Scope still matters to complete transaction fidelity. |
| T2 | success | Scope prices, reserve `d4A2prbA…`, obligation `Adnj8BDH…` | The Scope refresh materially affects later protocol state for this exact target. |

[T1 comparison](examples/phase-u3f-causal-attempt-1/comparison.json) and [T2 comparison](examples/phase-u3f-T2-causal-attempt-1/comparison.json) bind the actual outputs. These are account-state effects, not claims that the token transfer amount changed. There are no instruction-boundary snapshots; the actual VM instruction sequence produces intermediate state, and the final-state difference establishes the controlled effect.

## Existing semantics and experimental records

Only after complete fidelity, the unchanged U2 evaluator reports:

| Target | Existing economic subjects | Measured values |
|---|---|---|
| T1 deposit | liquidity_deposited; reserve_liquidity_received; obligation_collateral_deposited | 46,523,144 base units; 46,523,144 base units; 46,472,114 collateral units |
| T2 borrow | liquidity_borrowed; reserve_liquidity_drawn; origination_fee; debt_increased | 50,002,000,000; 50,002,000,000; 0; 50,007,275,238 base units, all six decimals |

The debt observation includes the existing evaluator's transaction-boundary debt delta, including refresh effects; it is not asserted to equal transferred principal. Exact scaled debt evidence remains ancillary text, not a new subject. Both baseline-versus-self evaluations emit zero findings.

Two existing boundaries surfaced during implementation. The legacy Kamino boundary helper assumes the SPL Token owner and rejects the real Token-2022 accounts in both transactions. Its rejection is retained, and ordinary admission is unchanged. The experimental path instead uses separately validated historical owners, typed Token-2022 layouts and full raw-state reconciliation. Also, directly serializing the evaluator's internally tagged scalar `FieldValue::Text` failed in the initial experimental borrow wrapper. The wrapper now uses the existing text rendering and an explicit experimental representation, preserving the value without changing product JSON or semantics.

[T1 record](examples/phase-u3f-record/record.json) and [T2 record](examples/phase-u3f-record/T2.json) use `eplyx.experimental.native-v0-envelope.v1`. They bind raw captures, message proof, seed provenance, executable evidence, runtime sources, code, fidelity, canonical pre/post hashes and existing subjects. Offline replay rebuilds inputs from those sources and executes again; it does not trust stored success booleans.

The existing `ReplayRecord` already carries named accounts, a Clock, dependency manifests, acquisition provenance, screening and original outcome digests. Those mechanisms should be reused. The smallest future versioned extension is an execution-message variant retaining native v0 plus its LUT proof, an ordered envelope/target identity, and a provenance-bound runtime context for the additional sysvars and implementation profile. Dependency binaries can use the existing manifest instead of a second dependency model. Strict post-account digests must additionally bind executable/rent-epoch/lifecycle facts, and validator evidence should retain the full CPI data used here. Loading such an extension must reconstruct its proof; serialized admissibility flags cannot authorize execution. U3F documents this design boundary and does not change schema version 1 or its legacy admission.

The corpus contains one deposit and one borrow, two distinct target reserves, two obligations and two ordered envelope shapes. It is validation evidence, with no claim of traffic representativeness. No honest reproducibly historical-compatible controlled KLend candidate was established, so candidate differential sensitivity is not claimed. Wrapped-SOL lifecycle, multi-action replay and attribution remain unsupported and were not started.

## T2–T4 recovery and measured stops

The three exact previously missing contexts were fetched first; all succeeded on the first attempt. A separate continuation then made **131 successful new requests**, reusing only successful retained responses with the exact same method, account, slot and slice. Existing failed attempts remain unchanged. It completed all three binary sets and reran the complete same-slot screen, including acquired ProgramData keys.

T2 and T3 had clean screens. The subsequent state/runtime capture made **81 attempts: 72 successful responses and nine rate-limit responses**, including retries that later succeeded and T3's terminal four-attempt Rent failure. Every request, safe transport diagnostic and raw body is retained. There is no provider/slot fallback or hidden retry reset. T3's 16 paired ordinary accounts, typed layouts, Scope relationships and complete binary/screen proof are retained separately; Rent and EpochSchedule are not silently borrowed from T1/T2.

T4's target is transaction index 946. The [screen](examples/phase-u3f-cohort-binaries/T4/result.json) identifies earlier writes to `3t4JZcue…` at index 596 and USDC mint `EPjFWdd5…` at index 560. Under unchanged screening rules, end-of-S−1 is therefore not a sufficient target pre-state. No T4 ordinary-state acquisition or runtime attempt followed. Recovering this case requires transaction-boundary history or execution of the necessary preceding transactions with its own evidence.

All archive claims remain conditional on the previously qualified provider boundary. Scheme/host matching alone is not a new qualification. No private credentials were read or used.

## Reproduction, tests and decision

```bash
python3 scripts/replay-kamino-u3f-corpus.py --output /tmp/eplyx-u3f-offline.json
python3 scripts/test_kamino_u3f.py
python3 scripts/test_kamino_u3f_cohort.py
python3 scripts/mutate-kamino-u3f.py --output /tmp/eplyx-u3f-mutants.json
cargo test --offline -p eplyx-engine
```

Offline replay strips provider configuration, forbids live transport, rebuilds with `cargo --offline`, revalidates captured binary reconstruction/screens and reruns the three variants in reverse order. Actual VM pre-state is compared to the historical seed manifest before accepting a result. Large T2 execution responses are losslessly gzip-compressed; [logical and physical hashes](examples/phase-u3f-validation/lossless-storage.json) preserve every original JSON byte.

**603 Rust and 136 Python tests pass: 739 total, including 22 new Python tests.** Twelve source mutants covering requested runtime controls 17–28 are killed by named behavioral assertions. Wrong ELF and Clock faults fail construction; executed faults fail exact fidelity or actual pre-state checks. The Instructions mutant substitutes an incorrect sysvar account reference in the submitted message; it is not a fabricated historical sysvar dump. No compiler-only failure counts. Mutants use a separate temporary example and leave production/example sources intact.

The six product reports and exits **0/0/1/1/3/5** remain byte-identical to U2. Normal/reversed U3A rebuilds and prior U3E offline reconstruction pass. Clippy with warnings denied, formatting, diff and secret scans pass. Acquisition, runtime preparation, VM execution, comparison, semantics and storage are measured separately in the final report. Full seed audits repeat binary bytes; compression reduces this experimental audit cost without claiming it is production ReplayRecord overhead.

**Adapter #4 and a declarative Contract v2 should still wait.** The important advance is two faithful real historical replays and a demonstrated Scope state effect. The next bounded engineering step is to integrate the proven native-v0 input/execution contract into the generic product path and resolve its mixed SPL Token/Token-2022 boundary validation, with the same fail-closed fidelity gate. T3 needs a separately versioned bounded acquisition attempt; T4 needs a stronger transaction-boundary strategy. Neither gap is evidence that the two successful replays generalize to arbitrary Kamino transactions.
