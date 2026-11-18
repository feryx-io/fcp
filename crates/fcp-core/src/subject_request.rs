//! Subject requests (`docs/COMPLIANCE_FOLLOWUPS.md` §6): a person's right to
//! the data a tenant holds about them, filed by the tenant and run by a
//! worker against a deadline.
//!
//! A request names its subjects; they are expanded once, when the request
//! starts ([`crate::legal_hold::expand`]), and the expansion stays on the
//! request. An export walks every store for data that is the subject's —
//! narrowly, by [`crate::subject::SubjectMatcher`] — and writes a bundle
//! the tenant downloads: `manifest.json`, `data/<class>.jsonl` and
//! `media/<class>/<id>.<ext>`.

use crate::legal_hold::{SubjectExpansion, SubjectIdentifier};
use crate::{SubjectRequestId, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

/// How long a tenant has to answer a request unless its policy says
/// otherwise.
pub const DEFAULT_REQUEST_DAYS: u32 = 30;

/// How long a finished export's bundle is kept for download.
pub const DEFAULT_EXPORT_TTL_DAYS: u32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectRequestKind {
    /// Everything held about the subject, as a bundle.
    Export,
    /// The subject's data removed or pseudonymised, after a preview a
    /// person confirms.
    Erasure,
    /// Every piece of a tenant's data removed, then the tenant.
    TenantDeletion,
}

impl SubjectRequestKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubjectRequestKind::Export => "export",
            SubjectRequestKind::Erasure => "erasure",
            SubjectRequestKind::TenantDeletion => "tenant_deletion",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "export" => Some(SubjectRequestKind::Export),
            "erasure" => Some(SubjectRequestKind::Erasure),
            "tenant_deletion" => Some(SubjectRequestKind::TenantDeletion),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectRequestStatus {
    /// Filed, waiting for the worker.
    Received,
    /// The worker is on it.
    Running,
    /// An erasure's preview is ready, waiting for a person to confirm it.
    Preview,
    /// Confirmed, waiting for the worker to erase.
    Confirmed,
    Completed,
    Failed,
}

impl SubjectRequestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubjectRequestStatus::Received => "received",
            SubjectRequestStatus::Running => "running",
            SubjectRequestStatus::Preview => "preview",
            SubjectRequestStatus::Confirmed => "confirmed",
            SubjectRequestStatus::Completed => "completed",
            SubjectRequestStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "received" => Some(SubjectRequestStatus::Received),
            "running" => Some(SubjectRequestStatus::Running),
            "preview" => Some(SubjectRequestStatus::Preview),
            "confirmed" => Some(SubjectRequestStatus::Confirmed),
            "completed" => Some(SubjectRequestStatus::Completed),
            "failed" => Some(SubjectRequestStatus::Failed),
            _ => None,
        }
    }

    /// Still owed an answer.
    pub fn is_open(&self) -> bool {
        matches!(
            self,
            SubjectRequestStatus::Received
                | SubjectRequestStatus::Running
                | SubjectRequestStatus::Preview
                | SubjectRequestStatus::Confirmed
        )
    }
}

/// What one class of data gave up to an export.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClassReport {
    /// Items that are the subject's.
    pub items: u64,
    /// Media files put in the bundle.
    #[serde(default)]
    pub files: u64,
    /// Media files that are the subject's but could not be read from this
    /// host, with why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_files: Vec<MissingFile>,
    /// Items under a legal hold (exported like the rest).
    #[serde(default)]
    pub held: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MissingFile {
    pub item: String,
    pub path: String,
    pub reason: String,
}

/// What an export found, class by class, and what it could not reach.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ExportReport {
    pub classes: BTreeMap<String, ClassReport>,
    /// Stores the export does not reach, with why (transit stores, operator
    /// stores, backups).
    pub not_exported: BTreeMap<String, String>,
    /// Recordings in which other people are heard, for the tenant to review
    /// before handing the bundle over (`media/<class>/<id>.<ext>`).
    #[serde(default)]
    pub review: Vec<String>,
    /// The holds covering exported items, by id, with their matters.
    #[serde(default)]
    pub held_by: BTreeMap<String, String>,
}

/// A finished export's bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleRef {
    /// The file on the exporting host.
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub expires_at: SystemTime,
    /// Removed after `expires_at`.
    #[serde(default)]
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubjectRequest {
    pub id: SubjectRequestId,
    pub tenant_id: TenantId,
    pub kind: SubjectRequestKind,
    pub subjects: Vec<SubjectIdentifier>,
    /// The subjects as they were when the request started.
    #[serde(default)]
    pub expansion: Option<SubjectExpansion>,
    pub status: SubjectRequestStatus,
    pub received_at: SystemTime,
    /// When the tenant owes the answer.
    pub due_at: SystemTime,
    pub requested_by: String,
    /// The tenant's own reference (a ticket, a letter).
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub started_at: Option<SystemTime>,
    #[serde(default)]
    pub completed_at: Option<SystemTime>,
    #[serde(default)]
    pub report: Option<ExportReport>,
    #[serde(default)]
    pub bundle: Option<BundleRef>,
    #[serde(default)]
    pub error: Option<String>,
    /// When a person confirmed an erasure's preview, and who.
    #[serde(default)]
    pub confirmed_at: Option<SystemTime>,
    #[serde(default)]
    pub confirmed_by: Option<String>,
    /// What an erasure would do, as the preview found it.
    #[serde(default)]
    pub preview: Option<crate::subject_erasure::ErasureReport>,
    /// What an erasure or a tenant's deletion did.
    #[serde(default)]
    pub erasure: Option<crate::subject_erasure::ErasureReport>,
}

impl SubjectRequest {
    /// A request just filed, due `days` from now.
    pub fn received(
        tenant_id: TenantId,
        kind: SubjectRequestKind,
        subjects: Vec<SubjectIdentifier>,
        requested_by: impl Into<String>,
        days: u32,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            id: SubjectRequestId::new(),
            tenant_id,
            kind,
            subjects,
            expansion: None,
            status: SubjectRequestStatus::Received,
            received_at: now,
            due_at: now + Duration::from_secs(u64::from(days) * 86_400),
            requested_by: requested_by.into(),
            reference: None,
            started_at: None,
            completed_at: None,
            report: None,
            bundle: None,
            error: None,
            confirmed_at: None,
            confirmed_by: None,
            preview: None,
            erasure: None,
        }
    }

    /// Open and past its deadline at `now`.
    pub fn is_overdue(&self, now: SystemTime) -> bool {
        self.status.is_open() && now > self.due_at
    }
}

// ── What an export reads (docs/COMPLIANCE_FOLLOWUPS.md §6.3) ───────────

/// The classes of data an export walks, in the order it walks them: the
/// call records before what hangs off a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportClass {
    /// The user's profile, without secrets.
    Profile,
    ApiTokens,
    Devices,
    LineBindings,
    Extensions,
    QueueMemberships,
    Meetings,
    Mailboxes,
    VoicemailGreetings,
    /// Messages in the subject's mailboxes, and those they left elsewhere.
    VoicemailMessages,
    Cdrs,
    CallRecordings,
    ConferenceRecordings,
    SipTraces,
    FlowTraces,
    Registrations,
    Presence,
    PresenceSubscriptions,
    PresencePolicies,
    ResourceLists,
    PresenceHistory,
    EmergencyCalls,
    /// Where the subject last said they were, for emergency calls
    /// (`docs/E911_BLOCK_2.md` §6.4).
    DeclaredLocations,
    AuditEvents,
    ConfigChanges,
    PortRequests,
    WebhookDeliveries,
    /// Entries naming the subject on the tenant's numbers' allow and block
    /// lists.
    NumberLists,
    /// The contacts the subject keeps (`docs/RECORD_AND_CONTACTS.md` §3).
    PersonalContacts,
    /// Chat the subject posted in a room (`docs/WEBINARS.md` §2.5).
    ConferenceChat,
}

impl ExportClass {
    pub const ALL: [ExportClass; 30] = [
        ExportClass::Profile,
        ExportClass::ApiTokens,
        ExportClass::Devices,
        ExportClass::LineBindings,
        ExportClass::Extensions,
        ExportClass::QueueMemberships,
        ExportClass::Meetings,
        ExportClass::Mailboxes,
        ExportClass::VoicemailGreetings,
        ExportClass::VoicemailMessages,
        ExportClass::Cdrs,
        ExportClass::CallRecordings,
        ExportClass::ConferenceRecordings,
        ExportClass::SipTraces,
        ExportClass::FlowTraces,
        ExportClass::Registrations,
        ExportClass::Presence,
        ExportClass::PresenceSubscriptions,
        ExportClass::PresencePolicies,
        ExportClass::ResourceLists,
        ExportClass::PresenceHistory,
        ExportClass::EmergencyCalls,
        ExportClass::DeclaredLocations,
        ExportClass::AuditEvents,
        ExportClass::ConfigChanges,
        ExportClass::PortRequests,
        ExportClass::WebhookDeliveries,
        ExportClass::NumberLists,
        ExportClass::PersonalContacts,
        ExportClass::ConferenceChat,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ExportClass::Profile => "profile",
            ExportClass::ApiTokens => "api_tokens",
            ExportClass::Devices => "devices",
            ExportClass::LineBindings => "line_bindings",
            ExportClass::Extensions => "extensions",
            ExportClass::QueueMemberships => "queue_memberships",
            ExportClass::Meetings => "meetings",
            ExportClass::Mailboxes => "mailboxes",
            ExportClass::VoicemailGreetings => "voicemail_greetings",
            ExportClass::VoicemailMessages => "voicemail_messages",
            ExportClass::Cdrs => "cdrs",
            ExportClass::CallRecordings => "call_recordings",
            ExportClass::ConferenceRecordings => "conference_recordings",
            ExportClass::SipTraces => "sip_traces",
            ExportClass::FlowTraces => "flow_traces",
            ExportClass::Registrations => "registrations",
            ExportClass::Presence => "presence",
            ExportClass::PresenceSubscriptions => "presence_subscriptions",
            ExportClass::PresencePolicies => "presence_policies",
            ExportClass::ResourceLists => "resource_lists",
            ExportClass::PresenceHistory => "presence_history",
            ExportClass::EmergencyCalls => "emergency_calls",
            ExportClass::DeclaredLocations => "declared_locations",
            ExportClass::AuditEvents => "audit_events",
            ExportClass::ConfigChanges => "config_changes",
            ExportClass::PortRequests => "port_requests",
            ExportClass::WebhookDeliveries => "webhook_deliveries",
            ExportClass::NumberLists => "number_lists",
            ExportClass::PersonalContacts => "personal_contacts",
            ExportClass::ConferenceChat => "conference_chat",
        }
    }

    /// The legal-hold class this export class's items belong to, if any.
    pub fn hold_class(&self) -> Option<crate::legal_hold::DataClass> {
        use crate::legal_hold::DataClass;
        match self {
            ExportClass::Cdrs => Some(DataClass::Cdr),
            ExportClass::CallRecordings => Some(DataClass::CallRecording),
            ExportClass::ConferenceRecordings => Some(DataClass::ConferenceRecording),
            ExportClass::VoicemailMessages => Some(DataClass::Voicemail),
            ExportClass::SipTraces => Some(DataClass::SipTrace),
            ExportClass::FlowTraces => Some(DataClass::FlowTrace),
            ExportClass::AuditEvents => Some(DataClass::AuditEvent),
            ExportClass::ConferenceChat => Some(DataClass::ConferenceChat),
            _ => None,
        }
    }

    /// Classes whose items are found by the calls the subject's records
    /// name.
    pub fn by_call(&self) -> bool {
        matches!(
            self,
            ExportClass::CallRecordings | ExportClass::SipTraces | ExportClass::FlowTraces
        )
    }
}

/// What a store is asked for: the subject as the request expanded them, and
/// the calls their records named.
#[derive(Debug, Clone, Default)]
pub struct SubjectScope {
    /// Every spelling of the subject.
    pub identifiers: Vec<String>,
    /// Their canonical keys, for a store to narrow candidates by.
    pub keys: Vec<String>,
    /// The identifiers without a host: usernames and extensions.
    pub names: Vec<String>,
    pub user_ids: Vec<String>,
    pub mailbox_ids: Vec<String>,
    /// Calls (`call-…`) whose records are the subject's.
    pub call_ids: Vec<String>,
}

impl SubjectScope {
    pub fn from_expansion(expansion: &SubjectExpansion) -> Self {
        Self {
            identifiers: expansion.identifiers.clone(),
            keys: expansion.keys(),
            names: expansion
                .identifiers
                .iter()
                .filter(|i| !i.contains('@'))
                .cloned()
                .collect(),
            user_ids: expansion.user_ids.iter().map(|u| u.to_string()).collect(),
            mailbox_ids: expansion.mailbox_ids.clone(),
            call_ids: Vec::new(),
        }
    }
}

/// Why a row a store returned is the subject's, or may be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowLink {
    /// It belongs to the subject's user or mailbox.
    Owned,
    /// It hangs off this call, whose record is the subject's.
    Call(String),
    /// It is the subject's if one of these identifiers is.
    Identifiers(Vec<String>),
}

/// One candidate item.
#[derive(Debug, Clone)]
pub struct SubjectRow {
    /// Unique within its class.
    pub id: String,
    pub link: RowLink,
    /// What the bundle carries for it, secrets left out by the store.
    pub data: serde_json::Value,
    /// When the item is dated, for a hold's window.
    pub at: Option<SystemTime>,
    /// The call it belongs to, when it has one.
    pub call_id: Option<String>,
    /// Its audio or video, when it has a file.
    pub file: Option<String>,
    /// Whether people other than the subject may be heard in the file.
    pub others_heard: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SubjectPage {
    pub rows: Vec<SubjectRow>,
    /// Where the next page starts; `None` on the last.
    pub next: Option<String>,
}

/// Where an export's candidates come from.
#[async_trait::async_trait]
pub trait SubjectDataStore: Send + Sync {
    /// A page of `class`'s candidates in `tenant` for `scope`, after
    /// `after`. A superset: the export decides which rows are the subject's.
    async fn subject_rows(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        scope: &SubjectScope,
        after: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<SubjectPage>;
}

/// Brings an item's file into the bundle.
#[async_trait::async_trait]
pub trait MediaFetcher: Send + Sync {
    /// Copy `row`'s file to `dest`; the bytes written, or why not.
    async fn fetch(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        row: &SubjectRow,
        dest: &std::path::Path,
    ) -> Result<u64, String>;
}

/// Files this host can read: those under the directories it mounts.
#[derive(Debug, Clone, Default)]
pub struct LocalMedia {
    pub roots: Vec<std::path::PathBuf>,
    /// Where profile pictures are, which a user's row names by file name.
    pub profile_pictures_dir: Option<std::path::PathBuf>,
}

#[async_trait::async_trait]
impl MediaFetcher for LocalMedia {
    async fn fetch(
        &self,
        _tenant: &TenantId,
        class: ExportClass,
        row: &SubjectRow,
        dest: &std::path::Path,
    ) -> Result<u64, String> {
        let file = row.file.as_deref().ok_or("no file")?;
        let mut path = std::path::PathBuf::from(file.strip_prefix("file://").unwrap_or(file));
        if class == ExportClass::Profile && path.is_relative() {
            let dir = self
                .profile_pictures_dir
                .as_ref()
                .ok_or("no profile pictures directory on this host")?;
            if path.components().count() != 1 {
                return Err("not a file name".to_string());
            }
            path = dir.join(path);
            return tokio::fs::copy(&path, dest)
                .await
                .map_err(|e| e.to_string());
        }
        if !crate::compliance::under_file_root(&path, &self.roots) {
            return Err("not under a directory this host mounts".to_string());
        }
        tokio::fs::copy(&path, dest)
            .await
            .map_err(|e| e.to_string())
    }
}

/// What the export stores and the pieces of the platform it cannot reach.
pub fn not_exported() -> BTreeMap<String, String> {
    [
        (
            "event_stream",
            "events in transit on the bus are not stored for export",
        ),
        (
            "cdr_stream",
            "records in transit to the processor; each is in cdrs once stored",
        ),
        (
            "cdr_archive",
            "the archive holds copies of the records in cdrs",
        ),
        (
            "dead_letters",
            "records that failed processing stay on the node or in Redis for the operator",
        ),
        (
            "parked_calls_and_callbacks",
            "live call state, gone when the call ends",
        ),
        (
            "caches",
            "TTS, API gateway and credential caches are rebuilt from the stores",
        ),
        (
            "logs",
            "service logs are the operator's, kept under its own retention",
        ),
        (
            "backups",
            "database backups and bucket versions age out under the operator's retention",
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// An export's report and the manifest it wrote.
pub struct ExportOutcome {
    pub report: ExportReport,
}

/// How many candidates a store is asked for at once, and how many calls a
/// call-linked class looks up at once.
const PAGE: usize = 500;
const CALLS_PER_QUERY: usize = 1000;

/// Walk every class for what is `request`'s subject's, writing the bundle's
/// contents under `staging`: `data/<class>.jsonl`, `media/<class>/<file>` and
/// `manifest.json`. Items under `holds` are exported and counted.
pub async fn run_export(
    data: &dyn SubjectDataStore,
    media: &dyn MediaFetcher,
    holds: &[crate::legal_hold::LegalHold],
    request: &SubjectRequest,
    expansion: &SubjectExpansion,
    staging: &std::path::Path,
) -> anyhow::Result<ExportOutcome> {
    use tokio::io::AsyncWriteExt;

    let matcher = expansion.matcher();
    let mut scope = SubjectScope::from_expansion(expansion);
    let mut calls = std::collections::BTreeSet::new();
    let mut report = ExportReport {
        not_exported: not_exported(),
        ..Default::default()
    };
    let mut files: Vec<ManifestFile> = Vec::new();
    let mut held_by: BTreeMap<String, String> = BTreeMap::new();
    tokio::fs::create_dir_all(staging.join("data")).await?;

    for class in ExportClass::ALL {
        if class.by_call() {
            scope.call_ids = calls.iter().cloned().collect();
            if scope.call_ids.is_empty() {
                continue;
            }
        }
        let chunks: Vec<Vec<String>> = if class.by_call() {
            scope
                .call_ids
                .chunks(CALLS_PER_QUERY)
                .map(|c| c.to_vec())
                .collect()
        } else {
            vec![Vec::new()]
        };
        let mut class_report = ClassReport::default();
        let mut out: Option<tokio::fs::File> = None;
        for chunk in chunks {
            let mut chunk_scope = scope.clone();
            if class.by_call() {
                chunk_scope.call_ids = chunk;
            }
            let mut after: Option<String> = None;
            loop {
                let page = data
                    .subject_rows(
                        &request.tenant_id,
                        class,
                        &chunk_scope,
                        after.as_deref(),
                        PAGE,
                    )
                    .await?;
                for row in &page.rows {
                    if !is_theirs(row, &matcher, &calls) {
                        continue;
                    }
                    if class == ExportClass::Cdrs {
                        if let Some(call) = &row.call_id {
                            calls.insert(call.clone());
                        }
                    }
                    class_report.items += 1;

                    if let Some(hold) = hold_over(holds, class, row) {
                        class_report.held += 1;
                        held_by.insert(hold.id.to_string(), hold.matter.clone());
                    }

                    let mut line = row.data.clone();
                    if let Some(file) = &row.file {
                        let name = media_name(&row.id, file);
                        let relative = format!("media/{}/{}", class.as_str(), name);
                        let dest = staging.join(&relative);
                        if let Some(dir) = dest.parent() {
                            tokio::fs::create_dir_all(dir).await?;
                        }
                        match media.fetch(&request.tenant_id, class, row, &dest).await {
                            Ok(_) => {
                                class_report.files += 1;
                                if row.others_heard {
                                    report.review.push(relative.clone());
                                }
                                if let serde_json::Value::Object(map) = &mut line {
                                    map.insert("bundle_file".into(), relative.clone().into());
                                }
                            }
                            Err(reason) => {
                                let _ = tokio::fs::remove_file(&dest).await;
                                class_report.missing_files.push(MissingFile {
                                    item: row.id.clone(),
                                    path: file.clone(),
                                    reason,
                                });
                            }
                        }
                    }
                    if out.is_none() {
                        out = Some(
                            tokio::fs::File::create(
                                staging.join(format!("data/{}.jsonl", class.as_str())),
                            )
                            .await?,
                        );
                    }
                    let mut bytes = serde_json::to_vec(&line)?;
                    bytes.push(b'\n');
                    if let Some(f) = out.as_mut() {
                        f.write_all(&bytes).await?;
                    }
                }
                match page.next {
                    Some(next) if !page.rows.is_empty() => after = Some(next),
                    _ => break,
                }
            }
        }
        if let Some(mut f) = out {
            f.flush().await?;
            files.push(ManifestFile::of(staging, &format!("data/{}.jsonl", class.as_str())).await?);
        }
        if class_report.items > 0 || !class_report.missing_files.is_empty() {
            report
                .classes
                .insert(class.as_str().to_string(), class_report);
        }
    }

    // Every media file, checksummed.
    for class in ExportClass::ALL {
        let dir = staging.join("media").join(class.as_str());
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        let mut names = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        for name in names {
            files.push(
                ManifestFile::of(staging, &format!("media/{}/{}", class.as_str(), name)).await?,
            );
        }
    }
    report.held_by = held_by;

    let manifest = serde_json::json!({
        "format": "fcp-subject-export/1",
        "request": {
            "id": request.id.to_string(),
            "tenant_id": request.tenant_id.to_string(),
            "kind": request.kind.as_str(),
            "subjects": request.subjects,
            "reference": request.reference,
            "requested_by": request.requested_by,
            "received_at": rfc3339(request.received_at),
            "due_at": rfc3339(request.due_at),
        },
        "expansion": expansion,
        "generated_at": rfc3339(SystemTime::now()),
        "classes": report.classes,
        "files": files,
        "not_exported": report.not_exported,
        "review": report.review,
        "held_by": report.held_by,
    });
    tokio::fs::write(
        staging.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )
    .await?;
    Ok(ExportOutcome { report })
}

/// Whether a candidate row is the subject's: owned by them, of one of their
/// calls, or naming them.
pub(crate) fn is_theirs(
    row: &SubjectRow,
    matcher: &crate::subject::SubjectMatcher,
    calls: &std::collections::BTreeSet<String>,
) -> bool {
    match &row.link {
        RowLink::Owned => true,
        RowLink::Call(call) => calls.contains(call),
        RowLink::Identifiers(ids) => matcher.matches_any(ids.iter().map(String::as_str)),
    }
}

/// The first of `holds` covering a row of `class`, if any.
pub(crate) fn hold_over<'h>(
    holds: &'h [crate::legal_hold::LegalHold],
    class: ExportClass,
    row: &SubjectRow,
) -> Option<&'h crate::legal_hold::LegalHold> {
    let (hold_class, at) = (class.hold_class()?, row.at?);
    let keys = match &row.link {
        RowLink::Identifiers(ids) => Some(
            ids.iter()
                .flat_map(|i| crate::subject::keys(i))
                .collect::<Vec<_>>(),
        ),
        _ => None,
    };
    let item = crate::legal_hold::DataRef {
        class: hold_class,
        at,
        keys,
        call_id: row.call_id.clone(),
    };
    holds.iter().find(|h| h.covers(&item))
}

/// A file named in the manifest.
#[derive(Debug, Clone, Serialize)]
struct ManifestFile {
    path: String,
    bytes: u64,
    sha256: String,
}

impl ManifestFile {
    async fn of(staging: &std::path::Path, relative: &str) -> anyhow::Result<Self> {
        let path = staging.join(relative);
        let (bytes, sha256) = sha256_file(&path).await?;
        Ok(Self {
            path: relative.to_string(),
            bytes,
            sha256,
        })
    }
}

/// A file's size and SHA-256, in hex.
pub async fn sha256_file(path: &std::path::Path) -> anyhow::Result<(u64, String)> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buf[..n]);
    }
    Ok((total, format!("{:x}", hasher.finalize())))
}

fn rfc3339(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339()
}

/// A bundle file name for an item: its id made safe, with its file's
/// extension.
fn media_name(id: &str, file: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ext = std::path::Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| format!(".{}", e.to_ascii_lowercase()))
        .unwrap_or_default();
    format!("{safe}{ext}")
}

/// Pack `staging` into a gzipped tar at `bundle`: the file's size and
/// SHA-256.
pub async fn pack_bundle(
    staging: &std::path::Path,
    bundle: &std::path::Path,
) -> anyhow::Result<(u64, String)> {
    let staging = staging.to_path_buf();
    let target = bundle.to_path_buf();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let partial = target.with_extension("partial");
        let file = std::fs::File::create(&partial)?;
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        tar.follow_symlinks(false);
        tar.append_path_with_name(staging.join("manifest.json"), "manifest.json")?;
        for dir in ["data", "media"] {
            if staging.join(dir).is_dir() {
                tar.append_dir_all(dir, staging.join(dir))?;
            }
        }
        tar.into_inner()?.finish()?.sync_all()?;
        std::fs::rename(&partial, &target)?;
        Ok(())
    })
    .await??;
    sha256_file(bundle).await
}

// ── The worker (docs/COMPLIANCE_FOLLOWUPS.md §6.2) ─────────────────────

/// Where bundles go and how long they are kept.
#[derive(Debug, Clone)]
pub struct ExportSettings {
    pub exports_dir: std::path::PathBuf,
    pub bundle_ttl: Duration,
}

impl ExportSettings {
    /// A tenant's bundle for request `id`.
    pub fn bundle_path(&self, tenant: &TenantId, id: &SubjectRequestId) -> std::path::PathBuf {
        self.exports_dir
            .join(tenant.to_string())
            .join(format!("{id}.tar.gz"))
    }
}

/// What the worker reaches beyond the database: the files an export copies
/// and an erasure removes, and the CDR pipeline's dead letters.
#[derive(Clone, Copy)]
pub struct WorkerAccess<'a> {
    pub media: &'a dyn MediaFetcher,
    pub remover: &'a dyn crate::subject_erasure::MediaRemover,
    pub dead_letters: Option<&'a dyn crate::subject_erasure::DeadLetterStore>,
}

/// Run a claimed request to its end and store the outcome: an export
/// `completed` with its report and bundle; an erasure's first run
/// `preview`, its confirmed run `completed`; a tenant's deletion
/// `completed`; or `failed` with why.
pub async fn run_request(
    storage: &crate::storage::DynStorage,
    access: WorkerAccess<'_>,
    settings: &ExportSettings,
    mut request: SubjectRequest,
) -> SubjectRequest {
    let outcome = match request.kind {
        SubjectRequestKind::Export => run_export_request(storage, access, settings, &mut request)
            .await
            .map(|(report, bundle)| {
                request.report = Some(report);
                request.bundle = Some(bundle);
                SubjectRequestStatus::Completed
            }),
        SubjectRequestKind::Erasure | SubjectRequestKind::TenantDeletion => {
            run_erasure_request(storage, access, &mut request).await
        }
    };

    request.completed_at = Some(SystemTime::now());
    match outcome {
        Ok(status) => {
            tracing::info!(
                request = %request.id,
                tenant_id = %request.tenant_id,
                kind = request.kind.as_str(),
                status = status.as_str(),
                "Subject request finished"
            );
            request.status = status;
            request.error = None;
        }
        Err(e) => {
            tracing::warn!(request = %request.id, tenant_id = %request.tenant_id, error = %e, "Subject request failed");
            request.status = SubjectRequestStatus::Failed;
            request.error = Some(e.to_string());
        }
    }
    if let Some(store) = storage.compliance_store() {
        if let Err(e) = store.update_subject_request(&request).await {
            tracing::warn!(request = %request.id, error = %e, "Could not store a subject request's outcome");
        }
    }
    request
}

async fn run_export_request(
    storage: &crate::storage::DynStorage,
    access: WorkerAccess<'_>,
    settings: &ExportSettings,
    request: &mut SubjectRequest,
) -> anyhow::Result<(ExportReport, BundleRef)> {
    let staging = settings
        .exports_dir
        .join(format!(".staging-{}", request.id));
    let store = storage
        .compliance_store()
        .ok_or_else(|| anyhow::anyhow!("no compliance store"))?;
    let data = storage
        .subject_data_store()
        .ok_or_else(|| anyhow::anyhow!("no subject data store"))?;
    let expansion =
        crate::legal_hold::expand(storage, &request.tenant_id, &request.subjects).await?;
    request.expansion = Some(expansion.clone());
    store.update_subject_request(request).await?;
    let holds = store.list_holds(&request.tenant_id, false).await?;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let exported = run_export(
        data.as_ref(),
        access.media,
        &holds,
        request,
        &expansion,
        &staging,
    )
    .await;
    let packed = match exported {
        Ok(outcome) => {
            let path = settings.bundle_path(&request.tenant_id, &request.id);
            pack_bundle(&staging, &path)
                .await
                .map(|(size_bytes, sha256)| {
                    (
                        outcome.report,
                        BundleRef {
                            path: path.to_string_lossy().into_owned(),
                            size_bytes,
                            sha256,
                            expires_at: SystemTime::now() + settings.bundle_ttl,
                            removed: false,
                        },
                    )
                })
        }
        Err(e) => Err(e),
    };
    let _ = tokio::fs::remove_dir_all(&staging).await;
    packed
}

/// An erasure's preview or its confirmed run, or a tenant's deletion, every
/// deletion attributed to the request.
async fn run_erasure_request(
    storage: &crate::storage::DynStorage,
    access: WorkerAccess<'_>,
    request: &mut SubjectRequest,
) -> anyhow::Result<SubjectRequestStatus> {
    use crate::compliance::{deleting_as, Deleter, DeletionCause};
    use crate::subject_erasure::{run_erasure, run_tenant_deletion, ErasureContext};

    let store = storage
        .compliance_store()
        .ok_or_else(|| anyhow::anyhow!("no compliance store"))?;
    let data = storage
        .subject_data_store()
        .ok_or_else(|| anyhow::anyhow!("no subject data store"))?;
    let erase = storage
        .subject_erase_store()
        .ok_or_else(|| anyhow::anyhow!("no subject erase store"))?;
    let policy = store.get_compliance_policy(&request.tenant_id).await?;
    let holds = store.list_holds(&request.tenant_id, false).await?;
    let ctx = ErasureContext {
        storage,
        data: data.as_ref(),
        erase: erase.as_ref(),
        media: access.remover,
        dead_letters: access.dead_letters,
        holds: &holds,
        cdr_erasure: policy.map(|p| p.cdr_erasure).unwrap_or_default(),
    };
    if request.kind == SubjectRequestKind::TenantDeletion {
        let actor = request.requested_by.clone();
        let deleter =
            Deleter::new(DeletionCause::TenantDeletion, actor).for_request(request.id.to_string());
        let report = deleting_as(deleter, run_tenant_deletion(&ctx, request)).await?;
        request.erasure = Some(report);
        return Ok(SubjectRequestStatus::Completed);
    }

    let expansion =
        crate::legal_hold::expand(storage, &request.tenant_id, &request.subjects).await?;
    request.expansion = Some(expansion);
    store.update_subject_request(request).await?;
    match request.confirmed_at {
        None => {
            request.preview = Some(run_erasure(&ctx, request, true).await?);
            Ok(SubjectRequestStatus::Preview)
        }
        Some(_) => {
            let actor = request
                .confirmed_by
                .clone()
                .unwrap_or_else(|| request.requested_by.clone());
            let deleter =
                Deleter::new(DeletionCause::Erasure, actor).for_request(request.id.to_string());
            let report = deleting_as(deleter, run_erasure(&ctx, request, false)).await?;
            request.erasure = Some(report);
            Ok(SubjectRequestStatus::Completed)
        }
    }
}

/// Run every waiting request, oldest first; the finished ones. The caller
/// holds the worker lock, so requests still marked running were left by a
/// worker that stopped, and go back in the queue first.
pub async fn run_subject_requests(
    storage: &crate::storage::DynStorage,
    access: WorkerAccess<'_>,
    settings: &ExportSettings,
) -> anyhow::Result<Vec<SubjectRequest>> {
    let store = storage
        .compliance_store()
        .ok_or_else(|| anyhow::anyhow!("no compliance store"))?;
    let requeued = store.requeue_running_subject_requests().await?;
    if requeued > 0 {
        tracing::warn!(requeued, "Subject requests left running were queued again");
    }
    let mut finished = Vec::new();
    while let Some(request) = store.claim_subject_request().await? {
        finished.push(run_request(storage, access, settings, request).await);
    }
    Ok(finished)
}

/// Remove the bundles past their time. Returns how many.
pub async fn remove_expired_bundles(
    storage: &crate::storage::DynStorage,
    now: SystemTime,
) -> anyhow::Result<usize> {
    let store = storage
        .compliance_store()
        .ok_or_else(|| anyhow::anyhow!("no compliance store"))?;
    let mut removed = 0;
    for mut request in store.expired_subject_bundles(now).await? {
        let Some(bundle) = request.bundle.as_mut() else {
            continue;
        };
        match tokio::fs::remove_file(&bundle.path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(request = %request.id, error = %e, "Expired bundle not removed");
                continue;
            }
        }
        bundle.removed = true;
        store.update_subject_request(&request).await?;
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_statuses_and_classes_read_as_their_names() {
        for k in [
            SubjectRequestKind::Export,
            SubjectRequestKind::Erasure,
            SubjectRequestKind::TenantDeletion,
        ] {
            assert_eq!(SubjectRequestKind::parse(k.as_str()), Some(k));
        }
        for s in [
            SubjectRequestStatus::Received,
            SubjectRequestStatus::Running,
            SubjectRequestStatus::Preview,
            SubjectRequestStatus::Confirmed,
            SubjectRequestStatus::Completed,
            SubjectRequestStatus::Failed,
        ] {
            assert_eq!(SubjectRequestStatus::parse(s.as_str()), Some(s));
        }
        let names: std::collections::BTreeSet<_> =
            ExportClass::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(names.len(), ExportClass::ALL.len(), "names are distinct");
        let cdrs = ExportClass::ALL
            .iter()
            .position(|c| *c == ExportClass::Cdrs)
            .unwrap();
        for class in ExportClass::ALL.iter().filter(|c| c.by_call()) {
            let at = ExportClass::ALL.iter().position(|c| c == class).unwrap();
            assert!(
                at > cdrs,
                "{class:?} is walked after the records that name its calls"
            );
        }
    }

    #[test]
    fn a_request_is_overdue_only_while_open_past_its_day() {
        let mut r = SubjectRequest::received(
            TenantId::nil(),
            SubjectRequestKind::Export,
            Vec::new(),
            "dpo",
            30,
        );
        let later = r.received_at + Duration::from_secs(31 * 86_400);
        assert!(!r.is_overdue(r.received_at));
        assert!(r.is_overdue(later));
        r.status = SubjectRequestStatus::Completed;
        assert!(!r.is_overdue(later));
    }

    #[test]
    fn media_names_keep_the_extension_and_lose_everything_else() {
        assert_eq!(media_name("rec-1", "/x/y/a.WAV"), "rec-1.wav");
        assert_eq!(media_name("../../etc", "/x/passwd"), "______etc");
        assert_eq!(media_name("42", "/x/a.tar.gz"), "42.gz");
        assert_eq!(media_name("7", "/x/a.w@v"), "7");
    }

    fn row(file: &str) -> SubjectRow {
        SubjectRow {
            id: "1".into(),
            link: RowLink::Owned,
            data: serde_json::Value::Null,
            at: None,
            call_id: None,
            file: Some(file.into()),
            others_heard: false,
        }
    }

    #[tokio::test]
    async fn local_files_come_only_from_mounted_directories() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside_file = root.path().join("a.wav");
        let outside_file = outside.path().join("b.wav");
        std::fs::write(&inside_file, b"abc").unwrap();
        std::fs::write(&outside_file, b"abc").unwrap();
        let media = LocalMedia {
            roots: vec![root.path().to_path_buf()],
            profile_pictures_dir: Some(root.path().to_path_buf()),
        };
        let dest = outside.path().join("copy");
        let t = TenantId::nil();
        let class = ExportClass::CallRecordings;
        assert_eq!(
            media
                .fetch(&t, class, &row(&inside_file.to_string_lossy()), &dest)
                .await,
            Ok(3)
        );
        assert!(media
            .fetch(&t, class, &row(&outside_file.to_string_lossy()), &dest)
            .await
            .is_err());
        assert_eq!(
            media
                .fetch(&t, ExportClass::Profile, &row("a.wav"), &dest)
                .await,
            Ok(3)
        );
        assert!(media
            .fetch(&t, ExportClass::Profile, &row("../b.wav"), &dest)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_bundle_packs_the_manifest_data_and_media() {
        use std::io::Read;
        let staging = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(staging.path().join("data")).unwrap();
        std::fs::create_dir_all(staging.path().join("media/cdrs")).unwrap();
        std::fs::write(staging.path().join("manifest.json"), b"{}").unwrap();
        std::fs::write(staging.path().join("data/cdrs.jsonl"), b"{}\n").unwrap();
        std::fs::write(staging.path().join("media/cdrs/1.wav"), b"RIFF").unwrap();
        let out = tempfile::tempdir().unwrap();
        let bundle = out.path().join("t/b.tar.gz");
        let (size, sha) = pack_bundle(staging.path(), &bundle).await.unwrap();
        assert_eq!(size, std::fs::metadata(&bundle).unwrap().len());
        assert_eq!(sha.len(), 64);
        assert!(!bundle.with_extension("partial").exists());
        let mut names = Vec::new();
        let gz = flate2::read::GzDecoder::new(std::fs::File::open(&bundle).unwrap());
        for entry in tar::Archive::new(gz).entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.header().entry_type().is_file() {
                let mut s = String::new();
                entry.read_to_string(&mut s).unwrap();
                names.push(entry.path().unwrap().to_string_lossy().into_owned());
            }
        }
        names.sort();
        assert_eq!(
            names,
            ["data/cdrs.jsonl", "manifest.json", "media/cdrs/1.wav"]
        );
    }
}
