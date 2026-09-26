# T0 — Stock Transition integration: reference and decisions

Status: **T0 architecture approved; pre-T1 corrections verified; T1 authorized**.
Recorded 26 September 2026. The owner-review addendum below closes the initial
checkpoint while retaining the original source limitations and test outcomes.

This is the behaviour-preservation contract and proposed integration design for
migrating Stock Transition Assurance (STA) into MAIN. It does not claim that the
migration has been implemented. MAIN is based on `5bfd81399eaff23301d14a05d6869a86a8e4720e`
on `t-token-migration`. STA is the read-only `Post-Hackathon` reference at
`ad1897e86ee05ff2255bd5277518a3230ee2340f`.

## 1. Checkpoint and accepted reference corrections

The first commit, `73a1c9d`, corrects MAIN's deliberately pinned adapter count from
four to five and names Drift in the assertion. No adapter has been added.

The requested MAIN SBF hash was reproduced without changing a pin. MAIN's format,
lint, full test, no-fail-fast test, frontend, report and governance checks pass.
STA's baseline is recorded, including these discrepancies:

1. The minimal reference candidate produces **20/20** stress cases behaving as
   specified, whereas the migration request says **19/20**. The deadline defect
   produces 19/20 and exactly one derived deadline counterexample, reproduced
   offline. The reference candidate, input and assertions were not changed.
2. Three STA cloud browser assertions fail because their expected wording differs
   from the rendered wording. Four cloud browser tests pass. The assertions have
   not been edited. Section 12 records both versions for review.
3. Feature-specific clippy on STA's fee-ceiling defect rejects an unused `VECTORS`
   constant. Its consuming test is already disabled for that defect. A scratch-only
   trial applying the same conditional compilation to the constant passes clippy.
   Port that mechanical fix with the program in T3; preserve all assertions.

Those were the initial checkpoint findings. The owner approved the architecture
and frozen fee-defect limitation; the pre-T1 review in section 13 confirms the
stress semantics and classifies all three wording mismatches as stale copy in
their tested scope. Corrected scratch browser assertions pass, and exact pinned
pnpm checks pass. T0 is accepted with those source limitations, rather than claiming
the untouched STA suite itself is green. No analytical assertion was weakened.

The owner explicitly approved Postgres for the narrow mutable-identity and
authorization role in section 10. There must be no second authoritative project,
run or evidence registry in Postgres. No Postgres dependency, migration, service or
deployment configuration has been added to MAIN at T0. The disposable local
database used to test STA is separate from this proposed product change.

The loader experiment and its conclusion are recorded in section 5. Release
packaging remains optional and requires a separate decision in T10.

## 2. Reference provenance and how to review it

The following files are part of this ADR:

- [Observed migration values](examples/phase-t0-stock-transition-integration/sta-migration-reference.json):
  per-case classes, both gates and all reason codes, readiness, every invariant,
  exact reconciliation equations, stress verdicts, searches and unsigned-plan
  cross-checks. These are STA values; MAIN values are pending T3.
- [Test contract](examples/phase-t0-stock-transition-integration/sta-test-contract.json):
  657 named tests in 83 source/test files, original file and test-body hashes,
  assertion expressions, helper assertions and fixture references. Assertions
  record expected meaning; successful execution is recorded separately.
- [Fixture inventory](examples/phase-t0-stock-transition-integration/sta-fixture-inventory.json):
  original paths, sizes and SHA-256 values. Its conservative static dependency
  closure is an inventory, **not an instruction to copy every file**.
- [Verification record](examples/phase-t0-stock-transition-integration/verification.json):
  commands, actual outcomes, setup failures, retries and explicitly pending checks.

STA was copied to disposable scratch storage for builds and tests. Only tracked
source/evidence was copied; private provider files, environment files, secret-key
file patterns, symlinks and agent settings were excluded. Existing dependency
caches were reused. A scratch Git context and generated lending fixtures were
needed by STA tests; neither changed STA. Browser/service outputs were confined to
scratch. No historical report in either repository was regenerated in place.

The test contract includes archive controls, because some moving guarantees are
currently tested through fixed-ratio packages or issuer-specific investigations.
During each port, map every applicable assertion to its MAIN test; leave archive
controls explicitly identified. A suite rename or passing aggregate count is not
evidence that all its assertions survived. The inventory records static suite and
helper dependencies, not a dynamic per-test file-access trace. Narrow and verify
the actual dependency closure before copying fixtures in T3/T6/T7/T8/T9.

## 3. Reproducing the SBF artefacts

The platform-tools version **and SBF architecture** must be pinned. Version alone
did not explain the initial mismatch. Tests used `cargo-build-sbf 4.4.0`, explicit
tool versions and architectures, existing installed tools, and offline dependency
resolution. MAIN's v1.54/v0 output was also reproduced with the platform-tools
Cargo directly for target `sbpf-solana-solana`, release/locked, remapped working
directory and `llvm-objcopy --strip-all`.

| Build | Platform tools | Architecture | SHA-256 |
| --- | --- | --- | --- |
| MAIN fixture lending V1 | v1.54 | v0 | `b307faa27e09fc74fee9e8087263f249bef565a6998f49fd5e562d2932bf1641` |
| STA migration reference | v1.57 | v3 | `e5db6948abca1317eb12155f73cdaf619d378c22d1bfc992c977063e10e9c1bb` |
| STA deadline-inclusive defect | v1.57 | v3 | `c4346736008188befe1d1a41cf167ea6a80c3b61a76cf1d3ba550f916b682275` |
| STA fee-ceiling defect | v1.57 | v3 | `a93aff2f19f249234950ac6646f97f2c36e492a43ddff073796372a037382c74` |
| STA archived demo conversion | v1.57 | v3 | `70fb9c964723a5a81c7dce21b6723411933273c7839d3ea25baf8f782f80048a` |
| STA fixture lending V1 (not MAIN's pin) | v1.57 | v3 | `585004ab8aa838fe866e5020e2c9232ae0702fa4cc51fe2a5d7acf4334ecd6bc` |

The migration artefact sizes are 53,488 bytes (reference and deadline defect) and
53,528 bytes (fee defect). The unchanged migration program lockfile hash is
`987495da71275fbbed8342d8b4597c05229bf3d7616f6582261382f7a7da3cb8`.
MAIN's fixture program lockfile hash is
`6dd0a6ba99e370eae9f4e09eff2b154ef64ff2c8a9814a57e886b726e7d8067f`.

v1.54 uses rustc 1.89.0-dev, LLVM 20.1.7, rust source commit
`daa3af4a1110ec3f10ce08083bb0b7855a88416f`; v1.57's rust source commit is
`ae660768afd467d131776918a1627e18920b230c`. Its Cargo source commit is
`bbbd3a76017677319ff85d096a7c9bee9813c3db`. Verify installed release metadata when
preparing CI. The artefact hashes above, not a version string alone, are acceptance
criteria.

Use the installed platform-tools' `rust/bin/rustc` as `RUSTC`. These are the
effective wrapper commands; `SBF_BUILD_BIN` names the installed 4.4.0 executable
and `TOOLS_154`/`TOOLS_157` name installed platform-tools directories:

```sh
# MAIN wrapper; pass through the repository script's arguments.
RUSTC="$TOOLS_154/rust/bin/rustc" "$SBF_BUILD_BIN" \
  --tools-version v1.54 --arch v0 --skip-tools-install \
  --no-rustup-override --offline "$@"

# STA migration/reference wrapper.
RUSTC="$TOOLS_157/rust/bin/rustc" "$SBF_BUILD_BIN" \
  --tools-version v1.57 --arch v3 --skip-tools-install \
  --no-rustup-override --offline "$@"
```

Place the appropriate wrapper first on PATH and run each repository's
`./scripts/build-programs.sh`. T0 used separate wrappers to avoid changing source
scripts. T3 must dispatch these choices per program in MAIN's build wiring, retain
the migration crate's separate lockfile, and build/test/lint both defect features.
Never use one ambient SBF default for all candidates or silently re-pin evidence.

## 4. Frozen behaviour and permitted identity differences

### 4.1 Token migration

The table below is the **before** contract. Every MAIN **after** value is pending
T3. The linked JSON contains exact values beyond this overview. “Stress” means
`behaves_as_specified == true` divided by cases; 0/0 is unevaluated. The unsigned
column counts matching fresh-VM executions. A null search is not zero findings.

| Case | Class counts | Block-only/strict | Stress | Search witnesses | Unsigned | Reconciliation |
| --- | --- | --- | --- | --- | --- | --- |
| case-a | Migratable 3 | Pass/Pass | 18/18 | 0 | 3 | FullyReconciled |
| case-b | Migratable 3 | Pass/Pass | 19/19 | 0 | 3 | FullyReconciled |
| case-c | Migratable 4 | Pass/Pass | 22/22 | 0 | 4 | FullyReconciled |
| case-d | InsufficientReserve 1, Migratable 2 | Block/Block | 16/16 | 1 | 2 | ReconciledForExecutedUnits |
| case-defect | Migratable 3 | Block/Block | 13/20 | 2 | 3 | Mismatch |
| case-e | Migratable 2, OutputBelowMinimum 1 | Warn/Block | 17/17 | 0 | 2 | ReconciledForExecutedUnits |
| case-f | AuthorityPathUnavailable 1, Frozen 1, Migratable 3, UnsupportedTokenSemantics 1 | Warn/Block | 20/20 | 0 | 3 | ReconciledForExecutedUnits |
| case-g | Migratable 3 | Block/Block | 18/18 | 0 | 3 | FullyReconciled |
| case-g-mint | FundingPathUnavailable 3 | Block/Block | 0/0 | 0 | 0 | NothingExecuted |
| case-h | MintStateBlocksMigration 3 | Warn/Block | 0/0 | 0 | 0 | NothingExecuted |
| case-h-paused | MintStateBlocksMigration 3 | Warn/Block | 0/0 | 0 | 0 | NothingExecuted |
| minimal | AuthorityPathUnavailable 1, Frozen 1, Migratable 4, ZeroBalance 1 | Warn/Block | 20/20 | not searched | 4 | ReconciledForExecutedUnits |
| minimal-deadline-defect | AuthorityPathUnavailable 1, Frozen 1, Migratable 4, ZeroBalance 1 | Block/Block | 19/20 | 1 | 4 | ReconciledForExecutedUnits |
| spacex-demo-frozen | AuthorityPathUnavailable 68, InsufficientReserve 3, Migratable 8, OutputBelowMinimum 308, UnverifiableAuthority 2925, UnverifiableDestination 6779, ZeroBalance 7779 | Block/Block | 18/18 | not searched | 8 | ReconciledForExecutedUnits |


For the frozen SPACEX world: 17,870 token accounts, 10,091 positive; funding is
Blocked with `INSUFFICIENT_RESERVE`; all seven equations hold. Its unchanged STA
world digest is
`4c797e791f1f3d006e32f9d2f8277ad9c548293c56b8c91a0be55eb50b14d040`.
The original fixed-ratio package also replayed byte-for-byte in STA; it remains an
archive control, not a second MAIN product kind.

The minimal example has seven accounts, six positive, four of four attempted
holders migrated, a warning gate under block-only and a blocked strict gate. Its
codes are `AUTHORITY_PATH_UNAVAILABLE`, `SOURCE_FROZEN`, `STRANDED_HOLDERS`. The
accepted reference statement is **20/20 stress cases behaved as specified**:
11 expected migrations succeeded and reconciled, and nine expected rejections
rejected with rollback verified. It does not mean 20 migrations succeeded. The
four-holder population rehearsal is separate from the stress matrix.
The deadline defect has one derived `UnexpectedSuccess` witness at the exclusive
deadline; `eplyx reproduce` returned success offline. No mainnet failure is implied.

All three `migration_capture` tests passed: mock-mainnet classification equals the
fixture, non-mainnet genesis is refused, and a one-holder bound records three
uninspected destinations. Only one of those becomes `UnverifiableDestination`;
the frozen and program-controlled holders retain their own classes.

The 27 golden economics vectors passed. The unchanged CSV hash is
`9cf2e06202ed0e4cb75af36c9d5c90203eed729c14347e7ce89be7fdabbadc7e` (1,817 bytes).
The minimal fixture recipe hash is
`5f47f8efaed31bd8268b62fc112bcf1a8ed942b73bd99ddf5a5d7d6a04bd75cc` (2,598 bytes).
The captured-program source `reports/milestone4-validation/live-market.capture.json`
is pinned at `3d7ba0f24d424bcc0042495f5cdf39555c0b09569d5dfcecc8017097a8604b52`
(5,560,223 bytes); record 6 supplies the pinned token programs. Copy required
capture bytes unchanged, with STA commit/path/hash in a per-directory README.

### 4.2 Other capabilities

The test contract preserves names and assertion expressions for coherence,
authority resolution, rebinding, observed search, lifecycle policy/consequence,
counterfactual, notice, readiness, rollout, current observations, wallet scope,
Transfer, market exit, positions, local store, dashboard and cloud. All Rust
baseline failures caused by missing scratch fixtures/Git context or loopback
restrictions were rerun successfully. The initial cloud browser discrepancies are
resolved as reference copy corrections in section 13; STA remains unchanged.
The contract also identifies opt-in live browser checks as not run; missing
artefacts are never treated as successful skips.

### 4.3 Identity changes: an explicit, bounded allowance

| Artefact | STA before | MAIN after / reason |
| --- | --- | --- |
| Existing program-upgrade ChangeSpec, bindings and reports | MAIN's current bytes | Identical; no allowance for change |
| Migration spec identity | SHA-256 of STA pretty JSON plus newline | MAIN domain-separated compact tuple including kind and top-level activation |
| Lifecycle identity | STA policy/scenario encodings | MAIN ChangeSpec identity over declared fields and activation |
| Proposed migration config/PDA addresses | Derived from STA spec digest | Derived from MAIN identity; validate every address and fresh-VM plan again |
| Derived world/plan/search IDs | STA schemas and parent digests | Change only when a rewritten schema, identifying field or proposed PDA changes their preimage |
| Report/binding digests | STA fields, numeric encoding, run metadata | New schema, MAIN binding, decimal strings and removal of wall-clock/run metadata |
| Raw frozen world/capture/recipe/program bytes | Original hash | Identical; a new descriptor may refer to the original bytes |
| Counterexample IDs | Original parent bindings/digests | Re-derived MAIN identity; same mutation, finding and offline witness semantics |

These are proposed encoding changes, not permission to replace expected economic
values, widen authority evidence, relax reconciliation, or discard a failed case.
T3/T6 must enumerate actual changed digest values and their preimages. Metadata
such as `run_id` and `evaluated_at` moves to run records; it is absent from canonical
reports. Two identical inputs must produce identical report bytes.

## 5. ChangeSpec, resolution and execution primitives

Keep MAIN's `ChangeSpec { schema_version, change_spec_id?, change, activation?,
metadata }`, `#[serde(tag = "kind", rename_all = "snake_case")]` and identity
`SHA256(("eplyx-change-spec-v1", schema_version, change, activation))`. Metadata
remains outside identity. Absent optional identifying fields are omitted. The
frozen program-upgrade ID must remain
`b5a894cdbec6251f73b4224a294579fe1af9e316232949fa92da852468900bf3`.

T1 first makes `with_delivery`, `bind`, candidate and target accessors, CI bindings
and server indexes kind-aware. Fallible upgrade-only operations reject another
kind. A lifecycle input must not acquire a dummy executable or program target.
Preserve the existing program-upgrade serialized shape and all byte contracts;
add variants only when executable implementations exist. Do not import STA's
seven-kind registry.

`token_migration` contains source/destination `{ mint, token_program, decimals }`,
conversion `{ ratio_basis, numerator, denominator, rounding, fee,
minimum_output_raw }`, eligibility `{ amount_policy, minimum_source_balance_raw,
holder_authorization, owner_authority_classes, excluded_accounts }`, source
disposition, destination funding, authorities, deadline and mechanism. Mechanism
contains `program_id` and MAIN's `ExecutableArtifact { sha256, len }`. This is one
composite change kind, resolving C1's open question about two changes.

`lifecycle_change` follows C1 §11: asset, destination, ratio, eligibility, optional
deadline and top-level activation, with explicit before/after policies and source
provenance for declared assumptions. Unknown terms stay unknown. Captured notices
can produce a specification through a generic source interface; they cannot prove
an official mechanism. Deterministic lifecycle evaluation times are inputs,
separate from run timestamps.

Migration activation moves from `window.activation` to the top-level slot or Unix
timestamp. Per-kind evaluation sets the rehearsal Clock at activation. Generic
activation semantics and program-upgrade execution remain unchanged. Activation
is inclusive; deadline is exclusive; incompatible time axes are invalid input.

`ChangeSpec::resolve` remains the sole constructor of executable resolved input.
It loads migration mechanism bytes through `CandidateSource` and MAIN's
`programs/<sha256>` CAS, checking length and hash. A separate validated state input
is mandatory; a schema-3 package no longer combines proposal, executable and state.

**Loader decision:** adopt MAIN's upgradeable Program/ProgramData loader in T3,
supported by the scratch experiment below. Captured token program
bytes retain their captured loaders; changing the candidate loader must not change
dependency loaders. Reference/defect outcomes, rollback, exact reconciliation,
search replay and unsigned-plan cross-checks must pass before adoption. A loader
match removes STA's candidate-loader mismatch qualification, but never proves
that local compute costs or account costs equal a live cluster.

The scratch-only substitution of `migration::adapter::CANDIDATE_LOADER` from
BPFLoader2 to the upgradeable loader passed all 30 migration integration tests
and all 17 selected migration unit tests. The performance benchmark was not run.
Cases A–H and the fee defect retained their exact classes, readiness, invariant
statuses/explanations, reconciliation, coverage, gate codes and stress outcomes;
their population compute totals were also identical. The comparison excludes
derived evidence/message/log hashes, not economic values. The full original
integration assertions, including search and fresh-VM cross-check/replay, passed.
See [loader experiment](examples/phase-t0-stock-transition-integration/loader-experiment.json).
The scratch substitution was restored. This establishes loader compatibility;
MAIN's resolved-artifact and ProgramData wiring still require tests in T3.

Use MAIN's `standard_programs::{spl_token, token2022}`. Extend typed decoding for
PermissionedBurn, MintCloseAuthority, InterestBearingConfig, NonTransferable,
CpiGuard, MemoTransfer, ImmutableOwner, MetadataPointer, TokenMetadata, Group
extensions, confidential transfer and MintBurn families (presence and required
fields). Test real byte layouts. Preserve tolerant decoding; fail closed in the
migration/lifecycle/path support policies on `Unrecognized` or `Malformed`.
No adapter may introduce private token layouts or a TLV walker. ScaledUiAmount
finiteness uses bits/integer checks, not `f64`.

Extend MAIN's synchronous curl `RpcProvider` with bounded responses, contextual
`getProgramAccounts` filters, `getMultipleAccounts` with `minContextSlot`, and
genesis verification. Add no engine `reqwest` or `tokio`. All migrated u64/i64
fields use MAIN decimal strings; base units remain paired with mint decimals.
Existing upgrade encodings stay byte-identical. Expand the no-floating-point
checks to every migrated valuation/report module.

Re-home shared dependencies narrowly:

| STA dependency | MAIN home |
| --- | --- |
| `stress::population`, `StressBudget` | migration population/capture on MAIN RPC |
| `conversion::package_gate` | `migration::gate`, shared typed policy, one evaluator per kind |
| `conversion::invariants::{Result, Severity}` | migration requirements/invariants |
| `conversion::demo::proposed_token_account` | standard token account builder |
| fixed-ratio Manifest/Config/Observation | minimal read-only frozen-fixture parser |
| lifecycle decode/hash/RPC | standard programs, shared digest helper, MAIN RPC |
| probe constants/Clock/transfer fee | shared IDs/sysvars and Token-2022 helpers |
| `resolution::PathStatus` | shared path-status type used by lifecycle/path checks |
| `expansion::{canonical, digest}` | shared deterministic pretty-JSON-plus-newline helper for new artefacts; never replace MAIN ChangeSpec's compact identity encoding |
| LoadedProgram / ProbeInnerInstruction | minimal required additions to MAIN executor/types |
| fixed-ratio run views/replay | archive only; no such run kind in MAIN store or sync |

Preserve serialization-only proof/evidence types and private verified constructors.
Deserializing stored inputs or view data must never manufacture verified evidence.

## 6. State, provenance and the conversion guarantees

State inputs contain either an immutable capture descriptor/transcripts or a
synthetic recipe. They are distinct from the proposal and CAS mechanism. Each
account retains its record index/JSON pointer, recipe step, or parent/mutation.
Observed, SyntheticFixture, Derived, Proposed and CapturedExecutable never merge.
Reuse universal `AccountObservation`, `DerivedAccount` and `ProgramBinary` where
they express the same claims; keep richer typed provenance where necessary.

Current worlds are “a composite of finalized read-only observations across a slot
range, not a validator bank”. Historical V1-reproduction proof is never inferred
for them. Proposed overlays and fixture minting stay visibly synthetic.

Bring conversion guarantees into migration, with their existing tests:

- Freeze discovery selection, then recapture the exact account set and Clock in
  at most three serial attempts with a monotonic `minContextSlot` chain. Failed
  coherence or source drift is Indeterminate; no replacement peer or approximate
  bank. A declared full-at-final-capture policy is distinct from opportunistically
  changing an amount after an outcome.
- Resolve non-wallet authority only through the bounded recorded plan. Exact
  DLMM PDA/vault proof establishes custody. Multisig decoding supplies no private
  approvals and never turns a protocol owner into a wallet signer.
- Preserve stress identity, discovery classification, shape and bucket across
  final-state rebinding; execute the same selected account at its exact final state.
- Keep observed search waves separate from derived local dimensions. Freeze exact
  accounts before capture, retain failed cases, and report each budget. Preserve
  migration's 64-probe/32-minimization limits, saved witnesses and offline replay.

The unsigned plan must re-execute serialized descriptors in a fresh VM. Keep
`UNSIGNED_PLAN_DIVERGENCE`, synthetic construction using real pinned token code,
the Token-2022 support matrix and build/schema identity information.

## 7. Gate, exits and language

Keep mechanism, funding, population and reconciliation as separate readiness axes.
Migration's typed blocking/warning invariant rule is per-kind policy. It does not
change MAIN's program-upgrade rule that severity never sets gate policy; document
both beside the `expected-changes.toml` review contract.

| Exit | Meaning / migration mapping |
| --- | --- |
| 0 | Passed; warnings still exit 0 under block-only |
| 1 | Gate failed: Blocked axis, violated blocking invariant, or strict violation of a declared requirement |
| 2 | Invalid configuration/input, unsupported encoding or fidelity failure before a compatible evaluable run exists |
| 3 | Existing stale-declaration contract; do not reuse STA's blocked exit 3 |
| 4 | Incompatible state/spec/bundle: binding, genesis or digest mismatch |
| 5 | Unevaluable: strict evidence gap prevents judgement |

Do not map a reason-code string alone: STA uses some violation-named codes for
Indeterminate invariants too. Classify typed causes. Under strict, a measured
frozen/mint-state restriction, exact output below minimum, insufficient reserve or
missing required authority violates a declared requirement and exits 1. An
unknown authority/destination, incomplete population, funding unverified,
unsupported semantics, exhausted rehearsal budget, or nothing executed solely
because evidence is missing exits 5. A real violation takes precedence over a
concurrent evidence gap. Do not turn “nothing executed” into evidence that no path
exists. Cases H/H-paused have decoded mint restrictions: strict exits 1; an
unrecognized-extension-only case is unevaluable, 5. Minimal strict exits 1.

Retain each STA gate outcome/reason code as analytical data; MAIN's typed CLI
status distinguishes failed from unevaluable. Existing governance-specific exits
remain unchanged. JSON aborts always emit structured errors with their code.

| Source wording | MAIN wording |
| --- | --- |
| preflight run | analysis |
| PASS / PASS WITH WARNINGS / BLOCKED | Passed / Passed with warnings / Failed |
| Proven | verified in the local VM (only with execution and exact reconciliation) |
| NotTested | Not evaluated |
| Unknown judgement | Cannot be judged; preserve the machine Unknown value |
| operator-supplied terms | Declared only |
| counterexample | counterexample, described as a witness within its recorded scope |
| stress cases / invariants | unchanged |

OfficialTransition remains NotTested and key possession Unknown. Use Eplyx,
British “Analyse”, sentence-case page headlines ending in a full stop, and MAIN's
banned-word tests. Never introduce asset-safety verdicts or risk scores.

## 8. One CLI, local store and lifecycle

One MAIN `eplyx` binary; no second lifecycle binary. Preserve MAIN's existing
compare/generate/reproduce/list meanings.

| Source command/capability | MAIN command |
| --- | --- |
| migration scaffold | `eplyx init --migration [--fixture]` |
| configuration check | `eplyx doctor` |
| preflight | `eplyx migration analyse` |
| search / gate / unsigned plan / fixture / reproduce | `eplyx migration search / gate / plan / fixture / reproduce` |
| change construction | `eplyx change token-migration --spec … --mechanism … [--program-id …] [--activation-slot\|--activation-unix …] --out …` |
| lifecycle construction | `eplyx change lifecycle …` |
| lifecycle snapshot, time views, counterfactual, notice, readiness, rollout | `eplyx lifecycle …` |
| current capture/replay and authority resolution | server library calls; `eplyx observe …` where useful locally |
| transfer, market exit and position checks | `eplyx path …` |
| runs / show / dashboard / login / logout / link / sync / version | same subcommands on MAIN's binary |

The hidden offline worker receives an empty environment and explicit immutable
inputs. Resolve paths before launch. Cloud commands alone read credentials.
Use `EPLYX_URL` for CLI hosted checks and sync, existing `EPLYX_TOKEN` and
`EPLYX_PROJECT_ID` meanings. Frontend's public `EPLYX_API_URL` build configuration
remains the browser API origin, not a second CLI cloud credential setting.

Validate `eplyx.toml` strictly: transition adapter, program, state, rehearsal,
invariants and gate. Reserve future configuration space without inventing a
program-upgrade configuration now. The unified layout is:

```text
.eplyx/
  bundle/                 # existing MAIN baseline bundle
  expected-changes.toml   # existing MAIN review declaration
  project.json
  runs/
  counterexamples/
  reproductions/
  cache/
  sync/
```

Lifecycle retains generic policy, immutable snapshots, entity/protocol separation,
exposure, deterministic time counterfactuals and notice normalization. The issuer
adapter is data behind the generic notice interface; resolvers contain no issuer
name conditionals. Bring only required readiness/rollout/resolution dependencies.
Keep all six path statuses; OfficialTransition, Redemption, market exit, Transfer
and Withdrawal never inherit evidence from one another. PreEvent is not Ready;
deadline passage alone is not Blocked; Unsupported is an evidence/executor boundary;
unknown eligibility stays unknown; no issuer/KYC blockers without mechanism
evidence. Ready/Blocked/Incomplete are policy findings, never asset judgements.

## 9. Current-state analysis and Meteora

Rebuild Node service functions in `eplyx-server`, with engine library evaluation:
catalogue, arbitrary validated mint, owner/selected-mint wallet scope, one account,
Transfer, bounded market exit, declared replacement scenarios, migration candidate
plan checks and stress over frozen samples. The browser submits terms only, never
code, instructions, metas, transaction bytes, paths, provider endpoints or statuses.

Observations use a dedicated server-only `EPLYX_OBSERVATION_RPC_URL`, bounded and
read-only, modelled on governance configuration. Observe outside runs; then pin
inputs and execute offline. MAIN's current `spawn_blocking` worker alone is not an
environment isolation boundary: add an empty-environment child process for these
new evaluations. Persist observation/evaluation stage transitions through P2's
durable run registry; do not create a parallel Node job store. Refresh is untested
until new evidence exists. No current run inherits historical demo facts, issuer
association or signing possession.

Choose a **separate current-state path-check interface** over universal evidence
and standard token decoders. Put DLMM meaning/layout/recognition in
`protocol::meteora_dlmm`; do not claim `ProtocolAdapter` upgrade-replay support.
The adapter count therefore remains five. Shared evidence owns pairing, boundaries
and generic deltas; no private token layout in DLMM code.

Preserve X-side discovery over at most 64 candidates, lexicographically first
compatible layout, exact captured deployed code, actual execution and minimum
output. PositionV2 `RemoveLiquidityByRange2` requires captured owner, pool, range
and shares plus token, withheld-fee, bin-supply and position-share reconciliation.
A vault is not LP ownership. Principal removal, fee claim, closure, market exit and
official conversion stay separate. No proof crosses positions/pools/ranges/
fractions/authorities/banks. Transfer establishes token movement only.

## 10. One hosted service and the proposed storage split

Use `eplyx-server`/axum and MAIN's filesystem registry/CAS as the source of truth
for immutable run/report/artefact/ChangeSpec/state-descriptor/search/witness/
reproduction bytes. P2 durable run records remain the queue. No second report
database or `eplyx-cloud` service.

**Explicitly owner-approved at T0:** Postgres stores mutable users (Argon2id), sessions,
device codes, workspaces, membership, hashed project credentials and project links.
It must not duplicate MAIN's authoritative project/run/evidence registry.
Mappings only connect workspace/authorization state to MAIN's `proj_…` IDs.
Migrations live in `server/migrations/`. Database-backed tests fail loudly when
`EPLYX_CLOUD_TEST_DATABASE_URL` is absent. The operator credential remains bootstrap
and admin; existing filesystem projects require explicit operator assignment to a
workspace. Do not guess ownership or rewrite old runs.

Avoid a cross-store partial-success window: use idempotent durable project-creation
intent and recovery; authorize only after the workspace link and registry project
agree. Immutable writes land in the CAS before their durable metadata reference.
Active bundle/baseline controls remain MAIN registry/operator operations. Postgres
identity data is not an alternate authority for analytical bytes.

Unify STA CI tokens with MAIN's project-scoped `eplyx_proj_…` capability, preserving
existing MAIN credential validity and revocation semantics. It can submit and sync
for its own opaque `proj_…` project, never change baseline/active bundle. Do not
introduce a second interchangeable credential namespace. Preserve STA's device
expiry/single-use/approval tests, session protections, member isolation and private
projects. Browser signup/login/device approval/settings become MAIN routes.

Program-upgrade `POST /v1/projects/{p}/checks` and the GitHub workflow contract stay
unchanged. Add kind-aware offline checks over uploaded state plus ChangeSpec;
do not require an upgrade bundle for a lifecycle-only analysis. Move cloud routes
under `/v1`, with MAIN `/health` and `/ready`, no duplicate health route.

Sync accepts exact bytes and SHA-256 for metadata, report, bindings, ChangeSpec,
state descriptor, search, witnesses and reproductions. Same content is idempotent;
different content conflicts. Synced results are never executed or promoted into
new evidence. Privacy scanning refuses leaks; it must not rewrite leaky text into
an apparently valid upload. No source/program/capture/config/credential/environment
or machine-path payloads. One project history labels hosted/local/ci sources.
Reuse engine dashboard projections from bytes for hosted and local views; browser
JavaScript performs no analysis.

Keep MAIN's API service, volume and Dockerfile approach. The owner must approve
and perform any infrastructure changes; no deployment is part of this work.

## 11. Frontend integration

Keep MAIN's public site and add a token-transitions section/route. Do not import
STA's landing/showcase/presentation or blue recolour. Preserve `/analyse`, projects,
runs, P3 impact and G1 governance functionality and tests.

Port STA's dashboard construction: shell/sidebar/crumbs/subnav; tiles, pills, key
values, meters and commands; eight migration answers in the same order; comparison
and witnesses; captured/synthetic/derived provenance. Add lifecycle/current views.
Use one app-wide `html[data-mode]` toggle with localStorage `eplyx-detail`,
`.tech-only`, `.ov-only` and MAIN `[data-technical]` sections.

Re-theme through role tokens only: MAIN ink `#0b0712`/`#171020`, violet
`#6d31f2`/`#9c68ff`, lavender `#e7ddff`, paper `#f7f5fa`, muted `#746d7d`;
positive fill/text `#2f936c`/`#5ec79b`, warning `#e58b3c`/`#edb77e`, critical
`#e7555d`/`#ef6870`, unknown `#8d85a0`, pending `#b28cff`. Panels derive from the
violet/ink surface; replace literal tints and gradients with `color-mix()`.
Enforce no dashboard colour literals outside `:root`. Keep one `TONES` mapping.
Embed licensed DM Sans and Manrope woff2 assets so loopback fetches nothing.
Use MAIN `brand.js`/`logo.svg`; local subtitle “Local analysis”, hosted workspace
and project. Update all Rust embedded-asset paths.

Port dashboard and cloud browser assertions, add MAIN node assertions and apply
existing banned-copy regexes to every page. Regenerate only the new dashboard
fixture from MAIN's engine. Playwright accepts `EPLYX_CHROME`. Update CLAUDE and
other changed scope statements in the same implementation commit introducing the
UI/dashboard, rather than waiting until T10.

## 12. Verification, limitations and next phases

| Check | Observed result |
| --- | --- |
| MAIN `make fmt-check` | Passed |
| MAIN `make lint` | Passed after restoring an available offline dependency cache |
| MAIN `make test` | Passed: 891 test passes; one ignored developer export utility |
| MAIN `cargo test --no-fail-fast` | Passed: 853 test passes; same ignored utility |
| MAIN `pnpm check:frontend` | Passed, including node assertion tests |
| MAIN `pnpm verify:report` | Passed |
| MAIN `pnpm verify:governance` | Passed |
| STA `make fmt-check`, `make lint` | Passed with original source; additional defect-feature lint caveat below |
| STA `./scripts/test-programs.sh` | Passed |
| STA workspace `cargo test --locked --workspace --no-fail-fast` | 612 passed, 20 setup failures, 13 ignored; all setup failures subsequently passed on explicit retries |
| STA loopback/Git retry | All 36 selected tests passed |
| STA missing generated-fixture retries | Both failed preservation/generator tests passed after scratch `make fixtures` |
| STA cloud `cargo test --locked -p eplyx-cloud -- --include-ignored` | All 14 passed, including 12 database integration tests |
| STA `npm run test:service` | All 51 passed on loopback retry |
| STA `npm run test:dashboard` | All 15 passed |
| STA `npm run test:cloud` | Four passed; three copy failures below |
| STA selected mock analysis Playwright specs | Ten passed; seven opt-in live tests not run |
| Both migration defect features, host unit tests | Passed |
| Deadline-defect feature clippy | Passed |
| Fee-defect feature clippy | Unused-constant failure on source; scratch conditional-compilation fix passed |
| Minimal built CLI | Reference exit 0, strict exit 3, deadline defect exit 3, search and offline reproduction exit 0 (STA codes) |
| Upgradeable-loader experiment | 30 integration and 17 unit tests passed; no measured semantic differences |

Counts are executions, not unique test totals across repeated runs. Commands,
per-test Rust results, retries and log digests are in the verification record.
The original full STA command remains recorded as failed; retries do not turn it
into a claimed clean single invocation. Required MAIN corpus, fixture-generator,
frozen ChangeSpec, replay, hosted/local report parity and numeric-scan assertions
passed as part of MAIN's suites. No MAIN production source changed in T0.

Node was 22.23.1. MAIN frontend checks used the available pnpm 11.19.0 fallback
against existing dependencies; the requested 11.24.0 was unavailable locally.
That initial pending check is now satisfied by section 13. Initial wrapper/cache/loopback
failures are recorded rather than counted as product regressions. Playwright used
cached Chromium through scratch-only `EPLYX_CHROME` support.

STA cloud copy failures, unchanged in the reference:

| Test | Expected text | Rendered text |
| --- | --- | --- |
| `cloud.spec.js:28` | Cloud sync is optional. Eplyx execution stays local. | Cloud sync is optional. CLI execution stays local. |
| `cloud.spec.js:36` | Viewing them does not run RPC, execution or replay | This page shows engine results from a developer machine or CI. It does not run RPC, execution or replay. |
| `cloud.spec.js:48` | Search domains differ — counterexample disappearance does not prove resolution. | Search domains differ. A missing counterexample does not prove resolution. |

The second and third actual cells quote the relevant sentence; the rendered pages
also contain surrounding provenance/difference text. T9 must preserve the full
privacy and comparison assertions while adapting approved copy to MAIN.

No live provider run was authorized. Current-state tests used mock providers and
frozen captures. Seven opt-in live browser tests were not run; the runner reported
them as skipped because their live flags were unset. No missing-artefact test was
waived. MAIN's ignored bundle-export utility and STA's opt-in performance benchmark
are identified in the verification record. Future MAIN migration/lifecycle
determinism and hosted parity tests are pending their implementations, not passed.

No transaction was built for, signed for, simulated against or sent to a live
cluster. Tests use local VMs and public-label fixture keys. No real private keys
were used; the private STA provider file was not opened. No secrets were committed,
no historical evidence was rewritten, no push/deploy/infrastructure change was
made, and `main` was not moved. Local database and browser test services are
temporary test infrastructure only.

| Phase | State / required result |
| --- | --- |
| T0 | Architecture approved; reference corrections and exact pnpm checks verified; source limitations accepted |
| T1 | Authorized; multi-kind plumbing, unchanged upgrade bytes and ID |
| T2 | Pending; token/RPC/digest/executor primitives and real-layout tests |
| T3 | Pending checkpoint; migration core, candidates, exact reference comparison |
| T4 | Pending; coherent recapture, authority, rebinding and observed search |
| T5 | Pending; single CLI, store, exit mapping, isolated worker and reproduction |
| T6 | Pending checkpoint; lifecycle kind and its frozen contract |
| T7 | Pending; current-state Meteora and position path checks |
| T8 | Pending; local dashboard and same-commit scope corrections |
| T9 | Pending checkpoint; approved identity storage, sync and hosted analysis/UI |
| T10 | Pending; guides/archive pointer; release packaging only after asking |

The next milestone is T1; no further architecture checkpoint is required. After the migration,
consider an explicitly authorized read-only provider analysis, partial/claim-based
migrations and a local-validator unsigned-plan check. None is inferred authorized
by this ADR.

## 13. Owner-review addendum: pre-T1 checks

The owner approved the filesystem/CAS authority and mutable-identity-only
Postgres split, MAIN's upgradeable loader, and the frozen STA fee-defect clippy
limitation. T3 must rerun the complete frozen migration contract on the actual
port and apply only the constant's conditional-compilation hygiene fix.
Live-provider checks remain intentionally not run.

The [review record](examples/phase-t0-stock-transition-integration/owner-review.json)
contains complete expected/rendered text, source file hashes, test file hashes,
individual test-body hashes, rationale and command receipts for each correction.
The [three-line assertion patch](examples/phase-t0-stock-transition-integration/cloud-copy-corrections.patch)
was applied only in scratch and then restored. The original reference values and
original failed-test receipts remain intact; this addendum supersedes the three
accepted assertion strings when porting their tests.

| Assertion | Classification | Semantic assessment |
| --- | --- | --- |
| Cloud landing headline, `cloud.spec.js:30` | Stale copy only | “CLI” explicitly names the same local/CI execution described by the surrounding optional-sync workflow. This is not a global assertion that all hosted Eplyx analysis runs on the caller's machine. Optional sync and private-project access assertions remain intact. |
| Synced banner, `cloud.spec.js:39` | Stale copy only | Both deny RPC, execution and replay when viewing synced results. Rendered copy also names developer-machine/CI provenance. |
| Search warning, `cloud.spec.js:51` | Stale copy only | Both deny that a missing witness proves resolution across different search domains. The reason text and assertions forbidding “Resolved (equivalent search)” and “fixed” remain intact. |

The minimal stress headline is now explicitly **20/20 stress cases behaved as
specified**, with 11 reconciled migrations and nine correct, rollback-verified
rejections. Its separate population rehearsal remains four of four attempted
holders migrated. No STA behaviour or raw reference outcome changed.

MAIN pins `pnpm@11.24.0`. That exact official package was acquired into scratch,
integrity checked, and its `--version` returned `11.24.0`. With Node 22.23.1,
`pnpm check:frontend`, `pnpm verify:report` and `pnpm verify:governance` all passed.
The report check retained 141 fixtures, 89 identical, 52 changed, 11 critical and
$6,182,370 collateral represented. No lockfile or installed project dependency
was changed. All seven cloud browser tests passed under pnpm 11.24.0 with only
the accepted text assertions corrected. An initial database-port setup failure
and its successful retry are recorded separately.

These results meet the owner's condition to begin T1. They do not waive T3's
port verification or turn the untouched STA source's copy/lint failures into
passing source results.


### T6 encoding comparison

The lifecycle port retains original frozen bytes separately from MAIN reference
encodings. `fixtures/lifecycle/encoding-projection.json` lists every original and
MAIN file hash and byte count, with the preimages for changed internal digests.
Only decimal-string integers, exact Scaled UI multiplier bit fields, the approved
MAIN exit mapping and hashes over those representations change. The projector
cannot execute an evaluator or rewrite an expected outcome. See
`docs/phase-t6-test-mapping.json` for assertion-level adaptations and archived scope.
T0's recorded source values and inventories are not overwritten.

`docs/phase-t6-identity-reference.json` records the baseline and notice-derived
lifecycle ChangeSpec IDs and exact MAIN identifying tuples beside their source
scenario hashes. Display metadata remains outside those tuples.
