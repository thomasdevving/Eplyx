# Stake Pool rollout counterexample candidate

**Constructed rollout counterexample. Not an upstream release. Not intended for
deployment. Qualified only for the stated rollout-rehearsal boundary.**

This fixture exists so the [rollout rehearsal](../../docs/rollout-rehearsal.md)
can show, by execution, a real class of rollout-order incompatibility: state that
was valid to create under the old program survives into a new version that would
no longer allow it to be created.

It is the Step 10B [config candidate](../fixture-stake-pool-config-candidate/README.md)
source with one behavioural addition. `SetFee(SolDeposit)` first applies the
official `check_too_high` validation, then rejects any fee above the exact
fraction 1/200 (0.5%) with the official `StakePoolError::FeeTooHigh`
(`Custom(4)`). `DepositSol` does **not** re-check that maximum: it reads and
applies whatever fee the pool holds. Everything else — dispatch limited to
SetFee(SolDeposit) and ungated ten-account DepositSol, full typed-layout
roundtrip, exact trailing bytes, manager key/signature checks — is unchanged.

| SetFee(SolDeposit) | Historical V1 | This candidate |
| --- | --- | --- |
| `0/0`, `1/1000`, `1/200` | accepted | accepted |
| `1/100` | accepted | rejected (`FeeTooHigh`) |
| `1/0`, `2/1` | rejected | rejected |

The analyzer does not know any of this. The rollout report's order effect is
whatever the VM produces from these bytes.

## Build identity

The engine executes the tracked bytes at
[`fixtures/rollout/fixture_stake_pool_rollout_v2.so`](../../fixtures/rollout/fixture_stake_pool_rollout_v2.so):
SHA-256 `64612be0d9dde5cb4f24d1542572f56ff59b1fa66552329f71227dcfd329c019`,
134320 bytes. Its build receipt is beside it.

Reproduce and re-check them on the qualified host — Linux x86_64,
cargo-build-sbf 4.0.0, cached platform-tools v1.54 (rustc 1.89.0-dev, the same
`version.md` revisions as Step 10B), SBF arch v0:

```sh
cargo fetch --locked --manifest-path programs/fixture-stake-pool-rollout-candidate/Cargo.toml
scripts/build-stake-pool-rollout-candidate.sh /tmp/rollout-candidate
```

The script stages only the source, manifest, lockfile, NOTICE and LICENSE in a
fresh tree, builds offline into a fresh target, refuses any other identity and
requires byte equality with the tracked fixture. Two independent clean trees
and the script itself produced the same ELF.

Other hosts and builders are **not** qualified. The bytes are tracked rather
than rebuilt in CI for that reason: the Step 10B source built from the same
platform-tools revisions gives a different ELF on Linux x86_64 /
cargo-build-sbf 4.0.0 than its Darwin arm64 / 4.4.0 qualification.

Dependencies are those of Step 10B (`spl-stake-pool=2.0.3`, `solana-program=2.3.0`,
`spl-token=8.0.0`, `solana-system-interface=1.0.0`, `borsh=1.8.1`), in this
directory's own lockfile. Attribution and the Apache-2.0 license are in `NOTICE`
and `LICENSE`.
