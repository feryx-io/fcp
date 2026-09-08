//! Conference recordings: what was recorded, where it is, and who may
//! reach it.
//!
//! A recording is a row first and a file second. The row is what makes a
//! recording addressable — by tenant, meeting and time rather than by a
//! filename on one node's disk — so it can be listed for a tenant,
//! downloaded through the admin API from whichever node holds it, and
//! deleted by a retention policy. The node that records writes the row
//! when it starts and completes it when it stops.
//!
//! See `docs/VIDEO_CONFERENCING.md` §11 and `docs/COMPLIANCE.md`.

use crate::{MeetingId, RecordingId, TenantId};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// The container a recording is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingFormat {
    /// Mixed audio only, as conference recordings have always been.
    #[default]
    Wav,
    /// The room's composite video and its mixed audio: VP8 or VP9 with
    /// Opus, in Matroska.
    Webm,
}

impl RecordingFormat {
    /// The file extension, without the dot.
    pub fn extension(&self) -> &'static str {
        match self {
            RecordingFormat::Wav => "wav",
            RecordingFormat::Webm => "webm",
        }
    }

    /// What a browser or player should be told the file is.
    pub fn content_type(&self) -> &'static str {
        match self {
            RecordingFormat::Wav => "audio/wav",
            RecordingFormat::Webm => "video/webm",
        }
    }

    /// Whether the container carries video.
    pub fn has_video(&self) -> bool {
        matches!(self, RecordingFormat::Webm)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingFormat::Wav => "wav",
            RecordingFormat::Webm => "webm",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "wav" => Some(RecordingFormat::Wav),
            "webm" => Some(RecordingFormat::Webm),
            _ => None,
        }
    }
}

/// Why a recording started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingTrigger {
    /// The meeting's `auto_record`, once enough participants were in.
    #[default]
    Auto,
    /// Asked for through the API.
    Api,
    /// A host's DTMF command.
    Dtmf,
}

impl RecordingTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingTrigger::Auto => "auto",
            RecordingTrigger::Api => "api",
            RecordingTrigger::Dtmf => "dtmf",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(RecordingTrigger::Auto),
            "api" => Some(RecordingTrigger::Api),
            "dtmf" => Some(RecordingTrigger::Dtmf),
            _ => None,
        }
    }
}

/// What the room was told, under the tenant's consent policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingConsent {
    /// The tenant's policy asks for no announcement.
    #[default]
    NotRequired,
    /// The room heard the announcement before recording began.
    Announced,
    /// An announcement was called for but could not be played; the
    /// recording went ahead and this says so.
    Failed,
}

impl RecordingConsent {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingConsent::NotRequired => "not_required",
            RecordingConsent::Announced => "announced",
            RecordingConsent::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "not_required" => Some(RecordingConsent::NotRequired),
            "announced" => Some(RecordingConsent::Announced),
            "failed" => Some(RecordingConsent::Failed),
            _ => None,
        }
    }
}

/// One conference recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub id: RecordingId,
    pub tenant_id: TenantId,
    /// The meeting, when the room is one; a room with no meeting behind
    /// it (a test room) records without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_id: Option<MeetingId>,
    pub room_number: String,
    /// The node holding the file: a download goes through it.
    pub node_id: String,
    /// The file, as that node sees it.
    pub path: String,
    pub format: RecordingFormat,
    pub trigger: RecordingTrigger,
    pub consent: RecordingConsent,
    pub started_at: SystemTime,
    /// Absent while the recording is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// Participants in the room when it started.
    #[serde(default)]
    pub participants: u32,
}

impl Recording {
    /// A recording that has just begun.
    pub fn starting(
        tenant_id: TenantId,
        room_number: impl Into<String>,
        node_id: impl Into<String>,
        path: impl Into<String>,
        format: RecordingFormat,
        trigger: RecordingTrigger,
    ) -> Self {
        Self {
            id: RecordingId::new(),
            tenant_id,
            meeting_id: None,
            room_number: room_number.into(),
            node_id: node_id.into(),
            path: path.into(),
            format,
            trigger,
            consent: RecordingConsent::NotRequired,
            started_at: SystemTime::now(),
            ended_at: None,
            duration_secs: None,
            size_bytes: None,
            participants: 0,
        }
    }

    /// Whether the recording is still being written.
    pub fn is_running(&self) -> bool {
        self.ended_at.is_none()
    }

    /// The file's name, without the directory.
    pub fn file_name(&self) -> &str {
        self.path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.path)
    }
}

/// Which recordings to list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordingFilter {
    pub room_number: Option<String>,
    pub meeting_id: Option<MeetingId>,
    /// Started at or after this.
    pub since: Option<SystemTime>,
    /// Started before this.
    pub until: Option<SystemTime>,
    /// Only recordings still being written, or only finished ones.
    pub running: Option<bool>,
    pub limit: usize,
    pub offset: usize,
}

impl RecordingFilter {
    /// The most recent `limit` recordings.
    pub fn recent(limit: usize) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }
}

/// Where recordings are registered.
#[async_trait::async_trait]
pub trait RecordingStore: Send + Sync {
    /// Register a recording that has started.
    async fn create_recording(&self, recording: &Recording) -> anyhow::Result<()>;

    /// Complete a recording: when it ended, how long it ran, how big it is.
    async fn finish_recording(
        &self,
        id: &RecordingId,
        ended_at: SystemTime,
        duration_secs: u64,
        size_bytes: u64,
    ) -> anyhow::Result<()>;

    async fn get_recording(&self, id: &RecordingId) -> anyhow::Result<Option<Recording>>;

    /// Recordings of one tenant, newest first.
    async fn list_recordings(
        &self,
        tenant_id: &TenantId,
        filter: &RecordingFilter,
    ) -> anyhow::Result<Vec<Recording>>;

    /// Forget a recording. The caller deletes the file.
    async fn delete_recording(&self, id: &RecordingId) -> anyhow::Result<bool>;

    /// Finished recordings of a tenant that started before `before`, for
    /// the retention sweep.
    async fn expired_recordings(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<Vec<Recording>>;

    /// Recordings a node still calls running — what it re-opens, or
    /// closes off, after a restart.
    async fn running_recordings(&self, node_id: &str) -> anyhow::Result<Vec<Recording>>;

    /// The tenants with recordings held by this node. The node's
    /// retention sweep walks these, since the days to keep are the
    /// tenant's to set.
    async fn node_tenants(&self, node_id: &str) -> anyhow::Result<Vec<TenantId>>;

    /// The recording this node holds at `path`, if the registry knows of
    /// one. The sweep asks before deleting a file by age, so a file that
    /// is a row's is left to the row's own retention.
    async fn recording_at(&self, node_id: &str, path: &str) -> anyhow::Result<Option<Recording>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_enums_round_trip_through_their_strings() {
        for f in [RecordingFormat::Wav, RecordingFormat::Webm] {
            assert_eq!(RecordingFormat::parse(f.as_str()), Some(f));
        }
        assert_eq!(RecordingFormat::parse("WebM"), Some(RecordingFormat::Webm));
        assert_eq!(RecordingFormat::parse("mp4"), None);
        assert_eq!(RecordingFormat::Webm.extension(), "webm");
        assert_eq!(RecordingFormat::Webm.content_type(), "video/webm");
        assert!(RecordingFormat::Webm.has_video());
        assert!(!RecordingFormat::Wav.has_video());

        for t in [
            RecordingTrigger::Auto,
            RecordingTrigger::Api,
            RecordingTrigger::Dtmf,
        ] {
            assert_eq!(RecordingTrigger::parse(t.as_str()), Some(t));
        }
        for c in [
            RecordingConsent::NotRequired,
            RecordingConsent::Announced,
            RecordingConsent::Failed,
        ] {
            assert_eq!(RecordingConsent::parse(c.as_str()), Some(c));
        }
    }

    #[test]
    fn a_starting_recording_is_running_and_names_its_file() {
        let r = Recording::starting(
            TenantId::new(),
            "8001",
            "conference-01",
            "/var/fcp/recordings/8001-123-abc.webm",
            RecordingFormat::Webm,
            RecordingTrigger::Api,
        );
        assert!(r.is_running());
        assert_eq!(r.file_name(), "8001-123-abc.webm");
        assert!(r.id.to_string().starts_with("rec-"));

        let bare = Recording {
            path: "room.wav".to_string(),
            ..r
        };
        assert_eq!(bare.file_name(), "room.wav");
    }

    #[test]
    fn a_recording_serializes_without_its_empty_fields() {
        let r = Recording::starting(
            TenantId::new(),
            "8001",
            "node",
            "/tmp/a.wav",
            RecordingFormat::Wav,
            RecordingTrigger::Auto,
        );
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"format\":\"wav\""));
        assert!(json.contains("\"trigger\":\"auto\""));
        assert!(json.contains("\"consent\":\"not_required\""));
        assert!(!json.contains("ended_at"), "{json}");
        assert!(!json.contains("meeting_id"), "{json}");
        let back: Recording = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}
