//! Compliance basics: per-tenant retention policy, recording-consent
//! policy, the admin audit log, and the retention engine.
//!
//! - [`CompliancePolicy`]: how long CDRs, SIP/flow traces, recordings, and
//!   audit events are kept, plus the recording-consent mode. `None` for a
//!   retention window means "keep forever" (the default).
//! - [`AuditEvent`] / [`AuditQuery`]: one row per mutating admin request
//!   (who, what, on which tenant, with what result), written by the API
//!   layer's audit middleware.
//! - [`run_retention`]: applies a policy to storage, deleting what is past
//!   its window and reporting counts. Idempotent; safe to run on a timer
//!   and on demand.
//! - [`ComplianceStore`]: persistence for policies, audit events, and
//!   retention run reports.

use crate::storage::{CdrStore, DynStorage};
use crate::TenantId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// How callers are told a call is being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingConsentMode {
    /// No announcement (single-party-consent jurisdictions, or handled by
    /// the IVR/flow already).
    #[default]
    None,
    /// Play the configured prompt to the call before recording starts.
    Announce,
}

impl std::fmt::Display for RecordingConsentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordingConsentMode::None => write!(f, "none"),
            RecordingConsentMode::Announce => write!(f, "announce"),
        }
    }
}

/// What to do when the consent announcement cannot be played.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConsentFailureAction {
    /// Record anyway; the recording and the record say `failed`.
    #[default]
    Record,
    /// Do not record; the record says `refused`.
    Skip,
}

impl ConsentFailureAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConsentFailureAction::Record => "record",
            ConsentFailureAction::Skip => "skip",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "record" => Some(ConsentFailureAction::Record),
            "skip" => Some(ConsentFailureAction::Skip),
            _ => None,
        }
    }
}

/// What an erasure does to the subject's call records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CdrErasure {
    /// Replace the subject's identifiers with their pseudonym: counts and
    /// billing still add up.
    #[default]
    Pseudonymise,
    /// Delete the records.
    Delete,
}

impl CdrErasure {
    pub fn as_str(&self) -> &'static str {
        match self {
            CdrErasure::Pseudonymise => "pseudonymise",
            CdrErasure::Delete => "delete",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pseudonymise" => Some(CdrErasure::Pseudonymise),
            "delete" => Some(CdrErasure::Delete),
            _ => None,
        }
    }
}

/// The shortest notice-tone interval a policy may set: a tone every few
/// seconds would drown the conversation it is a notice about.
pub const MIN_NOTICE_INTERVAL_SECS: u32 = 5;

/// Per-tenant retention and consent settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompliancePolicy {
    pub tenant_id: TenantId,
    /// Call detail records older than this are deleted.
    pub cdr_retention_days: Option<u32>,
    /// Raw SIP traces older than this are deleted.
    pub sip_trace_retention_days: Option<u32>,
    /// Flow execution traces older than this are deleted.
    pub flow_trace_retention_days: Option<u32>,
    /// Recording files older than this are deleted and unlinked from CDRs.
    pub recording_retention_days: Option<u32>,
    /// Admin audit events older than this are deleted.
    pub audit_retention_days: Option<u32>,
    /// Voicemail messages older than this are deleted with their audio. A
    /// mailbox's own `retention_days`, when shorter, applies as well.
    #[serde(default)]
    pub voicemail_retention_days: Option<u32>,
    /// Chat in a room (`docs/WEBINARS.md` §2.5) older than this is
    /// deleted.
    #[serde(default)]
    pub conference_chat_retention_days: Option<u32>,
    /// Questions asked in webinars (`docs/WEBINARS.md` §2.5) older than
    /// this are deleted, by when they were asked.
    #[serde(default)]
    pub webinar_question_retention_days: Option<u32>,
    /// Webinar registrations (`docs/WEBINARS.md` §2.6) older than this are
    /// deleted, by when they were made: a registrant's token stops working
    /// with their registration.
    #[serde(default)]
    pub webinar_registrant_retention_days: Option<u32>,
    /// Call detail records are deleted only once the CDR archive holds
    /// them, confirmed (`docs/CDR_PIPELINE.md` §13, phase 5): retention
    /// waits for the archiver rather than removing the only copy.
    #[serde(default)]
    pub cdr_archive_required: bool,
    pub recording_consent: RecordingConsentMode,
    /// Prompt played when `recording_consent` is `announce`.
    pub recording_consent_prompt_id: Option<String>,
    /// Whether a recording goes ahead when its announcement fails.
    #[serde(default)]
    pub recording_consent_on_failure: ConsentFailureAction,
    /// A tone played to the parties every this many seconds while a call
    /// is recorded, for regimes that require a continuing notice.
    #[serde(default)]
    pub recording_notice_interval_secs: Option<u32>,
    /// Days a subject request may take from receipt to answer.
    #[serde(default = "default_subject_request_days")]
    pub subject_request_days: u32,
    /// Whether an erasure pseudonymises or deletes the subject's records.
    #[serde(default)]
    pub cdr_erasure: CdrErasure,
    pub updated_at: SystemTime,
}

fn default_subject_request_days() -> u32 {
    crate::subject_request::DEFAULT_REQUEST_DAYS
}

impl CompliancePolicy {
    /// Keep everything forever, announce nothing.
    pub fn defaults(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            cdr_retention_days: None,
            sip_trace_retention_days: None,
            flow_trace_retention_days: None,
            recording_retention_days: None,
            audit_retention_days: None,
            voicemail_retention_days: None,
            conference_chat_retention_days: None,
            webinar_question_retention_days: None,
            webinar_registrant_retention_days: None,
            cdr_archive_required: false,
            recording_consent: RecordingConsentMode::None,
            recording_consent_prompt_id: None,
            recording_consent_on_failure: ConsentFailureAction::Record,
            recording_notice_interval_secs: None,
            subject_request_days: crate::subject_request::DEFAULT_REQUEST_DAYS,
            cdr_erasure: CdrErasure::Pseudonymise,
            updated_at: SystemTime::now(),
        }
    }

    /// True when at least one retention window is set.
    pub fn has_retention(&self) -> bool {
        self.cdr_retention_days.is_some()
            || self.sip_trace_retention_days.is_some()
            || self.flow_trace_retention_days.is_some()
            || self.recording_retention_days.is_some()
            || self.audit_retention_days.is_some()
            || self.voicemail_retention_days.is_some()
            || self.conference_chat_retention_days.is_some()
            || self.webinar_question_retention_days.is_some()
            || self.webinar_registrant_retention_days.is_some()
    }
}

// ── Recording consent (docs/COMPLIANCE_FOLLOWUPS.md §8) ───────────────

/// The words a synthesised announcement says when the tenant's prompt has
/// no audio to play.
pub const RECORDING_CONSENT_TEXT: &str = "This call is being recorded.";

/// The continuing notice: a short 1400 Hz beep, the tone the US regulator
/// describes for recorded lines (1260–1540 Hz, about 175 ms).
pub const NOTICE_TONE_HZ: f32 = 1400.0;
pub const NOTICE_TONE_MS: u32 = 175;

/// The notice beep as 16-bit samples at `sample_rate`, a quarter of full
/// scale, with 5 ms ramps so it does not click.
pub fn notice_tone_samples(sample_rate: u32) -> Vec<i16> {
    let n = (sample_rate as usize * NOTICE_TONE_MS as usize) / 1000;
    let ramp = (sample_rate as usize * 5 / 1000).max(1);
    (0..n)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let edge = (i.min(n - 1 - i) as f32 / ramp as f32).min(1.0);
            let v = (2.0 * std::f32::consts::PI * NOTICE_TONE_HZ * t).sin() * 0.25 * edge;
            (v * i16::MAX as f32) as i16
        })
        .collect()
}

/// What a recording host must do about consent before it records a call
/// of a tenant: resolved once from the policy and the prompt, then applied
/// by whoever plays audio to the parties (the B2BUA's media session, the
/// queue's RTP legs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentPlan {
    /// The parties must hear an announcement before the first sample.
    pub announce: bool,
    /// The tenant's prompt, when it names one with audio. With `announce`
    /// and no file, the host synthesises [`RECORDING_CONSENT_TEXT`] if it
    /// can.
    pub prompt_audio: Option<std::path::PathBuf>,
    pub on_failure: ConsentFailureAction,
    /// A notice tone every this long while the recording runs.
    pub notice_interval: Option<Duration>,
}

impl ConsentPlan {
    /// No announcement, no notice.
    pub fn not_required() -> Self {
        Self {
            announce: false,
            prompt_audio: None,
            on_failure: ConsentFailureAction::Record,
            notice_interval: None,
        }
    }

    /// The plan for `tenant`. A policy that cannot be read is treated as
    /// one asking for an announcement that cannot be made: the recording
    /// goes ahead marked `failed`, as the conference server has always done.
    pub async fn resolve(
        compliance: Option<&dyn ComplianceStore>,
        prompts: Option<&dyn crate::prompt::PromptStore>,
        tenant: &TenantId,
    ) -> Self {
        let Some(compliance) = compliance else {
            return Self::not_required();
        };
        let policy = match compliance.get_compliance_policy(tenant).await {
            Ok(Some(policy)) => policy,
            Ok(None) => return Self::not_required(),
            Err(e) => {
                tracing::warn!(tenant_id = %tenant, error = %e, "Could not read the recording-consent policy");
                return Self {
                    announce: true,
                    ..Self::not_required()
                };
            }
        };
        let notice_interval = policy
            .recording_notice_interval_secs
            .map(|s| Duration::from_secs(u64::from(s.max(MIN_NOTICE_INTERVAL_SECS))));
        if policy.recording_consent != RecordingConsentMode::Announce {
            return Self {
                notice_interval,
                ..Self::not_required()
            };
        }
        let mut prompt_audio = None;
        if let (Some(id), Some(prompts)) = (policy.recording_consent_prompt_id.as_deref(), prompts)
        {
            match id.parse::<crate::PromptId>() {
                Ok(prompt_id) => {
                    match crate::prompt::effective_prompt_by_id(prompts, tenant, &prompt_id).await {
                        Ok(Some(prompt)) => {
                            prompt_audio = prompt.audio_path.filter(|p| p.is_file());
                            if prompt_audio.is_none() {
                                tracing::warn!(tenant_id = %tenant, prompt = %id, "Recording-consent prompt has no audio file");
                            }
                        }
                        Ok(_) => {
                            tracing::warn!(tenant_id = %tenant, prompt = %id, "Recording-consent prompt not found")
                        }
                        Err(e) => {
                            tracing::warn!(tenant_id = %tenant, error = %e, "Could not read the recording-consent prompt")
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(tenant_id = %tenant, prompt = %id, error = %e, "Invalid recording-consent prompt id")
                }
            }
        }
        Self {
            announce: true,
            prompt_audio,
            on_failure: policy.recording_consent_on_failure,
            notice_interval,
        }
    }

    /// What the record says, given whether the announcement was heard to
    /// the end.
    pub fn outcome(&self, announced: bool) -> crate::recording::RecordingConsent {
        use crate::recording::RecordingConsent;
        match (self.announce, announced, self.on_failure) {
            (false, _, _) => RecordingConsent::NotRequired,
            (true, true, _) => RecordingConsent::Announced,
            (true, false, ConsentFailureAction::Record) => RecordingConsent::Failed,
            (true, false, ConsentFailureAction::Skip) => RecordingConsent::Refused,
        }
    }
}

/// Counts of call recordings by why they started and what the parties
/// were told, for a node's `/metrics` (`docs/COMPLIANCE_FOLLOWUPS.md` §9).
#[derive(Debug, Default)]
pub struct RecordingCounters {
    outcomes: std::sync::Mutex<std::collections::BTreeMap<(String, &'static str), u64>>,
}

impl RecordingCounters {
    /// The process's counters.
    pub fn global() -> &'static RecordingCounters {
        static COUNTERS: std::sync::OnceLock<RecordingCounters> = std::sync::OnceLock::new();
        COUNTERS.get_or_init(RecordingCounters::default)
    }

    /// A recording of `source` reached its consent step's `outcome`.
    pub fn note(&self, source: &str, outcome: crate::recording::RecordingConsent) {
        if let Ok(mut map) = self.outcomes.lock() {
            *map.entry((source.to_string(), outcome.as_str()))
                .or_insert(0) += 1;
        }
    }

    /// Prometheus text: attempts by source and consent outcome, and those
    /// that started (every outcome but `refused`). `labels` is prepended to
    /// each series' labels (`node="…"`), or empty.
    pub fn prometheus(&self, labels: &str) -> String {
        let Ok(map) = self.outcomes.lock() else {
            return String::new();
        };
        let sep = if labels.is_empty() { "" } else { "," };
        let mut out = String::from(
            "# HELP fcp_call_recording_consent_total Call recordings by source and consent outcome.\n\
             # TYPE fcp_call_recording_consent_total counter\n",
        );
        let mut started = std::collections::BTreeMap::<&str, u64>::new();
        for ((source, outcome), n) in map.iter() {
            out.push_str(&format!(
                "fcp_call_recording_consent_total{{{labels}{sep}source=\"{source}\",consent=\"{outcome}\"}} {n}\n"
            ));
            if *outcome != "refused" {
                *started.entry(source.as_str()).or_insert(0) += n;
            }
        }
        out.push_str(
            "# HELP fcp_call_recordings_started_total Call recordings started, by source.\n\
             # TYPE fcp_call_recordings_started_total counter\n",
        );
        for (source, n) in started {
            out.push_str(&format!(
                "fcp_call_recordings_started_total{{{labels}{sep}source=\"{source}\"}} {n}\n"
            ));
        }
        out
    }
}

// ── Who is deleting (docs/COMPLIANCE_FOLLOWUPS.md §7) ─────────────────

/// Why data was deleted, as the deletion log records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionCause {
    /// A retention window, run by the sweep or on demand.
    Retention,
    /// A person or a token, through the API or a phone menu.
    Manual,
    /// A node removing its own recordings on its sweep.
    NodeSweep,
    /// Platform upkeep: files no row names, messages users deleted.
    Upkeep,
    /// A subject erasure request.
    Erasure,
    /// A tenant's deletion.
    TenantDeletion,
    /// Deleted at its site, mirrored here as the deletion came up the
    /// site's link (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
    Site,
    /// A deprovisioned user's purge (`docs/USERS.md` §4).
    Offboarding,
}

impl DeletionCause {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeletionCause::Retention => "retention",
            DeletionCause::Manual => "manual",
            DeletionCause::NodeSweep => "node_sweep",
            DeletionCause::Upkeep => "upkeep",
            DeletionCause::Erasure => "erasure",
            DeletionCause::TenantDeletion => "tenant_deletion",
            DeletionCause::Site => "site",
            DeletionCause::Offboarding => "offboarding",
        }
    }
}

/// The cause and actor of the deletions made inside [`deleting_as`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleter {
    pub cause: DeletionCause,
    /// A username, `token-…`, `static_token`, or a host (`fcp-admin`, a
    /// conference node's id).
    pub actor: String,
    /// The subject request the deletions serve, when one does.
    pub request_id: Option<String>,
    /// The API request making the deletions (its `X-Request-Id`), which
    /// its audit row names too (#342).
    pub api_request_id: Option<String>,
    /// How the actor reached what they deleted: the phone a mailbox's
    /// owner called from.
    pub via: Option<String>,
}

impl Deleter {
    pub fn new(cause: DeletionCause, actor: impl Into<String>) -> Self {
        Self {
            cause,
            actor: actor.into(),
            request_id: None,
            api_request_id: None,
            via: None,
        }
    }

    /// The deletions serve request `id`.
    pub fn for_request(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    /// The deletions are API request `id`'s.
    pub fn in_api_request(mut self, id: Option<String>) -> Self {
        self.api_request_id = id;
        self
    }

    /// The actor reached what they deleted `via` this.
    pub fn via(mut self, via: impl Into<String>) -> Self {
        self.via = Some(via.into());
        self
    }
}

tokio::task_local! {
    static DELETER: Deleter;
}

/// Run `f` with every deletion it makes attributed to `deleter`. Storage
/// backends read it with [`current_deleter`] and hand it to the deletion
/// log; a deletion made outside any scope is logged as `unattributed`.
pub async fn deleting_as<F: std::future::Future>(deleter: Deleter, f: F) -> F::Output {
    DELETER.scope(deleter, f).await
}

/// The deleter of the current task, if [`deleting_as`] set one.
pub fn current_deleter() -> Option<Deleter> {
    DELETER.try_with(Clone::clone).ok()
}

/// One entry of the deletion log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeletionRecord {
    pub id: i64,
    pub tenant_id: Option<TenantId>,
    /// `cdr`, `call_recording`, `conference_recording`, `voicemail`,
    /// `sip_trace`, `flow_trace`, `audit_event`, `orphan_file`.
    pub class: String,
    /// The item's id, or a time range for a batch.
    pub item: String,
    pub detail: Option<serde_json::Value>,
    /// `retention`, `manual`, `node_sweep`, `upkeep`, `erasure`,
    /// `tenant_deletion`, `cascade` or `unattributed`.
    pub cause: String,
    pub actor: String,
    /// The subject request the deletion served.
    #[serde(default)]
    pub request_id: Option<String>,
    /// The API request that deleted it, as its audit row names it.
    #[serde(default)]
    pub api_request_id: Option<String>,
    /// How the actor reached it (the phone a mailbox's owner called from).
    #[serde(default)]
    pub via: Option<String>,
    pub deleted_at: SystemTime,
}

/// Filter for [`ComplianceStore::list_deletions`].
#[derive(Debug, Clone, Default)]
pub struct DeletionQuery {
    /// `None` lists every tenant (platform admins only).
    pub tenant_id: Option<TenantId>,
    pub class: Option<String>,
    pub since: Option<SystemTime>,
    /// Only deletions before this instant.
    pub until: Option<SystemTime>,
    /// Only what this subject request deleted.
    pub request_id: Option<String>,
    /// Only what this API request deleted.
    pub api_request_id: Option<String>,
    pub limit: usize,
    /// Only rows older than this `(deleted_at, id)`: the keyset cursor of
    /// the API's pages (docs/DEVELOPER_SURFACE.md §3, phase 2c).
    pub before: Option<(SystemTime, i64)>,
}

/// A file whose row is gone, waiting for the reaper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingFileDeletion {
    pub id: i64,
    pub tenant_id: Option<TenantId>,
    pub class: String,
    pub path: String,
    pub attempts: u32,
    /// A live row still names the path (a greeting rewritten in place):
    /// the entry is dropped and the file kept.
    pub still_referenced: bool,
    /// The site holding the file, when the row came up a site's link
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3): the site removes it, asked
    /// through its link, not this host.
    pub site_id: Option<crate::SiteId>,
}

/// The instant before which data of a given age is past its window.
pub fn cutoff(now: SystemTime, retention_days: u32) -> SystemTime {
    now - Duration::from_secs(u64::from(retention_days) * 86_400)
}

/// The instant before which a tenant's call detail records may be deleted
/// now: the policy's CDR window, held back to the archive's frontier when
/// the policy requires the archive. `None` keeps every record. The second
/// value is the window's own cutoff when the archive is holding it back.
pub async fn cdr_retention_cutoff(
    store: &dyn CdrStore,
    policy: &CompliancePolicy,
    now: SystemTime,
) -> anyhow::Result<(Option<SystemTime>, Option<SystemTime>)> {
    let Some(days) = policy.cdr_retention_days else {
        return Ok((None, None));
    };
    let window = cutoff(now, days);
    if !policy.cdr_archive_required {
        return Ok((Some(window), None));
    }
    let frontier = store.cdr_archive_frontier(&policy.tenant_id).await?;
    Ok(match frontier {
        Some(f) if f >= window => (Some(window), None),
        Some(f) => (Some(f), Some(window)),
        None => (None, Some(window)),
    })
}

/// How long past its retention a conference recording's row is left for
/// the node holding the file to remove, file and row together, before
/// this sweep takes the row on its own (`run_retention`).
pub const CONFERENCE_RECORDING_GRACE: Duration = Duration::from_secs(7 * 86_400);

/// One audited admin request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Database id; 0 before insertion.
    pub id: i64,
    /// Tenant the request acted on (`None` for platform-wide requests).
    pub tenant_id: Option<TenantId>,
    /// The actor's own tenant (`None` for the static token, and for rows
    /// written before v0.25.497): differs from `tenant_id` when a platform
    /// administrator acts on another tenant.
    #[serde(default)]
    pub actor_tenant_id: Option<TenantId>,
    /// Who: a username, an API token id, `static_token`, or `anonymous`.
    pub actor: String,
    /// `session`, `api_token`, `static_token`, or `anonymous`.
    pub actor_kind: String,
    pub method: String,
    /// The concrete request path (`/api/v1/users/alice`).
    pub path: String,
    pub status: u16,
    /// Method plus route template (`PUT /api/v1/users/:username`), stable
    /// across ids for filtering.
    pub action: String,
    /// Extra context (e.g. `{"export": "cdr", "rows": 120}`).
    pub detail: Option<serde_json::Value>,
    pub source_ip: Option<String>,
    /// The request's `X-Request-Id`, which the deletions it made name too.
    #[serde(default)]
    pub api_request_id: Option<String>,
    pub occurred_at: SystemTime,
}

/// Filter for [`ComplianceStore::list_audit_events`].
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// `None` lists every tenant (platform admins only).
    pub tenant_id: Option<TenantId>,
    /// Only the platform's own rows (no tenant); `tenant_id` is then unset.
    pub platform_only: bool,
    pub actor: Option<String>,
    /// Match on the route template prefix (`POST /api/v1/users`).
    pub action_prefix: Option<String>,
    pub since: Option<SystemTime>,
    /// Only events before this instant.
    pub until: Option<SystemTime>,
    /// Only the request with this `X-Request-Id`.
    pub api_request_id: Option<String>,
    pub limit: usize,
    /// Only events older than this `(occurred_at, id)`: the keyset cursor
    /// of the API's pages (docs/DEVELOPER_SURFACE.md §3, phase 2c).
    pub before: Option<(SystemTime, i64)>,
}

/// What one retention run deleted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetentionReport {
    pub tenant_id: TenantId,
    pub ran_at: SystemTime,
    pub cdrs_deleted: u64,
    pub sip_traces_deleted: u64,
    pub flow_traces_deleted: u64,
    pub recordings_deleted: u64,
    /// Conference recordings whose registry row this run removed. The
    /// node holding the file normally gets there first, file and row
    /// together; this counts what it left behind (see `run_retention`).
    pub conference_recordings_deleted: u64,
    /// Voicemail messages, their audio queued for removal.
    #[serde(default)]
    pub voicemail_messages_deleted: u64,
    /// Chat messages posted in rooms.
    #[serde(default)]
    pub conference_chat_deleted: u64,
    /// Questions asked in webinars.
    #[serde(default)]
    pub webinar_questions_deleted: u64,
    /// Webinar registrations.
    #[serde(default)]
    pub webinar_registrants_deleted: u64,
    pub audit_events_deleted: u64,
    /// What legal holds kept past a window, by class.
    #[serde(default)]
    pub held: std::collections::BTreeMap<String, u64>,
    /// Non-fatal problems (a file that could not be removed, a store that
    /// is not configured). The run continues past each.
    pub errors: Vec<String>,
}

impl RetentionReport {
    fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            ran_at: SystemTime::now(),
            cdrs_deleted: 0,
            sip_traces_deleted: 0,
            flow_traces_deleted: 0,
            recordings_deleted: 0,
            conference_recordings_deleted: 0,
            voicemail_messages_deleted: 0,
            conference_chat_deleted: 0,
            webinar_questions_deleted: 0,
            webinar_registrants_deleted: 0,
            audit_events_deleted: 0,
            held: Default::default(),
            errors: Vec::new(),
        }
    }

    pub fn total_deleted(&self) -> u64 {
        self.cdrs_deleted
            + self.sip_traces_deleted
            + self.flow_traces_deleted
            + self.recordings_deleted
            + self.conference_recordings_deleted
            + self.voicemail_messages_deleted
            + self.conference_chat_deleted
            + self.webinar_questions_deleted
            + self.webinar_registrants_deleted
            + self.audit_events_deleted
    }
}

/// Persistence for policies, the audit log, and retention reports.
#[async_trait]
pub trait ComplianceStore: Send + Sync {
    async fn get_compliance_policy(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Option<CompliancePolicy>>;
    async fn put_compliance_policy(&self, policy: &CompliancePolicy) -> anyhow::Result<()>;

    /// Insert an audit event and return its id.
    async fn record_audit_event(&self, event: &AuditEvent) -> anyhow::Result<i64>;
    async fn list_audit_events(&self, query: &AuditQuery) -> anyhow::Result<Vec<AuditEvent>>;
    async fn delete_audit_events_before(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<u64>;

    /// Delete a tenant's chat in rooms (`docs/WEBINARS.md` §2.5) posted
    /// before `before`, leaving what a legal hold covers.
    async fn delete_conference_chat_before(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<u64> {
        let _ = (tenant_id, before);
        anyhow::bail!("this store keeps no chat")
    }

    /// Delete a tenant's webinar questions asked before `before`, leaving
    /// what a legal hold covers.
    async fn delete_webinar_questions_before(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<u64> {
        let _ = (tenant_id, before);
        anyhow::bail!("this store keeps no webinar questions")
    }

    /// Delete a tenant's webinar registrations made before `before`,
    /// leaving what a legal hold covers.
    async fn delete_webinar_registrants_before(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<u64> {
        let _ = (tenant_id, before);
        anyhow::bail!("this store keeps no webinar registrations")
    }

    async fn record_retention_run(&self, report: &RetentionReport) -> anyhow::Result<()>;

    /// The deletion log, newest first.
    async fn list_deletions(&self, query: &DeletionQuery) -> anyhow::Result<Vec<DeletionRecord>>;

    /// The deletions logged after entry `after`, oldest first, at most
    /// `limit`: what a site ships up so the cloud drops its copies
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
    async fn deletions_after(
        &self,
        after: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<DeletionRecord>> {
        let _ = (after, limit);
        Ok(Vec::new())
    }

    /// Log a deletion that removed no row (a file no row named).
    async fn record_deletion(&self, record: &DeletionRecord) -> anyhow::Result<()>;

    /// Files due for removal, oldest first.
    async fn due_file_deletions(&self, limit: usize) -> anyhow::Result<Vec<PendingFileDeletion>>;

    /// The file is gone, or a row still names it: drop the entry.
    async fn file_deletion_done(&self, id: i64) -> anyhow::Result<()>;

    /// The file could not be removed: try again after `retry_in`.
    async fn file_deletion_failed(
        &self,
        id: i64,
        error: &str,
        retry_in: Duration,
    ) -> anyhow::Result<()>;

    /// Of `paths`, those a recording, voicemail, greeting, prompt or
    /// profile picture row still names.
    async fn referenced_paths(&self, paths: &[String]) -> anyhow::Result<Vec<String>>;

    // ── Legal holds (docs/COMPLIANCE_FOLLOWUPS.md §5) ──────────────────

    async fn place_hold(&self, hold: &crate::legal_hold::LegalHold) -> anyhow::Result<()> {
        let _ = hold;
        anyhow::bail!("this store keeps no legal holds")
    }

    async fn get_hold(
        &self,
        tenant_id: &TenantId,
        id: &crate::LegalHoldId,
    ) -> anyhow::Result<Option<crate::legal_hold::LegalHold>> {
        let _ = (tenant_id, id);
        Ok(None)
    }

    /// A tenant's holds, newest first; released ones too when asked.
    async fn list_holds(
        &self,
        tenant_id: &TenantId,
        include_released: bool,
    ) -> anyhow::Result<Vec<crate::legal_hold::LegalHold>> {
        let _ = (tenant_id, include_released);
        Ok(Vec::new())
    }

    /// Release an active hold. `false` when it is not active (or not the
    /// tenant's).
    async fn release_hold(
        &self,
        tenant_id: &TenantId,
        id: &crate::LegalHoldId,
        release: &crate::legal_hold::HoldRelease,
    ) -> anyhow::Result<bool> {
        let _ = (tenant_id, id, release);
        Ok(false)
    }

    /// How many items of each class a hold covers now; `None` for a class
    /// too large to count within the store's time budget.
    async fn held_counts(
        &self,
        tenant_id: &TenantId,
        id: &crate::LegalHoldId,
    ) -> anyhow::Result<Vec<(crate::legal_hold::DataClass, Option<u64>)>> {
        let _ = (tenant_id, id);
        Ok(Vec::new())
    }

    /// The hold covering one item (a recording's or a message's id), if any.
    async fn hold_on(
        &self,
        tenant_id: &TenantId,
        class: crate::legal_hold::DataClass,
        item: &str,
    ) -> anyhow::Result<Option<crate::LegalHoldId>> {
        let _ = (tenant_id, class, item);
        Ok(None)
    }

    /// How many of a tenant's items of `class` dated before `before` are
    /// held: what retention kept.
    async fn held_before(
        &self,
        tenant_id: &TenantId,
        class: crate::legal_hold::DataClass,
        before: SystemTime,
    ) -> anyhow::Result<u64> {
        let _ = (tenant_id, class, before);
        Ok(0)
    }
    async fn list_retention_runs(
        &self,
        tenant_id: &TenantId,
        limit: usize,
    ) -> anyhow::Result<Vec<RetentionReport>>;

    // ── Subject requests (docs/COMPLIANCE_FOLLOWUPS.md §6.2) ──────────

    async fn create_subject_request(
        &self,
        _request: &crate::subject_request::SubjectRequest,
    ) -> anyhow::Result<()> {
        anyhow::bail!("subject requests are not supported by this store")
    }

    async fn get_subject_request(
        &self,
        _tenant_id: &TenantId,
        _id: &crate::SubjectRequestId,
    ) -> anyhow::Result<Option<crate::subject_request::SubjectRequest>> {
        Ok(None)
    }

    /// A tenant's requests, newest first, optionally of one status.
    async fn list_subject_requests(
        &self,
        _tenant_id: &TenantId,
        _status: Option<crate::subject_request::SubjectRequestStatus>,
        _limit: usize,
    ) -> anyhow::Result<Vec<crate::subject_request::SubjectRequest>> {
        Ok(Vec::new())
    }

    /// The oldest received request of any tenant, marked running.
    async fn claim_subject_request(
        &self,
    ) -> anyhow::Result<Option<crate::subject_request::SubjectRequest>> {
        Ok(None)
    }

    /// Store a request's progress: status, expansion, times, report,
    /// bundle, error.
    async fn update_subject_request(
        &self,
        _request: &crate::subject_request::SubjectRequest,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Put requests left running by a worker that stopped back in the
    /// queue. Returns how many.
    async fn requeue_running_subject_requests(&self) -> anyhow::Result<u64> {
        Ok(0)
    }

    /// Completed requests whose bundle is past `now` and not yet removed.
    async fn expired_subject_bundles(
        &self,
        _now: SystemTime,
    ) -> anyhow::Result<Vec<crate::subject_request::SubjectRequest>> {
        Ok(Vec::new())
    }

    /// Open requests by kind: `(kind, open, past due at now)`.
    async fn subject_request_counts(
        &self,
        _now: SystemTime,
    ) -> anyhow::Result<Vec<(String, u64, u64)>> {
        Ok(Vec::new())
    }
}

/// Apply `policy` to `storage`, every deletion attributed to `actor` in
/// the deletion log. Each window is independent; an error in one step is
/// recorded and the next step still runs. Recordings are deleted by their
/// rows, whose files the reaper then removes (`docs/COMPLIANCE_FOLLOWUPS.md`
/// §4); nothing here walks a directory, since a directory holds every
/// tenant's files.
pub async fn run_retention(
    storage: &DynStorage,
    policy: &CompliancePolicy,
    actor: &str,
) -> RetentionReport {
    deleting_as(
        Deleter::new(DeletionCause::Retention, actor),
        apply_retention(storage, policy),
    )
    .await
}

async fn apply_retention(storage: &DynStorage, policy: &CompliancePolicy) -> RetentionReport {
    let now = SystemTime::now();
    let tenant = policy.tenant_id;
    let mut report = RetentionReport::new(tenant);

    if let Some(days) = policy.flow_trace_retention_days {
        match storage.flow_trace_store() {
            Some(store) => match store
                .delete_flow_traces_for_tenant_before(&tenant.to_string(), cutoff(now, days))
                .await
            {
                Ok(n) => report.flow_traces_deleted = n,
                Err(e) => report.errors.push(format!("flow traces: {e}")),
            },
            None => report
                .errors
                .push("flow traces: store not configured".to_string()),
        }
    }

    if let Some(days) = policy.sip_trace_retention_days {
        match storage.sip_trace_store() {
            Some(store) => match store
                .delete_sip_traces_for_tenant_before(&tenant.to_string(), cutoff(now, days))
                .await
            {
                Ok(n) => report.sip_traces_deleted = n,
                Err(e) => report.errors.push(format!("sip traces: {e}")),
            },
            None => report
                .errors
                .push("sip traces: store not configured".to_string()),
        }
    }

    if let Some(days) = policy.recording_retention_days {
        let before = cutoff(now, days);
        match storage.call_recording_store() {
            Some(store) => match store.delete_call_recordings_before(&tenant, before).await {
                Ok(n) => report.recordings_deleted = n,
                Err(e) => report.errors.push(format!("call recordings: {e}")),
            },
            None => report
                .errors
                .push("call recordings: store not configured".to_string()),
        }

        // Conference recordings are the holding node's to delete: it has
        // the file, and it applies this same policy on its own sweep.
        // What is left here well past the cutoff belonged to a node that
        // is gone, so the row goes; the deletion log names the file.
        if let Some(store) = storage.recording_store() {
            let abandoned = before - CONFERENCE_RECORDING_GRACE;
            match store.expired_recordings(&tenant, abandoned).await {
                Ok(recordings) => {
                    for recording in recordings {
                        // On shared storage the file is reachable here;
                        // usually it is not, and that is not an error.
                        let path = std::path::PathBuf::from(&recording.path);
                        let _ = tokio::task::spawn_blocking(move || {
                            crate::recording::remove_recording_path(&path)
                        })
                        .await;
                        match store.delete_recording(&recording.id).await {
                            Ok(true) => report.conference_recordings_deleted += 1,
                            Ok(false) => {}
                            Err(e) => report
                                .errors
                                .push(format!("conference recording {}: {e}", recording.id)),
                        }
                    }
                }
                Err(e) => report.errors.push(format!("conference recordings: {e}")),
            }
        }
    }

    if let Some(days) = policy.voicemail_retention_days {
        use crate::storage::VoicemailStore;
        match storage
            .delete_messages_before(&tenant, cutoff(now, days))
            .await
        {
            Ok(n) => report.voicemail_messages_deleted = n,
            Err(e) => report.errors.push(format!("voicemail: {e}")),
        }
    }

    if let Some(days) = policy.conference_chat_retention_days {
        match storage.compliance_store() {
            Some(store) => match store
                .delete_conference_chat_before(&tenant, cutoff(now, days))
                .await
            {
                Ok(n) => report.conference_chat_deleted = n,
                Err(e) => report.errors.push(format!("conference chat: {e}")),
            },
            None => report
                .errors
                .push("conference chat: store not configured".to_string()),
        }
    }

    if let Some(days) = policy.webinar_question_retention_days {
        match storage.compliance_store() {
            Some(store) => match store
                .delete_webinar_questions_before(&tenant, cutoff(now, days))
                .await
            {
                Ok(n) => report.webinar_questions_deleted = n,
                Err(e) => report.errors.push(format!("webinar questions: {e}")),
            },
            None => report
                .errors
                .push("webinar questions: store not configured".to_string()),
        }
    }

    if let Some(days) = policy.webinar_registrant_retention_days {
        match storage.compliance_store() {
            Some(store) => match store
                .delete_webinar_registrants_before(&tenant, cutoff(now, days))
                .await
            {
                Ok(n) => report.webinar_registrants_deleted = n,
                Err(e) => report.errors.push(format!("webinar registrations: {e}")),
            },
            None => report
                .errors
                .push("webinar registrations: store not configured".to_string()),
        }
    }

    match cdr_retention_cutoff(storage, policy, now).await {
        Ok((cutoff, held)) => {
            if let Some(before) = cutoff {
                match storage.delete_cdrs_before(&tenant, before).await {
                    Ok(n) => report.cdrs_deleted = n,
                    Err(e) => report.errors.push(format!("cdrs: {e}")),
                }
            }
            if let Some(window) = held {
                let at = |t: Option<SystemTime>| {
                    t.map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
                        .unwrap_or_else(|| "the start".to_string())
                };
                report.errors.push(format!(
                    "cdrs: kept from {} until the archive confirms them (retention reaches {})",
                    at(cutoff),
                    at(Some(window)),
                ));
            }
        }
        Err(e) => report.errors.push(format!("cdrs: archive frontier: {e}")),
    }

    if let Some(days) = policy.audit_retention_days {
        match storage.compliance_store() {
            Some(store) => match store
                .delete_audit_events_before(&tenant, cutoff(now, days))
                .await
            {
                Ok(n) => report.audit_events_deleted = n,
                Err(e) => report.errors.push(format!("audit: {e}")),
            },
            None => report
                .errors
                .push("audit: store not configured".to_string()),
        }
    }

    // What holds kept, for the report (docs/COMPLIANCE_FOLLOWUPS.md §5).
    if let Some(store) = storage.compliance_store() {
        use crate::legal_hold::DataClass;
        match store.list_holds(&tenant, false).await {
            Ok(holds) if !holds.is_empty() => {
                for (class, days) in [
                    (DataClass::Cdr, policy.cdr_retention_days),
                    (DataClass::SipTrace, policy.sip_trace_retention_days),
                    (DataClass::FlowTrace, policy.flow_trace_retention_days),
                    (DataClass::CallRecording, policy.recording_retention_days),
                    (
                        DataClass::ConferenceRecording,
                        policy.recording_retention_days,
                    ),
                    (DataClass::Voicemail, policy.voicemail_retention_days),
                    (
                        DataClass::ConferenceChat,
                        policy.conference_chat_retention_days,
                    ),
                    (
                        DataClass::WebinarQuestion,
                        policy.webinar_question_retention_days,
                    ),
                    (
                        DataClass::WebinarRegistrant,
                        policy.webinar_registrant_retention_days,
                    ),
                    (DataClass::AuditEvent, policy.audit_retention_days),
                ] {
                    let Some(days) = days else { continue };
                    match store.held_before(&tenant, class, cutoff(now, days)).await {
                        Ok(0) => {}
                        Ok(n) => {
                            report.held.insert(class.as_str().to_string(), n);
                        }
                        Err(e) => report.errors.push(format!("held {}: {e}", class.as_str())),
                    }
                }
            }
            Ok(_) => {}
            Err(e) => report.errors.push(format!("legal holds: {e}")),
        }
    }

    report
}

// ── Platform upkeep (docs/COMPLIANCE_FOLLOWUPS.md §4) ──────────────────

/// What the platform keeps tidy, whatever the tenants' policies say.
/// Files a site holds, removed through its link
/// (`docs/HYBRID_CONTROL_PLANE.md` §3.3): the cloud queues the request,
/// the site's link collects it and removes the file.
#[async_trait::async_trait]
pub trait SiteFileRemover: Send + Sync + std::fmt::Debug {
    /// Ask `site` to remove `path`; `Ok` once the request is queued for
    /// the link, which answers on its own time.
    async fn remove_at_site(
        &self,
        site: crate::SiteId,
        tenant: Option<TenantId>,
        class: &str,
        path: &str,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct UpkeepConfig {
    /// Who removes a file a site holds; none where no site links here.
    pub site_files: Option<std::sync::Arc<dyn SiteFileRemover>>,
    /// The directories files may be removed from: the recordings and
    /// voicemail directories this host mounts. A queued path outside them
    /// is never touched.
    pub file_roots: Vec<std::path::PathBuf>,
    /// Remove files under the roots that no row names once they are this
    /// old; `None` leaves them.
    pub orphan_files_after: Option<Duration>,
    /// Remove SIP traces of no tenant's call once they are this old.
    pub system_traces_after: Option<Duration>,
    /// Remove voicemail messages this long after a user deleted them.
    pub deleted_voicemail_grace: Duration,
    /// Files handled per round.
    pub batch: usize,
}

impl Default for UpkeepConfig {
    fn default() -> Self {
        Self {
            site_files: None,
            file_roots: Vec::new(),
            orphan_files_after: None,
            system_traces_after: None,
            deleted_voicemail_grace: Duration::from_secs(7 * 86_400),
            batch: 500,
        }
    }
}

/// What one upkeep round did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UpkeepReport {
    pub sip_traces_adopted: u64,
    pub flow_traces_adopted: u64,
    pub system_traces_deleted: u64,
    pub voicemail_expired: u64,
    pub voicemail_purged: u64,
    pub files_removed: u64,
    /// Files a site holds, whose removal was asked of the site through
    /// its link (`docs/HYBRID_CONTROL_PLANE.md` §3.3).
    pub files_asked_of_sites: u64,
    /// Queued files kept because a row still names them.
    pub files_kept: u64,
    pub orphan_files_removed: u64,
    pub errors: Vec<String>,
}

/// One round of platform upkeep, every deletion attributed to `actor`.
pub async fn run_upkeep(storage: &DynStorage, config: &UpkeepConfig, actor: &str) -> UpkeepReport {
    deleting_as(
        Deleter::new(DeletionCause::Upkeep, actor),
        upkeep(storage, config, actor),
    )
    .await
}

async fn upkeep(storage: &DynStorage, config: &UpkeepConfig, actor: &str) -> UpkeepReport {
    use crate::storage::VoicemailStore;
    let now = SystemTime::now();
    let mut report = UpkeepReport::default();

    if let Some(store) = storage.sip_trace_store() {
        match store.adopt_system_traces().await {
            Ok(n) => report.sip_traces_adopted = n,
            Err(e) => report.errors.push(format!("sip traces: adopt: {e}")),
        }
        if let Some(after) = config.system_traces_after {
            match store.delete_system_traces_before(now - after).await {
                Ok(n) => report.system_traces_deleted = n,
                Err(e) => report.errors.push(format!("sip traces: {e}")),
            }
        }
    }
    if let Some(store) = storage.flow_trace_store() {
        match store.adopt_untenanted_traces().await {
            Ok(n) => report.flow_traces_adopted = n,
            Err(e) => report.errors.push(format!("flow traces: adopt: {e}")),
        }
    }
    match storage.apply_mailbox_retention().await {
        Ok(n) => report.voicemail_expired = n,
        Err(e) => report
            .errors
            .push(format!("voicemail: mailbox retention: {e}")),
    }
    match storage
        .purge_deleted_messages(now - config.deleted_voicemail_grace)
        .await
    {
        Ok(n) => report.voicemail_purged = n,
        Err(e) => report.errors.push(format!("voicemail: purge: {e}")),
    }

    let Some(store) = storage.compliance_store() else {
        report
            .errors
            .push("files: compliance store not configured".to_string());
        return report;
    };
    reap_files(store.as_ref(), config, &mut report).await;
    if let Some(after) = config.orphan_files_after {
        remove_orphan_files(store.as_ref(), config, now - after, actor, &mut report).await;
    }
    report
}

/// Whether `path` is inside one of `roots`, without climbing out of it.
pub fn under_file_root(path: &Path, roots: &[std::path::PathBuf]) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        && roots.iter().any(|root| path.starts_with(root))
}

async fn reap_files(store: &dyn ComplianceStore, config: &UpkeepConfig, report: &mut UpkeepReport) {
    let due = match store.due_file_deletions(config.batch).await {
        Ok(due) => due,
        Err(e) => {
            report.errors.push(format!("files: {e}"));
            return;
        }
    };
    for entry in due {
        let outcome = if entry.still_referenced {
            report.files_kept += 1;
            Ok(())
        } else if let Some(site) = entry.site_id {
            // The site holds the file: asked of it through its link, and
            // the entry is done once the request is queued.
            match &config.site_files {
                Some(sites) => sites
                    .remove_at_site(site, entry.tenant_id, &entry.class, &entry.path)
                    .await
                    .map(|()| report.files_asked_of_sites += 1),
                None => Err(format!("held at site {site}, and no link runs here")),
            }
        } else if !under_file_root(Path::new(&entry.path), &config.file_roots) {
            Err("not under a configured file root".to_string())
        } else {
            // A webinar's stream is a directory (`docs/WEBINARS.md` §2.8).
            let removed = match tokio::fs::metadata(&entry.path).await {
                Ok(meta) if meta.is_dir() => tokio::fs::remove_dir_all(&entry.path).await,
                _ => tokio::fs::remove_file(&entry.path).await,
            };
            match removed {
                Ok(()) => {
                    report.files_removed += 1;
                    Ok(())
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        };
        let recorded = match outcome {
            Ok(()) => store.file_deletion_done(entry.id).await,
            Err(error) => {
                // A minute, doubling, at most a day.
                let retry = Duration::from_secs(60 * 2u64.pow(entry.attempts.min(10)))
                    .min(Duration::from_secs(86_400));
                report.errors.push(format!("file {}: {error}", entry.path));
                store.file_deletion_failed(entry.id, &error, retry).await
            }
        };
        if let Err(e) = recorded {
            report.errors.push(format!("file {}: {e}", entry.path));
        }
    }
}

async fn remove_orphan_files(
    store: &dyn ComplianceStore,
    config: &UpkeepConfig,
    before: SystemTime,
    actor: &str,
    report: &mut UpkeepReport,
) {
    for root in &config.file_roots {
        let candidates = match files_older_than(root, before).await {
            Ok(files) => files,
            Err(e) => {
                report
                    .errors
                    .push(format!("orphans {}: {e}", root.display()));
                continue;
            }
        };
        for chunk in candidates.chunks(config.batch.max(1)) {
            let paths: Vec<String> = chunk
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            let referenced = match store.referenced_paths(&paths).await {
                Ok(r) => r,
                Err(e) => {
                    report.errors.push(format!("orphans: {e}"));
                    return;
                }
            };
            for path in paths.iter().filter(|p| !referenced.contains(p)) {
                match tokio::fs::remove_file(path).await {
                    Ok(()) => {
                        report.orphan_files_removed += 1;
                        let record = DeletionRecord {
                            id: 0,
                            tenant_id: None,
                            class: "orphan_file".to_string(),
                            item: path.clone(),
                            detail: None,
                            cause: DeletionCause::Upkeep.as_str().to_string(),
                            actor: actor.to_string(),
                            request_id: None,
                            api_request_id: None,
                            via: None,
                            deleted_at: SystemTime::now(),
                        };
                        if let Err(e) = store.record_deletion(&record).await {
                            report.errors.push(format!("orphan {path}: log: {e}"));
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => report.errors.push(format!("orphan {path}: {e}")),
                }
            }
        }
    }
}

/// Regular files under `dir` (recursively) last modified before `before`.
pub async fn files_older_than(
    dir: &Path,
    before: SystemTime,
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&d).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let meta = entry.metadata().await?;
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() && meta.modified().map(|m| m < before).unwrap_or(false) {
                found.push(path);
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_consent_outcome_follows_the_plan_and_the_failure_rule() {
        use crate::recording::RecordingConsent;
        let none = ConsentPlan::not_required();
        assert_eq!(none.outcome(false), RecordingConsent::NotRequired);
        assert_eq!(none.outcome(true), RecordingConsent::NotRequired);

        let record = ConsentPlan {
            announce: true,
            ..ConsentPlan::not_required()
        };
        assert_eq!(record.outcome(true), RecordingConsent::Announced);
        assert_eq!(record.outcome(false), RecordingConsent::Failed);

        let skip = ConsentPlan {
            announce: true,
            on_failure: ConsentFailureAction::Skip,
            ..ConsentPlan::not_required()
        };
        assert_eq!(skip.outcome(true), RecordingConsent::Announced);
        assert_eq!(skip.outcome(false), RecordingConsent::Refused);

        assert_eq!(
            ConsentFailureAction::default(),
            ConsentFailureAction::Record
        );
        for a in [ConsentFailureAction::Record, ConsentFailureAction::Skip] {
            assert_eq!(ConsentFailureAction::parse(a.as_str()), Some(a));
        }
        assert_eq!(ConsentFailureAction::parse("maybe"), None);
    }

    #[test]
    fn the_notice_tone_is_a_short_quiet_beep() {
        let tone = notice_tone_samples(8000);
        assert_eq!(tone.len(), 8000 * NOTICE_TONE_MS as usize / 1000);
        let peak = tone.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(
            (7000..=8300).contains(&peak),
            "a quarter of full scale: {peak}"
        );
        assert!(tone[0].unsigned_abs() < 100 && tone[tone.len() - 1].unsigned_abs() < 500);
        assert_eq!(notice_tone_samples(48000).len(), 48000 * 175 / 1000);
    }

    #[test]
    fn recording_counters_render_attempts_and_starts() {
        use crate::recording::RecordingConsent;
        let counters = RecordingCounters::default();
        counters.note("user", RecordingConsent::Announced);
        counters.note("user", RecordingConsent::Announced);
        counters.note("user", RecordingConsent::Refused);
        counters.note("queue", RecordingConsent::NotRequired);
        let text = counters.prometheus("node=\"n1\"");
        assert!(text.contains(
            "fcp_call_recording_consent_total{node=\"n1\",source=\"user\",consent=\"announced\"} 2"
        ));
        assert!(text.contains(
            "fcp_call_recording_consent_total{node=\"n1\",source=\"user\",consent=\"refused\"} 1"
        ));
        assert!(text.contains("fcp_call_recordings_started_total{node=\"n1\",source=\"user\"} 2"));
        assert!(text.contains("fcp_call_recordings_started_total{node=\"n1\",source=\"queue\"} 1"));
        assert!(RecordingCounters::default()
            .prometheus("")
            .contains("# TYPE fcp_call_recordings_started_total counter"));
    }

    #[test]
    fn defaults_keep_everything() {
        let p = CompliancePolicy::defaults(TenantId::nil());
        assert!(!p.has_retention());
        assert_eq!(p.recording_consent, RecordingConsentMode::None);
        assert_eq!(
            serde_json::to_value(RecordingConsentMode::Announce).unwrap(),
            serde_json::json!("announce")
        );
    }

    #[test]
    fn cutoff_is_days_ago() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 86_400);
        assert_eq!(
            cutoff(now, 30),
            SystemTime::UNIX_EPOCH + Duration::from_secs(70 * 86_400)
        );
    }

    #[tokio::test]
    async fn only_old_files_are_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("tenant");
        tokio::fs::create_dir_all(&sub).await.unwrap();
        let old = sub.join("old.wav");
        let new = dir.path().join("new.wav");
        tokio::fs::write(&old, b"x").await.unwrap();
        tokio::fs::write(&new, b"y").await.unwrap();
        let past = SystemTime::now() - Duration::from_secs(10 * 86_400);
        filetime_set(&old, past);

        let before = SystemTime::now() - Duration::from_secs(86_400);
        assert_eq!(
            files_older_than(dir.path(), before).await.unwrap(),
            vec![old]
        );
        // A missing directory is not an error.
        assert!(files_older_than(&dir.path().join("nope"), before)
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn files_are_removed_only_under_their_roots() {
        let roots = vec![std::path::PathBuf::from("/srv/recordings")];
        assert!(under_file_root(
            Path::new("/srv/recordings/t/a.wav"),
            &roots
        ));
        assert!(!under_file_root(
            Path::new("/srv/recordings-other/a.wav"),
            &roots
        ));
        assert!(!under_file_root(
            Path::new("/srv/recordings/../etc/passwd"),
            &roots
        ));
        assert!(!under_file_root(Path::new("recordings/a.wav"), &roots));
        assert!(!under_file_root(Path::new("/etc/passwd"), &roots));
    }

    #[tokio::test]
    async fn deletions_are_attributed_inside_their_scope_only() {
        assert_eq!(current_deleter(), None);
        let inside = deleting_as(Deleter::new(DeletionCause::Manual, "alice"), async {
            current_deleter()
        })
        .await;
        assert_eq!(inside, Some(Deleter::new(DeletionCause::Manual, "alice")));
        assert_eq!(current_deleter(), None);
    }

    /// A record store whose archive is confirmed up to a fixed instant.
    struct Archived(Option<SystemTime>);

    #[async_trait]
    impl CdrStore for Archived {
        async fn insert_cdr(&self, _: crate::storage::CallDetailRecord) -> anyhow::Result<()> {
            Ok(())
        }
        async fn get_cdr(
            &self,
            _: &crate::CallId,
        ) -> anyhow::Result<Option<crate::storage::CallDetailRecord>> {
            Ok(None)
        }
        async fn query_cdrs(
            &self,
            _: crate::storage::CdrQuery,
        ) -> anyhow::Result<Vec<crate::storage::CallDetailRecord>> {
            Ok(Vec::new())
        }
        async fn count_cdrs(&self, _: crate::storage::CdrQuery) -> anyhow::Result<usize> {
            Ok(0)
        }
        async fn cdr_archive_frontier(&self, _: &TenantId) -> anyhow::Result<Option<SystemTime>> {
            Ok(self.0)
        }
    }

    #[tokio::test]
    async fn the_archive_holds_cdr_retention_back_only_when_required() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 86_400);
        let day = |d: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(d * 86_400);
        let mut policy = CompliancePolicy::defaults(TenantId::nil());

        // No window: nothing is deleted, archive or not.
        assert_eq!(
            cdr_retention_cutoff(&Archived(Some(now)), &policy, now)
                .await
                .unwrap(),
            (None, None)
        );

        // A window without the requirement ignores the archive.
        policy.cdr_retention_days = Some(30);
        assert_eq!(
            cdr_retention_cutoff(&Archived(None), &policy, now)
                .await
                .unwrap(),
            (Some(day(70)), None)
        );

        // Required: the frontier caps the window, and nothing archived
        // keeps everything.
        policy.cdr_archive_required = true;
        for (frontier, expected) in [
            (Some(day(90)), (Some(day(70)), None)),
            (Some(day(70)), (Some(day(70)), None)),
            (Some(day(60)), (Some(day(60)), Some(day(70)))),
            (None, (None, Some(day(70)))),
        ] {
            assert_eq!(
                cdr_retention_cutoff(&Archived(frontier), &policy, now)
                    .await
                    .unwrap(),
                expected,
                "frontier {frontier:?}"
            );
        }

        // Policies written before the flag existed read as not required.
        let mut json = serde_json::to_value(&policy).unwrap();
        json.as_object_mut().unwrap().remove("cdr_archive_required");
        let old: CompliancePolicy = serde_json::from_value(json).unwrap();
        assert!(!old.cdr_archive_required);
    }

    /// Set a file's mtime without pulling in the `filetime` crate.
    fn filetime_set(path: &Path, when: SystemTime) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(when).unwrap();
    }
}
