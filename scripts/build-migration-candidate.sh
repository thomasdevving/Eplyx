#!/usr/bin/env bash
# Reference mechanism and deliberately defective offline test candidates.
# Keep platform-tools v1.57 / SBF v3 separate from MAIN lending's v1.54 / v0.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/programs/eplyx-token-migration/Cargo.toml"
OUT="$ROOT/artifacts"
mkdir -p "$OUT"
for FLAVOUR in reference defect-deadline-inclusive defect-fee-ceiling; do
  echo "==> building eplyx-token-migration [$FLAVOUR; platform-tools v1.57, SBF v3]"
  BUILD_ARGS=(--tools-version v1.57 --arch v3 --manifest-path "$MANIFEST" --sbf-out-dir "$OUT/migration-$FLAVOUR")
  if [ "$FLAVOUR" != reference ]; then BUILD_ARGS+=(--features "$FLAVOUR"); fi
  cargo-build-sbf "${BUILD_ARGS[@]}"
  if [ "$FLAVOUR" = reference ]; then
    DEST="eplyx_token_migration.so"
  else
    DEST="eplyx_token_migration_${FLAVOUR//-/_}.so"
  fi
  cp "$OUT/migration-$FLAVOUR/eplyx_token_migration.so" "$OUT/$DEST"
done
python3 "$ROOT/scripts/verify-migration-programs.py"
