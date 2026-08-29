use crate::AppState;
use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};

pub async fn handle(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(state.pool())
        .await
    {
        Ok(1) => (StatusCode::OK, Json(json!({"status": "ok"}))),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "unavailable"})),
        ),
    }
}
