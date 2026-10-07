#!/usr/bin/env bash
# Record `eplyx login/link/sync` while the browser approves the real device code.
set -euo pipefail
cd "$(dirname "$0")"
live=../assets/casts/sync.cast.live
rm -f "$live"
node sessions.mjs sync &
rec=$!
for i in $(seq 1 120); do
  url=$(grep -o 'http://127.0.0.1:4390/device[^ ]*' "$live" 2>/dev/null | head -1 || true)
  [ -n "$url" ] && break; sleep 0.5
done
[ -n "$url" ] || { echo "no device URL"; kill $rec; exit 1; }
sleep 1.2
DEVICE_URL="$url" node capture-web.mjs device
wait $rec
