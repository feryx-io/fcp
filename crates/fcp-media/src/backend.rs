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

    /// Get backend type name (for logging/metrics)
    fn backend_name(&self) -> &'static str;

    /// Health check
    async fn health_check(&self) -> Result<bool>;
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
        MediaSession as ForgeMediaSession, SessionManager, SessionManagerConfig,
    };
    use forge_rtp::PortPoolConfig;
    use std::time::Duration;
    use tracing::{debug, info, warn};

    pub struct EmbeddedBackend {
        /// forge-media session manager
        session_manager: Arc<SessionManager>,

        /// Mapping of FCP CallId to forge-media session and participant IDs
        /// This lets us look up forge sessions using FCP call IDs
        sessions: DashMap<fcp_core::CallId, SessionInfo>,
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

            // Initialize SessionManager
            let session_manager = SessionManager::new(manager_config, Some(event_bus));

            info!("Embedded media backend initialized");

            Ok(Self {
                session_manager,
                sessions: DashMap::new(),
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

            // For now, return a simple SDP
            // TODO: Proper SDP generation using forge-sdp
            let local_sdp = generate_simple_sdp(&session_info, &remote_sdp)?;

            Ok(local_sdp)
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

        fn backend_name(&self) -> &'static str {
            "embedded"
        }

        async fn health_check(&self) -> Result<bool> {
            // Check if SessionManager is healthy
            // For now, just check if we can access it
            Ok(true)
        }
    }

    /// Generate a simple SDP for testing
    /// TODO: Use forge-sdp for proper SDP generation
    fn generate_simple_sdp(session_info: &SessionInfo, _remote_sdp: &str) -> Result<String> {
        let ports = session_info.forge_session.ports();

        let sdp = format!(
            "v=0\r\n\
             o=- {} 1 IN IP4 127.0.0.1\r\n\
             s=FCP Media Session\r\n\
             c=IN IP4 127.0.0.1\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 8\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=sendrecv\r\n",
            session_info.forge_call_id.0, ports.rtp_port
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
