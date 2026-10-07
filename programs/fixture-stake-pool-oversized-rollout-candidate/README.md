# Stake Pool oversized rollout counterexample candidate

**Constructed oversized rollout counterexample. Not an upstream release. Not a
production candidate. Not intended for deployment. Qualified only for the
rollout rehearsal's ProgramData-preparation boundary.**

The [rollout counterexample](../fixture-stake-pool-rollout-candidate/README.md)
fits the retained Stake Pool ProgramData (1,080,464 executable bytes). This
fixture exists only so the [rollout rehearsal](../../docs/rollout-rehearsal.md#programdata-preparation-and-extendprogram)
can exercise a real loader-v3 `ExtendProgram` before `Upgrade`.

Its source is the rollout counterexample's, behaviour unchanged — `SetFee(SolDeposit)`
rejects a new fee above 1/200, `DepositSol` applies whatever fee the pool holds —
plus `BALLAST`: 1,012,000 bytes of non-zero, deterministic read-only program
data, compiled into the ELF's `.rodata`. Nothing is appended after compilation;
the loader verifies and deploys it as an ordinary program. `BALLAST` is
referenced only on the rejected-instruction path (through
`core::hint::black_box`), so the qualified SetFee and DepositSol paths never
read it.

## Build identity

Tracked bytes: [`fixtures/rollout/fixture_stake_pool_oversized_rollout_v2.so`](../../fixtures/rollout/fixture_stake_pool_oversized_rollout_v2.so),
SHA-256 `329092d75f7116266f40bf61050ed74ce407483619f4bbb9e27b72474019c7c4`,
1,146,384 bytes — 65,920 bytes more than the retained ProgramData capacity.
The receipt is beside it.

Qualified host: Linux x86_64, cargo-build-sbf 4.0.0, cached platform-tools v1.54
(rustc 1.89.0-dev, the Step 10B `version.md` revisions), SBF arch v0. Two
independent clean trees and the pinned script produced identical bytes:

```sh
cargo fetch --locked --manifest-path programs/fixture-stake-pool-oversized-rollout-candidate/Cargo.toml
scripts/build-stake-pool-oversized-rollout-candidate.sh /tmp/oversized-candidate
```

Other hosts and builders are not qualified, which is why the bytes are tracked
rather than rebuilt in CI. Dependencies and lockfile are the rollout
counterexample's. Attribution and the Apache-2.0 license are in `NOTICE` and
`LICENSE`.
