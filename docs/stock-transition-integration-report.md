# Stock transition integration report

The migration integrates Token Migration V1, lifecycle assurance, current-state
path checks, local views and hosted identity/analysis into MAIN. The original
program-upgrade contracts remain in place. STA remains pinned and untouched.

## Phase commits

| Phase | Commit | Result |
| --- | --- | --- |
| Adapter correction | `73a1c9d` | MAIN's existing adapter count corrected to five |
| T0 reference | `4220968` | Frozen STA values, assertion contract, fixture inventory and ADR |
| T0 approval corrections | `39782b7` | Approved decisions, copy-only browser corrections and exact pnpm checks |
| T1 | `82aa1e9` | Kind-aware proposal/binding/index interfaces; upgrade identity unchanged |
| T2 | `beea202` | Shared token, RPC, canonical/digest and execution primitives |
| T3 | `a4f8358` | Token migration, reference/defect candidates and 14-case comparison |
| T4 | `0f00eab` | Coherence, non-wallet authority, final rebinding and observed waves |
| T5 | `8d491a4` | Migration CLI, immutable local store, offline search/gate/reproduction |
| T6 | `2ee9a93` | Lifecycle ChangeSpec, notices, consequences and assurance |
| T7 | `580724d` | Current Transfer, Meteora exit and position-withdrawal checks |
| T8 | `87573a4` | Local dashboard, shared presentation and public transition route |
| T9 | `351ccae` | Identity/sync, durable offline hosted kinds and current-state frontend |
| T10 | This documentation commit | Product guides, scope, archive and final verification |

## Applied architecture

MAIN's filesystem registry and CAS are authoritative for immutable analytical
bytes and durable runs. Postgres holds only mutable identity and authorization.
Project creation intents recover the two stores without inventing a second project
registry or restoring revoked membership. Legacy ownership is an operator action.

MAIN's upgradeable-loader model executes migrated candidates. There is no second
fixed-ratio product, proposal resolver, token decoder, protocol evidence layer or
Node analysis service. Five `ProtocolAdapter` implementations remain; the new path
checks reuse universal evidence without claiming historical DLMM upgrade support.
The existing program-upgrade report, ChangeSpec and API contracts remain tested.

Current acquisition uses a dedicated, bounded, read-only parent provider. Durable
workers receive exact staged bytes and no environment. Browser input supplies terms
only. A result cannot inherit proof across accounts, amounts, banks, positions,
ranges, paths or observations. Ready is a scoped policy finding; execution success
is not transaction authorization, issuer entitlement or signing possession.

## T0 before/after contract

The full comparison is retained as machine-readable evidence, not replaced by a
selection of favorable totals:

| Contract | Before | After and justified differences |
| --- | --- | --- |
| Migration cases A–H, defect variants, minimal and frozen SPACEX (14 cases) | [Pinned STA values](examples/phase-t0-stock-transition-integration/sta-migration-reference.json) | [MAIN values](phase-t3-migration-reference.json); populations, classifications, readiness, invariant status/text, reconciliation equations, both gates, stress and recorded search expectations reasserted |
| Candidate loader | Pinned STA loader experiment | MAIN upgradeable loader, approved at T0; actual T3 port re-runs the reference |
| Minimal stress | Old request said 19/20; pinned source observed 20/20 | **20/20 behaved as specified:** eleven expected migrations and nine expected rejections. STA behavior was not changed |
| Identities and numeric encoding | STA proposal/package identities and original encodings | MAIN ChangeSpec includes executable identity; canonical integer-string encoding and derived addresses change hashes. Approved projections normalize only these explicit changes |
| Lifecycle, notice and assurance | [Frozen test contract](examples/phase-t0-stock-transition-integration/sta-test-contract.json) | [T6 identity/projection record](phase-t6-identity-reference.json) and [source mapping](phase-t6-test-mapping.json); raw capture bytes unchanged |
| Current paths, probes and positions | Same frozen test contract | [T7 mapping](phase-t7-test-mapping.json), shared MAIN decoders/evidence and actual offline execution |
| Cloud browser copy | Three stale assertions against pinned rendered copy | [T0 owner review](examples/phase-t0-stock-transition-integration/owner-review.json); copy-only mismatches, no semantic change |
| Fee-defect lint | Frozen STA unused constant under its defect feature | Minimal MAIN-only hygiene fix; defect behavior unchanged |
| Toolchain | Initial frontend check used another pnpm | Exact pinned pnpm 11.24.0 checks appended to T0; same pin used thereafter |
| Cloud/runtime architecture | STA Postgres evidence registry, Node services and fixed-ratio checks | Approved MAIN registry/CAS plus identity-only Postgres; MAIN durable retries and Token Migration V1. [T9 mapping](phase-t9-test-mapping.json) records those adaptations |

The minimal deadline defect retains its derived counterexample and offline
reproduction. Frozen SPACEX retains 17,870 accounts, 10,091 positive accounts,
seven class totals, eight migrated units, the insufficient-reserve block and seven
reconciliation equations. No expectation was weakened to accommodate a port.

## Verification and limitations

[Final verification](phase-t10-verification.json) indexes the exact phase commands
and results, including unsuccessful attempts and successful retries. The required
checks include formatting, workspace/program clippy for every feature, program/
engine/server tests, no-fail-fast Rust execution, frontend/reference verification,
Postgres-backed identity/sync tests and both browser suites. Built-binary analyses
are deterministic: [the T9 record](phase-t9-determinism.json) gives the hashes of
seven migration artifacts and the complete lifecycle report.

All current-state acquisition tests used frozen fixtures or loopback mock providers.
Seven source browser cases requiring live providers were intentionally not run.
The two pre-existing ignored Rust checks remain identified in the phase logs.
Optional release packaging was not authorized and was not added. Docker was not
available for an image build; a reduced source-context compile passed without raw
captures or SBF files. The Postgres connector requires a trusted private transport.
A fresh checkout must import the exact ignored fixture dependencies from the pinned
archive before running their tests. Historical evidence in either repository was not overwritten; MAIN projections are separately labelled.

No real keys were used. Synthetic keys and signatures are confined to offline VM
fixtures. No transaction was built for, signed for, simulated on or submitted to a
live cluster. No live-provider check, infrastructure provisioning, deployment,
push or main-branch move occurred. The final audit matches all 1,633 pinned STA
files; final repository status is recorded with the documentation verification.

## Next milestone

The next owner-authorized milestone is a bounded read-only mainnet analysis of the
SPACEX migration specification through a real provider. Keep its fresh captures
separate from these reference fixtures. Partial and claim-based migration semantics
and a local-validator check of the unsigned plan follow as separate work; none is
implied by completing this migration.
