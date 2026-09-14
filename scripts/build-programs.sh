#!/usr/bin/env bash
# Build both versions of the fixture program to SBF bytecode.
#
# Both builds come from the same source tree and differ only by cargo feature.
# The resulting .so files are what the engine actually executes -- nothing in the
# comparison path calls Rust business logic directly.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/programs/fixture-lending/Cargo.toml"
OUT="$ROOT/artifacts"

# cargo-build-sbf shells out to `cargo`, so both toolchains must be on PATH.
[ -d "$HOME/.cargo/bin" ] && export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo-build-sbf >/dev/null 2>&1; then
  export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
fi
if ! command -v cargo-build-sbf >/dev/null 2>&1; then
  echo "error: cargo-build-sbf not found." >&2
  echo "Install the Solana toolchain:" >&2
  echo '  sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"' >&2
  exit 1
fi

mkdir -p "$OUT"
for VERSION in v1 v2; do
  echo "==> building fixture-lending [$VERSION]"
  cargo-build-sbf \
    --manifest-path "$MANIFEST" \
    --sbf-out-dir "$OUT/$VERSION" \
    --no-default-features \
    --features "$VERSION"
  cp "$OUT/$VERSION/fixture_lending.so" "$OUT/fixture_lending_$VERSION.so"
done

echo
echo "==> artifacts"
ls -l "$OUT"/*.so
echo
echo "==> sha256 (V1 and V2 must differ)"
shasum -a 256 "$OUT"/fixture_lending_v1.so "$OUT"/fixture_lending_v2.so 2>/dev/null \
  || sha256sum "$OUT"/fixture_lending_v1.so "$OUT"/fixture_lending_v2.so
