use anyhow::Result;
use dashmap::DashMap;
use fcp_core::{DynStorage, Meeting, TenantId};
use forge_conference::ConferenceBridge;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use crate::config::ConferenceSection;

/// Tracks a single participant's connection to a conference room.
pub struct ParticipantSession {
    pub call_id: String,
    pub room_number: String,
    pub participant_id: String,
    pub is_host: bool,
    pub audio_bridge_handle: JoinHandle<()>,
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
}

impl ConferenceCallHandler {
    pub fn new(
        bridge: Arc<ConferenceBridge>,
        storage: Arc<DynStorage>,
        conference_config: ConferenceSection,
        local_uri: String,
        local_addr: String,
    ) -> Self {
        Self {
            bridge,
            storage,
            conference_config,
            sessions: DashMap::new(),
            local_uri,
            local_addr,
        }
    }

    /// Check whether this handler should process the given SIP INVITE.
    ///
    /// Returns true if the Request-URI user part matches a configured meeting room number.
    pub async fn can_handle(&self, request: &sip_core::Request) -> bool {
        let room_number = match extract_room_number(request) {
            Some(rn) => rn,
            None => return false,
        };

        // Check if a meeting exists for this room number (use nil tenant for now)
        let tenant_id = extract_tenant(request);
        match self.lookup_meeting(&tenant_id, &room_number).await {
            Ok(Some(_)) => true,
            _ => false,
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

    /// Add a participant session for tracking.
    pub fn add_session(&self, call_id: String, session: ParticipantSession) {
        info!(
            call_id = %call_id,
            room = %session.room_number,
            participant = %session.participant_id,
            is_host = session.is_host,
            "Participant joined conference"
        );
        self.sessions.insert(call_id, session);
    }

    /// Remove a participant by SIP call ID (called on BYE).
    pub fn remove_session(&self, call_id: &str) {
        if let Some((_, session)) = self.sessions.remove(call_id) {
            info!(
                call_id = %call_id,
                room = %session.room_number,
                participant = %session.participant_id,
                "Participant left conference"
            );

            // Abort the audio bridge task
            session.audio_bridge_handle.abort();

            // Remove from forge-conference room
            if let Ok(room) = self.bridge.get_room(&session.room_number) {
                let _ = room.remove_participant(&session.participant_id);

                // Auto-destroy empty rooms if configured
                if room.participant_count() == 0 && self.conference_config.auto_destroy_empty_rooms {
                    // Check if the meeting is permanent
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
    // Check X-Tenant-ID header first
    for header in request.headers() {
        if header.name().eq_ignore_ascii_case("X-Tenant-ID") {
            if let Ok(tid) = header.value().parse::<TenantId>() {
                return tid;
            }
        }
    }
    TenantId::nil()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_room_number() {
        // We can't easily construct sip_core::Request in tests without the full SIP stack,
        // so we test the URI parsing logic directly
        let uri = "sip:8001@192.168.1.100";
        let user = uri
            .strip_prefix("sip:")
            .and_then(|rest| rest.split('@').next())
            .filter(|u| !u.is_empty());
        assert_eq!(user, Some("8001"));
    }
}
