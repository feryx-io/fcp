//! A room's sitting and a webinar's life (`docs/WEBINARS.md` §2.6, W4).
//!
//! A sitting is one run of a room on this node, from its first caller to
//! its last, lobby included. Its id is the `session_id` of the room's
//! questions and chat, and it is kept as a `conference_sittings` row when
//! it begins, goes live and ends.
//!
//! A webinar's sitting begins in practice. The stage is in the room and
//! hears itself; an attendee who arrives waits in the lobby, on hold
//! music, and is not in the room at all: not mixed, not on a roster, not
//! counted. Nothing is recorded. A host takes it live (`*95`, the API or
//! the app), and the lobby is let in at `[conference]
//! lobby_admit_per_sec`, oldest first, so a thousand do not join the mixer
//! in one frame. Recording starts if the meeting records. It ends when a
//! host ends it (`*00`, the API), or when its stage has been empty for
//! `webinar_stage_grace_secs` while live: everyone is told, hears the
//! ending announcement and is released. A meeting's sitting is live from
//! its start and has no lobby.
//!
//! A participant who asked for `fcp-qa` is told the phase when they join
//! and as it changes, and whether they are in the lobby
//! (`{"op":"webinar",…}`).

use super::{ConferenceCallHandler, ConferenceRole, RoomKey};
use fcp_core::qa::QaMessage;
use fcp_core::webinar::{Sitting, WebinarPhase};
use fcp_core::Meeting;
use fcp_events::ConferenceEvent;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, info, warn};

/// Why a webinar could not be moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebinarError {
    /// No room of that number on this node.
    NotFound,
    /// The room is a meeting, which has no phases.
    NotWebinar,
    /// The webinar has ended.
    Ended,
}

impl WebinarError {
    /// The HTTP status for this error.
    pub fn status(&self) -> u16 {
        match self {
            WebinarError::NotFound => 404,
            WebinarError::NotWebinar | WebinarError::Ended => 409,
        }
    }
}

impl std::fmt::Display for WebinarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebinarError::NotFound => f.write_str("no such room on this node"),
            WebinarError::NotWebinar => f.write_str("the room is a meeting, not a webinar"),
            WebinarError::Ended => f.write_str("the webinar has ended"),
        }
    }
}

impl std::error::Error for WebinarError {}

/// A room's sitting on this node.
pub(super) struct RoomSitting {
    kept: Sitting,
    webinar: bool,
    auto_record: bool,
    phase: WebinarPhase,
    /// Who waits in the lobby, oldest first. One who left is skipped when
    /// reached: [`waiting`](Self::waiting) is who is still there.
    lobby: VecDeque<String>,
    waiting: HashSet<String>,
    /// Whether a task is letting the lobby in.
    letting_in: bool,
    /// Since when a live webinar's stage has been empty.
    stage_empty_since: Option<Instant>,
}

impl RoomSitting {
    fn new(key: &RoomKey, meeting: &Meeting) -> Self {
        let now = SystemTime::now();
        let webinar = meeting.is_webinar();
        Self {
            kept: Sitting {
                id: uuid::Uuid::new_v4().to_string(),
                tenant_id: key.tenant,
                meeting_id: Some(meeting.id.to_string()),
                room_number: key.room.clone(),
                kind: meeting.kind.clone(),
                started_at: now,
                live_at: (!webinar).then_some(now),
                ended_at: None,
                ended_by: None,
            },
            webinar,
            auto_record: meeting.auto_record,
            phase: if webinar {
                WebinarPhase::Practice
            } else {
                WebinarPhase::Live
            },
            lobby: VecDeque::new(),
            waiting: HashSet::new(),
            letting_in: false,
            stage_empty_since: None,
        }
    }

    fn json(&self) -> serde_json::Value {
        let at = |t: Option<SystemTime>| {
            t.map(|t| {
                chrono::DateTime::<chrono::Utc>::from(t)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            })
        };
        serde_json::json!({
            "id": self.kept.id,
            "kind": self.kept.kind,
            "phase": self.phase.as_str(),
            "started_at": at(Some(self.kept.started_at)),
            "live_at": at(self.kept.live_at),
            "lobby": self.waiting.len(),
        })
    }
}

impl ConferenceCallHandler {
    /// A caller is joining `key`'s room of `meeting`: the room's sitting
    /// begins if it has not.
    pub(super) fn sitting_begin(&self, key: &RoomKey, meeting: &Meeting) {
        let mut begun = None;
        self.sittings.entry(key.clone()).or_insert_with(|| {
            let s = RoomSitting::new(key, meeting);
            begun = Some(s.kept.clone());
            s
        });
        if let Some(kept) = begun {
            info!(room = %key.room, sitting = %kept.id, kind = %kept.kind, "A sitting began");
            self.keep_sitting(kept);
        }
    }

    /// The id of `key`'s sitting: the `session_id` of its questions and
    /// chat. A room joined without one (which a caller never is) gets an
    /// id of its own.
    pub(super) fn sitting_id(&self, key: &RoomKey) -> String {
        self.sittings
            .get(key)
            .map(|s| s.kept.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    }

    /// This node is an edge of `key`'s webinar, and the stage says its
    /// sitting is `sitting_id` in `phase` (`docs/WEBINARS.md` §2.7). The
    /// edge's sitting becomes the stage's: its id (so questions, chat and
    /// the record name one sitting) and its phase. Going live lets the
    /// lobby in, and ending ends the edge. Quietly: the stage publishes the
    /// events, keeps the row and records.
    pub(super) fn follow_stage(&self, key: &RoomKey, sitting_id: &str, phase: WebinarPhase) {
        let (old_id, let_in, now) = {
            let Some(mut s) = self.sittings.get_mut(key) else {
                return;
            };
            if !s.webinar || sitting_id.is_empty() {
                return;
            }
            let old_id = (s.kept.id != sitting_id)
                .then(|| std::mem::replace(&mut s.kept.id, sitting_id.to_string()));
            let mut let_in = false;
            let now = match (s.phase, phase) {
                (WebinarPhase::Practice, WebinarPhase::Live) => {
                    s.phase = WebinarPhase::Live;
                    s.kept.live_at = Some(SystemTime::now());
                    let_in = !s.letting_in && !s.waiting.is_empty();
                    if let_in {
                        s.letting_in = true;
                    }
                    Some(WebinarPhase::Live)
                }
                (WebinarPhase::Practice | WebinarPhase::Live, WebinarPhase::Ended) => {
                    s.phase = WebinarPhase::Ended;
                    s.kept.ended_by = Some("stage".to_string());
                    Some(WebinarPhase::Ended)
                }
                _ => None,
            };
            (old_id, let_in, now)
        };
        if let Some(old) = old_id {
            info!(room = %key.room, sitting = %sitting_id, was = %old, "An edge took the stage's sitting");
            if let Some(mut q) = self.qa_rooms.get_mut(key) {
                q.set_session_id(sitting_id);
            }
            if let Some(mut c) = self.chat_rooms.get_mut(key) {
                c.set_session_id(sitting_id);
            }
            // The row it wrote before it knew it was an edge.
            if let Some(store) = self.storage.sitting_store().cloned() {
                let tenant = key.tenant;
                tokio::spawn(async move {
                    if let Err(e) = store.delete_sitting(&tenant, &old).await {
                        warn!(sitting = %old, error = %e, "An edge's own sitting could not be forgotten");
                    }
                });
            }
        }
        let Some(now) = now else {
            return;
        };
        info!(room = %key.room, sitting = %sitting_id, phase = now.as_str(), "An edge followed the stage");
        self.tell_phase(key, now);
        let Some(me) = self.me() else {
            return;
        };
        let key = key.clone();
        match now {
            WebinarPhase::Live if let_in => {
                tokio::spawn(async move { me.let_in_lobby(key).await });
            }
            WebinarPhase::Ended => {
                tokio::spawn(async move {
                    me.end_with_announcement(&key).await;
                });
            }
            _ => {}
        }
    }

    /// The sitting's id and phase, for a webinar's stage to tell its edges.
    pub(super) fn sitting_news(&self, key: &RoomKey) -> Option<(String, WebinarPhase)> {
        self.sittings
            .get(key)
            .filter(|s| s.webinar)
            .map(|s| (s.kept.id.clone(), s.phase))
    }

    /// The sitting's phase and id, for the room's JSON.
    pub fn sitting_json(&self, key: &RoomKey) -> Option<serde_json::Value> {
        let mut json = self.sittings.get(key).map(|s| s.json())?;
        if let Some(tree) = self.tree_json(key) {
            // The lobby is every node's: an edge's waits for the stage too.
            let elsewhere = tree["edges"]
                .as_array()
                .map(|e| e.iter().filter_map(|e| e["lobby"].as_u64()).sum::<u64>())
                .unwrap_or(0);
            if let Some(lobby) = json["lobby"].as_u64() {
                json["lobby"] = (lobby + elsewhere).into();
            }
            json["tree"] = tree;
        }
        if let Some(stream) = self.stream_json(key) {
            json["stream"] = stream;
        }
        if let Some(push) = self.push_json(key) {
            json["rtmp"] = push;
        }
        Some(json)
    }

    /// The phase of `key`'s sitting, when it has one.
    pub fn webinar_phase(&self, key: &RoomKey) -> Option<WebinarPhase> {
        self.sittings
            .get(key)
            .filter(|s| s.webinar)
            .map(|s| s.phase)
    }

    /// Whether an attendee arriving now waits in the lobby: the room is a
    /// webinar that is not live.
    pub(super) fn lobby_holds(&self, key: &RoomKey) -> bool {
        self.sittings
            .get(key)
            .is_some_and(|s| s.webinar && s.phase == WebinarPhase::Practice)
    }

    /// Whether `key`'s room is recorded as it fills (`auto_record`): a
    /// webinar only once it is live.
    pub(super) fn sitting_records(&self, key: &RoomKey) -> bool {
        self.sittings
            .get(key)
            .is_none_or(|s| s.phase == WebinarPhase::Live)
    }

    /// Whether a caller arriving now is refused: the webinar is ending.
    pub(super) fn sitting_ended(&self, key: &RoomKey) -> bool {
        self.sittings
            .get(key)
            .is_some_and(|s| s.phase == WebinarPhase::Ended)
    }

    /// `participant_id` waits in `key`'s lobby.
    pub(super) fn lobby_enter(&self, key: &RoomKey, participant_id: &str) {
        if let Some(mut s) = self.sittings.get_mut(key) {
            if s.waiting.insert(participant_id.to_string()) {
                s.lobby.push_back(participant_id.to_string());
            }
            info!(room = %key.room, participant = %participant_id, waiting = s.waiting.len(), "An attendee waits in the lobby");
        }
    }

    /// Whether `participant_id` is still waiting in `key`'s lobby.
    pub fn in_lobby(&self, key: &RoomKey, participant_id: &str) -> bool {
        self.sittings
            .get(key)
            .is_some_and(|s| s.waiting.contains(participant_id))
    }

    /// `participant_id` left `key`'s lobby without being let in.
    pub(super) fn lobby_leave(&self, key: &RoomKey, participant_id: &str) {
        if let Some(mut s) = self.sittings.get_mut(key) {
            s.waiting.remove(participant_id);
        }
    }

    /// Whether anyone waits in `key`'s lobby.
    pub(super) fn lobby_waits(&self, key: &RoomKey) -> bool {
        self.lobby_count(key) > 0
    }

    /// How many wait in `key`'s lobby.
    pub(super) fn lobby_count(&self, key: &RoomKey) -> usize {
        self.sittings.get(key).map_or(0, |s| s.waiting.len())
    }

    /// The participant on `call_id` joined: tell them, if they asked, the
    /// webinar's phase and whether they are in the lobby.
    pub(super) fn webinar_joined(&self, key: &RoomKey, call_id: &str, participant_id: &str) {
        let Some(phase) = self.webinar_phase(key) else {
            return;
        };
        let lobby = self.in_lobby(key, participant_id);
        let role = self.role_on(call_id);
        self.tell_qa(call_id, QaMessage::Webinar { phase, lobby, role });
    }

    /// The role of whoever is on `call_id`, as a client is told it.
    fn role_on(&self, call_id: &str) -> Option<fcp_core::JoinRole> {
        let role = self.sessions.get(call_id)?.role;
        Some(match role {
            super::ConferenceRole::Host => fcp_core::JoinRole::Host,
            super::ConferenceRole::Panelist => fcp_core::JoinRole::Panelist,
            super::ConferenceRole::Attendee => fcp_core::JoinRole::Attendee,
            super::ConferenceRole::Guest => fcp_core::JoinRole::Guest,
        })
    }

    /// Tell everyone in `key`'s room who asked the webinar's phase.
    fn tell_phase(&self, key: &RoomKey, phase: WebinarPhase) {
        let calls = self
            .room_sessions
            .get(key)
            .map(|c| c.clone())
            .unwrap_or_default();
        for call_id in calls {
            let Some((pid, trunk)) = self
                .sessions
                .get(&call_id)
                .map(|s| (s.participant_id.clone(), s.trunk_peer.is_some()))
            else {
                continue;
            };
            if trunk {
                continue;
            }
            let lobby = self.in_lobby(key, &pid);
            // One still waiting when the webinar goes live is told as they
            // are let in, so the two messages cannot cross.
            if lobby && phase == WebinarPhase::Live {
                continue;
            }
            let role = self.role_on(&call_id);
            self.tell_qa(&call_id, QaMessage::Webinar { phase, lobby, role });
        }
    }

    /// A host takes `key`'s webinar live: the lobby is let in, a few at a
    /// time, and recording starts if the meeting records. `by` is the
    /// host's participant id, `None` through the API. Whether anything
    /// changed.
    pub fn go_live(&self, key: &RoomKey, by: Option<&str>) -> Result<bool, WebinarError> {
        let (kept, auto_record, let_in) = {
            let mut s = self.sittings.get_mut(key).ok_or(WebinarError::NotFound)?;
            if !s.webinar {
                return Err(WebinarError::NotWebinar);
            }
            match s.phase {
                WebinarPhase::Live => return Ok(false),
                WebinarPhase::Ended => return Err(WebinarError::Ended),
                WebinarPhase::Practice => {}
            }
            s.phase = WebinarPhase::Live;
            s.kept.live_at = Some(SystemTime::now());
            let let_in = !s.letting_in && !s.waiting.is_empty();
            if let_in {
                s.letting_in = true;
            }
            (s.kept.clone(), s.auto_record, let_in)
        };
        info!(room = %key.room, sitting = %kept.id, by = ?by, "A webinar went live");
        self.publish_event(ConferenceEvent::WebinarLive {
            room_number: key.room.clone(),
            sitting_id: kept.id.clone(),
            by: by.map(str::to_string),
            tenant_id: key.tenant,
            timestamp: SystemTime::now(),
        });
        self.keep_sitting(kept);
        self.tell_phase(key, WebinarPhase::Live);
        if let Some(me) = self.me() {
            let key = key.clone();
            tokio::spawn(async move {
                if auto_record && !me.is_room_recording(&key) {
                    if let Err(e) = me
                        .start_room_recording(&key, fcp_core::RecordingTrigger::Auto)
                        .await
                    {
                        warn!(room = %key.room, error = %e, "Auto-record failed (going live)");
                    }
                }
                // Streamed out from the stage alone (§2.8), and pushed
                // over RTMP (W7).
                me.start_stream(&key).await;
                me.start_push(&key).await;
                if let_in {
                    me.let_in_lobby(key).await;
                }
            });
        }
        Ok(true)
    }

    /// Let `key`'s lobby in, oldest first, at `lobby_admit_per_sec`. Each
    /// is added to the room as a listener; their waiting task sees it and
    /// starts their bridges.
    async fn let_in_lobby(self: Arc<Self>, key: RoomKey) {
        let rate = self.conference_config.read().lobby_admit_per_sec.max(1);
        let gap = Duration::from_secs(1) / rate;
        let mut admitted = 0usize;
        loop {
            let next = {
                let Some(mut s) = self.sittings.get_mut(&key) else {
                    break;
                };
                if s.phase != WebinarPhase::Live {
                    s.letting_in = false;
                    break;
                }
                // One who left meanwhile is skipped.
                let mut next = None;
                while let Some(pid) = s.lobby.pop_front() {
                    if s.waiting.contains(&pid) {
                        next = Some(pid);
                        break;
                    }
                }
                if next.is_none() {
                    s.letting_in = false;
                }
                next
            };
            let Some(pid) = next else {
                break;
            };
            let added = match self.bridge.get_room(&key) {
                Ok(room) => room.add_listener_admitted(&pid).map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            // Out of the lobby only once in the room: their waiting task
            // takes "neither" as having been let go.
            self.lobby_leave(&key, &pid);
            match added {
                Ok(()) => {
                    admitted += 1;
                    if let Some(call_id) = self.call_of(&key, &pid) {
                        let role = self.role_on(&call_id);
                        self.tell_qa(
                            &call_id,
                            QaMessage::Webinar {
                                phase: WebinarPhase::Live,
                                lobby: false,
                                role,
                            },
                        );
                    }
                }
                Err(e) => {
                    warn!(room = %key.room, participant = %pid, error = %e, "An attendee could not be let in from the lobby");
                    if let Some(call_id) = self.call_of(&key, &pid) {
                        let dialog = self.sessions.get(&call_id).map(|s| s.dialog.clone());
                        if let Some(dialog) = dialog {
                            if let Err(e) = self.uac.bye(&dialog).await {
                                debug!(error = %e, "BYE to a refused attendee failed");
                            }
                        }
                        self.remove_session_for(
                            &call_id,
                            fcp_sip::TerminationReason::system(
                                fcp_sip::SystemCause::Other,
                                "Not let in from the lobby",
                            ),
                        );
                    }
                }
            }
            tokio::time::sleep(gap).await;
        }
        // The registry counted them waiting; this keeps it exact.
        self.sync_room_registry(&key);
        info!(room = %key.room, admitted, "The lobby let in");
    }

    /// The call of `participant_id` in `key`'s room, trunks aside.
    fn call_of(&self, key: &RoomKey, participant_id: &str) -> Option<String> {
        let calls = self.room_sessions.get(key)?.clone();
        calls.into_iter().find(|c| {
            self.sessions
                .get(c)
                .is_some_and(|s| s.participant_id == participant_id && s.trunk_peer.is_none())
        })
    }

    /// Mark `key`'s webinar ended by `by`, telling everyone. Whether it
    /// was not ended already (a meeting's sitting is never marked).
    fn mark_ended(&self, key: &RoomKey, by: &str) -> bool {
        let kept = {
            let Some(mut s) = self.sittings.get_mut(key) else {
                return false;
            };
            if !s.webinar || s.phase == WebinarPhase::Ended {
                return false;
            }
            s.phase = WebinarPhase::Ended;
            s.kept.ended_by = Some(by.to_string());
            s.kept.clone()
        };
        info!(room = %key.room, sitting = %kept.id, by, "A webinar ended");
        self.publish_event(ConferenceEvent::WebinarEnded {
            room_number: key.room.clone(),
            sitting_id: kept.id,
            by: by.to_string(),
            tenant_id: key.tenant,
            timestamp: SystemTime::now(),
        });
        self.tell_phase(key, WebinarPhase::Ended);
        // A stage's edges end with it: told now, before its room empties
        // and it is a stage no more.
        self.queue_tree_ending(key);
        if let Some(me) = self.me() {
            let key = key.clone();
            tokio::spawn(async move {
                me.stop_stream(&key).await;
                me.stop_push(&key).await;
            });
        }
        true
    }

    /// The room is being destroyed (`*00`, the API): a webinar in it ends
    /// by `by`, unless it has already.
    pub fn sitting_destroyed(&self, key: &RoomKey, by: &str) {
        self.mark_ended(key, by);
    }

    /// A host ends `key`'s webinar: everyone hears the ending announcement
    /// and is released. `by` is the host's participant id, or `api`. How
    /// many were released.
    pub async fn end_webinar(&self, key: &RoomKey, by: &str) -> Result<usize, WebinarError> {
        {
            let s = self.sittings.get(key).ok_or(WebinarError::NotFound)?;
            if !s.webinar {
                return Err(WebinarError::NotWebinar);
            }
        }
        if !self.mark_ended(key, by) {
            return Err(WebinarError::Ended);
        }
        Ok(self.end_with_announcement(key).await)
    }

    /// Play the ending announcement to everyone in `key`'s room, then
    /// release them. How many were released.
    pub async fn end_with_announcement(&self, key: &RoomKey) -> usize {
        if let Some(pcm) = self.dtmf_ending_samples.clone() {
            self.play_to_all_participants(key, &pcm).await;
        }
        match self.destroy_conference(key).await {
            Ok(n) => n,
            Err(e) => {
                warn!(room = %key.room, error = %e, "A room could not be ended");
                0
            }
        }
    }

    /// The sitting ended: its last caller left, or the room was destroyed.
    /// It is kept as it ended and let go.
    pub(super) fn sitting_closed(&self, key: &RoomKey) {
        let Some((_, mut s)) = self.sittings.remove(key) else {
            return;
        };
        s.kept.ended_at = Some(SystemTime::now());
        info!(room = %key.room, sitting = %s.kept.id, phase = s.phase.as_str(), "A sitting ended");
        self.keep_sitting(s.kept);
        self.leave_the_stage(key);
        self.audience_encode_held.remove(key);
        if let Some(me) = self.me() {
            let key = key.clone();
            tokio::spawn(async move {
                me.stop_stream(&key).await;
                me.stop_push(&key).await;
            });
        }
    }

    /// The live webinars whose stage has been empty past `grace`: each is
    /// ended (`stage_left`). Called once a second.
    pub fn end_empty_stages(&self, grace: Duration) {
        // An edge's stage is always empty: its webinar ends with the
        // stage's (`docs/WEBINARS.md` §2.7).
        let live: Vec<RoomKey> = self
            .sittings
            .iter()
            .filter(|s| s.webinar && s.phase == WebinarPhase::Live)
            .map(|s| s.key().clone())
            .filter(|key| !self.is_edge(key))
            .collect();
        let now = Instant::now();
        for key in live {
            let empty = self.stage_is_empty(&key);
            let expired = {
                let Some(mut s) = self.sittings.get_mut(&key) else {
                    continue;
                };
                if !empty {
                    s.stage_empty_since = None;
                    false
                } else {
                    let since = *s.stage_empty_since.get_or_insert(now);
                    now.duration_since(since) >= grace
                }
            };
            if expired && self.mark_ended(&key, "stage_left") {
                if let Some(me) = self.me() {
                    tokio::spawn(async move {
                        me.end_with_announcement(&key).await;
                    });
                }
            }
        }
    }

    /// Whether nobody is on `key`'s stage: no host, panelist or promoted
    /// attendee in the room (trunks and the lobby aside).
    fn stage_is_empty(&self, key: &RoomKey) -> bool {
        let Ok(room) = self.bridge.get_room(key) else {
            return true;
        };
        let calls = self
            .room_sessions
            .get(key)
            .map(|c| c.clone())
            .unwrap_or_default();
        let present: HashSet<String> = room.participants().into_iter().collect();
        !calls.iter().any(|c| {
            self.sessions.get(c).is_some_and(|s| {
                s.trunk_peer.is_none()
                    && !self.in_lobby(key, &s.participant_id)
                    && match s.role {
                        ConferenceRole::Attendee => {
                            !room.is_listener(&s.participant_id)
                                && present.contains(&s.participant_id)
                        }
                        _ => true,
                    }
            })
        })
    }

    /// Take a host's `*95`: the webinar goes live.
    pub fn take_go_live(&self, key: &RoomKey, participant_id: &str) {
        let host = self
            .call_of(key, participant_id)
            .and_then(|c| self.sessions.get(&c).map(|s| s.is_host))
            .unwrap_or(false);
        if !host {
            debug!(room = %key.room, participant = %participant_id, "*95 from someone not a host");
            return;
        }
        match self.go_live(key, Some(participant_id)) {
            Ok(_) => {}
            Err(e) => debug!(room = %key.room, error = %e, "*95 not taken"),
        }
    }

    /// The registration a join token names, when it stands: `Ok(None)` for
    /// a token naming none that the meeting admits without; the reason
    /// phrase to refuse with otherwise.
    pub(super) async fn admit_registrant(
        &self,
        meeting: &Meeting,
        token: &fcp_core::JoinToken,
    ) -> Result<Option<String>, &'static str> {
        let attendee = ConferenceRole::from_token(token.role, meeting) == ConferenceRole::Attendee;
        let Some(id) = token.registrant.as_deref() else {
            return if attendee && meeting.is_webinar() && meeting.registration {
                Err("Registration Required")
            } else {
                Ok(None)
            };
        };
        let Some(store) = self.storage.registrant_store() else {
            warn!(room = %meeting.room_number, "A registrant's token, and no registrations to check it against");
            return Err("Registration Unavailable");
        };
        match store.get_registrant(&meeting.tenant_id, id).await {
            Ok(Some(r)) if r.meeting_id == meeting.id.to_string() && r.stands() => Ok(Some(r.id)),
            Ok(Some(r)) if r.meeting_id == meeting.id.to_string() => Err("Registration Revoked"),
            Ok(_) => Err("No Such Registration"),
            Err(e) => {
                warn!(room = %meeting.room_number, error = %e, "A registration could not be read");
                Err("Registration Unavailable")
            }
        }
    }

    /// A page of `key`'s audience: its attendees (in the room, on the stage
    /// from it, or in the lobby), by participant id, `limit` after
    /// `after`, and how many there are. For a host who wants to find
    /// someone (`docs/WEBINARS.md` §2.6); nobody is NOTIFYed the list.
    pub fn audience_page(
        &self,
        key: &RoomKey,
        after: Option<&str>,
        limit: usize,
    ) -> Option<(Vec<serde_json::Value>, Option<String>, usize)> {
        let room = self.bridge.get_room(key).ok()?;
        let calls = self
            .room_sessions
            .get(key)
            .map(|c| c.clone())
            .unwrap_or_default();
        let mut audience: Vec<serde_json::Value> = calls
            .iter()
            .filter_map(|c| {
                let s = self.sessions.get(c)?;
                if s.trunk_peer.is_some() || s.role != ConferenceRole::Attendee {
                    return None;
                }
                let lobby = self.in_lobby(key, &s.participant_id);
                let rfc3339 = |t: SystemTime| {
                    chrono::DateTime::<chrono::Utc>::from(t)
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                };
                Some(serde_json::json!({
                    "participant_id": s.participant_id,
                    "display_name": s.display_name,
                    "caller_id": s.caller_id,
                    "registrant": s.registrant,
                    "joined_at": rfc3339(s.joined_at),
                    "hand": s.hand.map(rfc3339),
                    "lobby": lobby,
                    "on_stage": !lobby && !room.is_listener(&s.participant_id),
                }))
            })
            .collect();
        let total = audience.len();
        let id_of = |v: &serde_json::Value| v["participant_id"].as_str().unwrap_or("").to_string();
        audience.sort_by_key(id_of);
        let start = after.map_or(0, |a| audience.partition_point(|v| id_of(v).as_str() <= a));
        let page: Vec<serde_json::Value> =
            audience.into_iter().skip(start).take(limit + 1).collect();
        let more = page.len() > limit;
        let page: Vec<serde_json::Value> = page.into_iter().take(limit).collect();
        let next = more.then(|| page.last().map(id_of)).flatten();
        Some((page, next, total))
    }

    fn keep_sitting(&self, sitting: Sitting) {
        // An edge's sitting is the stage's, which the stage keeps.
        if self.is_edge(&RoomKey::new(
            sitting.tenant_id,
            sitting.room_number.clone(),
        )) {
            return;
        }
        let Some(store) = self.storage.sitting_store().cloned() else {
            return;
        };
        tokio::spawn(async move {
            if let Err(e) = store.put_sitting(&sitting).await {
                warn!(sitting = %sitting.id, error = %e, "A sitting could not be kept");
            }
        });
    }
}
