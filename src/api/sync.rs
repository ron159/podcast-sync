use crate::{
    AppState, MAXIMUM_BODY_BYTES, current_time_milliseconds,
    error::ApiError,
    model::{EntityType, HlcTimestamp, Mutation, SyncChange, SyncRequest, SyncResponse},
};
use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use serde_json::Value;
use sqlx::{FromRow, Row, Sqlite, Transaction, sqlite::SqliteRow};

const MAXIMUM_PULL_ROWS: i64 = 500;
const RESPONSE_BUDGET_BYTES: usize = MAXIMUM_BODY_BYTES - 4_096;
const MAXIMUM_ACTIVE_DEVICES: i64 = 16;

struct StoredOperation {
    server_seq: i64,
    device_id: String,
    hlc_wall_ms: i64,
    hlc_counter: i64,
    entity_type: String,
    entity_key: String,
    payload_json: String,
}

impl<'row> FromRow<'row, SqliteRow> for StoredOperation {
    fn from_row(row: &'row SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            server_seq: row.try_get("server_seq")?,
            device_id: row.try_get("device_id")?,
            hlc_wall_ms: row.try_get("hlc_wall_ms")?,
            hlc_counter: row.try_get("hlc_counter")?,
            entity_type: row.try_get("entity_type")?,
            entity_key: row.try_get("entity_key")?,
            payload_json: row.try_get("payload_json")?,
        })
    }
}

pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<SyncRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    state.authorize(&headers)?;
    let Json(mut request) = payload.map_err(ApiError::from_json_rejection)?;
    request
        .validate_and_normalize()
        .map_err(|_| ApiError::InvalidRequest)?;
    let response = synchronize(state, request).await?;
    Ok(Json(response).into_response())
}

async fn synchronize(state: AppState, request: SyncRequest) -> Result<SyncResponse, ApiError> {
    let mut transaction = state.pool().begin().await.map_err(ApiError::internal)?;
    reject_revoked_device(&mut transaction, &request.device_id).await?;
    let now = current_time_milliseconds()?;
    register_device(&mut transaction, &request.device_id, now).await?;
    let server_maximum = maximum_server_sequence(&mut transaction).await?;
    if request.cursor > server_maximum {
        return Err(ApiError::CursorAhead);
    }
    let mut accepted_local_sequence =
        maximum_local_sequence(&mut transaction, &request.device_id).await?;
    for mutation in &request.mutations {
        store_mutation(&mut transaction, &request.device_id, mutation, now).await?;
        accepted_local_sequence = accepted_local_sequence.max(mutation.local_seq);
    }
    let (next_cursor, changes) = pull_changes(&mut transaction, &request).await?;
    sqlx::query(
        "UPDATE devices SET last_seen_at = ?, last_cursor = MAX(last_cursor, ?) WHERE device_id = ?",
    )
    .bind(now)
    .bind(next_cursor)
    .bind(&request.device_id)
    .execute(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(SyncResponse {
        accepted_local_seq: accepted_local_sequence,
        next_cursor,
        changes,
    })
}

async fn reject_revoked_device(
    transaction: &mut Transaction<'_, Sqlite>,
    device_id: &str,
) -> Result<(), ApiError> {
    let revoked_at =
        sqlx::query_scalar::<_, Option<i64>>("SELECT revoked_at FROM devices WHERE device_id = ?")
            .bind(device_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(ApiError::internal)?;
    if matches!(revoked_at, Some(Some(_))) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

async fn register_device(
    transaction: &mut Transaction<'_, Sqlite>,
    device_id: &str,
    now: i64,
) -> Result<(), ApiError> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM devices WHERE device_id = ?)")
            .bind(device_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(ApiError::internal)?;
    if !exists {
        let active_devices =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM devices WHERE revoked_at IS NULL")
                .fetch_one(&mut **transaction)
                .await
                .map_err(ApiError::internal)?;
        if active_devices >= MAXIMUM_ACTIVE_DEVICES {
            return Err(ApiError::DeviceLimit);
        }
    }
    sqlx::query(
        "INSERT INTO devices(device_id, last_seen_at, last_cursor) VALUES (?, ?, 0) \
         ON CONFLICT(device_id) DO UPDATE SET last_seen_at = excluded.last_seen_at",
    )
    .bind(device_id)
    .bind(now)
    .execute(&mut **transaction)
    .await
    .map_err(ApiError::internal)?;
    Ok(())
}

async fn maximum_server_sequence(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<i64, ApiError> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(server_seq) FROM operations")
        .fetch_one(&mut **transaction)
        .await
        .map(|value| value.unwrap_or(0))
        .map_err(ApiError::internal)
}

async fn maximum_local_sequence(
    transaction: &mut Transaction<'_, Sqlite>,
    device_id: &str,
) -> Result<i64, ApiError> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(local_seq) FROM operations WHERE device_id = ?",
    )
    .bind(device_id)
    .fetch_one(&mut **transaction)
    .await
    .map(|value| value.unwrap_or(0))
    .map_err(ApiError::internal)
}

async fn store_mutation(
    transaction: &mut Transaction<'_, Sqlite>,
    device_id: &str,
    mutation: &Mutation,
    now: i64,
) -> Result<(), ApiError> {
    let payload_json = serde_json::to_string(&mutation.payload).map_err(ApiError::internal)?;
    let existing = sqlx::query_as::<_, StoredOperation>(
        "SELECT server_seq, device_id, hlc_wall_ms, hlc_counter, entity_type, \
         entity_key, payload_json FROM operations WHERE device_id = ? AND local_seq = ?",
    )
    .bind(device_id)
    .bind(mutation.local_seq)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(ApiError::internal)?;
    if let Some(existing) = existing {
        if existing.hlc_wall_ms != mutation.hlc.wall_ms
            || existing.hlc_counter != mutation.hlc.counter
            || existing.entity_type != mutation.entity_type.as_str()
            || existing.entity_key != mutation.entity_key
            || existing.payload_json != payload_json
        {
            return Err(ApiError::SequenceConflict);
        }
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO operations(device_id, local_seq, hlc_wall_ms, hlc_counter, entity_type, \
         entity_key, payload_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(device_id)
    .bind(mutation.local_seq)
    .bind(mutation.hlc.wall_ms)
    .bind(mutation.hlc.counter)
    .bind(mutation.entity_type.as_str())
    .bind(&mutation.entity_key)
    .bind(payload_json)
    .bind(now)
    .execute(&mut **transaction)
    .await
    .map_err(ApiError::internal)?;
    Ok(())
}

async fn pull_changes(
    transaction: &mut Transaction<'_, Sqlite>,
    request: &SyncRequest,
) -> Result<(i64, Vec<SyncChange>), ApiError> {
    let rows = sqlx::query_as::<_, StoredOperation>(
        "SELECT server_seq, device_id, hlc_wall_ms, hlc_counter, entity_type, \
         entity_key, payload_json FROM operations WHERE server_seq > ? ORDER BY server_seq LIMIT ?",
    )
    .bind(request.cursor)
    .bind(MAXIMUM_PULL_ROWS)
    .fetch_all(&mut **transaction)
    .await
    .map_err(ApiError::internal)?;
    let mut next_cursor = request.cursor;
    let mut changes = Vec::new();
    let mut response_bytes = 128;
    for row in rows {
        if row.device_id == request.device_id {
            next_cursor = row.server_seq;
            continue;
        }
        let change = row.into_change()?;
        let change_bytes = serde_json::to_vec(&change)
            .map_err(ApiError::internal)?
            .len()
            + 1;
        if response_bytes + change_bytes > RESPONSE_BUDGET_BYTES {
            break;
        }
        response_bytes += change_bytes;
        next_cursor = change.server_seq;
        changes.push(change);
    }
    Ok((next_cursor, changes))
}

impl StoredOperation {
    fn into_change(self) -> Result<SyncChange, ApiError> {
        let entity_type =
            EntityType::from_database(&self.entity_type).map_err(ApiError::internal)?;
        let payload =
            serde_json::from_str::<Value>(&self.payload_json).map_err(ApiError::internal)?;
        Ok(SyncChange {
            server_seq: self.server_seq,
            device_id: self.device_id,
            hlc: HlcTimestamp {
                wall_ms: self.hlc_wall_ms,
                counter: self.hlc_counter,
            },
            entity_type,
            entity_key: self.entity_key,
            payload,
        })
    }
}
