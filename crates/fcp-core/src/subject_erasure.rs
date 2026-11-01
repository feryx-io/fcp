//! Subject erasure and tenant deletion (`docs/COMPLIANCE_FOLLOWUPS.md` §6.3–6.6,
//! §4).
//!
//! An erasure runs twice. First as a preview: the subject resolved and every
//! store walked as an export walks them, each item that is the subject's
//! given what the tenant owes it — deleted, pseudonymised, its owner
//! cleared, or kept with why — and nothing changed. A person confirms the
//! preview; the erasure then runs against the data as it is at confirmation
//! and reports what it did. Items under a legal hold are never touched. A
//! record that must be kept has the subject's identifiers replaced by their
//! pseudonym: `erased-` and twenty hex digits of an HMAC-SHA-256, keyed by
//! the tenant's own secret, over the subject's canonical identifier — the
//! same token in every record and every later request.
//!
//! A tenant's deletion is the same machinery with the tenant as the subject:
//! its conference recordings' files, its dead letters, then every row of
//! every table, the tenant last.

use crate::compliance::CdrErasure;
use crate::legal_hold::{LegalHold, LegalHoldViolation, SubjectIdentifier};
use crate::subject::{pseudonymise_identifier, SubjectMatcher};
use crate::subject_request::{
    hold_over, is_theirs, ExportClass, SubjectDataStore, SubjectRequest, SubjectRow, SubjectScope,
};
use crate::TenantId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// How many item ids a class's report lists.
const SAMPLE: usize = 200;
const PAGE: usize = 500;
const CALLS_PER_QUERY: usize = 1000;

/// What an erasure does to a class's items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Treatment {
    Delete,
    /// Replace the subject's identifiers with their pseudonym and keep the
    /// item.
    Pseudonymise,
    /// Keep the item, no longer the subject's.
    ClearOwner,
    /// Keep the item as it is, for this reason.
    Keep(&'static str),
}

impl Treatment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Treatment::Delete => "delete",
            Treatment::Pseudonymise => "pseudonymise",
            Treatment::ClearOwner => "clear_owner",
            Treatment::Keep(_) => "keep",
        }
    }
}

/// What the tenant owes each class (the erasure column of §6.3).
pub fn treatment(class: ExportClass, cdrs: CdrErasure) -> Treatment {
    use ExportClass::*;
    match class {
        Cdrs => match cdrs {
            CdrErasure::Pseudonymise => Treatment::Pseudonymise,
            CdrErasure::Delete => Treatment::Delete,
        },
        EmergencyCalls | AuditEvents | ConfigChanges => Treatment::Pseudonymise,
        Devices | LineBindings | Meetings => Treatment::ClearOwner,
        PortRequests => Treatment::Keep("a carrier's record of a number port"),
        Profile
        | ApiTokens
        | Extensions
        | QueueMemberships
        | Mailboxes
        | VoicemailGreetings
        | VoicemailMessages
        | CallRecordings
        | ConferenceRecordings
        | SipTraces
        | FlowTraces
        | Registrations
        | Presence
        | PresenceSubscriptions
        | PresencePolicies
        | ResourceLists
        | PresenceHistory
        | DeclaredLocations
        | WebhookDeliveries
        | NumberLists
        | PersonalContacts => Treatment::Delete,
    }
}

/// The order an erasure acts in: records first (they name the calls), what
/// hangs off the calls next, a mailbox's contents before the mailbox, the
/// user last.
pub const ERASURE_ORDER: [ExportClass; 29] = [
    ExportClass::Cdrs,
    ExportClass::CallRecordings,
    ExportClass::SipTraces,
    ExportClass::FlowTraces,
    ExportClass::ConferenceRecordings,
    ExportClass::VoicemailMessages,
    ExportClass::VoicemailGreetings,
    ExportClass::Mailboxes,
    ExportClass::ApiTokens,
    ExportClass::Devices,
    ExportClass::LineBindings,
    ExportClass::Extensions,
    ExportClass::QueueMemberships,
    ExportClass::Meetings,
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
    ExportClass::Profile,
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ErasureClassReport {
    /// `delete`, `pseudonymise`, `clear_owner` or `keep`.
    pub treatment: String,
    /// Items it was (or, in a preview, would be) done to.
    pub items: u64,
    /// Items left alone because a legal hold covers them.
    #[serde(default)]
    pub held: u64,
    /// Items it could not act on, with why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// The first items' ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample: Vec<String>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeadLetterReport {
    /// Whether this host reached the dead letters.
    pub reached: bool,
    pub items: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What an erasure found, did, kept and could not reach.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ErasureReport {
    /// A preview changes nothing.
    pub preview: bool,
    pub classes: BTreeMap<String, ErasureClassReport>,
    /// Classes kept, with why.
    #[serde(default)]
    pub kept: BTreeMap<String, String>,
    /// The holds that kept items, by id, with their matters.
    #[serde(default)]
    pub held_by: BTreeMap<String, String>,
    /// What the erasure cannot reach, and when it goes.
    #[serde(default)]
    pub not_reached: BTreeMap<String, String>,
    #[serde(default)]
    pub dead_letters: Option<DeadLetterReport>,
    /// A tenant's deletion: rows removed, by table.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tables: BTreeMap<String, u64>,
}

/// A row's new values, by column.
#[derive(Debug, Clone, PartialEq)]
pub struct RowUpdate {
    pub id: String,
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// Where an erasure acts. Deletions run in the current
/// [`crate::compliance::deleting_as`] scope and come back as a
/// [`LegalHoldViolation`] when a hold refuses them.
#[async_trait::async_trait]
pub trait SubjectEraseStore: Send + Sync {
    /// The tenant's pseudonym key, created on first use.
    async fn pseudonym_secret(&self, tenant: &TenantId) -> anyhow::Result<Vec<u8>>;

    /// Delete `ids` of `class` (for number lists: remove the entries).
    async fn delete_subject_rows(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        ids: &[String],
    ) -> anyhow::Result<u64>;

    /// Write new values into rows of `class`; records rewritten this way are
    /// noted for the archive to write their days again.
    async fn update_subject_rows(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        updates: &[RowUpdate],
    ) -> anyhow::Result<u64>;

    /// Every archived day of the tenant noted for rewriting.
    async fn note_tenant_archive_rewrite(&self, tenant: &TenantId) -> anyhow::Result<u64>;

    /// Delete every row the tenant has, table by table, and the tenant last:
    /// rows removed by table.
    async fn delete_tenant_data(&self, tenant: &TenantId) -> anyhow::Result<Vec<(String, u64)>>;
}

/// Removes an item's file where no row deletion will: a conference
/// recording on its node, a profile picture.
#[async_trait::async_trait]
pub trait MediaRemover: Send + Sync {
    async fn remove(
        &self,
        tenant: &TenantId,
        class: ExportClass,
        row: &SubjectRow,
    ) -> Result<(), String>;
}

#[async_trait::async_trait]
impl MediaRemover for crate::subject_request::LocalMedia {
    async fn remove(
        &self,
        _tenant: &TenantId,
        class: ExportClass,
        row: &SubjectRow,
    ) -> Result<(), String> {
        let Some(file) = row.file.as_deref() else {
            return Ok(());
        };
        let path = match class {
            ExportClass::Profile => {
                let dir = self
                    .profile_pictures_dir
                    .as_ref()
                    .ok_or("no profile pictures directory on this host")?;
                if std::path::Path::new(file).components().count() != 1 {
                    return Err("not a file name".to_string());
                }
                dir.join(file)
            }
            _ => {
                let path = std::path::PathBuf::from(file.strip_prefix("file://").unwrap_or(file));
                if !crate::compliance::under_file_root(&path, &self.roots) {
                    return Err("not under a directory this host mounts".to_string());
                }
                path
            }
        };
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// The CDR pipeline's dead letters: records the processor refused, each the
/// record's JSON.
#[async_trait::async_trait]
pub trait DeadLetterStore: Send + Sync {
    /// A page of `(entry id, record JSON)` after `after`, and where the next
    /// starts.
    async fn dead_letter_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<(Vec<(String, String)>, Option<String>)>;

    async fn delete_dead_letters(&self, ids: &[String]) -> anyhow::Result<u64>;
}

/// What an erasure runs against.
pub struct ErasureContext<'a> {
    pub storage: &'a crate::storage::DynStorage,
    pub data: &'a dyn SubjectDataStore,
    pub erase: &'a dyn SubjectEraseStore,
    pub media: &'a dyn MediaRemover,
    pub dead_letters: Option<&'a dyn DeadLetterStore>,
    pub holds: &'a [LegalHold],
    pub cdr_erasure: CdrErasure,
}

/// The canonical identifier a subject's pseudonym is made from.
pub fn subject_anchor(subject: &SubjectIdentifier) -> String {
    let spelling = match subject {
        SubjectIdentifier::User { username } => format!("user:{}", username.trim().to_lowercase()),
        SubjectIdentifier::Number { number } => number.clone(),
        SubjectIdentifier::Aor { aor } => aor.clone(),
        SubjectIdentifier::Email { address } => address.clone(),
    };
    if spelling.starts_with("user:") {
        return spelling;
    }
    let keys = crate::subject::keys(&spelling);
    keys.iter()
        .find(|k| k.starts_with("num:") || k.starts_with("aor:"))
        .or_else(|| keys.first())
        .cloned()
        .unwrap_or(spelling)
}

/// A subject's pseudonym under `secret`.
pub fn pseudonym(secret: &[u8], anchor: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret).expect("HMAC takes any key");
    mac.update(anchor.as_bytes());
    let digest = mac.finalize().into_bytes();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("erased-{}", &hex[..20])
}

/// One subject of a request: what identifies them and their pseudonym.
struct Subject {
    matcher: SubjectMatcher,
    token: String,
}

impl Subject {
    fn token_for<'s>(subjects: &'s [Subject], identifier: &str) -> Option<&'s str> {
        subjects
            .iter()
            .find(|s| s.matcher.matches(identifier))
            .map(|s| s.token.as_str())
    }
}

/// What the stores cannot reach, and when it goes on its own.
fn not_reached(dead_letters_reached: bool) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = [
        (
            "backups",
            "database backups and archive bucket versions keep erased data until the operator's retention expires them",
        ),
        (
            "cdr_archive",
            "the days of erased records are written again on the archiver's next run; a bucket with versioning keeps the old version until its non-current expiry",
        ),
        (
            "spool_dead_letters",
            "records a node's processor refused stay in its spool until [cdr] dead_letter_days (30 by default)",
        ),
        (
            "streams",
            "records and events in transit leave the Redis streams when they are trimmed ([cdr] retain_hours in fcp-jobs, 24 by default)",
        ),
        (
            "caches",
            "API gateway responses expire with its cache TTL (60 s by default); credential caches on nodes with theirs",
        ),
        ("logs", "service logs are the operator's, kept under its own retention"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    if !dead_letters_reached {
        out.insert(
            "stream_dead_letters".to_string(),
            "this host has no Redis configured ([cluster] redis_url): the CDR stream's dead letters were not searched".to_string(),
        );
    }
    out
}

/// Every string leaf of a JSON value.
fn strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => {
            if s.len() <= 512 {
                out.push(s.clone())
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

/// The new values that pseudonymise a row, `None` when nothing in it names
/// a subject.
fn pseudonymised(class: ExportClass, row: &SubjectRow, subjects: &[Subject]) -> Option<RowUpdate> {
    let data = row.data.as_object()?;
    let mut fields = serde_json::Map::new();
    let text = |key: &str| data.get(key).and_then(|v| v.as_str());
    let replace = |fields: &mut serde_json::Map<String, serde_json::Value>, key: &str| {
        if let Some(value) = text(key) {
            if let Some(token) = Subject::token_for(subjects, value) {
                fields.insert(
                    key.to_string(),
                    pseudonymise_identifier(value, token).into(),
                );
                return true;
            }
        }
        false
    };
    match class {
        ExportClass::Cdrs => {
            if replace(&mut fields, "from_uri") {
                fields.insert("from_display_name".into(), serde_json::Value::Null);
            }
            if replace(&mut fields, "to_uri") {
                fields.insert("to_display_name".into(), serde_json::Value::Null);
            }
            if let Some(serde_json::Value::Object(custom)) = data.get("custom_data") {
                let kept: serde_json::Map<_, _> = custom
                    .iter()
                    .filter(|(_, v)| {
                        let mut found = Vec::new();
                        strings(v, &mut found);
                        !found
                            .iter()
                            .any(|s| Subject::token_for(subjects, s).is_some())
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if kept.len() != custom.len() {
                    fields.insert("custom_data".into(), serde_json::Value::Object(kept));
                }
            }
            if !fields.is_empty() {
                fields.insert("sip_call_id".into(), "erased".into());
            }
        }
        ExportClass::EmergencyCalls => {
            for key in ["caller_uri", "caller_username", "callback_number"] {
                replace(&mut fields, key);
            }
        }
        ExportClass::AuditEvents => {
            replace(&mut fields, "actor");
            if let Some(path) = text("path") {
                let mut changed = false;
                let rewritten: Vec<String> = path
                    .split('/')
                    .map(|segment| match Subject::token_for(subjects, segment) {
                        Some(token) if !segment.is_empty() => {
                            changed = true;
                            token.to_string()
                        }
                        _ => segment.to_string(),
                    })
                    .collect();
                if changed {
                    fields.insert("path".into(), rewritten.join("/").into());
                }
            }
        }
        ExportClass::ConfigChanges => {
            replace(&mut fields, "changed_by");
        }
        _ => return None,
    }
    (!fields.is_empty()).then(|| RowUpdate {
        id: row.id.clone(),
        fields,
    })
}

/// The owner columns cleared for a class.
fn owner_cleared(class: ExportClass, id: &str) -> RowUpdate {
    let column = match class {
        ExportClass::Meetings => "owner_id",
        _ => "owner_user_id",
    };
    let mut fields = serde_json::Map::new();
    fields.insert(column.into(), serde_json::Value::Null);
    RowUpdate {
        id: id.to_string(),
        fields,
    }
}

fn note(report: &mut ErasureClassReport, id: &str) {
    report.items += 1;
    if report.sample.len() < SAMPLE {
        report.sample.push(id.to_string());
    } else {
        report.truncated = true;
    }
}

/// Delete `ids`, one at a time once a hold refuses the batch: what went, and
/// what a hold kept.
async fn delete_around_holds(
    erase: &dyn SubjectEraseStore,
    tenant: &TenantId,
    class: ExportClass,
    ids: &[String],
    report: &mut ErasureClassReport,
    held_by: &mut BTreeMap<String, String>,
    holds: &[LegalHold],
) {
    if ids.is_empty() {
        return;
    }
    match erase.delete_subject_rows(tenant, class, ids).await {
        Ok(_) => ids.iter().for_each(|id| note(report, id)),
        Err(e) if e.downcast_ref::<LegalHoldViolation>().is_some() => {
            for id in ids {
                match erase
                    .delete_subject_rows(tenant, class, std::slice::from_ref(id))
                    .await
                {
                    Ok(_) => note(report, id),
                    Err(e) => match e.downcast_ref::<LegalHoldViolation>() {
                        Some(v) => {
                            report.held += 1;
                            let matter = holds
                                .iter()
                                .find(|h| h.id.to_string() == v.hold_id)
                                .map(|h| h.matter.clone())
                                .unwrap_or_default();
                            held_by.insert(v.hold_id.clone(), matter);
                        }
                        None => report.errors.push(format!("{id}: {e}")),
                    },
                }
            }
        }
        Err(e) => report.errors.push(format!("{} items: {e}", ids.len())),
    }
}

/// Run an erasure of `request`'s subjects: a preview when `preview`,
/// otherwise the erasure itself, in the deleter scope the caller opened.
pub async fn run_erasure(
    ctx: &ErasureContext<'_>,
    request: &SubjectRequest,
    preview: bool,
) -> anyhow::Result<ErasureReport> {
    let tenant = request.tenant_id;
    let expansion = crate::legal_hold::expand(ctx.storage, &tenant, &request.subjects).await?;
    let matcher = expansion.matcher();
    let secret = if preview {
        Vec::new()
    } else {
        ctx.erase.pseudonym_secret(&tenant).await?
    };
    let mut subjects = Vec::new();
    for subject in &request.subjects {
        let own =
            crate::legal_hold::expand(ctx.storage, &tenant, std::slice::from_ref(subject)).await?;
        subjects.push(Subject {
            matcher: own.matcher(),
            token: pseudonym(&secret, &subject_anchor(subject)),
        });
    }

    let mut scope = SubjectScope::from_expansion(&expansion);
    let mut calls = BTreeSet::new();
    let mut report = ErasureReport {
        preview,
        ..Default::default()
    };

    for class in ERASURE_ORDER {
        let how = treatment(class, ctx.cdr_erasure);
        let mut class_report = ErasureClassReport {
            treatment: how.as_str().to_string(),
            ..Default::default()
        };
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
        for chunk in chunks {
            let mut chunk_scope = scope.clone();
            if class.by_call() {
                chunk_scope.call_ids = chunk;
            }
            let mut after: Option<String> = None;
            loop {
                let page = ctx
                    .data
                    .subject_rows(&tenant, class, &chunk_scope, after.as_deref(), PAGE)
                    .await?;
                let mut to_delete: Vec<String> = Vec::new();
                let mut to_update: Vec<RowUpdate> = Vec::new();
                for row in &page.rows {
                    if !is_theirs(row, &matcher, &calls) {
                        continue;
                    }
                    if class == ExportClass::Cdrs {
                        if let Some(call) = &row.call_id {
                            calls.insert(call.clone());
                        }
                    }
                    if let Some(hold) = hold_over(ctx.holds, class, row) {
                        class_report.held += 1;
                        report
                            .held_by
                            .insert(hold.id.to_string(), hold.matter.clone());
                        continue;
                    }
                    match how {
                        Treatment::Keep(_) => note(&mut class_report, &row.id),
                        _ if preview => note(&mut class_report, &row.id),
                        Treatment::Delete => {
                            if matches!(
                                class,
                                ExportClass::ConferenceRecordings | ExportClass::Profile
                            ) && row.file.is_some()
                            {
                                if let Err(reason) = ctx.media.remove(&tenant, class, row).await {
                                    class_report
                                        .errors
                                        .push(format!("{}: file not removed: {reason}", row.id));
                                    if class == ExportClass::ConferenceRecordings {
                                        // The row is what finds the file again.
                                        continue;
                                    }
                                }
                            }
                            to_delete.push(row.id.clone());
                        }
                        Treatment::ClearOwner => to_update.push(owner_cleared(class, &row.id)),
                        Treatment::Pseudonymise => {
                            match pseudonymised(class, row, &subjects) {
                                Some(update) => to_update.push(update),
                                // Matched through a spelling no field holds as
                                // is: nothing to rewrite.
                                None => note(&mut class_report, &row.id),
                            }
                        }
                    }
                }
                delete_around_holds(
                    ctx.erase,
                    &tenant,
                    class,
                    &to_delete,
                    &mut class_report,
                    &mut report.held_by,
                    ctx.holds,
                )
                .await;
                if !to_update.is_empty() {
                    match ctx
                        .erase
                        .update_subject_rows(&tenant, class, &to_update)
                        .await
                    {
                        Ok(_) => to_update
                            .iter()
                            .for_each(|u| note(&mut class_report, &u.id)),
                        Err(e) => class_report
                            .errors
                            .push(format!("{} items: {e}", to_update.len())),
                    }
                }
                match page.next {
                    Some(next) if !page.rows.is_empty() => after = Some(next),
                    _ => break,
                }
            }
        }
        if let Treatment::Keep(reason) = how {
            if class_report.items > 0 {
                report
                    .kept
                    .insert(class.as_str().to_string(), reason.to_string());
            }
        }
        if class_report.items > 0 || class_report.held > 0 || !class_report.errors.is_empty() {
            report
                .classes
                .insert(class.as_str().to_string(), class_report);
        }
    }

    report.dead_letters = Some(match ctx.dead_letters {
        Some(dead) => dead_letters(dead, &tenant, Some(&matcher), preview).await,
        None => DeadLetterReport {
            reached: false,
            items: 0,
            note: Some("no Redis configured on this host".to_string()),
        },
    });
    report.not_reached = not_reached(ctx.dead_letters.is_some());
    Ok(report)
}

/// Remove the tenant's dead letters naming the subject (`matcher`), or all
/// of the tenant's when there is none.
async fn dead_letters(
    dead: &dyn DeadLetterStore,
    tenant: &TenantId,
    matcher: Option<&SubjectMatcher>,
    preview: bool,
) -> DeadLetterReport {
    let tenant_value = serde_json::to_value(tenant).unwrap_or_default();
    let mut report = DeadLetterReport {
        reached: true,
        ..Default::default()
    };
    let mut after: Option<String> = None;
    loop {
        let (entries, next) = match dead.dead_letter_page(after.as_deref(), PAGE).await {
            Ok(page) => page,
            Err(e) => {
                report.note = Some(e.to_string());
                return report;
            }
        };
        let mut ids = Vec::new();
        for (id, raw) in &entries {
            let Ok(record) = serde_json::from_str::<serde_json::Value>(raw) else {
                continue;
            };
            if record.get("tenant_id") != Some(&tenant_value) {
                continue;
            }
            let theirs = match matcher {
                None => true,
                Some(m) => {
                    let mut found = Vec::new();
                    strings(&record, &mut found);
                    found.iter().any(|s| m.matches(s))
                }
            };
            if theirs {
                ids.push(id.clone());
            }
        }
        report.items += ids.len() as u64;
        if !preview && !ids.is_empty() {
            if let Err(e) = dead.delete_dead_letters(&ids).await {
                report.note = Some(e.to_string());
            }
        }
        match next {
            Some(n) if !entries.is_empty() => after = Some(n),
            _ => break,
        }
    }
    report
}

/// Delete everything a tenant has: its conference recordings' files through
/// their nodes, its dead letters, its archived days noted for rewriting,
/// every row, then the tenant.
pub async fn run_tenant_deletion(
    ctx: &ErasureContext<'_>,
    request: &SubjectRequest,
) -> anyhow::Result<ErasureReport> {
    let tenant = request.tenant_id;
    let mut report = ErasureReport::default();

    if let Some(recordings) = ctx.storage.recording_store() {
        let mut files = ErasureClassReport {
            treatment: "delete".to_string(),
            ..Default::default()
        };
        let mut offset = 0;
        loop {
            let page = recordings
                .list_recordings(
                    &tenant,
                    &crate::recording::RecordingFilter {
                        limit: PAGE,
                        offset,
                        ..Default::default()
                    },
                )
                .await?;
            for recording in &page {
                let row = SubjectRow {
                    id: recording.id.to_string(),
                    link: crate::subject_request::RowLink::Owned,
                    data: serde_json::json!({ "node_id": recording.node_id }),
                    at: Some(recording.started_at),
                    call_id: None,
                    file: Some(recording.path.clone()),
                    others_heard: true,
                };
                match ctx
                    .media
                    .remove(&tenant, ExportClass::ConferenceRecordings, &row)
                    .await
                {
                    Ok(()) => note(&mut files, &row.id),
                    Err(reason) => files.errors.push(format!("{}: {reason}", row.id)),
                }
            }
            if page.len() < PAGE {
                break;
            }
            offset += PAGE;
        }
        if files.items > 0 || !files.errors.is_empty() {
            report
                .classes
                .insert("conference_recording_files".to_string(), files);
        }
    }

    report.dead_letters = Some(match ctx.dead_letters {
        Some(dead) => dead_letters(dead, &tenant, None, false).await,
        None => DeadLetterReport {
            reached: false,
            items: 0,
            note: Some("no Redis configured on this host".to_string()),
        },
    });
    ctx.erase.note_tenant_archive_rewrite(&tenant).await?;
    for (table, rows) in ctx.erase.delete_tenant_data(&tenant).await? {
        report.tables.insert(table, rows);
    }
    report.not_reached = not_reached(ctx.dead_letters.is_some());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subject_request::RowLink;

    #[test]
    fn a_pseudonym_is_stable_keyed_and_short() {
        let a = pseudonym(b"tenant-secret", "user:alice");
        assert_eq!(a, pseudonym(b"tenant-secret", "user:alice"));
        assert_ne!(a, pseudonym(b"another-secret", "user:alice"));
        assert_ne!(a, pseudonym(b"tenant-secret", "user:bob"));
        assert!(
            a.starts_with("erased-") && a.len() == "erased-".len() + 20,
            "{a}"
        );
        assert_eq!(
            subject_anchor(&SubjectIdentifier::Number {
                number: "+1 (415) 555-0100".into()
            }),
            "num:14155550100"
        );
        assert_eq!(
            subject_anchor(&SubjectIdentifier::User {
                username: " Alice ".into()
            }),
            "user:alice"
        );
        assert_eq!(
            subject_anchor(&SubjectIdentifier::Aor {
                aor: "sip:Bob@Carrier.example".into()
            }),
            "aor:bob@carrier.example"
        );
    }

    #[test]
    fn every_class_has_a_treatment_and_a_place_in_the_order() {
        let ordered: BTreeSet<_> = ERASURE_ORDER.iter().map(|c| c.as_str()).collect();
        assert_eq!(ordered.len(), ExportClass::ALL.len());
        let at = |c| ERASURE_ORDER.iter().position(|x| *x == c).unwrap();
        assert!(at(ExportClass::Cdrs) < at(ExportClass::CallRecordings));
        assert!(at(ExportClass::VoicemailMessages) < at(ExportClass::Mailboxes));
        assert_eq!(ERASURE_ORDER.last(), Some(&ExportClass::Profile));
        assert_eq!(
            treatment(ExportClass::Cdrs, CdrErasure::Delete),
            Treatment::Delete
        );
        assert!(matches!(
            treatment(ExportClass::PortRequests, CdrErasure::Pseudonymise),
            Treatment::Keep(_)
        ));
    }

    fn subject(ids: &[&str], token: &str) -> Subject {
        Subject {
            matcher: SubjectMatcher::new(ids),
            token: token.to_string(),
        }
    }

    fn row(data: serde_json::Value) -> SubjectRow {
        SubjectRow {
            id: "r1".into(),
            link: RowLink::Identifiers(Vec::new()),
            data,
            at: None,
            call_id: None,
            file: None,
            others_heard: false,
        }
    }

    #[test]
    fn a_record_keeps_its_form_without_the_subject() {
        let subjects = [subject(
            &["alice@pbx.example", "+14155550100"],
            "erased-aaaa",
        )];
        let update = pseudonymised(
            ExportClass::Cdrs,
            &row(serde_json::json!({
                "from_uri": "sip:alice@pbx.example",
                "from_display_name": "Alice",
                "to_uri": "sip:bob@pbx.example",
                "to_display_name": "Bob",
                "sip_call_id": "abc@host",
                "custom_data": { "billing_code": "77", "callback": "+14155550100" }
            })),
            &subjects,
        )
        .unwrap();
        let f = &update.fields;
        assert_eq!(f["from_uri"], "sip:erased-aaaa@pbx.example");
        assert!(f["from_display_name"].is_null());
        assert!(!f.contains_key("to_uri"), "the other party is left alone");
        assert!(!f.contains_key("to_display_name"));
        assert_eq!(f["sip_call_id"], "erased");
        assert_eq!(
            f["custom_data"],
            serde_json::json!({ "billing_code": "77" })
        );

        let untouched = pseudonymised(
            ExportClass::Cdrs,
            &row(
                serde_json::json!({ "from_uri": "sip:bob@pbx.example", "to_uri": "sip:carol@pbx.example" }),
            ),
            &subjects,
        );
        assert!(untouched.is_none());
    }

    #[test]
    fn an_audit_event_loses_the_subject_as_actor_and_in_its_path() {
        let subjects = [subject(&["alice"], "erased-bbbb")];
        let update = pseudonymised(
            ExportClass::AuditEvents,
            &row(serde_json::json!({ "actor": "alice", "path": "/api/v1/users/alice/avatar" })),
            &subjects,
        )
        .unwrap();
        assert_eq!(update.fields["actor"], "erased-bbbb");
        assert_eq!(update.fields["path"], "/api/v1/users/erased-bbbb/avatar");
        assert_eq!(
            owner_cleared(ExportClass::Meetings, "m").fields["owner_id"],
            serde_json::Value::Null
        );
        assert!(owner_cleared(ExportClass::Devices, "d")
            .fields
            .contains_key("owner_user_id"));
    }
}
