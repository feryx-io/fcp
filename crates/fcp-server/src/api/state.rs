//! Shared API state.

use fcp_core::FileBackedStorage;
use std::sync::Arc;

/// Shared state for API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Storage for users and registrations
    pub storage: Arc<FileBackedStorage>,
}

impl ApiState {
    pub fn new(storage: Arc<FileBackedStorage>) -> Self {
        Self { storage }
    }
}
