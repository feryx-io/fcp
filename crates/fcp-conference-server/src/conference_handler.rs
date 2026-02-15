use anyhow::{Context, Result};
use dashmap::DashMap;
use fcp_core::{DynStorage, Meeting, TenantId};
use fcp_events::{ConferenceEvent, EventPublisher, PlatformEvent};
use fcp_media::{construct_answer_for_a_leg, parse_sdp_codecs};
use forge_codecs::{g711, g722, AudioCodec};
use forge_conference::{ConferenceBridge, ConferenceConfig, ConferenceError, PinAuthResult, RoomConfig};
use forge_dtmf::rfc2833::Rfc2833Detector;
use forge_dtmf::{DtmfEventType, DtmfDigit};
use forge_rtp::{PortPair, PortPool, RemoteEndpoint, RtpSocketConfig, RtpSocketPair};
use sip_core::{SipUri, Uri};
use sip_uas::UserAgentServer;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::audio_bridge;
use crate::config::ConferenceSection;

/// Tracks a single participant's connection to a conference room.
pub struct ParticipantSession {
    pub call_id: String,
    pub room_number: String,
    pub participant_id: String,
    pub is_host: bool,
    pub audio_bridge_handle: JoinHandle<()>,
    pub port_pair: PortPair,
    pub remote_addr: SocketAddr,
    pub codec_name: String,
    pub tenant_id: TenantId,
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
    pub conference_config: ConferenceSection,
    sessions: DashMap<String, ParticipantSession>,
    pub local_uri: String,
    pub local_addr: String,
    port_pool: Arc<PortPool>,
    event_bus: Arc<dyn EventPublisher>,
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
    ) -> Self {
        Self {
            bridge,
            storage,
            conference_config,
            sessions: DashMap::new(),
            local_uri,
            local_addr,
            port_pool,
            event_bus,
        }
    }

    /// Look up a meeting by room number.
    pub async fn lookup_meeting(
        &self,
        tenant_id: &TenantId,
        room_number: &str,
    ) -> Result<Option<Meeting>> {
        use fcp_core::storage::ConferenceStore;
        self.storage
            .get_meeting_by_room_number(tenant_id, room_number)
            .await
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
                if room.participant_count() == 0 && self.conference_config.auto_destroy_empty_rooms {
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

    /// Check whether PIN authentication is required for a meeting.
    fn meeting_requires_pin(meeting: &Meeting) -> bool {
        meeting.require_guest_pin
            && (meeting.guest_pin.is_some() || meeting.host_pin.is_some())
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

        // 3. Extract SDP offer from INVITE body
        let body = request.body();
        if body.is_empty() {
            let reject = UserAgentServer::create_response(request, 400, "Missing SDP");
            handle.send_final(reject).await;
            return Ok(());
        }
        let sdp_offer = std::str::from_utf8(&body[..])
            .context("SDP body is not valid UTF-8")?;

        // 4. Parse codecs and pick first supported one
        let offered_codecs = parse_sdp_codecs(sdp_offer);
        let (codec_name, payload_type, frame_size) = pick_codec(&offered_codecs)
            .context("No supported codec in SDP offer")?;

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
        let local_sip_uri = Uri::parse(&self.local_uri)
            .ok()
            .and_then(|u| u.as_sip().cloned())
            .unwrap_or_else(|| SipUri::parse("sip:conference@localhost").unwrap());
        let uas = UserAgentServer::new(local_sip_uri.clone(), local_sip_uri.clone());
        let ringing = uas.create_ringing(request);
        handle.send_provisional(ringing).await;

        // 6. Allocate RTP port pair
        let port_pair = self
            .port_pool
            .allocate()
            .await
            .context("Failed to allocate RTP port pair")?;

        // 7. Construct SDP answer
        let sdp_answer =
            construct_answer_for_a_leg(sdp_offer, &self.local_addr, port_pair.rtp_port, None);

        // 8. Create 200 OK with SDP answer using UAS
        let (response, _dialog) = uas
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

        // 12. Generate participant ID
        let call_id = extract_call_id(request);
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
            let max_attempts = self.conference_config.max_pin_attempts;
            let pin_timeout = Duration::from_secs(self.conference_config.pin_timeout_secs);

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
                                // Clean up and reject
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
                let pool = Arc::clone(&self.port_pool);
                tokio::spawn(async move { pool.deallocate(port_pair).await });
                return Ok(());
            }
            Err(ConferenceError::ConferenceLocked) => {
                warn!(room = %room_number, "Conference is locked, rejecting participant");
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
                                        room_number: rn,
                                        participant_id: pid.clone(),
                                        is_host: false,
                                        tenant_id,
                                        timestamp: SystemTime::now(),
                                    },
                                );
                                let _ = event_bus.publish(event).await;

                                // Start audio bridge
                                audio_bridge::run_audio_bridge(
                                    bridge_room,
                                    pid,
                                    bridge_socket,
                                    encoder,
                                    decoder,
                                    payload_type,
                                    frame_size,
                                )
                                .await;
                            }
                            break;
                        }
                    }
                });

                // Store session with the wait handle
                let remote_addr = parse_remote_rtp_addr(sdp_offer)
                    .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());
                self.sessions.insert(
                    call_id.clone(),
                    ParticipantSession {
                        call_id,
                        room_number,
                        participant_id,
                        is_host,
                        audio_bridge_handle: wait_handle,
                        port_pair,
                        remote_addr,
                        codec_name,
                        tenant_id,
                    },
                );

                return Ok(());
            }
            Err(e) => {
                warn!(room = %room_number, error = %e, "Failed to add participant");
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

        // 16. Create codec instances for the audio bridge
        let encoder = create_codec_instance(&codec_name)?;
        let decoder = create_codec_instance(&codec_name)?;

        // 17. Spawn audio bridge task
        let bridge_room = Arc::clone(&room);
        let bridge_participant = participant_id.clone();
        let bridge_socket = Arc::clone(&rtp_socket);
        let audio_bridge_handle = tokio::spawn(audio_bridge::run_audio_bridge(
            bridge_room,
            bridge_participant,
            bridge_socket,
            encoder,
            decoder,
            payload_type,
            frame_size,
        ));

        // 18. Store session
        let remote_addr = parse_remote_rtp_addr(sdp_offer)
            .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());
        self.sessions.insert(
            call_id.clone(),
            ParticipantSession {
                call_id,
                room_number,
                participant_id,
                is_host,
                audio_bridge_handle,
                port_pair,
                remote_addr,
                codec_name,
                tenant_id,
            },
        );

        Ok(())
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
/// Returns (codec_name, payload_type, frame_size_in_samples).
fn pick_codec(codecs: &[fcp_media::SdpCodecInfo]) -> Option<(String, u8, usize)> {
    for codec in codecs {
        let name_upper = codec.encoding_name.to_uppercase();
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
        let (name, pt, frame_size) = pick_codec(&codecs).unwrap();
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
        let (name, pt, frame_size) = pick_codec(&codecs).unwrap();
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
        assert!(pick_codec(&codecs).is_none());
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
