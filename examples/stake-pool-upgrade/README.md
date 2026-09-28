# Offline SPL Stake Pool upgrade-impact check

This is the canonical first Eplyx analysis. It uses the existing, tracked
[`deploy/bundle`](../../deploy/bundle) as its offline historical evidence and
the normal `eplyx ci check` engine. The bundle contains the pinned baseline,
two dependency programs and ten retained historical observations; no second
bundle or duplicated evidence is used.

The comparison candidate is built from
[`programs/fixture-stake-pool-candidate`](../../programs/fixture-stake-pool-candidate),
a locally constructed MIT-declared fixture. It is deliberately regressed,
supports `DepositSol` only, and is **not** an SPL Stake Pool release candidate
or code to deploy.

## Run the control and regression

From the repository root, choose either the supported macOS arm64 release binary
or a source build. The [root installation instructions](../../README.md#install-the-cli)
cover archive verification. The release contains only the CLI, so this example's
separately pinned SBF candidate must still be built from source. For an extracted
release binary in the repository root:

```sh
EPLYX=./eplyx
```

Or build the CLI from source:

```sh
cargo build --locked -p eplyx-engine
EPLYX=target/debug/eplyx
```

Then run the shared example steps:

```sh
cargo install cargo-build-sbf --version 4.4.0 --locked

./scripts/build-stake-pool-candidate.sh

"$EPLYX" bundle verify \
  --bundle deploy/bundle

"$EPLYX" ci check \
  --bundle deploy/bundle \
  --candidate deploy/bundle/binaries/current.so \
  --format json \
  --out control.json

"$EPLYX" ci check \
  --bundle deploy/bundle \
  --candidate artifacts/fixture_stake_pool_v2.so \
  --format json \
  --out regression.json
```

The last command exits **1 by design**. It is the expected gate verdict for the
known regression, not a broken Eplyx invocation. Both JSON files are disposable
outputs and need not be committed.

The build script uses the tracked candidate `Cargo.lock`, platform-tools v1.54
and SBF architecture v0. It writes `artifacts/fixture_stake_pool_v2.so` and
fails unless its SHA-256 is exactly:

```text
3193eabd9fe2e479109ef3b2dd7301fffd06774325133ff8f88916ed482db099
```

The canonical fixture identity is currently verified by the repository's
`macos-15` GitHub Actions environment. A Linux build produced different SBF
bytes and was correctly rejected by the hash check. Other platforms are not yet
part of this reproducibility contract; there are no alternate canonical hashes.

## Interpret the reports

The control is the bundle's pinned current program checked against its own
historical evidence. Expect exit code 0, zero findings, ten pinned observations
and six covered semantic subjects. This demonstrates that the retained
replay/evidence path reproduces without an upgrade regression. It does not prove
the program, engine or corpus universally correct.

The constructed candidate produces exit code 1 and exactly two finding
categories:

- One `DepositSol` observation has an economic decrease in pool tokens received.
  This is the fixture's deliberate cached-rate arithmetic defect.
- Nine `WithdrawSol` executions now revert. These failures also reflect the
  fixture's intentionally narrow `DepositSol`-only instruction support.

This is stronger than a toy unit test because it uses retained historical
observations, a pinned baseline and dependencies, the production CI-check path,
semantic coverage reporting, and a candidate identity protected by clean-checkout
CI. The analysis itself is offline once the CLI and candidate have been built.

Its claim remains bounded to the included corpus. It does not establish that all
Stake Pool behavior is covered, all Solana transactions are replayable, every
upgrade can be analysed automatically, or the absence of findings proves
universal safety.

The candidate source declares MIT and its `solana-program` dependency declares
Apache-2.0. The example builds from source and adds no compiled program bytes or
new redistribution notices; the historical bundle remains in `deploy/bundle`.
