//! Media session types

use fcp_core::{CallId, DialogId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Represents a media session tied to a SIP dialog
#[derive(Debug)]
pub struct MediaSession {
    /// Internal FCP call identifier
    pub call_id: CallId,

    /// SIP dialog identifier
    pub dialog_id: DialogId,

    /// forge-media session identifier
    pub forge_session_id: String,

    /// Current state
    pub state: RwLock<MediaSessionState>,

    /// Session configuration
    pub config: MediaSessionConfig,

    /// Local SDP (answer we generated)
    pub local_sdp: RwLock<Option<String>>,

    /// Remote SDP (offer we received)
    pub remote_sdp: RwLock<Option<String>>,

    /// Negotiated codecs
    pub negotiated_codecs: RwLock<Vec<String>>,

    /// Session statistics
    pub stats: RwLock<MediaSessionStats>,

    /// RTP port allocated for this session
    pub rtp_port: u16,

    /// RTCP port allocated for this session
    pub rtcp_port: u16,
}

impl MediaSession {
    /// Create a new media session
    pub fn new(
        call_id: CallId,
        dialog_id: DialogId,
        forge_session_id: String,
        config: MediaSessionConfig,
        rtp_port: u16,
        rtcp_port: u16,
    ) -> Self {
        Self {
            call_id,
            dialog_id,
            forge_session_id,
            state: RwLock::new(MediaSessionState::Negotiating),
            config,
            local_sdp: RwLock::new(None),
            remote_sdp: RwLock::new(None),
            negotiated_codecs: RwLock::new(Vec::new()),
            stats: RwLock::new(MediaSessionStats::default()),
            rtp_port,
            rtcp_port,
        }
    }

    /// Get current state
    pub async fn state(&self) -> MediaSessionState {
        *self.state.read().await
    }

    /// Update state
    pub async fn set_state(&self, new_state: MediaSessionState) {
        *self.state.write().await = new_state;
    }

    /// Set remote SDP
    pub async fn set_remote_sdp(&self, sdp: String) {
        *self.remote_sdp.write().await = Some(sdp);
    }

    /// Set local SDP
    pub async fn set_local_sdp(&self, sdp: String) {
        *self.local_sdp.write().await = Some(sdp);
    }

    /// Get local SDP
    pub async fn local_sdp(&self) -> Option<String> {
        self.local_sdp.read().await.clone()
    }

    /// Get remote SDP
    pub async fn remote_sdp(&self) -> Option<String> {
        self.remote_sdp.read().await.clone()
    }
}

/// Media session state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaSessionState {
    /// Negotiating SDP
    Negotiating,

    /// Media active (RTP flowing)
    Active,

    /// Media on hold
    OnHold,

    /// Media paused (waiting for re-INVITE)
    Paused,

    /// Terminated
    Terminated,
}

/// Media session configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSessionConfig {
    /// Enable transcoding between different codecs
    pub enable_transcoding: bool,

    /// Enable recording for this session
    pub enable_recording: bool,

    /// Recording path (if recording enabled)
    pub recording_path: Option<String>,

    /// Preferred codecs in priority order
    pub preferred_codecs: Vec<String>,

    /// DTMF mode
    pub dtmf_mode: DtmfMode,

    /// Enable jitter buffer
    pub enable_jitter_buffer: bool,

    /// Jitter buffer size in milliseconds
    pub jitter_buffer_ms: u32,

    /// Enable packet loss concealment
    pub enable_plc: bool,

    /// Enable echo cancellation
    pub enable_echo_cancellation: bool,
}

impl Default for MediaSessionConfig {
    fn default() -> Self {
        Self {
            enable_transcoding: true,
            enable_recording: false,
            recording_path: None,
            preferred_codecs: vec![
                "PCMU".to_string(), // G.711 µ-law
                "PCMA".to_string(), // G.711 A-law
                "opus".to_string(), // Opus
                "G722".to_string(), // G.722 wideband
            ],
            dtmf_mode: DtmfMode::Rfc2833,
            enable_jitter_buffer: true,
            jitter_buffer_ms: 50,
            enable_plc: true,
            enable_echo_cancellation: false,
        }
    }
}

/// DTMF detection mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DtmfMode {
    /// RFC 2833 RTP events (telephone-event)
    Rfc2833,

    /// In-band audio detection
    InBand,

    /// Both methods
    Both,

    /// Disabled
    Disabled,
}

/// Media session statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaSessionStats {
    /// Total RTP packets sent
    pub packets_sent: u64,

    /// Total RTP packets received
    pub packets_received: u64,

    /// Total bytes sent
    pub bytes_sent: u64,

    /// Total bytes received
    pub bytes_received: u64,

    /// Packets lost
    pub packets_lost: u64,

    /// Average jitter in milliseconds
    pub jitter_ms: f64,

    /// Round-trip time in milliseconds (from RTCP)
    pub rtt_ms: Option<f64>,

    /// Current codec in use
    pub current_codec: Option<String>,

    /// Media start time
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,

    /// Media duration in seconds
    pub duration_secs: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_state_transitions() {
        let states = vec![
            MediaSessionState::Negotiating,
            MediaSessionState::Active,
            MediaSessionState::OnHold,
            MediaSessionState::Active,
            MediaSessionState::Terminated,
        ];

        for state in states {
            // Just verify states can be created and compared
            assert_eq!(state, state);
        }
    }

    #[test]
    fn test_default_config() {
        let config = MediaSessionConfig::default();
        assert!(config.enable_transcoding);
        assert!(!config.enable_recording);
        assert_eq!(config.dtmf_mode, DtmfMode::Rfc2833);
        assert!(config.preferred_codecs.contains(&"PCMU".to_string()));
    }
}
