#!/bin/sh
set -eu

project_directory=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
backup_name="sync-${timestamp}.db"

docker compose --project-directory "$project_directory" exec -T state-sync \
    sqlite3 /data/sync.db "PRAGMA wal_checkpoint(TRUNCATE);"
docker compose --project-directory "$project_directory" exec -T state-sync \
    sqlite3 /data/sync.db ".backup '/backups/${backup_name}'"
find "$project_directory/backups" -type f -name 'sync-*.db' -mtime +14 -delete
printf '%s\n' "$project_directory/backups/$backup_name"

