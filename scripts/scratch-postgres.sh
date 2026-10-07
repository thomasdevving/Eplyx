#!/usr/bin/env bash
# A throwaway loopback Postgres cluster for the cloud suites.
#
#   eval "$(scripts/scratch-postgres.sh start [DIR] [PORT])"   # exports the URL
#   scripts/scratch-postgres.sh stop [DIR]
#
# The cluster lives entirely under DIR, listens on 127.0.0.1 only, trusts the
# local superuser and holds nothing but databases the tests create and drop.
# `start` prints `export EPLYX_CLOUD_TEST_DATABASE_URL=...` on stdout and
# everything else on stderr, so it composes with `eval` and with
# `>> "$GITHUB_ENV"` after stripping the `export ` prefix.
set -euo pipefail

command="${1:-}"
dir="${2:-${TMPDIR:-/tmp}/eplyx-scratch-postgres}"
port="${3:-54329}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: $1 not found; install PostgreSQL (e.g. brew install postgresql@17) and put its bin directory on PATH" >&2
    exit 2
  }
}

case "$command" in
  start)
    need initdb
    need pg_ctl
    user="$(id -un)"
    if [ ! -f "$dir/PG_VERSION" ]; then
      mkdir -p "$dir"
      initdb --pgdata="$dir" --username="$user" --auth=trust --encoding=UTF8 --no-locale >&2
    fi
    if ! pg_ctl --pgdata="$dir" status >/dev/null 2>&1; then
      # macOS postmasters refuse to start under an unset or invalid locale.
      export LC_ALL=C
      # TCP on loopback only: no Unix socket, whose path length is limited
      # and which would otherwise land outside DIR.
      pg_ctl --pgdata="$dir" --log="$dir/server.log" --wait \
        -o "-h 127.0.0.1 -p $port -k ''" start >&2
    fi
    echo "export EPLYX_CLOUD_TEST_DATABASE_URL=postgres://$user@127.0.0.1:$port/postgres"
    ;;
  stop)
    need pg_ctl
    if [ -f "$dir/PG_VERSION" ]; then
      pg_ctl --pgdata="$dir" --mode=fast --wait stop >&2 || true
    fi
    ;;
  *)
    echo "usage: $0 start|stop [DIR] [PORT]" >&2
    exit 2
    ;;
esac
