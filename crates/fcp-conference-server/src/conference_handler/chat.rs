//! Chat in every room (`docs/WEBINARS.md` §2.5, W3b).
//!
//! Anyone in a room posts to the room or privately to its hosts and
//! panelists, as the meeting's `chat` allows ([`ChatScope`]); hosts and
//! panelists always have their own channel. A host or panelist deletes any
//! message, which is then gone for everyone, and mutes a participant's chat
//! for the rest of the sitting.
//!
//! A room's chat lives here, on the node holding the room, for one sitting:
//! from the room's first participant to its last. Each message is a
//! `conference_chat` row, written as it is posted and again when deleted.
//! A participant who asked to be told (`Recv-Info: fcp-chat`) is sent the
//! messages they may see as they arrive, coalesced into one `fcp-chat`
//! INFO each [`CHAT_FLUSH`] (`{"op":"messages",…}`), and the last
//! [`CHAT_BACKLOG`] when they join a room that has some.

use super::{ConferenceCallHandler, ConferenceRole, RoomKey};
use fcp_core::chat::{
    ChatAudience, ChatMessage, ChatRecord, ChatScope, ChatView, CHAT_BACKLOG, MAX_CHAT_CHARS,
};
use fcp_core::Meeting;
use fcp_events::ConferenceEvent;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, info, warn};

/// How often new messages are sent to those told.
pub const CHAT_FLUSH: Duration = Duration::from_millis(250);
/// The most messages a room holds in memory for a sitting; older ones are
/// kept as rows only.
const MAX_HELD_MESSAGES: usize = 2000;
/// How many messages one participant may post in [`POST_WINDOW`].
const MAX_POSTS_PER_WINDOW: usize = 10;
const POST_WINDOW: Duration = Duration::from_secs(10);
/// The largest `messages` INFO body sent: the rest go in the next one.
const MAX_MESSAGES_BODY: usize = 8 * 1024;

/// Why a message could not be posted, deleted or a participant muted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatError {
    /// The room's chat does not take what was sent from them.
    Off,
    /// Nobody of that id or call in the room, or no such message for them.
    NotFound,
    /// Not theirs to do: muted, a moderator's operation, a moderator muted.
    Refused(&'static str),
    /// What was sent is not a message.
    Invalid(&'static str),
    /// Too many messages too quickly.
    TooMany,
}

impl ChatError {
    /// The HTTP status for this error.
    pub fn status(&self) -> u16 {
        match self {
            ChatError::Off | ChatError::Refused(_) => 403,
            ChatError::NotFound => 404,
            ChatError::Invalid(_) => 400,
            ChatError::TooMany => 429,
        }
    }
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatError::Off => f.write_str("this room's chat does not take that message"),
            ChatError::NotFound => f.write_str("no such message or participant in the room"),
            ChatError::Refused(why) | ChatError::Invalid(why) => f.write_str(why),
            ChatError::TooMany => f.write_str("too many messages; wait a moment"),
        }
    }
}

impl std::error::Error for ChatError {}

/// One message held by its room.
struct Held {
    kept: ChatRecord,
    sender_moderates: bool,
}

/// A room's chat for one sitting.
pub(super) struct ChatRoom {
    session_id: String,
    meeting_id: Option<String>,
    scope: ChatScope,
    messages: VecDeque<Held>,
    /// Messages posted or deleted since the last flush, by id.
    changed: Vec<String>,
    /// Calls owed the backlog: they joined since.
    fresh: HashSet<String>,
    /// Participants whose chat is muted, by participant id.
    muted: HashSet<String>,
    /// Each participant's recent posts, for the rate limit.
    posts: HashMap<String, VecDeque<Instant>>,
}

impl ChatRoom {
    fn new(meeting: &Meeting, session_id: String) -> Self {
        Self {
            session_id,
            meeting_id: Some(meeting.id.to_string()),
            scope: meeting.chat_scope(),
            messages: VecDeque::new(),
            changed: Vec::new(),
            fresh: HashSet::new(),
            muted: HashSet::new(),
            posts: HashMap::new(),
        }
    }

    fn find(&mut self, id: &str) -> Option<&mut Held> {
        self.messages.iter_mut().find(|m| m.kept.id == id)
    }

    /// Whether `pid` may post now, counting this post if so.
    fn take_post(&mut self, pid: &str, now: Instant) -> bool {
        let recent = self.posts.entry(pid.to_string()).or_default();
        while recent
            .front()
            .is_some_and(|t| now.duration_since(*t) >= POST_WINDOW)
        {
            recent.pop_front();
        }
        if recent.len() >= MAX_POSTS_PER_WINDOW {
            return false;
        }
        recent.push_back(now);
        true
    }
}

/// Who a message is being shown to.
#[derive(Debug, Clone, Copy)]
struct Viewer<'a> {
    /// `None` for a host through the API.
    pid: Option<&'a str>,
    moderates: bool,
}

impl Viewer<'_> {
    /// Whether this viewer is told of `m` at all: a moderator of every
    /// message, anyone else of the room's and their own.
    fn told_of(&self, m: &Held) -> bool {
        self.moderates || m.kept.audience == ChatAudience::Room || self.mine(m)
    }

    /// Whether `m` is in the list this viewer sees: told of it, and not
    /// deleted.
    fn sees(&self, m: &Held) -> bool {
        m.kept.deleted_at.is_none() && self.told_of(m)
    }

    fn mine(&self, m: &Held) -> bool {
        self.pid.is_some_and(|p| p == m.kept.participant_id)
    }

    fn view(&self, m: &Held) -> ChatView {
        let deleted = m.kept.deleted_at.is_some();
        ChatView {
            id: m.kept.id.clone(),
            text: if deleted {
                String::new()
            } else {
                m.kept.text.clone()
            },
            sender: m.kept.sender_name.clone(),
            sender_id: m.kept.participant_id.clone(),
            sender_moderates: m.sender_moderates,
            mine: self.mine(m),
            to: m.kept.audience,
            deleted,
            sent_at: chrono::DateTime::<chrono::Utc>::from(m.kept.sent_at)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        }
    }
}

/// `views` as `messages` INFO bodies of at most [`MAX_MESSAGES_BODY`] each
/// (a single message larger than that goes alone).
fn message_bodies(views: Vec<ChatView>) -> Vec<String> {
    let mut bodies = Vec::new();
    let mut batch: Vec<ChatView> = Vec::new();
    let mut size = 0;
    for v in views {
        let len = serde_json::to_string(&v).map(|s| s.len()).unwrap_or(0) + 1;
        if !batch.is_empty() && size + len > MAX_MESSAGES_BODY {
            bodies.push(
                ChatMessage::Messages {
                    messages: std::mem::take(&mut batch),
                }
                .to_body(),
            );
            size = 0;
        }
        size += len;
        batch.push(v);
    }
    if !batch.is_empty() {
        bodies.push(ChatMessage::Messages { messages: batch }.to_body());
    }
    bodies
}

/// The last [`CHAT_BACKLOG`] of `messages` that `viewer` sees, oldest first.
fn backlog<'a>(
    messages: impl DoubleEndedIterator<Item = &'a Held>,
    viewer: Viewer<'_>,
) -> Vec<ChatView> {
    let mut views: Vec<ChatView> = messages
        .rev()
        .filter(|m| viewer.sees(m))
        .take(CHAT_BACKLOG)
        .map(|m| viewer.view(m))
        .collect();
    views.reverse();
    views
}

impl ConferenceCallHandler {
    /// A participant joined `key`'s room of `meeting`: the sitting takes the
    /// meeting's chat scope, and a participant who asked to be told is owed
    /// the backlog.
    pub(super) fn chat_joined(
        &self,
        key: &RoomKey,
        meeting: &Meeting,
        call_id: &str,
        chat_notices: bool,
    ) {
        let mut room = self
            .chat_rooms
            .entry(key.clone())
            .or_insert_with(|| ChatRoom::new(meeting, self.sitting_id(key)));
        room.scope = meeting.chat_scope();
        if chat_notices && !room.messages.is_empty() {
            room.fresh.insert(call_id.to_string());
        }
    }

    /// The room's sitting ended (its last participant left): its chat is
    /// let go. Every message was written as it changed.
    pub(super) fn chat_closed(&self, key: &RoomKey) {
        self.chat_rooms.remove(key);
    }

    /// The call and whether they moderate, of `participant_id` in `key`'s
    /// room.
    fn chat_participant(&self, key: &RoomKey, participant_id: &str) -> Option<(String, bool)> {
        let calls = self.room_sessions.get(key)?.clone();
        calls.into_iter().find_map(|c| {
            let s = self.sessions.get(&c)?;
            (s.participant_id == participant_id && s.trunk_peer.is_none())
                .then(|| (c.clone(), s.is_host || s.role == ConferenceRole::Panelist))
        })
    }

    /// `participant_id` posts `text` to `to`.
    pub fn post_chat(
        &self,
        key: &RoomKey,
        participant_id: &str,
        text: &str,
        to: ChatAudience,
    ) -> Result<ChatView, ChatError> {
        let (call_id, moderates) = self
            .chat_participant(key, participant_id)
            .ok_or(ChatError::NotFound)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(ChatError::Invalid("a message has text"));
        }
        if text.chars().count() > MAX_CHAT_CHARS {
            return Err(ChatError::Invalid("a message is at most 1000 characters"));
        }
        // The name the room shows them by, and where they called from.
        let (sender_name, sender_address) = self
            .sessions
            .get(&call_id)
            .map(|s| {
                (
                    s.display_name.clone().or_else(|| s.caller_id.clone()),
                    Some(s.record.from_uri.clone()).filter(|u| u != "unknown"),
                )
            })
            .unwrap_or_default();
        let now = SystemTime::now();
        let (kept, view) = {
            let mut room = self.chat_rooms.get_mut(key).ok_or(ChatError::NotFound)?;
            if !room.scope.allows(moderates, to) {
                return Err(ChatError::Off);
            }
            if room.muted.contains(participant_id) {
                return Err(ChatError::Refused("your chat is muted"));
            }
            if !room.take_post(participant_id, Instant::now()) {
                return Err(ChatError::TooMany);
            }
            let held = Held {
                kept: ChatRecord {
                    id: format!("m-{}", uuid::Uuid::new_v4()),
                    tenant_id: key.tenant,
                    meeting_id: room.meeting_id.clone(),
                    room_number: key.room.clone(),
                    session_id: room.session_id.clone(),
                    participant_id: participant_id.to_string(),
                    sender_name,
                    sender_address,
                    audience: to,
                    text: text.to_string(),
                    sent_at: now,
                    deleted_at: None,
                    deleted_by: None,
                },
                sender_moderates: moderates,
            };
            let view = Viewer {
                pid: Some(participant_id),
                moderates,
            }
            .view(&held);
            let kept = held.kept.clone();
            room.changed.push(kept.id.clone());
            room.messages.push_back(held);
            while room.messages.len() > MAX_HELD_MESSAGES {
                room.messages.pop_front();
            }
            (kept, view)
        };
        debug!(room = %key.room, participant = %participant_id, message = %kept.id, to = to.as_str(), "A chat message posted");
        self.keep_chat(vec![kept]);
        Ok(view)
    }

    /// A host or panelist deletes message `id`. `by` is the participant
    /// doing it, when they are in the room; a host through the API has been
    /// checked by the API. Whether anything changed.
    pub fn delete_chat(
        &self,
        key: &RoomKey,
        by: Option<&str>,
        id: &str,
    ) -> Result<bool, ChatError> {
        self.check_chat_moderator(key, by)?;
        let now = SystemTime::now();
        let kept = {
            let mut room = self.chat_rooms.get_mut(key).ok_or(ChatError::NotFound)?;
            let m = room.find(id).ok_or(ChatError::NotFound)?;
            if m.kept.deleted_at.is_some() {
                return Ok(false);
            }
            m.kept.deleted_at = Some(now);
            m.kept.deleted_by = by.map(str::to_string);
            let kept = m.kept.clone();
            room.changed.push(id.to_string());
            kept
        };
        info!(room = %key.room, message = %id, by = ?by, "A chat message deleted");
        self.publish_event(ConferenceEvent::ChatDeleted {
            room_number: key.room.clone(),
            message_id: id.to_string(),
            participant_id: kept.participant_id.clone(),
            by: by.map(str::to_string),
            tenant_id: key.tenant,
            timestamp: now,
        });
        self.keep_chat(vec![kept]);
        Ok(true)
    }

    /// A host or panelist mutes (`on`) or unmutes `participant_id`'s chat
    /// for the rest of the sitting. Whether anything changed.
    pub fn mute_chat(
        &self,
        key: &RoomKey,
        by: Option<&str>,
        participant_id: &str,
        on: bool,
    ) -> Result<bool, ChatError> {
        self.check_chat_moderator(key, by)?;
        let (call_id, moderates) = self
            .chat_participant(key, participant_id)
            .ok_or(ChatError::NotFound)?;
        if moderates && on {
            return Err(ChatError::Refused("a host or panelist is not muted"));
        }
        let changed = {
            let mut room = self.chat_rooms.get_mut(key).ok_or(ChatError::NotFound)?;
            if on {
                room.muted.insert(participant_id.to_string())
            } else {
                room.muted.remove(participant_id)
            }
        };
        if !changed {
            return Ok(false);
        }
        info!(room = %key.room, participant = %participant_id, on, by = ?by, "A participant's chat muted");
        self.publish_event(ConferenceEvent::ChatMuted {
            room_number: key.room.clone(),
            participant_id: participant_id.to_string(),
            on,
            by: by.map(str::to_string),
            tenant_id: key.tenant,
            timestamp: SystemTime::now(),
        });
        if self.sessions.get(&call_id).is_some_and(|s| s.chat_notices) {
            self.tell_chat_body(&call_id, ChatMessage::Muted { on }.to_body());
        }
        Ok(true)
    }

    fn check_chat_moderator(&self, key: &RoomKey, by: Option<&str>) -> Result<(), ChatError> {
        if let Some(pid) = by {
            let (_, moderates) = self.chat_participant(key, pid).ok_or(ChatError::NotFound)?;
            if !moderates {
                return Err(ChatError::Refused("only a host or panelist moderates chat"));
            }
        }
        Ok(())
    }

    /// The chat of `key`'s room as `participant_id` sees it, or as a
    /// moderator does when `None`, oldest first: the last
    /// [`CHAT_BACKLOG`] messages, and whether they are muted.
    pub fn chat(
        &self,
        key: &RoomKey,
        participant_id: Option<&str>,
    ) -> Result<(Vec<ChatView>, bool), ChatError> {
        let viewer = match participant_id {
            Some(pid) => {
                let (_, moderates) = self.chat_participant(key, pid).ok_or(ChatError::NotFound)?;
                Viewer {
                    pid: Some(pid),
                    moderates,
                }
            }
            None => Viewer {
                pid: None,
                moderates: true,
            },
        };
        let Some(room) = self.chat_rooms.get(key) else {
            return Ok((Vec::new(), false));
        };
        let muted = participant_id.is_some_and(|p| room.muted.contains(p));
        Ok((backlog(room.messages.iter(), viewer), muted))
    }

    /// A participant's `fcp-chat` INFO: the SIP response's code and reason.
    pub(super) fn take_chat(
        &self,
        key: &RoomKey,
        participant_id: &str,
        body: &[u8],
    ) -> (u16, &'static str) {
        let result = match ChatMessage::parse(body) {
            Ok(ChatMessage::Post { text, to }) => {
                self.post_chat(key, participant_id, &text, to).map(|_| ())
            }
            Ok(ChatMessage::Delete { id }) => {
                self.delete_chat(key, Some(participant_id), &id).map(|_| ())
            }
            Ok(ChatMessage::Mute {
                participant_id: whom,
                on,
            }) => self
                .mute_chat(key, Some(participant_id), &whom, on)
                .map(|_| ()),
            Ok(ChatMessage::Messages { .. } | ChatMessage::Muted { .. }) => {
                Err(ChatError::Refused("only the room sends that"))
            }
            Err(e) => {
                debug!(participant = %participant_id, error = %e, "fcp-chat INFO refused");
                return (400, "Bad Request");
            }
        };
        match result {
            Ok(()) => (200, "OK"),
            Err(e) => {
                debug!(participant = %participant_id, error = %e, "fcp-chat INFO refused");
                match e.status() {
                    400 => (400, "Bad Request"),
                    404 => (404, "Not Found"),
                    _ => (403, "Forbidden"),
                }
            }
        }
    }

    /// Tell each participant who asked to be told of the messages posted
    /// or deleted since the last flush that they may see (the backlog to
    /// one who just joined). Run every [`CHAT_FLUSH`].
    pub fn flush_chat(&self) {
        let keys: Vec<RoomKey> = self
            .chat_rooms
            .iter()
            .filter(|r| !r.changed.is_empty() || !r.fresh.is_empty())
            .map(|r| r.key().clone())
            .collect();
        for key in keys {
            let calls = self
                .room_sessions
                .get(&key)
                .map(|c| c.clone())
                .unwrap_or_default();
            let told: Vec<(String, String, bool)> = calls
                .into_iter()
                .filter_map(|c| {
                    let s = self.sessions.get(&c)?;
                    (s.chat_notices && s.trunk_peer.is_none()).then(|| {
                        (
                            c.clone(),
                            s.participant_id.clone(),
                            s.is_host || s.role == ConferenceRole::Panelist,
                        )
                    })
                })
                .collect();
            let sends = {
                let Some(mut room) = self.chat_rooms.get_mut(&key) else {
                    continue;
                };
                let changed = std::mem::take(&mut room.changed);
                let fresh = std::mem::take(&mut room.fresh);
                // In the order they changed, each once.
                let mut seen = HashSet::new();
                let changed: Vec<&Held> = changed
                    .iter()
                    .filter(|id| seen.insert(id.as_str()))
                    .filter_map(|id| room.messages.iter().find(|m| &m.kept.id == id))
                    .collect();
                let mut sends = Vec::new();
                for (call_id, pid, moderates) in &told {
                    let viewer = Viewer {
                        pid: Some(pid),
                        moderates: *moderates,
                    };
                    let views: Vec<ChatView> = if fresh.contains(call_id) {
                        backlog(room.messages.iter(), viewer)
                    } else {
                        changed
                            .iter()
                            .filter(|m| viewer.told_of(m))
                            .map(|m| viewer.view(m))
                            .collect()
                    };
                    if !views.is_empty() {
                        sends.push((call_id.clone(), views));
                    }
                }
                sends
            };
            for (call_id, views) in sends {
                for body in message_bodies(views) {
                    self.tell_chat_body(&call_id, body);
                }
            }
        }
    }

    /// Write `messages` as they now are, when there is a store.
    fn keep_chat(&self, messages: Vec<ChatRecord>) {
        let Some(store) = self.storage.chat_store().cloned() else {
            return;
        };
        tokio::spawn(async move {
            for m in messages {
                if let Err(e) = store.put_chat(&m).await {
                    warn!(message = %m.id, error = %e, "A chat message could not be kept");
                }
            }
        });
    }

    /// Send `body`, an `fcp-chat` message, to the participant on `call_id`.
    fn tell_chat_body(&self, call_id: &str, body: String) {
        let Some(me) = self.me() else {
            debug!(call_id = %call_id, "No handler to send chat with");
            return;
        };
        let call_id = call_id.to_string();
        tokio::spawn(async move { me.send_chat_body(call_id, body).await });
    }

    /// Send `body`, an `fcp-chat` message, in the dialog of `call_id`.
    async fn send_chat_body(self: Arc<Self>, call_id: String, body: String) {
        self.send_package_body(
            call_id,
            fcp_core::CHAT_INFO_PACKAGE,
            fcp_core::CHAT_INFO_TYPE,
            body,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(id: usize, pid: &str, to: ChatAudience, deleted: bool) -> Held {
        Held {
            kept: ChatRecord {
                id: format!("m-{id}"),
                tenant_id: fcp_core::TenantId::new(),
                meeting_id: None,
                room_number: "1".into(),
                session_id: "s".into(),
                participant_id: pid.into(),
                sender_name: None,
                sender_address: None,
                audience: to,
                text: "x".repeat(400),
                sent_at: SystemTime::UNIX_EPOCH,
                deleted_at: deleted.then_some(SystemTime::UNIX_EPOCH),
                deleted_by: None,
            },
            sender_moderates: false,
        }
    }

    #[test]
    fn messages_are_sent_in_bodies_that_fit() {
        let viewer = Viewer {
            pid: None,
            moderates: true,
        };
        let held: Vec<Held> = (0..60)
            .map(|i| held(i, "p", ChatAudience::Room, false))
            .collect();
        let bodies = message_bodies(held.iter().map(|m| viewer.view(m)).collect());
        assert!(bodies.len() > 1);
        let mut ids = Vec::new();
        for body in &bodies {
            assert!(body.len() <= MAX_MESSAGES_BODY, "{} bytes", body.len());
            let ChatMessage::Messages { messages } = ChatMessage::parse(body.as_bytes()).unwrap()
            else {
                panic!("not a messages message");
            };
            ids.extend(messages.into_iter().map(|m| m.id));
        }
        assert_eq!(ids, (0..60).map(|i| format!("m-{i}")).collect::<Vec<_>>());
        assert!(message_bodies(Vec::new()).is_empty());
    }

    #[test]
    fn a_viewer_sees_the_room_and_their_own() {
        let ann = Viewer {
            pid: Some("ann"),
            moderates: false,
        };
        let host = Viewer {
            pid: Some("host"),
            moderates: true,
        };
        let to_room = held(1, "bob", ChatAudience::Room, false);
        let bobs_private = held(2, "bob", ChatAudience::Hosts, false);
        let anns_private = held(3, "ann", ChatAudience::Hosts, false);
        let deleted = held(4, "bob", ChatAudience::Room, true);
        assert!(ann.sees(&to_room));
        assert!(!ann.sees(&bobs_private) && !ann.told_of(&bobs_private));
        assert!(ann.sees(&anns_private) && ann.view(&anns_private).mine);
        assert!(host.sees(&bobs_private));
        // A deleted message is told once more, without its text, and not
        // seen again.
        assert!(!ann.sees(&deleted) && ann.told_of(&deleted));
        let v = ann.view(&deleted);
        assert!(v.deleted && v.text.is_empty());
    }

    #[test]
    fn the_backlog_is_the_last_hundred_a_viewer_sees() {
        let ann = Viewer {
            pid: Some("ann"),
            moderates: false,
        };
        let mut all: Vec<Held> = (0..150)
            .map(|i| held(i, "bob", ChatAudience::Room, false))
            .collect();
        all.push(held(150, "bob", ChatAudience::Hosts, false));
        all.push(held(151, "bob", ChatAudience::Room, true));
        let views = backlog(all.iter(), ann);
        assert_eq!(views.len(), CHAT_BACKLOG);
        assert_eq!(views.first().unwrap().id, "m-50");
        assert_eq!(views.last().unwrap().id, "m-149");
    }

    #[test]
    fn posts_are_limited_per_window() {
        let mut room = ChatRoom {
            session_id: "s".into(),
            meeting_id: None,
            scope: ChatScope::Everyone,
            messages: VecDeque::new(),
            changed: Vec::new(),
            fresh: HashSet::new(),
            muted: HashSet::new(),
            posts: HashMap::new(),
        };
        let t = Instant::now();
        for _ in 0..MAX_POSTS_PER_WINDOW {
            assert!(room.take_post("p", t));
        }
        assert!(!room.take_post("p", t));
        assert!(room.take_post("q", t));
        assert!(room.take_post("p", t + POST_WINDOW));
    }
}
