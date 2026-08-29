use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MAX_DEVICE_ID_BYTES: usize = 128;
pub const MAX_ENTITY_KEY_BYTES: usize = 256;
pub const MAX_MUTATIONS: usize = 500;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncRequest {
    pub device_id: String,
    pub cursor: i64,
    pub mutations: Vec<Mutation>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HlcTimestamp {
    pub wall_ms: i64,
    pub counter: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Subscription,
    EpisodePlayback,
    QueueEntry,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Subscription => "subscription",
            Self::EpisodePlayback => "episode_playback",
            Self::QueueEntry => "queue_entry",
        }
    }

    pub fn from_database(value: &str) -> Result<Self, &'static str> {
        match value {
            "subscription" => Ok(Self::Subscription),
            "episode_playback" => Ok(Self::EpisodePlayback),
            "queue_entry" => Ok(Self::QueueEntry),
            _ => Err("invalid stored entity type"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub local_seq: i64,
    pub hlc: HlcTimestamp,
    pub entity_type: EntityType,
    pub entity_key: String,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct SyncChange {
    pub server_seq: i64,
    pub device_id: String,
    pub hlc: HlcTimestamp,
    pub entity_type: EntityType,
    pub entity_key: String,
    pub payload: Value,
}

#[derive(Debug, Serialize)]
pub struct SyncResponse {
    pub accepted_local_seq: i64,
    pub next_cursor: i64,
    pub changes: Vec<SyncChange>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub protocol_version: u32,
    pub base_server_seq: i64,
    pub created_at: i64,
    pub subscriptions: Vec<Value>,
    pub episode_playback_states: Vec<Value>,
    pub queue_entries: Vec<Value>,
    pub aliases: Vec<Value>,
    pub tombstones: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotResponse {
    pub snapshot_id: i64,
    pub snapshot: Snapshot,
}

impl SyncRequest {
    pub fn validate_and_normalize(&mut self) -> Result<(), &'static str> {
        validate_device_id(&self.device_id)?;
        if self.cursor < 0 || self.mutations.len() > MAX_MUTATIONS {
            return Err("invalid cursor or mutation count");
        }
        let mut previous = None;
        for mutation in &mut self.mutations {
            if previous.is_some_and(|value| mutation.local_seq <= value) {
                return Err("mutations must be strictly ordered");
            }
            mutation.validate_and_normalize()?;
            previous = Some(mutation.local_seq);
        }
        Ok(())
    }
}

impl Mutation {
    fn validate_and_normalize(&mut self) -> Result<(), &'static str> {
        if self.local_seq < 0 || self.hlc.wall_ms < 0 || self.hlc.counter < 0 {
            return Err("negative sequence or clock");
        }
        validate_identifier(&self.entity_key, MAX_ENTITY_KEY_BYTES)?;
        let payload = self
            .payload
            .as_object_mut()
            .ok_or("payload must be an object")?;
        match self.entity_type {
            EntityType::Subscription => validate_subscription(payload, &self.entity_key),
            EntityType::EpisodePlayback => validate_playback(payload, &self.entity_key),
            EntityType::QueueEntry => validate_queue(payload, &self.entity_key),
        }
    }
}

impl Snapshot {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.protocol_version != 1 || self.base_server_seq < 0 || self.created_at < 0 {
            return Err("invalid snapshot metadata");
        }
        for values in [
            &self.subscriptions,
            &self.episode_playback_states,
            &self.queue_entries,
            &self.aliases,
            &self.tombstones,
        ] {
            if values.len() > 50_000 || values.iter().any(|value| !value.is_object()) {
                return Err("invalid snapshot collection");
            }
        }
        Ok(())
    }
}

pub fn validate_device_id(value: &str) -> Result<(), &'static str> {
    if value.is_empty()
        || value.len() > MAX_DEVICE_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err("invalid device identifier");
    }
    Ok(())
}

fn validate_identifier(value: &str, maximum: usize) -> Result<(), &'static str> {
    if value.is_empty()
        || value.len() > maximum
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("invalid identifier");
    }
    Ok(())
}

fn validate_subscription(payload: &mut Map<String, Value>, key: &str) -> Result<(), &'static str> {
    ensure_fields(payload, &["feed_key", "feed_url", "subscribed"])?;
    normalize_key(payload, "feed_key", key)?;
    require_string(payload, "feed_url", 4_096)?;
    require_bool(payload, "subscribed")
}

fn validate_playback(payload: &mut Map<String, Value>, key: &str) -> Result<(), &'static str> {
    ensure_fields(
        payload,
        &[
            "episode_key",
            "intent",
            "position_ms",
            "duration_ms",
            "played",
            "feed_url",
            "guid",
        ],
    )?;
    normalize_key(payload, "episode_key", key)?;
    let intent = require_string(payload, "intent", 32)?;
    if ![
        "checkpoint",
        "playback_pause",
        "seek",
        "mark_played",
        "mark_unplayed",
    ]
    .contains(&intent)
    {
        return Err("invalid playback intent");
    }
    require_nonnegative_integer(payload, "position_ms")?;
    if payload.contains_key("duration_ms") {
        require_nonnegative_integer(payload, "duration_ms")?;
    }
    require_bool(payload, "played")?;
    optional_string(payload, "feed_url", 4_096)?;
    optional_string(payload, "guid", 2_048)
}

fn validate_queue(payload: &mut Map<String, Value>, key: &str) -> Result<(), &'static str> {
    ensure_fields(payload, &["episode_key", "order_token", "present"])?;
    normalize_key(payload, "episode_key", key)?;
    let order_token = require_string(payload, "order_token", 16)?;
    if order_token.len() != 16
        || order_token == "0000000000000000"
        || !order_token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid queue order token");
    }
    require_bool(payload, "present")
}

fn ensure_fields(payload: &Map<String, Value>, allowed: &[&str]) -> Result<(), &'static str> {
    if payload.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("unknown payload field");
    }
    Ok(())
}

fn normalize_key(
    payload: &mut Map<String, Value>,
    field: &str,
    key: &str,
) -> Result<(), &'static str> {
    if let Some(existing) = payload.get(field) {
        if existing.as_str() != Some(key) {
            return Err("payload key does not match envelope");
        }
    } else {
        payload.insert(field.to_owned(), Value::String(key.to_owned()));
    }
    Ok(())
}

fn require_string<'a>(
    payload: &'a Map<String, Value>,
    field: &str,
    maximum: usize,
) -> Result<&'a str, &'static str> {
    let value = payload
        .get(field)
        .and_then(Value::as_str)
        .ok_or("missing string field")?;
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err("invalid string field");
    }
    Ok(value)
}

fn optional_string(
    payload: &Map<String, Value>,
    field: &str,
    maximum: usize,
) -> Result<(), &'static str> {
    match payload.get(field) {
        Some(Value::String(value)) if !value.is_empty() && value.len() <= maximum => Ok(()),
        Some(_) => Err("invalid optional string field"),
        None => Ok(()),
    }
}

fn require_bool(payload: &Map<String, Value>, field: &str) -> Result<(), &'static str> {
    payload
        .get(field)
        .and_then(Value::as_bool)
        .map(|_| ())
        .ok_or("missing boolean field")
}

fn require_nonnegative_integer(
    payload: &Map<String, Value>,
    field: &str,
) -> Result<(), &'static str> {
    match payload.get(field).and_then(Value::as_i64) {
        Some(value) if value >= 0 => Ok(()),
        _ => Err("invalid integer field"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn android_subscription_payload_is_normalized_for_ios() {
        let mut request: SyncRequest = serde_json::from_value(json!({
            "device_id": "android-phone",
            "cursor": 0,
            "mutations": [{
                "local_seq": 1,
                "hlc": {"wall_ms": 10, "counter": 0},
                "entity_type": "subscription",
                "entity_key": "feed_1",
                "payload": {"feed_url": "https://example.com/feed", "subscribed": true}
            }]
        }))
        .unwrap();

        request.validate_and_normalize().unwrap();

        assert_eq!(request.mutations[0].payload["feed_key"], "feed_1");
    }

    #[test]
    fn protocol_whitelist_rejects_download_payloads() {
        let request = serde_json::from_value::<SyncRequest>(json!({
            "device_id": "ios-phone",
            "cursor": 0,
            "mutations": [{
                "local_seq": 1,
                "hlc": {"wall_ms": 10, "counter": 0},
                "entity_type": "download",
                "entity_key": "file_1",
                "payload": {"path": "/file.mp3"}
            }]
        }));

        assert!(request.is_err());
    }

    #[test]
    fn queue_tokens_match_both_client_fractional_index_formats() {
        let request = |order_token: &str| {
            serde_json::from_value::<SyncRequest>(json!({
                "device_id": "android-phone",
                "cursor": 0,
                "mutations": [{
                    "local_seq": 1,
                    "hlc": {"wall_ms": 10, "counter": 0},
                    "entity_type": "queue_entry",
                    "entity_key": "episode_1",
                    "payload": {"order_token": order_token, "present": true}
                }]
            }))
            .unwrap()
        };

        assert!(request("8000000000000000").validate_and_normalize().is_ok());
        for invalid in ["a", "0000000000000000", "7FFFFFFFFFFFFFFF"] {
            assert!(request(invalid).validate_and_normalize().is_err());
        }
    }

    #[test]
    fn device_ids_are_safe_in_api_paths_and_logs() {
        for valid in ["android-phone", "ios_1.0:primary"] {
            assert!(validate_device_id(valid).is_ok());
        }
        for invalid in ["phone/name", "phone name", "phone\nname"] {
            assert!(validate_device_id(invalid).is_err());
        }
    }
}
