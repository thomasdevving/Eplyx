# Phase C1 — minimal ChangeSpec architecture

## Decision

Eplyx now has an explicit input contract for **what proposed change it is
evaluating**. [`change.rs`](../engine/src/change.rs) defines `ChangeSpec`,
which has one implemented kind, `program_upgrade`. Every `eplyx ci check` goes
through the same path:

```text
ChangeSpec → bind to the target the bundle proves → resolve candidate bytes by hash → execute
```

`--candidate <ELF>` still works. It stands for the minimal spec: upgrade the
bundle's program to exactly these bytes. Every CI report now names the change
it is a verdict about. Nothing about the baseline world moved: bundle
identities, replay proofs, semantic bindings and verdicts are unchanged on
every frozen bundle. The pilot report is byte-identical to its pre-C1 bytes
once the new `change` object is removed.

## 1. Inventory: where the change was implicit

| Current input | What it really represents | Where identity is lost |
| --- | --- | --- |
| `ci check --candidate <ELF>` → `ci::check` | "Upgrade the bundle's program to these bytes" | The target is never stated; it is whatever program the bundle measures. The report carries only `candidate {sha256, len}`, so nothing says which program the bytes were proposed for, or that the bytes tested were the bytes proposed. |
| `pipeline::execute(record, resolved, &[u8])` (schema 2) | Splice candidate bytes into the target's ProgramData at the PRE boundary | Raw bytes with no identity; `candidate == baseline_elf` is a byte comparison. |
| `replay::compare_with_dependencies(v1, v2)` (schema 1, `compare --corpus`) | The same upgrade, over the Phase 7–9 corpus | `ProgramVersion { label, bytes }`. `ReplayReport` names the candidate by its **label** (`"v2"`) and carries no candidate hash at all. |
| `compare` (synthetic fixture corpus) | V1→V2 of the fixture protocol | `Report` records artefact **paths** as strings. |
| `bundle build --baseline v1.so` | The baseline world, not a change | Correctly content-addressed (`baseline_program_sha256`); nothing to fix. |
| Server `POST /checks` multipart `candidate` → `candidate.so` → worker `ci::check` | "Upgrade this project's active-bundle program to these bytes" | The run record stores `candidate_sha256` only. The target is implied by whichever bundle was active when the run was created. |
| Frontend analyse/report pages | Upload `candidate.so`; show "Candidate SHA" | Same as the server; there is no notion of a target. |
| Eplyx Stock transition package (`conversion/package.rs`, separate repo) | Asset lifecycle transition **plus** a program artefact | Has an identity (`transition_package_sha256`), but it mixes the candidate program (`candidate_program {program_id, artifact, sha256}`) with asset terms and config file bytes, and there is no kind boundary. |
| Fixture candidates (`artifacts/*.so`, `programs/fixture-*-candidate`) | Locally constructed counterexample upgrades | Tests refer to them by filename. |

## 2. Model

```text
ChangeSpec
├── schema_version        1
├── change_spec_id        optional in a file; recomputed and must agree
├── change                identifying; internally tagged by `kind`
│   └── kind: program_upgrade
│       ├── target        ProgramTarget { program_id, programdata_address? }
│       ├── candidate     ExecutableArtifact { sha256, len }
│       ├── replaces?     ExecutableArtifact: the executable being replaced
│       └── expected_upgrade_authority?
├── activation?           identifying; { slot?, unix_timestamp? }
└── metadata?             NOT identifying; { label?, source? }
```

- **The target lives inside the kind, not at the top level.** A program upgrade
  targets a program; a lifecycle change targets an asset. Putting
  `ProgramTarget` at the top level would force every future kind to carry, or
  pretend to carry, a program.
- **Target and candidate are separate objects.** A buffer, a Squads proposal
  and a staged deploy can all name one target with different artefacts, and
  one artefact can be proposed for different targets.
- **No results, findings, proof strategy, protocol fields or baseline state.**
  `replaces` is a stated precondition ("this proposal replaces X"), checked
  against the bundle. It is not a copy of the baseline world.
- **Unknown fields and unknown kinds are refused** (`deny_unknown_fields`). If
  a misspelled `expected_upgrade_authorty` were silently dropped, the spec's
  identity would no longer match what its author wrote.

## 3. Identity rules

```text
change_spec_id = sha256( JSON( ["eplyx-change-spec-v1", schema_version, change, activation] ) )
```

The JSON is compact serde output: struct fields in declaration order, no maps,
absent optionals omitted, `activation` as `null` when absent. An independent
Python recomputation reproduces the pinned test ID, so the encoding can be
implemented outside Rust.

| Field | In identity | Why |
| --- | :---: | --- |
| `schema_version`, `kind` | yes | A different contract or kind is a different proposal |
| `target.program_id`, `target.programdata_address` | yes | Which on-chain identity changes |
| `candidate.sha256`, `candidate.len` | yes | What would be executed |
| `replaces`, `expected_upgrade_authority` | yes | Preconditions (parameters) of the proposal |
| `activation.slot`, `activation.unix_timestamp` | yes | When it takes effect |
| `metadata.label`, `metadata.source` | **no** | Renaming a proposal must not make it a different proposal |
| `change_spec_id` | — | A commitment, recomputed and compared, never trusted |

Canonicalization: addresses must be canonical base58, and hashes must be
lowercase hex. Otherwise one proposal could have two spellings and two IDs.
New optional identifying fields must be skipped when absent, so that adding one
later does not re-identify existing specs. `a_frozen_spec_keeps_its_identity`
pins one ID, so any change to the encoding shows up as a failing test.

## 4. ProgramUpgrade

Binding a spec to a bundle (`ChangeSpec::bind`) checks every expectation the
spec states, against the bundle's own evidence:

| Stated in the spec | Proved from (schema 2) | Schema 1 | Mismatch |
| --- | --- | --- | --- |
| `target.program_id` (always) | manifest `program_id` | manifest `program_id` | exit 4 |
| (always) upgradeable loader | each observation's target binary `loader` | dependency-manifest loader, where recorded | exit 4 if proven non-upgradeable |
| `programdata_address` | each observation's `programdata_address` (must agree) | not carried → **refused** | exit 4 |
| `replaces` | `baseline_program_sha256` + length | the same | exit 4 |
| `expected_upgrade_authority` | every observation's authority at its slot; a window across an authority change satisfies neither | not carried → **refused** | exit 4 |

Evidence the bundle does not carry never satisfies an expectation the spec
states. An expectation the spec does not state is not checked.

## 5. Evidence binding

The spec names bytes; it never contains them. `ChangeSpec::resolve` is the
**only** constructor of `ResolvedCandidate`, and execution accepts only a
`ResolvedCandidate`. The candidate sources are:

- `Bytes`: the implicit `--candidate` path.
- `File`: `--change-spec` together with `--candidate`.
- `Store`: `--change-spec` together with `--artifacts DIR`, the existing
  `EvidenceStore` layout at `programs/<sha256>`. The store's own
  hash-on-read check applies.

Each source is verified against `candidate {sha256, len}`. A missing object, a
tampered object, or different bytes all fail with exit 2, and nothing executes.

## 6. Candidate-resolution flow

```text
open + verify bundle (schema 1 or 2)             exit 4 on failure
BaselineTarget from the bundle's own evidence
input: --candidate  → ChangeSpec::program_upgrade(bundle program, bytes)
       --change-spec → ChangeSpec::load + validate          exit 2
spec.bind(BaselineTarget)                         exit 4
spec.resolve(source) → ResolvedCandidate          exit 2
check_v1 / check_v2(bundle, ResolvedCandidate)    unchanged replay, review, verdict
report.change = binding
```

`ci::check(bundle, path, expectations)` is a thin wrapper over
`ci::check_change(bundle, &ChangeInput::Candidate(path), expectations)`. The
server and every existing caller are unchanged.

## 7. CLI compatibility

```bash
eplyx ci check --bundle B --candidate candidate.so                       # unchanged
eplyx change program-upgrade --program <ID> --candidate candidate.so \
  [--programdata A] [--replaces current.so] [--upgrade-authority K] \
  [--activation-slot S] [--activation-unix-timestamp T] [--label L] \
  [--store DIR] [--out change.json]
eplyx ci check --bundle B --change-spec change.json --artifacts DIR
eplyx ci check --bundle B --change-spec change.json --candidate candidate.so
```

When both `--change-spec` and `--candidate` are given, the spec is
authoritative and the file only supplies bytes. The bytes must be exactly the
spec's candidate, otherwise the check exits 2 before anything executes.
`--artifacts` requires `--change-spec` and conflicts with `--candidate`. The
text report prints a `Change:` line with the ID, kind and target.

## 8. Report and bundle binding

`CiReport` gains one object, after `candidate`:

```json
"change": {
  "change_spec_id": "…",
  "kind": "program_upgrade",
  "target_program_id": "SPoo1Ku8…",
  "candidate_sha256": "ec2dfef…"
}
```

It is `Option` only so that reports written before C1 still deserialize.
Activation and metadata are deliberately left out of the report.

Bundle binding is structural: the bytes executed are the bytes
`ResolvedCandidate` verified against the spec, and the spec was bound to the
bundle's target before resolution. No bundle file changes. `bundle.json` is
byte-identical before and after a check, and `bundle.sha256` and
`replay_proof` are identical under two different specs.

The pilot's canonical report changes from `7be26a66…` to `cb0e9d12…`. With
`change` removed it is still exactly `7be26a66…`, so the Phase U1 guard now
asserts both.

## 9. Tests

In `change.rs` (unit tests) and [`engine/tests/change_spec.rs`](../engine/tests/change_spec.rs):

| # | Property | Test |
| ---: | --- | --- |
| 1 | same semantic spec → same ID (round trip, with or without stated id) | `the_same_semantic_spec_has_the_same_id` |
| 2, 3, 5 | candidate bytes / length, target program / ProgramData, replaced executable, authority, activation slot / time, schema each move the ID | `every_semantic_field_moves_the_id` |
| 4 | label / source do not move the ID and survive round trip | `cosmetic_metadata_does_not_move_the_id` |
| — | encoding frozen | `a_frozen_spec_keeps_its_identity` |
| — | stated id disagreeing, unknown fields, unknown kind, non-canonical encodings refused | `a_stated_id_that_disagrees_is_refused`, `unknown_fields_and_kinds_are_refused`, `malformed_identities_are_refused` |
| 5 | missing store object, and no byte source at all → exit 2 | `candidate_evidence_must_exist_and_match_its_address` |
| 6 | tampered store object → exit 2 | same |
| 7 | spec for another program → exit 4 | `a_spec_for_another_program_is_refused` |
| 8 | spec A + `--candidate` B → exit 2, library and binary, JSON error, no findings | `a_spec_for_one_candidate_never_executes_another`, `the_cli_binds_and_refuses_through_its_exit_codes` |
| 9 | `--candidate` ≡ spec by file ≡ spec by store: identical report bytes | `the_candidate_flag_is_the_minimal_program_upgrade` |
| 10–12 | Drift U14 (exit 0), Orca U12 (exit 0), replay-only U11.2 (exit 2) keep their verdicts and name their change | `frozen_schema_two_controls_keep_their_verdicts` |
| 13 | spec leaves bundle identity, replay proof, semantic binding, summary and `bundle.json` untouched | `a_change_spec_does_not_touch_baseline_proof_identity` |
| 14 | the top level is kind-neutral | `the_top_level_is_kind_neutral` |
| — | stated ProgramData / authority / replaced executable proved from schema-2 evidence; unprovable on schema 1 | `stated_target_expectations_are_proved_from_bundle_evidence`, `binding_checks_every_stated_expectation` |

The existing controls also still pass: the pilot baseline (exit 0), the
stake-pool known regression (exit 1, same two fingerprints), the U13.3 sequence
bundle through the binary (exit 2), and the Drift / Orca / semantic-binding
suites.

## 10. Governance compatibility

No governance fields were added. The model leaves room for them:

- A future `delivery` field inside `program_upgrade` (a buffer address, a
  Squads proposal and transaction index, or the SHA-256 of a serialized
  message) would be an **identifying** optional field, skipped when absent. It
  can be added without re-identifying existing specs.
- The target is already separate from the artefact, and the artefact is
  content-addressed, so "buffer B holds bytes H" becomes a binding check
  ("buffer bytes hash to `candidate.sha256`"), not a new identity.
- The invariant `PROPOSAL BEING SIGNED == CHANGE EPLYX ANALYSED` then reduces
  to comparing the governance message's derived spec ID with the report's
  `change.change_spec_id`.

## 11. LifecycleChange representability

The Eplyx Stock lifecycle inputs have the following fields: asset mint,
`effective_at`, eligibility inputs, a numerator/denominator ratio with rounding
and fee, a destination mint, and an optional deadline. They map onto this model
without touching the top level:

```text
change.kind = "lifecycle_change"
change.asset          { mint }            ← the target, inside the kind
change.destination    { mint }
change.ratio          { numerator, denominator, rounding, fee_bps }
change.eligibility    …policy…
change.deadline?      …
activation.unix_timestamp                  ← effective_at, the generic top level
```

That mapping is why the target is per-kind and `activation` carries only the
generic "when it takes effect". The claim deadline is lifecycle-specific and
belongs to the kind. The Stock package bundles a candidate program together
with asset terms; in this model that would be two changes, or a composite kind
later, not one flat record. Nothing was implemented. Today a `lifecycle_change`
document is refused as an unknown kind.

## 12. Frozen regression controls

| Control | Result |
| --- | --- |
| Pilot schema-1 bundle, baseline | exit 0; report minus `change` = `7be26a66…` (unchanged) |
| Stake-pool known regression candidate | exit 1; same two fingerprints |
| U14 Drift semantic bundle | exit 0; bundle, proof and binding unchanged |
| U12 Orca semantic bundle | exit 0; coverage 4, no findings |
| U11.2 / U13.3 replay-only bundles | exit 2 `no_semantic_coverage`; contract 2 / 3 proofs matched |

## Known gaps, deliberately left

- `compare --corpus` (`ReplayReport`) and the synthetic `compare` still name
  the candidate by label or path. They are developer diff paths with no bundle
  target to bind against. They are the next place to thread a `ChangeBinding`.
- ~~The hosted API accepts only the multipart `candidate`.~~ Closed by
  [Phase P1](phase-p1-productized-changespec.md): runs carry and are indexed by
  `change_spec_id`, a `change_spec` part is accepted, and the frontend shows the
  change.
- `activation` is identity only. The gate evaluates a candidate
  counterfactually over historical state and does not condition on activation.

## 13. Recommended next step

The next step should be **API/registry adoption of ChangeSpec**, not a new
kind: `POST /checks` accepting a `change_spec` alongside, or instead of, the
bytes, runs indexed by `change_spec_id`, and the frontend showing target and
change ID. That turns the identity into a product surface, which is what
governance binding needs first. Of the new kinds, `GovernanceExecution` (the
delivery binding in §10) is the right one to add after that, because it reuses
this evaluator unchanged. `ParameterChange` and `LifecycleChange` each need a
new evaluator.
