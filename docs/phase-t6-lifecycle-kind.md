# T6 — Lifecycle declarations and assurance

MAIN now represents declared lifecycle terms as `lifecycle_change`. Its asset,
optional successor, ratio, eligibility, deadline, before/after policy and field-level
source assertions participate in MAIN's ChangeSpec identity. Activation remains a
top-level whole-second timestamp. Display metadata and local artifact paths do not
identify the change. Absent ratio and eligibility stay unknown.

[The identity reference](phase-t6-identity-reference.json) records the source scenario
hashes, both MAIN proposal IDs and their exact identifying preimages.

A lifecycle declaration has no executable, program-upgrade target or delivery.
The existing executable resolver refuses it; the evaluator binds the exact declared
policy and source facts before comparing lifecycle consequences. Issuer notices
remain sources of assertions. They do not establish eligibility, entitlement,
official conversion, legal identity or execution. The asset-specific notice parser
lives behind the generic notice interface.

## Evaluation and commands

The policy evaluator keeps immutable chain observations separate from declared
policy, account consequences and protocol observations. Counterfactual times reuse
one pinned world. PreEvent is not Ready; crossing a deadline alone does not imply
Blocked. Unsupported records an evidence or executor boundary. Transfer, market
exit, withdrawal, redemption and official transition cannot inherit one another's
proof. Principal removal does not prove fee collection or position closure.

`eplyx change lifecycle --scenario scenario.json --out change.json` constructs the
proposal. `--format json` emits MAIN's structured result. The single `eplyx` binary
also exposes these `lifecycle` subcommands:

- `analyse --snapshot … --scenario … --at … [--before …] [--change-spec …]`
- `snapshot --asset … --out …`
- `compare-scenarios --snapshot … --scenario … --readiness-policy …`
- `ingest-notice`, `scenario-from-event`, `preflight-from-notice`
- `readiness`, `resolve-paths`, `evaluate-rollout`, `guard-rollout`, `demo-rollout`

Inputs are explicit; no issuer fixture is a product default. Analytical commands
accept `--format json` and create outputs exclusively. Every offline command runs
in an `env_clear` worker which rejects inherited variables. The parent supplies
its non-secret temporary root as explicit transport metadata for guarded markers;
this preserves the temporary-only restriction when macOS defaults change after
clearing the environment. The read-only snapshot
parent uses MAIN's RPC client; no provider was contacted during this phase.
Analysis completion is distinct from readiness authorization. Ready exits 0,
Blocked 1 and Incomplete 5; invalid inputs exit 2. Guarded demonstrations can write
only their explicit local marker. They have no cluster transaction path.

## Shared architecture and reference integrity

MAIN's token decoders, Clock, loader decoding, RPC observations, authority evidence,
hashing and byte-range measurements supply the common mechanisms. A narrow token
support facade applies lifecycle support boundaries without copying private token
layouts. Protocol-specific frozen swap/position replay lives under
`protocol::meteora_dlmm`; the historical adapter count remains five. T7 completes
the current-state path interface and its discovery/capture surface.

Reusable frozen selection and coverage validation are retained only to verify
assurance dependencies. STA's Phase 6/7 population investigation products and their
CLI remain archived. The 15 applicable coverage assertions remain; the old standalone
`coverage` CLI test stays archived. Output immutability, portability and canonical
bytes are tested through the actual lifecycle commands.

Deserializable execution records are untrusted report DTOs. They cannot construct
sealed execution/withdrawal proofs. Resolution replays captured deployed bytes and
compares the complete execution result before admitting a path proof, including
when an attacker has rehashed a fabricated result and its index. Readiness of
historical records is relative to the explicitly supplied trusted digest manifest;
that manifest is not a signature or independent chain-inclusion attestation.

Local artifact reads have a 128 MiB ceiling checked before allocation. Nested
references must remain relative inside the explicit package root and cannot
traverse symlinks. Tests use independent scratch packages for tampering, preserving
both original and derived references.

[Fixture provenance](../fixtures/lifecycle/README.md) records the exact originals.
[The encoding projection](../fixtures/lifecycle/encoding-projection.json) enumerates
all 62 changed file digests and six changed internal digests, with their preimages.
The import holds 92 exact originals (371,438,848 bytes); MAIN derivatives total
372,229,380 bytes. Payloads are ignored, with only provenance and scripts committed. The generator performs
only representation changes and the approved exit-code mapping; it cannot derive
expected analytical findings. Original raw captures and binaries stay unchanged.
Wide integers serialize as decimal strings. Scaled UI multipliers retain exact
bit patterns without floating-point analytical arithmetic.

## Verification

The [source mapping](phase-t6-test-mapping.json) accounts for the frozen assertions
and the archived CLI boundary. The [verification record](phase-t6-verification.json)
records actual attempts, failures and retries. `make test` passed 1,226 tests and
the separate no-fail-fast workspace run passed 1,175; both had zero failures and
two existing ignores. Formatting, lint, frontend, report, governance, exact fixture
import and accepted projection checks passed. All 1,633 inventoried STA files
remain unchanged and its working tree is clean. No live-provider check, cluster
signing, simulation, submission, deployment or push is authorized or performed.
