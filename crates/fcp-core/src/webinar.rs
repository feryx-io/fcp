//! A webinar's life (`docs/WEBINARS.md` §2.6, W4): a room's sitting and,
//! in a webinar, the phase it is in.
//!
//! A **sitting** is one run of a room, from the first caller to the last
//! (lobby included). Its id is the `session_id` of the questions and chat
//! of that run, and of the attendance read back for it. Every room has
//! sittings, meetings too; only a webinar's has a phase other than
//! `live`.
//!
//! A webinar's sitting starts in **practice**: the stage (hosts and
//! panelists) is in the room and hears itself, attendees who arrive wait
//! in the lobby, and nothing is recorded. A host takes it **live** (`*95`,
//! the API or the app), which lets the lobby in a few at a time and starts
//! recording when the meeting asks for it. It **ends** when a host ends
//! it, or when the stage has been empty for a grace period while live:
//! everyone is told and released.

use crate::TenantId;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Where a webinar's sitting is. A meeting's is always `live`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WebinarPhase {
    /// The stage is in the room; attendees wait in the lobby.
    Practice,
    /// The audience is in.
    Live,
    /// A host ended it: everyone is being released.
    Ended,
}

impl WebinarPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            WebinarPhase::Practice => "practice",
            WebinarPhase::Live => "live",
            WebinarPhase::Ended => "ended",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "practice" => Some(WebinarPhase::Practice),
            "live" => Some(WebinarPhase::Live),
            "ended" => Some(WebinarPhase::Ended),
            _ => None,
        }
    }
}

/// One sitting of a room, as kept (`conference_sittings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sitting {
    /// The sitting's id: the `session_id` of its questions and chat.
    pub id: String,
    pub tenant_id: TenantId,
    pub meeting_id: Option<String>,
    pub room_number: String,
    /// The meeting's kind when the sitting began (`meeting` or `webinar`).
    pub kind: String,
    /// When its first caller arrived.
    pub started_at: SystemTime,
    /// When it went live: a meeting's sitting at its start.
    pub live_at: Option<SystemTime>,
    /// When its last caller left, or a host ended it.
    pub ended_at: Option<SystemTime>,
    /// Who ended it: a participant id, `api`, `stage_left`, or `None` when
    /// its last caller left.
    pub ended_by: Option<String>,
}

/// Where sittings are kept.
#[async_trait::async_trait]
pub trait SittingStore: Send + Sync {
    /// Write a sitting as it now is.
    async fn put_sitting(&self, sitting: &Sitting) -> anyhow::Result<()>;
    /// A meeting's sittings, newest first, at most `limit`.
    async fn sittings_of_meeting(
        &self,
        tenant: &TenantId,
        meeting_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Sitting>>;
    /// One sitting in the tenant.
    async fn get_sitting(&self, tenant: &TenantId, id: &str) -> anyhow::Result<Option<Sitting>>;
    /// Forget a sitting that turned out to be part of another: an edge's,
    /// once it learns the stage's (`docs/WEBINARS.md` §2.7).
    async fn delete_sitting(&self, tenant: &TenantId, id: &str) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phase_reads_back_as_it_is_written() {
        for p in [
            WebinarPhase::Practice,
            WebinarPhase::Live,
            WebinarPhase::Ended,
        ] {
            assert_eq!(WebinarPhase::parse(p.as_str()), Some(p));
            assert_eq!(
                serde_json::to_string(&p).unwrap(),
                format!("\"{}\"", p.as_str())
            );
        }
        assert_eq!(WebinarPhase::parse("scheduled"), None);
    }
}

/// One person registered for a webinar (`webinar_registrants`): they join
/// by the attendee token minted for them, which names this registration,
/// and the room admits it only while the registration is not revoked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registrant {
    /// `reg-<uuid>`.
    pub id: String,
    pub tenant_id: TenantId,
    pub meeting_id: String,
    /// The name the room shows them by.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub created_at: SystemTime,
    /// When a host or administrator revoked it: their token is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<SystemTime>,
}

impl Registrant {
    /// A new registration for `meeting_id`.
    pub fn new(
        tenant_id: TenantId,
        meeting_id: impl Into<String>,
        name: impl Into<String>,
        email: Option<String>,
    ) -> Self {
        Self {
            id: format!("reg-{}", uuid::Uuid::new_v4()),
            tenant_id,
            meeting_id: meeting_id.into(),
            name: name.into(),
            email,
            created_at: SystemTime::now(),
            revoked_at: None,
        }
    }

    /// Whether their token is still admitted.
    pub fn stands(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// The most registrants one webinar may have.
pub const MAX_REGISTRANTS: usize = 10_000;

/// Where webinar registrations are kept.
#[async_trait::async_trait]
pub trait RegistrantStore: Send + Sync {
    /// Write a registration as it now is.
    async fn put_registrant(&self, registrant: &Registrant) -> anyhow::Result<()>;
    /// One registration in the tenant.
    async fn get_registrant(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> anyhow::Result<Option<Registrant>>;
    /// A meeting's registrations, oldest first: at most `limit` after the
    /// registration `after` (by id, in that order), and how many it has.
    async fn registrants_of_meeting(
        &self,
        tenant: &TenantId,
        meeting_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<(Vec<Registrant>, u64)>;
    /// A registrant was seen watching `sitting_id`'s stream at `at` (W8):
    /// the row made, or its last seen moved on, with `add_secs` watched.
    async fn record_stream_view(&self, view: &StreamViewSeen) -> anyhow::Result<()>;
    /// Who watched `sitting_id`'s stream by a registrant's link.
    async fn stream_views_of_sitting(
        &self,
        tenant: &TenantId,
        sitting_id: &str,
    ) -> anyhow::Result<Vec<StreamView>>;
}

/// A registrant seen watching a sitting's stream (W8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamViewSeen {
    pub tenant_id: TenantId,
    pub meeting_id: String,
    pub sitting_id: String,
    pub registrant_id: String,
    pub at: SystemTime,
    /// Seconds watched since the row was last written.
    pub add_secs: u64,
}

/// What a registrant watched of a sitting's stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamView {
    pub registrant_id: String,
    pub first_at: SystemTime,
    pub last_at: SystemTime,
    /// Seconds watched: the gaps under 30 s between the playlist's fetches
    /// while it was live. A replay opened shows none.
    pub watched_secs: u64,
}

/// Where a webinar is pushed over RTMP (`docs/WEBINARS.md` §2.8, W7): a
/// live service's ingest and the stream key it handed out. The key is a
/// secret: sealed at rest, never returned by the API, and kept apart from
/// the meeting so it never travels with it (to a site, by the journal).
#[derive(Clone, PartialEq, Eq)]
pub struct RtmpDestination {
    pub tenant_id: TenantId,
    /// The meeting's id.
    pub meeting_id: String,
    /// `rtmp://…` or `rtmps://…`, the application in its path.
    pub url: String,
    pub key: String,
    pub updated_at: SystemTime,
}

impl std::fmt::Debug for RtmpDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtmpDestination")
            .field("tenant_id", &self.tenant_id)
            .field("meeting_id", &self.meeting_id)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

/// Why an ingest URL is not one a push can use, if it is not: `rtmp://` or
/// `rtmps://`, a host, an application in the path, no credentials and no
/// query.
pub fn check_rtmp_url(url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("rtmps://")
        .or_else(|| url.strip_prefix("rtmp://"))
        .ok_or("the URL must start rtmp:// or rtmps://")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.is_empty() {
        return Err("the URL has no host".into());
    }
    if authority.contains('@') {
        return Err("the URL may not carry credentials: the stream key goes apart".into());
    }
    if url.contains('?') || url.contains('#') || url.chars().any(char::is_whitespace) {
        return Err("the URL may not carry a query, a fragment or spaces".into());
    }
    if path.trim_matches('/').is_empty() {
        return Err(
            "the URL needs the service's application in its path, as rtmp://host/live".into(),
        );
    }
    if url.len() > 512 {
        return Err("the URL is too long".into());
    }
    Ok(())
}

/// Where RTMP destinations are kept.
#[async_trait::async_trait]
pub trait RtmpDestinationStore: Send + Sync {
    /// The meeting's destination, its key opened.
    async fn get_rtmp_destination(
        &self,
        tenant: &TenantId,
        meeting_id: &str,
    ) -> anyhow::Result<Option<RtmpDestination>>;
    /// Write the destination, its key sealed.
    async fn put_rtmp_destination(&self, destination: &RtmpDestination) -> anyhow::Result<()>;
    /// Forget it; whether there was one.
    async fn delete_rtmp_destination(
        &self,
        tenant: &TenantId,
        meeting_id: &str,
    ) -> anyhow::Result<bool>;
}

#[cfg(test)]
mod rtmp_tests {
    use super::*;

    #[test]
    fn an_ingest_url_names_its_host_and_application_and_nothing_secret() {
        for ok in [
            "rtmp://a.rtmp.youtube.com/live2",
            "rtmps://live-api-s.facebook.com:443/rtmp/",
            "rtmp://[::1]:1935/live",
        ] {
            assert!(check_rtmp_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "https://x/live",
            "rtmp://x",
            "rtmp://x/",
            "rtmp:///live",
            "rtmp://u:p@x/live",
            "rtmp://x/live?key=1",
            "rtmp://x/li ve",
        ] {
            assert!(check_rtmp_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_destination_never_prints_its_key() {
        let d = RtmpDestination {
            tenant_id: TenantId::new(),
            meeting_id: "m".into(),
            url: "rtmp://x/live".into(),
            key: "s3cret-key".into(),
            updated_at: SystemTime::now(),
        };
        assert!(!format!("{d:?}").contains("s3cret"));
    }
}
