# Token migration rehearsal.

Analytical input `c0ef5d2749a9f0cb66b3a8e49b2a8350f737e7c37448f03fbbb54b5c43d1e61c` · candidate `c4346736008188befe1d1a41cf167ea6a80c3b61a76cf1d3ba550f916b682275` (Declared only) · state SyntheticFixture (eplyx-synthetic-fixture)

1. **Can the migration execute for the tested states?** No (mechanism Blocked).
2. **Population coverage:** 7 token accounts, 6 with a positive balance; 4 attempted and 3 migrated in the sequential rehearsal (enumeration CompleteSyntheticFixture).
3. **Who cannot migrate:** 1 AuthorityPathUnavailable (77 raw); 1 Frozen (5 raw); 1 InsufficientReserve (250 raw).
4. **Does accounting reconcile?** Yes — ReconciledForExecutedUnits.
5. **Is funding sufficient?** No (required 1191000, available 1000000).
6. **Failures:** 1 stress cases deviate from the specification; 1 population units did not migrate. Run `eplyx migration search` for bounded counterexamples.
7. **Limitations:** local VM rehearsal only; Official transition: Not evaluated; signatures assumed locally; derived states are not chain state.
8. **Reproduce:** `eplyx migration reproduce <cx-id>` or replay this run offline (no RPC).

Analytical status: **Blocked**

## Migration invariants

- **Satisfied** candidate_binary_matches_package (blocking): The VM loaded exactly the packaged candidate bytes.
- **Satisfied** required_authorities_match (blocking): All 1 declared authority expectations match captured state.
- **Violated** migration_arithmetic_matches_spec (blocking): 1 executed migrations differ from the specification's exact arithmetic or deltas.
- **Satisfied** failed_migrations_roll_back (blocking): All 9 rejected executions rolled back every referenced account.
- **Satisfied** supply_reconciles (blocking): Every population reconciliation equation holds over the executed units.
- **Violated** reserve_covers_eligible_holders (blocking): INSUFFICIENT_RESERVE: eligible holders need 1191000 raw; the reserve holds 1000000 raw (shortfall 191000).
- **Violated** window_rules_hold (blocking): Deviating cases: stress-12 AtDeadline (UnexpectedSuccess).
- **Violated** no_unsupported_execution_succeeds (blocking): 1 executions succeeded although the specification requires rejection.
- **Violated** all_positive_holders_migrate (warning): 3 of 6 positive-balance holders did not migrate under the current mechanism.

## Deployment gate

**Failed** under `block-only`.

Reason codes: `AUTHORITY_PATH_UNAVAILABLE`, `INSUFFICIENT_RESERVE`, `MIGRATION_ARITHMETIC_MISMATCH`, `MIGRATION_WINDOW_INVALID`, `REHEARSAL_UNITS_NOT_MIGRATED`, `SOURCE_FROZEN`, `STRANDED_HOLDERS`, `UNEXPECTED_MIGRATION_SUCCESS`

- mechanism_readiness is Blocked
- Stress stress-12 AtDeadline (DerivedFromSynthetic) expected Reject, candidate ReconciliationMismatch
- funding_readiness is Blocked
- Eligible holders need 1191000 raw destination units; the reserve holds 1000000 raw (shortfall 191000).
- population_readiness is Incomplete
- 1 positive-balance holder: AUTHORITY_PATH_UNAVAILABLE
- 1 positive-balance holder: INSUFFICIENT_RESERVE
- 1 positive-balance holder: SOURCE_FROZEN
- 1 attempted units did not migrate in the sequential rehearsal.
- invariant minv-f4b81b4d375a12202c09 (migration_arithmetic_matches_spec) is Violated: 1 executed migrations differ from the specification's exact arithmetic or deltas.
- invariant minv-49ee6272a5e25b997654 (reserve_covers_eligible_holders) is Violated: INSUFFICIENT_RESERVE: eligible holders need 1191000 raw; the reserve holds 1000000 raw (shortfall 191000).
- invariant minv-a2d2d17d3f472acaa205 (window_rules_hold) is Violated: Deviating cases: stress-12 AtDeadline (UnexpectedSuccess).
- invariant minv-f6235289b7c6ea5718a0 (no_unsupported_execution_succeeds) is Violated: 1 executions succeeded although the specification requires rejection.
- invariant minv-83a2eda3d3dbb60c33bd (all_positive_holders_migrate) is Violated: 3 of 6 positive-balance holders did not migrate under the current mechanism.

No mainnet transaction was sent and no funds moved.
