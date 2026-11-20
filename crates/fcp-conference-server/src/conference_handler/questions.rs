//! Questions from the audience (`docs/WEBINARS.md` §2.5, W3.2).
//!
//! Anyone in a room whose meeting takes questions (`qa` is `moderated` or
//! `open`) asks, and upvotes others' questions; a host or panelist
//! publishes, answers (in text, aloud, or both) and dismisses. A host or
//! panelist sees every question; anyone else the published ones and their
//! own. In a `moderated` room a question waits for a host to publish it;
//! in an `open` room it is published as it is asked. A written answer to a
//! question nobody else sees is an answer to its asker alone.
//!
//! A room's questions live here, on the node holding the room, for one
//! sitting: from the room's first participant to its last. Each is kept as
//! a `webinar_questions` row, written as it changes (upvotes at most once a
//! second). A participant who asked to be told (`Recv-Info: fcp-qa`) is
//! sent the questions they may see as they change, coalesced into one
//! `fcp-qa` INFO a second (`{"op":"questions",…}`), and all of them when
//! they join a room that has some.

use super::{ConferenceCallHandler, ConferenceRole, RoomKey};
use fcp_core::qa::{
    QaMessage, QaMode, Question, QuestionStatus, QuestionView, MAX_ANSWER_CHARS, MAX_QUESTION_CHARS,
};
use fcp_core::Meeting;
use fcp_events::ConferenceEvent;
use std::collections::HashSet;
use std::time::SystemTime;
use tracing::{debug, info, warn};

/// The most questions one participant may ask in a sitting.
pub const MAX_QUESTIONS_PER_PARTICIPANT: usize = 50;
/// The most questions a sitting of a room may hold.
pub const MAX_QUESTIONS_PER_ROOM: usize = 2000;
/// The largest `questions` INFO body sent: the rest go in the next one.
const MAX_QUESTIONS_BODY: usize = 8 * 1024;

/// Why a question could not be asked or moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QaError {
    /// The room takes no questions.
    Off,
    /// Nobody of that id or call in the room, or no such question for them.
    NotFound,
    /// Not theirs to do: a moderator's operation, one's own question's
    /// upvote, a dismissed question.
    Refused(&'static str),
    /// What was sent is not a question or an answer.
    Invalid(&'static str),
    /// Too many questions.
    TooMany,
}

impl QaError {
    /// The HTTP status for this error.
    pub fn status(&self) -> u16 {
        match self {
            QaError::Off | QaError::Refused(_) => 403,
            QaError::NotFound => 404,
            QaError::Invalid(_) => 400,
            QaError::TooMany => 429,
        }
    }
}

impl std::fmt::Display for QaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QaError::Off => f.write_str("this room takes no questions"),
            QaError::NotFound => f.write_str("no such question or participant in the room"),
            QaError::Refused(why) | QaError::Invalid(why) => f.write_str(why),
            QaError::TooMany => f.write_str("too many questions"),
        }
    }
}

impl std::error::Error for QaError {}

/// One question held by its room.
struct Held {
    kept: Question,
    /// Who upvoted it, by participant id.
    upvoters: HashSet<String>,
}

/// A room's questions for one sitting.
pub(super) struct QaRoom {
    session_id: String,
    meeting_id: Option<String>,
    mode: QaMode,
    anonymous_allowed: bool,
    questions: Vec<Held>,
    /// Questions changed since the last flush, by id.
    dirty: HashSet<String>,
    /// Calls owed every question they may see: they joined since.
    fresh: HashSet<String>,
}

impl QaRoom {
    fn new(meeting: &Meeting, session_id: String) -> Self {
        Self {
            session_id,
            meeting_id: Some(meeting.id.to_string()),
            mode: meeting.qa_mode(),
            anonymous_allowed: meeting.qa_anonymous,
            questions: Vec::new(),
            dirty: HashSet::new(),
            fresh: HashSet::new(),
        }
    }

    fn find(&mut self, id: &str) -> Option<&mut Held> {
        self.questions.iter_mut().find(|q| q.kept.id == id)
    }
}

/// Who a question is being shown to.
#[derive(Debug, Clone, Copy)]
struct Viewer<'a> {
    /// `None` for an administrator or host through the API.
    pid: Option<&'a str>,
    moderates: bool,
}

impl Viewer<'_> {
    /// Whether `q` is in the list this viewer sees: a moderator sees every
    /// question, anyone else their own and the published ones until they
    /// are dismissed.
    fn sees(&self, q: &Held) -> bool {
        self.moderates
            || (q.kept.status != QuestionStatus::Dismissed && (self.mine(q) || q.kept.published))
    }

    /// Whether this viewer is told of a change to `q`: they see it, or saw
    /// it before it was dismissed.
    fn told_of(&self, q: &Held) -> bool {
        self.moderates || self.mine(q) || q.kept.published
    }

    fn mine(&self, q: &Held) -> bool {
        self.pid.is_some_and(|p| p == q.kept.participant_id)
    }

    fn view(&self, q: &Held) -> QuestionView {
        QuestionView {
            id: q.kept.id.clone(),
            text: q.kept.text.clone(),
            asker: q.kept.asker_name.clone(),
            mine: self.mine(q),
            status: q.kept.status,
            published: q.kept.published,
            upvotes: q.upvoters.len() as u32,
            upvoted: self.pid.is_some_and(|p| q.upvoters.contains(p)),
            answer: q.kept.answer.clone(),
            answered_live: q.kept.answered_live,
            asked_at: chrono::DateTime::<chrono::Utc>::from(q.kept.asked_at)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        }
    }
}

/// `views` as `questions` INFO bodies of at most [`MAX_QUESTIONS_BODY`]
/// each (a single question larger than that goes alone).
fn question_bodies(views: Vec<QuestionView>) -> Vec<String> {
    let mut bodies = Vec::new();
    let mut batch: Vec<QuestionView> = Vec::new();
    let mut size = 0;
    for v in views {
        let len = serde_json::to_string(&v).map(|s| s.len()).unwrap_or(0) + 1;
        if !batch.is_empty() && size + len > MAX_QUESTIONS_BODY {
            bodies.push(
                QaMessage::Questions {
                    questions: std::mem::take(&mut batch),
                }
                .to_body(),
            );
            size = 0;
        }
        size += len;
        batch.push(v);
    }
    if !batch.is_empty() {
        bodies.push(QaMessage::Questions { questions: batch }.to_body());
    }
    bodies
}

/// A written answer, trimmed and checked; `None` for none.
fn answer_text(text: Option<&str>) -> Result<Option<String>, QaError> {
    let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if text.chars().count() > MAX_ANSWER_CHARS {
        return Err(QaError::Invalid("an answer is at most 1000 characters"));
    }
    Ok(Some(text.to_string()))
}

impl ConferenceCallHandler {
    /// A participant joined `key`'s room of `meeting`: the room's sitting
    /// takes the meeting's Q&A settings, and a participant who asked to be
    /// told is owed the questions already asked.
    pub(super) fn qa_joined(
        &self,
        key: &RoomKey,
        meeting: &Meeting,
        call_id: &str,
        qa_notices: bool,
    ) {
        let mut room = self
            .qa_rooms
            .entry(key.clone())
            .or_insert_with(|| QaRoom::new(meeting, self.sitting_id(key)));
        room.mode = meeting.qa_mode();
        room.anonymous_allowed = meeting.qa_anonymous;
        if qa_notices && !room.questions.is_empty() {
            room.fresh.insert(call_id.to_string());
        }
    }

    /// The room's sitting ended (its last participant left): what changed
    /// is written, and the questions let go.
    pub(super) fn qa_closed(&self, key: &RoomKey) {
        let Some((_, room)) = self.qa_rooms.remove(key) else {
            return;
        };
        let changed: Vec<Question> = room
            .questions
            .iter()
            .filter(|q| room.dirty.contains(&q.kept.id))
            .map(Self::counted)
            .collect();
        self.keep_questions(changed);
    }

    /// The call and whether they moderate, of `participant_id` in `key`'s room.
    fn qa_participant(&self, key: &RoomKey, participant_id: &str) -> Option<(String, bool)> {
        let calls = self.room_sessions.get(key)?.clone();
        calls.into_iter().find_map(|c| {
            let s = self.sessions.get(&c)?;
            (s.participant_id == participant_id && s.trunk_peer.is_none())
                .then(|| (c.clone(), s.is_host || s.role == ConferenceRole::Panelist))
        })
    }

    /// `participant_id` asks a question.
    pub fn ask_question(
        &self,
        key: &RoomKey,
        participant_id: &str,
        text: &str,
        anonymous: bool,
    ) -> Result<QuestionView, QaError> {
        let (call_id, moderates) = self
            .qa_participant(key, participant_id)
            .ok_or(QaError::NotFound)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(QaError::Invalid("a question has text"));
        }
        if text.chars().count() > MAX_QUESTION_CHARS {
            return Err(QaError::Invalid("a question is at most 500 characters"));
        }
        // The name the room shows them by: their display name, else
        // their caller id.
        let display_name = self
            .sessions
            .get(&call_id)
            .and_then(|s| s.display_name.clone().or_else(|| s.caller_id.clone()));
        // Where they called from, kept even for an anonymous question so
        // a hold or a subject request finds it (`docs/COMPLIANCE.md`).
        let asker_address = self
            .sessions
            .get(&call_id)
            .map(|s| s.record.from_uri.clone());
        let now = SystemTime::now();
        let (kept, view) = {
            let mut room = self.qa_rooms.get_mut(key).ok_or(QaError::Off)?;
            if room.mode == QaMode::Off {
                return Err(QaError::Off);
            }
            if anonymous && !room.anonymous_allowed {
                return Err(QaError::Refused("this room takes no anonymous questions"));
            }
            if room.questions.len() >= MAX_QUESTIONS_PER_ROOM
                || room
                    .questions
                    .iter()
                    .filter(|q| q.kept.participant_id == participant_id)
                    .count()
                    >= MAX_QUESTIONS_PER_PARTICIPANT
            {
                return Err(QaError::TooMany);
            }
            let held = Held {
                kept: Question {
                    id: format!("q-{}", uuid::Uuid::new_v4()),
                    tenant_id: key.tenant,
                    meeting_id: room.meeting_id.clone(),
                    room_number: key.room.clone(),
                    session_id: room.session_id.clone(),
                    participant_id: participant_id.to_string(),
                    asker_name: if anonymous { None } else { display_name },
                    asker_address,
                    text: text.to_string(),
                    status: QuestionStatus::Pending,
                    published: room.mode == QaMode::Open,
                    upvotes: 0,
                    answer: None,
                    answered_live: false,
                    asked_at: now,
                    updated_at: now,
                },
                upvoters: HashSet::new(),
            };
            let view = Viewer {
                pid: Some(participant_id),
                moderates,
            }
            .view(&held);
            let kept = held.kept.clone();
            room.dirty.insert(kept.id.clone());
            room.questions.push(held);
            (kept, view)
        };
        info!(room = %key.room, participant = %participant_id, question = %kept.id, anonymous, "A question asked");
        self.publish_event(ConferenceEvent::QuestionAsked {
            room_number: key.room.clone(),
            question_id: kept.id.clone(),
            participant_id: participant_id.to_string(),
            anonymous,
            tenant_id: key.tenant,
            timestamp: now,
        });
        self.keep_questions(vec![kept]);
        Ok(view)
    }

    /// `participant_id` upvotes question `id` (`on`) or takes it back.
    /// Whether anything changed.
    pub fn upvote_question(
        &self,
        key: &RoomKey,
        participant_id: &str,
        id: &str,
        on: bool,
    ) -> Result<bool, QaError> {
        let (_, moderates) = self
            .qa_participant(key, participant_id)
            .ok_or(QaError::NotFound)?;
        let mut room = self.qa_rooms.get_mut(key).ok_or(QaError::NotFound)?;
        let viewer = Viewer {
            pid: Some(participant_id),
            moderates,
        };
        let q = room.find(id).ok_or(QaError::NotFound)?;
        if !viewer.sees(q) {
            return Err(QaError::NotFound);
        }
        if viewer.mine(q) {
            return Err(QaError::Refused("one's own question is not upvoted"));
        }
        if q.kept.status == QuestionStatus::Dismissed {
            return Err(QaError::Refused("the question was dismissed"));
        }
        let changed = if on {
            q.upvoters.insert(participant_id.to_string())
        } else {
            q.upvoters.remove(participant_id)
        };
        if changed {
            room.dirty.insert(id.to_string());
        }
        Ok(changed)
    }

    /// A host or panelist moves question `id`: publishes it, answers it, or
    /// dismisses it. `by` is the participant doing it, when they are in the
    /// room; a host through the API has been checked by the API. The
    /// question as a moderator sees it.
    pub fn moderate_question(
        &self,
        key: &RoomKey,
        by: Option<&str>,
        id: &str,
        action: Moderation<'_>,
    ) -> Result<QuestionView, QaError> {
        if let Some(pid) = by {
            let (_, moderates) = self.qa_participant(key, pid).ok_or(QaError::NotFound)?;
            if !moderates {
                return Err(QaError::Refused(
                    "only a host or panelist moderates questions",
                ));
            }
        }
        let now = SystemTime::now();
        let (kept, view, event) = {
            let mut room = self.qa_rooms.get_mut(key).ok_or(QaError::NotFound)?;
            let q = room.find(id).ok_or(QaError::NotFound)?;
            let event = match action {
                Moderation::Publish => {
                    if q.kept.status == QuestionStatus::Dismissed {
                        return Err(QaError::Refused("the question was dismissed"));
                    }
                    if q.kept.published {
                        None
                    } else {
                        q.kept.published = true;
                        Some(ConferenceEvent::QuestionPublished {
                            room_number: key.room.clone(),
                            question_id: id.to_string(),
                            tenant_id: key.tenant,
                            timestamp: now,
                        })
                    }
                }
                Moderation::Answer { text, live } => {
                    if q.kept.status == QuestionStatus::Dismissed {
                        return Err(QaError::Refused("the question was dismissed"));
                    }
                    let text = answer_text(text)?;
                    if text.is_none() && !live {
                        return Err(QaError::Invalid("an answer has text or is live"));
                    }
                    if text.is_some() {
                        q.kept.answer = text;
                    }
                    q.kept.answered_live |= live;
                    q.kept.status = QuestionStatus::Answered;
                    Some(ConferenceEvent::QuestionAnswered {
                        room_number: key.room.clone(),
                        question_id: id.to_string(),
                        live,
                        tenant_id: key.tenant,
                        timestamp: now,
                    })
                }
                Moderation::Dismiss => {
                    if q.kept.status == QuestionStatus::Dismissed {
                        None
                    } else {
                        q.kept.status = QuestionStatus::Dismissed;
                        Some(ConferenceEvent::QuestionDismissed {
                            room_number: key.room.clone(),
                            question_id: id.to_string(),
                            tenant_id: key.tenant,
                            timestamp: now,
                        })
                    }
                }
            };
            if event.is_some() {
                q.kept.updated_at = now;
            }
            let kept = Self::counted(q);
            let view = Viewer {
                pid: by,
                moderates: true,
            }
            .view(q);
            if event.is_some() {
                room.dirty.insert(id.to_string());
            }
            (kept, view, event)
        };
        if let Some(event) = event {
            info!(room = %key.room, question = %id, action = action.name(), "A question moderated");
            self.publish_event(event);
            self.keep_questions(vec![kept]);
        }
        Ok(view)
    }

    /// The questions of `key`'s room as `participant_id` sees them, or as
    /// a moderator does when `None`, in the order asked.
    pub fn questions(
        &self,
        key: &RoomKey,
        participant_id: Option<&str>,
    ) -> Result<Vec<QuestionView>, QaError> {
        let viewer = match participant_id {
            Some(pid) => {
                let (_, moderates) = self.qa_participant(key, pid).ok_or(QaError::NotFound)?;
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
        let Some(room) = self.qa_rooms.get(key) else {
            return Ok(Vec::new());
        };
        Ok(room
            .questions
            .iter()
            .filter(|q| viewer.sees(q))
            .map(|q| viewer.view(q))
            .collect())
    }

    /// A participant's `fcp-qa` INFO about questions.
    pub(super) fn take_question_info(
        &self,
        key: &RoomKey,
        participant_id: &str,
        message: QaMessage,
    ) -> Result<(), QaError> {
        match message {
            QaMessage::Ask { text, anonymous } => self
                .ask_question(key, participant_id, &text, anonymous)
                .map(|_| ()),
            QaMessage::Upvote { id, on } => self
                .upvote_question(key, participant_id, &id, on)
                .map(|_| ()),
            QaMessage::Publish { id } => self
                .moderate_question(key, Some(participant_id), &id, Moderation::Publish)
                .map(|_| ()),
            QaMessage::Answer { id, text, live } => self
                .moderate_question(
                    key,
                    Some(participant_id),
                    &id,
                    Moderation::Answer {
                        text: text.as_deref(),
                        live,
                    },
                )
                .map(|_| ()),
            QaMessage::Dismiss { id } => self
                .moderate_question(key, Some(participant_id), &id, Moderation::Dismiss)
                .map(|_| ()),
            QaMessage::Hand { .. }
            | QaMessage::Stage { .. }
            | QaMessage::Questions { .. }
            | QaMessage::Webinar { .. } => Err(QaError::Refused("only the room sends that")),
        }
    }

    /// Tell each participant who asked to be told of the questions that
    /// changed (all of them to one who just joined), and keep the changed
    /// rows. Run once a second.
    pub fn flush_questions(&self) {
        let keys: Vec<RoomKey> = self
            .qa_rooms
            .iter()
            .filter(|r| !r.dirty.is_empty() || !r.fresh.is_empty())
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
                    (s.qa_notices && s.trunk_peer.is_none()).then(|| {
                        (
                            c.clone(),
                            s.participant_id.clone(),
                            s.is_host || s.role == ConferenceRole::Panelist,
                        )
                    })
                })
                .collect();
            let (sends, changed) = {
                let Some(mut room) = self.qa_rooms.get_mut(&key) else {
                    continue;
                };
                let dirty = std::mem::take(&mut room.dirty);
                let fresh = std::mem::take(&mut room.fresh);
                let mut sends = Vec::new();
                for (call_id, pid, moderates) in &told {
                    let viewer = Viewer {
                        pid: Some(pid),
                        moderates: *moderates,
                    };
                    let views: Vec<QuestionView> = if fresh.contains(call_id) {
                        room.questions
                            .iter()
                            .filter(|q| viewer.sees(q))
                            .map(|q| viewer.view(q))
                            .collect()
                    } else {
                        room.questions
                            .iter()
                            .filter(|q| dirty.contains(&q.kept.id) && viewer.told_of(q))
                            .map(|q| viewer.view(q))
                            .collect()
                    };
                    if !views.is_empty() {
                        sends.push((call_id.clone(), views));
                    }
                }
                let changed: Vec<Question> = room
                    .questions
                    .iter()
                    .filter(|q| dirty.contains(&q.kept.id))
                    .map(Self::counted)
                    .collect();
                (sends, changed)
            };
            for (call_id, views) in sends {
                for body in question_bodies(views) {
                    self.tell_qa_body(&call_id, body);
                }
            }
            self.keep_questions(changed);
        }
    }

    /// A held question's row with its upvotes counted.
    fn counted(q: &Held) -> Question {
        let mut kept = q.kept.clone();
        kept.upvotes = q.upvoters.len() as u32;
        kept
    }

    /// Write `questions` as they now are, when there is a store.
    fn keep_questions(&self, questions: Vec<Question>) {
        if questions.is_empty() {
            return;
        }
        let Some(store) = self.storage.question_store().cloned() else {
            return;
        };
        tokio::spawn(async move {
            for q in questions {
                if let Err(e) = store.put_question(&q).await {
                    warn!(question = %q.id, error = %e, "A question could not be kept");
                }
            }
        });
    }

    /// Send `body`, an `fcp-qa` message, to the participant on `call_id`.
    fn tell_qa_body(&self, call_id: &str, body: String) {
        let Some(me) = self.me() else {
            debug!(call_id = %call_id, "No handler to send questions with");
            return;
        };
        let call_id = call_id.to_string();
        tokio::spawn(async move { me.send_qa_body(call_id, body).await });
    }
}

/// What a host or panelist does to a question.
#[derive(Debug, Clone, Copy)]
pub enum Moderation<'a> {
    Publish,
    Answer { text: Option<&'a str>, live: bool },
    Dismiss,
}

impl Moderation<'_> {
    fn name(&self) -> &'static str {
        match self {
            Moderation::Publish => "publish",
            Moderation::Answer { .. } => "answer",
            Moderation::Dismiss => "dismiss",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(id: usize, text_len: usize) -> QuestionView {
        QuestionView {
            id: format!("q-{id}"),
            text: "x".repeat(text_len),
            asker: None,
            mine: false,
            status: QuestionStatus::Pending,
            published: true,
            upvotes: 0,
            upvoted: false,
            answer: None,
            answered_live: false,
            asked_at: "2026-11-17T10:00:00.000Z".into(),
        }
    }

    #[test]
    fn questions_are_sent_in_bodies_that_fit() {
        let bodies = question_bodies((0..60).map(|i| view(i, 400)).collect());
        assert!(
            bodies.len() > 1,
            "sixty long questions take more than one body"
        );
        let mut ids = Vec::new();
        for body in &bodies {
            assert!(body.len() <= MAX_QUESTIONS_BODY, "{} bytes", body.len());
            let QaMessage::Questions { questions } = QaMessage::parse(body.as_bytes()).unwrap()
            else {
                panic!("not a questions message");
            };
            ids.extend(questions.into_iter().map(|q| q.id));
        }
        assert_eq!(ids, (0..60).map(|i| format!("q-{i}")).collect::<Vec<_>>());
        assert!(question_bodies(Vec::new()).is_empty());
    }

    #[test]
    fn an_answer_is_trimmed_and_bounded() {
        assert_eq!(answer_text(None), Ok(None));
        assert_eq!(answer_text(Some("  ")), Ok(None));
        assert_eq!(answer_text(Some(" Yes. ")), Ok(Some("Yes.".into())));
        assert!(answer_text(Some(&"y".repeat(MAX_ANSWER_CHARS + 1))).is_err());
    }
}
