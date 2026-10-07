#!/usr/bin/env bash
# The full local regression gate: every suite, including the ones that read
# fixture payloads kept out of git on purpose (scripts/private-fixture-suites.txt).
#
# GitHub CI runs the public tier: everything a clean checkout can build. This is
# the rest, and it is what has to pass before a change lands on main. Nothing is
# skipped: missing payloads, artefacts or browsers stop the run with the command
# that supplies them.
#
#   ./scripts/regression.sh            (or: make regression)
#
# Postgres: uses EPLYX_CLOUD_TEST_DATABASE_URL, or starts a loopback scratch
# cluster for the duration of the run. Browser: EPLYX_CHROME, else Google
# Chrome, else Playwright's bundled Chromium.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# cargo may only be reachable through rustup.
if ! command -v cargo >/dev/null 2>&1 && command -v rustup >/dev/null 2>&1; then
  PATH="$(dirname "$(rustup which cargo)"):$PATH"
fi
[ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"
export PATH

step() {
  echo
  echo "==> $*"
  "$@"
}

python3 scripts/public-tier.py require-private
ls artifacts/*.so >/dev/null 2>&1 || {
  echo "error: no SBF artefacts in artifacts/; run make test-artifacts first" >&2
  exit 2
}

if [ -z "${EPLYX_CLOUD_TEST_DATABASE_URL:-}" ]; then
  scratch="${TMPDIR:-/tmp}/eplyx-regression-postgres"
  eval "$(scripts/scratch-postgres.sh start "$scratch")"
  trap 'scripts/scratch-postgres.sh stop "$scratch"' EXIT
fi

if [ -z "${EPLYX_CHROME:-}" ] && [ ! -d "/Applications/Google Chrome.app" ]; then
  EPLYX_CHROME="$(node -e "console.log(require('@playwright/test').chromium.executablePath())" 2>/dev/null || true)"
  [ -n "$EPLYX_CHROME" ] && [ -x "$EPLYX_CHROME" ] || {
    echo "error: no browser; install Google Chrome, set EPLYX_CHROME, or run pnpm exec playwright install chromium" >&2
    exit 2
  }
  export EPLYX_CHROME
fi

step make fmt-check
step make lint
step make test-programs
step cargo test --locked --workspace
step cargo build --locked -p eplyx-engine -p eplyx-server --bins
step pnpm check:frontend
step pnpm test:frontend-runtime
step pnpm verify:report
step pnpm verify:governance
step pnpm check:legal
step python3 scripts/test_eplyx_submit.py
step python3 scripts/test_eplyx_pr_comment.py
step python3 scripts/public-tier.py check
step pnpm test:dashboard
step pnpm test:cloud
step pnpm test:public
step pnpm test:governance-browser

echo
echo "regression: every suite passed"
