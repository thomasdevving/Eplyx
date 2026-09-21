#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Recompute sealed LUT/envelope proofs, every captured ELF and the existing
# same-slot screen from retained responses before deriving the input inventory.
bash scripts/rebuild-kamino-u3c-envelopes.sh --verify
python3 scripts/kamino_u3d_inventory.py "$@"
