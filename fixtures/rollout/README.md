# Rollout rehearsal fixture

`fixture_stake_pool_rollout_v2.so` is the constructed rollout counterexample
built from `programs/fixture-stake-pool-rollout-candidate` — not an upstream
release and not intended for deployment. `fixture_stake_pool_rollout_v2.build.json`
is the receipt of the qualified build. `scripts/build-stake-pool-rollout-candidate.sh`
reproduces both and fails unless the rebuilt ELF equals these bytes. See
[the rollout rehearsal](../../docs/rollout-rehearsal.md).
