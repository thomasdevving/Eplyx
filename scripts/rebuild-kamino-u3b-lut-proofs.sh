#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
CARGO_NET_OFFLINE=true cargo build --offline -q -p eplyx-engine --example reconstruct_lut
exec python3 scripts/kamino_u3b_lut.py "$@"
