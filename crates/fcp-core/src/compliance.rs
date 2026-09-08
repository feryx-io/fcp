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
    pub recording_consent: RecordingConsentMode,
    /// Prompt played when `recording_consent` is `announce`.
    pub recording_consent_prompt_id: Option<String>,
    pub updated_at: SystemTime,
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
            recording_consent: RecordingConsentMode::None,
            recording_consent_prompt_id: None,
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
    }
}

/// The instant before which data of a given age is past its window.
pub fn cutoff(now: SystemTime, retention_days: u32) -> SystemTime {
    now - Duration::from_secs(u64::from(retention_days) * 86_400)
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
    pub occurred_at: SystemTime,
}

/// Filter for [`ComplianceStore::list_audit_events`].
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// `None` lists every tenant (platform admins only).
    pub tenant_id: Option<TenantId>,
    pub actor: Option<String>,
    /// Match on the route template prefix (`POST /api/v1/users`).
    pub action_prefix: Option<String>,
    pub since: Option<SystemTime>,
    pub limit: usize,
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
    pub audit_events_deleted: u64,
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
            audit_events_deleted: 0,
            errors: Vec::new(),
        }
    }

    pub fn total_deleted(&self) -> u64 {
        self.cdrs_deleted
            + self.sip_traces_deleted
            + self.flow_traces_deleted
            + self.recordings_deleted
            + self.conference_recordings_deleted
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

    async fn record_retention_run(&self, report: &RetentionReport) -> anyhow::Result<()>;
    async fn list_retention_runs(
        &self,
        tenant_id: &TenantId,
        limit: usize,
    ) -> anyhow::Result<Vec<RetentionReport>>;
}

/// Apply `policy` to `storage`. Each window is independent; an error in one
/// step is recorded and the next step still runs. Recording files are
/// resolved from the CDRs past the recording window (their `recording_url`,
/// a local path) and, when `recordings_dir` is given, any file in that
/// directory tree older than the window.
pub async fn run_retention(
    storage: &DynStorage,
    policy: &CompliancePolicy,
    recordings_dir: Option<&Path>,
) -> RetentionReport {
    let now = SystemTime::now();
    let tenant = policy.tenant_id;
    let mut report = RetentionReport::new(tenant);

    // Flow traces first: they are found through the CDRs they belong to.
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
        match storage.list_recordings_before(&tenant, before).await {
            Ok(entries) => {
                for (call_id, url) in entries {
                    let path = url.strip_prefix("file://").unwrap_or(&url);
                    match tokio::fs::remove_file(path).await {
                        Ok(()) => report.recordings_deleted += 1,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => report.errors.push(format!("recording {url}: {e}")),
                    }
                    if let Err(e) = storage.clear_recording_url(&call_id).await {
                        report
                            .errors
                            .push(format!("recording {call_id}: unlink from CDR: {e}"));
                    }
                }
            }
            Err(e) => report.errors.push(format!("recordings: {e}")),
        }
        if let Some(dir) = recordings_dir {
            match prune_files_older_than(dir, before).await {
                Ok(n) => report.recordings_deleted += n,
                Err(e) => report
                    .errors
                    .push(format!("recordings dir {}: {e}", dir.display())),
            }
        }

        // Conference recordings are the holding node's to delete: it has
        // the file, and it applies this same policy on its own sweep.
        // What is left here well past the cutoff belonged to a node that
        // is gone, so the row goes and the file — which nothing can now
        // reach — is reported rather than silently forgotten.
        if let Some(store) = storage.recording_store() {
            let abandoned = before - CONFERENCE_RECORDING_GRACE;
            match store.expired_recordings(&tenant, abandoned).await {
                Ok(recordings) => {
                    for recording in recordings {
                        // On shared storage the file is reachable here;
                        // usually it is not, and that is not an error.
                        match tokio::fs::remove_file(&recording.path).await {
                            Ok(()) | Err(_) => {}
                        }
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

    if let Some(days) = policy.cdr_retention_days {
        match storage.delete_cdrs_before(&tenant, cutoff(now, days)).await {
            Ok(n) => report.cdrs_deleted = n,
            Err(e) => report.errors.push(format!("cdrs: {e}")),
        }
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

    report
}

/// Delete regular files under `dir` (recursively) whose modification time
/// is before `before`. Directories are left in place.
pub async fn prune_files_older_than(dir: &Path, before: SystemTime) -> anyhow::Result<u64> {
    let mut deleted = 0;
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
                tokio::fs::remove_file(&path).await?;
                deleted += 1;
            }
        }
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn prune_deletes_only_old_files() {
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
        let n = prune_files_older_than(dir.path(), before).await.unwrap();
        assert_eq!(n, 1);
        assert!(!old.exists());
        assert!(new.exists());
        assert!(sub.is_dir(), "directories are kept");

        // A missing directory is not an error.
        let n = prune_files_older_than(&dir.path().join("nope"), before)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    /// Set a file's mtime without pulling in the `filetime` crate.
    fn filetime_set(path: &Path, when: SystemTime) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(when).unwrap();
    }
}
