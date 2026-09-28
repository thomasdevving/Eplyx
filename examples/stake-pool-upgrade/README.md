# Stake Pool regression fixture

This example uses the existing, tracked [`deploy/bundle`](../../deploy/bundle) as its offline baseline. Its candidate is built from [`programs/fixture-stake-pool-candidate`](../../programs/fixture-stake-pool-candidate), a locally constructed MIT-declared fixture. It is deliberately regressed, supports `DepositSol` only, and is **not** an SPL Stake Pool release candidate or code to deploy. The baseline and dependency program bytes are already in the bundle; this example does not copy them.

From the repository root, install Rust/Cargo and `cargo-build-sbf` 4.4.0 (`cargo install cargo-build-sbf --version 4.4.0 --locked` if needed), then run:

```sh
./scripts/build-stake-pool-candidate.sh
```

The script uses the tracked `programs/fixture-stake-pool-candidate/Cargo.lock`, platform-tools v1.54 and SBF architecture v0. It writes `artifacts/fixture_stake_pool_v2.so` and fails unless its SHA-256 is exactly:

```text
3193eabd9fe2e479109ef3b2dd7301fffd06774325133ff8f88916ed482db099
```

With `eplyx` built by `cargo build --locked -p eplyx-engine`, check the existing bundle and candidate:

```sh
target/debug/eplyx bundle verify --bundle deploy/bundle
target/debug/eplyx ci check --bundle deploy/bundle \
  --candidate artifacts/fixture_stake_pool_v2.so \
  --format json --out regression.json
```

The check exits **1** with exactly two finding categories: one `DepositSol` economic decrease (the deliberate cached-rate arithmetic defect) and nine `WithdrawSol` executions now reverting (the fixture's intentionally narrow instruction support). The output file contains the canonical Eplyx CI report. `regression.json` is disposable output; keep it outside the repository or remove it after inspection.

The candidate source was constructed for this repository and declares MIT; its `solana-program` dependency declares Apache-2.0. A compiled `.so` could technically be committed, but this example uses the reproducible source build and adds no program bytes or new redistribution notices. The tracked historical bundle remains in `deploy/bundle`.
