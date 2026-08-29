use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    Forbidden,
    DeviceLimit,
    RateLimited,
    InvalidRequest,
    PayloadTooLarge,
    SequenceConflict,
    CursorAhead,
    NotFound,
    Internal,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

impl ApiError {
    pub fn from_json_rejection(rejection: JsonRejection) -> Self {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            Self::PayloadTooLarge
        } else {
            Self::InvalidRequest
        }
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(error = %error, "StateSync request failed");
        Self::Internal
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication required",
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "device_revoked", "Device is revoked"),
            Self::DeviceLimit => (
                StatusCode::FORBIDDEN,
                "device_limit",
                "Active device limit reached",
            ),
            Self::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many requests",
            ),
            Self::InvalidRequest => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_request",
                "Invalid request",
            ),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "Request body is too large",
            ),
            Self::SequenceConflict => (
                StatusCode::CONFLICT,
                "sequence_conflict",
                "Local sequence was reused",
            ),
            Self::CursorAhead => (
                StatusCode::CONFLICT,
                "cursor_ahead",
                "Cursor is ahead of the server",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Resource not found"),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal server error",
            ),
        };
        let mut response = (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response();
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"StateSync Lite\""),
            );
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        }
        response
    }
}
