//! Media backend abstraction
//!
//! Provides a unified interface for both embedded and remote media backends.

use crate::{adapters, Error, MediaSession, MediaSessionConfig, Result};
use async_trait::async_trait;
use fcp_core::{CallId, DialogId};
use std::sync::Arc;

/// Media backend trait
///
/// This trait abstracts over different deployment models:
/// - EmbeddedBackend: forge-media in-process
/// - RemoteBackend: forge-media as HTTP service
#[async_trait]
pub trait MediaBackend: Send + Sync {
    /// Create a new media session
    async fn create_session(
        &self,
        call_id: CallId,
        dialog_id: DialogId,
        config: MediaSessionConfig,
    ) -> Result<Arc<MediaSession>>;

    /// Get an existing session
    async fn get_session(&self, call_id: &CallId) -> Result<Arc<MediaSession>>;

    /// Update session with SDP offer/answer
    async fn update_session(&self, call_id: &CallId, remote_sdp: String) -> Result<String>; // Returns local SDP

    /// Set the B-leg remote endpoint from SDP.
    ///
    /// Used by B2BUA when the second call leg answers. This sets participant_b's
    /// remote address so the ForwardingEngine knows where to send RTP packets
    /// destined for the B-leg endpoint.
    async fn update_session_b_leg(&self, call_id: &CallId, remote_sdp: String) -> Result<()>;

    /// Put session on hold
    async fn hold_session(&self, call_id: &CallId) -> Result<()>;

    /// Resume session from hold
    async fn resume_session(&self, call_id: &CallId) -> Result<()>;

    /// Terminate a media session
    async fn terminate_session(&self, call_id: &CallId) -> Result<()>;

    /// Get session statistics
    async fn get_stats(&self, call_id: &CallId) -> Result<crate::session::MediaSessionStats>;

    /// Send DTMF digit
    async fn send_dtmf(&self, call_id: &CallId, digit: char, duration_ms: u32) -> Result<()>;

    /// Start recording
    async fn start_recording(&self, call_id: &CallId, path: String) -> Result<()>;

    /// Stop recording
    async fn stop_recording(&self, call_id: &CallId) -> Result<()>;

    /// Play an audio file into the call
    ///
    /// # Arguments
    ///
    /// * `call_id` - The call to inject audio into
    /// * `file_path` - Path to the audio file (WAV, MP3, etc.)
    /// * `target` - Which participant(s) should hear the audio
    ///
    /// # Returns
    ///
    /// A playback ID that can be used to stop the playback
    async fn play_file(
        &self,
        call_id: &CallId,
        file_path: String,
        target: AudioTarget,
    ) -> Result<String>;

    /// Play text-to-speech into the call
    ///
    /// # Arguments
    ///
    /// * `call_id` - The call to inject audio into
    /// * `text` - Text to convert to speech
    /// * `language` - Language code (e.g., "en-US")
    /// * `voice` - Voice identifier (provider-specific)
    /// * `target` - Which participant(s) should hear the audio
    ///
    /// # Returns
    ///
    /// A playback ID that can be used to stop the playback
    async fn play_tts(
        &self,
        call_id: &CallId,
        text: String,
        language: String,
        voice: Option<String>,
        target: AudioTarget,
    ) -> Result<String>;

    /// Stop an active playback
    ///
    /// # Arguments
    ///
    /// * `call_id` - The call with active playback
    /// * `playback_id` - ID returned from play_file or play_tts
    async fn stop_playback(&self, call_id: &CallId, playback_id: String) -> Result<()>;

    /// Wait for a playback to complete
    ///
    /// # Arguments
    ///
    /// * `playback_id` - ID returned from play_file or play_tts
    ///
    /// # Returns
    ///
    /// Returns Ok(()) when playback completes successfully, or an error if playback failed
    async fn wait_playback_completion(&self, playback_id: String) -> Result<()>;

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
    async fn bridge_calls(&self, a_leg_call_id: &CallId, b_leg_call_id: &CallId) -> Result<()>;

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
    async fn unbridge_calls(&self, a_leg_call_id: &CallId, b_leg_call_id: &CallId) -> Result<()>;

    /// Get backend type name (for logging/metrics)
    fn backend_name(&self) -> &'static str;

    /// Health check
    async fn health_check(&self) -> Result<bool>;
}

/// Audio injection target
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTarget {
    /// Inject audio to participant A (caller)
    ParticipantA,
    /// Inject audio to participant B (callee)
    ParticipantB,
    /// Inject audio to both participants
    Both,
}

/// Embedded backend - forge-media runs in-process
#[cfg(feature = "embedded")]
pub mod embedded {
    use super::*;
    use crate::MediaSessionStats;
    use dashmap::DashMap;
    use forge_core::{EventBus, ParticipantId};
    use forge_engine::{
        session::{DtmfConfig, ParticipantCodecConfig, TranscodingConfig},
        AudioTarget as ForgeAudioTarget, MediaSession as ForgeMediaSession, MixMode,
        PlaybackHandle, PlaybackManager, SessionManager, SessionManagerConfig,
    };
    use forge_injection::FileSource;
    use forge_rtp::PortPoolConfig;
    use std::time::Duration;
    use tracing::{debug, info, warn};

    pub struct EmbeddedBackend {
        /// forge-media session manager
        session_manager: Arc<SessionManager>,

        /// Audio playback manager
        playback_manager: Arc<PlaybackManager>,

        /// Mapping of FCP CallId to forge-media session and participant IDs
        /// This lets us look up forge sessions using FCP call IDs
        sessions: DashMap<fcp_core::CallId, SessionInfo>,

        /// Active playback handles for waiting on completion
        playback_handles: DashMap<String, PlaybackHandle>,

        /// IP address to advertise in SDP (for RTP media)
        advertise_ip: String,
    }

    /// Information about a forge-media session
    struct SessionInfo {
        /// forge-media session reference
        forge_session: Arc<ForgeMediaSession>,
        /// Forge CallId (converted from FCP CallId)
        forge_call_id: forge_core::CallId,
        /// Participant A ID
        participant_a: ParticipantId,
        /// Participant B ID
        participant_b: ParticipantId,
        /// FCP dialog ID
        dialog_id: fcp_core::DialogId,
    }

    /// Parse remote RTP endpoint from SDP
    /// Extracts IP from c= line and port from m=audio line
    fn parse_remote_endpoint_from_sdp(
        sdp: &str,
    ) -> std::result::Result<std::net::SocketAddr, String> {
        use std::net::IpAddr;

        let mut ip: Option<IpAddr> = None;
        let mut port: Option<u16> = None;

        for line in sdp.lines() {
            let line = line.trim();

            // Parse c= line: c=IN IP4 192.168.1.80
            if line.starts_with("c=") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 && parts[1] == "IP4" {
                    if let Ok(parsed_ip) = parts[2].parse::<IpAddr>() {
                        ip = Some(parsed_ip);
                    }
                } else if parts.len() >= 3 && parts[1] == "IP6" {
                    if let Ok(parsed_ip) = parts[2].parse::<IpAddr>() {
                        ip = Some(parsed_ip);
                    }
                }
            }

            // Parse m= line: m=audio 51163 RTP/AVP 0 8
            if line.starts_with("m=audio ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(parsed_port) = parts[1].parse::<u16>() {
                        port = Some(parsed_port);
                    }
                }
            }
        }

        match (ip, port) {
            (Some(ip), Some(port)) => Ok(std::net::SocketAddr::new(ip, port)),
            _ => Err("Failed to parse IP or port from SDP".to_string()),
        }
    }

    impl EmbeddedBackend {
        pub async fn new(config: &crate::config::MediaConfig) -> Result<Self> {
            info!("Initializing embedded media backend");

            // Create forge-media configuration
            let port_pool_config = PortPoolConfig {
                min_port: config.rtp_port_start,
                max_port: config.rtp_port_end,
            };

            let session_config = forge_engine::MediaSessionConfig {
                socket_config: forge_rtp::RtpSocketConfig::default(),
                session_timeout: Duration::from_secs(300), // 5 minutes
                dtmf_config: DtmfConfig {
                    enable_rfc2833: config.enable_dtmf,
                    enable_inband: config.enable_dtmf,
                    enable_dedup: true,
                    opus_payload_type: Some(111),
                },
                transcoding_config: TranscodingConfig {
                    enable_transcoding: config.enable_transcoding,
                    ..Default::default()
                },
            };

            let manager_config = SessionManagerConfig {
                port_pool_config,
                session_config,
                cleanup_interval: Duration::from_secs(30),
            };

            // Create event bus for forge-media events
            let event_bus = Arc::new(EventBus::new());

            // Initialize SessionManager (returns Arc<Self>)
            let session_manager = SessionManager::new(manager_config, Some(event_bus));

            // Initialize PlaybackManager for audio injection (with session_manager reference)
            let playback_manager = Arc::new(PlaybackManager::new_with_session_manager(
                session_manager.clone(),
            ));

            // Determine the IP address to advertise in SDP
            let advertise_ip = if let Some(ref public_ip) = config.public_ip {
                info!("Using configured public IP for SDP: {}", public_ip);
                public_ip.clone()
            } else {
                warn!("No public_ip configured in media config - using 127.0.0.1 (this will NOT work for remote clients!)");
                warn!("Configure media.public_ip in fcp.toml to fix this");
                "127.0.0.1".to_string()
            };

            info!(
                "Embedded media backend initialized, advertise_ip: {}",
                advertise_ip
            );

            Ok(Self {
                session_manager,
                playback_manager,
                sessions: DashMap::new(),
                playback_handles: DashMap::new(),
                advertise_ip,
            })
        }
    }

    #[async_trait]
    impl MediaBackend for EmbeddedBackend {
        async fn create_session(
            &self,
            call_id: fcp_core::CallId,
            dialog_id: fcp_core::DialogId,
            config: MediaSessionConfig,
        ) -> Result<Arc<MediaSession>> {
            debug!(?call_id, ?dialog_id, "Creating embedded media session");

            // Check if session already exists
            if self.sessions.contains_key(&call_id) {
                return Err(Error::InvalidState(format!(
                    "Session already exists for call {}",
                    call_id
                )));
            }

            // Convert FCP CallId to forge CallId
            let forge_call_id = adapters::fcp_to_forge_call_id(&call_id);

            // Create participant IDs for both legs
            let (participant_a, participant_b) = adapters::create_participant_ids(&call_id);

            // Determine codec configuration
            let codec_config = if let Some(first_codec) = config.preferred_codecs.first() {
                match first_codec.to_uppercase().as_str() {
                    "PCMU" => ParticipantCodecConfig {
                        payload_type: 0,
                        codec: forge_core::AudioCodec::PCMU,
                        clock_rate: 8000,
                    },
                    "PCMA" => ParticipantCodecConfig {
                        payload_type: 8,
                        codec: forge_core::AudioCodec::PCMA,
                        clock_rate: 8000,
                    },
                    "G722" => ParticipantCodecConfig {
                        payload_type: 9,
                        codec: forge_core::AudioCodec::G722,
                        clock_rate: 16000,
                    },
                    "OPUS" | "opus" => ParticipantCodecConfig {
                        payload_type: 111,
                        codec: forge_core::AudioCodec::Opus,
                        clock_rate: 48000,
                    },
                    _ => ParticipantCodecConfig::default(), // Default to PCMU
                }
            } else {
                ParticipantCodecConfig::default()
            };

            // Create forge-media session
            // For now, both participants use the same codec
            // TODO: Handle different codecs per participant
            let forge_session = self
                .session_manager
                .create_session_with_codecs(
                    forge_call_id.clone(),
                    participant_a.clone(),
                    participant_b.clone(),
                    codec_config.clone(),
                    codec_config,
                    None, // SDP will be set later
                    None, // from_tag will be set later
                    None, // to_tag will be set later
                    None, // Use default config
                )
                .await
                .map_err(|e| Error::ForgeMedia(e.to_string()))?;

            // Store session info
            let session_info = SessionInfo {
                forge_session: forge_session.clone(),
                forge_call_id: forge_call_id.clone(),
                participant_a: participant_a.clone(),
                participant_b: participant_b.clone(),
                dialog_id: dialog_id.clone(),
            };

            self.sessions.insert(call_id.clone(), session_info);

            // Start RTP forwarding (required for media to flow)
            forge_session
                .start_forwarding()
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to start forwarding: {}", e)))?;

            // Get RTP/RTCP ports from forge-media session
            let ports = forge_session.ports();

            // Create FCP MediaSession wrapper
            let fcp_session = Arc::new(MediaSession::new(
                call_id.clone(),
                dialog_id,
                forge_call_id.0.clone(), // Use forge CallId string as session ID
                config,
                ports.rtp_port,
                ports.rtcp_port,
            ));

            // Update state to active
            fcp_session
                .set_state(crate::session::MediaSessionState::Active)
                .await;

            info!(
                ?call_id,
                forge_call_id = %forge_call_id,
                ?participant_a,
                ?participant_b,
                "Embedded media session created"
            );

            Ok(fcp_session)
        }

        async fn get_session(&self, call_id: &fcp_core::CallId) -> Result<Arc<MediaSession>> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Get RTP/RTCP ports from forge-media session
            let ports = session_info.forge_session.ports();

            // Reconstruct FCP MediaSession from stored info
            // In a real implementation, we'd cache the FCP session too
            let config = MediaSessionConfig::default(); // TODO: Store original config
            let fcp_session = Arc::new(MediaSession::new(
                call_id.clone(),
                session_info.dialog_id.clone(),
                session_info.forge_call_id.0.clone(),
                config,
                ports.rtp_port,
                ports.rtcp_port,
            ));

            Ok(fcp_session)
        }

        async fn update_session(
            &self,
            call_id: &fcp_core::CallId,
            remote_sdp: String,
        ) -> Result<String> {
            debug!(?call_id, "Updating session with remote SDP");

            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Parse remote SDP to extract client's RTP endpoint
            // This is critical for IVR scenarios where server sends audio first
            if let Ok(remote_endpoint) = parse_remote_endpoint_from_sdp(&remote_sdp) {
                // Set participant_a's remote address (the caller in IVR mode)
                // Leave participant_b's endpoint unset - there's no B-leg in IVR
                let participant_a = session_info.forge_session.participant_a();
                participant_a.write().await.remote_addr = Some(remote_endpoint);

                info!(
                    ?call_id,
                    endpoint = %remote_endpoint,
                    "Set remote RTP endpoint from SDP"
                );
            } else {
                warn!(?call_id, "Failed to parse remote endpoint from SDP - will rely on symmetric RTP learning");
            }

            // Generate local SDP
            let local_sdp = generate_simple_sdp(&session_info, &remote_sdp, &self.advertise_ip)?;

            Ok(local_sdp)
        }

        async fn update_session_b_leg(
            &self,
            call_id: &fcp_core::CallId,
            remote_sdp: String,
        ) -> Result<()> {
            debug!(?call_id, "Setting B-leg remote endpoint from SDP");

            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            if let Ok(remote_endpoint) = parse_remote_endpoint_from_sdp(&remote_sdp) {
                let participant_b = session_info.forge_session.participant_b();
                participant_b.write().await.remote_addr = Some(remote_endpoint);
                info!(
                    ?call_id,
                    endpoint = %remote_endpoint,
                    "Set B-leg remote RTP endpoint from SDP"
                );
            } else {
                warn!(?call_id, "Failed to parse B-leg remote endpoint from SDP");
            }

            Ok(())
        }

        async fn hold_session(&self, call_id: &fcp_core::CallId) -> Result<()> {
            debug!(?call_id, "Putting session on hold");

            let _session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // forge-media doesn't have explicit hold/resume
            // Hold is typically done via SDP re-negotiation (sendonly/inactive)
            // For now, this is a no-op

            info!(?call_id, "Session on hold");
            Ok(())
        }

        async fn resume_session(&self, call_id: &fcp_core::CallId) -> Result<()> {
            debug!(?call_id, "Resuming session from hold");

            let _session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Resume via SDP re-negotiation
            // For now, this is a no-op

            info!(?call_id, "Session resumed");
            Ok(())
        }

        async fn terminate_session(&self, call_id: &fcp_core::CallId) -> Result<()> {
            debug!(?call_id, "Terminating media session");

            if let Some((_, session_info)) = self.sessions.remove(call_id) {
                // Stop recording if active (must do this before terminating session)
                if session_info.forge_session.is_recording().await {
                    if let Err(e) = session_info.forge_session.disable_recording().await {
                        warn!(?call_id, error = %e, "Failed to stop recording during session termination");
                    }
                }

                // Stop forwarding (terminate session)
                session_info
                    .forge_session
                    .stop_forwarding()
                    .await
                    .map_err(|e| Error::ForgeMedia(e.to_string()))?;

                info!(?call_id, "Media session terminated");
            } else {
                warn!(?call_id, "Attempted to terminate non-existent session");
            }

            Ok(())
        }

        async fn get_stats(&self, call_id: &fcp_core::CallId) -> Result<MediaSessionStats> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Get stats from forge-media session
            let stats_a = session_info.forge_session.participant_a_stats().await;
            let stats_b = session_info.forge_session.participant_b_stats().await;

            // Aggregate stats
            Ok(MediaSessionStats {
                packets_sent: stats_a.packets_sent + stats_b.packets_sent,
                packets_received: stats_a.packets_received + stats_b.packets_received,
                bytes_sent: stats_a.bytes_sent + stats_b.bytes_sent,
                bytes_received: stats_a.bytes_received + stats_b.bytes_received,
                packets_lost: stats_a.packets_lost + stats_b.packets_lost,
                jitter_ms: 0.0, // TODO: Calculate from forge stats
                rtt_ms: None,
                current_codec: Some("PCMU".to_string()), // TODO: Get from session
                start_time: None,
                duration_secs: 0,
            })
        }

        async fn send_dtmf(
            &self,
            call_id: &fcp_core::CallId,
            digit: char,
            duration_ms: u32,
        ) -> Result<()> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Send DTMF via forge-media
            // forge-media expects DTMF to be injected as RTP events
            // This is a placeholder - full implementation depends on forge-media API
            debug!(?call_id, digit = %digit, duration_ms, "Sending DTMF");

            drop(session_info); // Explicit drop to release lock
            Ok(())
        }

        async fn start_recording(&self, call_id: &fcp_core::CallId, path: String) -> Result<()> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Determine format from file extension and codec
            // For G.711 codecs (PCMU/PCMA), use 8kHz sample rate
            // For other codecs, use 48kHz
            let format = if path.ends_with(".opus") {
                forge_core::AudioFormat::opus_mono()
            } else {
                // Default to WAV (PCM) at 8kHz for G.711 compatibility
                // Most VoIP codecs operate at 8kHz (G.711, G.729)
                forge_core::AudioFormat::new(8000, 1, forge_core::AudioCodec::PCM)
            };

            debug!(?call_id, path = %path, format = ?format, "Starting recording");

            // Enable recording on forge-media session
            session_info
                .forge_session
                .enable_recording(&path, format)
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to start recording: {}", e)))?;

            info!(?call_id, path = %path, "Recording started");

            Ok(())
        }

        async fn stop_recording(&self, call_id: &fcp_core::CallId) -> Result<()> {
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            debug!(?call_id, "Stopping recording");

            // Disable recording on forge-media session
            session_info
                .forge_session
                .disable_recording()
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to stop recording: {}", e)))?;

            info!(?call_id, "Recording stopped");

            Ok(())
        }

        async fn play_file(
            &self,
            call_id: &fcp_core::CallId,
            file_path: String,
            target: AudioTarget,
        ) -> Result<String> {
            debug!(?call_id, path = %file_path, target = ?target, "Starting audio playback");

            // Get session info to retrieve forge call_id
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            let forge_call_id = session_info.forge_call_id.clone();
            drop(session_info); // Release lock

            // Create FileSource for the audio file
            let source = Box::new(
                FileSource::new(&file_path)
                    .map_err(|e| Error::ForgeMedia(format!("Failed to open audio file: {}", e)))?,
            );

            // Convert FCP AudioTarget to forge AudioTarget
            let forge_target = match target {
                AudioTarget::ParticipantA => ForgeAudioTarget::ParticipantA,
                AudioTarget::ParticipantB => ForgeAudioTarget::ParticipantB,
                AudioTarget::Both => ForgeAudioTarget::Both,
            };

            // Start playback
            let handle = self
                .playback_manager
                .start_playback(forge_call_id, source, forge_target, MixMode::Replace)
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to start playback: {}", e)))?;

            let playback_id = handle.id().to_string();

            // Store handle so we can wait for completion later
            self.playback_handles.insert(playback_id.clone(), handle);

            info!(?call_id, playback_id = %playback_id, "Audio playback started");

            Ok(playback_id)
        }

        async fn play_tts(
            &self,
            call_id: &fcp_core::CallId,
            text: String,
            language: String,
            voice: Option<String>,
            target: AudioTarget,
        ) -> Result<String> {
            debug!(
                ?call_id,
                text = %text,
                language = %language,
                voice = ?voice,
                target = ?target,
                "Starting TTS playback"
            );

            // Get session info to retrieve forge call_id
            let _session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            // Create TtsSource for text-to-speech
            // Note: TtsSource requires a TTS provider to be configured
            // For now, we'll return an error indicating TTS is not yet configured
            // TODO: Add TTS provider configuration (Google, AWS, Azure)
            return Err(Error::InvalidState(
                "TTS provider not yet configured. Use play_file() with pre-recorded audio instead."
                    .to_string(),
            ));

            // Future implementation when TTS providers are added:
            /*
            let source = Box::new(
                TtsSource::new(text, language, voice)
                    .map_err(|e| Error::ForgeMedia(format!("Failed to create TTS source: {}", e)))?,
            );

            let forge_target = match target {
                AudioTarget::ParticipantA => ForgeAudioTarget::ParticipantA,
                AudioTarget::ParticipantB => ForgeAudioTarget::ParticipantB,
                AudioTarget::Both => ForgeAudioTarget::Both,
            };

            let handle = self
                .playback_manager
                .start_playback(forge_call_id, source, forge_target, MixMode::Replace)
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to start TTS playback: {}", e)))?;

            let playback_id = handle.id().to_string();

            info!(?call_id, playback_id = %playback_id, "TTS playback started");

            Ok(playback_id)
            */
        }

        async fn stop_playback(
            &self,
            call_id: &fcp_core::CallId,
            playback_id: String,
        ) -> Result<()> {
            debug!(?call_id, playback_id = %playback_id, "Stopping playback");

            // Parse playback_id (format: "playback-{u64}")
            // Validate format but we'll use stop_all_playbacks for now
            let _id_num = playback_id
                .strip_prefix("playback-")
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| {
                    Error::InvalidState(format!("Invalid playback ID format: {}", playback_id))
                })?;

            // Create PlaybackId from number
            // Note: This is a workaround since PlaybackId::new() generates a new ID
            // We need to reconstruct the PlaybackId from the string representation
            // TODO: Add PlaybackId::from_u64() method to forge-engine
            // For now, we'll use stop_all_playbacks as a workaround

            // Get session info to retrieve forge call_id
            let session_info = self
                .sessions
                .get(call_id)
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))?;

            let forge_call_id = session_info.forge_call_id.clone();
            drop(session_info);

            // Stop all playbacks for this call
            // TODO: Stop specific playback when PlaybackId reconstruction is available
            self.playback_manager
                .stop_all_playbacks(&forge_call_id)
                .await
                .map_err(|e| Error::ForgeMedia(format!("Failed to stop playback: {}", e)))?;

            info!(?call_id, "Playback stopped");

            Ok(())
        }

        async fn wait_playback_completion(&self, playback_id: String) -> Result<()> {
            debug!(playback_id = %playback_id, "Waiting for playback completion");

            // Get the handle from storage
            let handle = self
                .playback_handles
                .remove(&playback_id)
                .ok_or_else(|| {
                    Error::InvalidState(format!(
                        "Playback ID not found or already completed: {}",
                        playback_id
                    ))
                })?
                .1; // DashMap::remove returns (key, value) tuple

            // Wait for completion
            match handle.wait_completion().await {
                Ok(status) => {
                    use forge_engine::PlaybackStatus;
                    match status {
                        PlaybackStatus::Completed => {
                            info!(playback_id = %playback_id, "Playback completed successfully");
                            Ok(())
                        }
                        PlaybackStatus::Stopped => {
                            info!(playback_id = %playback_id, "Playback stopped by request");
                            Ok(())
                        }
                        PlaybackStatus::Failed(reason) => {
                            warn!(playback_id = %playback_id, reason = %reason, "Playback failed");
                            Err(Error::ForgeMedia(format!("Playback failed: {}", reason)))
                        }
                    }
                }
                Err(e) => {
                    warn!(playback_id = %playback_id, error = %e, "Failed to wait for playback completion");
                    Err(Error::ForgeMedia(format!(
                        "Failed to wait for playback: {}",
                        e
                    )))
                }
            }
        }

        async fn bridge_calls(&self, a_leg_call_id: &CallId, b_leg_call_id: &CallId) -> Result<()> {
            info!(
                a_leg = %a_leg_call_id,
                b_leg = %b_leg_call_id,
                "Bridging calls for B2BUA operation"
            );

            let a_session = self
                .sessions
                .get(a_leg_call_id)
                .ok_or_else(|| Error::SessionNotFound(a_leg_call_id.to_string()))?;

            // Single-session mode: both call IDs refer to the same media session
            // (used by handler.rs B2BUA where one session handles both legs)
            if a_leg_call_id == b_leg_call_id {
                let a_addr = a_session.forge_session.participant_a().read().await.remote_addr;
                let b_addr = a_session.forge_session.participant_b().read().await.remote_addr;
                info!(
                    a_leg = %a_leg_call_id,
                    a_endpoint = ?a_addr,
                    b_endpoint = ?b_addr,
                    "Single-session bridge verified"
                );
                return Ok(());
            }

            // Two-session mode: cross-wire endpoints between separate sessions
            // A-leg session's participant_b → B-leg phone address
            // B-leg session's participant_b → A-leg phone address
            let b_session = self
                .sessions
                .get(b_leg_call_id)
                .ok_or_else(|| Error::SessionNotFound(b_leg_call_id.to_string()))?;

            // Get B-leg phone's address from B-leg session's participant_a
            let b_phone_addr = b_session.forge_session.participant_a().read().await.remote_addr;

            // Get A-leg phone's address from A-leg session's participant_a
            let a_phone_addr = a_session.forge_session.participant_a().read().await.remote_addr;

            // Cross-wire: A-leg session forwards to B-leg phone
            if let Some(addr) = b_phone_addr {
                a_session.forge_session.participant_b().write().await.remote_addr = Some(addr);
            }

            // Cross-wire: B-leg session forwards to A-leg phone
            if let Some(addr) = a_phone_addr {
                b_session.forge_session.participant_b().write().await.remote_addr = Some(addr);
            }

            info!(
                a_leg = %a_leg_call_id,
                b_leg = %b_leg_call_id,
                a_endpoint = ?a_phone_addr,
                b_endpoint = ?b_phone_addr,
                "Two-session bridge established"
            );

            Ok(())
        }

        async fn unbridge_calls(
            &self,
            a_leg_call_id: &CallId,
            b_leg_call_id: &CallId,
        ) -> Result<()> {
            info!(
                a_leg = %a_leg_call_id,
                b_leg = %b_leg_call_id,
                "Unbridging calls"
            );

            // Single-session mode: nothing to cross-unwire
            if a_leg_call_id == b_leg_call_id {
                return Ok(());
            }

            // Clear cross-wired participant_b on both sessions
            if let Some(a_session) = self.sessions.get(a_leg_call_id) {
                a_session.forge_session.participant_b().write().await.remote_addr = None;
            }
            if let Some(b_session) = self.sessions.get(b_leg_call_id) {
                b_session.forge_session.participant_b().write().await.remote_addr = None;
            }

            info!(
                a_leg = %a_leg_call_id,
                b_leg = %b_leg_call_id,
                "Call bridge removed"
            );

            Ok(())
        }

        fn backend_name(&self) -> &'static str {
            "embedded"
        }

        async fn health_check(&self) -> Result<bool> {
            // Check if SessionManager is healthy
            // For now, just check if we can access it
            Ok(true)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_parse_remote_endpoint_ipv4() {
            let sdp = "v=0\r\n\
                        o=- 123 1 IN IP4 192.168.1.100\r\n\
                        s=Test\r\n\
                        c=IN IP4 192.168.1.100\r\n\
                        t=0 0\r\n\
                        m=audio 51163 RTP/AVP 0 8\r\n";
            let result = parse_remote_endpoint_from_sdp(sdp);
            assert!(result.is_ok());
            let addr = result.unwrap();
            assert_eq!(addr.ip().to_string(), "192.168.1.100");
            assert_eq!(addr.port(), 51163);
        }

        #[test]
        fn test_parse_remote_endpoint_missing_ip() {
            let sdp = "v=0\r\n\
                        m=audio 51163 RTP/AVP 0 8\r\n";
            let result = parse_remote_endpoint_from_sdp(sdp);
            assert!(result.is_err());
        }

        #[test]
        fn test_parse_remote_endpoint_missing_port() {
            let sdp = "v=0\r\n\
                        c=IN IP4 192.168.1.100\r\n";
            let result = parse_remote_endpoint_from_sdp(sdp);
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_update_session_b_leg_sets_participant_b() {
            let config = crate::config::MediaConfig {
                rtp_port_start: 40000,
                rtp_port_end: 41000,
                ..Default::default()
            };
            let backend = EmbeddedBackend::new(&config).await.unwrap();

            let call_id = fcp_core::CallId::new();
            let dialog_id = fcp_core::DialogId::new();
            let session_config = crate::MediaSessionConfig::default();

            backend
                .create_session(call_id.clone(), dialog_id, session_config)
                .await
                .unwrap();

            let b_leg_sdp = "v=0\r\n\
                              o=- 456 1 IN IP4 10.0.0.50\r\n\
                              s=Test\r\n\
                              c=IN IP4 10.0.0.50\r\n\
                              t=0 0\r\n\
                              m=audio 6000 RTP/AVP 0\r\n"
                .to_string();

            backend
                .update_session_b_leg(&call_id, b_leg_sdp)
                .await
                .unwrap();

            // Verify participant_b remote_addr is set
            let b_addr = {
                let session_info = backend.sessions.get(&call_id).unwrap();
                let participant_b = session_info.forge_session.participant_b();
                let addr = participant_b.read().await.remote_addr;
                addr
            };
            assert!(b_addr.is_some());
            let addr = b_addr.unwrap();
            assert_eq!(addr.ip().to_string(), "10.0.0.50");
            assert_eq!(addr.port(), 6000);

            backend.terminate_session(&call_id).await.unwrap();
        }

        #[tokio::test]
        async fn test_bridge_calls_single_session() {
            let config = crate::config::MediaConfig {
                rtp_port_start: 41000,
                rtp_port_end: 42000,
                ..Default::default()
            };
            let backend = EmbeddedBackend::new(&config).await.unwrap();

            let call_id = fcp_core::CallId::new();
            let dialog_id = fcp_core::DialogId::new();

            backend
                .create_session(call_id.clone(), dialog_id, crate::MediaSessionConfig::default())
                .await
                .unwrap();

            // Single-session bridge: same call_id for both legs
            let result = backend.bridge_calls(&call_id, &call_id).await;
            assert!(result.is_ok());

            backend.terminate_session(&call_id).await.unwrap();
        }

        #[tokio::test]
        async fn test_bridge_calls_two_sessions() {
            let config = crate::config::MediaConfig {
                rtp_port_start: 42000,
                rtp_port_end: 43000,
                ..Default::default()
            };
            let backend = EmbeddedBackend::new(&config).await.unwrap();

            let call_id_a = fcp_core::CallId::new();
            let call_id_b = fcp_core::CallId::new();

            // Create A-leg session with known remote endpoint
            backend
                .create_session(
                    call_id_a.clone(),
                    fcp_core::DialogId::new(),
                    crate::MediaSessionConfig::default(),
                )
                .await
                .unwrap();

            // Set A-leg participant_a remote addr
            let a_sdp = "v=0\r\nc=IN IP4 10.0.0.1\r\nm=audio 5000 RTP/AVP 0\r\n";
            backend.update_session(&call_id_a, a_sdp.to_string()).await.unwrap();

            // Create B-leg session
            backend
                .create_session(
                    call_id_b.clone(),
                    fcp_core::DialogId::new(),
                    crate::MediaSessionConfig::default(),
                )
                .await
                .unwrap();

            // Set B-leg participant_a remote addr
            let b_sdp = "v=0\r\nc=IN IP4 10.0.0.2\r\nm=audio 6000 RTP/AVP 0\r\n";
            backend.update_session(&call_id_b, b_sdp.to_string()).await.unwrap();

            // Bridge calls
            backend.bridge_calls(&call_id_a, &call_id_b).await.unwrap();

            // Verify cross-wiring: A-leg's participant_b should point to B-leg phone (10.0.0.2:6000)
            {
                let a_session = backend.sessions.get(&call_id_a).unwrap();
                let b_remote = a_session.forge_session.participant_b().read().await.remote_addr;
                assert!(b_remote.is_some());
                let addr = b_remote.unwrap();
                assert_eq!(addr.ip().to_string(), "10.0.0.2");
                assert_eq!(addr.port(), 6000);
            }

            // Verify cross-wiring: B-leg's participant_b should point to A-leg phone (10.0.0.1:5000)
            {
                let b_session = backend.sessions.get(&call_id_b).unwrap();
                let a_remote = b_session.forge_session.participant_b().read().await.remote_addr;
                assert!(a_remote.is_some());
                let addr = a_remote.unwrap();
                assert_eq!(addr.ip().to_string(), "10.0.0.1");
                assert_eq!(addr.port(), 5000);
            }

            backend.terminate_session(&call_id_a).await.unwrap();
            backend.terminate_session(&call_id_b).await.unwrap();
        }

        #[tokio::test]
        async fn test_unbridge_calls_clears_participant_b() {
            let config = crate::config::MediaConfig {
                rtp_port_start: 43000,
                rtp_port_end: 44000,
                ..Default::default()
            };
            let backend = EmbeddedBackend::new(&config).await.unwrap();

            let call_id_a = fcp_core::CallId::new();
            let call_id_b = fcp_core::CallId::new();

            backend
                .create_session(
                    call_id_a.clone(),
                    fcp_core::DialogId::new(),
                    crate::MediaSessionConfig::default(),
                )
                .await
                .unwrap();

            backend
                .create_session(
                    call_id_b.clone(),
                    fcp_core::DialogId::new(),
                    crate::MediaSessionConfig::default(),
                )
                .await
                .unwrap();

            // Set up endpoints and bridge
            let a_sdp = "v=0\r\nc=IN IP4 10.0.0.1\r\nm=audio 5000 RTP/AVP 0\r\n";
            let b_sdp = "v=0\r\nc=IN IP4 10.0.0.2\r\nm=audio 6000 RTP/AVP 0\r\n";
            backend.update_session(&call_id_a, a_sdp.to_string()).await.unwrap();
            backend.update_session(&call_id_b, b_sdp.to_string()).await.unwrap();
            backend.bridge_calls(&call_id_a, &call_id_b).await.unwrap();

            // Unbridge
            backend.unbridge_calls(&call_id_a, &call_id_b).await.unwrap();

            // Verify participant_b remote_addr cleared on both
            {
                let a_session = backend.sessions.get(&call_id_a).unwrap();
                let b_remote = a_session.forge_session.participant_b().read().await.remote_addr;
                assert!(b_remote.is_none());
            }
            {
                let b_session = backend.sessions.get(&call_id_b).unwrap();
                let a_remote = b_session.forge_session.participant_b().read().await.remote_addr;
                assert!(a_remote.is_none());
            }

            backend.terminate_session(&call_id_a).await.unwrap();
            backend.terminate_session(&call_id_b).await.unwrap();
        }
    }

    /// Generate a simple SDP for testing
    /// TODO: Use forge-sdp for proper SDP generation
    fn generate_simple_sdp(
        session_info: &SessionInfo,
        _remote_sdp: &str,
        advertise_ip: &str,
    ) -> Result<String> {
        let ports = session_info.forge_session.ports();

        let sdp = format!(
            "v=0\r\n\
             o=- {} 1 IN IP4 {}\r\n\
             s=FCP Media Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 8\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=sendrecv\r\n",
            session_info.forge_call_id.0, advertise_ip, advertise_ip, ports.rtp_port
        );

        Ok(sdp)
    }
}

#[cfg(feature = "embedded")]
pub use embedded::EmbeddedBackend;

/// Remote backend - forge-media runs as HTTP service
#[cfg(feature = "remote")]
pub mod remote {
    use super::*;
    use crate::{config::BackendType, MediaSessionStats};
    use dashmap::DashMap;
    use reqwest::Client;
    use serde::{Deserialize, Serialize};
    use tracing::{debug, info};

    pub struct RemoteBackend {
        /// HTTP client
        client: Client,

        /// Base URL of forge-media server
        base_url: String,

        /// Optional authentication token
        auth_token: Option<String>,

        /// Session cache (CallId -> MediaSession)
        sessions: DashMap<fcp_core::CallId, Arc<MediaSession>>,
    }

    impl RemoteBackend {
        pub async fn new(config: &crate::config::MediaConfig) -> Result<Self> {
            let (base_url, auth_token, timeout_ms) = match &config.backend {
                BackendType::Remote {
                    endpoint,
                    auth_token,
                    timeout_ms,
                    ..
                } => (endpoint.clone(), auth_token.clone(), *timeout_ms),
                _ => {
                    return Err(Error::Config(
                        "RemoteBackend requires Remote backend type".into(),
                    ))
                }
            };

            info!(base_url, "Initializing remote media backend");

            let client = Client::builder()
                .timeout(std::time::Duration::from_millis(timeout_ms))
                .build()
                .map_err(|e| Error::Config(format!("Failed to create HTTP client: {}", e)))?;

            // Health check
            let health_url = format!("{}/health", base_url);
            client
                .get(&health_url)
                .send()
                .await
                .map_err(|e| Error::Backend(format!("forge-media not reachable: {}", e)))?;

            info!("Remote media backend connected");

            Ok(Self {
                client,
                base_url,
                auth_token,
                sessions: DashMap::new(),
            })
        }

        fn auth_header(&self) -> Option<String> {
            self.auth_token
                .as_ref()
                .map(|token| format!("Bearer {}", token))
        }
    }

    #[derive(Serialize)]
    struct CreateSessionReq {
        call_id: String,
    }

    #[derive(Deserialize)]
    struct CreateSessionResp {
        session_id: String,
    }

    #[async_trait]
    impl MediaBackend for RemoteBackend {
        async fn create_session(
            &self,
            call_id: fcp_core::CallId,
            dialog_id: fcp_core::DialogId,
            config: MediaSessionConfig,
        ) -> Result<Arc<MediaSession>> {
            debug!(?call_id, ?dialog_id, "Creating remote media session");

            let req = CreateSessionReq {
                call_id: call_id.to_string(),
            };

            let url = format!("{}/v1/sessions", self.base_url);
            let mut request = self.client.post(&url).json(&req);

            if let Some(auth) = self.auth_header() {
                request = request.header("Authorization", auth);
            }

            let resp: CreateSessionResp = request.send().await?.json().await?;

            // TODO: Get actual RTP/RTCP ports from remote forge-media API
            // For now, use placeholder values since remote mode is future work
            let rtp_port = 0;
            let rtcp_port = 0;

            let session = Arc::new(MediaSession::new(
                call_id.clone(),
                dialog_id,
                resp.session_id,
                config,
                rtp_port,
                rtcp_port,
            ));

            self.sessions.insert(call_id.clone(), session.clone());

            info!(
                ?call_id,
                forge_session_id = session.forge_session_id(),
                "Remote media session created"
            );

            Ok(session)
        }

        async fn get_session(&self, call_id: &fcp_core::CallId) -> Result<Arc<MediaSession>> {
            self.sessions
                .get(call_id)
                .map(|entry| entry.value().clone())
                .ok_or_else(|| Error::SessionNotFound(call_id.to_string()))
        }

        async fn update_session(
            &self,
            call_id: &fcp_core::CallId,
            remote_sdp: String,
        ) -> Result<String> {
            debug!(?call_id, "Updating remote session with SDP");

            let session = self.get_session(call_id).await?;

            #[derive(Serialize)]
            struct UpdateReq {
                sdp: String,
            }

            #[derive(Deserialize)]
            struct UpdateResp {
                sdp: String,
            }

            let url = format!(
                "{}/v1/sessions/{}/sdp",
                self.base_url,
                session.forge_session_id()
            );
            let mut request = self.client.post(&url).json(&UpdateReq {
                sdp: remote_sdp.clone(),
            });

            if let Some(auth) = self.auth_header() {
                request = request.header("Authorization", auth);
            }

            let resp: UpdateResp = request.send().await?.json().await?;

            session.set_remote_sdp(remote_sdp).await;
            session.set_local_sdp(resp.sdp.clone()).await;
            session
                .set_state(crate::session::MediaSessionState::Active)
                .await;

            Ok(resp.sdp)
        }

        async fn update_session_b_leg(
            &self,
            _call_id: &fcp_core::CallId,
            _remote_sdp: String,
        ) -> Result<()> {
            // POST /v1/sessions/{id}/sdp-b-leg
            todo!("Implement remote B-leg SDP update")
        }

        async fn hold_session(&self, _call_id: &fcp_core::CallId) -> Result<()> {
            // POST /v1/sessions/{id}/hold
            todo!("Implement remote hold")
        }

        async fn resume_session(&self, _call_id: &fcp_core::CallId) -> Result<()> {
            // POST /v1/sessions/{id}/resume
            todo!("Implement remote resume")
        }

        async fn terminate_session(&self, call_id: &fcp_core::CallId) -> Result<()> {
            debug!(?call_id, "Terminating remote media session");

            if let Some((_, session)) = self.sessions.remove(call_id) {
                session
                    .set_state(crate::session::MediaSessionState::Terminated)
                    .await;

                let url = format!(
                    "{}/v1/sessions/{}",
                    self.base_url,
                    session.forge_session_id()
                );
                let mut request = self.client.delete(&url);

                if let Some(auth) = self.auth_header() {
                    request = request.header("Authorization", auth);
                }

                request.send().await?;

                info!(?call_id, "Remote media session terminated");
            }

            Ok(())
        }

        async fn get_stats(&self, call_id: &fcp_core::CallId) -> Result<MediaSessionStats> {
            let session = self.get_session(call_id).await?;

            let url = format!(
                "{}/v1/sessions/{}/stats",
                self.base_url,
                session.forge_session_id()
            );
            let mut request = self.client.get(&url);

            if let Some(auth) = self.auth_header() {
                request = request.header("Authorization", auth);
            }

            let stats: MediaSessionStats = request.send().await?.json().await?;
            Ok(stats)
        }

        async fn send_dtmf(
            &self,
            _call_id: &fcp_core::CallId,
            _digit: char,
            _duration_ms: u32,
        ) -> Result<()> {
            // POST /v1/sessions/{id}/dtmf
            todo!("Implement remote DTMF")
        }

        async fn start_recording(&self, _call_id: &fcp_core::CallId, _path: String) -> Result<()> {
            // POST /v1/sessions/{id}/recording
            todo!("Implement remote recording")
        }

        async fn stop_recording(&self, _call_id: &fcp_core::CallId) -> Result<()> {
            // DELETE /v1/sessions/{id}/recording
            todo!("Implement remote recording stop")
        }

        async fn play_file(
            &self,
            _call_id: &fcp_core::CallId,
            _file_path: String,
            _target: AudioTarget,
        ) -> Result<String> {
            // POST /v1/sessions/{id}/playback/file
            todo!("Implement remote audio playback")
        }

        async fn play_tts(
            &self,
            _call_id: &fcp_core::CallId,
            _text: String,
            _language: String,
            _voice: Option<String>,
            _target: AudioTarget,
        ) -> Result<String> {
            // POST /v1/sessions/{id}/playback/tts
            todo!("Implement remote TTS playback")
        }

        async fn stop_playback(
            &self,
            _call_id: &fcp_core::CallId,
            _playback_id: String,
        ) -> Result<()> {
            // DELETE /v1/sessions/{id}/playback/{playback_id}
            todo!("Implement remote playback stop")
        }

        async fn wait_playback_completion(&self, _playback_id: String) -> Result<()> {
            // GET /v1/playback/{playback_id}/wait or use websocket for completion events
            todo!("Implement remote playback completion waiting")
        }

        async fn bridge_calls(
            &self,
            _a_leg_call_id: &CallId,
            _b_leg_call_id: &CallId,
        ) -> Result<()> {
            // POST /v1/bridge
            // Body: { "a_leg": "call-id-1", "b_leg": "call-id-2" }
            todo!("Implement remote call bridging")
        }

        async fn unbridge_calls(
            &self,
            _a_leg_call_id: &CallId,
            _b_leg_call_id: &CallId,
        ) -> Result<()> {
            // DELETE /v1/bridge
            // Body: { "a_leg": "call-id-1", "b_leg": "call-id-2" }
            todo!("Implement remote call unbridging")
        }

        fn backend_name(&self) -> &'static str {
            "remote"
        }

        async fn health_check(&self) -> Result<bool> {
            let url = format!("{}/health", self.base_url);
            let resp = self.client.get(&url).send().await?;
            Ok(resp.status().is_success())
        }
    }
}

#[cfg(feature = "remote")]
pub use remote::RemoteBackend;
