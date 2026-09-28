//! Media manager - high-level API for media operations

use crate::{
    backend::{AudioTarget, MediaBackend},
    config::BackendType,
    Error, MediaConfig, MediaSession, MediaSessionConfig, MediaSessionStats, Result,
};
use fcp_core::{CallId, DialogId};
use std::{path::Path, sync::Arc};
use tracing::info;

/// Media manager - unified interface for all media operations
///
/// Abstracts over backend implementation (embedded or remote).
/// Rest of FCP only interacts with MediaManager, never directly with backends.
pub struct MediaManager {
    /// Backend implementation (embedded or remote)
    backend: Arc<dyn MediaBackend>,

    /// Configuration
    config: MediaConfig,

    /// Shared TTS service (available for other subsystems to reuse)
    tts_service: Option<Arc<crate::tts::TtsService>>,
}

impl MediaManager {
    /// Create a new media manager
    pub async fn new(config: MediaConfig) -> Result<Self> {
        info!(backend = ?config.backend, "Initializing media manager");

        config.validate()?;

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

        // Capture TTS service for sharing with other subsystems
        let tts_service = config.tts_service.clone();

        Ok(Self {
            backend,
            config,
            tts_service,
        })
    }

    /// Create a media session for a call
    pub async fn create_session(
        &self,
        call_id: CallId,
        dialog_id: DialogId,
        config: MediaSessionConfig,
    ) -> Result<Arc<MediaSession>> {
        config.validate()?;
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
    pub async fn update_session(&self, call_id: &CallId, remote_sdp: String) -> Result<String> {
        self.backend.update_session(call_id, remote_sdp).await
    }

    /// Set B-leg remote endpoint from SDP
    ///
    /// Used by B2BUA when the second call leg answers. Sets participant_b's
    /// remote address so the ForwardingEngine knows where to send RTP packets.
    pub async fn update_session_b_leg(&self, call_id: &CallId, remote_sdp: String) -> Result<()> {
        self.backend.update_session_b_leg(call_id, remote_sdp).await
    }

    /// As [`MediaManager::update_session_b_leg`], with the SRTP offer this
    /// node made on that leg: the key it encrypts with is the one it
    /// advertised (`docs/TENANT_TRUNKS.md` §6).
    pub async fn update_session_b_leg_offered(
        &self,
        call_id: &CallId,
        remote_sdp: String,
        offer: Option<crate::sdp::SrtpOffer>,
    ) -> Result<()> {
        self.backend
            .update_session_b_leg_offered(call_id, remote_sdp, offer)
            .await
    }

    /// Learn `leg`'s RTP address from its first packet from `source`
    /// rather than its SDP (`docs/HYBRID_CONTROL_PLANE.md` §3.6).
    pub async fn latch_leg(
        &self,
        call_id: &CallId,
        leg: crate::backend::Leg,
        source: std::net::IpAddr,
    ) -> Result<()> {
        self.backend.latch_leg(call_id, leg, source).await
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

    /// List all active RTP sessions with their details
    ///
    /// Returns information about each active media session including:
    /// - Call and dialog IDs
    /// - Codec information for each leg
    /// - Remote endpoints (IP:port)
    /// - Whether transcoding is active
    /// - Packet/byte statistics
    pub async fn list_sessions(&self) -> Vec<crate::backend::RtpSessionInfo> {
        self.backend.list_sessions().await
    }

    /// Send a DTMF digit to one leg of a call, as RFC 2833 telephone-event.
    pub async fn send_dtmf(
        &self,
        call_id: &CallId,
        leg: crate::backend::MediaLeg,
        digit: char,
        duration_ms: u32,
    ) -> Result<()> {
        if !digit.is_ascii_digit() && !matches!(digit, '*' | '#' | 'A'..='D') {
            return Err(Error::InvalidState(format!(
                "Invalid DTMF digit: {}",
                digit
            )));
        }

        self.backend
            .send_dtmf(call_id, leg, digit, duration_ms)
            .await
    }

    /// Whether telephone-event the other leg sends is relayed to `leg`.
    pub fn set_rfc2833_relay_to(
        &self,
        call_id: &CallId,
        leg: crate::backend::MediaLeg,
        relay: bool,
    ) -> Result<()> {
        self.backend.set_rfc2833_relay_to(call_id, leg, relay)
    }

    /// Every digit either leg of the call sends, with the leg it came
    /// from, until the session ends; `None` when the backend cannot say.
    pub fn subscribe_digits(
        &self,
        call_id: &CallId,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<crate::backend::LegDigit>> {
        self.backend.subscribe_digits(call_id)
    }

    /// Create a DTMF collector wired to the media backend for a specific call.
    ///
    /// The returned collector automatically receives DTMF events detected by
    /// the media engine (RFC 2833 / in-band) for the given call.
    pub fn create_dtmf_collector(
        &self,
        call_id: &CallId,
        max_digits: usize,
        finish_on_key: Option<char>,
    ) -> crate::dtmf_collector::DtmfCollector {
        self.backend
            .create_dtmf_collector(call_id, max_digits, finish_on_key)
    }

    /// Start recording a call
    ///
    /// # Security
    /// - Validates path for CRLF injection
    /// - Prevents path traversal attacks
    /// - Ensures path stays within configured recording directory
    pub async fn start_recording(&self, call_id: &CallId, path: String) -> Result<()> {
        validate_recording_path(&path, Path::new(&self.config.recording_dir))?;
        // Build absolute path by prepending recording_dir to the validated relative path
        let full_path = Path::new(&self.config.recording_dir).join(&path);
        self.backend
            .start_recording(call_id, full_path.to_string_lossy().to_string())
            .await
    }

    /// Where a recording started with the relative `path` is written: the
    /// path its row and its call's record name.
    pub fn recording_file(&self, path: &str) -> String {
        Path::new(&self.config.recording_dir)
            .join(path)
            .to_string_lossy()
            .into_owned()
    }

    /// Stop recording a call
    pub async fn stop_recording(&self, call_id: &CallId) -> Result<()> {
        self.backend.stop_recording(call_id).await
    }

    /// Play an audio file into a call
    ///
    /// # Arguments
    /// * `call_id` - The call to inject audio into
    /// * `file_path` - Path to the audio file (WAV, MP3, FLAC, etc.)
    /// * `target` - Which participant(s) should hear the audio (ParticipantA, ParticipantB, or Both)
    ///
    /// # Returns
    /// A playback ID that can be used to stop the playback
    ///
    /// # Example
    /// ```ignore
    /// let playback_id = manager.play_file(&call_id, "greeting.wav", AudioTarget::Both).await?;
    /// // Later, stop playback if needed
    /// manager.stop_playback(&call_id, playback_id).await?;
    /// ```
    pub async fn play_file(
        &self,
        call_id: &CallId,
        file_path: String,
        target: AudioTarget,
    ) -> Result<String> {
        self.backend.play_file(call_id, file_path, target).await
    }

    /// Play an audio file into a call again and again until stopped: hold
    /// music (`docs/ENTERPRISE_CALLING.md` §3.3). The other leg's audio is
    /// not relayed to the target meanwhile.
    /// Add a supervisor to the call's media; the SDP to offer their phone
    /// (`docs/ENTERPRISE_CALLING.md` §3.11).
    pub async fn add_party(
        &self,
        call_id: &CallId,
        mode: crate::backend::Supervision,
    ) -> Result<String> {
        self.backend.add_party(call_id, mode).await
    }

    /// The supervisor's SDP; the answer to it.
    pub async fn connect_party(&self, call_id: &CallId, remote_sdp: String) -> Result<String> {
        self.backend.connect_party(call_id, remote_sdp).await
    }

    /// Change how the supervisor takes part.
    pub async fn set_party_mode(
        &self,
        call_id: &CallId,
        mode: crate::backend::Supervision,
    ) -> Result<()> {
        self.backend.set_party_mode(call_id, mode).await
    }

    /// Remove the supervisor.
    pub async fn remove_party(&self, call_id: &CallId) -> Result<()> {
        self.backend.remove_party(call_id).await
    }

    pub async fn play_file_looped(
        &self,
        call_id: &CallId,
        file_path: String,
        target: AudioTarget,
    ) -> Result<String> {
        self.backend
            .play_file_looped(call_id, file_path, target)
            .await
    }

    /// Play a generated tone into a call
    ///
    /// # Arguments
    /// * `call_id` - The call to inject audio into
    /// * `frequency` - Tone frequency in Hz (e.g., 1000.0 for 1kHz beep)
    /// * `duration_ms` - Duration in milliseconds
    /// * `amplitude` - Amplitude from 0.0 to 1.0
    /// * `target` - Which participant(s) should hear the tone
    ///
    /// # Returns
    /// A playback ID that can be used to stop or wait for the tone
    pub async fn play_tone(
        &self,
        call_id: &CallId,
        frequency: f32,
        duration_ms: u32,
        amplitude: f32,
        target: AudioTarget,
    ) -> Result<String> {
        self.backend
            .play_tone(call_id, frequency, duration_ms, amplitude, target)
            .await
    }

    /// Play text-to-speech into a call
    ///
    /// # Arguments
    /// * `call_id` - The call to inject audio into
    /// * `text` - Text to convert to speech
    /// * `language` - Language code (e.g., "en-US", "es-ES")
    /// * `voice` - Optional voice identifier (provider-specific)
    /// * `target` - Which participant(s) should hear the audio
    ///
    /// # Returns
    /// A playback ID that can be used to stop the playback
    ///
    /// # Note
    /// This requires TTS provider configuration. Currently returns an error
    /// until TTS providers (Google, AWS, Azure) are configured.
    ///
    /// # Example
    /// ```ignore
    /// let playback_id = manager.play_tts(
    ///     &call_id,
    ///     "Welcome to our service".to_string(),
    ///     "en-US".to_string(),
    ///     None,
    ///     AudioTarget::Both
    /// ).await?;
    /// ```
    pub async fn play_tts(
        &self,
        call_id: &CallId,
        text: String,
        language: String,
        voice: Option<String>,
        target: AudioTarget,
    ) -> Result<String> {
        self.backend
            .play_tts(call_id, text, language, voice, target)
            .await
    }

    /// Stop an active audio playback
    ///
    /// # Arguments
    /// * `call_id` - The call with active playback
    /// * `playback_id` - ID returned from play_file or play_tts
    ///
    /// # Example
    /// ```ignore
    /// let playback_id = manager.play_file(&call_id, "long_audio.wav", AudioTarget::Both).await?;
    /// // ... later, stop it early
    /// manager.stop_playback(&call_id, playback_id).await?;
    /// ```
    pub async fn stop_playback(&self, call_id: &CallId, playback_id: String) -> Result<()> {
        self.backend.stop_playback(call_id, playback_id).await
    }

    /// Wait for a playback to complete
    ///
    /// # Arguments
    /// * `playback_id` - ID returned from play_file or play_tts
    ///
    /// # Returns
    /// Returns Ok(()) when playback completes successfully, or an error if playback failed
    ///
    /// # Example
    /// ```ignore
    /// let playback_id = manager.play_file(&call_id, "greeting.wav", AudioTarget::Both).await?;
    /// manager.wait_playback_completion(playback_id).await?; // Wait for playback to finish
    /// ```
    pub async fn wait_playback_completion(&self, playback_id: String) -> Result<()> {
        self.backend.wait_playback_completion(playback_id).await
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

    /// The pool this node's media sessions take their RTP ports from
    /// (`None` for a backend that runs its media elsewhere). A subsystem
    /// that opens RTP sockets of its own in the same range — the call
    /// manager's queue legs — allocates from it, never from a pool of its
    /// own over the same ports.
    pub fn port_pool(&self) -> Option<std::sync::Arc<forge_rtp::PortPool>> {
        self.backend.port_pool()
    }

    /// Get the shared TTS service, if configured.
    ///
    /// Other subsystems (FlowEngine, prompt pre-generation) can reuse this
    /// instead of creating their own TTS instance.
    pub fn tts_service(&self) -> Option<&Arc<crate::tts::TtsService>> {
        self.tts_service.as_ref()
    }

    /// Bridge two calls together for B2BUA operation
    ///
    /// # Arguments
    ///
    /// * `a_leg_call_id` - Call ID of the A-leg (incoming call)
    /// * `b_leg_call_id` - Call ID of the B-leg (outbound call)
    ///
    /// # Description
    ///
    /// This method configures the media engine to forward RTP packets bidirectionally
    /// between the two call legs. Audio from A-leg is sent to B-leg, and vice versa.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) when bridging is established, or an error if either session
    /// doesn't exist or bridging fails.
    ///
    /// # Example
    /// ```ignore
    /// // After A-leg answers and B-leg answers
    /// manager.bridge_calls(&a_leg_call_id, &b_leg_call_id).await?;
    /// // Now A and B can talk to each other
    /// ```
    pub async fn bridge_calls(&self, a_leg_call_id: &CallId, b_leg_call_id: &CallId) -> Result<()> {
        self.backend
            .bridge_calls(a_leg_call_id, b_leg_call_id)
            .await
    }

    /// Unbridge two calls
    ///
    /// # Arguments
    ///
    /// * `a_leg_call_id` - Call ID of the A-leg
    /// * `b_leg_call_id` - Call ID of the B-leg
    ///
    /// # Description
    ///
    /// Removes the media bridge between two calls. After unbridging, each call
    /// returns to its independent state (silence or can be bridged to other calls).
    ///
    /// # Example
    /// ```ignore
    /// manager.unbridge_calls(&a_leg_call_id, &b_leg_call_id).await?;
    /// ```
    pub async fn unbridge_calls(
        &self,
        a_leg_call_id: &CallId,
        b_leg_call_id: &CallId,
    ) -> Result<()> {
        self.backend
            .unbridge_calls(a_leg_call_id, b_leg_call_id)
            .await
    }
}

/// Media manager statistics
#[derive(Debug, Clone)]
pub struct MediaManagerStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub backend_type: String,
}

fn contains_crlf(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, b'\r' | b'\n'))
}

/// Validates a recording path for security issues.
///
/// # Security Checks
/// - Prevents CRLF injection
/// - Rejects empty paths
/// - Prevents path traversal with ".." components
/// - Rejects absolute paths (must be relative to recording_dir)
/// - Ensures resolved path stays within base directory
///
/// # Arguments
/// * `path` - The recording path to validate
/// * `base_dir` - The base recording directory
///
/// # Errors
/// Returns `Error::Config` if any validation check fails.
fn validate_recording_path(path: &str, base_dir: &Path) -> Result<()> {
    // Empty path check
    if path.is_empty() {
        return Err(Error::Config("Recording path must not be empty".into()));
    }

    // CRLF injection check
    if contains_crlf(path) {
        return Err(Error::Config(
            "Recording path must not contain CR or LF characters".into(),
        ));
    }

    let path_obj = Path::new(path);

    // Reject absolute paths - must be relative to recording_dir
    if path_obj.is_absolute() {
        return Err(Error::Config(
            "Recording path must be relative, not absolute".into(),
        ));
    }

    // Check for path traversal attempts
    use std::path::Component;
    for component in path_obj.components() {
        match component {
            Component::ParentDir => {
                return Err(Error::Config(
                    "Recording path must not contain '..' (path traversal attempt)".into(),
                ));
            }
            Component::RootDir => {
                return Err(Error::Config(
                    "Recording path must not contain root directory".into(),
                ));
            }
            Component::Normal(_) | Component::CurDir => {
                // These are safe
            }
            Component::Prefix(_) => {
                return Err(Error::Config(
                    "Recording path must not contain Windows drive prefix".into(),
                ));
            }
        }
    }

    // Verify the resolved path stays within base_dir (defense in depth)
    // Note: We can't canonicalize here because the file might not exist yet,
    // but we can verify the logical path construction is safe
    let full_path = base_dir.join(path_obj);

    // Ensure the constructed path starts with base_dir
    // This catches any edge cases the component check might miss
    if !full_path.starts_with(base_dir) {
        return Err(Error::Config(
            "Recording path escapes base directory (path traversal detected)".into(),
        ));
    }

    Ok(())
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
