# T3 — Token migration core

T3 ports the Token Migration V1 evaluator onto MAIN’s ChangeSpec, content-addressed
candidate resolution, shared decoders and upgradeable-loader execution model.
The original STA source remains the behavioral reference at
`ad1897e86ee05ff2255bd5277518a3230ee2340f`.

The evaluator is available as a library. CLI/local storage arrives in T5 and hosted
workers in T9. The fixed-ratio conversion product remains archived in STA.

## Proposal and state

`Change::TokenMigration` owns source/destination token identities, conversion terms,
eligibility, source disposition, destination funding, authorities, deadline and
mechanism. Activation is MAIN’s top-level field. Terms use canonical base58 and
snake_case, and monetary quantities remain canonical decimal strings. Execution
sets the rehearsal Clock according to the migration policy; activation remains
inclusive and deadline exclusive. Upgrade identity and JSON encodings are unchanged.

`migration::input::assemble` creates `change.json`, `state.json`, a mechanism at
`programs/<sha256>`, and optional state bytes. The state descriptor contains the
clock policy, bounds and declared invariants. Its source is a fixture recipe,
a hash-bound captured world, or a bounded capture intent. Capture transcripts are
frozen and hash-bound in run bindings before execution. No combined STA schema-3
manifest is accepted as a MAIN proposal.

`ValidatedInput` is an unserialized runtime aggregation with private fields.
`ChangeSpec::resolve` constructs its executable candidate after hash/length checks.
Rehearsal inputs can only be constructed from a validated input. Input identity
binds the ChangeSpec ID and state descriptor; cosmetic metadata stays outside it.
Changing declared invariants changes analysis input identity, not proposal identity.

Run IDs and timestamps reside in bindings, outside `report.json`. Identical minimal
runs compare the report, markdown, plan, stress matrix/results, rehearsal and unsigned
plan byte-for-byte. Search replay recomputes its evidence. A search gate binds the
analysis input, candidate, world, plan and stress-plan digests rather than a timestamp.

## Execution and interpretation

The migrated candidate uses MAIN’s upgradeable loader. Its three SBF hashes match
STA exactly: reference `e5db6948…`, deadline defect `c4346736…`, fee defect `a93aff2f…`.
Build scripts explicitly use platform-tools v1.57 / SBF v3 for these candidates and
v1.54 / SBF v0 for MAIN’s lending fixture. Missing or mismatched candidates fail.
The sole candidate source change is the approved feature guard on the unused test
vectors constant; defect behavior is unchanged.

RPC-shaped token views call `standard_programs::{spl_token,token2022}`. Unknown or
malformed extensions remain visible and migration policy excludes them from
execution. Clock, loader headers, multisig state and proposed token-account packing
also live in shared modules. Reconciliation propagates decoding failures rather
than turning malformed data into zero balances. Public and withheld balances are
separate measurements. New serialized 64-bit values use decimal strings and accept
legacy numeric capture fields on input. Existing upgrade encodings remain frozen.

The migration gate preserves four readiness axes and all source invariant statuses.
A typed requirement-violation projection distinguishes strict failures (exit 1)
from evidence gaps (exit 5). Violated blocking invariants and blocked axes also
exit 1. Block-only warnings exit 0. This is migration’s per-kind policy; upgrade
severity and expected-change policy are unchanged.

Provenance keeps Observed, SyntheticFixture, Derived, Proposed and CapturedExecutable
separate. Current observations remain a composite across finalized context slots,
not a validator bank or historical replay proof. Unsigned plans contain descriptors,
not signed transactions, and are cross-checked in another fresh local VM.

## Frozen contract

`migration_cases`, `migration_demo` and `migration_reference` compare every saved
T0 analytical field for 14 cases: populations, classes, gates under both policies,
readiness, invariant statuses and explanations, reconciliation equations, coverage,
stress verdicts, unsigned cross-check counts and complete searches where T0 recorded
them. The comparator normalizes only approved identities, their derived addresses,
run metadata, evidence references and exact slot-string encoding.

The minimal example uses its original nine invariant declarations, not the broader
14-invariant recommended set. Reference stress means **20/20 cases behaved as
specified** (11 expected migrations and nine expected rejections). Its deadline
candidate produces one derived counterexample, reproduced offline. SPACEX retains
17,870 accounts, 10,091 positive, the seven frozen class totals, eight migrated units,
blocked funding with `INSUFFICIENT_RESERVE`, and all seven reconciliation equations.

World/spec/plan/report/search identities change because MAIN’s identifying proposal
includes the executable, field encodings follow MAIN, and capture provenance paths
point to fixture directories. Configuration and authority PDAs are seeded by the
spec identity; their changed spellings in diagnostics are recomputed by the test.
Captured account bytes, candidate hashes and analytical expectations do not change.
The source SPACEX world ID remains in T0; MAIN’s world ID is
`99491370b6381a4a21eee5b1aa07cb1a9ae88e19de7ac86f292930f783ec7dc6`.

Four raw captures contain historical provider-origin fields. The absolute no-URL
rule takes precedence: those files remain byte-identical local fixtures outside
Git, imported with `scripts/import-migration-fixtures.py --sta <archive-checkout>`.
Their provenance, hashes and sizes are tracked. The importer reads an explicit
allowlist and never opens provider configuration. Tests fail if captures are missing.
This means a fresh clone needs the pinned STA archive once to import these bytes.

Verification passed: `make fmt-check`, `make lint`, `make test` (964 passed, two
existing/source ignores), `cargo test --no-fail-fast`, and the frontend, report and
governance checks with pnpm 11.24.0. The [verification record](phase-t3-verification.json)
retains each command, failures and successful retries, execution counts and log hashes.
The [MAIN comparison record](phase-t3-migration-reference.json) appends all 14 actual
results without changing T0. Live-provider checks remain intentionally not run.
Capture tests use mock providers only. Dashboard and hosted browser tests follow
when those capabilities are ported.
