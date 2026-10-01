#!/bin/sh
# Review-only absolute deadline. Restarting never renews the review period.
set -eu
case "${EPLYX_REVIEW_EXPIRES_UNIX:-}" in
  ''|*[!0-9]*) echo "Review expiry missing or invalid" >&2; exit 64 ;;
esac
[ "$#" -gt 0 ] || exit 64
remaining=$((EPLYX_REVIEW_EXPIRES_UNIX - $(date -u +%s)))
if [ "$remaining" -le 0 ]; then
  echo "Review expired; compute will not start" >&2
  exit 0
fi
# GNU timeout signals the process group, including analytical child workers.
exec timeout --signal=TERM --kill-after=10s "${remaining}s" "$@"
