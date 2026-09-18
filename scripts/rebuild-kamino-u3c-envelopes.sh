#!/usr/bin/env bash
set -eo pipefail
cd "$(dirname "$0")/.."
cargo build --offline -q -p eplyx-engine --example audit_kamino_envelope --example acquire_envelope_dependencies
flags=()
output=""
while (($#)); do
  case "$1" in
    --verify|--reverse) flags+=("$1"); shift ;;
    --output) output="$2"; shift 2 ;;
    *) echo "Usage: $0 [--verify] [--reverse] [--output directory]" >&2; exit 2 ;;
  esac
done
if [[ -n "$output" ]]; then
  python3 scripts/kamino_u3c_envelope.py "${flags[@]}" --output "$output/envelope"
  python3 scripts/kamino_u3c_dependencies.py "${flags[@]}" --output "$output/dependencies"
else
  python3 scripts/kamino_u3c_envelope.py "${flags[@]}"
  python3 scripts/kamino_u3c_dependencies.py "${flags[@]}"
fi
