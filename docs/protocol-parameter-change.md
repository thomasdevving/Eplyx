# Active Token-2022 fee parameter change

Eplyx's fourth schema-1 ChangeSpec is `protocol_parameter_change`. It supports exactly `token_2022_active_newer_transfer_fee_basis_points_v1`: change the already-active newer transfer-fee basis points on one observed Token-2022 mint, then run the same original-owner `TransferChecked` independently against current and derived mint state.

This is a captured current-state counterfactual. It establishes the measured result of this exact transfer under the declared rate. It does not execute `SetTransferFee`, establish fee-authority control, deployability or on-chain activation, judge configuration safety, or establish effects for other holders. Governance and composition remain outside this contract. The authenticated guided form edits only the proposed basis-point rate for an eligible retained transfer.

## Proposal schema and identity

This is the actual qualification proposal; epoch and maximum fee use canonical decimal strings to preserve u64 precision. Both expected and proposed bps are JSON integers in `0..=10000`.

```json
{
  "schema_version": 1,
  "change_spec_id": "7f24680b92d65edec79f97175be9a81cb35dbf8670581af13519c8f444da3a11",
  "change": {
    "kind": "protocol_parameter_change",
    "target": {
      "program_id": "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
      "config_account": "PreANxuXjsy2pvisWWMNB6YaJNzr7681wJJr2rHsfTh"
    },
    "operation": {
      "kind": "token_2022_active_newer_transfer_fee_basis_points_v1",
      "expected_current": {
        "account_data_sha256": "5031771a84320c80236f4453c9b7d554446acc23490a9e1abffdd141cfada219",
        "basis_points": 50,
        "schedule_epoch": "1032",
        "maximum_fee_raw": "18446744073709551615"
      },
      "proposed_basis_points": 200
    }
  }
}
```

The existing outer identity hashes `("eplyx-change-spec-v1", schema_version, change, activation)` with existing ordered serde serialization. Program, config mint, expected mint-data hash/rate/epoch/cap, operation/version and proposed rate identify the proposal. Transfer accounts, amount, Clock, runtime, file locations and display metadata do not identify it. Generic activation is rejected; unknown operations/fields, maps and byte offsets are rejected. `candidate()` remains `None`, with an explicit configuration target and bound-change variant. Existing upgrade, migration and lifecycle field order/serialization remain unchanged.

## Mutation and provenance

`engine/src/parameter_change/mod.rs::mutate` starts with exact retained bytes and the pinned official `spl-token-2022-interface = 3.1.1`. It strictly decodes the initialized mint and all extensions, refuses unknown, malformed or duplicate TLVs, and checks all four mandatory current expectations. A mismatch returns `current_state_mismatch` before execution. The captured epoch must be at least the newer schedule epoch; a pending schedule returns `schedule_not_active` without advancing Clock.

The helper clones the account snapshot and uses `StateWithExtensionsMut<Mint>::get_extension_mut<TransferFeeConfig>()` to set only `newer_transfer_fee.transfer_fee_basis_points`. It re-decodes and compares every extension and base mint field. Restoring the two bytes belonging to the actual official typed field must restore the entire original byte buffer. No caller supplies an offset. This proves preservation of older schedule, epochs, cap, authorities, supply, base state, unrelated extensions, TLV order/padding, length and account envelope. Failure prevents execution.

The report keeps four origins separate:

| Origin | Retained evidence |
| --- | --- |
| Observed finalized capture | Immutable RPC transcript, account bytes, fixture/input/capture commitments |
| Validated declaration | ChangeSpec and exact bound ID/target/operation |
| Derived proposed pre-state | Parent observed mint hash, proposed mint hash, revision, mutation descriptor and complete derived snapshot |
| Simulated post-action state | Each independent execution, logs, raw watched accounts, execution digest and reconciliation |

The derived mint is a typed parent-bound delta; no ordered handoff or composite execution is implemented.

## Execution and report

The engine reuses `capture_current`/retained transcripts, `build_current`, `execute_probe_message` and `reconcile_current`. It does not implement another transfer executor. The exact supported original-owner action boundary, deployed Program/ProgramData relationship, source/destination mint relationships, decimals, owner evidence, finalized capture context and required Clock remain enforced. Unsupported extensions are refused before comparison.

Each side uses a fresh LiteSVM. An exhaustive plan commitment comparison, after restoring only the proposed mint snapshot to its observed parent, requires equal ELF hashes, loaders/dependencies, accounts, message/instructions/metas, amount, signer assumptions, watch set, Clock and runtime. Baseline post-state never becomes proposed pre-state. Signature and recent-blockhash verification are disabled under the existing captured-path contract; the synthetic payer and assumed owner signature do not prove key possession. Runtime commitments include lockfile, executor, transfer and derivation source hashes.

`eplyx-protocol-parameter-report-v1` is a separate analytical report with no candidate ELF. It embeds the retained input for portable offline reproduction, both executions and pre/post snapshots, decoded raw public/withheld balances, mint identities and independent reconciliation. Successful transfers must satisfy source debit = recipient public credit + destination withheld increment. VM output and reconciliation are authoritative; arithmetic alone cannot create findings. Actual rejection retains rollback evidence. Reconciliation failure retains execution evidence and emits no economic findings.

Only exact reconciled differences produce existing semantic directions on `token-2022/transfer_checked/economic/recipient_tokens_received` and `token-2022/transfer_checked/economic/destination_withheld_transfer_fee`. Supported execution differences use `token-2022/transfer_checked/execution/transaction/now_reverts` or `now_succeeds`. Equal outputs, including an equal-rate control, are `no_observed_consequence` for this case. New findings use raw integers; there is no USD valuation or safety judgment.

| Factual status | Meaning |
| --- | --- |
| `unsupported_config_field` | Wrong supported program/operation or absent fee config |
| `invalid_proposed_value` | Rate outside the integer range; invalid declarations are rejected during parsing/validation |
| `config_evidence_missing` | Missing/inconsistent required capture evidence |
| `current_state_mismatch` | Required observed mint expectation/target differs; no execution |
| `schedule_not_active` | Newer record pending at captured epoch; no execution |
| `mutation_unsupported` | Strict layout or isolated preservation cannot be proved |
| `downstream_action_unsupported` | Transfer falls outside the existing supported contract |
| `execution_rejected` | Supported executions reject without a measured execution difference |
| `execution_unavailable` | VM infrastructure cannot produce the paired execution |
| `reconciliation_failed` | Retained executions cannot establish required invariants; no economic findings |
| `semantic_consequence_observed` | Exact reconciled economic/execution difference |
| `no_observed_consequence` | Successful pair with equal measured outputs for this transfer |

## Local and hosted use

Acquire an eligible finalized transfer capture through the existing current path, then declare expectations from those bytes. The practical local entry points run in the existing credential-free child:

```sh
eplyx parameter analyse --change change.json --capture transfer.capture.json \
  --out parameter-report.json --record /path/to/project
eplyx parameter reproduce --change change.json --report parameter-report.json
```

An existing exact retained transfer fixture/context can instead be supplied using `--input input.json`. Select exactly one of `--capture`/`--input`. Output must be new. `--record` persists the analytical report and validated spec in the existing `.eplyx` store. Analysis returns exit 0 after paired execution and exit 2 for a factual pre-execution failure; inspect report status for the scientific result. Reproduction checks bindings/derivation, runs both sides again without RPC and requires complete report equality. Read-only integrity verification reconstructs plans and reconciliation from retained evidence without running the VM or reacquiring state.

Hosted submission uses authenticated project access:

```text
POST /v1/projects/{project_id}/runs/{retained_current_path_run}/parameter-changes
{ "request_key": "unique-key", "change_spec": <schema above> }
```

The parent must be a completed, project-owned retained current transfer run. The server selects its authoritative immutable capture; callers cannot submit proposed account bytes, independent ELF, execution results, fee calculations or offsets. The existing durable queue and empty-environment worker validate the stored spec/capture, derive the mint themselves, execute, persist and index the result. Reads reverify parent ownership, authoritative capture, spec/index/binding equality and report projection. No provider is configured inside the worker. Restart recovery preserves the same input references.

### Guided authenticated browser flow

Open a retained hosted current-transfer run at `/p/{project_id}/runs/{run_id}`. Its **Parameter Change** section requests authenticated run-specific eligibility from:

```text
GET /v1/projects/{project_id}/runs/{run_id}/parameter-change/eligibility
```

The server verifies the parent projection/capture, original-owner Token-2022 transfer admission, deployed executable evidence and isolated no-op mutation proof at the retained Clock. It performs no RPC or VM execution. Eligible responses contain only safe facts: mint/program, current bps, schedule/captured epochs, maximum fee, mint/capture/ELF hashes and source/destination/raw amount/decimals. Raw account bytes and provider internals are omitted. Ineligible responses carry a stable `reason_code` and factual explanation, including pending schedule, missing fee config, unsupported shape, incomplete capture, wrong path/run/program or unfinished parent. Project access and parent ownership apply equally to this GET and the existing POST.

Eligible parents link to `/p/{project_id}/runs/{run_id}/parameter-change`. The form has one editable analytical field, **Proposed transfer fee (basis points)**, accepting integers `0..=10000`, including an equal-value control. Every expected-current field comes from the immutable server response. Mint, cap, epochs, source/destination and amount remain read-only. Project capabilities independently gate submission.

Preview shows current → proposed rate, retained configuration/interaction and the same-code comparison contract. Technical mode exposes the exact existing schema-1 ChangeSpec, operation and evidence identities. Overview/Technical switching changes presentation and retains the proposed value. No JavaScript fee calculation or predicted recipient/withheld output is present. The server verifies the ChangeSpec ID; the accepted result exposes it in Technical mode.

Submission sends only `{request_key, change_spec}` to the existing `/parameter-changes` POST. Editing the rate invalidates its preview; a retry of the same preview retains its request key. Validation or server rejection preserves the rate and displays the factual error. A mismatch is an evidence-binding issue; the browser never refreshes chain state or rebases expectations. Acceptance navigates to the existing run page, which polls a queued/running parameter job and recovers its durable status on refresh.

The result shows the authoritative rate declaration, unchanged program bytes, raw recipient credit and withheld increment when reconciled, and the recorded ChangeSpec/ELF identities. No-consequence means the measured result of this exact retained transfer did not change. Pending schedule and stale evidence do not imply program rejection. Reconciliation failure establishes no economic findings. Internal worker errors remain separate from analytical statuses. The scope is still one captured transfer, with no `SetTransferFee`, signing, authority, activation or governance claim.

The shared dashboard view-model reader and hosted analytical reader recognize the kind through the existing metadata/report contract. Stored validated spec ID = indexed change ID = report binding ID is enforced. The minimal dashboard view displays raw consequences, status and limitations. This complete evidence-bearing report exceeds the existing 4 MiB generic local-to-cloud sync report bound; generic sync is not the submission path for this capability. Use the authenticated hosted API above. Sync privacy scanning and limits remain unchanged; archived provider-bearing captures are private fixture inputs, not public downloadable report examples.

## Qualification and implementation coverage

The real observed qualification uses the untouched retained finalized transfer transcript captured **2026-09-18T07:39:15.727432Z**, imported through the existing reviewed lifecycle fixture importer. Its mint has newer epoch **1032**, already active at captured epoch **1037**, rate **50 bps** and cap **18446744073709551615**. The later retained current-transfer capture has a pending newer schedule and qualifies the factual rejection; neither capture was modified or its Clock advanced. This is an exact retained observed case, not a newly acquired live observation or historical coverage claim.

| Same input: 10,000 raw | Baseline 50 bps | Proposed 200 bps |
| --- | ---: | ---: |
| Source debit | 10,000 | 10,000 |
| Recipient public credit | 9,950 | 9,800 |
| Destination withheld increment | 50 | 200 |
| Execution/reconciliation | Successful/exact | Successful/exact |

The same deployed Token-2022 ELF hash is `0999dbf708971e723b08d1caafc988826a59c6001ed6dc02260da07defbe1469`. The transfer fixture commitment is `d99ec5e1c0c6aedf4d1a675ddaff9c5cbf7dab11b77c478ef045416c6185c264`. See [the compact qualification commitments](examples/protocol-parameter-change-qualification.json); full provider-bearing capture/report bytes remain private imported evidence.

Synthetic controls use actual deployed-code VM execution: cap 10 with rates 100/200 yields equal recipient credit 9,990 and fee 10; amount 1 at rates 50/200 rounds equally; equal-rate 50/50 reproduces unchanged results; rates 0 and 10,000 execute/reconcile. Deliberately wrong instruction decimals qualify actual rejection and watched-account rollback in a labelled derived test world. These controls do not alter or relabel the real observed qualification capture.

Implementation files cover schema/binding (`engine/src/change.rs`), typed mutation/report/reproduction (`engine/src/parameter_change/mod.rs`), retained current conversion and transfer reuse (`engine/src/path/current.rs`, `token_transfer.rs`), local CLI/store/readers (`cli_parameter.rs`, `main.rs`, `local_store.rs`, `dashboard/view.rs`), build schema/operation inventory (`build_info.rs`), execution deserialization (`executor.rs`), hosted API/index/queue/worker/projection (`server/src/api.rs`, `registry.rs`, `hosted/{mod,parameter,worker}.rs`) and minimal viewing (`frontend/dashboard/analytical.js`). Module registration and the server test dependency/lockfile accompany these. No execution algorithm or governance adapter is replaced.

`engine/tests/parameter_change.rs` qualifies isolated mutation, TLV order/padding, endpoints, malformed/duplicate/unknown layouts, every stale expectation, pending schedule, same-state pair, actual outputs, no-op/rounding/cap, rollback, corrupted withheld reconciliation, offline reproduction and local durable CLI/readers. Tamper tests cover observed/derived bytes, declaration, rate, ELF, Clock, accounts, executions and findings, including a resealed inconsistent result. `server/tests/hosted_parameter.rs` qualifies authorized submission, denial boundaries, current parent selection, stale/no-consequence results, empty worker, authoritative projection, durable restart/recovery and index equality. Existing transfer admission tests cover unsupported capture shapes. Existing upgrade/migration/lifecycle identity and capability tests retain compatibility. Frontend checks add consequence, no-consequence and stale views with exact large quantities and escaping.

Qualification passed **62 distinct integration tests**: 10 new engine parameter tests, four new hosted parameter tests and 48 existing regression tests covering ChangeSpec, current execution, Token-2022, migration/lifecycle identity, project capabilities and older hosted analyses. The frontend suite passed, including 70 dashboard views. Clippy passed for engine/server libraries, binaries and both new test targets with warnings denied. The broader engine library run passed 652 tests; five existing authority-resolution tests failed to read missing archived migration population fixtures. The pinned migration importer confirmed the available local archive lacks those artifacts, so full-library verification remains incomplete. The updated outer deserializer was additionally checked against all 11 ChangeSpec library tests and existing migration/lifecycle identity tests.

Exact Phase 8B commitments and verification limits are recorded with the [qualification summary](examples/protocol-parameter-change-qualification.json). Phase 8C adds only authenticated eligibility, guided rate declaration and result presentation over that closed analytical contract. Its focused coverage lives in `frontend/parameter.test.mjs`, `frontend/tests/dashboard/parameter.spec.js` and the eligibility additions to `server/tests/hosted_parameter.rs`. There is no second parameter, fee-authority change, `SetTransferFee` execution, governance extension or composition.

Phase 8C verification passed all five hosted parameter tests and both project-capability tests, the complete frontend checks (including 70 dashboard views and the new guided-form tests), and one focused Playwright flow with mocked hosted HTTP. That browser flow covers entry, bounds, exact preview, mode retention, rejected submission/retry, accepted navigation, async completion, refresh recovery and mobile layout. Server Clippy with warnings denied and repository formatting checks also passed. No implementation blocker remains for this one guided capability; the historical Phase 8B full-library fixture limitation above is unchanged.
