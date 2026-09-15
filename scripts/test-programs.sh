#!/usr/bin/env bash
# Host-target unit tests for the fixture program, run once per build flavour.
# These cover the arithmetic in isolation; the differential suite in
# engine/tests covers it through real SBF execution.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/programs/fixture-lending/Cargo.toml"
[ -d "$HOME/.cargo/bin" ] && export PATH="$HOME/.cargo/bin:$PATH"

for VERSION in v1 v2; do
  echo "==> testing fixture-lending [$VERSION]"
  cargo test --manifest-path "$MANIFEST" --no-default-features --features "$VERSION"
done

echo "==> testing fixture-memo-candidate"
cargo test --manifest-path "$ROOT/programs/fixture-memo-candidate/Cargo.toml"

echo "==> testing fixture-token2022-candidate"
cargo test --manifest-path "$ROOT/programs/fixture-token2022-candidate/Cargo.toml"

for FLAVOUR in "" "--features reference"; do
  echo "==> testing fixture-stake-pool-candidate [${FLAVOUR:-candidate}]"
  # shellcheck disable=SC2086
  cargo test --manifest-path "$ROOT/programs/fixture-stake-pool-candidate/Cargo.toml" $FLAVOUR
done
