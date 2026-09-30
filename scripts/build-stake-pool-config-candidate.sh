#!/usr/bin/env bash
# Constructed two-instruction test fixture; not a deployment command.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/programs/fixture-stake-pool-config-candidate"
OUT="${1:-$ROOT/artifacts}"
EXPECTED_SHA256="a664f74b73dedc713f16934829b25f9a0c0c3a06c6ce21f03fdc7869ae5b555d"
EXPECTED_LEN=133992
BUILDER="${EPLYX_SBF_BUILDER:-cargo-build-sbf}"
TOOLS="${EPLYX_SBF_TOOLS:-$HOME/.cache/solana/v1.54/platform-tools}"
[[ "$(uname -sm)" == "Darwin arm64" ]] || { echo 'Only Darwin arm64 is qualified' >&2; exit 1; }
VERSION="$("$BUILDER" --version)"
[[ "${VERSION%%$'\n'*}" == 'cargo-build-sbf 4.4.0' ]] || { echo 'Expected cargo-build-sbf 4.4.0' >&2; exit 1; }
[[ "$("$TOOLS/rust/bin/rustc" --version)" == 'rustc 1.89.0-dev' ]] || { echo 'Expected cached platform-tools v1.54 rustc' >&2; exit 1; }
# Version commitments identify the qualified platform-tools source revisions.
[[ "$(shasum -a 256 "$TOOLS/version.md" | awk '{print $1}')" == 'c33d3fa3cae3d9ece4ea08bc8e51a754041286a351067bc3afd2635595619895' ]] || { echo 'Unexpected platform-tools source revisions' >&2; exit 1; }
STAGE="$(mktemp -d "${TMPDIR:-/private/tmp}/eplyx-stake-config-build.XXXXXX")"
mkdir -p "$STAGE/source/src" "$STAGE/output" "$OUT"
cp "$SOURCE/Cargo.toml" "$SOURCE/Cargo.lock" "$SOURCE/NOTICE" "$SOURCE/LICENSE" "$STAGE/source/"
cp "$SOURCE/src/lib.rs" "$STAGE/source/src/"
export RUSTC="$TOOLS/rust/bin/rustc"
export CARGO_TARGET_DIR="$STAGE/source/target"
# Each invocation has a new source/target/output tree. Only public dependency
# and platform-tool caches are reused. No rustup mutation or network installation.
"$BUILDER" --tools-version v1.54 --arch v0 --skip-tools-install --no-rustup-override --offline \
  --manifest-path "$STAGE/source/Cargo.toml" --sbf-out-dir "$STAGE/output" -- --locked
ELF="$STAGE/output/fixture_stake_pool_config_candidate.so"
ACTUAL="$(shasum -a 256 "$ELF" | awk '{print $1}')"
LENGTH="$(wc -c < "$ELF" | tr -d ' ')"
[[ "$ACTUAL" == "$EXPECTED_SHA256" && "$LENGTH" == "$EXPECTED_LEN" ]] || {
  echo "Unexpected fixture identity: $ACTUAL ($LENGTH bytes); expected $EXPECTED_SHA256 ($EXPECTED_LEN bytes)" >&2
  exit 1
}
cp "$ELF" "$OUT/fixture_stake_pool_config_v2.so"
python3 - "$STAGE/source" "$OUT" "$TOOLS" "$ROOT/scripts/build-stake-pool-config-candidate.sh" <<'PY'
import hashlib, json, pathlib, platform, sys
source, out, tools, script = map(pathlib.Path, sys.argv[1:])
def commitment(p):
    b = p.read_bytes()
    return {"sha256": hashlib.sha256(b).hexdigest(), "len": len(b)}
receipt = {
    "schema": "eplyx-stake-pool-config-fixture-build-v1",
    "origin": "constructed test fixture; not upstream release; not for deployment",
    "host": {"system": platform.system(), "machine": platform.machine(), "macos": platform.mac_ver()[0]},
    "cargo_build_sbf": "4.4.0", "platform_tools": "v1.54", "rustc": "1.89.0-dev", "arch": "v0",
    "offline": True, "source_only_staging": True, "fresh_target_and_output": True,
    "public_dependency_and_tool_caches_reused": True, "fresh_network_installation_tested": False,
    "platform_tools_revisions": commitment(tools / "version.md"),
    "build_script": commitment(script),
    "files": {p: commitment(source / p) for p in ["Cargo.toml", "Cargo.lock", "src/lib.rs", "NOTICE", "LICENSE"]},
    "elf": commitment(out / "fixture_stake_pool_config_v2.so")
}
(out / "fixture_stake_pool_config_v2.build.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
PY
echo "Qualified identity: $ACTUAL ($LENGTH bytes); source-only staging: $STAGE"
