//! The audience's voice in a room (`docs/WEBINARS.md` §2.5, W3): a raised
//! hand, and a host bringing an attendee to the stage and sending them
//! back.
//!
//! A hand is raised or lowered by its owner — `*4` on a phone, an `fcp-qa`
//! INFO from a client, or their app through the admin — and lowered by a
//! host, one or all, or by being brought to the stage. Whose hand is up is
//! kept on the participant's session, so it goes when they do. The stage's
//! rosters show the audience's raised hands; the owner's shows their own.
//!
//! Bringing a webinar's attendee to the stage makes them heard: the room
//! takes them out of its listeners (`set_listener`), so they hear N−1 and
//! are mixed from the next frame, and the video room stops counting them as
//! audience. Sending them back makes them a listener again. A participant
//! whose INVITE said `Recv-Info: fcp-qa` is told both, and told when a host
//! lowers their hand, by an `fcp-qa` INFO.

use super::{ConferenceCallHandler, ConferenceRole, RoomKey};
use fcp_core::qa::QaMessage;
use fcp_events::ConferenceEvent;
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, info, warn};

/// Why a hand or the stage could not be moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageError {
    /// Nobody of that id (or call) in the room.
    NotFound,
    /// The participant cannot do that: a host's hand, the stage for
    /// someone who is not a webinar's attendee.
    Refused(&'static str),
}

impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StageError::NotFound => f.write_str("no such participant in the room"),
            StageError::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for StageError {}

/// Who moved a hand: what decides whether its owner is told and how the
/// event reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandBy {
    /// Its owner, by an `fcp-qa` INFO: they know, nobody tells them.
    OwnerInfo,
    /// Its owner by `*4` or their app: their client is told.
    Owner,
    /// A host.
    Host,
    /// Its owner being brought to the stage: the stage's own message says
    /// the hand came down.
    Stage,
}

impl ConferenceCallHandler {
    /// The call of `participant_id` in `key`'s room.
    fn call_of_participant(&self, key: &RoomKey, participant_id: &str) -> Option<String> {
        let calls = self.room_sessions.get(key)?.clone();
        calls.into_iter().find(|c| {
            self.sessions
                .get(c)
                .is_some_and(|s| s.participant_id == participant_id && s.trunk_peer.is_none())
        })
    }

    /// The participant on `call_id` (the call manager's or the gateway's
    /// `X-FCP-Call-Id`, or the leg's own SIP Call-ID) in `key`'s room: how
    /// an app names itself to the admin.
    pub fn participant_by_call(&self, key: &RoomKey, call_id: &str) -> Option<String> {
        let calls = self.room_sessions.get(key)?.clone();
        calls.into_iter().find_map(|c| {
            let s = self.sessions.get(&c)?;
            (s.trunk_peer.is_none() && (c == call_id || s.record.call_id.to_string() == call_id))
                .then(|| s.participant_id.clone())
        })
    }

    /// Raise or lower `participant_id`'s hand. Whether it changed.
    pub fn set_hand(
        &self,
        key: &RoomKey,
        participant_id: &str,
        raised: bool,
        by: HandBy,
    ) -> Result<bool, StageError> {
        let call_id = self
            .call_of_participant(key, participant_id)
            .ok_or(StageError::NotFound)?;
        let (display_name, tell) = {
            let mut s = self
                .sessions
                .get_mut(&call_id)
                .ok_or(StageError::NotFound)?;
            if raised && s.is_host {
                return Err(StageError::Refused("a host has no hand to raise"));
            }
            if s.hand.is_some() == raised {
                return Ok(false);
            }
            s.hand = raised.then(SystemTime::now);
            let tell = s.qa_notices && matches!(by, HandBy::Owner | HandBy::Host);
            (s.display_name.clone(), tell)
        };
        info!(
            room = %key.room,
            participant = %participant_id,
            raised,
            by = ?by,
            "Hand {}",
            if raised { "raised" } else { "lowered" }
        );
        let timestamp = SystemTime::now();
        self.publish_event(if raised {
            ConferenceEvent::HandRaised {
                room_number: key.room.clone(),
                participant_id: participant_id.to_string(),
                display_name,
                tenant_id: key.tenant,
                timestamp,
            }
        } else {
            ConferenceEvent::HandLowered {
                room_number: key.room.clone(),
                participant_id: participant_id.to_string(),
                by_host: matches!(by, HandBy::Host | HandBy::Stage),
                tenant_id: key.tenant,
                timestamp,
            }
        });
        if tell {
            self.tell_qa(&call_id, QaMessage::Hand { raised });
        }
        Ok(true)
    }

    /// `*4`: raise the hand if it is down, lower it if it is up.
    pub fn toggle_hand(&self, key: &RoomKey, participant_id: &str) -> Result<bool, StageError> {
        let call_id = self
            .call_of_participant(key, participant_id)
            .ok_or(StageError::NotFound)?;
        let up = self
            .sessions
            .get(&call_id)
            .ok_or(StageError::NotFound)?
            .hand
            .is_some();
        self.set_hand(key, participant_id, !up, HandBy::Owner)
    }

    /// A host lowers every hand in the room. How many came down.
    pub fn lower_hands(&self, key: &RoomKey) -> usize {
        let calls = self
            .room_sessions
            .get(key)
            .map(|c| c.clone())
            .unwrap_or_default();
        let raised: Vec<String> = calls
            .iter()
            .filter_map(|c| {
                let s = self.sessions.get(c)?;
                s.hand.is_some().then(|| s.participant_id.clone())
            })
            .collect();
        raised
            .iter()
            .filter(|pid| matches!(self.set_hand(key, pid, false, HandBy::Host), Ok(true)))
            .count()
    }

    /// Bring a webinar's attendee to the stage (`on`) or send them back.
    /// Whether anything changed.
    pub fn set_stage(
        &self,
        key: &RoomKey,
        participant_id: &str,
        on: bool,
    ) -> Result<bool, StageError> {
        let call_id = self
            .call_of_participant(key, participant_id)
            .ok_or(StageError::NotFound)?;
        let (role, display_name) = {
            let s = self.sessions.get(&call_id).ok_or(StageError::NotFound)?;
            (s.role, s.display_name.clone())
        };
        if role != ConferenceRole::Attendee {
            return Err(StageError::Refused(
                "only a webinar's attendee is brought to the stage and sent back",
            ));
        }
        let room = self
            .bridge
            .get_room(key)
            .map_err(|_| StageError::NotFound)?;
        if room.is_listener(participant_id) != on {
            return Ok(false);
        }
        room.set_listener(participant_id, !on)
            .map_err(|_| StageError::NotFound)?;
        info!(
            room = %key.room,
            participant = %participant_id,
            on,
            "An attendee {}",
            if on { "brought to the stage" } else { "sent back to the audience" }
        );
        if on {
            let _ = self.set_hand(key, participant_id, false, HandBy::Stage);
        }
        let timestamp = SystemTime::now();
        self.publish_event(if on {
            ConferenceEvent::StageJoined {
                room_number: key.room.clone(),
                participant_id: participant_id.to_string(),
                display_name,
                tenant_id: key.tenant,
                timestamp,
            }
        } else {
            ConferenceEvent::StageLeft {
                room_number: key.room.clone(),
                participant_id: participant_id.to_string(),
                tenant_id: key.tenant,
                timestamp,
            }
        });
        self.tell_qa(&call_id, QaMessage::Stage { on });
        Ok(true)
    }

    /// A participant's `fcp-qa` INFO: their own hand. The room alone says
    /// who is on the stage.
    pub(super) fn take_qa(
        &self,
        key: &RoomKey,
        participant_id: &str,
        body: &[u8],
    ) -> (u16, &'static str) {
        match QaMessage::parse(body) {
            Ok(QaMessage::Hand { raised }) => {
                match self.set_hand(key, participant_id, raised, HandBy::OwnerInfo) {
                    Ok(_) => (200, "OK"),
                    Err(StageError::Refused(_)) => (403, "Forbidden"),
                    Err(StageError::NotFound) => (481, "Call/Transaction Does Not Exist"),
                }
            }
            Ok(QaMessage::Stage { .. }) => (403, "Forbidden"),
            Err(e) => {
                debug!(participant = %participant_id, error = %e, "fcp-qa INFO refused");
                (400, "Bad Request")
            }
        }
    }

    /// Tell the participant on `call_id`, when they asked to be told
    /// (`Recv-Info: fcp-qa`), what the room did.
    fn tell_qa(&self, call_id: &str, message: QaMessage) {
        let asked = self.sessions.get(call_id).is_some_and(|s| s.qa_notices);
        let Some(me) = self.me().filter(|_| asked) else {
            return;
        };
        let call_id = call_id.to_string();
        tokio::spawn(async move { me.send_qa_info(call_id, message).await });
    }

    async fn send_qa_info(self: Arc<Self>, call_id: String, message: QaMessage) {
        // The INFO's CSeq is taken from the session's dialog as it is
        // cloned, so two sent at once (a hand and the stage) differ.
        let Some(mut dialog) = self.sessions.get_mut(&call_id).map(|mut s| {
            let dialog = s.dialog.clone();
            s.dialog.next_local_cseq();
            dialog
        }) else {
            return;
        };
        let request = match fcp_sip::conference_relay::in_dialog_request(
            &dialog,
            sip_core::Method::Info,
            &[
                (
                    "Info-Package".to_string(),
                    fcp_core::QA_INFO_PACKAGE.to_string(),
                ),
                (
                    "Content-Type".to_string(),
                    fcp_core::QA_INFO_TYPE.to_string(),
                ),
            ],
            message.to_body().as_bytes(),
        ) {
            Ok(request) => request,
            Err(e) => {
                warn!(call_id = %call_id, error = %e, "fcp-qa INFO could not be built");
                return;
            }
        };
        match self
            .uac
            .inner()
            .send_in_dialog_non_invite(&mut dialog, request)
            .await
        {
            Ok(resp) if (200..300).contains(&resp.start_line().code()) => {}
            Ok(resp) => debug!(
                call_id = %call_id,
                code = resp.start_line().code(),
                "fcp-qa INFO refused"
            ),
            Err(e) => debug!(call_id = %call_id, error = %e, "fcp-qa INFO failed"),
        }
    }
}
