#!/usr/bin/env bash
# Build the deliberately regressed SPL-Stake-Pool-compatible Phase 8 candidate.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/programs/fixture-stake-pool-candidate/Cargo.toml"
OUT="$ROOT/artifacts"

[ -d "$HOME/.cargo/bin" ] && export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo-build-sbf >/dev/null 2>&1; then
  export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
fi
if ! command -v cargo-build-sbf >/dev/null 2>&1; then
  echo "error: cargo-build-sbf not found" >&2
  exit 1
fi

# Two builds of one source: the candidate carries the defect, the reference does
# not. The reference exists so the cross-program execution path can be exercised
# locally against the real SPL Token program without a mainnet download; the
# demo compares the candidate against the binary mainnet actually deployed.
for FLAVOUR in candidate reference; do
  DIR="$OUT/stake-pool-$FLAVOUR"
  mkdir -p "$DIR"
  if [ "$FLAVOUR" = reference ]; then
    cargo-build-sbf --manifest-path "$MANIFEST" --sbf-out-dir "$DIR" --features reference
    cp "$DIR/fixture_stake_pool_candidate.so" "$OUT/fixture_stake_pool_reference.so"
  else
    cargo-build-sbf --manifest-path "$MANIFEST" --sbf-out-dir "$DIR"
    cp "$DIR/fixture_stake_pool_candidate.so" "$OUT/fixture_stake_pool_v2.so"
  fi
done
for ARTIFACT in "$OUT/fixture_stake_pool_reference.so" "$OUT/fixture_stake_pool_v2.so"; do
  shasum -a 256 "$ARTIFACT" 2>/dev/null || sha256sum "$ARTIFACT"
done
