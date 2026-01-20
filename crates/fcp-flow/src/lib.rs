//! FCP Flow Engine - JSON-based call flow execution
//!
//! This crate provides a programmable voice platform with JSON-based call flows,
//! similar to Twilio Studio but self-hosted.

pub mod schema;
pub mod engine;
pub mod storage;
pub mod executor;

// Re-exports
pub use schema::{FlowDefinition, FlowNode, NodeType};
pub use engine::{FlowEngine, FlowContext, FlowExecutionResult, AnswerCallback, HangupCallback};
pub use storage::{FlowStore, FlowQuery, InMemoryFlowStore};

use thiserror::Error;

/// Result type for flow operations
pub type Result<T> = std::result::Result<T, FlowError>;

/// Errors that can occur during flow operations
#[derive(Debug, Error)]
pub enum FlowError {
    #[error("Flow not found: {0}")]
    NotFound(String),

    #[error("Invalid flow definition: {0}")]
    InvalidDefinition(String),

    #[error("Node not found: {0}")]
    NodeNotFound(String),

    #[error("Execution error: {0}")]
    ExecutionError(String),

    #[error("Variable not found: {0}")]
    VariableNotFound(String),

    #[error("HTTP request failed: {0}")]
    HttpError(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Other error: {0}")]
    Other(String),
}

impl From<anyhow::Error> for FlowError {
    fn from(err: anyhow::Error) -> Self {
        FlowError::Other(err.to_string())
    }
}
