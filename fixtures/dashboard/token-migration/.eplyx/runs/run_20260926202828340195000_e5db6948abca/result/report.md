# Token migration rehearsal.

Analytical input `4c8e817f419088daaaa61fbc24a17397373eb781b3b366e6153d585ea69fdedd` · candidate `e5db6948abca1317eb12155f73cdaf619d378c22d1bfc992c977063e10e9c1bb` (Declared only) · state SyntheticFixture (eplyx-synthetic-fixture)

1. **Can the migration execute for the tested states?** Yes (mechanism Ready).
2. **Population coverage:** 7 token accounts, 6 with a positive balance; 4 attempted and 4 migrated in the sequential rehearsal (enumeration CompleteSyntheticFixture).
3. **Who cannot migrate:** 1 AuthorityPathUnavailable (77000000 raw); 1 Frozen (5000000 raw).
4. **Does accounting reconcile?** Yes — ReconciledForExecutedUnits.
5. **Is funding sufficient?** Yes (required 1191234567000, available 2000000000000).
6. **Failures:** 0 stress cases deviate from the specification; 0 population units did not migrate. Run `eplyx migration search` for bounded counterexamples.
7. **Limitations:** local VM rehearsal only; Official transition: Not evaluated; signatures assumed locally; derived states are not chain state.
8. **Reproduce:** `eplyx migration reproduce <cx-id>` or replay this run offline (no RPC).

Analytical status: **Incomplete**

## Migration invariants

- **Satisfied** candidate_binary_matches_package (blocking): The VM loaded exactly the packaged candidate bytes.
- **Satisfied** required_authorities_match (blocking): All 1 declared authority expectations match captured state.
- **Satisfied** migration_arithmetic_matches_spec (blocking): All 15 successful migrations reconciled exactly (debit, disposition, funding, credit, fees, log).
- **Satisfied** failed_migrations_roll_back (blocking): All 9 rejected executions rolled back every referenced account.
- **Satisfied** supply_reconciles (blocking): Every population reconciliation equation holds over the executed units.
- **Satisfied** reserve_covers_eligible_holders (blocking): The reserve of 2000000000000 raw covers the 1191234567000 raw every eligible holder needs.
- **Satisfied** window_rules_hold (blocking): 4 boundary cases behave as specified.
- **Satisfied** no_unsupported_execution_succeeds (blocking): All 9 cases the specification rejects were rejected by the candidate.
- **Violated** all_positive_holders_migrate (warning): 2 of 6 positive-balance holders did not migrate under the current mechanism.

## Deployment gate

**Passed with warnings** under `block-only`.

Reason codes: `AUTHORITY_PATH_UNAVAILABLE`, `SOURCE_FROZEN`, `STRANDED_HOLDERS`

- population_readiness is Incomplete
- 1 positive-balance holder: AUTHORITY_PATH_UNAVAILABLE
- 1 positive-balance holder: SOURCE_FROZEN
- invariant minv-83a2eda3d3dbb60c33bd (all_positive_holders_migrate) is Violated: 2 of 6 positive-balance holders did not migrate under the current mechanism.

No mainnet transaction was sent and no funds moved.
