//! Media session types

use fcp_core::{CallId, DialogId};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Represents a media session tied to a SIP dialog
#[derive(Debug)]
pub struct MediaSession {
    /// Internal FCP call identifier
    call_id: CallId,

    /// SIP dialog identifier
    dialog_id: DialogId,

    /// forge-media session identifier
    forge_session_id: String,

    /// Current state
    state: RwLock<MediaSessionState>,

    /// Session configuration
    config: MediaSessionConfig,

    /// Local SDP (answer we generated)
    local_sdp: RwLock<Option<String>>,

    /// Remote SDP (offer we received)
    remote_sdp: RwLock<Option<String>>,

    /// Negotiated codecs
    negotiated_codecs: RwLock<Vec<String>>,

    /// Session statistics
    stats: RwLock<MediaSessionStats>,

    /// The session's socket pair per leg, live: a swap (a phone moved
    /// between legs) is seen at once.
    legs: forge_engine::LegSockets,

    /// The call's relayed video sections, the camera's and a shared
    /// screen's, opened when an SDP first carries them
    /// (`docs/SOFTPHONE_VIDEO.md` §2.2).
    video: parking_lot::Mutex<Vec<std::sync::Arc<crate::video_relay::VideoRelay>>>,
}

impl MediaSession {
    /// Create a new media session
    pub fn new(
        call_id: CallId,
        dialog_id: DialogId,
        forge_session_id: String,
        config: MediaSessionConfig,
        legs: forge_engine::LegSockets,
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
            legs,
            video: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Call identifier.
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// Dialog identifier.
    pub fn dialog_id(&self) -> &DialogId {
        &self.dialog_id
    }

    /// forge-media session identifier.
    pub fn forge_session_id(&self) -> &str {
        &self.forge_session_id
    }

    /// Session configuration.
    pub fn config(&self) -> &MediaSessionConfig {
        &self.config
    }

    /// The RTP/RTCP ports `leg`'s peer is told: each leg has its own, so
    /// an SDP names the leg it goes to.
    pub fn ports_for(&self, leg: crate::backend::Leg) -> (u16, u16) {
        let label = match leg {
            crate::backend::Leg::A => forge_engine::ParticipantLabel::A,
            crate::backend::Leg::B => forge_engine::ParticipantLabel::B,
        };
        let ports = self.legs.ports_for(label);
        (ports.rtp_port, ports.rtcp_port)
    }

    /// The relay for one video stream, when the call has opened one.
    pub fn video_relay(
        &self,
        stream: crate::sdp::VideoStream,
    ) -> Option<std::sync::Arc<crate::video_relay::VideoRelay>> {
        self.video
            .lock()
            .iter()
            .find(|r| r.stream() == stream)
            .cloned()
    }

    /// Every video relay the call has opened.
    pub fn video_relays(&self) -> Vec<std::sync::Arc<crate::video_relay::VideoRelay>> {
        self.video.lock().clone()
    }

    /// Keep a relay the call has opened. One per stream: a second for the
    /// same stream is closed by the caller instead.
    pub(crate) fn add_video_relay(
        &self,
        relay: std::sync::Arc<crate::video_relay::VideoRelay>,
    ) -> std::sync::Arc<crate::video_relay::VideoRelay> {
        let mut video = self.video.lock();
        if let Some(existing) = video.iter().find(|r| r.stream() == relay.stream()) {
            return existing.clone();
        }
        video.push(relay.clone());
        relay
    }

    /// The ports an SDP to `leg` names: audio's, and each relayed video
    /// stream's.
    pub fn anchor_ports(&self, leg: crate::backend::Leg) -> crate::sdp::AnchorPorts {
        let port = |stream| self.video_relay(stream).map(|r| r.port(leg));
        crate::sdp::AnchorPorts {
            audio: self.ports_for(leg).0,
            video: port(crate::sdp::VideoStream::Camera),
            content: port(crate::sdp::VideoStream::Content),
        }
    }

    /// Stop every video relay and give its ports back: the call is over.
    pub async fn close_video(&self) {
        let relays = std::mem::take(&mut *self.video.lock());
        for relay in relays {
            relay.close().await;
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

    /// Snapshot of negotiated codecs.
    pub async fn negotiated_codecs(&self) -> Vec<String> {
        self.negotiated_codecs.read().await.clone()
    }

    /// Snapshot of stats.
    pub async fn stats(&self) -> MediaSessionStats {
        self.stats.read().await.clone()
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

    /// Whether to relay RFC 2833 telephone-event packets between legs
    pub enable_relay_rfc2833: bool,
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
            enable_relay_rfc2833: true,
        }
    }
}

impl MediaSessionConfig {
    /// Validates configuration for security and correctness.
    ///
    /// # Security Checks
    /// - **Recording paths**: Prevents CRLF injection and rejects empty paths
    ///   - Note: Full path traversal validation (checking for "..", absolute paths, etc.)
    ///     is performed by `MediaManager::start_recording()` where the base directory is known
    /// - **Codec list**: Limits to 32 entries to prevent DoS via excessive codec lists
    /// - **Jitter buffer**: Bounds check (1-5000ms) prevents resource exhaustion
    ///
    /// # Errors
    /// Returns `Error::Config` if any validation check fails.
    pub fn validate(&self) -> crate::Result<()> {
        if self.enable_recording {
            if let Some(path) = &self.recording_path {
                if path.is_empty() {
                    return Err(crate::Error::Config(
                        "recording_path must not be empty when recording is enabled".into(),
                    ));
                }
                if contains_crlf(path) {
                    return Err(crate::Error::Config(
                        "recording_path must not contain CR or LF characters".into(),
                    ));
                }
                // Path traversal checks (absolute paths, "..", etc.) are performed
                // by MediaManager::start_recording() where base_dir is available
                validate_path_components(path)?;
            }
        }

        if self.preferred_codecs.len() > 32 {
            return Err(crate::Error::Config(
                "preferred_codecs list is too long (max 32 entries)".into(),
            ));
        }

        if self.jitter_buffer_ms == 0 || self.jitter_buffer_ms > 5000 {
            return Err(crate::Error::Config(
                "jitter_buffer_ms must be between 1 and 5000".into(),
            ));
        }

        Ok(())
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

fn contains_crlf(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, b'\r' | b'\n'))
}

/// Validates path components for basic path traversal patterns.
///
/// This performs early validation that can be done without knowing the base directory.
/// Full path traversal validation is performed by `MediaManager::start_recording()`.
fn validate_path_components(path: &str) -> crate::Result<()> {
    use std::path::{Component, Path};

    let path_obj = Path::new(path);

    // Reject absolute paths
    if path_obj.is_absolute() {
        return Err(crate::Error::Config(
            "Recording path must be relative, not absolute".into(),
        ));
    }

    // Check for suspicious components
    for component in path_obj.components() {
        match component {
            Component::ParentDir => {
                return Err(crate::Error::Config(
                    "Recording path must not contain '..' (path traversal attempt)".into(),
                ));
            }
            Component::RootDir => {
                return Err(crate::Error::Config(
                    "Recording path must not contain root directory".into(),
                ));
            }
            Component::Prefix(_) => {
                return Err(crate::Error::Config(
                    "Recording path must not contain Windows drive prefix".into(),
                ));
            }
            Component::Normal(_) | Component::CurDir => {
                // These are safe
            }
        }
    }

    Ok(())
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
        assert!(config.validate().is_ok());
    }
}
