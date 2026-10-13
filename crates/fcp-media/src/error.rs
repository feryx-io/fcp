//! Error types for fcp-media

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Media backend error: {0}")]
    Backend(String),

    #[error("forge-media error: {0}")]
    ForgeMedia(String),

    #[error("SDP error: {0}")]
    Sdp(String),

    #[error("Codec negotiation failed: {0}")]
    CodecNegotiation(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// Convert from forge-media errors
impl From<forge_core::error::ForgeError> for Error {
    fn from(err: forge_core::error::ForgeError) -> Self {
        Error::ForgeMedia(err.to_string())
    }
}
