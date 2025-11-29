//! Core primitives shared across the FCP project.
//!
//! This crate holds common error types, identifiers, and configuration
//! structures that other crates depend on.

pub mod codec;
pub mod config;
pub mod error;
pub mod identifiers;
pub mod service;
pub mod storage;
pub mod time;
pub mod transport;

// Re-export commonly used types
pub use crate::codec::{AudioCodec, Codec, CodecPreferences, VideoCodec};
pub use crate::config::{
    ApiConfig, FcpConfig, LogFormat, NodeConfig, ObservabilityConfig, StorageBackend,
    StorageConfig, TransportConfig,
};
pub use crate::error::{
    DialogError, Error, MediaError, ParseError, Result, StorageError, TransactionError,
    TransportError,
};
pub use crate::identifiers::*;
pub use crate::service::{Service, ServiceManager};
pub use crate::storage::{
    CredentialStore, FileBackedStorage, Registration, RegistrationStore, User,
};
pub use crate::transport::{
    ConnectionId, ConnectionMetadata, ConnectionState, Transport, TransportMessage,
    TransportReceiver, TransportType,
};
