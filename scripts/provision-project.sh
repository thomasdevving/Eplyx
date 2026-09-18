#!/usr/bin/env bash
# Provision one hosted Eplyx project from a bundle already in the image.
#
# Run it where eplyx-server and the volume are — on Railway that is
# `railway ssh`, which is why it takes no credential: the operator token is for
# the HTTP surface, and these commands read and write the registry directly.
#
# It does four things and narrates all of them: verify the baked bundle is
# there, find or create the project, register the bundle to it, and print what
# resulted. Activation is the fifth, and it is not one of them unless you ask.
#
#   ./provision-project.sh --name "Solana Stake Pool Pilot" --program SPoo1Ku8… 
#   ./provision-project.sh --name … --program … --activate
#
# Activation moves what every future pull request is measured against. A script
# that did it because it had just installed something would make the baseline
# rotate whenever somebody redeployed, which is the one thing the separation
# between registering and activating exists to prevent. So it stays behind a
# flag, and the flag is printed back at you before it is used.
set -uo pipefail

NAME="" PROGRAM="" BUNDLE_PATH=/opt/eplyx/bundle ACTIVATE=no
while [ $# -gt 0 ]; do
  case "$1" in
    --name) NAME="$2"; shift 2;;
    --program) PROGRAM="$2"; shift 2;;
    --bundle-path) BUNDLE_PATH="$2"; shift 2;;
    --activate) ACTIVATE=yes; shift;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done
[ -n "$NAME" ] || { echo "--name is required" >&2; exit 2; }
[ -n "$PROGRAM" ] || { echo "--program is required" >&2; exit 2; }
command -v eplyx-server >/dev/null || { echo "eplyx-server is not on PATH; run this on the service" >&2; exit 2; }

step() { printf '\n== %s ==\n' "$1"; }

step "1. the bundle baked into this image"
[ -f "$BUNDLE_PATH/bundle.json" ] || { echo "no bundle at $BUNDLE_PATH" >&2; exit 1; }
python3 - "$BUNDLE_PATH/bundle.json" <<'PYEOF' 2>/dev/null || echo "  (python3 unavailable; skipping the manifest summary)"
import json, sys
m = json.load(open(sys.argv[1]))
for key in ("program_id", "bundle_sha256", "baseline_sha256", "corpus_sha256"):
    value = m.get(key) or (m.get("bundle") or {}).get(key)
    if value:
        print(f"  {key:16s} {value}")
PYEOF

step "2. the project"
EXISTING=$(eplyx-server admin list-projects 2>/dev/null | awk -v n="$NAME" -F'  +' '$5 == n {print $1}' | head -1)
if [ -n "$EXISTING" ]; then
  PROJECT="$EXISTING"
  echo "  reusing $PROJECT"
else
  OUTPUT=$(eplyx-server admin create-project --name "$NAME" --program-id "$PROGRAM") || exit 1
  PROJECT=$(printf '%s' "$OUTPUT" | grep -oE 'proj_[A-Z0-9]+' | head -1)
  echo "  created $PROJECT"
fi

step "3. register the bundle to it"
# Registering verifies every hash against the bytes on disk and copies the
# bundle onto the volume, so it survives the next image build. It does not
# change what anything is measured against.
OUTPUT=$(eplyx-server admin register-bundle --project "$PROJECT" --path "$BUNDLE_PATH") || exit 1
printf '%s\n' "$OUTPUT" | sed 's/^/  /'
BUNDLE=$(printf '%s' "$OUTPUT" | grep -oE 'bndl_[A-Z0-9]+' | head -1)

step "4. activation"
if [ "$ACTIVATE" = yes ]; then
  echo "  --activate was given: pointing $PROJECT at $BUNDLE"
  eplyx-server admin activate-bundle --project "$PROJECT" --bundle "$BUNDLE" | sed 's/^/  /'
else
  echo "  not activated, because --activate was not given."
  echo "  This decides what every future pull request is compared against:"
  echo
  echo "    eplyx-server admin activate-bundle --project $PROJECT --bundle $BUNDLE"
fi

step "result"
eplyx-server admin list-projects | sed 's/^/  /'
echo
echo "Issue a CI token when you want one. It is printed once and only a hash is kept:"
echo "  eplyx-server admin create-token --project $PROJECT --label ci"
