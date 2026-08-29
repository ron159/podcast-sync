mod api;
mod auth;
mod config;
mod error;
mod model;
mod storage;

use auth::RateLimiter;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, header},
    routing::{delete, get, post, put},
};
use sqlx::SqlitePool;
use std::{
    iter::once,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tower_http::{
    sensitive_headers::SetSensitiveRequestHeadersLayer, set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};

pub use config::Config;
pub use storage::connect_database;

pub const MAXIMUM_BODY_BYTES: usize = 1_048_576;

#[derive(Clone)]
pub struct AppState {
    pool: SqlitePool,
    token: Arc<[u8]>,
    rate_limiter: RateLimiter,
}

impl AppState {
    pub fn new(
        pool: SqlitePool,
        token: String,
        requests_per_minute: u32,
    ) -> Result<Self, &'static str> {
        config::validate_token(&token).map_err(|_| "invalid StateSync token")?;
        Ok(Self {
            pool,
            token: Arc::from(token.into_bytes()),
            rate_limiter: RateLimiter::new(requests_per_minute)?,
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    fn token(&self) -> &[u8] {
        &self.token
    }

    fn rate_limiter(&self) -> &RateLimiter {
        &self.rate_limiter
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(api::health::handle))
        .route("/v1/sync", post(api::sync::handle))
        .route("/v1/snapshots/latest", get(api::snapshot::latest))
        .route("/v1/snapshots", put(api::snapshot::put))
        .route("/v1/devices/{device_id}", delete(api::devices::revoke))
        .layer(DefaultBodyLimit::max(MAXIMUM_BODY_BYTES))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetSensitiveRequestHeadersLayer::new(once(
            header::AUTHORIZATION,
        )))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn current_time_milliseconds() -> Result<i64, error::ApiError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(error::ApiError::internal)?;
    i64::try_from(duration.as_millis()).map_err(error::ApiError::internal)
}
