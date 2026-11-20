//! The body of an `fcp-chat` INFO (`docs/WEBINARS.md` §2.5, W3b): chat in
//! every room, meetings as well as webinars.
//!
//! A participant posts to the room, or privately to its hosts and
//! panelists:
//!
//! ```json
//! { "op": "post", "text": "Hello", "to": "room" }
//! { "op": "post", "text": "Can you share the slides?", "to": "hosts" }
//! ```
//!
//! A host or panelist deletes any message, which is then gone for
//! everyone, and mutes one participant's chat for the rest of the sitting:
//!
//! ```json
//! { "op": "delete", "id": "m-…" }
//! { "op": "mute", "participant_id": "…", "on": true }
//! ```
//!
//! A participant whose INVITE said `Recv-Info: fcp-chat` is sent the
//! messages they may see as they arrive, a few times a second, and the last
//! 100 of them when they join a room that has some:
//!
//! ```json
//! { "op": "messages", "messages": [ { "id": "m-…", "text": "…", … } ] }
//! ```
//!
//! A deleted message is sent once more, `deleted` and without its text, to
//! those who saw it. A muted participant is told so:
//!
//! ```json
//! { "op": "muted", "on": true }
//! ```
//!
//! Who may post where is the meeting's `chat` ([`ChatScope`]); hosts and
//! panelists always have their own channel (`"to": "hosts"`).

use serde::{Deserialize, Serialize};

/// One `fcp-chat` message, either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatMessage {
    /// A message posted to the room or to its hosts.
    Post {
        text: String,
        #[serde(default)]
        to: ChatAudience,
    },
    /// A host or panelist deletes a message.
    Delete { id: String },
    /// A host or panelist mutes (`on`) or unmutes a participant's chat.
    Mute { participant_id: String, on: bool },
    /// The room's word of new or deleted messages, as they are for the
    /// participant told. Only the room sends it.
    Messages { messages: Vec<ChatView> },
    /// The participant's chat was muted (`on`) or unmuted. Only the room
    /// sends it.
    Muted { on: bool },
}

/// Who a message is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ChatAudience {
    /// Everyone in the room.
    #[default]
    Room,
    /// The room's hosts and panelists, and its sender.
    Hosts,
}

impl ChatAudience {
    pub fn as_str(self) -> &'static str {
        match self {
            ChatAudience::Room => "room",
            ChatAudience::Hosts => "hosts",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "room" => Some(ChatAudience::Room),
            "hosts" => Some(ChatAudience::Hosts),
            _ => None,
        }
    }
}

/// Who may chat in a room (a meeting's `chat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatScope {
    /// Anyone posts to the room or to the hosts.
    Everyone,
    /// An attendee or guest posts to the hosts only; hosts and panelists
    /// post to the room as well.
    Hosts,
    /// Nobody posts to the room; hosts and panelists keep their own
    /// channel.
    Off,
}

impl ChatScope {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "everyone" => Some(ChatScope::Everyone),
            "hosts" => Some(ChatScope::Hosts),
            "off" => Some(ChatScope::Off),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ChatScope::Everyone => "everyone",
            ChatScope::Hosts => "hosts",
            ChatScope::Off => "off",
        }
    }

    /// Whether someone who moderates (`moderates`) or not may post to `to`
    /// in a room of this scope.
    pub fn allows(self, moderates: bool, to: ChatAudience) -> bool {
        match (self, to) {
            // Hosts and panelists always have their own channel.
            (_, ChatAudience::Hosts) if moderates => true,
            (ChatScope::Off, _) => false,
            (ChatScope::Everyone, _) => true,
            (ChatScope::Hosts, ChatAudience::Hosts) => true,
            (ChatScope::Hosts, ChatAudience::Room) => moderates,
        }
    }
}

/// The longest message, in characters.
pub const MAX_CHAT_CHARS: usize = 1000;
/// How many messages a participant joining late is sent.
pub const CHAT_BACKLOG: usize = 100;

/// A message as one participant sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ChatView {
    /// `m-<uuid>`.
    pub id: String,
    /// Empty once deleted.
    pub text: String,
    /// The sender's name, or `null` when they have none.
    pub sender: Option<String>,
    /// The sender's participant id in this sitting.
    pub sender_id: String,
    /// Sent by a host or panelist.
    pub sender_moderates: bool,
    /// Sent by the participant told.
    pub mine: bool,
    pub to: ChatAudience,
    /// Deleted by a host or panelist: drop it.
    pub deleted: bool,
    /// When it was posted, RFC 3339.
    pub sent_at: String,
}

/// A message as it is kept (`conference_chat`), one sitting of a room.
/// Serialized between the nodes of a webinar's tree (`docs/WEBINARS.md`
/// §2.7), never by the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRecord {
    pub id: String,
    pub tenant_id: crate::TenantId,
    pub meeting_id: Option<String>,
    pub room_number: String,
    /// One sitting of the room: from its first participant to its last.
    pub session_id: String,
    /// The sender's participant id in that sitting.
    pub participant_id: String,
    /// The sender's name as the room showed it.
    pub sender_name: Option<String>,
    /// Where the sender called from (their From URI), for a subject
    /// request to find them by.
    pub sender_address: Option<String>,
    pub audience: ChatAudience,
    pub text: String,
    pub sent_at: std::time::SystemTime,
    /// When a host or panelist deleted it, and who (a participant id, or
    /// `None` through the API).
    pub deleted_at: Option<std::time::SystemTime>,
    pub deleted_by: Option<String>,
}

/// Where chat is kept.
#[async_trait::async_trait]
pub trait ChatStore: Send + Sync {
    /// Write a message as it now is.
    async fn put_chat(&self, message: &ChatRecord) -> anyhow::Result<()>;
    /// A room's messages in the tenant, oldest first, at most `limit`.
    async fn chat_of_room(
        &self,
        tenant: &crate::TenantId,
        room_number: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ChatRecord>>;
    /// One sitting's messages in the tenant, oldest first, at most `limit`
    /// (`docs/WEBINARS.md` §2.6: a webinar's attendance).
    async fn chat_of_session(
        &self,
        tenant: &crate::TenantId,
        session_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ChatRecord>>;
}

impl ChatMessage {
    /// Read an INFO's body. What it means and who may send it is the
    /// room's to judge.
    pub fn parse(body: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(body).map_err(|e| format!("not an fcp-chat message: {e}"))
    }

    /// The body to send.
    pub fn to_body(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_read_as_they_are_written() {
        assert_eq!(
            ChatMessage::parse(br#"{"op":"post","text":"Hi"}"#).unwrap(),
            ChatMessage::Post {
                text: "Hi".into(),
                to: ChatAudience::Room
            }
        );
        for msg in [
            ChatMessage::Post {
                text: "Hi".into(),
                to: ChatAudience::Hosts,
            },
            ChatMessage::Delete { id: "m-1".into() },
            ChatMessage::Mute {
                participant_id: "p".into(),
                on: true,
            },
            ChatMessage::Muted { on: false },
            ChatMessage::Messages {
                messages: vec![ChatView {
                    id: "m-1".into(),
                    text: "Hi".into(),
                    sender: Some("Ann".into()),
                    sender_id: "p".into(),
                    sender_moderates: false,
                    mine: true,
                    to: ChatAudience::Room,
                    deleted: false,
                    sent_at: "2026-11-18T10:00:00.000Z".into(),
                }],
            },
        ] {
            assert_eq!(ChatMessage::parse(msg.to_body().as_bytes()).unwrap(), msg);
        }
        assert!(ChatMessage::parse(br#"{"op":"post","text":"Hi","to":"all"}"#).is_err());
        assert!(ChatMessage::parse(br#"{"op":"mute","participant_id":"p"}"#).is_err());
        assert!(ChatMessage::parse(br#"{"op":"post","text":"Hi","x":1}"#).is_err());
    }

    #[test]
    fn a_scope_says_who_posts_where() {
        use ChatAudience::{Hosts, Room};
        for s in [ChatScope::Everyone, ChatScope::Hosts, ChatScope::Off] {
            assert_eq!(ChatScope::parse(s.as_str()), Some(s));
            // Hosts and panelists always have their own channel.
            assert!(s.allows(true, Hosts));
        }
        assert!(ChatScope::Everyone.allows(false, Room));
        assert!(ChatScope::Everyone.allows(false, Hosts));
        assert!(!ChatScope::Hosts.allows(false, Room));
        assert!(ChatScope::Hosts.allows(false, Hosts));
        assert!(ChatScope::Hosts.allows(true, Room));
        assert!(!ChatScope::Off.allows(false, Hosts));
        assert!(!ChatScope::Off.allows(true, Room));
        assert_eq!(ChatScope::parse("Everyone"), None);
    }
}
