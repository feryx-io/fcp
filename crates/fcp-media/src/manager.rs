//! Media manager - high-level API for media operations

use crate::{
    backend::MediaBackend, config::BackendType, Error, MediaConfig, MediaSession,
    MediaSessionConfig, MediaSessionStats, Result,
};
use fcp_core::{CallId, DialogId};
use std::sync::Arc;
use tracing::{info, warn};

/// Media manager - unified interface for all media operations
///
/// Abstracts over backend implementation (embedded or remote).
/// Rest of FCP only interacts with MediaManager, never directly with backends.
pub struct MediaManager {
    /// Backend implementation (embedded or remote)
    backend: Arc<dyn MediaBackend>,

    /// Configuration
    config: MediaConfig,
}

impl MediaManager {
    /// Create a new media manager
    pub async fn new(config: MediaConfig) -> Result<Self> {
        info!(backend = ?config.backend, "Initializing media manager");

        // Create backend based on configuration
        let backend: Arc<dyn MediaBackend> = match &config.backend {
            #[cfg(feature = "embedded")]
            BackendType::Embedded => {
                let embedded = crate::backend::EmbeddedBackend::new(&config).await?;
                Arc::new(embedded)
            }

            #[cfg(not(feature = "embedded"))]
            BackendType::Embedded => {
                return Err(Error::Config(
                    "Embedded backend requested but 'embedded' feature not enabled".into(),
                ));
            }

            #[cfg(feature = "remote")]
            BackendType::Remote { .. } => {
                let remote = crate::backend::RemoteBackend::new(&config).await?;
                Arc::new(remote)
            }

            #[cfg(not(feature = "remote"))]
            BackendType::Remote { .. } => {
                return Err(Error::Config(
                    "Remote backend requested but 'remote' feature not enabled".into(),
                ));
            }
        };

        info!(
            backend_type = backend.backend_name(),
            "Media manager initialized"
        );

        Ok(Self { backend, config })
    }

    /// Create a media session for a call
    pub async fn create_session(
        &self,
        call_id: CallId,
        dialog_id: DialogId,
        config: MediaSessionConfig,
    ) -> Result<Arc<MediaSession>> {
        self.backend
            .create_session(call_id, dialog_id, config)
            .await
    }

    /// Create a session with default configuration
    pub async fn create_session_default(
        &self,
        call_id: CallId,
        dialog_id: DialogId,
    ) -> Result<Arc<MediaSession>> {
        self.create_session(call_id, dialog_id, MediaSessionConfig::default())
            .await
    }

    /// Get an existing media session
    pub async fn get_session(&self, call_id: &CallId) -> Result<Arc<MediaSession>> {
        self.backend.get_session(call_id).await
    }

    /// Update session with remote SDP (offer/answer exchange)
    ///
    /// Returns local SDP to send in response
    pub async fn update_session(
        &self,
        call_id: &CallId,
        remote_sdp: String,
    ) -> Result<String> {
        self.backend.update_session(call_id, remote_sdp).await
    }

    /// Put call on hold (send silence/comfort noise)
    pub async fn hold_session(&self, call_id: &CallId) -> Result<()> {
        self.backend.hold_session(call_id).await
    }

    /// Resume call from hold
    pub async fn resume_session(&self, call_id: &CallId) -> Result<()> {
        self.backend.resume_session(call_id).await
    }

    /// Terminate a media session
    pub async fn terminate_session(&self, call_id: &CallId) -> Result<()> {
        self.backend.terminate_session(call_id).await
    }

    /// Get session statistics (RTP packets, jitter, etc.)
    pub async fn get_session_stats(&self, call_id: &CallId) -> Result<MediaSessionStats> {
        self.backend.get_stats(call_id).await
    }

    /// Send DTMF digit on a call
    pub async fn send_dtmf(
        &self,
        call_id: &CallId,
        digit: char,
        duration_ms: u32,
    ) -> Result<()> {
        if !digit.is_ascii_digit() && digit != '*' && digit != '#' {
            return Err(Error::InvalidState(format!(
                "Invalid DTMF digit: {}",
                digit
            )));
        }

        self.backend.send_dtmf(call_id, digit, duration_ms).await
    }

    /// Start recording a call
    pub async fn start_recording(&self, call_id: &CallId, path: String) -> Result<()> {
        self.backend.start_recording(call_id, path).await
    }

    /// Stop recording a call
    pub async fn stop_recording(&self, call_id: &CallId) -> Result<()> {
        self.backend.stop_recording(call_id).await
    }

    /// Get statistics for all sessions
    pub async fn get_stats(&self) -> MediaManagerStats {
        // This would need to be implemented in backend trait if needed
        // For now, return placeholder
        MediaManagerStats {
            total_sessions: 0,
            active_sessions: 0,
            backend_type: self.backend.backend_name().to_string(),
        }
    }

    /// Health check - verify backend is operational
    pub async fn health_check(&self) -> Result<bool> {
        self.backend.health_check().await
    }

    /// Get backend type name
    pub fn backend_name(&self) -> &str {
        self.backend.backend_name()
    }
}

/// Media manager statistics
#[derive(Debug, Clone)]
pub struct MediaManagerStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub backend_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[cfg(feature = "embedded")]
    async fn test_create_manager_embedded() {
        let config = MediaConfig {
            backend: BackendType::Embedded,
            ..Default::default()
        };

        let manager = MediaManager::new(config).await;
        assert!(manager.is_ok());

        let manager = manager.unwrap();
        assert_eq!(manager.backend_name(), "embedded");
    }

    #[test]
    fn test_invalid_dtmf() {
        let invalid_digits = vec!['a', 'z', '@', ' '];
        for digit in invalid_digits {
            assert!(!digit.is_ascii_digit() && digit != '*' && digit != '#');
        }
    }
}
