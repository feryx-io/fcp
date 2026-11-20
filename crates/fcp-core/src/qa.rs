//! The body of an `fcp-qa` INFO (`docs/WEBINARS.md` §2.5): the audience's
//! voice in a room — a raised hand, being brought to the stage, and
//! questions.
//!
//! A participant sends the room its hand:
//!
//! ```json
//! { "op": "hand", "raised": true }
//! ```
//!
//! and a participant whose INVITE said `Recv-Info: fcp-qa` is told what
//! the room did to them:
//!
//! ```json
//! { "op": "hand", "raised": false }
//! { "op": "stage", "on": true }
//! ```
//!
//! A hand comes down when its owner lowers it, a host lowers it (or all
//! of them), or a host brings its owner to the stage. Being brought to the
//! stage means being heard by the room; being sent back means listening
//! again.
//!
//! Questions (W3.2). Anyone in the room asks and upvotes others'; a host
//! or panelist publishes, answers and dismisses:
//!
//! ```json
//! { "op": "ask", "text": "When does it ship?", "anonymous": false }
//! { "op": "upvote", "id": "q-…", "on": true }
//! { "op": "publish", "id": "q-…" }
//! { "op": "answer", "id": "q-…", "text": "In May.", "live": false }
//! { "op": "dismiss", "id": "q-…" }
//! ```
//!
//! The room tells each participant who asked to be told of the questions
//! they may see as they change, at most once a second, as they are for
//! them (`mine`, `upvoted`):
//!
//! ```json
//! { "op": "questions", "questions": [ { "id": "q-…", "text": "…", … } ] }
//! ```
//!
//! A host or panelist sees every question; anyone else the published ones
//! and their own. A dismissed question is sent once more, `dismissed`, to
//! those who saw it, so they can drop it.
//!
//! A webinar's phase (W4, [`crate::webinar`]). The room tells each
//! participant who asked when they join and as it changes, and whether
//! they are waiting in the lobby:
//!
//! ```json
//! { "op": "webinar", "phase": "practice", "lobby": true }
//! { "op": "webinar", "phase": "live", "lobby": false }
//! ```

use serde::{Deserialize, Serialize};

/// One `fcp-qa` message, either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum QaMessage {
    /// A hand raised or lowered: by its owner toward the room, or by the
    /// room toward its owner.
    Hand { raised: bool },
    /// The room brought the participant to the stage (`on`) or sent them
    /// back to the audience. Only the room sends it.
    Stage { on: bool },
    /// A question asked, without the asker's name when `anonymous` (and the
    /// meeting allows it).
    Ask {
        text: String,
        #[serde(default)]
        anonymous: bool,
    },
    /// An upvote given (`on`) or taken back.
    Upvote { id: String, on: bool },
    /// A host or panelist shows a question to the room.
    Publish { id: String },
    /// A host or panelist answers a question: in text, or `live` (answered
    /// aloud), or both.
    Answer {
        id: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        live: bool,
    },
    /// A host or panelist dismisses a question.
    Dismiss { id: String },
    /// The room's word of the questions that changed, as they are for the
    /// participant told. Only the room sends it.
    Questions { questions: Vec<QuestionView> },
    /// A webinar's phase, and whether the participant told is waiting in
    /// the lobby. Only the room sends it.
    Webinar {
        phase: crate::webinar::WebinarPhase,
        #[serde(default)]
        lobby: bool,
    },
}

/// How a room takes questions (a meeting's `qa`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QaMode {
    /// No questions.
    Off,
    /// A question is seen by the room once a host or panelist publishes it.
    Moderated,
    /// A question is seen by the room as soon as it is asked.
    Open,
}

impl QaMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(QaMode::Off),
            "moderated" => Some(QaMode::Moderated),
            "open" => Some(QaMode::Open),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            QaMode::Off => "off",
            QaMode::Moderated => "moderated",
            QaMode::Open => "open",
        }
    }
}

/// Where a question is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum QuestionStatus {
    /// Asked, not yet answered or dismissed.
    Pending,
    /// Answered, in text or aloud.
    Answered,
    /// Dismissed by a host or panelist.
    Dismissed,
}

impl QuestionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            QuestionStatus::Pending => "pending",
            QuestionStatus::Answered => "answered",
            QuestionStatus::Dismissed => "dismissed",
        }
    }
}

/// The longest question, in characters.
pub const MAX_QUESTION_CHARS: usize = 500;
/// The longest written answer, in characters.
pub const MAX_ANSWER_CHARS: usize = 1000;

/// A question as one participant sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct QuestionView {
    /// `q-<uuid>`.
    pub id: String,
    pub text: String,
    /// The asker's name, or `null` when asked anonymously or nameless.
    pub asker: Option<String>,
    /// Asked by the participant told.
    pub mine: bool,
    pub status: QuestionStatus,
    /// Seen by the whole room.
    pub published: bool,
    pub upvotes: u32,
    /// Upvoted by the participant told.
    pub upvoted: bool,
    /// The written answer, if any.
    pub answer: Option<String>,
    /// Answered aloud.
    pub answered_live: bool,
    /// When it was asked, RFC 3339.
    pub asked_at: String,
}

/// A question as it is kept (`webinar_questions`), one sitting of a room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub tenant_id: crate::TenantId,
    pub meeting_id: Option<String>,
    pub room_number: String,
    /// One sitting of the room: from its first question to its end.
    pub session_id: String,
    /// The asker's participant id in that sitting.
    pub participant_id: String,
    /// `None` when asked anonymously or nameless.
    pub asker_name: Option<String>,
    pub text: String,
    pub status: QuestionStatus,
    pub published: bool,
    pub upvotes: u32,
    pub answer: Option<String>,
    pub answered_live: bool,
    pub asked_at: std::time::SystemTime,
    pub updated_at: std::time::SystemTime,
}

/// Where questions are kept.
#[async_trait::async_trait]
pub trait QuestionStore: Send + Sync {
    /// Write a question as it now is.
    async fn put_question(&self, question: &Question) -> anyhow::Result<()>;
    /// A room's questions in the tenant, oldest first, at most `limit`.
    async fn questions_of_room(
        &self,
        tenant: &crate::TenantId,
        room_number: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Question>>;
}

impl QaMessage {
    /// Read an INFO's body. What it means and who may send it is the
    /// room's to judge.
    pub fn parse(body: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(body).map_err(|e| format!("not an fcp-qa message: {e}"))
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
        for (body, msg) in [
            (
                r#"{"op":"hand","raised":true}"#,
                QaMessage::Hand { raised: true },
            ),
            (
                r#"{"op":"stage","on":false}"#,
                QaMessage::Stage { on: false },
            ),
        ] {
            assert_eq!(QaMessage::parse(body.as_bytes()).unwrap(), msg);
            assert_eq!(msg.to_body(), body);
        }
        assert!(QaMessage::parse(br#"{"op":"hand"}"#).is_err());
        assert!(QaMessage::parse(br#"{"op":"wave","raised":true}"#).is_err());
        assert!(QaMessage::parse(br#"{"op":"hand","raised":true,"x":1}"#).is_err());
    }

    #[test]
    fn question_messages_read_as_they_are_written() {
        assert_eq!(
            QaMessage::parse(br#"{"op":"ask","text":"Why?"}"#).unwrap(),
            QaMessage::Ask {
                text: "Why?".into(),
                anonymous: false
            }
        );
        assert_eq!(
            QaMessage::parse(br#"{"op":"answer","id":"q-1","live":true}"#).unwrap(),
            QaMessage::Answer {
                id: "q-1".into(),
                text: None,
                live: true
            }
        );
        for msg in [
            QaMessage::Upvote {
                id: "q-1".into(),
                on: true,
            },
            QaMessage::Publish { id: "q-1".into() },
            QaMessage::Dismiss { id: "q-1".into() },
            QaMessage::Questions {
                questions: vec![QuestionView {
                    id: "q-1".into(),
                    text: "Why?".into(),
                    asker: None,
                    mine: true,
                    status: QuestionStatus::Answered,
                    published: true,
                    upvotes: 3,
                    upvoted: false,
                    answer: Some("Because.".into()),
                    answered_live: false,
                    asked_at: "2026-11-17T10:00:00.000Z".into(),
                }],
            },
        ] {
            assert_eq!(QaMessage::parse(msg.to_body().as_bytes()).unwrap(), msg);
        }
        assert!(QaMessage::parse(br#"{"op":"upvote","id":"q-1"}"#).is_err());
    }

    #[test]
    fn a_mode_reads_its_own_names_only() {
        for m in [QaMode::Off, QaMode::Moderated, QaMode::Open] {
            assert_eq!(QaMode::parse(m.as_str()), Some(m));
        }
        assert_eq!(QaMode::parse("Open"), None);
    }
}
