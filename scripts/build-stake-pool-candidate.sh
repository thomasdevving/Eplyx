#!/usr/bin/env bash
# Build the deliberately regressed SPL-Stake-Pool-compatible Phase 8 candidate.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/programs/fixture-stake-pool-candidate/Cargo.toml"
OUT="$ROOT/artifacts"
EXPECTED_CANDIDATE_SHA256="3193eabd9fe2e479109ef3b2dd7301fffd06774325133ff8f88916ed482db099"

[ -d "$HOME/.cargo/bin" ] && export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo-build-sbf >/dev/null 2>&1; then
  export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
fi
if ! command -v cargo-build-sbf >/dev/null 2>&1; then
  echo "error: cargo-build-sbf not found" >&2
  exit 1
fi
SBF_VERSION="$(cargo-build-sbf --version)"
if [ "${SBF_VERSION%%$'\n'*}" != "cargo-build-sbf 4.4.0" ]; then
  echo "error: this fixture requires cargo-build-sbf 4.4.0" >&2
  exit 1
fi
mkdir -p "$OUT"
rm -f "$OUT/fixture_stake_pool_v2.so"

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    echo "error: shasum or sha256sum is required" >&2
    return 1
  fi
}

# Two builds of one source: the candidate carries the defect, the reference does
# not. The reference exists so the cross-program execution path can be exercised
# locally against the real SPL Token program without a mainnet download; the
# demo compares the candidate against the binary mainnet actually deployed.
for FLAVOUR in candidate reference; do
  DIR="$OUT/stake-pool-$FLAVOUR"
  mkdir -p "$DIR"
  if [ "$FLAVOUR" = reference ]; then
    cargo-build-sbf --tools-version v1.54 --arch v0 \
      --manifest-path "$MANIFEST" --sbf-out-dir "$DIR" --features reference -- --locked
    cp "$DIR/fixture_stake_pool_candidate.so" "$OUT/fixture_stake_pool_reference.so"
  else
    cargo-build-sbf --tools-version v1.54 --arch v0 \
      --manifest-path "$MANIFEST" --sbf-out-dir "$DIR" -- --locked
    cp "$DIR/fixture_stake_pool_candidate.so" "$OUT/fixture_stake_pool_v2.so"
  fi
done

CANDIDATE="$OUT/fixture_stake_pool_v2.so"
ACTUAL_CANDIDATE_SHA256="$(sha256_file "$CANDIDATE")"
echo "candidate: $CANDIDATE"
echo "sha256:    $ACTUAL_CANDIDATE_SHA256"
if [ "$ACTUAL_CANDIDATE_SHA256" != "$EXPECTED_CANDIDATE_SHA256" ]; then
  rm -f "$CANDIDATE"
  echo "error: candidate does not match expected fixture SHA-256 $EXPECTED_CANDIDATE_SHA256" >&2
  exit 1
fi
echo "identity: matches expected fixture SHA-256"
echo "reference: $OUT/fixture_stake_pool_reference.so ($(sha256_file "$OUT/fixture_stake_pool_reference.so"))"
