#!/usr/bin/env bash
# Back up and restore a hosted Eplyx service: the data volume and Postgres
# identity storage, together.
#
#   scripts/eplyx-backup.sh backup  --data-dir DIR --database-url URL --out BACKUP
#   scripts/eplyx-backup.sh restore --from BACKUP --data-dir DIR --database-url URL
#
# The two stores hold different truths. The volume owns every project, run,
# report, bundle and artefact; Postgres owns users, sessions, workspaces,
# workspace->project assignment and user-issued tokens. A restore that pairs a
# volume and a database from different moments starts a service whose halves
# disagree, so:
#
#   * backup dumps Postgres FIRST and copies the volume SECOND. The volume is
#     append-mostly, so every project the dump assigns is already on disk when
#     the copy is taken.
#   * restore refuses a non-empty data directory or a database that already
#     has tables, restores both, and then runs `eplyx-server admin
#     verify-volume`, which compares them. Its exit code is this script's.
#
# Token revocations and sessions written after the dump are not in it: rotate
# project tokens and expect users to sign in again after a restore.
#
# Needs pg_dump/pg_restore/psql on PATH and the eplyx-server binary
# (EPLYX_SERVER, default target/release/eplyx-server, then target/debug).
set -euo pipefail

usage() {
  sed -n '2,8p' "$0" >&2
  exit 2
}

command="${1:-}"
[ -n "$command" ] || usage
shift
data_dir="" database_url="" out="" from=""
while [ $# -gt 0 ]; do
  case "$1" in
    --data-dir) data_dir="$2"; shift 2 ;;
    --database-url) database_url="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --from) from="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$data_dir" ] && [ -n "$database_url" ] || usage
for tool in pg_dump pg_restore psql tar; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool not found" >&2; exit 2; }
done

sha256() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  else sha256sum "$1" | awk '{print $1}'; fi
}

server_binary() {
  local root
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  for candidate in "${EPLYX_SERVER:-}" "$root/target/release/eplyx-server" "$root/target/debug/eplyx-server"; do
    [ -n "$candidate" ] && [ -x "$candidate" ] && { echo "$candidate"; return; }
  done
  echo "error: eplyx-server not found; build it or set EPLYX_SERVER" >&2
  exit 2
}

case "$command" in
  backup)
    [ -n "$out" ] || usage
    [ -d "$data_dir" ] || { echo "error: $data_dir is not a directory" >&2; exit 2; }
    [ ! -e "$out" ] || { echo "error: $out already exists; a backup is written once" >&2; exit 2; }
    mkdir -p "$out"
    # 1. Postgres first.
    pg_dump --format=custom --no-owner --no-privileges --file="$out/identity.dump" "$database_url"
    # 2. The volume second. Scratch and lock files are not state.
    tar -C "$data_dir" \
      --exclude='./.lock' --exclude='./.ready' \
      --exclude='./artifacts/tmp' --exclude='./runs/*/work' \
      -cf "$out/volume.tar" .
    cat > "$out/manifest.json" <<JSON
{
  "schema_version": 1,
  "created_at_unix_seconds": $(date +%s),
  "order": ["identity.dump", "volume.tar"],
  "identity_sha256": "$(sha256 "$out/identity.dump")",
  "volume_sha256": "$(sha256 "$out/volume.tar")"
}
JSON
    echo "backup written to $out" >&2
    ;;
  restore)
    [ -n "$from" ] || usage
    for part in manifest.json identity.dump volume.tar; do
      [ -f "$from/$part" ] || { echo "error: $from/$part is missing" >&2; exit 2; }
    done
    for part in identity volume; do
      file="$from/$part.dump"; [ "$part" = volume ] && file="$from/volume.tar"
      want="$(sed -n "s/.*\"${part}_sha256\": \"\([0-9a-f]*\)\".*/\1/p" "$from/manifest.json")"
      [ "$(sha256 "$file")" = "$want" ] || { echo "error: $file does not match its manifest hash" >&2; exit 2; }
    done
    if [ -e "$data_dir" ] && [ -n "$(ls -A "$data_dir" 2>/dev/null)" ]; then
      echo "error: $data_dir is not empty; restore into a fresh volume" >&2
      exit 2
    fi
    tables="$(psql --no-psqlrc -tA -c "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'" "$database_url")"
    [ "$tables" = 0 ] || { echo "error: the target database already has tables; restore into an empty one" >&2; exit 2; }
    pg_restore --no-owner --no-privileges --exit-on-error --dbname="$database_url" "$from/identity.dump"
    mkdir -p "$data_dir"
    tar -C "$data_dir" -xf "$from/volume.tar"
    echo "restored; verifying the two halves agree" >&2
    EPLYX_DATA_DIR="$data_dir" EPLYX_DATABASE_URL="$database_url" \
      "$(server_binary)" admin verify-volume
    ;;
  *) usage ;;
esac
