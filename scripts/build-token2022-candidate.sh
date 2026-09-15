#!/usr/bin/env bash
# Build the deliberately regressed Token-2022-compatible Phase 7 candidate.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/programs/fixture-token2022-candidate/Cargo.toml"
OUT="$ROOT/artifacts"

[ -d "$HOME/.cargo/bin" ] && export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo-build-sbf >/dev/null 2>&1; then
  export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
fi
if ! command -v cargo-build-sbf >/dev/null 2>&1; then
  echo "error: cargo-build-sbf not found" >&2
  exit 1
fi

mkdir -p "$OUT/token2022-v2"
cargo-build-sbf \
  --manifest-path "$MANIFEST" \
  --sbf-out-dir "$OUT/token2022-v2"
cp "$OUT/token2022-v2/fixture_token2022_candidate.so" "$OUT/fixture_token2022_v2.so"
shasum -a 256 "$OUT/fixture_token2022_v2.so" 2>/dev/null \
  || sha256sum "$OUT/fixture_token2022_v2.so"
