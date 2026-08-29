CREATE TABLE devices (
    device_id TEXT PRIMARY KEY,
    last_seen_at INTEGER NOT NULL,
    last_cursor INTEGER NOT NULL DEFAULT 0 CHECK (last_cursor >= 0),
    revoked_at INTEGER
);

CREATE TABLE operations (
    server_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    local_seq INTEGER NOT NULL CHECK (local_seq >= 0),
    hlc_wall_ms INTEGER NOT NULL CHECK (hlc_wall_ms >= 0),
    hlc_counter INTEGER NOT NULL CHECK (hlc_counter >= 0),
    entity_type TEXT NOT NULL CHECK (entity_type IN ('subscription', 'episode_playback', 'queue_entry')),
    entity_key TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    created_at INTEGER NOT NULL,
    UNIQUE(device_id, local_seq)
);

CREATE INDEX operations_server_seq_idx ON operations(server_seq);
CREATE INDEX operations_device_local_seq_idx ON operations(device_id, local_seq);

CREATE TABLE snapshots (
    snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,
    base_server_seq INTEGER NOT NULL CHECK (base_server_seq >= 0),
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    created_at INTEGER NOT NULL
);

