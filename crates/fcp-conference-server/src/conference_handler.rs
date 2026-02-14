use anyhow::{Context, Result};
use dashmap::DashMap;
use fcp_core::{DynStorage, Meeting, TenantId};
use fcp_media::{construct_answer_for_a_leg, parse_sdp_codecs};
use forge_codecs::{g711, g722, AudioCodec};
use forge_conference::ConferenceBridge;
use forge_rtp::{PortPair, PortPool, RemoteEndpoint, RtpSocketConfig, RtpSocketPair};
use sip_core::{SipUri, Uri};
use sip_uas::UserAgentServer;
use std::net::SocketAddr;
use std::sync::Arc;
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
}

/// Handles SIP INVITEs for conference rooms.
///
/// When a SIP INVITE arrives targeting a conference room number:
/// 1. Looks up the meeting config from storage
/// 2. Answers the call and sets up RTP
/// 3. Optionally collects a PIN for authentication
/// 4. Creates/joins the forge-conference room
/// 5. Spawns a per-participant audio bridge task
pub struct ConferenceCallHandler {
    pub bridge: Arc<ConferenceBridge>,
    pub storage: Arc<DynStorage>,
    pub conference_config: ConferenceSection,
    sessions: DashMap<String, ParticipantSession>,
    pub local_uri: String,
    pub local_addr: String,
    port_pool: Arc<PortPool>,
}

impl ConferenceCallHandler {
    pub fn new(
        bridge: Arc<ConferenceBridge>,
        storage: Arc<DynStorage>,
        conference_config: ConferenceSection,
        local_uri: String,
        local_addr: String,
        port_pool: Arc<PortPool>,
    ) -> Self {
        Self {
            bridge,
            storage,
            conference_config,
            sessions: DashMap::new(),
            local_uri,
            local_addr,
            port_pool,
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

            // Remove from forge-conference room
            if let Ok(room) = self.bridge.get_room(&session.room_number) {
                let _ = room.remove_participant(&session.participant_id);

                // Auto-destroy empty rooms if configured
                if room.participant_count() == 0 && self.conference_config.auto_destroy_empty_rooms {
                    let room_number = session.room_number.clone();
                    let bridge = Arc::clone(&self.bridge);
                    let storage = Arc::clone(&self.storage);
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
                        }
                    });
                }
            }
        } else {
            debug!(call_id = %call_id, "No conference session found for BYE");
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

        debug!(
            room = %room_number,
            codec = %codec_name,
            pt = payload_type,
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

        // 12. Get or create conference room
        let room = self
            .bridge
            .get_or_create_room(&room_number, None)
            .context("Failed to create conference room")?;

        // 13. Generate participant ID
        let call_id = extract_call_id(request);
        let short_uuid = uuid::Uuid::new_v4().to_string();
        let participant_id = format!(
            "{}-{}",
            &call_id[..call_id.len().min(8)],
            &short_uuid[..8]
        );

        // 14. Add participant to room
        room.add_participant(&participant_id, false)
            .context("Failed to add participant to conference room")?;

        info!(
            call_id = %call_id,
            room = %room_number,
            participant = %participant_id,
            codec = %codec_name,
            "Participant joined conference"
        );

        // 15. Create codec instances for the audio bridge
        let encoder = create_codec_instance(&codec_name)?;
        let decoder = create_codec_instance(&codec_name)?;

        // 16. Spawn audio bridge task
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

        // 17. Store session
        let remote_addr = parse_remote_rtp_addr(sdp_offer)
            .unwrap_or_else(|| "0.0.0.0:0".parse().unwrap());
        self.sessions.insert(
            call_id.clone(),
            ParticipantSession {
                call_id,
                room_number,
                participant_id,
                is_host: false,
                audio_bridge_handle,
                port_pair,
                remote_addr,
                codec_name,
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
}
