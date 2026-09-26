# Token migration rehearsal.

Analytical input `7336b0468e05d782e93dce7c7e11a03d0958f2cebe1410896dca5e042ad66ecf` · candidate `c4346736008188befe1d1a41cf167ea6a80c3b61a76cf1d3ba550f916b682275` (Declared only) · state SyntheticFixture (eplyx-synthetic-fixture)

1. **Can the migration execute for the tested states?** No (mechanism Blocked).
2. **Population coverage:** 7 token accounts, 6 with a positive balance; 4 attempted and 4 migrated in the sequential rehearsal (enumeration CompleteSyntheticFixture).
3. **Who cannot migrate:** 1 AuthorityPathUnavailable (77000000 raw); 1 Frozen (5000000 raw).
4. **Does accounting reconcile?** Yes — ReconciledForExecutedUnits.
5. **Is funding sufficient?** Yes (required 1191234567000, available 2000000000000).
6. **Failures:** 1 stress cases deviate from the specification; 0 population units did not migrate. Run `eplyx migration search` for bounded counterexamples.
7. **Limitations:** local VM rehearsal only; Official transition: Not evaluated; signatures assumed locally; derived states are not chain state.
8. **Reproduce:** `eplyx migration reproduce <cx-id>` or replay this run offline (no RPC).

Analytical status: **Blocked**

## Migration invariants

- **Satisfied** candidate_binary_matches_package (blocking): The VM loaded exactly the packaged candidate bytes.
- **Satisfied** required_authorities_match (blocking): All 1 declared authority expectations match captured state.
- **Violated** migration_arithmetic_matches_spec (blocking): 1 executed migrations differ from the specification's exact arithmetic or deltas.
- **Satisfied** failed_migrations_roll_back (blocking): All 8 rejected executions rolled back every referenced account.
- **Satisfied** supply_reconciles (blocking): Every population reconciliation equation holds over the executed units.
- **Satisfied** reserve_covers_eligible_holders (blocking): The reserve of 2000000000000 raw covers the 1191234567000 raw every eligible holder needs.
- **Violated** window_rules_hold (blocking): Deviating cases: stress-12 AtDeadline (UnexpectedSuccess).
- **Violated** no_unsupported_execution_succeeds (blocking): 1 executions succeeded although the specification requires rejection.
- **Violated** all_positive_holders_migrate (warning): 2 of 6 positive-balance holders did not migrate under the current mechanism.

## Deployment gate

**Failed** under `block-only`.

Reason codes: `AUTHORITY_PATH_UNAVAILABLE`, `MIGRATION_ARITHMETIC_MISMATCH`, `MIGRATION_WINDOW_INVALID`, `SOURCE_FROZEN`, `STRANDED_HOLDERS`, `UNEXPECTED_MIGRATION_SUCCESS`

- mechanism_readiness is Blocked
- Stress stress-12 AtDeadline (DerivedFromSynthetic) expected Reject, candidate ReconciliationMismatch
- population_readiness is Incomplete
- 1 positive-balance holder: AUTHORITY_PATH_UNAVAILABLE
- 1 positive-balance holder: SOURCE_FROZEN
- invariant minv-f4b81b4d375a12202c09 (migration_arithmetic_matches_spec) is Violated: 1 executed migrations differ from the specification's exact arithmetic or deltas.
- invariant minv-a2d2d17d3f472acaa205 (window_rules_hold) is Violated: Deviating cases: stress-12 AtDeadline (UnexpectedSuccess).
- invariant minv-f6235289b7c6ea5718a0 (no_unsupported_execution_succeeds) is Violated: 1 executions succeeded although the specification requires rejection.
- invariant minv-83a2eda3d3dbb60c33bd (all_positive_holders_migrate) is Violated: 2 of 6 positive-balance holders did not migrate under the current mechanism.

No mainnet transaction was sent and no funds moved.
