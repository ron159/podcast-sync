use crate::{
    AppState,
    error::ApiError,
    model::{Snapshot, SnapshotResponse},
};
use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use sqlx::{FromRow, Row, sqlite::SqliteRow};

struct StoredSnapshot {
    snapshot_id: i64,
    payload_json: String,
}

impl<'row> FromRow<'row, SqliteRow> for StoredSnapshot {
    fn from_row(row: &'row SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            snapshot_id: row.try_get("snapshot_id")?,
            payload_json: row.try_get("payload_json")?,
        })
    }
}

pub async fn latest(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SnapshotResponse>, ApiError> {
    state.authorize(&headers)?;
    let row = sqlx::query_as::<_, StoredSnapshot>(
        "SELECT snapshot_id, payload_json FROM snapshots ORDER BY snapshot_id DESC LIMIT 1",
    )
    .fetch_optional(state.pool())
    .await
    .map_err(ApiError::internal)?
    .ok_or(ApiError::NotFound)?;
    let snapshot = serde_json::from_str(&row.payload_json).map_err(ApiError::internal)?;
    Ok(Json(SnapshotResponse {
        snapshot_id: row.snapshot_id,
        snapshot,
    }))
}

pub async fn put(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<Snapshot>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    state.authorize(&headers)?;
    let Json(snapshot) = payload.map_err(ApiError::from_json_rejection)?;
    snapshot.validate().map_err(|_| ApiError::InvalidRequest)?;
    let mut transaction = state.pool().begin().await.map_err(ApiError::internal)?;
    let maximum_server_sequence =
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(server_seq) FROM operations")
            .fetch_one(&mut *transaction)
            .await
            .map_err(ApiError::internal)?
            .unwrap_or(0);
    if snapshot.base_server_seq > maximum_server_sequence {
        return Err(ApiError::CursorAhead);
    }
    let payload_json = serde_json::to_string(&snapshot).map_err(ApiError::internal)?;
    let result = sqlx::query(
        "INSERT INTO snapshots(base_server_seq, payload_json, created_at) VALUES (?, ?, ?)",
    )
    .bind(snapshot.base_server_seq)
    .bind(payload_json)
    .bind(snapshot.created_at)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    sqlx::query(
        "DELETE FROM snapshots WHERE snapshot_id NOT IN (\
         SELECT snapshot_id FROM snapshots ORDER BY snapshot_id DESC LIMIT 14)",
    )
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "snapshot_id": result.last_insert_rowid(),
            "base_server_seq": snapshot.base_server_seq
        })),
    ))
}
