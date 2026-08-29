use crate::{AppState, current_time_milliseconds, error::ApiError, model::validate_device_id};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};

pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.authorize(&headers)?;
    validate_device_id(&device_id).map_err(|_| ApiError::InvalidRequest)?;
    let now = current_time_milliseconds()?;
    sqlx::query(
        "INSERT INTO devices(device_id, last_seen_at, last_cursor, revoked_at) VALUES (?, ?, 0, ?) \
         ON CONFLICT(device_id) DO UPDATE SET revoked_at = excluded.revoked_at",
    )
    .bind(device_id)
    .bind(now)
    .bind(now)
    .execute(state.pool())
    .await
    .map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}
