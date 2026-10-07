#!/usr/bin/env bash
# Constructed rollout counterexample; not a deployment command.
#
# Builds programs/fixture-stake-pool-rollout-candidate from a source-only staging
# tree and refuses any output other than the qualified identity. The identity is
# qualified on Linux x86_64 with cargo-build-sbf 4.0.0 and cached platform-tools
# v1.54 (rustc 1.89.0-dev), SBF arch v0. Other hosts and builders are not
# qualified: the Step 10B fixture built from the same toolchain revisions on
# Darwin arm64 / cargo-build-sbf 4.4.0 has a different ELF hash than on Linux.
#
# The tracked bytes the engine executes live at
# fixtures/rollout/fixture_stake_pool_rollout_v2.so; this script reproduces and
# re-checks them, and optionally writes a copy plus a build receipt to $1.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/programs/fixture-stake-pool-rollout-candidate"
OUT="${1:-$ROOT/artifacts}"
TRACKED="$ROOT/fixtures/rollout/fixture_stake_pool_rollout_v2.so"
EXPECTED_SHA256="64612be0d9dde5cb4f24d1542572f56ff59b1fa66552329f71227dcfd329c019"
EXPECTED_LEN=134320
BUILDER="${EPLYX_SBF_BUILDER:-cargo-build-sbf}"
TOOLS="${EPLYX_SBF_TOOLS:-$HOME/.cache/solana/v1.54/platform-tools}"
[[ "$(uname -sm)" == "Linux x86_64" ]] || { echo 'Only Linux x86_64 is qualified for this fixture' >&2; exit 1; }
VERSION="$("$BUILDER" --version)"
[[ "${VERSION%%$'\n'*}" == 'cargo-build-sbf 4.0.0' ]] || { echo 'Expected cargo-build-sbf 4.0.0' >&2; exit 1; }
[[ "$("$TOOLS/rust/bin/rustc" --version)" == 'rustc 1.89.0-dev' ]] || { echo 'Expected cached platform-tools v1.54 rustc' >&2; exit 1; }
# The same platform-tools source revisions as the Step 10B qualification.
[[ "$(sha256sum "$TOOLS/version.md" | awk '{print $1}')" == 'c33d3fa3cae3d9ece4ea08bc8e51a754041286a351067bc3afd2635595619895' ]] || { echo 'Unexpected platform-tools source revisions' >&2; exit 1; }
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/eplyx-stake-rollout-build.XXXXXX")"
mkdir -p "$STAGE/source/src" "$STAGE/output" "$OUT"
cp "$SOURCE/Cargo.toml" "$SOURCE/Cargo.lock" "$SOURCE/NOTICE" "$SOURCE/LICENSE" "$STAGE/source/"
cp "$SOURCE/src/lib.rs" "$STAGE/source/src/"
export RUSTC="$TOOLS/rust/bin/rustc"
export CARGO_TARGET_DIR="$STAGE/source/target"
# Fresh source/target/output tree per invocation. Only public dependency and
# platform-tool caches are reused; no rustup mutation or network installation.
"$BUILDER" --tools-version v1.54 --arch v0 --skip-tools-install --no-rustup-override --offline \
  --manifest-path "$STAGE/source/Cargo.toml" --sbf-out-dir "$STAGE/output" -- --locked
ELF="$STAGE/output/fixture_stake_pool_rollout_candidate.so"
ACTUAL="$(sha256sum "$ELF" | awk '{print $1}')"
LENGTH="$(wc -c < "$ELF" | tr -d ' ')"
[[ "$ACTUAL" == "$EXPECTED_SHA256" && "$LENGTH" == "$EXPECTED_LEN" ]] || {
  echo "Unexpected fixture identity: $ACTUAL ($LENGTH bytes); expected $EXPECTED_SHA256 ($EXPECTED_LEN bytes)" >&2
  exit 1
}
cmp "$ELF" "$TRACKED" || { echo "Tracked rollout fixture differs from the reproduced build" >&2; exit 1; }
cp "$ELF" "$OUT/fixture_stake_pool_rollout_v2.so"
python3 - "$STAGE/source" "$OUT" "$TOOLS" "$ROOT/scripts/build-stake-pool-rollout-candidate.sh" <<'PY'
import hashlib, json, pathlib, platform, sys
source, out, tools, script = map(pathlib.Path, sys.argv[1:])
def commitment(p):
    b = p.read_bytes()
    return {"sha256": hashlib.sha256(b).hexdigest(), "len": len(b)}
receipt = {
    "schema": "eplyx-stake-pool-rollout-fixture-build-v1",
    "origin": "constructed rollout counterexample; not upstream release; not intended for deployment",
    "host": {"system": platform.system(), "machine": platform.machine()},
    "cargo_build_sbf": "4.0.0", "platform_tools": "v1.54", "rustc": "1.89.0-dev", "arch": "v0",
    "offline": True, "source_only_staging": True, "fresh_target_and_output": True,
    "public_dependency_and_tool_caches_reused": True,
    "platform_tools_revisions": commitment(tools / "version.md"),
    "build_script": commitment(script),
    "files": {p: commitment(source / p) for p in ["Cargo.toml", "Cargo.lock", "src/lib.rs", "NOTICE", "LICENSE"]},
    "elf": commitment(out / "fixture_stake_pool_rollout_v2.so")
}
(out / "fixture_stake_pool_rollout_v2.build.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
PY
echo "Qualified identity: $ACTUAL ($LENGTH bytes); source-only staging: $STAGE"
