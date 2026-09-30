# Fourth ChangeSpec audit

Audit of MAIN at `603ec1dffa670d71ca77574573a337e6f14406d7`, 30 September 2026. Phase 8A recorded the recommendation below. **Implemented in Phase 8B:** exactly `protocol_parameter_change` / `token_2022_active_newer_transfer_fee_basis_points_v1`, with typed isolated mint mutation, paired same-ELF execution, offline reproduction and hosted analytical support. See [the implemented contract and qualification](protocol-parameter-change.md). The audit's illustrative schema/vectors remain design history; the linked implementation document defines the final schema and measured results. No governance or composition support was added.

## Decision

Choose **`protocol_parameter_change`**, initially allowing exactly one operation: replace **`TransferFeeConfig.newer_transfer_fee.transfer_fee_basis_points` on a Token-2022 mint whose newer schedule is already active at the captured Clock epoch**, then independently execute the existing original-owner **`TransferChecked`** path against current and proposed mint state.

Program: `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`. Account: the actual observed mint, owned by that program. Proposed production: **A, exact typed account-state mutation**, using the already pinned `spl-token-2022-interface = 3.1.1` extension API. Preserve maximum fee, both schedule epochs, older schedule, authorities, supply, all other extensions and account envelope. Measure recipient public credit and destination withheld fee for the same raw input amount.

This is an executable configuration counterfactual: “if this already-active rate were B instead of A, this exact transfer would receive X instead of Y.” It is **not** execution of `SetTransferFee`, a claim of immediate on-chain activation, or proof that an actor can make the change. That distinction is a product acceptance requirement, not a footnote. A real fee-setting instruction can produce a different scheduled state; that instruction contract is outside this first shape.

The decisive reuse is [the captured transfer path](../engine/src/path/token_transfer.rs): `capture_current`, `build_current` and `reconcile_current` already load deployed ELF, use captured Clock, construct TransferChecked, observe actual token accounts and reconcile debit = public credit + withheld fee. The existing test `deployed_token_2022_transfer_executes_and_fees_reconcile` asserts input 100, public credit 99, withheld fee 1. This audit inspected that test; it did not rerun it or execute the proposed mutation.

## 1. ChangeSpec extension requirements

The three current variants are genuinely different contracts, not three labels around one upgrade pipeline:

| Variant | Identifying proposal | Candidate handling | Evaluation timing |
|---|---|---|---|
| `program_upgrade` | Target program/optional ProgramData, executable hash + length, optional replaced executable, expected upgrade authority, delivery | Content-resolved executable; every consumer re-verifies | Generic activation is identifying; historical CI counterfactual is not an activation simulation |
| `token_migration` | Source/destination, conversion, eligibility, source disposition, funding, authorities, deadline, mechanism | Exact migration mechanism ELF | Activation and deadline become typed migration window terms |
| `lifecycle_change` | Asset, optional destination/ratio, eligibility declaration, before/after status, deadline and assertion sources | No candidate executable | Required Unix-time activation; lifecycle declarations alone are not an executable transition |

Sources: [change.rs](../engine/src/change.rs), [migration/spec.rs](../engine/src/migration/spec.rs), [lifecycle/spec.rs](../engine/src/lifecycle/spec.rs).

### Identity and validation

`ChangeSpec::id` hashes serde JSON of `("eplyx-change-spec-v1", schema_version, change, activation)`. Struct declaration order is canonical here; renaming/reordering existing fields breaks identity. Metadata label/source are outside identity. A supplied ID is recomputed, not trusted. Schema version is currently exactly 1; unknown kinds/fields fail closed. Canonical address and lowercase SHA-256 spellings matter. Validation establishes well-formedness; binding separately establishes compatibility with evidence. The frozen upgrade identity and binding-byte tests must remain unchanged.

Adding a variant requires explicit extensions to `Change`, `ChangeKind`, kind/accessor matches, `ChangeTarget`, `BoundChange`, and binding logic. A configuration account needs its own typed target view; do not present it as an upgrade or misuse `as_program_upgrade`. For the selected kind, `candidate()` returns None. `resolve(CandidateSource)` remains executable-only. Proposed account bytes are derived evidence, not a candidate ELF artifact.

### Runs, reports and persistence

One spec can have many runs with different captured worlds/interactions. Change identity names the proposal; run identity additionally binds capture, action inputs, runtime, program/dependency bytes, derivation revision and results. Preserve the consistency equation:

`indexed change ID == stored validated ChangeSpec ID == report change ID`.

Hosted integration is explicit work:

- [server/registry.rs](../server/src/registry.rs): add `IndexedChange` and `RunChange::of/kind` coverage. `verify_report(CiReport)` currently accepts only the upgrade pair; it cannot verify this analytical report unchanged.
- [server/storage.rs](../server/src/storage.rs): reuse canonical spec persistence, validation on read and project/change run index; add kind-specific binding checks where required.
- [server/hosted/mod.rs](../server/src/hosted/mod.rs) and [worker.rs](../server/src/hosted/worker.rs): durable `Input`, kind dispatch, offline execution, projection verification and replay require a new analytical branch.
- [local_store.rs](../engine/src/local_store.rs), [cloud/contract.rs](../engine/src/cloud/contract.rs), [server/analytical.rs](../server/src/analytical.rs), [dashboard/view.rs](../engine/src/dashboard/view.rs): artifact retention/sync exists, but kind allowlists and report verification must recognize the new analysis. Generic JSON storage is not semantic acceptance.
- [server/api.rs](../server/src/api.rs): executable run submission explicitly requires an upgrade; analytical submission dispatches migration/lifecycle. Add an analytical route/branch, not an invented candidate upload.

Use a versioned analytical report, rather than force `CiReport`'s baseline/candidate executable fields onto configuration state. Include `ChangeBinding`, current/proposed account hashes, identical executable commitments, capture/interaction commitments and both executions. Older records have no new fields; keep their exact serialization and legacy handling. An additive fourth schema-1 enum variant can preserve all established IDs, but **old readers still reject its kind**. Release engine/server readers before new writers; cover CLI/cloud/dashboard/frontend readers. Do not bump every existing spec to version 2 merely to add a variant. Version the new derivation/report contract and bump shared semantic schema only if existing vocabulary meaning changes.

### Activation, guided flows and governance

For this first shape require generic `activation` to be absent. It is a captured-epoch state comparison, not a future slot/time deployment. Bind the effective schedule epoch in the typed expectation; bind the exact captured Clock in the run. Reject captures where the newer schedule is pending. Do not advance Clock to manufacture an impact.

[frontend/src/analyse.js](../frontend/src/analyse.js) and [change.js](../frontend/src/change.js) are upgrade-oriented; the latter's kind table only knows upgrades. Current observation/path/candidate flows also exist through [hosted/observation.rs](../server/src/hosted/observation.rs), [proposal.rs](../server/src/hosted/proposal.rs) and the frontend current-state views. Reuse retained observations, not the upgrade ELF form. The future form needs an eligible observed mint, proposed integer bps and an existing source/destination/amount selection. Derive program, current value/hash, cap, epoch and decimals from evidence. No arbitrary raw-account JSON or byte offsets for ordinary users. No UI is part of this audit.

[governance/squads.rs](../engine/src/governance/squads.rs) and the governance binding machinery interpret the supported loader Upgrade message. Existing delivery accessors are upgrade-specific. They do not already prove a config update. A later typed `SetTransferFee` proposal can bind target, expected state, rate, retained cap, exact instruction/message and scheduling semantics through a separately tagged operation/delivery commitment. Preserve the outer spec identity machinery; do not equate that future instruction proposal with this direct active-state mutation, or reuse its ID. New optional identity fields can remain absent on old specs. No G1/G2 extension now.

## 2. Authority capability inventory

“Visible” below means decoded address; “semantic” names a specific consequence already represented. None proves private-key possession.

| Authority | Visible today | Existing behavioral understanding and boundary |
|---|---|---|
| Mint authority | Shared mint decoder | Migration `MintTo` funding checks authority and supply increase; Token-2022 adapter recognizes mint-to operations. Rotating it does not itself change ordinary owner transfers. |
| Freeze authority | Mint base decoder | Migration derivation executes real FreezeAccount and checks reachability; frozen accounts constrain migration/current transfer. Removing authority does not thaw existing frozen accounts. |
| Token-account owner | Shared account decoder | Current TransferChecked requires actual directly signing wallet-compatible owner; migration plans owner wallet/multisig paths. Owner changes affect which signer is accepted. |
| Approved delegate + allowance | Account base | Migration resolver checks active delegate and sufficient allowance; Token-2022 adapter understands Approve/Revoke. Current original-owner transfer does not use delegate. |
| Permanent delegate | Mint extension | Migration has an explicit issuer delegate path when terms allow it; original-owner current transfer leaves it unused. |
| Close / mint-close authority | Account/mint decoding | Visible; not a qualified close-action consequence path in the selected machinery. Do not claim semantic support from decoding. |
| Transfer-fee config / withheld-withdraw authority | Typed TransferFeeConfig | Visible. Fee calculation/transfer is understood; fee-admin and harvest/withdraw authorization are not an existing first-class analysis. |
| ProgramData upgrade authority | Loader typed decoding / baseline evidence | Upgrade binding checks expected authority; governance interprets loader upgrade. Rotation/revocation economics and loader authority-change replay are not an existing downstream user-action analysis. |
| SPL multisig members/threshold | Shared multisig decoder | Migration resolves threshold signer lists and under-threshold stress cases; no key possession or general governance equivalence. |
| Squads multisig/vault/config authority | Squads decoders and PDA checks | Proposal/message/status evidence and supported upgrade delivery; not arbitrary protocol admin capability execution. |
| Migration external authority / reserve authority | Migration spec/adapter/derived PDA | Exact v1 mechanism checks migration signer, custody/funding and destination mint authority. This is mechanism-specific, not a generic protocol admin engine. |
| Stake-pool SOL deposit/withdraw authority | StakePool decode | Adapter rejects gated shapes; extra authority account is outside supported DepositSol/WithdrawSol shapes. Decoding does not mean Eplyx can execute the gated action. |
| Kamino owner/market role; DLMM position authority/operator | Adapter-specific roles/state | Kamino obligation ownership and exact instruction roles; DLMM supported custody/withdrawal scope. No general reserve/admin configuration-change instruction support. |

Sources: [standard_programs/token.rs](../engine/src/standard_programs/token.rs), [token2022/details.rs](../engine/src/standard_programs/token2022/details.rs), [migration/authority.rs](../engine/src/migration/authority.rs), [derive.rs](../engine/src/migration/derive.rs), [fixture.rs](../engine/src/migration/fixture.rs), [evidence/authority.rs](../engine/src/evidence/authority.rs), [stake_pool.rs](../engine/src/protocol/stake_pool.rs), [kamino/state.rs](../engine/src/protocol/kamino/state.rs), [DLMM position.rs](../engine/src/protocol/meteora_dlmm/position.rs).

Account-role descriptors establish where a signer/account belongs. They do not by themselves establish the consequence of rotating an admin. Orca explicitly limits its account decoding claims; do not infer fee/admin semantics from a `token-authority` role. Drift's supported transaction/account interpretation similarly does not supply a general admin mutation contract.

## 3. Parameter/configuration capability inventory

Rows require existing behavioral use, not just display. “Can apply” describes feasibility of a bounded new wrapper, not an already implemented ChangeSpec.

| Parameter/config | Protocol/program | Decoded today? | Used in execution/semantics? | Can proposed value be applied locally? | Downstream supported action |
|---|---|---|---|---|---|
| Transfer fee bps, maximum and older/newer epochs | Token-2022 mint | Yes, typed complete fee records | Captured-epoch selection, ceil/cap fee and exact withheld/public reconciliation; newer-only adapter ranking is not the economic oracle | Yes via pinned official mutable extension API; fee-specific production helper still needed | Current original-owner TransferChecked |
| SOL deposit rational fee | SPL stake pool | Yes, denominator/numerator via variable-length Borsh reader | Real DepositSol execution; pool tokens received and manager fee outputs; bps boundary ranking | Feasible, but no complete production encoder or typed SetFee path in inspected MAIN | DepositSol |
| SOL withdrawal rational fee | SPL stake pool | Yes, including pending fee parsing | Real WithdrawSol, user SOL received, pool tokens debited/burned and manager fee outputs | Same encoder/instruction gap; pending-epoch handling adds scope | WithdrawSol |
| SOL referral percentage | SPL stake pool | Yes | Deposit action fee/referral accounts and execution outputs | Same complete-layout gap | DepositSol |
| Paused flag | Token-2022 Pausable mint | Yes | Current transfer/swap builders gate paused mints | Official mutable extension API already used in transfer negative tests; production comparison must distinguish unsupported precondition from executed revert | Existing transfer, admitted only when unpaused |
| Required incoming memo / CPI Guard | Token-2022 token account | Yes | Migration extension requirements and real-instruction stress derivation; current transfer rejects required memo without captured memo path | Real token instructions already used by migration derivation | Migration; bounded current transfer admission |
| Conversion numerator/denominator, fee bps | Eplyx v1 migration mechanism | Yes, typed proposal/overlay | Candidate executes exact conversion; source/destination and reserve reconciliation | Adapter generates typed mechanism config | Existing migration unit |

Important exclusions: Kamino's Reserve decoder reads custody addresses, available/borrowed liquidity, collateral supply and decimals; these support amount/position semantics but do **not** decode an editable risk-config contract. Lending-market discriminator recognition is not an LTV/borrow-limit/oracle-admin engine. DLMM retains `_parameters` and `_v_parameters` as opaque regions; exact swap fee events and bin reconciliation do not qualify changing an opaque pool fee field. Orca explicitly disclaims pricing/ticks/fees. Token interest/scaled-UI metadata is not a raw-transfer economic parameter oracle. Epoch fee display on stake pools does not establish an epoch-update action for this task. Account balances, bin shares and accrued fees are state quantities, not proposed protocol configuration. Migration terms would deepen an existing change family and introduce candidate mechanism concerns; do not select them as the fourth category.

## 4. Existing state-production machinery

| Machinery | What it really provides | Reuse for this decision |
|---|---|---|
| Candidate executable seeding | Replaces ELF in replay/executor with other initial inputs pinned | Do not invoke it; same captured ELF on both sides |
| Migration adapter overlays | Derives v1 config/PDA/funding accounts from typed terms | Pattern for explicit proposed origin; not a pool/token config encoder |
| Fixture construction | Real token instructions plus declared synthetic funding/accounts | Qualification only; cannot become observed product evidence |
| Shared token packers | Official `StateWithExtensionsMut` base packing for proposed accounts, amounts and supply | Existing dependency/API and preservation pattern; add one fee-field wrapper |
| Migration derivation | Typed mutations; actual ApproveChecked, FreezeAccount, memo/CPI guard instructions in scratch VM; retained derived parents | Provenance and instruction-derived-state pattern; no existing fee-update mutation |
| Universal/account evidence | Exact byte commitments and retained source provenance | Retain/re-verify; evidence alone does not manufacture a valid proposal |
| Captured current paths | Raw finalized transcript, exact accounts/Clock/programs, typed action plans | Primary corpus and execution foundation |
| `execute_probe_message` | Fresh LiteSVM, supplied accounts/programs/Clock/message, post-state watches | Run twice independently; supports proposed account loading now |
| Migration-order intermediate states | Retained state handoff in mechanism-specific sessions | Future handoff pattern only; no extension to migration order |

Sources: [executor.rs](../engine/src/executor.rs), [migration/world.rs](../engine/src/migration/world.rs), [migration/adapter.rs](../engine/src/migration/adapter.rs), [migration/derive.rs](../engine/src/migration/derive.rs), [standard_programs/token.rs](../engine/src/standard_programs/token.rs), [universal/evidence.rs](../engine/src/universal/evidence.rs), [path/captured.rs](../engine/src/path/captured.rs), [migration/order.rs](../engine/src/migration/order.rs).

## 5. Concrete authority candidates: A/B/C feasibility

### ProgramData upgrade authority rotation/revocation

Exact account is loader-owned ProgramData: typed header with deploy slot and optional authority, followed by ELF. [upgradeable_loader.rs](../engine/src/standard_programs/upgradeable_loader.rs) has strict decode and `encode::programdata`; loader instruction classification treats SetAuthority/SetAuthorityChecked as other instructions, not an evaluated upgrade.

**A:** typed header replacement is possible, preserving deploy slot/ELF and envelope; revocation changes the Option representation. **B:** real loader authority-change execution needs current authority privileges, and checked rotation additionally needs the new authority's required privileges. A new qualified loader-operation path would be needed. Verify only intended authority/header representation changes and unchanged executable payload. **C:** expected-authority binding can detect a mismatch, but that is evidence validation, not executing authority consequences.

The meaningful existing claim is “this authority no longer matches the upgrade precondition,” not “users now receive less” or “an attacker can upgrade.” Deposit/transfer actions with identical ELF do not normally consume the upgrade authority. Testing a future loader upgrade would add a new downstream action contract. Reject as the first category.

### Token-account approved delegate revocation

Exact account is SPL/Token-2022 token-account base: delegate COption and delegated amount. **A:** official base packing could clear both consistently while preserving extensions; changing only a public key is insufficient. **B:** real Revoke is understood by the Token-2022 adapter and the pinned instruction API; execute as owner, retain account post-state, verify delegate=None, allowance=0 and unchanged unrelated state. Migration derivation already supplies a scratch-instruction pattern, but its Mutation enum currently has Approve, not Revoke. **C:** migration authority resolver alone can report loss of a delegate path, without proving instruction execution.

Existing downstream option is a fixed migration unit with a previously sufficient delegate and terms allowing delegate authorization. Hold its exact migration candidate/terms/reserve constant; executing the same delegate-authorized unit can then reject. If owner or permanent-delegate fallback is allowed and replanning picks it, “migration impossible” is false. Current owner-only TransferChecked will usually be unaffected. Signatures remain local assumptions. This is feasible but couples a fourth kind to another proposed change/mechanism and its authorization policy; it has more evidence and claim complexity than the selected direct economic transfer.

### Freeze authority removal (cross-check)

Mint base freeze-authority COption is packable (**A**), and typed SetAuthority is available through the pinned interface (**B**). Verify mint authority field change and unchanged supply/extensions; current freeze authority must be assumed to sign. Existing migration Freeze derivation supplies a concrete future capability check. Ordinary transfers do not become unfrozen merely because authority was removed. **C** can say future freeze derivation is unreachable, not that frozen users can transact. No strong immediate economic consequence for the existing owner transfer; not selected.

## 6. Concrete parameter candidates: A/B/C feasibility

### Token-2022 already-active fee basis-point rate — selected

Current source is exact mint bytes and Clock from the final transfer capture batch. Proposed value is an integer 0–10,000; only the active newer record's bps changes. **A** is precise through `StateWithExtensionsMut<Mint>::get_extension_mut<TransferFeeConfig>()`, with strict pre/post checks below. The dependency is already pinned and mutable extension usage already appears in transfer tests. A production fee-specific helper does not yet exist.

**B** is available as a future construction model through the official `extension::transfer_fee::instruction::set_transfer_fee` builder (mint, config authority, signer set, bps, maximum fee), but MAIN has no qualified fee-update execution path. Executing it must retain and inspect actual resulting schedules and activation timing. It must not be substituted for immediate active-record replacement. **C**, `calculate_fee` alone, supplies an independent oracle but not executable evidence.

Affected existing action: raw TransferChecked from an actual wallet-compatible owner account to a different actual account of the same mint. Outputs: raw input debit, recipient public credit, destination withheld-fee increment, success/error and rollback. Cap or rounding can make a different rate produce identical output. That is a valid “no observed consequence” for this case.

### SPL stake-pool SOL deposit fee — runner-up, not recommended in parallel

Current source: decoded `StakePool.sol_deposit_fee` on exact pool account. Proposed representation would be exact canonical numerator/denominator, not lossy displayed bps. Existing DepositSol action measures `pool_tokens_received`, minted supply and `manager_fee_pool_tokens`; this is economically useful with identical ELF.

**A:** decoding exists but the reader skips fields and parses Options/FutureEpoch values of variable encoded length. MAIN does not supply a full production round-trip pool serializer or an isolated mutation helper. Rebuilding from this partial view could silently overwrite unknown fields. **B:** a typed actual fee-setting instruction plus manager authority would avoid a full rewrite, but neither fee-admin execution qualification nor complete state-change verification is present in the inspected adapter. **C:** calculating displayed bps or a predicted fee is not execution. Real DepositSol is already supported, so this is not a new action/protocol problem; its greater new state-production contract makes it weaker than Token-2022 for the first implementation.

SOL withdrawal fee is another concrete supported economic output candidate, but pending fee scheduling and the same serialization gap increase scope. No Kamino or DLMM generic-config inference is required to make the decision.

## 7. Authority versus parameter comparison

| Criterion | Best authority case: delegate revocation | Selected parameter case: active Token-2022 fee bps |
|---|---|---|
| New semantic code | Small mutation; larger authorization/path comparison and migration coupling | Small typed fee mutation and paired existing reconciler |
| Existing typed state | Token-account delegate and allowance | Mint TransferFeeConfig including epochs/cap |
| Exact proposed production | Real Revoke + retained post-state possible | Official mutable extension, one field, strict preservation |
| Existing downstream action | Delegate-authorized migration with fixed mechanism/terms | Current original-owner TransferChecked |
| Economic usefulness | Primarily authorization availability; possible loss of a migration path | Recipient credit and withheld fee directly observed |
| Generalizability | Other authority paths need specific fallback semantics | Establishes state-only execution pattern; each later parameter still needs qualification |
| Evidence complexity | Owner/delegate authority, migration terms/program/reserve, alternative paths | Mint/source/destination/owner/Clock and token ELF |
| False-claim risk | Loss of one delegate may be mislabeled loss of all access | Timing/authorization may be overstated; explicitly bounded active-state counterfactual |
| Infrastructure fit | New enum/index/binding/analytical kind; existing migration result assumptions | Same extension work; naturally fits captured-path analytical storage |

Choose the parameter family only. The account-state nature, rather than an imagined generic configuration engine, is the capability being added.

## 8. Evidence and provenance contract

First product: **captured current-state analysis only**, using existing current transfer cases. No historical representativeness claim. Historical adapter replay is available in principle but would add PRE-state proof and corpus qualification requirements; do not add it now. Fixtures qualify the engine but cannot alone support the product claim.

Require retained, re-verifiable finalized RPC transcript with genesis hash, final batch slot, complete mint/source/destination/owner/Clock account bytes and envelopes, program header/canonical loader linkage, deployed ELF and all required dependency binaries. Verify Clock coherence, capture revision, program ownership, mint decimals, account identity, source balance and existing transfer boundary. Record the LiteSVM/runtime revision and feature assumptions.

Keep four classes separate:

1. **Observed current:** immutable capture bytes and source record/pointer/slot commitments. Current acquisition is a finalized capture, not proof of a historical validator bank.
2. **Proposed declaration:** validated ChangeSpec field/value/expectations; it is not chain evidence.
3. **Derived proposed pre-state:** new mint bytes, parent current-account hash, derivation revision, typed mutation and resulting hash. Never rewrite the capture transcript to make these bytes appear observed.
4. **Simulated post-action:** each side's execution and watched post-account bytes, bound to its respective pre-state. These are neither observed chain state nor a fee-admin transaction result.

Synthetic fee payer remains declared; no actual source/destination balance or token account is fabricated. Signature/blockhash verification are disabled locally; owner signing compatibility is checked, while possession/authorization remain unknown. A direct config mutation makes no config-authority signer assumption and proves no ability to submit a fee change.

## 9. Proposed identity

Illustrative schema, to be frozen in tests before implementation:

```json
{
  "schema_version": 1,
  "change": {
    "kind": "protocol_parameter_change",
    "target": {
      "program_id": "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
      "config_account": "<actual mint>"
    },
    "operation": {
      "kind": "token_2022_active_newer_transfer_fee_basis_points_v1",
      "expected_current": {
        "account_data_sha256": "<64 lowercase hex>",
        "basis_points": 100,
        "schedule_epoch": "123",
        "maximum_fee_raw": "10000"
      },
      "proposed_basis_points": 200
    }
  }
}
```

Required expected current data hash and typed bps/epoch/cap prevent “A→B” evaluation against C. Reject inconsistent redundant expectations. This first proposal intentionally targets one captured account state; any unrelated mint-data change also requires refreshed expectations/new spec ID. Account owner/executable validity and full envelope/Clock/capture commitments belong to run binding. Program artifact hash also belongs to the run; both sides must share it. The config account is the mint: no separate invented protocol config PDA.

Outer v1 identity domain, ordered serialization, no arbitrary field strings/maps/offsets, canonical addresses/hashes/u64 decimal strings, metadata excluded. Changing target, expected hash/value/epoch/cap, operation or proposed bps changes ID. Source/destination/amount select the run case, not the proposal. Required absence of generic activation keeps timing unambiguous. Typed operation version identifies active-newer mutation semantics; implementation binary/capture details remain in run identity.

Stale current state is an **evidence mismatch before execution**, analogous to upgrade binding's `replaces`/expected-authority mismatch, not a syntactically invalid spec or an economic finding. Keep the well-formed spec and factual mismatch result; do not silently rebase its expectations. Malformed address/hash/range remains invalid input.

## 10. Mutation safety and execution/comparison

1. Validate immutable observed capture and build the existing baseline transfer plan. Strictly decode mint and enumerate extensions using shared checked decoders and official pinned interface. Require exactly one well-formed fee config, initialized mint and `captured_epoch >= newer.epoch`. Fail closed on unknown/malformed/duplicate extensions, incompatible layout or unsupported transfer extensions. Do not broaden the action boundary.
2. Bind expected mint hash, bps, cap and epoch. Decode the same bytes through the official mutable interface, clone the original byte buffer, set only `newer_transfer_fee.transfer_fee_basis_points`. No TLV addition/reallocation or caller-selected offset.
3. Re-decode the entire proposed mint. Verify every base/extension semantic field except this bps is identical; preserve every byte outside the officially identified bps field, including TLV headers/order, padding and account envelope. Obtain the field location through the pinned typed layout/validated extension slice, not a user-supplied offset. Unknown regions cannot be reserialized from a partial view. Store parent/proposed hashes and mutation descriptor. If these checks cannot establish preservation, stop as mutation unsupported.
4. Create proposed plan by cloning the verified baseline plan and replacing only that mint snapshot. Retain captured evidence as the parent and add explicit derived provenance. Do not pass a fabricated RPC transcript back to `build_current` as observed evidence. Run baseline and proposed via two fresh `execute_probe_message` calls, with identical program byte hashes/loaders, dependency set, Clock, message bytes, signers, watch set, other accounts and runtime. No sequential holder depletion or Step 7 ordering.
5. Use `reconcile_current` on each side with that side's actual pre-state. Its unchanged-mint check compares simulated post-mint to the respective current/proposed pre-mint. The fee oracle selects the schedule at captured epoch; the adapter's newer-only ranking helper must not replace it. Verify atomic watched rollback on failed execution and exact conservation on successful execution.
6. Compare success/errors, source debit, destination public credit and withheld increment. Require both successful sides to reconcile before emitting economic direction. Include raw units and decimals, never scaled UI value or USD valuation. Failed execution is factual execution outcome; a reconciliation failure is an evidence/invariant problem, not a supported economic delta.

Illustrative qualification vector: input 10,000 raw units, cap at least 200, already-active bps 100 versus 200 gives recipient credit 9,900 versus 9,800 and withheld increment 100 versus 200, subject to actual ELF execution/reconciliation. It is a proposed test, not an audited live result. A cap-bound input or tiny rounded transfer may show no consequence. Freeze/pause/hook/confidential/memo restrictions remain the existing supported action boundary.

Findings reuse [semantics.rs](../engine/src/semantics.rs) domains/directions: `economic` + `increased/decreased`, `execution/transaction/now_reverts` or `now_succeeds` when both sides are admissible/executed. Existing token semantic account fields include `amount` and `withheld_transfer_fee`. For this paired action report, introduce the smallest explicit action-output subject if needed: `token-2022/transfer_checked/economic/recipient_tokens_received/decreased`; fee subject can be `destination_withheld_transfer_fee/increased`. Both derive from already reconciled raw quantities. Do not compare closing balances alone without accounting for each opening state, and do not call a larger configured bps itself economic harm. No `dangerous_parameter_change` finding or new domain is needed.

The proposed mint snapshot is a reusable account delta with a parent commitment. A later ordered engine can apply that exact delta after checking the then-current expectation; the state model permits handoff. No composition or migration-order extension now.

## 11. Failure vocabulary

Proposed names are a small analytical taxonomy, not existing implemented error constants.

| Result | Meaning / handling |
|---|---|
| `unsupported_config_field` | Operation other than the single qualified typed fee field; refuse before execution |
| `invalid_proposed_value` | Noncanonical/noninteger or bps outside 0–10,000; invalid input |
| `config_evidence_missing` | Missing mint bytes, Clock, loader/ELF, source proof or incomplete account data; cannot evaluate |
| `current_state_mismatch` | Valid expectations disagree with observed hash/value/epoch/cap; evidence mismatch, no rebase |
| `schedule_not_active` | Captured epoch precedes newer epoch; outside first shape, no Clock advancement |
| `mutation_unsupported` | Unknown layout/extension, duplicate/malformed TLV, inability to prove isolated preservation; no proposal execution |
| `downstream_action_unsupported` | Unsupported signer/account/extension/message shape; no claim of program rejection |
| `execution_rejected` | Admissible action actually fails in VM; retain side, error, logs, state and rollback |
| `execution_unavailable` | ELF loading/runtime infrastructure fails; distinguish from program rejection |
| `reconciliation_failed` | Execution cannot support the exact quantity/conservation claim; no economic finding |
| `semantic_consequence_observed` | Paired verified execution shows typed economic/execution difference |
| `no_observed_consequence` | Complete supported comparison has no measured difference for the selected case; not universal safety |

Equal proposed/current bps may be accepted as a no-op control; changed spec identity still binds the declared value. A no-op must not be confused with an unexecuted/unsupported case.

## 12. Hosted/productization recommendation

Choose **B: engine + hosted analytical support in the same next implementation**, with engine contract/qualification completed first inside that scope. Durable captured current paths and credential-free workers already exist; there is no need to repeat a long local-only sequencing project. This is not free: add explicit typed input/report/projection bindings, kind dispatch, index compatibility, artifact size limits, replay and project ownership checks. Reuse [hosted/current path processing](../server/src/hosted/process.rs), observed-artifact references and run projections. Acquisition is read-only and precedes acceptance; offline worker re-verifies all evidence and constructs proposed state itself. Do not accept claimed execution results or caller-provided arbitrary proposed account bytes.

Server-owned eligible capture references plus proposed bps and existing interaction selection suffice for guided input later. A new capture may be required because an observation alone does not necessarily retain every transfer dependency/recipient. Reject incompatible retained capture rather than fabricate missing state. Deliver an API/CLI-backed analytical report; a guided form is not required for this next engine/hosted implementation.

## 13. Exactly one next implementation

**Implement `protocol_parameter_change / token_2022_active_newer_transfer_fee_basis_points_v1` with the mutation and paired captured TransferChecked contract above.** No second field or authority operation.

Components to change, all proposed:

- `engine/src/change.rs`: variant, target, validation, identity, binding and frozen serialization tests; new small typed spec/evaluator module (for example `engine/src/parameter_change/`).
- `engine/src/standard_programs/token.rs` / `token2022.rs`: isolated official-interface fee mutation helper with strict preservation and active-schedule checks. Reuse checked decoders; avoid a second TLV parser.
- `engine/src/path/token_transfer.rs` and `executor.rs`: reuse plan/reconciliation/fresh execution APIs; add only a provenance-safe proposed-plan seam if required, not a new transfer action or runtime.
- `semantics.rs`/report integration: reuse domains/directions, add narrowly typed output projection; version new report/derivation. Keep executable-only CI reporting intact.
- `local_store.rs`, CLI/local analysis dispatch, dashboard verification and `cloud/contract.rs`: retention, offline replay and kind support.
- `server/src/registry.rs`, `storage.rs`, `hosted/{mod,worker,process}.rs`, analytical/API/projection verification: new candidate-free analytical kind, project-owned input artifacts and exact spec/report/index consistency.
- Future frontend changes are identified above; no guided UI, governance or composition implementation in this next bounded contract.

### Tests and acceptance criteria

1. Existing three kinds and frozen upgrade spec/binding bytes/IDs remain unchanged. New spec round-trips; every semantic field changes ID; labels and paths do not. Unknown operation/fields and malformed addresses/hashes/numbers are refused.
2. Mutation tests construct official TransferFeeConfig layouts and cover active/pending epoch boundaries, multiple TLV ordering/padding arrangements, bps endpoints, stale hash/value/cap/epoch, missing/duplicate/malformed/unknown extensions. Prove only intended bps bytes change and all account envelopes/other accounts/program artifacts stay identical. No arbitrary offsets accepted.
3. Real deployed-ELF qualification uses an existing retained transfer capture with an active newer schedule; assert original baseline result unchanged, proposed fee/credit changed where cap/rounding permits, exact two-sided conservation and mint unchanged during each transfer. Synthetic vectors supplement this; they cannot replace the retained observed case.
4. Cover cap-bound/rounded/no-op no-consequence controls, failure rollback and deliberate withholding corruption causing reconciliation failure. If the available retained capture is not eligible, explicitly report that and obtain a new eligible read-only capture; never modify observation evidence to pass.
5. Repeat offline paired replay deterministically. Bind identical ELF/loader/dependencies/Clock/message/runtime and complete account evidence. A program hash mismatch or tampered capture/derived/report bytes must reject, not compare.
6. Persist/retrieve/index a hosted run, re-verify its stored ChangeSpec and paired report; independently reproduce from retained inputs without RPC. Test cross-project artifact refusal, mismatched input/capture/spec/projection commitments and old-record compatibility. No dummy candidate ELF hash.
7. Report observed, declared, derived pre-state and simulated post-action origins separately; state exact mint, active rate/cap/epoch, source/destination/input and local signer/runtime assumptions. Unsupported admission must never appear as an executed revert.
8. The report can demonstrate meaningful credit/withheld delta **with identical program bytes**, and labels it an active-state counterfactual. It makes no claim of immediate SetTransferFee activation, deployability, key possession, all-holder impact, historical representativeness or global safety.

### Non-goals and stop condition

No implementation in this audit and no commit. No generic Solana config engine, arbitrary byte patching, second parameter, authority-change variant, new adapter/action, real fee-admin execution, epoch/time simulation, Squads G1/G2 changes, guided UI, market valuation, historical corpus expansion, sequencing/composition or further migration-order analysis.

The audit answers the stop question: **one Token-2022 active fee-rate field; proposed mint state produced by a pinned typed isolated mutation; existing captured TransferChecked demonstrates recipient-credit and withheld-fee consequences while the ELF stays identical.**
