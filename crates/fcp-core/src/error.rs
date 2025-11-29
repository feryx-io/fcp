use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("SIP parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("SIP protocol error: {0}")]
    Protocol(String),
    #[error("Transaction error: {0}")]
    Transaction(#[from] TransactionError),
    #[error("Dialog error: {0}")]
    Dialog(#[from] DialogError),
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("Media error: {0}")]
    Media(#[from] MediaError),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Invalid SIP message")]
    InvalidMessage,
    #[error("Invalid URI: {0}")]
    InvalidUri(String),
    #[error("Invalid header: {0}")]
    InvalidHeader(String),
    #[error("Missing required header: {0}")]
    MissingHeader(&'static str),
    #[error("Invalid method: {0}")]
    InvalidMethod(String),
    #[error("Invalid status code: {0}")]
    InvalidStatusCode(u16),
    #[error("Incomplete message")]
    Incomplete,
    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

#[derive(Debug, Error)]
pub enum TransactionError {
    #[error("Transaction timeout")]
    Timeout,
    #[error("Transaction already exists")]
    AlreadyExists,
    #[error("Transaction not found")]
    NotFound,
    #[error("Invalid transaction state: {0}")]
    InvalidState(String),
}

#[derive(Debug, Error)]
pub enum DialogError {
    #[error("Dialog not found")]
    NotFound,
    #[error("Dialog already exists")]
    AlreadyExists,
    #[error("Invalid dialog state")]
    InvalidState,
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("DNS resolution failed: {0}")]
    DnsResolution(String),
    #[error("Message too large: {size} bytes (max {max})")]
    MessageTooLarge { size: usize, max: usize },
    #[error("Connection closed unexpectedly")]
    ConnectionClosed,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Migration failed: {0}")]
    Migration(String),
    #[error("Record not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("RTP error: {0}")]
    Rtp(String),
    #[error("RTCP error: {0}")]
    Rtcp(String),
    #[error("Codec not supported: {0}")]
    UnsupportedCodec(String),
    #[error("SDP error: {0}")]
    Sdp(String),
}
