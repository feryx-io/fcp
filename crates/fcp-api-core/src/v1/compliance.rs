//! Compliance administration: retention/consent policy, on-demand retention
//! runs, and the admin audit log.

use crate::auth::{can_access_all_tenants, AuthIdentity, TenantContext, PLATFORM_TENANT_HEADER};
use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Extension, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use fcp_core::compliance::{
    run_retention, AuditEvent, AuditQuery, CompliancePolicy, ComplianceStore, ConsentFailureAction,
    RecordingConsentMode, RetentionReport, MIN_NOTICE_INTERVAL_SECS,
};
use fcp_core::permissions::Permission;
use fcp_core::TenantId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

fn require_store(state: &ApiState) -> Result<&Arc<dyn ComplianceStore>, ApiError> {
    state
        .storage
        .as_ref()
        .and_then(|s| s.compliance_store())
        .ok_or_else(|| ApiError::Internal("Compliance store not configured".to_string()))
}

/// The compliance routes take their own permissions rather than any
/// admin's (`docs/COMPLIANCE_FOLLOWUPS.md` §4). A request with no
/// identity reached a router without authentication and is let through, as
/// on every other route.
fn permit(
    state: &ApiState,
    identity: &Option<Extension<AuthIdentity>>,
    permission: Permission,
) -> Result<(), ApiError> {
    match identity {
        Some(Extension(identity)) => {
            crate::auth::check_permission(identity, permission, state.deployment_mode)
        }
        None => Ok(()),
    }
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

// ─── Policy ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PolicyResponse {
    pub tenant_id: String,
    pub cdr_retention_days: Option<u32>,
    pub sip_trace_retention_days: Option<u32>,
    pub flow_trace_retention_days: Option<u32>,
    pub recording_retention_days: Option<u32>,
    pub audit_retention_days: Option<u32>,
    pub voicemail_retention_days: Option<u32>,
    pub recording_consent: String,
    pub recording_consent_prompt_id: Option<String>,
    /// `record` or `skip`: whether a recording goes ahead when its
    /// announcement cannot be played.
    pub recording_consent_on_failure: String,
    /// A notice tone every this many seconds while a call is recorded.
    pub recording_notice_interval_secs: Option<u32>,
    /// Days the tenant gives itself to answer a subject request.
    pub subject_request_days: u32,
    /// `pseudonymise` or `delete`: what an erasure does to call records.
    pub cdr_erasure: String,
    /// CDRs are deleted only once the archive has confirmed them.
    pub cdr_archive_required: bool,
    /// Unix seconds: every one of the tenant's CDRs that started before
    /// this is in the archive, confirmed. `null` when none is.
    pub cdr_archived_before: Option<u64>,
    /// False until the tenant has saved a policy (defaults shown).
    pub configured: bool,
    pub updated_at: u64,
}

impl PolicyResponse {
    fn from_policy(
        p: CompliancePolicy,
        configured: bool,
        archived_before: Option<SystemTime>,
    ) -> Self {
        Self {
            tenant_id: p.tenant_id.to_string(),
            cdr_retention_days: p.cdr_retention_days,
            sip_trace_retention_days: p.sip_trace_retention_days,
            flow_trace_retention_days: p.flow_trace_retention_days,
            recording_retention_days: p.recording_retention_days,
            audit_retention_days: p.audit_retention_days,
            voicemail_retention_days: p.voicemail_retention_days,
            recording_consent: p.recording_consent.to_string(),
            recording_consent_prompt_id: p.recording_consent_prompt_id,
            recording_consent_on_failure: p.recording_consent_on_failure.as_str().to_string(),
            recording_notice_interval_secs: p.recording_notice_interval_secs,
            subject_request_days: p.subject_request_days,
            cdr_erasure: p.cdr_erasure.as_str().to_string(),
            cdr_archive_required: p.cdr_archive_required,
            cdr_archived_before: archived_before.map(unix_secs),
            configured,
            updated_at: unix_secs(p.updated_at),
        }
    }
}

/// Every field optional; `null` on a retention field means "keep forever".
#[derive(Debug, Deserialize)]
pub struct UpdatePolicyRequest {
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub cdr_retention_days: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub sip_trace_retention_days: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub flow_trace_retention_days: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub recording_retention_days: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub audit_retention_days: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub voicemail_retention_days: Option<Option<u32>>,
    pub recording_consent: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub recording_consent_prompt_id: Option<Option<String>>,
    pub recording_consent_on_failure: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub recording_notice_interval_secs: Option<Option<u32>>,
    pub subject_request_days: Option<u32>,
    pub cdr_erasure: Option<String>,
    pub cdr_archive_required: Option<bool>,
}

/// How far the tenant's CDR archive is confirmed; `None` when it is not
/// known (no storage, or the lookup failed, which is logged).
async fn archived_before(state: &ApiState, tenant: &fcp_core::TenantId) -> Option<SystemTime> {
    use fcp_core::storage::CdrStore;
    let storage = state.storage.as_ref()?;
    match storage.cdr_archive_frontier(tenant).await {
        Ok(frontier) => frontier,
        Err(e) => {
            tracing::warn!(tenant_id = %tenant, error = %e, "Could not read the CDR archive frontier");
            None
        }
    }
}

/// Distinguish "absent" (keep current) from `null` (clear).
fn deserialize_double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn check_days(name: &str, days: Option<u32>) -> Result<(), ApiError> {
    if let Some(d) = days {
        if d == 0 || d > 36_500 {
            return Err(ApiError::BadRequest(format!(
                "{name} must be between 1 and 36500 days, or null to keep forever"
            )));
        }
    }
    Ok(())
}

pub async fn get_policy(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::ComplianceManage)?;
    let store = require_store(&state)?;
    let stored = store
        .get_compliance_policy(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let configured = stored.is_some();
    let policy = stored.unwrap_or_else(|| CompliancePolicy::defaults(tenant.0));
    let archived = archived_before(&state, &tenant.0).await;
    Ok(Json(PolicyResponse::from_policy(
        policy, configured, archived,
    )))
}

pub async fn update_policy(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Json(req): Json<UpdatePolicyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::ComplianceManage)?;
    let store = require_store(&state)?;
    let mut policy = store
        .get_compliance_policy(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| CompliancePolicy::defaults(tenant.0));

    if let Some(v) = req.cdr_retention_days {
        check_days("cdr_retention_days", v)?;
        policy.cdr_retention_days = v;
    }
    if let Some(v) = req.sip_trace_retention_days {
        check_days("sip_trace_retention_days", v)?;
        policy.sip_trace_retention_days = v;
    }
    if let Some(v) = req.flow_trace_retention_days {
        check_days("flow_trace_retention_days", v)?;
        policy.flow_trace_retention_days = v;
    }
    if let Some(v) = req.recording_retention_days {
        check_days("recording_retention_days", v)?;
        policy.recording_retention_days = v;
    }
    if let Some(v) = req.audit_retention_days {
        check_days("audit_retention_days", v)?;
        policy.audit_retention_days = v;
    }
    if let Some(v) = req.voicemail_retention_days {
        check_days("voicemail_retention_days", v)?;
        policy.voicemail_retention_days = v;
    }
    if let Some(required) = req.cdr_archive_required {
        policy.cdr_archive_required = required;
    }
    if let Some(mode) = req.recording_consent {
        policy.recording_consent = match mode.as_str() {
            "none" => RecordingConsentMode::None,
            "announce" => RecordingConsentMode::Announce,
            other => {
                return Err(ApiError::BadRequest(format!(
                    "recording_consent must be 'none' or 'announce', got '{other}'"
                )))
            }
        };
    }
    if let Some(prompt) = req.recording_consent_prompt_id {
        if let Some(p) = &prompt {
            crate::validation::reject_crlf("recording_consent_prompt_id", p)?;
            if let Some(prompts) = state.storage.as_ref().and_then(|s| s.prompt_store()) {
                let id = p
                    .parse::<fcp_core::PromptId>()
                    .map_err(|e| ApiError::BadRequest(format!("Invalid prompt id: {e}")))?;
                let found = prompts
                    .get_prompt(&id)
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
                    .filter(|pr| pr.tenant_id == tenant.0);
                if found.is_none() {
                    return Err(ApiError::BadRequest(format!("Prompt {p} not found")));
                }
            }
        }
        policy.recording_consent_prompt_id = prompt;
    }
    if let Some(action) = req.recording_consent_on_failure {
        policy.recording_consent_on_failure =
            ConsentFailureAction::parse(&action).ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "recording_consent_on_failure must be 'record' or 'skip', got '{action}'"
                ))
            })?;
    }
    if let Some(how) = req.cdr_erasure {
        policy.cdr_erasure = fcp_core::compliance::CdrErasure::parse(&how).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "cdr_erasure must be 'pseudonymise' or 'delete', got '{how}'"
            ))
        })?;
    }
    if let Some(days) = req.subject_request_days {
        if !(1..=365).contains(&days) {
            return Err(ApiError::BadRequest(
                "subject_request_days must be between 1 and 365".to_string(),
            ));
        }
        policy.subject_request_days = days;
    }
    if let Some(interval) = req.recording_notice_interval_secs {
        if let Some(secs) = interval {
            if secs < MIN_NOTICE_INTERVAL_SECS {
                return Err(ApiError::BadRequest(format!(
                    "recording_notice_interval_secs must be at least {MIN_NOTICE_INTERVAL_SECS}"
                )));
            }
        }
        policy.recording_notice_interval_secs = interval;
    }
    if policy.recording_consent == RecordingConsentMode::Announce
        && policy.recording_consent_prompt_id.is_none()
    {
        return Err(ApiError::BadRequest(
            "recording_consent 'announce' needs recording_consent_prompt_id".to_string(),
        ));
    }
    policy.updated_at = SystemTime::now();
    store
        .put_compliance_policy(&policy)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    info!(tenant_id = %tenant.0, "Compliance policy updated");
    let archived = archived_before(&state, &tenant.0).await;
    Ok(Json(PolicyResponse::from_policy(policy, true, archived)))
}

// ─── Retention ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RetentionRunResponse {
    pub tenant_id: String,
    pub ran_at: u64,
    pub cdrs_deleted: u64,
    pub sip_traces_deleted: u64,
    pub flow_traces_deleted: u64,
    pub recordings_deleted: u64,
    /// Conference recordings whose registry row the run removed.
    pub conference_recordings_deleted: u64,
    pub voicemail_messages_deleted: u64,
    pub audit_events_deleted: u64,
    pub total_deleted: u64,
    pub errors: Vec<String>,
}

impl From<RetentionReport> for RetentionRunResponse {
    fn from(r: RetentionReport) -> Self {
        Self {
            tenant_id: r.tenant_id.to_string(),
            ran_at: unix_secs(r.ran_at),
            total_deleted: r.total_deleted(),
            cdrs_deleted: r.cdrs_deleted,
            sip_traces_deleted: r.sip_traces_deleted,
            flow_traces_deleted: r.flow_traces_deleted,
            recordings_deleted: r.recordings_deleted,
            conference_recordings_deleted: r.conference_recordings_deleted,
            voicemail_messages_deleted: r.voicemail_messages_deleted,
            audit_events_deleted: r.audit_events_deleted,
            errors: r.errors,
        }
    }
}

/// Run the tenant's retention policy now and record the outcome.
pub async fn run_retention_now(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::ComplianceManage)?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let store = require_store(&state)?;
    let policy = store
        .get_compliance_policy(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| CompliancePolicy::defaults(tenant.0));
    let actor = identity
        .as_ref()
        .map(|Extension(i)| i.actor())
        .unwrap_or_else(|| "anonymous".to_string());
    let report = run_retention(storage, &policy, &actor).await;
    store
        .record_retention_run(&report)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    info!(
        tenant_id = %tenant.0,
        deleted = report.total_deleted(),
        errors = report.errors.len(),
        "Retention run completed"
    );
    Ok(Json(RetentionRunResponse::from(report)))
}

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

pub async fn list_retention_runs(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Query(q): Query<LimitQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::ComplianceManage)?;
    let store = require_store(&state)?;
    let runs = store
        .list_retention_runs(&tenant.0, q.limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let out: Vec<RetentionRunResponse> = runs.into_iter().map(Into::into).collect();
    Ok(Json(out))
}

// ─── Audit log ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub actor: Option<String>,
    /// Route-template prefix, e.g. `POST /api/v1/users`.
    pub action: Option<String>,
    /// Unix seconds.
    pub since: Option<u64>,
    #[serde(default = "default_audit_limit")]
    pub limit: usize,
}

fn default_audit_limit() -> usize {
    100
}

#[derive(Debug, Serialize)]
pub struct AuditEventResponse {
    pub id: i64,
    pub tenant_id: Option<String>,
    pub actor: String,
    pub actor_kind: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub action: String,
    pub detail: Option<serde_json::Value>,
    pub source_ip: Option<String>,
    pub occurred_at: u64,
}

impl From<AuditEvent> for AuditEventResponse {
    fn from(e: AuditEvent) -> Self {
        Self {
            id: e.id,
            tenant_id: e.tenant_id.map(|t| t.to_string()),
            actor: e.actor,
            actor_kind: e.actor_kind,
            method: e.method,
            path: e.path,
            status: e.status,
            action: e.action,
            detail: e.detail,
            source_ip: e.source_ip,
            occurred_at: unix_secs(e.occurred_at),
        }
    }
}

/// Tenant admins see their tenant; the platform token sees every tenant
/// unless it selected one with the override header.
pub async fn list_audit_log(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Query(q): Query<AuditLogQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::AuditView)?;
    let store = require_store(&state)?;
    let platform_wide = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0))
        && !headers.contains_key(PLATFORM_TENANT_HEADER);
    let tenant_id: Option<TenantId> = if platform_wide { None } else { Some(tenant.0) };
    let events = store
        .list_audit_events(&AuditQuery {
            tenant_id,
            actor: q.actor,
            action_prefix: q.action,
            since: q
                .since
                .map(|s| UNIX_EPOCH + std::time::Duration::from_secs(s)),
            limit: q.limit,
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let out: Vec<AuditEventResponse> = events.into_iter().map(Into::into).collect();
    Ok(Json(out))
}

// ─── Deletion log ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DeletionLogQuery {
    /// `cdr`, `call_recording`, `conference_recording`, `voicemail`,
    /// `sip_trace`, `flow_trace`, `audit_event`, `orphan_file`.
    pub class: Option<String>,
    /// Unix seconds.
    pub since: Option<u64>,
    /// What one subject request deleted.
    pub request_id: Option<String>,
    #[serde(default = "default_audit_limit")]
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct DeletionResponse {
    pub id: i64,
    pub tenant_id: Option<String>,
    pub class: String,
    pub item: String,
    pub detail: Option<serde_json::Value>,
    pub cause: String,
    pub actor: String,
    pub request_id: Option<String>,
    pub deleted_at: u64,
}

/// `GET /compliance/deletions`: what was deleted, when, why and by whom
/// (`docs/COMPLIANCE_FOLLOWUPS.md` §7). Tenant admins see their tenant; the
/// platform token sees every tenant, platform upkeep included, unless it
/// selected one.
pub async fn list_deletions(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Query(q): Query<DeletionLogQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::AuditView)?;
    let store = require_store(&state)?;
    let platform_wide = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0))
        && !headers.contains_key(PLATFORM_TENANT_HEADER);
    let records = store
        .list_deletions(&fcp_core::compliance::DeletionQuery {
            tenant_id: (!platform_wide).then_some(tenant.0),
            class: q.class,
            since: q
                .since
                .map(|s| UNIX_EPOCH + std::time::Duration::from_secs(s)),
            request_id: q.request_id,
            limit: q.limit,
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let out: Vec<DeletionResponse> = records
        .into_iter()
        .map(|r| DeletionResponse {
            id: r.id,
            tenant_id: r.tenant_id.map(|t| t.to_string()),
            class: r.class,
            item: r.item,
            detail: r.detail,
            cause: r.cause,
            actor: r.actor,
            request_id: r.request_id,
            deleted_at: unix_secs(r.deleted_at),
        })
        .collect();
    Ok(Json(out))
}

// ─── Legal holds (docs/COMPLIANCE_FOLLOWUPS.md §5) ───────────────────────

use fcp_core::legal_hold::{DataClass, HoldRelease, HoldScope, LegalHold, SubjectIdentifier};

#[derive(Debug, Deserialize)]
pub struct PlaceHoldRequest {
    pub matter: String,
    #[serde(default)]
    pub reason: Option<String>,
    /// Unix seconds; data from this instant.
    #[serde(default)]
    pub from: Option<u64>,
    /// Unix seconds; data before this instant.
    #[serde(default)]
    pub to: Option<u64>,
    #[serde(default)]
    pub subjects: Vec<SubjectIdentifier>,
    #[serde(default)]
    pub call_ids: Vec<String>,
    #[serde(default)]
    pub classes: Vec<DataClass>,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseHoldRequest {
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct ListHoldsQuery {
    /// Released holds too.
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Serialize)]
pub struct HoldResponse {
    pub id: String,
    pub tenant_id: String,
    pub matter: String,
    pub reason: Option<String>,
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub subjects: Vec<SubjectIdentifier>,
    pub call_ids: Vec<String>,
    pub classes: Vec<DataClass>,
    /// The subjects as the store matches them.
    pub subject_keys: Vec<String>,
    pub created_by: String,
    pub created_at: u64,
    pub active: bool,
    pub released_by: Option<String>,
    pub released_at: Option<u64>,
    pub release_reason: Option<String>,
    /// Items held now, by class; `null` for a class too large to count
    /// quickly. Only on `GET /compliance/holds/:id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held: Option<std::collections::BTreeMap<String, Option<u64>>>,
}

impl From<LegalHold> for HoldResponse {
    fn from(h: LegalHold) -> Self {
        Self {
            id: h.id.to_string(),
            tenant_id: h.tenant_id.to_string(),
            matter: h.matter,
            reason: h.reason,
            from: h.scope.from.map(unix_secs),
            to: h.scope.to.map(unix_secs),
            subjects: h.scope.subjects,
            call_ids: h.scope.call_ids,
            classes: h.scope.classes,
            subject_keys: h.subject_keys,
            created_by: h.created_by,
            created_at: unix_secs(h.created_at),
            active: h.released.is_none(),
            released_by: h.released.as_ref().map(|r| r.by.clone()),
            released_at: h.released.as_ref().map(|r| unix_secs(r.at)),
            release_reason: h.released.map(|r| r.reason),
            held: None,
        }
    }
}

fn actor_of(identity: &Option<Extension<AuthIdentity>>) -> String {
    identity
        .as_ref()
        .map(|Extension(i)| i.actor())
        .unwrap_or_else(|| "anonymous".to_string())
}

fn parse_hold_id(id: &str) -> Result<fcp_core::LegalHoldId, ApiError> {
    id.parse()
        .map_err(|_| ApiError::BadRequest(format!("Invalid hold id: {id}")))
}

/// `POST /compliance/holds`: place a hold.
pub async fn place_hold(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Json(req): Json<PlaceHoldRequest>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::LegalHoldManage)?;
    let store = require_store(&state)?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let matter = req.matter.trim().to_string();
    if matter.is_empty() {
        return Err(ApiError::BadRequest("matter is required".to_string()));
    }
    crate::validation::reject_crlf("matter", &matter)?;
    if let (Some(from), Some(to)) = (req.from, req.to) {
        if from >= to {
            return Err(ApiError::BadRequest("from must be before to".to_string()));
        }
    }
    let secs = |s: u64| UNIX_EPOCH + std::time::Duration::from_secs(s);
    let subject_keys = fcp_core::legal_hold::expand_subjects(storage, &tenant.0, &req.subjects)
        .await
        .map_err(|e| ApiError::BadRequest(format!("subjects: {e}")))?;
    let hold = LegalHold {
        id: fcp_core::LegalHoldId::new(),
        tenant_id: tenant.0,
        matter,
        reason: req.reason.filter(|r| !r.trim().is_empty()),
        scope: HoldScope {
            from: req.from.map(secs),
            to: req.to.map(secs),
            subjects: req.subjects,
            call_ids: req.call_ids,
            classes: req.classes,
        },
        subject_keys,
        created_by: actor_of(&identity),
        created_at: SystemTime::now(),
        released: None,
    };
    store.place_hold(&hold).await.map_err(ApiError::storage)?;
    info!(tenant_id = %tenant.0, hold = %hold.id, matter = %hold.matter, "Legal hold placed");
    state
        .publish_event(fcp_events::PlatformEvent::Compliance(
            fcp_events::ComplianceEvent::HoldPlaced {
                hold_id: hold.id.to_string(),
                tenant_id: tenant.0,
                matter: hold.matter.clone(),
                actor: hold.created_by.clone(),
                timestamp: hold.created_at,
            },
        ))
        .await;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(HoldResponse::from(hold)),
    ))
}

/// `GET /compliance/holds?all=`: the tenant's holds, active ones unless
/// `all`.
pub async fn list_holds(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Query(q): Query<ListHoldsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::LegalHoldManage)?;
    let store = require_store(&state)?;
    let holds = store
        .list_holds(&tenant.0, q.all)
        .await
        .map_err(ApiError::storage)?;
    Ok(Json(
        holds
            .into_iter()
            .map(HoldResponse::from)
            .collect::<Vec<_>>(),
    ))
}

/// `GET /compliance/holds/:id`: a hold, with what it holds now.
pub async fn get_hold(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::LegalHoldManage)?;
    let store = require_store(&state)?;
    let id = parse_hold_id(&id)?;
    let hold = store
        .get_hold(&tenant.0, &id)
        .await
        .map_err(ApiError::storage)?
        .ok_or_else(|| ApiError::NotFound(format!("No hold {id}")))?;
    let counts = if hold.is_active() {
        store
            .held_counts(&tenant.0, &id)
            .await
            .map_err(ApiError::storage)?
    } else {
        Vec::new()
    };
    let mut response = HoldResponse::from(hold);
    response.held = Some(
        counts
            .into_iter()
            .map(|(class, n)| (class.as_str().to_string(), n))
            .collect(),
    );
    Ok(Json(response))
}

/// `POST /compliance/holds/:id/release`: release an active hold. The hold
/// stays, with who released it, when and why.
pub async fn release_hold(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(req): Json<ReleaseHoldRequest>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::LegalHoldManage)?;
    let store = require_store(&state)?;
    let id = parse_hold_id(&id)?;
    let reason = req.reason.trim().to_string();
    if reason.is_empty() {
        return Err(ApiError::BadRequest("a reason is required".to_string()));
    }
    let hold = store
        .get_hold(&tenant.0, &id)
        .await
        .map_err(ApiError::storage)?
        .ok_or_else(|| ApiError::NotFound(format!("No hold {id}")))?;
    let release = HoldRelease {
        by: actor_of(&identity),
        at: SystemTime::now(),
        reason,
    };
    if !store
        .release_hold(&tenant.0, &id, &release)
        .await
        .map_err(ApiError::storage)?
    {
        return Err(ApiError::Conflict(format!("Hold {id} is already released")));
    }
    info!(tenant_id = %tenant.0, hold = %id, "Legal hold released");
    state
        .publish_event(fcp_events::PlatformEvent::Compliance(
            fcp_events::ComplianceEvent::HoldReleased {
                hold_id: id.to_string(),
                tenant_id: tenant.0,
                matter: hold.matter.clone(),
                actor: release.by.clone(),
                reason: release.reason.clone(),
                timestamp: release.at,
            },
        ))
        .await;
    let mut released = hold;
    released.released = Some(release);
    Ok(Json(HoldResponse::from(released)))
}

// ─── Subject requests (docs/COMPLIANCE_FOLLOWUPS.md §6) ─────────────────

use fcp_core::subject_request::{SubjectRequest, SubjectRequestKind, SubjectRequestStatus};

#[derive(Debug, Deserialize)]
pub struct FileSubjectRequest {
    /// `export` or `erasure`.
    pub kind: String,
    pub subjects: Vec<SubjectIdentifier>,
    /// The tenant's own reference: a ticket, a letter.
    #[serde(default)]
    pub reference: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListSubjectRequestsQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct BundleResponse {
    pub size_bytes: u64,
    pub sha256: String,
    pub expires_at: u64,
    pub removed: bool,
}

#[derive(Debug, Serialize)]
pub struct SubjectRequestResponse {
    pub id: String,
    pub tenant_id: String,
    pub kind: String,
    pub status: String,
    pub subjects: Vec<SubjectIdentifier>,
    /// The subjects as the request found them when it started.
    pub expansion: Option<fcp_core::legal_hold::SubjectExpansion>,
    pub reference: Option<String>,
    pub requested_by: String,
    pub received_at: u64,
    pub due_at: u64,
    /// Open and past `due_at`.
    pub overdue: bool,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub report: Option<fcp_core::subject_request::ExportReport>,
    pub bundle: Option<BundleResponse>,
    pub error: Option<String>,
    pub confirmed_at: Option<u64>,
    pub confirmed_by: Option<String>,
    /// What an erasure would do, for a person to confirm.
    pub preview: Option<fcp_core::subject_erasure::ErasureReport>,
    /// What an erasure or a tenant's deletion did.
    pub erasure: Option<fcp_core::subject_erasure::ErasureReport>,
}

impl From<SubjectRequest> for SubjectRequestResponse {
    fn from(r: SubjectRequest) -> Self {
        Self {
            overdue: r.is_overdue(SystemTime::now()),
            id: r.id.to_string(),
            tenant_id: r.tenant_id.to_string(),
            kind: r.kind.as_str().to_string(),
            status: r.status.as_str().to_string(),
            subjects: r.subjects,
            expansion: r.expansion,
            reference: r.reference,
            requested_by: r.requested_by,
            received_at: unix_secs(r.received_at),
            due_at: unix_secs(r.due_at),
            started_at: r.started_at.map(unix_secs),
            completed_at: r.completed_at.map(unix_secs),
            report: r.report,
            bundle: r.bundle.map(|b| BundleResponse {
                size_bytes: b.size_bytes,
                sha256: b.sha256,
                expires_at: unix_secs(b.expires_at),
                removed: b.removed,
            }),
            error: r.error,
            confirmed_at: r.confirmed_at.map(unix_secs),
            confirmed_by: r.confirmed_by,
            preview: r.preview,
            erasure: r.erasure,
        }
    }
}

fn parse_request_id(id: &str) -> Result<fcp_core::SubjectRequestId, ApiError> {
    id.parse()
        .map_err(|_| ApiError::BadRequest(format!("Invalid subject request id: {id}")))
}

/// `POST /compliance/subject-requests`: file a request. The subjects are
/// checked now (a user the tenant does not have is refused) and expanded
/// again when the request runs.
pub async fn file_subject_request(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Json(req): Json<FileSubjectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::SubjectRequestManage)?;
    let store = require_store(&state)?;
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))?;
    let kind = match SubjectRequestKind::parse(&req.kind) {
        Some(kind @ (SubjectRequestKind::Export | SubjectRequestKind::Erasure)) => kind,
        Some(SubjectRequestKind::TenantDeletion) => {
            return Err(ApiError::BadRequest(
                "a tenant is deleted with DELETE /tenants/:id".to_string(),
            ))
        }
        None => {
            return Err(ApiError::BadRequest(format!(
                "kind must be 'export' or 'erasure', got '{}'",
                req.kind
            )))
        }
    };
    if req.subjects.is_empty() {
        return Err(ApiError::BadRequest(
            "at least one subject is required".to_string(),
        ));
    }
    let reference = req
        .reference
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    if let Some(r) = &reference {
        crate::validation::reject_crlf("reference", r)?;
    }
    let expansion = fcp_core::legal_hold::expand(storage, &tenant.0, &req.subjects)
        .await
        .map_err(|e| ApiError::BadRequest(format!("subjects: {e}")))?;
    if expansion.identifiers.is_empty() {
        return Err(ApiError::BadRequest(
            "subjects: nothing to identify the subject by".to_string(),
        ));
    }
    let days = store
        .get_compliance_policy(&tenant.0)
        .await
        .map_err(ApiError::storage)?
        .map(|p| p.subject_request_days)
        .unwrap_or(fcp_core::subject_request::DEFAULT_REQUEST_DAYS);
    let mut request =
        SubjectRequest::received(tenant.0, kind, req.subjects, actor_of(&identity), days);
    request.reference = reference;
    store
        .create_subject_request(&request)
        .await
        .map_err(ApiError::storage)?;
    info!(tenant_id = %tenant.0, request = %request.id, kind = kind.as_str(), "Subject request filed");
    Ok((
        axum::http::StatusCode::CREATED,
        Json(SubjectRequestResponse::from(request)),
    ))
}

/// `GET /compliance/subject-requests`: the tenant's requests, newest first.
pub async fn list_subject_requests(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Query(q): Query<ListSubjectRequestsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::SubjectRequestManage)?;
    let store = require_store(&state)?;
    let status = match q.status.as_deref() {
        None => None,
        Some(s) => Some(
            SubjectRequestStatus::parse(s)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown status '{s}'")))?,
        ),
    };
    let requests = store
        .list_subject_requests(&tenant.0, status, q.limit.unwrap_or(100).clamp(1, 1000))
        .await
        .map_err(ApiError::storage)?;
    let requests: Vec<SubjectRequestResponse> = requests.into_iter().map(Into::into).collect();
    Ok(Json(
        serde_json::json!({ "count": requests.len(), "requests": requests }),
    ))
}

async fn scoped_subject_request(
    state: &ApiState,
    tenant: &TenantId,
    id: &str,
) -> Result<SubjectRequest, ApiError> {
    let id = parse_request_id(id)?;
    require_store(state)?
        .get_subject_request(tenant, &id)
        .await
        .map_err(ApiError::storage)?
        .ok_or_else(|| ApiError::NotFound(format!("Subject request {id} not found")))
}

/// `GET /compliance/subject-requests/:id`: one request, with its report.
pub async fn get_subject_request(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::SubjectRequestManage)?;
    let request = scoped_subject_request(&state, &tenant.0, &id).await?;
    Ok(Json(SubjectRequestResponse::from(request)))
}

/// `POST /compliance/subject-requests/:id/confirm`: confirm an erasure's
/// preview. The erasure then runs against the data as it is, not as the
/// preview found it, and cannot be undone. `409` unless a preview is waiting.
pub async fn confirm_subject_request(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::SubjectRequestManage)?;
    let store = require_store(&state)?;
    let mut request = scoped_subject_request(&state, &tenant.0, &id).await?;
    if request.kind != SubjectRequestKind::Erasure
        || request.status != SubjectRequestStatus::Preview
    {
        return Err(ApiError::Conflict(format!(
            "Subject request {} is a {} request {}; only an erasure's preview is confirmed",
            request.id,
            request.kind.as_str(),
            request.status.as_str()
        )));
    }
    request.status = SubjectRequestStatus::Confirmed;
    request.confirmed_at = Some(SystemTime::now());
    request.confirmed_by = Some(actor_of(&identity));
    store
        .update_subject_request(&request)
        .await
        .map_err(ApiError::storage)?;
    info!(tenant_id = %tenant.0, request = %request.id, by = ?request.confirmed_by, "Subject erasure confirmed");
    Ok(Json(SubjectRequestResponse::from(request)))
}

/// `GET /compliance/subject-requests/:id/bundle`: the export's bundle, a
/// gzipped tar. Every download is in the audit log. `409` until the export
/// has completed, `410` once the bundle has expired.
pub async fn download_subject_bundle(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::response::Response, ApiError> {
    permit(&state, &identity, Permission::SubjectRequestManage)?;
    let request = scoped_subject_request(&state, &tenant.0, &id).await?;
    let Some(bundle) = request.bundle.as_ref() else {
        return Err(ApiError::Conflict(format!(
            "Subject request {} is {}; it has no bundle",
            request.id,
            request.status.as_str()
        )));
    };
    if bundle.removed || bundle.expires_at <= SystemTime::now() {
        return Err(ApiError::Gone(format!(
            "The bundle of {} expired; file a new request",
            request.id
        )));
    }
    let file = tokio::fs::File::open(&bundle.path).await.map_err(|e| {
        ApiError::ServiceUnavailable(format!(
            "The bundle is not readable on this host ({e}); download it from the host that ran the export"
        ))
    })?;
    let stream = futures_util::stream::unfold(file, |mut file| async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 64 * 1024];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, std::io::Error>(axum::body::Bytes::from(buf)), file))
            }
            Err(e) => Some((Err(e), file)),
        }
    });
    info!(tenant_id = %tenant.0, request = %request.id, actor = %actor_of(&identity), "Subject export bundle downloaded");
    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("content-type", "application/gzip")
        .header("content-length", bundle.size_bytes.to_string())
        .header(
            "content-disposition",
            format!("attachment; filename=\"{}.tar.gz\"", request.id),
        )
        .header("x-content-sha256", bundle.sha256.clone())
        .body(axum::body::Body::from_stream(stream))
        .map_err(|e| ApiError::Internal(e.to_string()))
}
