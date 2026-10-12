# Bounded program-upgrade rollout rehearsal

This is a bounded rollout rehearsal under retained state and explicit signer
assumptions. It is **not** a mainnet deployment, proof that any authority
possesses keys, governance approval, exact validator-bank equivalence, generic
arbitrary transaction sequencing, or complete protocol release safety.

It answers one question:

> Does changing the order of an actual installed program upgrade and the
> qualified Stake Pool `SetFee(SolDeposit)` configuration change alter the final
> retained `DepositSol` outcome?

The [upgrade × parameter interaction](upgrade-parameter-interaction.md) overlays
V2 bytes on fresh banks and never executes a loader `Upgrade`. This rehearsal
executes the upgradeable loader's own `Upgrade` instruction inside a pinned
world, keeps the installed ProgramData byte for byte, respects the next-slot
visibility boundary, and continues the scenario from that installed world. The
overlay path is kept, unchanged, as the control it is anchored against.

## Supported shape

Exactly one retained world, one historical installed V1, one explicitly
qualified V2 rollout candidate, one existing Stake Pool deposit-fee proposal
and one retained supported `DepositSol`:

| Input | Value |
| --- | --- |
| Retained record | `mainnet-spl-stake-pool-151010f709e113e7` in `deploy/bundle`, slot 447850493 |
| Program | `SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy`, loader-v3 |
| Historical V1 | `ec2dfefa…d7e1`, 1080464 bytes, deployed at slot 429882117 (retained manifest) |
| ProgramData | `EmiU8AQkB2sswTxVB6aCmsAJftoowZGGDXuytm6X65R3` (loader derivation), 1080464-byte capacity |
| Pinned dependency | SPL Token `8190d3f7…f697` |
| Configuration | `spl_stake_pool_sol_deposit_fee_v1`, pool `CV6bkrUk…Fpbtd`, unchanged qualified contract |
| Candidate profiles | constructed rollout counterexample `64612be0…c019` (134320 bytes); same-code V1 |

The two proposals stay what they are — one `program_upgrade` and one
`protocol_parameter_change` ChangeSpec, with their own identities. No new
ChangeSpec kind exists. The rollout has its own analysis-input identity,
`eplyx-rollout-rehearsal-input-v1`, binding both proposal IDs, S0, the
Program/ProgramData/Buffer identities, candidate artefact, configuration
commitment, retained action, dependencies, runtime and source revisions,
Clock, visibility rule, signer/payer assumptions, the five scenario
definitions and the account closure. Labels, paths and results are outside it.

Admission refuses (exit 2, nothing analysed): candidate bytes that are not the
upgrade ChangeSpec's candidate (`candidate_identity_mismatch`), activation,
Squads delivery or a stated expected upgrade authority (an assumed simulation
authority cannot prove one), a stated ProgramData address that is not the
loader derivation, a `replaces` that is not V1, and everything the qualified
SetFee/interaction scope refuses.

## S0: the installed world

S0 is the retained record's accounts and absences, the configuration
contract's assumed manager and payer, and the installed-program envelope:

- **Program** account: loader-owned, executable, `Program { programdata }`.
- **ProgramData**: header `{ slot: 429882117, authority: <assumed> }` plus the
  retained V1 bytes, padding included — exactly what the loader hands the VM.
- **Buffer** (`[83; 32]`): `Buffer { authority: <assumed> }` plus the candidate
  bytes, assumed written beforehand.
- **Upgrade authority** (`[81; 32]`) and **upgrade payer/spill** (`[82; 32]`).

Lamports of these envelopes are the default rent-exempt minimum; the authority
is a simulation key. They are reconstructed from retained evidence, not
observed. Retained absences — the depositor and the withdraw-authority PDA,
`absent_at_both_boundaries` in the record — are `KnownAbsent`. A missing row is
unknown, never absent: a message key with neither state, an absence proof, a
pinned program, a builtin nor a runtime sysvar is an `evidence_gap`.

States are content addressed. A state is `{ clock, accounts: address →
Present{account_sha256} | KnownAbsent }`; account bytes are stored once under
their own digest, so the 1 MiB ProgramData is retained once per distinct value.

## Execution and state handoff

Every step runs in a **fresh** LiteSVM seeded from its byte-bearing input
state: Clock first, the pinned Token program, then every Present account with
ProgramData before the executable Program header, so the loader relationship
comes from seeded bytes. The target program is never added by the backend. The
seeded closure is read back and must equal the state exactly. After the
transaction the full closure is read back (Present or KnownAbsent) and the VM's
complete account census is compared before/after: any change outside the
closure fails the step as `unexpected_write`. The output state is exactly the
execution's post rows; the next step starts from it. Nothing else carries.

### Upgrade

The step is the loader-v3 `Upgrade` instruction (legacy four-byte encoding,
which closes the Buffer on every runtime) with ProgramData, Program, Buffer,
spill, Rent, Clock and the authority as signer. The loader itself checks
ownership, the Program↔ProgramData link, Buffer and ProgramData authorities,
the signature, capacity, funding, "deployed in this block already", and
verifies and deploys the ELF. The rehearsal then verifies the installed world:

- only ProgramData, Buffer and the payer changed;
- ProgramData keeps its length, owner and authority, its slot is the
  upgrade slot, and its bytes are the Buffer's followed by all-zero padding;
- ProgramData is funded to exactly rent exemption; the Buffer is closed to
  `KnownAbsent`; the spill credit and fee reconcile exactly.

A rejection must roll back everything except the payer's fee. It is
`upgrade_rejected` (authority, signer, loader state) or
`candidate_installation_failed` (the loader refused to deploy the bytes).

### ProgramData capacity

Before any upgrade, the candidate's length is compared with ProgramData's
capacity. If it does not fit, every Upgrade step is `unsupported` with
`programdata_capacity_insufficient`, carrying the ProgramData address, account
digest, capacity and required length. Nothing is resized and a v1 analysis never
executes `ExtendProgram`: the finding is that this declared rollout cannot
execute until ProgramData is extended. Declaring that extension is input v2 —
see [ProgramData preparation and ExtendProgram](#programdata-preparation-and-extendprogram). A candidate with no compiled rollout profile is
likewise `candidate_not_qualified` and never installed. Steps that need no
upgrade still run.

### Slot visibility

On mainnet an upgrade executed at slot N is callable from N +
`DELAY_VISIBILITY_SLOT_OFFSET` (1). LiteSVM 0.16 does not reproduce that: its
post-transaction account sync reloads the Program with `effective_slot = N`, so
the new code runs in the same slot (`litesvm_exposes_an_upgrade_in_the_same_slot_which_the_model_refuses`
shows it). The rollout model compensates rather than inheriting that:

- any SetFee or DepositSol requested while `clock.slot < deploy_slot + 1` is not
  executed and is `unsupported` with `visibility_boundary_not_crossed`;
- `advance_to_visible_slot` is an explicit transition changing **only**
  `Clock.slot`, from N to N + 1. `Clock.epoch`, `epoch_start_timestamp`,
  `leader_schedule_epoch` and `unix_timestamp` keep their retained schema-1
  values (epoch 0, the existing fixed-Clock contract);
- the transition is refused as `unsupported_rollout_clock_transition` if N and
  N + 1 lie in different epochs of the declared mainnet schedule (432000 slots,
  no warmup), or if there is no pending boundary. Slot 447850493 is slot 298493
  of epoch 1036, far from a boundary.

Before/after Clocks are retained on every step.

### SetFee and DepositSol

The configuration step is the existing `spl_stake_pool_sol_deposit_fee_v1`
machinery, not a second engine: the same ChangeSpec, current-state checks,
official instruction encoding, manager identity, `assumed_simulation_only`
manager signer, separate configuration payer, complete pool-preservation check
(`verify_config`) and rollback check (`verify_config_rejection`). It is only
admitted from the retained configuration accounts; a pool, manager or payer
that differs from S0 is a `handoff_failure`.

The action is the retained `DepositSol` message, reconciled by the existing
unique-account reconciliation. Only a pool produced by a verified SetFee in the
same scenario may differ from S0, and only in its SOL deposit fee. Upgrade and
configuration transaction fees never enter action economics.

## The five scenarios

Each starts from an independently restored S0. Within a scenario, a verified
step hands its world to the next; a rejected, unsupported or failed step stops
the scenario and every later step is `not_executed` — never `rejected`, and
never a zero measurement.

| Scenario | Steps |
| --- | --- |
| Control | DepositSol |
| Upgrade control | Upgrade(V2) → advance → DepositSol |
| Config control | SetFee(C1) → DepositSol |
| Order A | Upgrade(V2) → advance → SetFee(C1) → DepositSol |
| Order B | SetFee(C1) → Upgrade(V2) → advance → DepositSol |

Scenario identities share the analysis root and differ by name and steps.
Step outcomes are `verified`, `rejected`, `unsupported`, `evidence_gap`,
`handoff_failure`, `unexpected_write`, `reconciliation_failed`,
`execution_unavailable` and `not_executed`, each with a reason.

## Fidelity anchors

**Anchor A — S0 → DepositSol.** The control, executed through the seeded
installed-program world, must reproduce the retained historical outcome under
the existing fidelity gate (`Matched`, no failures: outcome, fee, invocation
graph and post-state). Otherwise `baseline_world_fidelity_failed` and no other
scenario is executed.

**Anchor B — installed V2 vs overlay V2.** The upgrade control's DepositSol
(after a real Upgrade and the visibility boundary) is compared with the existing
candidate-overlay execution of the same bytes over S0: success, error, fee,
compute units, complete logs, CPI shape, watched post-state, reconciliation,
pre-action accounts and executable identity. Complete world bytes are not
compared, because ProgramData, deployment slot, Buffer, payer balances and
`Clock.slot` legitimately differ. Any difference is
`installed_overlay_behavior_mismatch` and the comparison is not established.

## Findings

| Status | Meaning |
| --- | --- |
| `rollout_order_effect_observed` | Anchors matched, controls completed, both orders fully classified, and a step's outcome or established result differs |
| `no_order_effect_observed` | Anchors matched and both orders reached equivalent reconciled final DepositSol results |
| `rollout_not_established` | Anything else; the reasons are listed |

The report answers: did all steps execute, what installed state followed each
step (installed executable identity, deploy slot, visible-from slot, authority),
whether the final states differ (and which accounts), whether the final action
executed differently, every final metric for both orders (or why unavailable),
the first divergent step, and which results are established versus unavailable.
Wording never says safe, unsafe, approved or vulnerable.

## Qualified results (retained record)

The counterexample candidate rejects a newly configured SOL deposit fee above
1/200 but applies whatever fee the pool already holds (see its
[README](../programs/fixture-stake-pool-rollout-candidate/README.md)). With the
existing proposal C1 = 1/100:

| Scenario | Result |
| --- | --- |
| Control | verified; historical fidelity `Matched`; recipient +760985008 |
| Upgrade control | Upgrade verified (Buffer closed, 946144 zero-padding bytes); advance 447850493 → 447850494; DepositSol verified, identical to the overlay |
| Config control | SetFee(1/100) under V1 verified; DepositSol recipient +753375157, manager +7609851 |
| Order A | Upgrade verified; advance; **SetFee(1/100) rejected under V2** (`FeeTooHigh`, rollback verified); DepositSol `not_executed` |
| Order B | SetFee(1/100) under V1 verified; Upgrade verified; advance; DepositSol under V2 applies the inherited 1/100 |

Both anchors match. Status `rollout_order_effect_observed`, finding
`rollout/order/configuration_installation_outcome_differs`, first divergence
SetFee: *under the pinned world and explicit signer assumptions, changing
rollout order changed whether the proposed fee configuration could be
installed.* Order B's final DepositSol: recipient credit 753375157, manager
fee-account credit 7609851, mint and pool-token supply +760985008, reserve and
pool total lamports +822000000, funding payer debit 822014000 (822000000
excluding the 14000-lamport action fee); referral is unavailable because the
roles alias. Order A's final metrics are unavailable (not executed), never zero.

With C1 = 1/1000, inside the candidate's maximum, every step of every scenario
is verified and the status is `no_order_effect_observed`. The same-code V1
candidate, which fits ProgramData exactly, is also `no_order_effect_observed`.

## Portable evidence, verify and reproduce

```sh
cargo build -p eplyx-engine --bin eplyx --offline --locked
target/debug/eplyx rollout analyse \
  --upgrade upgrade.json \
  --parameter docs/examples/stake-pool-parameter-change.json \
  --bundle deploy/bundle \
  --record-id mainnet-spl-stake-pool-151010f709e113e7 \
  --candidate fixtures/rollout/fixture_stake_pool_rollout_v2.so \
  --out /tmp/eplyx-rollout --format json
target/debug/eplyx rollout verify --artifact /tmp/eplyx-rollout --format json
target/debug/eplyx rollout reproduce --artifact /tmp/eplyx-rollout --format json
```

`upgrade.json` is a `program_upgrade` ChangeSpec naming the candidate's SHA-256
and length. `--out` must be fresh. All three commands run in the existing
empty-environment local worker; a refused preflight is a structured JSON error
with exit 2. A completed analysis exits 0 whatever its status.

The artifact follows the interaction convention: `objects/<sha256>` written
first, `report.md`, then `manifest.json` last, naming `input.json`,
`contract.json`, `report.json`, `proposals/{upgrade,parameter}.json`,
`programs/…`, `states/<id>`, `executions/<id>` and `accounts/<id>`. A state,
execution or account's id is its own canonical digest, so identical content is
stored once (51 objects, about 9 MB for the counterexample).

**Verify** never starts a VM. It checks the seal, every object hash and strict
canonical form, rebuilds the input contract and preflight (including the
capacity claim) from the input, and reduces the retained executions again:
state chain, Clock transitions, visibility guards, handoffs, classifications,
reconciliation, anchors and the order comparison must reproduce the retained
report and evidence exactly. A tampered state, execution or report, or a
resealed inconsistent one, fails. Verification proves internal consistency,
not that a VM ran: a rewritten log on a step that reads no log stays consistent.

**Reproduce** verifies first, then reruns all five scenarios and the overlay in
fresh VMs (11 executions for the counterexample) from retained bytes alone and
requires byte-identical report and evidence. No provider, network, bundle or
checkout is involved; the test suite runs it inside a private network
namespace. Runtime differences are not waived.

## ProgramData preparation and ExtendProgram

A loader-v3 program's bytes live in its ProgramData account: a 45-byte header
(`UpgradeableLoaderState::ProgramData { slot, upgrade_authority }`) followed by
the **executable capacity** that `Upgrade` overwrites. `Upgrade` never resizes
that account, so a candidate longer than the capacity cannot be installed until
`ExtendProgram` grows it. Four lengths are kept distinct: the account data length
(1,080,509 for the retained Stake Pool), the header (45), the executable
capacity (1,080,464, all of it the retained V1 including padding) and the
candidate ELF length.

The rehearsal **diagnoses** a missing extension; the **declared rollout decides**
whether to include one. A computed minimum is presentation and never turns a
plan without an extension into one with it.

### The declared plan (input v2)

`eplyx rollout analyse --plan plan.json` reads a small strict document:

```json
{"schema_version": 1, "programdata_preparation": {"kind": "none"}}
{"schema_version": 1, "programdata_preparation": {"kind": "extend_program", "additional_bytes": "65920"}}
```

`additional_bytes` is a canonical decimal string; unknown fields, other kinds,
numbers and non-canonical strings are refused. A plan makes the analysis input
`eplyx-rollout-rehearsal-input-v2`: the identity additionally binds the declared
preparation, the preflight, the rent identity, the exact ExtendProgram message
and the extension payer assumption, besides everything v1 binds. Without
`--plan` the analysis is the unchanged v1 family above. No ChangeSpec kind is
added: preparation is rollout setup, not a protocol proposal.

### Preflight

`eplyx rollout preflight` (same inputs, optional `--plan`) executes no VM and
writes nothing. From the retained world and the candidate it reports the
Program and ProgramData identities, loader, account length, header length,
executable capacity, the installed executable, the candidate, whether it fits,
the exact `minimum_additional_bytes_required`, and for a declared extension the
extended lengths, surplus or shortfall and the rent funding
(`max(1, (128 + new_len) × 6960) − current lamports`, from the same `Rent` as
the runtime sysvar) against the assumed payer. Status is one of
`ready_without_extension`, `extension_required`,
`declared_extension_sufficient`, `declared_extension_insufficient` or
`extension_unsupported` (`zero_extension`, `below_minimum_extension`,
`programdata_size_overflow`). It is setup evidence — arithmetic over retained
bytes — and never states that an extension executed.

### The pinned loader's ExtendProgram

Established from `solana-bpf-loader-program 4.2.2` (`common_extend_program`,
`check_authority = false`) and the official `solana-loader-v3-interface 8.0.1`
builder, under LiteSVM 0.16's mainnet features:

- accounts: ProgramData (writable), Program (writable), System program, payer
  (writable, signer). **No upgrade-authority signature is read.**
- `additional_bytes` is a `u32`; `0` is `InvalidInstructionData`; a new length
  above 10 MiB is `InvalidRealloc`; SIMD-0431 (active) requires at least
  10,240 bytes unless the extension exactly fills the remaining headroom.
- an immutable program, a Program↔ProgramData mismatch or a same-slot repeat is
  refused.
- if `max(1, rent(new_len))` exceeds the current balance, the difference is
  transferred from the payer by one System CPI.
- the account grows with zero bytes, the **existing executable is redeployed**
  and ProgramData's slot becomes `Clock.slot`.

That last point matters: like `Upgrade`, `ExtendProgram` redeploys, so the
loader refuses an `Upgrade` in the same slot ("Program was deployed in this block
already") and the Step 16A visibility rule — callable from `deploy_slot + 1` —
applies after the extension too. The rule is unchanged; it is driven by
ProgramData's slot, which the loader rewrites. Every extension is therefore
followed by the same explicit `advance_to_visible_slot` transition.

The rehearsal executes the official instruction through the loader and then
verifies byte for byte: only ProgramData and the payer changed; the account grew
by exactly the declared bytes, all zero; the header keeps its authority and takes
the new slot; the installed V1 bytes are identical (V1 stays the effective
executable; no candidate bytes are installed); ProgramData holds exactly the
rent-exempt minimum for its new length; the payer paid exactly that funding plus
the 5,000-lamport fee; funding was exactly one System transfer. A rejection must
leave ProgramData untouched with only the fee charged: `extend_program_rejected`,
`extension_funding_insufficient` or `programdata_size_overflow`; a post-state that
does not reconcile is `extend_program_post_state_mismatch`. The upgrade payer's
spill, the extension funding, every transaction fee and the user action are
reconciled separately.

### Scenarios with a declared extension

| Scenario | Steps |
| --- | --- |
| Control (P0) | DepositSol |
| Upgrade without preparation (P1) | Upgrade → advance → DepositSol |
| Preparation control (P2) | ExtendProgram → advance → DepositSol |
| Upgrade control (P3) | ExtendProgram → advance → Upgrade → advance → DepositSol |
| Config control | SetFee → DepositSol |
| Order A (P4) | ExtendProgram → advance → Upgrade → advance → SetFee → DepositSol |
| Order B (P5) | SetFee → ExtendProgram → advance → Upgrade → advance → DepositSol |

With `none`, the five v1 scenarios run. Each Upgrade is admitted against the
ProgramData it actually receives: when the candidate does not fit, it is
`unsupported` with `programdata_extension_required` (no extension in that
scenario) or `declared_extension_insufficient` (an executed extension was too
small), carrying the current capacity, the requirement and the missing bytes.
No impossible Upgrade is executed and nothing is extended implicitly. An
extension that succeeded stays in the world: the rollout is a sequence of
transactions, not an atomic one.

Anchors: the Step 16A baseline-fidelity and installed-vs-overlay anchors, plus
the **extension-only anchor** — P2's DepositSol must equal P0's in outcome, fee,
compute units, logs, CPI shape, watched post-state, reconciliation and effective
executable (V1). A difference is `extension_behavior_mismatch`.

If the declared rollout's Upgrade is blocked by capacity, the status is
`rollout_precondition_missing`. In order B the report retains, byte for byte,
the pool SetFee produced at each later world up to the final action's input
(`inherited_configuration`).

### Qualified results

The [oversized counterexample](../programs/fixture-stake-pool-oversized-rollout-candidate/README.md)
(`329092d7…c7c4`, 1,146,384 bytes) has the counterexample's behaviour plus
read-only ballast; it needs exactly **65,920** more bytes.

| Declared plan | Result |
| --- | --- |
| `none` | `rollout_precondition_missing`: "The declared rollout cannot install the candidate because the retained ProgramData account is too small. At least 65920 additional bytes are required." No ProgramData byte changes anywhere. |
| `extend_program` 65,919 | ExtendProgram verified (ProgramData 1,146,428 bytes, retained); Upgrade `declared_extension_insufficient`, 1 byte short; nothing downstream; `rollout_precondition_missing` |
| `extend_program` 65,920 | ExtendProgram verified: ProgramData 1,080,509 → 1,146,429 bytes, lamports 7,521,233,520 → 7,980,036,720 (458,803,200 funding), payer −458,808,200 including the fee. Extension-only anchor matched (V1 DepositSol identical at slot +1). Upgrade at slot +1 installs the candidate filling the capacity exactly; installed-vs-overlay anchor matched. `rollout_order_effect_observed` |
| `extend_program` 86,400 | same, with 20,480 bytes of zero capacity after the candidate; identical user-action results |
| `0`, `4096`, 10 MiB | loader rejection (`InvalidInstructionData`, `InvalidArgument`, `InvalidRealloc`), ProgramData untouched, fee only |
| above `u32::MAX` | refused: not encodable as ExtendProgram |

With the 65,920-byte extension, order A's SetFee(1/100) is rejected under V2
(`FeeTooHigh`, rollback verified) and its DepositSol is not executed; in order B
SetFee(1/100) is accepted under V1, the resulting pool is byte-identical through
the extension and the upgrade, and V2's DepositSol applies it: recipient
753,375,157, manager fee account 7,609,851, mint and pool-token supply
+760,985,008, reserve and pool total lamports +822,000,000, funding payer debit
822,014,000 (action fee 14,000); referral unavailable (aliased).

### Evidence and compatibility

The artifact carries the plan, the preflight, the ExtendProgram message and
execution, complete before/after ProgramData, payer evidence and every state;
identical bytes are stored once. `verify` (0 VMs) rebuilds the v2 identity and
preflight and reduces the extension, upgrade, transitions, configuration,
action and comparison again; a tampered plan, preflight, resized ProgramData,
payer delta, upgrade input or scenario chain fails. `reproduce` reruns every
step offline.

Step 16A artifacts keep working unchanged. Their contract commits the Step 16A
analyzer source (`mod.rs` `d7229bb3…`, `world.rs` `d7a5be63…`); this build accepts
exactly that commitment for input v1, reduces and reproduces it through its
preserved v1 code path, and reaches the identical report bytes and identities
(analysis input `ffafb0ad…`, report `74454af9…`). Any other source commitment
that is not the build's own is refused. New v1 analyses bind the current
source, as interaction receipts do.

## Limitations

- One retained world, one record, one action and one fixed configuration
  operation. Only the Program/ProgramData/Buffer envelope is reconstructed;
  its lamports and authority are assumptions.
- The upgrade authority, upgrade payer, Buffer contents, manager signer and
  configuration payer are simulation assumptions; signature verification and
  blockhash age are disabled locally.
- Only `Clock.slot` advances. No time, epoch or feature-set evolution is
  modelled; LiteSVM 0.16 with its mainnet feature set is the backend.
- The candidate is a constructed counterexample, not an upstream release, and
  its identity is qualified on Linux x86_64 only (the bytes are tracked).
- Not executed: automatic or repeated `ExtendProgram`, `ExtendProgramChecked`,
  `Write`/`InitializeBuffer`, Squads or other governance messages, migrations,
  multiple candidates, arbitrary step sequences, live acquisition, hosted
  routes, valuation.
