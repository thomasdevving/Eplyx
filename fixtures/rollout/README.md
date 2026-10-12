# Rollout rehearsal fixtures

Constructed counterexamples, not upstream releases, not production candidates
and not intended for deployment. Each `.so` has its qualified build receipt
beside it, and its build script fails unless the rebuilt ELF equals these bytes.
See [the rollout rehearsal](../../docs/rollout-rehearsal.md).

| File | Source | Rebuild |
| --- | --- | --- |
| `fixture_stake_pool_rollout_v2.so` (`64612be0…`, 134,320 bytes) | `programs/fixture-stake-pool-rollout-candidate` | `scripts/build-stake-pool-rollout-candidate.sh` |
| `fixture_stake_pool_oversized_rollout_v2.so` (`329092d7…`, 1,146,384 bytes) | `programs/fixture-stake-pool-oversized-rollout-candidate` | `scripts/build-stake-pool-oversized-rollout-candidate.sh` |
