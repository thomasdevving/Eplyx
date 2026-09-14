# Phase 1–3 implementation audit — 2026-09-14

## Overall status

Phase 1: PASS WITH MINOR ISSUES. Phase 2: PASS WITH MINOR ISSUES.
Phase 3: PASS WITH MINOR ISSUES. Concrete defects below are fixed and regression-tested.
GO for the controlled Phase 4 path, with explicit fidelity validation; this is not
approval to claim arbitrary historical mainnet replay.

## Requirement matrix

| # | Status | Evidence / finding |
|---|---|---|
| 1 | PASS | `load_versions` reads both ELF files; `executor::execute` calls `LiteSVM::add_program`, builds a signed transaction and `send_transaction`; captures metadata and account post-state. No host business-logic call. |
| 2 | PASS | Fresh VM per call, same fixture, fixed Clock, same payer/signers/metas/data/program address. Repeated execution test compares full results. |
| 3 | PASS | Both feature builds link the same feature-free interface crate. Account layouts, errors and authority checks are shared. |
| 4 | PASS | Only `math::collateral_value` changes arithmetic: multiply/divide vs divide/multiply. Build-version log labels also differ intentionally. Whole-unit controls pass. |
| 5 | PASS | 141 stable generated IDs, nine categories, deterministic generation; checked-in JSON equals generator. CLI uses the generator, not directory iteration/loading. Corpus includes controls and parameter sweeps, not just failures. |
| 6 | PASS | Success, errors, fees, CU, logs, CPI and watched account bytes/balances captured; health and economics decoded from those bytes. |
| 7 | PASS | Outcome, decoded fields, raw data, balances, liquidation, CPI and compute types. Fixed previously ignored owner/executable/rent metadata. Compute excluded from behavioral classification. |
| 8 | PASS | boundary-position-017: both succeed, health 1.003783 → 0.998738, liquidatable false → true. |
| 9 | PASS | 141 tested, 89 identical, 52 changed, 11 critical. Audit fixes do not shift baseline. |
| 10 | PASS | u128 micro-USD / i128 signed values, decimal strings; fixed rendering overflow, signed difference wrapping and signed-minimum parsing. Saturating aggregate addition is explicitly documented, not silent by accident. |
| 11 | PASS | Multiplication widened before division; SOL 9 decimals, debt 6 decimals at $1. Fixed exponent overflow for decimals ≥39 (exact result zero for u64 inputs). Boundary tests included. |
| 12 | PASS | 141 valued, collateral $6,182,370.00, debt $2,531,638.09, net $3,650,731.91; Rust and independent TypeScript integer sums. |
| 13 | PASS | Any non-compute difference is affected; this includes structural/CPI changes and is broader than proven loss. Labels disclose definition. |
| 14 | PASS | Affected 52 ($444,570 collateral/$318,298.09 debt), critical 11 ($109,650/$85,410), unaffected 89. Independent subset sums tested. |
| 15 | PASS | Deduplicated ordered consequence sets; newly-liquidatable and now-succeeds distinct. Categories partition this corpus; in general consequences may overlap and their capital must not be added as a disjoint total. |
| 16 | PASS | Four false→true transitions, $39,800 collateral, $31,780 debt. |
| 17 | PASS | Flagship collateral $9,950, debt $7,930, net $2,020, consequence newly_liquidatable. |
| 18 | PASS | Money serializes as decimal strings and Rust round-trips; TS recomputes totals with BigInt. Numeric cluster ranges still use integer JSON numbers: Rust preserves these, JS consumers must parse large range bounds losslessly. |
| 19 | PASS | Signed integer basis points, exact JSON round-trip; fixed severity abs overflow for i32::MIN. Runtime CU bounded well within representation. |
| 20 | PASS | Corpus capital represented/affected, no measured deployed TVL or loss claim. |
| 21 | PASS | Consequence/action/difference kinds/fields/outcome signature, ordered maps. Fixed ID collisions for same-action distinct classes; added account-specific liquidation direction to signature. |
| 22 | PASS | Common conditions derived from fixture and observed output; fractional collateral and transition properties. Broad ranges not promoted as triggers. |
| 23 | PASS | Min/max of collateral, debt, valuation and both health factors. Infinite health omitted; 20% spread gate. Fixture bounds safe in i128. See JSON range caveat at #18. |
| 24 | PASS | Simplicity, economic magnitude, boundary distance, stable ID tie-break. Flagship naturally wins among equal-size newly-liquidatable members by debt magnitude. |
| 25 | PASS | Collateral/debt mutations, greedy ratio and single-field descent, 96 probes; each runs both binaries. Fixed zero-granularity nontermination and vault overflow. |
| 26 | PASS | Exact regression signature required, now including per-account liquidation directions. |
| 27 | PASS | Independent test re-executes each minimized critical witness and compares signatures. Newly-liquidatable: 99.5 SOL/$7,930 → 0.001 SOL/$0.01; health 8.000000 → 0, both succeed. |
| 28 | PASS | `reproduce boundary-position-017` and `reproduce newly-liquidatable` supported and exercised. |
| 29 | PASS | IDs, consequences, action/kinds, member IDs, common conditions/ranges, representative and minimized witness serialized and round-tripped. |
| 30 | PASS | CLI shows conditions, counts, economics, representative, minimization and detailed low-level reproduction. |
| 31 | PASS | Execution, interpretation, aggregation, clustering, shrinking and reporting in separate modules. No large refactor necessary. |
| 32 | PARTIAL | Claimed protocol independence was overstated: executor names lending errors; diff invokes interpreter and contains liquidation types. README corrected. Adapter seam is still a module convention, not a trait. |
| 33 | PASS | Generation/execution/shrinking determinism tested; repeated full JSON comparison recorded below. Absolute binary paths are machine-specific metadata. |
| 34 | PASS | Runtime tests use local binaries only, no network/wallet/time inputs; root resolved via manifest. Installing build dependencies can require network. |
| 35 | PASS | Only actual floating-point computation is in a test demonstrating precision loss (`money.rs`). Remaining mentions are comments/test scanner/docs, no source-of-truth float path. |
| 36 | PASS | Fixed monetary and decimal bounds and vault overflow. Fixed protocol units bound source values; aggregate saturation and infinite-health sentinel remain documented policies. Lamport/CU deltas assume valid Solana runtime bounds, not arbitrary u64 synthetic metadata. |
| 37 | PASS | Build/load/execution errors propagated, corpus comparisons collect Result and do not skip failed executions. Optional economics/clustering skip non-position records by design; malformed external records require validation before future ingestion. |
| 38 | PASS | Full run is 282 base executions plus two per minimizer probe and two final executions per successful witness; timing and probe totals below. Fresh VM setup dominates. |
| 39 | PASS | Synthetic, single-protocol scope stated. Corrected adapter-independence and smallest-witness wording. |
| 40 | PASS | No active old brand in binary/package/README commands. Core IDs and schemas remain neutral. |

## Fixes and remaining limitations

Fixed: money boundary/parser errors, high decimal exponents, metadata diffs,
cluster-ID collisions, liquidation signature detail, zero-granularity termination,
vault overflow, compute severity absolute value. Added targeted regression tests.

Expected limitations: synthetic independent positions; one purpose-built protocol;
no deployed TVL, arbitrary protocols, historical snapshots, multi-step search or
formal verification. Minimization is a greedy witness with 0.001 SOL/$0.01
resolution, not a minimality proof. Corpus repeated interactions would represent
exposure observations rather than unique TVL and must be labelled accordingly.

Technical debt: interpretation is called from structural diffing, error names from
execution; optional missing economics is not itself an error. Phase 4 must validate
record completeness, state provenance and V1 fidelity before interpreting V2.

## Actual verification

`make fmt-check`, `make lint` (warnings denied), `make test`: passed.
7 V1 + 7 V2 + 36 engine unit + 42 integration + 6 interface tests passed.
Full minimized JSON runs: 32.08 s and 28.93 s wall time.
Both files byte-identical, SHA-256 `9ab19005e77842f6113b60c7e19787193d0072f8ead8372d9d39d6e013480fa1`.
224 minimizer probes, 736 total VM executions including final witnesses.
CLI text, flagship reproduction and critical-cluster reproduction also passed.
