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
    /// The composite and the mix as fragmented MP4: H.264, HEVC or AV1
    /// with Opus — what a GPU node records on its encoder, and the
    /// container every player opens (design §15.9).
    Mp4,
}

impl RecordingFormat {
    /// The file extension, without the dot.
    pub fn extension(&self) -> &'static str {
        match self {
            RecordingFormat::Wav => "wav",
            RecordingFormat::Webm => "webm",
            RecordingFormat::Mp4 => "mp4",
        }
    }

    /// What a browser or player should be told the file is.
    pub fn content_type(&self) -> &'static str {
        match self {
            RecordingFormat::Wav => "audio/wav",
            RecordingFormat::Webm => "video/webm",
            RecordingFormat::Mp4 => "video/mp4",
        }
    }

    /// Whether the container carries video.
    pub fn has_video(&self) -> bool {
        matches!(self, RecordingFormat::Webm | RecordingFormat::Mp4)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingFormat::Wav => "wav",
            RecordingFormat::Webm => "webm",
            RecordingFormat::Mp4 => "mp4",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "wav" => Some(RecordingFormat::Wav),
            "webm" => Some(RecordingFormat::Webm),
            "mp4" => Some(RecordingFormat::Mp4),
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

/// What the parties were told, under the tenant's consent policy.
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
    /// An announcement was called for and could not be played, and the
    /// tenant's policy says not to record without one: nothing was recorded.
    Refused,
}

impl RecordingConsent {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingConsent::NotRequired => "not_required",
            RecordingConsent::Announced => "announced",
            RecordingConsent::Failed => "failed",
            RecordingConsent::Refused => "refused",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "not_required" => Some(RecordingConsent::NotRequired),
            "announced" => Some(RecordingConsent::Announced),
            "failed" => Some(RecordingConsent::Failed),
            "refused" => Some(RecordingConsent::Refused),
            _ => None,
        }
    }
}

/// One conference recording.
/// What a conference recording captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingKind {
    /// The room's mix.
    #[default]
    Room,
    /// One participant's own audio.
    Participant,
    /// A webinar's stream out (`docs/WEBINARS.md` §2.8): its path is the
    /// stream's directory, whose files read back in order are one
    /// fragmented MP4.
    Stream,
}

impl RecordingKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingKind::Room => "room",
            RecordingKind::Participant => "participant",
            RecordingKind::Stream => "stream",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "room" => Some(RecordingKind::Room),
            "participant" => Some(RecordingKind::Participant),
            "stream" => Some(RecordingKind::Stream),
            _ => None,
        }
    }
}

/// Someone who was in the room while it was recorded, so a subject's voice
/// can be found (`docs/COMPLIANCE_FOLLOWUPS.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingParticipant {
    pub participant_id: String,
    /// The caller's address, or `webrtc:<session>` for a browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub joined_at: SystemTime,
}

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
    /// The site holding the file, at the cloud: the row came up its link
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3), and the file is fetched
    /// through it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_id: Option<crate::SiteId>,
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
    #[serde(default)]
    pub kind: RecordingKind,
    /// The participant a `participant` recording is of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<String>,
    /// Everyone heard in the recording.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participant_list: Vec<RecordingParticipant>,
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
            site_id: None,
            path: path.into(),
            format,
            trigger,
            consent: RecordingConsent::NotRequired,
            started_at: SystemTime::now(),
            ended_at: None,
            duration_secs: None,
            size_bytes: None,
            participants: 0,
            kind: RecordingKind::Room,
            participant_id: None,
            participant_list: Vec::new(),
        }
    }

    /// Whether the recording is still being written.
    pub fn is_running(&self) -> bool {
        self.ended_at.is_none()
    }

    /// The file's name, without the directory: what its node knows it by.
    /// A stream's is its directory's, the sitting.
    pub fn file_name(&self) -> &str {
        self.path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.path)
    }

    /// The name a download is saved under: a stream's directory comes down
    /// as one MP4.
    pub fn download_name(&self) -> String {
        match self.kind {
            RecordingKind::Stream => format!("{}.{}", self.file_name(), self.format.extension()),
            _ => self.file_name().to_string(),
        }
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
    /// Only one kind: a meeting's streams, say.
    pub kind: Option<RecordingKind>,
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

    /// Write a recording as another deployment has it, every field
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
    async fn put_recording(&self, recording: &Recording) -> anyhow::Result<()> {
        self.create_recording(recording).await
    }

    /// Recordings written or changed after `after` in shipping order
    /// (`ship_seq`), oldest first, at most `limit`.
    async fn recordings_after(
        &self,
        after: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, Recording)>> {
        let _ = (after, limit);
        Ok(Vec::new())
    }

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

    /// Add people heard in a recording that is running.
    async fn add_recording_participants(
        &self,
        id: &RecordingId,
        participants: &[RecordingParticipant],
    ) -> anyhow::Result<()>;
}

// ── Call recordings (docs/COMPLIANCE_FOLLOWUPS.md §4) ───────────────────

/// Why a call is being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallRecordingSource {
    /// `POST /calls/:id/recording/start`.
    Api,
    /// A flow's `StartRecording` node.
    Flow,
    /// A queue with `record_calls`.
    Queue,
    /// `record_all_calls` on either party's user.
    User,
    /// A party asking from their app, under `record_on_demand`
    /// (`/me/calls/:id/recording/start`).
    OnDemand,
    /// `recording_enabled` on the dialled or calling number.
    Number,
    /// `media.enable_recording` on the node: every anchored call.
    Platform,
    /// A recording a record named before recordings had rows.
    Legacy,
}

impl CallRecordingSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            CallRecordingSource::Api => "api",
            CallRecordingSource::Flow => "flow",
            CallRecordingSource::Queue => "queue",
            CallRecordingSource::User => "user",
            CallRecordingSource::OnDemand => "on_demand",
            CallRecordingSource::Number => "number",
            CallRecordingSource::Platform => "platform",
            CallRecordingSource::Legacy => "legacy",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "api" => Some(CallRecordingSource::Api),
            "flow" => Some(CallRecordingSource::Flow),
            "queue" => Some(CallRecordingSource::Queue),
            "user" => Some(CallRecordingSource::User),
            "on_demand" => Some(CallRecordingSource::OnDemand),
            "number" => Some(CallRecordingSource::Number),
            "platform" => Some(CallRecordingSource::Platform),
            "legacy" => Some(CallRecordingSource::Legacy),
            _ => None,
        }
    }
}

/// A recording of a call, registered when it starts so it can be governed
/// before the call's record is stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallRecording {
    pub id: RecordingId,
    pub tenant_id: TenantId,
    /// The call's id as its record carries it (`call-…`).
    pub call_id: String,
    pub source: CallRecordingSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// The site holding the file, at the cloud: the row came up its link
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3), and the file is fetched
    /// through it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_id: Option<crate::SiteId>,
    /// Where the file is, as the recording host wrote it.
    pub path: String,
    /// The file's extension (`wav`, `opus`, `mp3`).
    pub format: String,
    pub consent: RecordingConsent,
    pub started_at: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<SystemTime>,
}

impl CallRecording {
    pub fn starting(
        tenant_id: TenantId,
        call_id: impl Into<String>,
        source: CallRecordingSource,
        path: impl Into<String>,
    ) -> Self {
        let path = path.into();
        let format = std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "wav".to_string());
        Self {
            id: RecordingId::new(),
            tenant_id,
            call_id: call_id.into(),
            source,
            node_id: None,
            site_id: None,
            path,
            format,
            consent: RecordingConsent::NotRequired,
            started_at: SystemTime::now(),
            ended_at: None,
        }
    }
}

/// Remove a recording's file, or a stream's whole directory. Nothing there
/// is not an error.
pub fn remove_recording_path(path: &std::path::Path) -> std::io::Result<()> {
    let removed = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) => Err(e),
    };
    match removed {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// What is on disk at a recording's path: its size (a stream's directory,
/// every file in it) and its last write, or nothing.
pub fn recording_on_disk(path: &std::path::Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_dir() {
        return Some((meta.len(), meta.modified().ok()?));
    }
    let mut size = 0;
    let mut last = meta.modified().ok()?;
    for entry in std::fs::read_dir(path).ok()?.flatten() {
        if let Ok(m) = entry.metadata() {
            if m.is_file() {
                size += m.len();
                if let Ok(t) = m.modified() {
                    last = last.max(t);
                }
            }
        }
    }
    Some((size, last))
}

/// The directory a new recording goes in: the tenant, then the UTC date,
/// so neither a caller's number nor a queue's name is in a file's path.
pub fn dated_recording_dir(tenant_id: &TenantId, at: SystemTime) -> std::path::PathBuf {
    let day = chrono::DateTime::<chrono::Utc>::from(at);
    std::path::PathBuf::from(tenant_id.to_string()).join(day.format("%Y/%m/%d").to_string())
}

/// A relative recording path a caller supplied, placed under the tenant's
/// directory. `None` for an absolute path or one that climbs out.
pub fn tenant_recording_path(tenant_id: &TenantId, supplied: &str) -> Option<std::path::PathBuf> {
    let relative = std::path::Path::new(supplied);
    let plain = !supplied.is_empty()
        && relative
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
    plain.then(|| std::path::PathBuf::from(tenant_id.to_string()).join(relative))
}

/// Close a call's recordings still open, when its recording stops.
pub async fn finish_call_recordings(
    store: &dyn CallRecordingStore,
    call_id: &str,
    at: SystemTime,
) -> anyhow::Result<usize> {
    let mut finished = 0;
    for recording in store.call_recordings_for(call_id).await? {
        if recording.ended_at.is_none() {
            store.finish_call_recording(&recording.id, at).await?;
            finished += 1;
        }
    }
    Ok(finished)
}

#[async_trait::async_trait]
pub trait CallRecordingStore: Send + Sync {
    async fn create_call_recording(&self, recording: &CallRecording) -> anyhow::Result<()>;

    async fn finish_call_recording(
        &self,
        id: &RecordingId,
        ended_at: SystemTime,
    ) -> anyhow::Result<()>;

    async fn get_call_recording(&self, id: &RecordingId) -> anyhow::Result<Option<CallRecording>>;

    /// A call's recordings, oldest first.
    async fn call_recordings_for(&self, call_id: &str) -> anyhow::Result<Vec<CallRecording>>;

    /// Write a call recording as another deployment has it, every field
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
    async fn put_call_recording(&self, recording: &CallRecording) -> anyhow::Result<()> {
        self.create_call_recording(recording).await
    }

    /// Delete one call recording's row; the file follows through the
    /// deletion queue, as every row deletion's does.
    async fn delete_call_recording(&self, id: &RecordingId) -> anyhow::Result<bool> {
        let _ = id;
        anyhow::bail!("this store cannot delete a call recording by id")
    }

    /// Call recordings written or changed after `after` in shipping
    /// order (`ship_seq`), oldest first, at most `limit`.
    async fn call_recordings_after(
        &self,
        after: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(i64, CallRecording)>> {
        let _ = (after, limit);
        Ok(Vec::new())
    }

    /// Delete a tenant's recordings that started before `before`; their
    /// files are queued for removal with them. Returns how many.
    async fn delete_call_recordings_before(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<u64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_enums_round_trip_through_their_strings() {
        for f in [
            RecordingFormat::Wav,
            RecordingFormat::Webm,
            RecordingFormat::Mp4,
        ] {
            assert_eq!(RecordingFormat::parse(f.as_str()), Some(f));
            assert_eq!(f.extension(), f.as_str());
        }
        assert_eq!(RecordingFormat::parse("WebM"), Some(RecordingFormat::Webm));
        assert_eq!(RecordingFormat::parse("MP4"), Some(RecordingFormat::Mp4));
        assert_eq!(RecordingFormat::parse("mkv"), None);
        assert_eq!(RecordingFormat::Webm.extension(), "webm");
        assert_eq!(RecordingFormat::Webm.content_type(), "video/webm");
        assert_eq!(RecordingFormat::Mp4.content_type(), "video/mp4");
        assert!(RecordingFormat::Webm.has_video());
        assert!(RecordingFormat::Mp4.has_video());
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
            RecordingConsent::Refused,
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
        assert_eq!(bare.download_name(), "room.wav");

        let stream = Recording {
            path: "/var/fcp/recordings/hls/5f1c".to_string(),
            format: RecordingFormat::Mp4,
            kind: RecordingKind::Stream,
            ..bare
        };
        assert_eq!(stream.file_name(), "5f1c");
        assert_eq!(stream.download_name(), "5f1c.mp4");
        assert_eq!(RecordingKind::parse("stream"), Some(RecordingKind::Stream));
    }

    #[test]
    fn a_stream_s_directory_is_measured_and_removed_whole() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("hls").join("s1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("init.mp4"), [0u8; 10]).unwrap();
        std::fs::write(dir.join("seg-1.m4s"), [0u8; 32]).unwrap();
        let (size, _) = recording_on_disk(&dir).unwrap();
        assert_eq!(size, 42);
        let file = root.path().join("room.wav");
        std::fs::write(&file, [0u8; 7]).unwrap();
        assert_eq!(recording_on_disk(&file).unwrap().0, 7);
        assert_eq!(recording_on_disk(&root.path().join("gone")), None);

        remove_recording_path(&dir).unwrap();
        remove_recording_path(&file).unwrap();
        assert!(!dir.exists() && !file.exists());
        remove_recording_path(&dir).unwrap();
    }

    #[test]
    fn new_recordings_go_under_the_tenant_and_the_day() {
        let tenant: TenantId = "00000000-0000-0000-0000-000000000009".parse().unwrap();
        let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_789_430_400);
        assert_eq!(
            dated_recording_dir(&tenant, at),
            std::path::PathBuf::from("tenant-00000000-0000-0000-0000-000000000009/2026/09/15")
        );
        assert_eq!(
            tenant_recording_path(&tenant, "calls/a.wav"),
            Some(std::path::PathBuf::from(
                "tenant-00000000-0000-0000-0000-000000000009/calls/a.wav"
            ))
        );
        for bad in ["", "/etc/passwd", "../other/a.wav", "calls/../../a.wav"] {
            assert_eq!(tenant_recording_path(&tenant, bad), None, "{bad:?}");
        }
        let r = CallRecording::starting(tenant, "call-1", CallRecordingSource::Flow, "x/y.OPUS");
        assert_eq!(r.format, "opus");
        for source in [
            CallRecordingSource::Api,
            CallRecordingSource::Flow,
            CallRecordingSource::Queue,
            CallRecordingSource::User,
            CallRecordingSource::Number,
            CallRecordingSource::Platform,
            CallRecordingSource::Legacy,
        ] {
            assert_eq!(CallRecordingSource::parse(source.as_str()), Some(source));
        }
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
