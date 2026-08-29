use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use podcast_sync_server::{AppState, connect_database, router};
use serde_json::{Value, json};
use sqlx::Row;
use tempfile::TempDir;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

struct TestApp {
    _directory: TempDir,
    state: AppState,
}

impl TestApp {
    async fn new() -> Self {
        Self::with_rate_limit(1_000).await
    }

    async fn with_rate_limit(requests_per_minute: u32) -> Self {
        let directory = TempDir::new().unwrap();
        let database_path = directory.path().join("sync.db");
        let pool = connect_database(&database_path).await.unwrap();
        Self {
            _directory: directory,
            state: AppState::new(pool, TOKEN.to_owned(), requests_per_minute).unwrap(),
        }
    }

    async fn request(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response = router(self.state.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1_048_576).await.unwrap();
        let value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        };
        (status, value)
    }

    async fn sync(&self, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/sync")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        self.request(request.body(Body::from(body.to_string())).unwrap())
            .await
    }
}

fn subscription(local_sequence: i64, feed_key: &str, include_payload_key: bool) -> Value {
    let mut payload = json!({
        "feed_url": "https://example.com/feed.xml",
        "subscribed": true
    });
    if include_payload_key {
        payload["feed_key"] = json!(feed_key);
    }
    json!({
        "local_seq": local_sequence,
        "hlc": { "wall_ms": 1_787_935_800_123_i64, "counter": 2 },
        "entity_type": "subscription",
        "entity_key": feed_key,
        "payload": payload
    })
}

#[tokio::test]
async fn health_is_public_and_protected_routes_require_bearer_authentication() {
    let app = TestApp::new().await;
    let (status, body) = app
        .request(Request::get("/healthz").body(Body::empty()).unwrap())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"status": "ok"}));

    let request = json!({"device_id": "ios-phone", "cursor": 0, "mutations": []});
    assert_eq!(
        app.sync(None, request.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.sync(Some("wrong-token"), request).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn sync_is_idempotent_does_not_echo_and_normalizes_android_payloads_for_ios() {
    let app = TestApp::new().await;
    let feed_key = "feed_0123456789abcdef";
    let push = json!({
        "device_id": "android-phone",
        "cursor": 0,
        "mutations": [subscription(7, feed_key, false)]
    });

    let (status, first) = app.sync(Some(TOKEN), push.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["accepted_local_seq"], 7);
    assert_eq!(first["changes"], json!([]));
    assert_eq!(first["next_cursor"], 1);

    let (status, retry) = app.sync(Some(TOKEN), push).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(retry, first);

    let pull = json!({"device_id": "ios-phone", "cursor": 0, "mutations": []});
    let (status, response) = app.sync(Some(TOKEN), pull).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["accepted_local_seq"], 0);
    assert_eq!(response["next_cursor"], 1);
    assert_eq!(response["changes"].as_array().unwrap().len(), 1);
    assert_eq!(response["changes"][0]["server_seq"], 1);
    assert_eq!(response["changes"][0]["device_id"], "android-phone");
    assert_eq!(response["changes"][0]["payload"]["feed_key"], feed_key);

    let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM operations")
        .fetch_one(app.state.pool())
        .await
        .unwrap()
        .get("count");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn queue_tokens_round_trip_between_android_and_ios() {
    let app = TestApp::new().await;
    let episode_key = "episode_0123456789abcdef";
    let mutation = |order_token: &str| {
        json!({
            "local_seq": 1,
            "hlc": { "wall_ms": 1_787_935_800_123_i64, "counter": 0 },
            "entity_type": "queue_entry",
            "entity_key": episode_key,
            "payload": { "order_token": order_token, "present": true }
        })
    };

    let invalid = json!({
        "device_id": "android-phone",
        "cursor": 0,
        "mutations": [mutation("7FFFFFFFFFFFFFFF")]
    });
    assert_eq!(
        app.sync(Some(TOKEN), invalid).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let push = json!({
        "device_id": "android-phone",
        "cursor": 0,
        "mutations": [mutation("8000000000000000")]
    });
    assert_eq!(app.sync(Some(TOKEN), push).await.0, StatusCode::OK);

    let pull = json!({"device_id": "ios-phone", "cursor": 0, "mutations": []});
    let (status, response) = app.sync(Some(TOKEN), pull).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response["changes"][0]["payload"]["episode_key"],
        episode_key
    );
    assert_eq!(
        response["changes"][0]["payload"]["order_token"],
        "8000000000000000"
    );
}

#[tokio::test]
async fn reusing_a_local_sequence_with_different_content_is_rejected() {
    let app = TestApp::new().await;
    let first = json!({
        "device_id": "android-phone",
        "cursor": 0,
        "mutations": [subscription(1, "feed_first", false)]
    });
    assert_eq!(app.sync(Some(TOKEN), first).await.0, StatusCode::OK);

    let conflicting = json!({
        "device_id": "android-phone",
        "cursor": 0,
        "mutations": [subscription(1, "feed_second", false)]
    });
    let (status, body) = app.sync(Some(TOKEN), conflicting).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "sequence_conflict");
}

#[tokio::test]
async fn revoked_devices_cannot_sync() {
    let app = TestApp::new().await;
    let register = json!({
        "device_id": "lost-phone",
        "cursor": 0,
        "mutations": [subscription(1, "feed_lost", false)]
    });
    assert_eq!(app.sync(Some(TOKEN), register).await.0, StatusCode::OK);

    let revoke = Request::builder()
        .method("DELETE")
        .uri("/v1/devices/lost-phone")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.request(revoke).await.0, StatusCode::NO_CONTENT);

    let retry = json!({"device_id": "lost-phone", "cursor": 1, "mutations": []});
    assert_eq!(app.sync(Some(TOKEN), retry).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn latest_snapshot_round_trips_as_normalized_json() {
    let app = TestApp::new().await;
    let snapshot = json!({
        "protocol_version": 1,
        "base_server_seq": 0,
        "created_at": 1_787_935_800_123_i64,
        "subscriptions": [],
        "episode_playback_states": [],
        "queue_entries": [],
        "aliases": [],
        "tombstones": []
    });
    let put = Request::builder()
        .method("PUT")
        .uri("/v1/snapshots")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(snapshot.to_string()))
        .unwrap();
    assert_eq!(app.request(put).await.0, StatusCode::CREATED);

    let get = Request::builder()
        .uri("/v1/snapshots/latest")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (status, response) = app.request(get).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["snapshot"], snapshot);
    assert_eq!(response["snapshot_id"], 1);
}

#[tokio::test]
async fn invalid_protocol_entities_and_oversized_bodies_are_rejected() {
    let app = TestApp::new().await;
    let invalid = json!({
        "device_id": "ios-phone",
        "cursor": 0,
        "mutations": [{
            "local_seq": 1,
            "hlc": { "wall_ms": 1, "counter": 0 },
            "entity_type": "download",
            "entity_key": "file_1",
            "payload": { "path": "/private/file.mp3" }
        }]
    });
    let (status, body) = app.sync(Some(TOKEN), invalid).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "invalid_request");

    let oversized = json!({
        "device_id": "ios-phone",
        "cursor": 0,
        "mutations": [],
        "padding": "x".repeat(1_048_576)
    });
    assert_eq!(
        app.sync(Some(TOKEN), oversized).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn a_failed_batch_rolls_back_earlier_inserts() {
    let app = TestApp::new().await;
    let initial = json!({
        "device_id": "android-phone",
        "cursor": 0,
        "mutations": [subscription(2, "feed_existing", false)]
    });
    assert_eq!(app.sync(Some(TOKEN), initial).await.0, StatusCode::OK);

    let conflicting_batch = json!({
        "device_id": "android-phone",
        "cursor": 0,
        "mutations": [
            subscription(1, "feed_new", false),
            subscription(2, "feed_conflict", false)
        ]
    });
    assert_eq!(
        app.sync(Some(TOKEN), conflicting_batch).await.0,
        StatusCode::CONFLICT
    );
    let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM operations")
        .fetch_one(app.state.pool())
        .await
        .unwrap()
        .get("count");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn protected_routes_are_rate_limited() {
    let app = TestApp::with_rate_limit(2).await;
    let request = json!({"device_id": "ios-phone", "cursor": 0, "mutations": []});
    assert_eq!(
        app.sync(Some(TOKEN), request.clone()).await.0,
        StatusCode::OK
    );
    assert_eq!(
        app.sync(Some(TOKEN), request.clone()).await.0,
        StatusCode::OK
    );
    let (status, body) = app.sync(Some(TOKEN), request).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
}

#[tokio::test]
async fn active_devices_and_snapshot_history_are_bounded() {
    let app = TestApp::new().await;
    for index in 0..16 {
        let request = json!({
            "device_id": format!("device-{index}"),
            "cursor": 0,
            "mutations": []
        });
        assert_eq!(app.sync(Some(TOKEN), request).await.0, StatusCode::OK);
    }
    let overflow = json!({"device_id": "device-16", "cursor": 0, "mutations": []});
    let (status, body) = app.sync(Some(TOKEN), overflow).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "device_limit");

    for index in 0..15 {
        let snapshot = json!({
            "protocol_version": 1,
            "base_server_seq": 0,
            "created_at": index,
            "subscriptions": [],
            "episode_playback_states": [],
            "queue_entries": [],
            "aliases": [],
            "tombstones": []
        });
        let put = Request::builder()
            .method("PUT")
            .uri("/v1/snapshots")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(snapshot.to_string()))
            .unwrap();
        assert_eq!(app.request(put).await.0, StatusCode::CREATED);
    }
    let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM snapshots")
        .fetch_one(app.state.pool())
        .await
        .unwrap()
        .get("count");
    assert_eq!(count, 14);
}
