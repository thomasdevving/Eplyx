#!/usr/bin/env bash
# Lay out the working directories the recorded terminal sessions run in.
# Every input is a tracked repository file or a hash-checked CI artefact;
# nothing here is generated to make a result look a certain way.
#
#   ~/stake-pool-program   a protocol repo: .eplyx/bundle (deploy/bundle),
#                          .eplyx/expected-changes.toml (docs/pilot bounded case),
#                          target/deploy/*.so (pinned baseline and the
#                          constructed regression fixture from CI)
#   ~/token-migration      examples/migrations/minimal + the reference mechanism
#   ~/lifecycle            synthetic snapshot/scenario from the dashboard_records example
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
HOME_DIR="${1:-/home/demo}"
rm -rf "$HOME_DIR"; mkdir -p "$HOME_DIR"

sp="$HOME_DIR/stake-pool-program"
mkdir -p "$sp/.eplyx" "$sp/target/deploy" "$sp/proposals"
cp -R "$REPO/deploy/bundle" "$sp/.eplyx/bundle"
cp "$REPO/docs/pilot/expected-changes.bounded.toml" "$sp/.eplyx/expected-changes.toml"
cp "$REPO/artifacts/fixture_stake_pool_v2.so" "$sp/target/deploy/fixture_stake_pool_v2.so"
cp "$REPO/artifacts/fixture_stake_pool_config_v2.so" "$sp/target/deploy/fixture_stake_pool_config_v2.so"
cp "$REPO/docs/examples/stake-pool-parameter-change.json" "$sp/proposals/deposit-fee-1pct.json"
cp "$REPO/docs/examples/stake-pool-config-upgrade-change.json" "$sp/proposals/config-upgrade.json"

mg="$HOME_DIR/token-migration"
cp -R "$REPO/examples/migrations/minimal" "$mg"
mkdir -p "$mg/target/deploy"
cp "$REPO/artifacts/eplyx_token_migration.so" "$mg/target/deploy/migration.so"

# the deliberately defective migration build used to show a counterexample
mkdir -p "$HOME_DIR/builds"
cp "$REPO/artifacts/eplyx_token_migration_defect_deadline_inclusive.so" "$HOME_DIR/builds/migration-deadline-defect.so"

# the CI client a team vendors from examples/github
mkdir -p "$sp/scripts"
cp "$REPO/scripts/eplyx-submit.sh" "$sp/scripts/eplyx-submit.sh"

# a real Squads V4 proposal's governance-bound ChangeSpec (G1.1 witness)
mkdir -p "$HOME_DIR/squads"
cp "$REPO/docs/examples/phase-g1-1-squads-mainnet/witness-primary/bound-change-spec.json" "$HOME_DIR/squads/bound.json"

# historical research starts from an empty directory; the constructed
# Token-2022 regression fixture is kept next to it
mkdir -p "$HOME_DIR/history" "$HOME_DIR/candidates"
cp "$REPO/artifacts/fixture_token2022_v2.so" "$HOME_DIR/candidates/fixture_token2022_v2.so"

lc="$HOME_DIR/lifecycle"
mkdir -p "$lc"
env -i "$REPO/target/release/examples/dashboard_records" "$lc/inputs" >/dev/null
echo "demo home ready: $HOME_DIR"
