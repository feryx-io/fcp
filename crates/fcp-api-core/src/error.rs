//! API error handling.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

/// API error response: the one shape every error under `/api/v1` has
/// (docs/DEVELOPER_SURFACE.md §3, phase 2), whether a handler, the auth
/// layer or an extractor refused the request.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorResponse {
    /// The status's reason phrase, e.g. `Not Found`.
    pub error: String,
    /// A stable machine-readable code: the status as snake case
    /// (`not_found`, `bad_request`, `rate_limited`, …).
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl ErrorResponse {
    /// The error body for a status, with the given detail.
    pub fn for_status(status: StatusCode, details: Option<String>) -> Self {
        Self {
            error: status.canonical_reason().unwrap_or("Error").to_string(),
            code: code_for(status).to_string(),
            details,
        }
    }
}

impl ErrorResponse {
    /// The API's error for a failure a service this API proxies to
    /// answered: its status, and what it said as the detail.
    pub fn from_upstream(status: StatusCode, body: &serde_json::Value) -> Self {
        let detail = body["details"]
            .as_str()
            .or_else(|| body["error"].as_str())
            .or_else(|| body["message"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| body.to_string());
        Self::for_status(status, Some(detail))
    }
}

/// The machine code for a status.
pub fn code_for(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::PAYMENT_REQUIRED => "payment_required",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::METHOD_NOT_ALLOWED => "method_not_allowed",
        StatusCode::CONFLICT => "conflict",
        StatusCode::GONE => "gone",
        StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "unsupported_media_type",
        StatusCode::UNPROCESSABLE_ENTITY => "unprocessable_entity",
        StatusCode::TOO_MANY_REQUESTS => "rate_limited",
        StatusCode::SERVICE_UNAVAILABLE => "service_unavailable",
        StatusCode::BAD_GATEWAY => "bad_gateway",
        StatusCode::GATEWAY_TIMEOUT => "gateway_timeout",
        s if s.is_client_error() => "client_error",
        _ => "internal",
    }
}

/// API error type.
#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    Internal(String),
    Conflict(String),
    /// A backing service this endpoint proxies to is not configured or
    /// not reachable.
    ServiceUnavailable(String),
    /// What was asked for existed and is gone for good.
    Gone(String),
    /// The body is over what the route takes.
    PayloadTooLarge(String),
}

impl ApiError {
    /// What the error says.
    pub fn message(&self) -> &str {
        match self {
            ApiError::NotFound(msg)
            | ApiError::BadRequest(msg)
            | ApiError::Unauthorized(msg)
            | ApiError::Forbidden(msg)
            | ApiError::Internal(msg)
            | ApiError::Conflict(msg)
            | ApiError::Gone(msg)
            | ApiError::PayloadTooLarge(msg)
            | ApiError::ServiceUnavailable(msg) => msg,
        }
    }

    /// The status this error answers with.
    pub fn status(&self) -> StatusCode {
        match self {
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden(_) => StatusCode::FORBIDDEN,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::Gone(_) => StatusCode::GONE,
            ApiError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            ApiError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let details = match self {
            ApiError::NotFound(msg)
            | ApiError::BadRequest(msg)
            | ApiError::Unauthorized(msg)
            | ApiError::Forbidden(msg)
            | ApiError::Internal(msg)
            | ApiError::Conflict(msg)
            | ApiError::Gone(msg)
            | ApiError::PayloadTooLarge(msg)
            | ApiError::ServiceUnavailable(msg) => msg,
        };
        (
            status,
            Json(ErrorResponse::for_status(status, Some(details))),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::storage(err)
    }
}

impl ApiError {
    /// A storage error: a delete a legal hold refused, or a write a unique
    /// index refused (`StorageError::Conflict`, e.g. two creates racing for
    /// one name), is a conflict the caller can act on; anything else is
    /// internal.
    pub fn storage(err: anyhow::Error) -> Self {
        if let Some(v) = err.downcast_ref::<fcp_core::legal_hold::LegalHoldViolation>() {
            return ApiError::Conflict(format!(
                "held by legal hold {} (docs/COMPLIANCE.md: legal holds)",
                v.hold_id
            ));
        }
        if let Some(fcp_core::Error::Storage(fcp_core::StorageError::Conflict(msg))) =
            err.downcast_ref::<fcp_core::Error>()
        {
            return ApiError::Conflict(msg.clone());
        }
        ApiError::Internal(err.to_string())
    }
}

impl From<fcp_core::Error> for ApiError {
    fn from(err: fcp_core::Error) -> Self {
        ApiError::Internal(err.to_string())
    }
}
