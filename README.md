# StateSync Lite NAS Server

Single-user StateSync Lite v1 server for the AntennaPod Android fork and FennecCast iOS client. It stores only subscription state, episode playback state and queue state in a SQLite WAL operation log. It never accepts podcast databases, media files, downloads, Feed XML or application settings.

## Run on a NAS

Requirements: Docker Compose, a private Tailscale path or an HTTPS reverse proxy, and a host user that owns the data directories.

```sh
cp .env.example .env
mkdir -p data backups secrets
openssl rand -hex 32 > secrets/sync_token
chmod 600 secrets/sync_token
docker compose up -d --build
curl http://127.0.0.1:8080/healthz
```

Set `PUID` and `PGID` in `.env` to the owner of `data`, `backups` and `secrets`. The default published address is `127.0.0.1`; keep it for a same-host HTTPS reverse proxy. For Tailscale-only access, set `STATE_SYNC_BIND_ADDRESS` to the NAS Tailscale IP. Do not publish plain HTTP on a LAN or the public internet.

Configure both mobile clients with the base HTTPS URL, such as `https://podcast-sync.example.net`. Do not append `/v1/sync`; both clients append it themselves. Enter the exact content of `secrets/sync_token` as the token.

## API

- `POST /v1/sync`: atomic push/pull, 500 mutations and 1 MiB per request.
- `GET /v1/snapshots/latest`: latest normalized Snapshot v1.
- `PUT /v1/snapshots`: store a Snapshot v1; only the latest 14 are retained.
- `DELETE /v1/devices/{device_id}`: revoke a device ID.
- `GET /healthz`: unauthenticated database health check.

All `/v1` routes require `Authorization: Bearer <token>`. The server accepts at most 16 active device IDs and defaults to 120 protected requests per minute. Rotate the shared token after revoking a lost device because a device that still knows the token can choose a new ID.

The server normalizes Android operations by copying the envelope `entity_key` into `payload.feed_key` or `payload.episode_key`. This makes stored operations decodable by the stricter iOS model without changing either client. Queue order tokens are restricted to the clients' shared 16-character lowercase hexadecimal format and cannot be zero.

## Backup and restore

Run `./scripts/backup.sh` from the NAS scheduler once per day. It checkpoints WAL, creates a consistent SQLite backup under `backups/`, and removes backups older than 14 days.

To restore, stop the service, preserve the current `data/` directory, copy one selected backup to `data/sync.db`, remove only the stale `sync.db-wal` and `sync.db-shm` sidecars, then start the service and check `/healthz`. Clients with a cursor ahead of the restored server receive `409 cursor_ahead`; reset their StateSync configuration so they bootstrap from cursor zero.

## Local development

Rust 1.94 or newer is required.

```sh
export STATE_SYNC_TOKEN=0123456789abcdef0123456789abcdef
export STATE_SYNC_DATABASE_PATH=/tmp/state-sync.db
cargo run
cargo test --all-targets
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

The protocol contract is in [openapi.yaml](openapi.yaml), with standalone schemas and fixtures under [protocol](protocol/).

## Framework references

- Axum 0.8 request limits and server API: https://docs.rs/axum/0.8.9/axum/extract/struct.DefaultBodyLimit.html and https://docs.rs/axum/0.8.9/axum/fn.serve.html
- SQLx 0.9 SQLite WAL, busy timeout, raw SQL and checked row access: https://docs.rs/sqlx/0.9.0/sqlx/sqlite/struct.SqliteConnectOptions.html, https://docs.rs/sqlx/0.9.0/sqlx/fn.raw_sql.html and https://docs.rs/sqlx/0.9.0/sqlx/trait.Row.html
- tower-http 0.7 response headers: https://docs.rs/tower-http/0.7.0/tower_http/set_header/
- subtle 2.6 constant-time equality: https://docs.rs/subtle/2.6.1/subtle/trait.ConstantTimeEq.html

License: AGPL-3.0-only.
