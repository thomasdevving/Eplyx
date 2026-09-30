# Protocol parameter changes

Eplyx's fourth schema-1 ChangeSpec is `protocol_parameter_change`. It supports two typed operations. The original `token_2022_active_newer_transfer_fee_basis_points_v1` operation lets you change the already-active newer transfer-fee basis points on one observed Token-2022 mint, then run the same original-owner `TransferChecked` independently against current and derived mint state.

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

Exact Phase 8B commitments and verification limits are recorded with the [qualification summary](examples/protocol-parameter-change-qualification.json). Phase 8C adds only authenticated eligibility, guided rate declaration and result presentation over that closed analytical contract. Its focused coverage lives in `frontend/parameter.test.mjs`, `frontend/tests/dashboard/parameter.spec.js` and the eligibility additions to `server/tests/hosted_parameter.rs`. Phase 8C adds no fee-authority change, `SetTransferFee` execution, governance extension or composition. Step 9B adds the bounded second operation described below.

Phase 8C verification passed all five hosted parameter tests and both project-capability tests, the complete frontend checks (including 70 dashboard views and the new guided-form tests), and one focused Playwright flow with mocked hosted HTTP. That browser flow covers entry, bounds, exact preview, mode retention, rejected submission/retry, accepted navigation, async completion, refresh recovery and mobile layout. Server Clippy with warnings denied and repository formatting checks also passed. No implementation blocker remains for this one guided capability; the historical Phase 8B full-library fixture limitation above is unchanged.


## Step 9B: historical Stake Pool SOL deposit fee

`spl_stake_pool_sol_deposit_fee_v1` is the second schema-1 operation inside the same `protocol_parameter_change` family. It executes the retained deployed Stake Pool program's real `SetFee(SolDeposit(Fee))` in an isolated VM, verifies its complete instruction-produced pool account, and independently executes the same historical `DepositSol` against original and proposed pool state. There is no direct-mutation fallback. This simulation establishes no manager authorization, key possession, governance approval or on-chain update.

The outer identity mechanism, `candidate() = None`, absent activation, and original Token-2022 identifying bytes remain unchanged. Exact fractions are declarations: `1/100` and `2/200` identify different proposals. Record, deposit amount, executable, Clock and manager assumptions identify the analytical evidence, not the proposal. All new u64 fields require canonical decimal **strings**; referral percent is a JSON integer in `0..=100`. Unknown fields, byte offsets and unknown operations are rejected. No bps normalization or blanket positive-denominator validation is applied. The qualified deployed program accepts `0/0` and rejects `1/0` and `2/1` through actual configuration execution.

The [public proposal](examples/stake-pool-parameter-change.json) has this operation:

```json
{
  "kind": "spl_stake_pool_sol_deposit_fee_v1",
  "expected_current": {
    "account_data_sha256": "943b7d5c8ca449d060e197a6dc851f7b2d170b4a50a7a768e431d6f22ab979f2",
    "numerator": "0",
    "denominator": "1000",
    "sol_referral_fee_percent": 0,
    "last_update_epoch": "1036"
  },
  "proposed_fee": { "numerator": "1", "denominator": "100" }
}
```

The configuration target is pool `CV6bkrUksMwcEC4jfLTJsbHwF3Y2YurZdWWua95Fpbtd`, owned by program `SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy`. Any differing observed hash, rational, referral percentage or last-update epoch produces `current_state_mismatch` before any VM execution; expectations are never rebased.

### Executed retained qualification

The untouched tracked `deploy/bundle` contains the explicitly selected `mainnet-spl-stake-pool-151010f709e113e7`, at slot `447850493`, signature `313DzTBevDnV33BCsQojL6mfZj3fsPPGnAALQohcdoo2kQnNeEXq25ugR9whszK6Y5UYcKxGo3495k2jAPd3UH5F`. Its original baseline replays with `Matched` fidelity and no failures under the existing historical-archive contract. `Exact` is the separate controlled-snapshot contract. The historical Stake Pool ELF is `ec2dfefaa70d560754a0000f39bd2cabc192b895d36205b3c428f601b6e1d7e1`, loaded by the upgradeable loader; the retained SPL Token dependency is pinned separately. The original retained Clock, including epoch `0`, is held fixed; this is the existing schema-1 VM contract, not a claim of slot-accurate validator feature reconstruction. No Clock advancement occurs.

Only this qualified Stake Pool deployment is admitted by the new configuration path. A different deployment requires new layout and manager-boundary qualification and returns `config_execution_unavailable`. The official interface is pinned at `spl-stake-pool = 2.0.3` with `no-entrypoint`; it adds its required dependency graph without upgrading existing packages. Compatibility is established by complete retained Borsh roundtrip, cross-checks with Eplyx's partial reader, official instruction/enum equality, and successful execution of the retained ELF. It is **not** inferred from the crate release or asserted to be the ELF's build version. The wire Fee fields are denominator then numerator; the JSON declaration order is numerator then denominator. See the pinned official [state](https://docs.rs/spl-stake-pool/2.0.3/src/spl_stake_pool/state.rs.html), [instruction](https://docs.rs/spl-stake-pool/2.0.3/src/spl_stake_pool/instruction.rs.html) and [processor](https://docs.rs/spl-stake-pool/2.0.3/src/spl_stake_pool/processor.rs.html) sources.

The decoded manager is `8zVQTFGiwCZQSkNhedqMnWSddeGDrSahmM4JdZMceagx`. Its historical account envelope is absent from the DepositSol boundary. The report explicitly retains an **assumed simulation-only** System-owned, empty-data, non-executable manager envelope with 1,000,000 lamports and an assumed signer meta. Qualification tests prove key/signer enforcement: wrong manager and missing signer reject, while differing manager owner/data/balance still succeed. This boundary is pinned to the qualified executable. A separate assumed fee payer funds the config VM; its state never enters either action VM.

Actual SetFee succeeds using 3,597 CU and charges 10,000 modeled lamports. Complete post-state decoding verifies exact `1/100` and equality of every unrelated typed field, including authorities, balances, mint/reserve relationships, other fees, Options and FutureEpoch contents. Pool owner, lamports, executable flag, rent epoch, data length and exact trailing bytes are preserved. All non-payer/non-pool accounts are preserved; the payer changes only by its actual transaction fee. Populated variable-option/future-state controls run the real instruction and verify the same preservation. No padding differences are masked.

The two action VMs share the same complete message, metas, deployed program/dependencies, Clock/runtime, signer assumptions, watch set and every non-pool account. Restoring the original pool in the proposed plan must yield the complete baseline execution commitment. Only the verified post-SetFee pool account is substituted.

| Actual account delta | Baseline `0/1000` | Proposed `1/100` |
| --- | ---: | ---: |
| Recipient pool-token account credit | 760985008 | 753375157 |
| Independent manager fee-account credit | 0 | 7609851 |
| Mint supply delta | 760985008 | 760985008 |
| StakePool pool-token supply delta | 760985008 | 760985008 |
| Reserve / pool total-lamports delta | 822000000 | 822000000 |
| Funding payer debit excluding action transaction fee | 822000000 | 822000000 |
| Action transaction fee, separate from configuration | 14000 | 14000 |

The recipient and referral roles alias the same token account; referral percentage is zero. Their combined account credit is measured once, with no independently measured referral split asserted. Unique-account credits reconcile exactly to mint and pool supply deltas, and native-account deltas reconcile to the action fee. Only the existing `pool_tokens_received` economic subject is promoted; manager/referral/supply/reserve evidence remains operation-specific. No local fee arithmetic is the economic oracle. The actual retained deployment rounds a tiny positive fraction upward to one pool token, despite the existing partial reader's truncating helper; that unrelated helper is unchanged.

### Reports, failures and offline use

The existing outer `eplyx-protocol-parameter-report-v1` binding and seal dispatch by typed operation. New reports retain separate origins for observed historical state, declared proposal, assumed manager-signed config instruction/result, instruction-produced proposed pool state, and both simulated user-action results. Token-2022 recipient-transfer and withheld-fee fields are not reused. The prior published Token-2022 report seal remains verifiable and fully reproducible: only its known original source/lock commitments are admitted, while every execution-relevant runtime field must still match.

`config_execution_rejected` describes an executed SetFee rejection with verified rollback and separately retained config fee. Missing bytes or unqualified layout/signer boundaries are `config_evidence_missing` or `config_execution_unavailable`; unexplained post-state changes are `post_config_state_mismatch`. Unsupported actions fail admission as `downstream_action_unsupported`. DepositSol rejection/rollback is evaluated separately. Failed reconciliation emits no economic direction. Measured consequences and no-consequence controls are completed analytical results, not deployment BLOCK decisions.

Run from the repository root, using the existing CLI:

```sh
eplyx parameter analyse \
  --change docs/examples/stake-pool-parameter-change.json \
  --bundle deploy/bundle \
  --record-id mainnet-spl-stake-pool-151010f709e113e7 \
  --out stake-fee-report.json --record .
eplyx parameter reproduce \
  --change docs/examples/stake-pool-parameter-change.json \
  --report stake-fee-report.json
```

`--input` alternatively accepts the typed `spl_stake_pool_historical_deposit_v1` input. Existing Token-2022 `--capture`/`--input` commands remain compatible. A bundle always requires explicit `--record-id`. Reports retain the complete original record/boundaries/outcome, ELF/loader/dependencies, Clock, message/assumptions, config execution/post-state, action executions and commitments. Existing bounded artifact storage, local run reader and offline child execution are reused. Verification reconstructs bindings, preservation and reconciliation without a VM or provider. Reproduction executes configuration, baseline and proposed in three fresh VMs and requires complete deterministic report equality. Missing bytes fail; nothing is repaired from RPC.

### Hosted historical parent and readers

Use the existing authenticated `POST /v1/projects/{project}/runs/{parent}/parameter-changes` with `{ "request_key": "<unique valid key of at least 16 characters>", "change_spec": <proposal>, "record_id": "mainnet-spl-stake-pool-151010f709e113e7" }`. The historical parent is a terminal retained program-upgrade run with a retained report and project-owned qualified bundle, including a completed evaluation whose candidate was blocked. Its pinned **historical baseline**, never its candidate or a later active bundle, supplies this analysis's program bytes. Token-2022 still uses the original retained CurrentPath parent route and does not accept `record_id`.

`Input::ProtocolParameterChange` gains an optional historical projection binding bundle SHA, record ID/SHA and baseline SHA. The server constructs authoritative retained input in its existing content-addressed capture storage; callers cannot supply proposed bytes, binaries, config results, offsets or findings. Acceptance persists the ChangeSpec, exact input and parent projection before 202. Existing queue, isolated offline worker, run kind, project/change index and restart recovery are reused. Read verification checks indexed/stored/report ChangeSpec IDs plus exact authoritative parent, record, baseline and operation-specific input equality. Clearing or changing the active bundle cannot change a queued input. Tests include cross-project denial, wrong record/operation, a deliberately different parent candidate, durable reader/index tampering and provider-free restarted execution.

Minimal dashboard/hosted readers show exact fractions and integer deltas, record/slot/program commitments, manager assumption, configuration outcome separately from DepositSol, aliasing limits and factual failures. The completed Token-2022 guided form remains unchanged; no Stake Pool guided form is added.

Qualification tests cover the retained real gate, wrong/missing manager signer, complete variable Borsh preservation, exact identities and canonical strings, stale expectations, `0/0`, invalid fees, equal fee and tiny-rounding no-consequence, actual downstream privilege rejection and rollback, corrupted reconciliation with no findings, resealed cross-object tampering, saved offline reproduction, and both hosted parent routes. Synthetic controls are explicitly labelled; they never replace or rewrite the observed record. Compact public evidence and test results are in the [qualification commitments](examples/stake-pool-parameter-change-qualification.json). Full input/report bytes need not be published.

The scope is one retained interaction and one qualified deployment. There is no third operation, withdrawal/referral parameter, authority change, generic serializer, population sequencing, epoch advancement, governance, provider acquisition, signing, live execution or deployment.
