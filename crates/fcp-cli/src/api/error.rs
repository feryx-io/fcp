//! API error types

use reqwest::StatusCode;
use thiserror::Error;

/// API client errors
#[derive(Debug, Error)]
pub enum ApiError {
    /// Network error (connection, timeout, etc.)
    #[error("Network error: {0}")]
    Network(String),

    /// HTTP error (non-success status code)
    #[error("HTTP {0}: {1}")]
    Http(StatusCode, String),

    /// Deserialization error
    #[error("Failed to deserialize response: {0}")]
    Deserialization(String),
}
