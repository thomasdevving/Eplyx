#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Build only the read-only audit example; Cargo cannot access the network.
CARGO_NET_OFFLINE=true cargo build --offline -q -p eplyx-engine --example classify_kamino_baseline
exec python3 scripts/kamino_u3_baseline.py "$@"
