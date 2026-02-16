use anyhow::{Context, Result};
use dashmap::DashMap;
use fcp_core::{DynStorage, Meeting, TenantId};
use fcp_events::{ConferenceEvent, EventPublisher, PlatformEvent};
use fcp_media::{
    construct_answer_for_a_leg, decode_sdes_key_params, generate_srtp_key_material,
    parse_sdes_crypto, parse_sdp_codecs, SrtpSdpContext,
};
use fcp_sip::prelude::SipUserAgentClient;
use forge_codecs::{g711, g722, AudioCodec};
use forge_conference::{
    ConferenceAIConfig, ConferenceAIManager, ConferenceBridge, ConferenceConfig,
    ConferenceDtmfConfig, ConferenceError, DtmfCommandHandler, PinAuthResult, RoomConfig,
};
use forge_engine::{AISessionConfig, AISessionManager};
use forge_dtmf::rfc2833::Rfc2833Detector;
use forge_dtmf::{DtmfDigit, DtmfEventType};
use forge_rtp::{PortPair, PortPool, RemoteEndpoint, RtpSocketConfig, RtpSocketPair, SrtpContext, SrtpKeyMaterial};
use sip_core::{SipUri, Uri};
use sip_dialog::Dialog;
use sip_uas::UserAgentServer;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use parking_lot::RwLock;
use std::time::{Duration, Instant, SystemTime};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::audio_bridge::{self, AudioStats};
use crate::config::{AiSection, ConferenceSection};

/// Fallback remote address used when SDP parsing fails to extract the caller's RTP endpoint.
const UNKNOWN_REMOTE: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0);

/// Tracks a single participant's connection to a conference room.
pub struct ParticipantSession {
    pub call_id: String,
    pub room_number: String,
    pub participant_id: String,
    pub is_host: bool,
    pub audio_bridge_handle: JoinHandle<()>,
    pub port_pair: PortPair,
    pub rtp_socket: Arc<RtpSocketPair>,
    pub remote_addr: SocketAddr,
    pub codec_name: String,
    pub tenant_id: TenantId,
    pub dialog: Dialog,
    pub joined_at: SystemTime,
    pub te_pt: Option<u8>,
    pub stats: Arc<AudioStats>,
    /// Display name from SIP From header or P-Asserted-Identity
    pub display_name: Option<String>,
    /// Caller ID (SIP URI user part or tel number)
    pub caller_id: Option<String>,
    /// SRTP context for encrypting/decrypting RTP packets (None = plain RTP)
    pub srtp_context: Option<Arc<tokio::sync::RwLock<SrtpContext>>>,
}

/// Handles SIP INVITEs for conference rooms.
///
/// When a SIP INVITE arrives targeting a conference room number:
/// 1. Looks up the meeting config from storage
/// 2. Answers the call and sets up RTP
/// 3. Optionally collects a PIN for authentication
/// 4. Creates/joins the forge-conference room (with limits enforced)
/// 5. Spawns a per-participant audio bridge task
/// 6. Publishes conference events (join/leave)
pub struct ConferenceCallHandler {
    pub bridge: Arc<ConferenceBridge>,
    pub storage: Arc<DynStorage>,
    pub conference_config: RwLock<ConferenceSection>,
    sessions: DashMap<String, ParticipantSession>,
    pub local_uri: String,
    pub local_addr: String,
    port_pool: Arc<PortPool>,
    event_bus: Arc<dyn EventPublisher>,
    uac: Arc<SipUserAgentClient>,
    dtmf_handlers: DashMap<String, Arc<DtmfCommandHandler>>,
    /// In-memory meeting cache: (TenantId, room_number) → (Meeting, cached_at)
    meeting_cache: DashMap<(TenantId, String), (Meeting, Instant)>,
    /// Shared AI session manager (created once, reused for all rooms)
    ai_session_manager: Arc<AISessionManager>,
    pub ai_config: RwLock<AiSection>,
}

impl ConferenceCallHandler {
    pub fn new(
        bridge: Arc<ConferenceBridge>,
        storage: Arc<DynStorage>,
        conference_config: ConferenceSection,
        local_uri: String,
        local_addr: String,
        port_pool: Arc<PortPool>,
        event_bus: Arc<dyn EventPublisher>,
        uac: Arc<SipUserAgentClient>,
        ai_config: AiSection,
    ) -> Self {
        Self {
            bridge,
            storage,
            conference_config: RwLock::new(conference_config),
            sessions: DashMap::new(),
            local_uri,
            local_addr,
            port_pool,
            event_bus,
            uac,
            dtmf_handlers: DashMap::new(),
            meeting_cache: DashMap::new(),
            ai_session_manager: Arc::new(AISessionManager::new()),
            ai_config: RwLock::new(ai_config),
        }
    }

    /// Look up a meeting by room number, with in-memory cache and DB fallback.
    ///
    /// Cache strategy:
    /// - On cache hit within TTL: return cached meeting (no DB query)
    /// - On cache miss or expired: query DB, update cache on success
    /// - On DB failure: return stale cached entry if available (graceful degradation)
    pub async fn lookup_meeting(
        &self,
        tenant_id: &TenantId,
        room_number: &str,
    ) -> Result<Option<Meeting>> {
        use fcp_core::storage::ConferenceStore;

        let cache_key = (tenant_id.clone(), room_number.to_string());
        let ttl = Duration::from_secs(self.conference_config.read().cache_ttl_secs);

        // Check cache first (if TTL > 0)
        if ttl > Duration::ZERO {
            if let Some(entry) = self.meeting_cache.get(&cache_key) {
                let (meeting, cached_at) = entry.value();
                if cached_at.elapsed() < ttl {
                    return Ok(Some(meeting.clone()));
                }
            }
        }

        // Query database
        match self
            .storage
            .get_meeting_by_room_number(tenant_id, room_number)
            .await
        {
            Ok(Some(meeting)) => {
                if ttl > Duration::ZERO {
                    self.meeting_cache
                        .insert(cache_key, (meeting.clone(), Instant::now()));
                }
                Ok(Some(meeting))
            }
            Ok(None) => {
                // Meeting doesn't exist — remove stale cache entry if any
                self.meeting_cache.remove(&cache_key);
                Ok(None)
            }
            Err(e) => {
                // DB failure — return stale cache if available
                if let Some(entry) = self.meeting_cache.get(&cache_key) {
                    let (meeting, cached_at) = entry.value();
                    warn!(
                        room = %room_number,
                        stale_secs = cached_at.elapsed().as_secs(),
                        error = %e,
                        "DB lookup failed, using stale cached meeting"
                    );
                    return Ok(Some(meeting.clone()));
                }
                Err(e)
            }
        }
    }

    /// Invalidate cached meeting entry (call after update/delete).
    pub fn invalidate_meeting_cache(&self, tenant_id: &TenantId, room_number: &str) {
        self.meeting_cache
            .remove(&(tenant_id.clone(), room_number.to_string()));
    }

    /// Check if a call ID belongs to an active conference session.
    pub fn has_session(&self, call_id: &str) -> bool {
        self.sessions.contains_key(call_id)
    }

    /// Get the total number of active participant sessions.
    pub fn active_sessions(&self) -> usize {
        self.sessions.len()
    }

    /// List all active rooms with participant counts.
    pub fn active_rooms(&self) -> Vec<(String, usize)> {
        self.bridge
            .list_rooms()
            .into_iter()
            .filter_map(|room_id| {
                self.bridge
                    .get_room(&room_id)
                    .ok()
                    .map(|room| (room_id, room.participant_count()))
            })
            .collect()
    }

    /// Get participant list for a specific room.
    pub fn room_participants(&self, room_number: &str) -> Option<Vec<String>> {
        self.bridge
            .get_room(room_number)
            .ok()
            .map(|room| room.participants())
    }

    /// Get detailed metadata for a specific participant in a room.
    pub fn get_participant_details(
        &self,
        room_number: &str,
        participant_id: &str,
    ) -> Result<serde_json::Value> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        let meta = room
            .get_participant_metadata(participant_id)
            .map_err(|e| anyhow::anyhow!("Participant not found: {}", e))?;

        // Enrich with session data (codec, remote addr, stats)
        let session_data = self
            .sessions
            .iter()
            .find(|entry| entry.value().participant_id == participant_id);

        let mut json = serde_json::json!({
            "participant_id": meta.id,
            "state": format!("{:?}", meta.state),
            "gain": meta.gain,
            "is_recording": meta.is_recording,
            "packets_received": meta.packets_received,
            "is_speaking": meta.is_speaking,
            "is_host": room.is_host(participant_id),
        });

        if let Some(entry) = session_data {
            let session = entry.value();
            let joined_secs = session
                .joined_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            json["codec"] = serde_json::json!(session.codec_name);
            json["remote_addr"] = serde_json::json!(session.remote_addr.to_string());
            json["joined_at"] = serde_json::json!(joined_secs);
            json["call_id"] = serde_json::json!(session.call_id);
            if let Some(ref dn) = session.display_name {
                json["display_name"] = serde_json::json!(dn);
            }
            if let Some(ref cid) = session.caller_id {
                json["caller_id"] = serde_json::json!(cid);
            }
            json["stats"] = serde_json::json!({
                "packets_received": session.stats.packets_received.load(Ordering::Relaxed),
                "packets_decoded": session.stats.packets_decoded.load(Ordering::Relaxed),
                "decode_errors": session.stats.decode_errors.load(Ordering::Relaxed),
                "packets_sent": session.stats.packets_sent.load(Ordering::Relaxed),
                "encode_errors": session.stats.encode_errors.load(Ordering::Relaxed),
                "dtmf_events": session.stats.dtmf_events.load(Ordering::Relaxed),
            });
        }

        Ok(json)
    }

    /// List participants waiting for moderator in a room.
    pub fn list_waiting(&self, room_number: &str) -> Result<Vec<String>> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        Ok(room.waiting_participants())
    }

    /// Promote a participant to host (releases waiting participants).
    pub fn promote_participant(&self, room_number: &str, participant_id: &str) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        room.promote_to_host(participant_id)
            .map_err(|e| anyhow::anyhow!("Failed to promote: {}", e))
    }

    /// Admit a specific waiting participant into the conference (admin bypass).
    pub fn admit_waiting(&self, room_number: &str, participant_id: &str) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        room.admit_waiting_participant(participant_id)
            .map_err(|e| anyhow::anyhow!("Failed to admit: {}", e))
    }

    /// Admit all waiting participants into the conference (admin bypass).
    pub fn admit_all_waiting(&self, room_number: &str) -> Result<usize> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        Ok(room.admit_all_waiting())
    }

    /// Set participant audio gain.
    pub fn set_gain(&self, room_number: &str, participant_id: &str, gain: f32) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;
        room.set_participant_gain(participant_id, gain)
            .map_err(|e| anyhow::anyhow!("Failed to set gain: {}", e))
    }

    /// Get aggregated room stats.
    pub fn get_room_stats(&self, room_number: &str) -> Result<serde_json::Value> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        let all_meta = room.get_all_participant_metadata();
        let participants: Vec<serde_json::Value> = all_meta
            .iter()
            .map(|meta| {
                let mut p = serde_json::json!({
                    "participant_id": meta.id,
                    "state": format!("{:?}", meta.state),
                    "gain": meta.gain,
                    "is_recording": meta.is_recording,
                    "is_speaking": meta.is_speaking,
                    "packets_received": meta.packets_received,
                });

                // Enrich with session data
                if let Some(entry) = self.sessions.iter().find(|e| e.value().participant_id == meta.id) {
                    let session = entry.value();
                    if let Some(ref dn) = session.display_name {
                        p["display_name"] = serde_json::json!(dn);
                    }
                    if let Some(ref cid) = session.caller_id {
                        p["caller_id"] = serde_json::json!(cid);
                    }
                    let s = &session.stats;
                    p["rtp_stats"] = serde_json::json!({
                        "packets_received": s.packets_received.load(Ordering::Relaxed),
                        "packets_decoded": s.packets_decoded.load(Ordering::Relaxed),
                        "decode_errors": s.decode_errors.load(Ordering::Relaxed),
                        "packets_sent": s.packets_sent.load(Ordering::Relaxed),
                        "encode_errors": s.encode_errors.load(Ordering::Relaxed),
                        "dtmf_events": s.dtmf_events.load(Ordering::Relaxed),
                    });
                }

                p
            })
            .collect();

        Ok(serde_json::json!({
            "room_number": room_number,
            "participant_count": room.participant_count(),
            "waiting_count": room.waiting_count(),
            "is_locked": room.is_locked(),
            "is_recording": room.is_recording(),
            "host_count": room.host_count(),
            "participants": participants,
        }))
    }

    /// Start per-participant recording.
    pub async fn start_participant_recording(
        &self,
        room_number: &str,
        participant_id: &str,
    ) -> Result<String> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        let timestamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = format!(
            "{}/{}-{}-{}.wav",
            self.conference_config.read().recordings_dir, room_number, participant_id, timestamp
        );

        room.start_participant_recording(participant_id, &path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start recording: {}", e))?;

        info!(
            room = %room_number,
            participant = %participant_id,
            path = %path,
            "Per-participant recording started"
        );
        Ok(path)
    }

    /// Stop per-participant recording.
    pub fn stop_participant_recording(
        &self,
        room_number: &str,
        participant_id: &str,
    ) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        room.stop_participant_recording(participant_id)
            .map_err(|e| anyhow::anyhow!("Failed to stop recording: {}", e))?;

        info!(
            room = %room_number,
            participant = %participant_id,
            "Per-participant recording stopped"
        );
        Ok(())
    }

    /// Play an audio announcement into a conference room.
    pub async fn play_announcement(&self, room_number: &str, audio_path: &str) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        room.play_announcement(audio_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to play announcement: {}", e))?;

        info!(room = %room_number, path = %audio_path, "Announcement played");
        Ok(())
    }

    /// Get or create DTMF command handler for a room.
    fn get_or_create_dtmf_handler(&self, room_number: &str) -> Arc<DtmfCommandHandler> {
        if let Some(handler) = self.dtmf_handlers.get(room_number) {
            return Arc::clone(handler.value());
        }
        let config = ConferenceDtmfConfig::default();
        let handler = Arc::new(DtmfCommandHandler::new(room_number.to_string(), config));
        self.dtmf_handlers
            .insert(room_number.to_string(), Arc::clone(&handler));
        handler
    }

    /// Attach an AI participant to a conference room.
    pub async fn attach_ai(&self, room_number: &str) -> Result<()> {
        let ai_conf = self.ai_config.read().clone();
        if !ai_conf.enabled {
            anyhow::bail!("AI participants are not enabled in configuration");
        }

        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        // Build AI session config from server config
        let mut session_config = AISessionConfig::default();
        if let Some(ref key) = ai_conf.api_key {
            session_config.api_key = forge_core::SecureString::new(key.clone());
        }
        session_config.model = ai_conf.model.clone();
        session_config.voice = Some(ai_conf.voice.clone());
        if let Some(ref instructions) = ai_conf.instructions {
            session_config.instructions = Some(instructions.clone());
        }
        if let Some(ref endpoint) = ai_conf.endpoint {
            session_config.endpoint = Some(endpoint.clone());
        }

        // Create ConferenceAIManager
        let call_id = forge_core::CallId::generate();
        let conference_ai_config =
            ConferenceAIConfig::new(forge_core::AudioFormat::pcm_mono());
        let ai_manager = ConferenceAIManager::new(
            room_number.to_string(),
            call_id.clone(),
            Arc::clone(&self.ai_session_manager),
            conference_ai_config,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create AI manager: {}", e))?;

        // Attach AI session to the session manager
        self.ai_session_manager
            .attach_ai(call_id, session_config, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to attach AI session: {}", e))?;

        // Attach AI participant to room
        room.attach_ai(ai_manager, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to attach AI to room: {}", e))?;

        info!(room = %room_number, "AI participant attached");
        Ok(())
    }

    /// Detach AI participant from a conference room.
    pub async fn detach_ai(&self, room_number: &str) -> Result<()> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        room.detach_ai()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to detach AI: {}", e))?;

        info!(room = %room_number, "AI participant detached");
        Ok(())
    }

    /// Get AI participant status for a room.
    pub fn ai_status(&self, room_number: &str) -> Result<serde_json::Value> {
        let room = self
            .bridge
            .get_room(room_number)
            .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

        let has_ai = room.has_ai();
        let state = room.ai_state().map(|s| format!("{:?}", s));
        let ai_conf = self.ai_config.read();

        Ok(serde_json::json!({
            "room_number": room_number,
            "ai_enabled": ai_conf.enabled,
            "ai_attached": has_ai,
            "ai_state": state,
            "model": ai_conf.model,
        }))
    }

    /// Kick a participant by their participant_id: send SIP BYE and clean up session.
    pub async fn kick_participant(&self, participant_id: &str) -> Result<()> {
        // Reverse lookup: find call_id by participant_id
        let call_id = self
            .sessions
            .iter()
            .find(|entry| entry.value().participant_id == participant_id)
            .map(|entry| entry.key().clone());

        let call_id = call_id
            .ok_or_else(|| anyhow::anyhow!("No session found for participant: {}", participant_id))?;

        // Send BYE before cleaning up
        if let Some(session) = self.sessions.get(&call_id) {
            if let Err(e) = self.uac.bye(&session.dialog).await {
                warn!(call_id = %call_id, error = %e, "Failed to send BYE for kicked participant");
            }
        }

        self.remove_session(&call_id);
        Ok(())
    }

    /// Graceful shutdown: drain all participant sessions and clean up resources.
    pub async fn shutdown(&self) {
        let session_count = self.sessions.len();
        info!(sessions = session_count, "Shutting down conference sessions");

        // Collect all call IDs
        let call_ids: Vec<String> = self
            .sessions
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        // Send BYE and remove each session
        for call_id in &call_ids {
            if let Some(session) = self.sessions.get(call_id) {
                if let Err(e) = self.uac.bye(&session.dialog).await {
                    warn!(call_id = %call_id, error = %e, "Failed to send BYE during shutdown");
                }
                drop(session); // Release DashMap ref before remove_session
            }
            self.remove_session(call_id);
        }

        // Stop recordings and delete all active rooms
        for room_id in self.bridge.list_rooms() {
            if let Ok(room) = self.bridge.get_room(&room_id) {
                if room.is_recording() {
                    let _ = room.stop_recording();
                }
            }
            let _ = self.bridge.delete_room(&room_id);
        }

        info!("Conference shutdown complete");
    }

    /// Hot-reload safe configuration settings (conference section, AI config).
    /// Called by the config watcher when the config file changes.
    pub fn reload_config(&self, conference: ConferenceSection, ai: AiSection) {
        info!(
            cache_ttl = conference.cache_ttl_secs,
            dtmf = conference.dtmf_enabled,
            ai_enabled = ai.enabled,
            "Config hot-reload applied"
        );
        *self.conference_config.write() = conference;
        *self.ai_config.write() = ai;
    }

    /// Get the recordings directory path (convenience accessor).
    pub fn recordings_dir(&self) -> String {
        self.conference_config.read().recordings_dir.clone()
    }

    /// Get the number of active SIP sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Publish a conference event to the event bus.
    fn publish_event(&self, event: ConferenceEvent) {
        let event_bus = Arc::clone(&self.event_bus);
        let platform_event = PlatformEvent::Conference(event);
        tokio::spawn(async move {
            if let Err(e) = event_bus.publish(platform_event).await {
                warn!(error = %e, "Failed to publish conference event");
            }
        });
    }

    /// Remove a participant by SIP call ID (called on BYE).
    fn remove_session(&self, call_id: &str) {
        if let Some((_, session)) = self.sessions.remove(call_id) {
            info!(
                call_id = %call_id,
                room = %session.room_number,
                participant = %session.participant_id,
                "Participant left conference"
            );

            // Abort the audio bridge task
            session.audio_bridge_handle.abort();

            // Deallocate the RTP port pair (async, fire-and-forget)
            let port_pool = Arc::clone(&self.port_pool);
            let port_pair = session.port_pair;
            tokio::spawn(async move {
                port_pool.deallocate(port_pair).await;
            });

            // Publish leave event
            self.publish_event(ConferenceEvent::ParticipantLeft {
                room_number: session.room_number.clone(),
                participant_id: session.participant_id.clone(),
                tenant_id: session.tenant_id,
                timestamp: SystemTime::now(),
            });

            // Remove from forge-conference room
            if let Ok(room) = self.bridge.get_room(&session.room_number) {
                let _ = room.remove_participant(&session.participant_id);

                // Publish MeetingEnded when last participant leaves
                if room.participant_count() == 0 {
                    self.publish_event(ConferenceEvent::MeetingEnded {
                        room_number: session.room_number.clone(),
                        tenant_id: session.tenant_id,
                        timestamp: SystemTime::now(),
                    });
                }

                // Auto-destroy empty rooms if configured
                if room.participant_count() == 0 && self.conference_config.read().auto_destroy_empty_rooms {
                    let room_number = session.room_number.clone();
                    let bridge = Arc::clone(&self.bridge);
                    let storage = Arc::clone(&self.storage);
                    let event_bus = Arc::clone(&self.event_bus);
                    let tenant_id = session.tenant_id;
                    tokio::spawn(async move {
                        use fcp_core::storage::ConferenceStore;
                        let should_destroy = match storage
                            .get_meeting_by_room_number(&TenantId::nil(), &room_number)
                            .await
                        {
                            Ok(Some(meeting)) => !meeting.permanent,
                            _ => true,
                        };
                        if should_destroy {
                            debug!(room = %room_number, "Destroying empty conference room");
                            let _ = bridge.delete_room(&room_number);

                            let event =
                                PlatformEvent::Conference(ConferenceEvent::RoomDestroyed {
                                    room_number,
                                    tenant_id,
                                    timestamp: SystemTime::now(),
                                });
                            let _ = event_bus.publish(event).await;
                        }
                    });
                }
            }
        } else {
            debug!(call_id = %call_id, "No conference session found for BYE");
        }
    }

    /// Build a forge-conference RoomConfig from a Meeting's settings.
    fn meeting_to_room_config(meeting: &Meeting) -> RoomConfig {
        RoomConfig {
            guest_pin: meeting.guest_pin.clone(),
            host_pin: meeting.host_pin.clone(),
            require_guest_pin: Some(meeting.require_guest_pin),
            max_pin_attempts: None,
            default_locked: Some(meeting.default_locked),
            enable_dtmf: None,
            dtmf_commands: None,
            max_channels: if meeting.max_participants > 0 { Some(meeting.max_participants) } else { None },
            wait_for_moderator: Some(meeting.wait_for_moderator),
            min_users: None,
            min_recording_participants: if meeting.auto_record { Some(1) } else { None },
            join_sound: None,
            exit_sound: None,
            alone_sound: None,
            invalid_pin_sound: None,
            locked_sound: None,
            unlocked_sound: None,
            kicked_sound: None,
            max_members_sound: None,
            moh_sound: None,
            pin_prompt_sound: None,
            recording_started_sound: None,
            recording_stopped_sound: None,
        }
    }

    /// Parse the local SIP URI configured for this handler.
    fn parse_local_sip_uri(&self) -> SipUri {
        Uri::parse(&self.local_uri)
            .ok()
            .and_then(|u| u.as_sip().cloned())
            .unwrap_or_else(|| {
                // "sip:conference@localhost" is a valid literal; this cannot fail
                SipUri::parse("sip:conference@localhost")
                    .expect("static SIP URI literal")
            })
    }

    /// Check whether PIN authentication is required for a meeting.
    fn meeting_requires_pin(meeting: &Meeting) -> bool {
        meeting.require_guest_pin
            && (meeting.guest_pin.is_some() || meeting.host_pin.is_some())
    }

    /// Negotiate SDES-SRTP from a parsed crypto line.
    ///
    /// Decodes the remote key material, generates a local key, and returns both
    /// the `SrtpContext` (for the audio bridge) and the `SrtpSdpContext` (for
    /// the SDP answer).
    fn negotiate_srtp(
        crypto: &fcp_media::SdesCryptoLine,
    ) -> Result<(SrtpContext, SrtpSdpContext)> {
        let (key, salt, profile) = decode_sdes_key_params(&crypto.key_params, &crypto.suite)?;
        let remote_key = SrtpKeyMaterial::new(key, salt, profile)
            .map_err(|e| anyhow::anyhow!("Invalid SRTP key material: {}", e))?;

        let (local_key, b64) = generate_srtp_key_material()?;

        let mut ctx = SrtpContext::new();
        ctx.set_remote_key(remote_key);
        ctx.set_local_key(local_key);

        let sdp_ctx = SrtpSdpContext {
            local_key_b64: format!("inline:{}", b64),
            suite: crypto.suite.clone(),
            tag: crypto.tag,
        };

        debug!(suite = %crypto.suite, tag = crypto.tag, "SRTP negotiated");
        Ok((ctx, sdp_ctx))
    }

    /// Collect a PIN via RFC 2833 DTMF from inbound RTP.
    ///
    /// Listens for telephone-event packets with the given `te_pt` payload type.
    /// Returns the collected digits when `#` is pressed or max length reached.
    /// Returns `None` on timeout.
    async fn collect_pin_dtmf(
        rtp_socket: &RtpSocketPair,
        te_pt: Option<u8>,
        timeout: Duration,
    ) -> Option<String> {
        let te_payload_type = te_pt.unwrap_or(101); // common default
        let mut detector = Rfc2833Detector::new(8000);
        let mut digits = String::new();
        let max_digits = 20;
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                debug!("PIN collection timed out after {} digits", digits.len());
                return if digits.is_empty() { None } else { Some(digits) };
            }

            match tokio::time::timeout(remaining, rtp_socket.recv_rtp()).await {
                Ok(Ok((packet, _addr))) => {
                    // Only process telephone-event payload type
                    if packet.header.payload_type() != te_payload_type {
                        continue;
                    }

                    if let Ok(events) =
                        detector.process_with_timestamp(&packet.payload[..], packet.header.timestamp)
                    {
                        for event in events {
                            if event.event_type == DtmfEventType::End {
                                if event.digit == DtmfDigit::Hash {
                                    // '#' terminates PIN entry
                                    debug!(digits = %digits, "PIN entry completed with #");
                                    return Some(digits);
                                }
                                if let Some(ch) = dtmf_digit_to_char(&event.digit) {
                                    digits.push(ch);
                                    debug!(digit = %ch, collected = %digits, "DTMF digit received");
                                    if digits.len() >= max_digits {
                                        return Some(digits);
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    debug!(error = %e, "RTP recv error during PIN collection");
                    return None;
                }
                Err(_) => {
                    // Timeout
                    debug!("PIN collection timed out");
                    return if digits.is_empty() { None } else { Some(digits) };
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl fcp_sip::conference::ConferenceHandler for ConferenceCallHandler {
    async fn can_handle(&self, request: &sip_core::Request) -> bool {
        let room_number = match extract_room_number(request) {
            Some(rn) => rn,
            None => return false,
        };

        let tenant_id = extract_tenant(request);
        match self.lookup_meeting(&tenant_id, &room_number).await {
            Ok(Some(meeting)) => meeting.enabled,
            _ => false,
        }
    }

    async fn handle_invite(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
        _peer: SocketAddr,
    ) -> Result<()> {
        // 1. Extract room number
        let room_number = extract_room_number(request)
            .context("Failed to extract room number from Request-URI")?;

        // 2. Extract tenant and look up meeting
        let tenant_id = extract_tenant(request);
        let meeting = self
            .lookup_meeting(&tenant_id, &room_number)
            .await?
            .context("Meeting not found")?;

        if !meeting.enabled {
            let reject = UserAgentServer::create_response(request, 403, "Conference Disabled");
            handle.send_final(reject).await;
            return Ok(());
        }

        // Snapshot config for use throughout this method (avoids holding lock across awaits)
        let conf = self.conference_config.read().clone();

        // 3. Extract SDP offer from INVITE body
        let body = request.body();
        if body.is_empty() {
            let reject = UserAgentServer::create_response(request, 400, "Missing SDP");
            handle.send_final(reject).await;
            return Ok(());
        }
        let sdp_offer = std::str::from_utf8(&body[..])
            .context("SDP body is not valid UTF-8")?;

        // 4. Parse codecs and pick first supported one (filtered by allowed_codecs config)
        let offered_codecs = parse_sdp_codecs(sdp_offer);
        let allowed = conf.allowed_codecs.as_deref();
        let codec_result = pick_codec(&offered_codecs, allowed);
        let (codec_name, payload_type, frame_size) = match codec_result {
            Some(c) => c,
            None => {
                let reject =
                    UserAgentServer::create_response(request, 488, "Not Acceptable Here");
                handle.send_final(reject).await;
                return Ok(());
            }
        };

        // Also look for telephone-event payload type for DTMF
        let te_pt = find_telephone_event_pt(&offered_codecs, sdp_offer);

        debug!(
            room = %room_number,
            codec = %codec_name,
            pt = payload_type,
            te_pt = ?te_pt,
            "Negotiated codec for conference participant"
        );

        // 5. Send 180 Ringing
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri.clone());
        let ringing = uas.create_ringing(request);
        handle.send_provisional(ringing).await;

        // 6. Allocate RTP port pair
        let port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port pair")?;

        // 7. Negotiate SRTP (if enabled and offered by client)
        let (srtp_context, srtp_sdp) = if conf.enable_srtp {
            let crypto_lines = parse_sdes_crypto(sdp_offer);
            if let Some(crypto) = crypto_lines.first() {
                match Self::negotiate_srtp(crypto) {
                    Ok((ctx, sdp_ctx)) => (Some(Arc::new(tokio::sync::RwLock::new(ctx))), Some(sdp_ctx)),
                    Err(e) => {
                        warn!(error = %e, "SRTP negotiation failed, falling back to plain RTP");
                        (None, None)
                    }
                }
            } else {
                (None, None) // Client didn't offer crypto
            }
        } else {
            (None, None)
        };

        // 7b. Construct SDP answer
        let sdp_answer = construct_answer_for_a_leg(
            sdp_offer,
            &self.local_addr,
            port_pair.rtp_port,
            srtp_sdp.as_ref(),
        );

        // 8. Create 200 OK with SDP answer using UAS
        let (response, dialog) = uas
            .accept_invite(request, Some(&sdp_answer))
            .context("Failed to create 200 OK")?;

        // 9. Send 200 OK
        handle.send_final(response).await;

        // 10. Create RTP socket pair
        let rtp_socket = RtpSocketPair::new(port_pair, RtpSocketConfig::default())
            .await
            .context("Failed to create RTP socket pair")?;
        let rtp_socket = Arc::new(rtp_socket);

        // 11. Parse remote RTP address from SDP offer
        if let Some(remote_addr) = parse_remote_rtp_addr(sdp_offer) {
            rtp_socket
                .set_remote_endpoint(RemoteEndpoint {
                    rtp_addr: remote_addr,
                    rtcp_addr: None,
                })
                .await;

            debug!(
                remote = %remote_addr,
                local_rtp_port = port_pair.rtp_port,
                "RTP endpoints configured"
            );
        } else {
            warn!("Could not parse remote RTP address from SDP offer");
        }

        // 12. Generate participant ID and extract caller info
        let call_id = extract_call_id(request);
        let (display_name, caller_id) = extract_caller_info(request);
        let short_uuid = uuid::Uuid::new_v4().to_string();
        let participant_id = format!(
            "{}-{}",
            &call_id[..call_id.len().min(8)],
            &short_uuid[..8]
        );

        // 13. Get or create conference room, configure with meeting settings
        let room = self
            .bridge
            .get_or_create_room(&room_number, None)
            .context("Failed to create conference room")?;

        // Wire room configuration from meeting → forge-conference
        let room_config = Self::meeting_to_room_config(&meeting);
        let global_config = ConferenceConfig::default();
        if let Err(e) = room.configure(room_config, &global_config) {
            warn!(room = %room_number, error = %e, "Failed to configure room (may already be configured)");
        }

        // 14. PIN authentication (if required)
        let is_host = if Self::meeting_requires_pin(&meeting) {
            let max_attempts = conf.max_pin_attempts;
            let pin_timeout = Duration::from_secs(conf.pin_timeout_secs);

            let mut authenticated = false;
            let mut is_host_auth = false;

            for attempt in 1..=max_attempts {
                debug!(
                    room = %room_number,
                    participant = %participant_id,
                    attempt = attempt,
                    "Waiting for PIN entry via DTMF"
                );

                match Self::collect_pin_dtmf(&rtp_socket, te_pt, pin_timeout).await {
                    Some(pin) => {
                        let result = room.authenticate_participant(&participant_id, &pin);
                        match result {
                            PinAuthResult::GuestAuthenticated => {
                                info!(room = %room_number, participant = %participant_id, "Guest PIN accepted");
                                authenticated = true;
                                is_host_auth = false;
                                break;
                            }
                            PinAuthResult::HostAuthenticated => {
                                info!(room = %room_number, participant = %participant_id, "Host PIN accepted");
                                authenticated = true;
                                is_host_auth = true;
                                break;
                            }
                            PinAuthResult::IncorrectPin { attempts_remaining } => {
                                warn!(
                                    room = %room_number,
                                    participant = %participant_id,
                                    attempts_remaining = attempts_remaining,
                                    "Incorrect PIN"
                                );
                            }
                            PinAuthResult::LockedOut { .. } => {
                                warn!(room = %room_number, participant = %participant_id, "PIN locked out");
                                self.publish_event(ConferenceEvent::PinFailed {
                                    room_number: room_number.clone(),
                                    participant_id: participant_id.clone(),
                                    tenant_id,
                                    timestamp: SystemTime::now(),
                                });
                                // Send BYE to disconnect the answered call
                                if let Err(e) = self.uac.bye(&dialog).await {
                                    warn!(error = %e, "Failed to send BYE after PIN lockout");
                                }
                                let pool = Arc::clone(&self.port_pool);
                                tokio::spawn(async move { pool.deallocate(port_pair).await });
                                return Ok(());
                            }
                            PinAuthResult::NoPinRequired => {
                                authenticated = true;
                                break;
                            }
                        }
                    }
                    None => {
                        // Timeout — no DTMF received
                        warn!(room = %room_number, participant = %participant_id, "PIN collection timed out");
                        break;
                    }
                }
            }

            if !authenticated {
                info!(room = %room_number, participant = %participant_id, "PIN authentication failed, rejecting");
                self.publish_event(ConferenceEvent::PinFailed {
                    room_number: room_number.clone(),
                    participant_id: participant_id.clone(),
                    tenant_id,
                    timestamp: SystemTime::now(),
                });
                // Send BYE to disconnect the answered call
                if let Err(e) = self.uac.bye(&dialog).await {
                    warn!(error = %e, "Failed to send BYE after PIN failure");
                }
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                return Ok(());
            }

            is_host_auth
        } else {
            false
        };

        // 15. Add participant to room (enforces capacity/locked/wait-for-moderator)
        match room.add_participant(&participant_id, is_host) {
            Ok(()) => {
                // Successfully added
            }
            Err(ConferenceError::ConferenceFull(max)) => {
                warn!(room = %room_number, max = max, "Conference is full, rejecting participant");
                if let Err(e) = self.uac.bye(&dialog).await {
                    warn!(error = %e, "Failed to send BYE after conference full");
                }
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                return Ok(());
            }
            Err(ConferenceError::ConferenceLocked) => {
                warn!(room = %room_number, "Conference is locked, rejecting participant");
                if let Err(e) = self.uac.bye(&dialog).await {
                    warn!(error = %e, "Failed to send BYE after conference locked");
                }
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                return Ok(());
            }
            Err(ConferenceError::WaitingForModerator) => {
                info!(
                    room = %room_number,
                    participant = %participant_id,
                    "Participant waiting for moderator"
                );
                // Participant is in waiting_participants — spawn a task that polls
                // until they appear in the mixer (when a host joins and releases them),
                // then start the audio bridge.
                let bridge_room = Arc::clone(&room);
                let bridge_participant = participant_id.clone();
                let bridge_socket = Arc::clone(&rtp_socket);
                let event_bus = Arc::clone(&self.event_bus);
                let rn = room_number.clone();
                let pid = participant_id.clone();
                let encoder = create_codec_instance(&codec_name)?;
                let decoder = create_codec_instance(&codec_name)?;
                let jitter_buffer_ms = conf.jitter_buffer_ms;
                let recordings_dir = conf.recordings_dir.clone();
                let dtmf_handler = if conf.dtmf_enabled {
                    Some(self.get_or_create_dtmf_handler(&room_number))
                } else {
                    None
                };
                let wait_stats = Arc::new(AudioStats::new());
                let wait_stats_clone = Arc::clone(&wait_stats);
                let wait_srtp = srtp_context.clone();

                let wait_handle = tokio::spawn(async move {
                    // Poll every 500ms waiting to be released into the mixer
                    loop {
                        tokio::time::sleep(Duration::from_millis(500)).await;

                        // Check if we're still in the waiting list
                        let still_waiting = bridge_room
                            .waiting_participants()
                            .contains(&bridge_participant);

                        if !still_waiting {
                            // We were either released into mixer or removed entirely.
                            // Check if we're now in the mixer.
                            if bridge_room.participants().contains(&bridge_participant) {
                                info!(
                                    room = %rn,
                                    participant = %pid,
                                    "Moderator arrived, participant released"
                                );

                                // Publish join event
                                let event = PlatformEvent::Conference(
                                    ConferenceEvent::ParticipantJoined {
                                        room_number: rn.clone(),
                                        participant_id: pid.clone(),
                                        is_host: false,
                                        tenant_id,
                                        timestamp: SystemTime::now(),
                                    },
                                );
                                let _ = event_bus.publish(event).await;

                                // Auto-record check after moderator releases waiting participants
                                if !bridge_room.is_recording() {
                                    if let Some(cfg) = bridge_room.get_effective_config() {
                                        if cfg.min_recording_participants > 0
                                            && bridge_room.participant_count()
                                                >= cfg.min_recording_participants
                                        {
                                            let ts = SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .unwrap_or_default()
                                                .as_secs();
                                            let rec_path = format!(
                                                "{}/{}-{}.wav",
                                                recordings_dir, rn, ts
                                            );
                                            match bridge_room
                                                .start_recording(&rec_path, None)
                                                .await
                                            {
                                                Ok(()) => {
                                                    tracing::info!(
                                                        room = %rn,
                                                        path = %rec_path,
                                                        "Auto-recording started (after moderator join)"
                                                    );
                                                    let rec_event = PlatformEvent::Conference(
                                                        ConferenceEvent::RecordingStarted {
                                                            room_number: rn.clone(),
                                                            tenant_id,
                                                            timestamp: SystemTime::now(),
                                                        },
                                                    );
                                                    let _ = event_bus.publish(rec_event).await;
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        room = %rn,
                                                        error = %e,
                                                        "Auto-record failed (after moderator join)"
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }

                                // Start audio bridge
                                audio_bridge::run_audio_bridge(
                                    bridge_room,
                                    pid,
                                    bridge_socket,
                                    encoder,
                                    decoder,
                                    payload_type,
                                    frame_size,
                                    jitter_buffer_ms,
                                    te_pt,
                                    dtmf_handler,
                                    false, // waiting participants are not hosts
                                    wait_stats_clone,
                                    wait_srtp.clone(),
                                )
                                .await;
                            }
                            break;
                        }
                    }
                });

                // Store session with the wait handle
                let remote_addr = parse_remote_rtp_addr(sdp_offer)
                    .unwrap_or(UNKNOWN_REMOTE);
                self.sessions.insert(
                    call_id.clone(),
                    ParticipantSession {
                        call_id,
                        room_number,
                        participant_id,
                        is_host,
                        audio_bridge_handle: wait_handle,
                        port_pair,
                        rtp_socket: Arc::clone(&rtp_socket),
                        remote_addr,
                        codec_name,
                        tenant_id,
                        dialog: dialog.clone(),
                        joined_at: SystemTime::now(),
                        te_pt,
                        stats: wait_stats,
                        display_name: display_name.clone(),
                        caller_id: caller_id.clone(),
                        srtp_context: srtp_context.clone(),
                    },
                );

                return Ok(());
            }
            Err(e) => {
                warn!(room = %room_number, error = %e, "Failed to add participant");
                if let Err(bye_err) = self.uac.bye(&dialog).await {
                    warn!(error = %bye_err, "Failed to send BYE after add_participant error");
                }
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                return Ok(());
            }
        }

        info!(
            call_id = %call_id,
            room = %room_number,
            participant = %participant_id,
            codec = %codec_name,
            is_host = is_host,
            "Participant joined conference"
        );

        // Publish MeetingStarted if this is the first participant
        if room.participant_count() == 1 {
            self.publish_event(ConferenceEvent::MeetingStarted {
                room_number: room_number.clone(),
                tenant_id,
                timestamp: SystemTime::now(),
            });
        }

        // Publish join event
        self.publish_event(ConferenceEvent::ParticipantJoined {
            room_number: room_number.clone(),
            participant_id: participant_id.clone(),
            is_host,
            tenant_id,
            timestamp: SystemTime::now(),
        });

        // Auto-record: start recording when participant threshold is reached
        if !room.is_recording() {
            if let Some(cfg) = room.get_effective_config() {
                if cfg.min_recording_participants > 0
                    && room.participant_count() >= cfg.min_recording_participants
                {
                    let timestamp = SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let path = format!(
                        "{}/{}-{}.wav",
                        conf.recordings_dir, room_number, timestamp
                    );
                    match room.start_recording(&path, None).await {
                        Ok(()) => {
                            info!(room = %room_number, path = %path, "Auto-recording started");
                            self.publish_event(ConferenceEvent::RecordingStarted {
                                room_number: room_number.clone(),
                                tenant_id,
                                timestamp: SystemTime::now(),
                            });
                        }
                        Err(e) => {
                            warn!(room = %room_number, error = %e, "Auto-record failed");
                        }
                    }
                }
            }
        }

        // 16. Create codec instances for the audio bridge
        let encoder = create_codec_instance(&codec_name)?;
        let decoder = create_codec_instance(&codec_name)?;

        // 17. Create DTMF handler and stats for audio bridge
        let dtmf_handler = if conf.dtmf_enabled {
            Some(self.get_or_create_dtmf_handler(&room_number))
        } else {
            None
        };
        let stats = Arc::new(AudioStats::new());

        // 18. Spawn audio bridge task
        let bridge_room = Arc::clone(&room);
        let bridge_participant = participant_id.clone();
        let bridge_socket = Arc::clone(&rtp_socket);
        let stats_clone = Arc::clone(&stats);
        let bridge_srtp = srtp_context.clone();
        let audio_bridge_handle = tokio::spawn(audio_bridge::run_audio_bridge(
            bridge_room,
            bridge_participant,
            bridge_socket,
            encoder,
            decoder,
            payload_type,
            frame_size,
            conf.jitter_buffer_ms,
            te_pt,
            dtmf_handler,
            is_host,
            stats_clone,
            bridge_srtp,
        ));

        // 19. Store session
        let remote_addr = parse_remote_rtp_addr(sdp_offer)
            .unwrap_or(UNKNOWN_REMOTE);
        self.sessions.insert(
            call_id.clone(),
            ParticipantSession {
                call_id,
                room_number,
                participant_id,
                is_host,
                audio_bridge_handle,
                port_pair,
                rtp_socket,
                remote_addr,
                codec_name,
                tenant_id,
                dialog,
                joined_at: SystemTime::now(),
                te_pt,
                stats,
                display_name,
                caller_id,
                srtp_context,
            },
        );

        Ok(())
    }

    fn handle_reinvite(&self, request: &sip_core::Request) -> Result<sip_core::Response> {
        let call_id = extract_call_id(request);
        let conf = self.conference_config.read().clone();

        // Look up session — if not found, return Err so caller falls through to B2BUA
        let mut session = self
            .sessions
            .get_mut(&call_id)
            .ok_or_else(|| anyhow::anyhow!("No conference session for call_id: {}", call_id))?;

        // Parse new SDP body
        let body = request.body();
        let sdp_offer = std::str::from_utf8(&body[..])
            .context("re-INVITE SDP body is not valid UTF-8")?;

        // Check for remote address change
        if let Some(new_remote) = parse_remote_rtp_addr(sdp_offer) {
            if new_remote != session.remote_addr {
                info!(
                    call_id = %call_id,
                    old = %session.remote_addr,
                    new = %new_remote,
                    "re-INVITE: remote RTP address changed"
                );
                session.remote_addr = new_remote;
                let rtp_socket = Arc::clone(&session.rtp_socket);
                tokio::spawn(async move {
                    rtp_socket
                        .set_remote_endpoint(RemoteEndpoint {
                            rtp_addr: new_remote,
                            rtcp_addr: None,
                        })
                        .await;
                });
            }
        }

        // Check for codec change
        let offered_codecs = parse_sdp_codecs(sdp_offer);
        let allowed = conf.allowed_codecs.as_deref();
        if let Some((new_codec, new_pt, new_frame_size)) = pick_codec(&offered_codecs, allowed) {
            if new_codec != session.codec_name {
                info!(
                    call_id = %call_id,
                    old_codec = %session.codec_name,
                    new_codec = %new_codec,
                    "re-INVITE: codec changed, restarting audio bridge"
                );

                // Abort old audio bridge
                session.audio_bridge_handle.abort();

                // Create new codec instances
                let encoder = create_codec_instance(&new_codec)?;
                let decoder = create_codec_instance(&new_codec)?;

                // Get room reference
                let room = self
                    .bridge
                    .get_room(&session.room_number)
                    .map_err(|e| anyhow::anyhow!("Room not found: {}", e))?;

                // Spawn new audio bridge (reuse existing DTMF handler and stats)
                let bridge_room = Arc::clone(&room);
                let bridge_participant = session.participant_id.clone();
                let bridge_socket = Arc::clone(&session.rtp_socket);
                let jitter_buffer_ms = conf.jitter_buffer_ms;
                let reinvite_dtmf = if conf.dtmf_enabled {
                    Some(self.get_or_create_dtmf_handler(&session.room_number))
                } else {
                    None
                };
                let reinvite_stats = Arc::clone(&session.stats);
                let reinvite_is_host = session.is_host;
                let reinvite_te_pt = session.te_pt;
                let reinvite_srtp = session.srtp_context.clone();
                let new_handle = tokio::spawn(audio_bridge::run_audio_bridge(
                    bridge_room,
                    bridge_participant,
                    bridge_socket,
                    encoder,
                    decoder,
                    new_pt,
                    new_frame_size,
                    jitter_buffer_ms,
                    reinvite_te_pt,
                    reinvite_dtmf,
                    reinvite_is_host,
                    reinvite_stats,
                    reinvite_srtp,
                ));

                session.audio_bridge_handle = new_handle;
                session.codec_name = new_codec;
            }
        }

        // Construct SDP answer with current local RTP port
        let sdp_answer = construct_answer_for_a_leg(
            sdp_offer,
            &self.local_addr,
            session.port_pair.rtp_port,
            None,
        );

        // Build 200 OK response with SDP answer
        let local_sip_uri = self.parse_local_sip_uri();
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri);
        let response = uas
            .create_ok(request, Some(&sdp_answer))
            .map_err(|e| anyhow::anyhow!("Failed to create 200 OK for re-INVITE: {}", e))?;

        Ok(response)
    }

    fn handle_bye(&self, call_id: &str) -> Result<()> {
        if self.sessions.contains_key(call_id) {
            self.remove_session(call_id);
            Ok(())
        } else {
            anyhow::bail!("No conference session for call_id: {}", call_id)
        }
    }
}

/// Extract the room number from a SIP Request-URI user part.
pub fn extract_room_number(request: &sip_core::Request) -> Option<String> {
    let uri = request.uri();
    let uri_str = uri.to_string();

    // Extract user part from sip:user@host
    if let Some(rest) = uri_str.strip_prefix("sip:") {
        if let Some(user) = rest.split('@').next() {
            if !user.is_empty() {
                return Some(user.to_string());
            }
        }
    }

    None
}

/// Extract tenant ID from the SIP request.
fn extract_tenant(request: &sip_core::Request) -> TenantId {
    for header in request.headers() {
        if header.name().eq_ignore_ascii_case("X-Tenant-ID") {
            if let Ok(tid) = header.value().parse::<TenantId>() {
                return tid;
            }
        }
    }
    TenantId::nil()
}

/// Extract Call-ID from a SIP request.
fn extract_call_id(request: &sip_core::Request) -> String {
    request
        .headers()
        .get("Call-ID")
        .map(|v| v.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Extract display name and caller ID from SIP request headers.
///
/// Checks P-Asserted-Identity first (trusted network identity), then falls back
/// to the From header. Returns (display_name, caller_id).
fn extract_caller_info(request: &sip_core::Request) -> (Option<String>, Option<String>) {
    // Try P-Asserted-Identity first (more trusted on SBC/trunk calls)
    if let Ok(Some(pai)) = sip_core::parse_p_asserted_identity(request.headers()) {
        if let Some(identity) = pai.identities().next() {
            let display = identity.display_name().map(|s| s.to_string());
            let caller = identity.uri().as_sip().and_then(|u| u.user().map(|s| s.to_string()));
            if display.is_some() || caller.is_some() {
                return (display, caller);
            }
        }
    }

    // Fall back to From header
    if let Some(from_val) = request.headers().get_smol("From") {
        if let Some(from) = sip_parse::parse_from_header(from_val) {
            let name_addr = from.inner();
            let display = name_addr.display_name().map(|s| s.to_string());
            let caller = name_addr.sip_uri().and_then(|u| u.user().map(|s| s.to_string()));
            return (display, caller);
        }
    }

    (None, None)
}

/// Parse remote RTP endpoint from an SDP offer.
///
/// Extracts the IP from the `c=IN IP4 <addr>` line and the port from
/// the `m=audio <port>` line, combining them into a SocketAddr.
fn parse_remote_rtp_addr(sdp: &str) -> Option<SocketAddr> {
    let mut ip: Option<&str> = None;
    let mut port: Option<u16> = None;

    for line in sdp.lines() {
        let line = line.trim();
        if line.starts_with("c=IN IP4 ") || line.starts_with("c=IN ip4 ") {
            // c=IN IP4 192.168.1.100
            ip = line.split_whitespace().nth(2);
        } else if line.starts_with("c=IN IP6 ") || line.starts_with("c=IN ip6 ") {
            ip = line.split_whitespace().nth(2);
        } else if line.starts_with("m=audio ") {
            // m=audio 10000 RTP/AVP 0 8 9
            if let Some(port_str) = line.split_whitespace().nth(1) {
                port = port_str.parse().ok();
            }
        }
    }

    match (ip, port) {
        (Some(ip_str), Some(p)) => format!("{}:{}", ip_str, p).parse().ok(),
        _ => None,
    }
}

/// Pick the first supported codec from an SDP offer.
///
/// If `allowed` is `Some`, only codecs in that list are considered.
/// Returns (codec_name, payload_type, frame_size_in_samples).
fn pick_codec(
    codecs: &[fcp_media::SdpCodecInfo],
    allowed: Option<&[String]>,
) -> Option<(String, u8, usize)> {
    for codec in codecs {
        let name_upper = codec.encoding_name.to_uppercase();
        // Check against allowed list if configured
        if let Some(allowed_list) = allowed {
            if !allowed_list.iter().any(|a| a.eq_ignore_ascii_case(&name_upper)) {
                continue;
            }
        }
        match name_upper.as_str() {
            "PCMU" => return Some(("PCMU".to_string(), codec.payload_type, 160)),
            "PCMA" => return Some(("PCMA".to_string(), codec.payload_type, 160)),
            "G722" => return Some(("G722".to_string(), codec.payload_type, 320)),
            _ => continue,
        }
    }
    None
}

/// Find telephone-event payload type from the SDP.
///
/// Looks for `a=rtpmap:<pt> telephone-event/8000` in the SDP text, or
/// checks parsed codecs for a "telephone-event" encoding name.
fn find_telephone_event_pt(codecs: &[fcp_media::SdpCodecInfo], sdp: &str) -> Option<u8> {
    // Check parsed codecs first
    for codec in codecs {
        if codec.encoding_name.eq_ignore_ascii_case("telephone-event") {
            return Some(codec.payload_type);
        }
    }

    // Fallback: parse SDP lines manually
    for line in sdp.lines() {
        let line = line.trim();
        if line.starts_with("a=rtpmap:") && line.contains("telephone-event") {
            // a=rtpmap:101 telephone-event/8000
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(pt_str) = parts.first() {
                if let Some(pt_num) = pt_str.strip_prefix("a=rtpmap:") {
                    if let Ok(pt) = pt_num.parse::<u8>() {
                        return Some(pt);
                    }
                }
            }
        }
    }

    None
}

/// Convert a DtmfDigit to its character representation.
fn dtmf_digit_to_char(digit: &DtmfDigit) -> Option<char> {
    match digit {
        DtmfDigit::Zero => Some('0'),
        DtmfDigit::One => Some('1'),
        DtmfDigit::Two => Some('2'),
        DtmfDigit::Three => Some('3'),
        DtmfDigit::Four => Some('4'),
        DtmfDigit::Five => Some('5'),
        DtmfDigit::Six => Some('6'),
        DtmfDigit::Seven => Some('7'),
        DtmfDigit::Eight => Some('8'),
        DtmfDigit::Nine => Some('9'),
        DtmfDigit::Star => Some('*'),
        DtmfDigit::Hash => None, // '#' used as terminator
        DtmfDigit::A => Some('A'),
        DtmfDigit::B => Some('B'),
        DtmfDigit::C => Some('C'),
        DtmfDigit::D => Some('D'),
    }
}

/// Create a codec instance by name for audio encoding/decoding.
fn create_codec_instance(codec_name: &str) -> Result<Box<dyn AudioCodec>> {
    match codec_name {
        "PCMU" => Ok(Box::new(g711::G711MuLaw::new(8000))),
        "PCMA" => Ok(Box::new(g711::G711ALaw::new(8000))),
        "G722" => Ok(Box::new(g722::G722Codec::new(g722::G722BitRate::Rate64k))),
        other => anyhow::bail!("Unsupported codec: {}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_room_number() {
        let uri = "sip:8001@192.168.1.100";
        let user = uri
            .strip_prefix("sip:")
            .and_then(|rest| rest.split('@').next())
            .filter(|u| !u.is_empty());
        assert_eq!(user, Some("8001"));
    }

    #[test]
    fn test_parse_remote_rtp_addr() {
        let sdp = "v=0\r\no=- 1234 1234 IN IP4 192.168.1.50\r\nc=IN IP4 192.168.1.50\r\nm=audio 10000 RTP/AVP 0 8\r\n";
        let addr = parse_remote_rtp_addr(sdp).unwrap();
        assert_eq!(addr, "192.168.1.50:10000".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn test_parse_remote_rtp_addr_missing() {
        let sdp = "v=0\r\no=- 1234 1234 IN IP4 0.0.0.0\r\n";
        assert!(parse_remote_rtp_addr(sdp).is_none());
    }

    #[test]
    fn test_pick_codec_pcmu() {
        let codecs = vec![
            fcp_media::SdpCodecInfo {
                payload_type: 0,
                encoding_name: "PCMU".to_string(),
                clock_rate: 8000,
            },
            fcp_media::SdpCodecInfo {
                payload_type: 8,
                encoding_name: "PCMA".to_string(),
                clock_rate: 8000,
            },
        ];
        let (name, pt, frame_size) = pick_codec(&codecs, None).unwrap();
        assert_eq!(name, "PCMU");
        assert_eq!(pt, 0);
        assert_eq!(frame_size, 160);
    }

    #[test]
    fn test_pick_codec_g722() {
        let codecs = vec![fcp_media::SdpCodecInfo {
            payload_type: 9,
            encoding_name: "G722".to_string(),
            clock_rate: 8000,
        }];
        let (name, pt, frame_size) = pick_codec(&codecs, None).unwrap();
        assert_eq!(name, "G722");
        assert_eq!(pt, 9);
        assert_eq!(frame_size, 320);
    }

    #[test]
    fn test_pick_codec_none() {
        let codecs = vec![fcp_media::SdpCodecInfo {
            payload_type: 96,
            encoding_name: "opus".to_string(),
            clock_rate: 48000,
        }];
        assert!(pick_codec(&codecs, None).is_none());
    }

    #[test]
    fn test_pick_codec_allowed_filter() {
        let codecs = vec![
            fcp_media::SdpCodecInfo {
                payload_type: 0,
                encoding_name: "PCMU".to_string(),
                clock_rate: 8000,
            },
            fcp_media::SdpCodecInfo {
                payload_type: 8,
                encoding_name: "PCMA".to_string(),
                clock_rate: 8000,
            },
            fcp_media::SdpCodecInfo {
                payload_type: 9,
                encoding_name: "G722".to_string(),
                clock_rate: 8000,
            },
        ];
        // Only allow PCMA
        let allowed = vec!["PCMA".to_string()];
        let (name, pt, _) = pick_codec(&codecs, Some(&allowed)).unwrap();
        assert_eq!(name, "PCMA");
        assert_eq!(pt, 8);

        // Allow none of the offered codecs
        let allowed = vec!["G729".to_string()];
        assert!(pick_codec(&codecs, Some(&allowed)).is_none());
    }

    #[test]
    fn test_find_telephone_event_pt() {
        let sdp = "v=0\r\nm=audio 10000 RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\n";
        let codecs = vec![fcp_media::SdpCodecInfo {
            payload_type: 0,
            encoding_name: "PCMU".to_string(),
            clock_rate: 8000,
        }];
        assert_eq!(find_telephone_event_pt(&codecs, sdp), Some(101));
    }

    #[test]
    fn test_dtmf_digit_to_char() {
        assert_eq!(dtmf_digit_to_char(&DtmfDigit::Zero), Some('0'));
        assert_eq!(dtmf_digit_to_char(&DtmfDigit::Nine), Some('9'));
        assert_eq!(dtmf_digit_to_char(&DtmfDigit::Star), Some('*'));
        assert_eq!(dtmf_digit_to_char(&DtmfDigit::Hash), None);
    }
}
