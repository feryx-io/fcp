//! Compliance administration: retention/consent policy, on-demand retention
//! runs, and the admin audit log.

use crate::auth::{can_access_all_tenants, AuthIdentity, TenantContext, PLATFORM_TENANT_HEADER};
use crate::{error::ApiError, state::ApiState};
use axum::{
    extract::{Extension, Query, State},
    http::{header, HeaderMap, StatusCode},
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
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
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
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
#[derive(Debug, Deserialize, utoipa::ToSchema)]
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

#[utoipa::path(
    get,
    path = "/api/v1/compliance/policy",
    tag = "compliance",
    operation_id = "compliance_get_policy",
    responses(
        (status = 200, description = "OK", body = PolicyResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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

#[utoipa::path(
    put,
    path = "/api/v1/compliance/policy",
    tag = "compliance",
    operation_id = "compliance_update_policy",
    request_body = UpdatePolicyRequest,
    responses(
        (status = 200, description = "OK", body = PolicyResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
                    .filter(|pr| pr.visible_to(&tenant.0));
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RetentionRunResponse {
    pub tenant_id: String,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
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
#[utoipa::path(
    post,
    path = "/api/v1/compliance/retention/run",
    tag = "compliance",
    operation_id = "compliance_run_retention_now",
    responses(
        (status = 200, description = "OK", body = RetentionRunResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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

#[utoipa::path(
    get,
    path = "/api/v1/compliance/retention/runs",
    tag = "compliance",
    operation_id = "compliance_list_retention_runs",
    params(crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<RetentionRunResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_retention_runs(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::ComplianceManage)?;
    let store = require_store(&state)?;
    // A run a day: the newest thousand are years of them.
    let runs = store
        .list_retention_runs(&tenant.0, crate::page::MAX_LIMIT)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let out: Vec<RetentionRunResponse> = runs.into_iter().map(Into::into).collect();
    Ok(Json(crate::page::paginate(
        out,
        &page,
        crate::page::Order::Descending,
        |r| crate::page::time_key(r.ran_at, ""),
    )?))
}

// ─── Audit log ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditLogQuery {
    pub actor: Option<String>,
    /// Route-template prefix, e.g. `POST /api/v1/users`.
    pub action: Option<String>,
    /// Unix seconds.
    pub since: Option<u64>,
    /// Unix seconds: only events before it.
    pub until: Option<u64>,
    /// Only the request with this `X-Request-Id`: what a deletion's
    /// `api_request_id` names.
    pub api_request_id: Option<String>,
    /// A platform administrator's choice of rows: `all`, `platform` (the
    /// platform's own actions, without a tenant) or a tenant's id. Without
    /// it, the tenant picked (`x-fcp-tenant-id`), else all. Anyone else
    /// reads their own tenant's rows whatever this says.
    pub tenant: Option<String>,
}

/// Which audit rows a reader gets: a tenant's, the platform's, or every row.
struct AuditReach {
    tenant_id: Option<TenantId>,
    platform_only: bool,
    /// The reader is a platform administrator: actors are shown as they are.
    platform_reader: bool,
}

fn audit_reach(
    tenant: &TenantContext,
    identity: &Option<Extension<AuthIdentity>>,
    headers: &HeaderMap,
    q: &AuditLogQuery,
) -> Result<AuditReach, ApiError> {
    let platform_reader = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0));
    if !platform_reader {
        return Ok(AuditReach {
            tenant_id: Some(tenant.0),
            platform_only: false,
            platform_reader,
        });
    }
    let (tenant_id, platform_only) = match q.tenant.as_deref().map(str::trim) {
        Some("all") => (None, false),
        Some("platform") => (None, true),
        Some(id) if !id.is_empty() => (
            Some(id.parse::<TenantId>().map_err(|e| {
                ApiError::BadRequest(format!(
                    "tenant must be 'all', 'platform' or a tenant id: {e}"
                ))
            })?),
            false,
        ),
        _ if headers.contains_key(PLATFORM_TENANT_HEADER) => (Some(tenant.0), false),
        _ => (None, false),
    };
    Ok(AuditReach {
        tenant_id,
        platform_only,
        platform_reader,
    })
}

impl AuditLogQuery {
    /// The store's query for these filters, in the reach's rows.
    fn to_query(&self, reach: &AuditReach, limit: usize) -> AuditQuery {
        let at = |s: u64| UNIX_EPOCH + std::time::Duration::from_secs(s);
        AuditQuery {
            tenant_id: reach.tenant_id,
            platform_only: reach.platform_only,
            actor: self.actor.clone(),
            action_prefix: self.action.clone(),
            since: self.since.map(at),
            until: self.until.map(at),
            api_request_id: self.api_request_id.clone(),
            limit,
            before: None,
        }
    }
}

/// The keyset cursor of a time-ordered log: `(nanoseconds, id)`.
fn log_key(at: SystemTime, id: i64) -> String {
    let nanos = at.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{nanos:032}/{id:020}")
}

fn parse_log_key(key: &str) -> Result<(SystemTime, i64), ApiError> {
    let bad = || ApiError::BadRequest("Invalid cursor".into());
    let (nanos, id) = key.split_once('/').ok_or_else(bad)?;
    let nanos: u128 = nanos.parse().map_err(|_| bad())?;
    let id: i64 = id.parse().map_err(|_| bad())?;
    let at = UNIX_EPOCH
        + std::time::Duration::new(
            (nanos / 1_000_000_000) as u64,
            (nanos % 1_000_000_000) as u32,
        );
    Ok((at, id))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AuditEventResponse {
    pub id: i64,
    /// The tenant acted on; `null` for the platform's own actions.
    pub tenant_id: Option<String>,
    /// The actor's own tenant, `null` for the static token. Shown only to a
    /// platform administrator.
    pub actor_tenant_id: Option<String>,
    /// Who acted. A tenant's readers see an actor from outside it (a
    /// platform administrator) as `platform administrator`.
    pub actor: String,
    pub actor_kind: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub action: String,
    pub detail: Option<serde_json::Value>,
    pub source_ip: Option<String>,
    /// The request's `X-Request-Id`; the deletions it made name it too.
    pub api_request_id: Option<String>,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub occurred_at: u64,
}

impl From<AuditEvent> for AuditEventResponse {
    fn from(e: AuditEvent) -> Self {
        Self {
            id: e.id,
            tenant_id: e.tenant_id.map(|t| t.to_string()),
            actor_tenant_id: e.actor_tenant_id.map(|t| t.to_string()),
            actor: e.actor,
            actor_kind: e.actor_kind,
            method: e.method,
            path: e.path,
            status: e.status,
            action: e.action,
            detail: e.detail,
            source_ip: e.source_ip,
            api_request_id: e.api_request_id,
            occurred_at: unix_secs(e.occurred_at),
        }
    }
}

/// An event as a reader sees it: a tenant's reader sees an actor from
/// outside the tenant — the static token, or someone of another tenant (a
/// platform administrator) — as `platform administrator`, without their
/// tenant or address.
fn as_seen(e: AuditEvent, platform_reader: bool) -> AuditEventResponse {
    let outsider =
        e.actor_kind == "static_token" || e.actor_tenant_id.is_some_and(|t| Some(t) != e.tenant_id);
    let mut r = AuditEventResponse::from(e);
    if !platform_reader {
        if outsider {
            r.actor = "platform administrator".to_string();
            r.actor_kind = "platform".to_string();
            r.source_ip = None;
        }
        r.actor_tenant_id = None;
    }
    r
}

/// A row belongs to the tenant acted on (docs/TENANT_SCOPE.md §3.3).
/// Tenant admins see their tenant's rows, platform administrators' actions
/// on it included; a platform administrator sees every row, one tenant's
/// (`tenant` or the picked tenant) or the platform's own.
#[utoipa::path(
    get,
    path = "/api/v1/audit-log",
    tag = "compliance",
    operation_id = "compliance_list_audit_log",
    params(AuditLogQuery, crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<AuditEventResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_audit_log(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Query(q): Query<AuditLogQuery>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::AuditView)?;
    let store = require_store(&state)?;
    let reach = audit_reach(&tenant, &identity, &headers, &q)?;
    // One more than the page, to know whether there is another.
    let mut query = q.to_query(&reach, page.limit()? + 1);
    query.before = page.after()?.as_deref().map(parse_log_key).transpose()?;
    let events = store
        .list_audit_events(&query)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let limit = page.limit()?;
    let next_key = (events.len() > limit).then(|| {
        let last = &events[limit - 1];
        log_key(last.occurred_at, last.id)
    });
    let out: Vec<AuditEventResponse> = events
        .into_iter()
        .take(limit)
        .map(|e| as_seen(e, reach.platform_reader))
        .collect();
    Ok(Json(crate::page::Page::new(out, next_key, None)))
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditExportQuery {
    /// `csv` (the default) or `jsonl`.
    pub format: Option<String>,
}

impl AuditExportQuery {
    /// Whether the export is JSON Lines.
    fn jsonl(&self) -> Result<bool, ApiError> {
        match self.format.as_deref().unwrap_or("csv") {
            "csv" => Ok(false),
            "jsonl" => Ok(true),
            other => Err(ApiError::BadRequest(format!(
                "Invalid format: {other}. Valid values: csv, jsonl"
            ))),
        }
    }
}

/// A CSV field, quoted when it must be.
fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

fn csv_line(fields: Vec<String>) -> String {
    let mut line = fields.join(",");
    line.push('\n');
    line
}

/// One audit event as a CSV line; the header is [`AUDIT_CSV_HEADER`].
fn audit_csv_row(e: &AuditEventResponse) -> String {
    let opt = |v: &Option<String>| csv_field(v.as_deref().unwrap_or(""));
    let occurred = crate::time::format(std::time::Duration::from_secs(e.occurred_at));
    let detail = e.detail.as_ref().map(|d| d.to_string()).unwrap_or_default();
    csv_line(vec![
        e.id.to_string(),
        csv_field(&occurred),
        opt(&e.tenant_id),
        csv_field(&e.actor),
        csv_field(&e.actor_kind),
        csv_field(&e.method),
        csv_field(&e.path),
        e.status.to_string(),
        csv_field(&e.action),
        opt(&e.source_ip),
        csv_field(&detail),
        opt(&e.actor_tenant_id),
        opt(&e.api_request_id),
    ])
}

const AUDIT_CSV_HEADER: &str =
    "id,occurred_at,tenant_id,actor,actor_kind,method,path,status,action,source_ip,detail,actor_tenant_id,api_request_id\n";

/// The rows an export reads from the store at a time.
const LOG_EXPORT_PAGE: usize = 1000;

/// A whole log as a streamed CSV or JSON Lines body, newest first: `first`
/// is the first page, read before the response starts so a failing query
/// is an error status and not an empty file; `fetch` reads the page before
/// a `(time, id)` key, `key` gives a row's, `render` writes one row.
fn log_export<R, F, Fut>(
    what: &'static str,
    jsonl: bool,
    csv_header: &'static str,
    first: Vec<R>,
    fetch: F,
    key: fn(&R) -> (SystemTime, i64),
    render: impl Fn(R) -> String + Send + 'static,
) -> impl IntoResponse
where
    R: Send + 'static,
    F: Fn((SystemTime, i64)) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = anyhow::Result<Vec<R>>> + Send,
{
    struct Walk<R, F, W> {
        fetch: F,
        render: W,
        page: Option<Vec<R>>,
        header_sent: bool,
    }
    let walk = Walk {
        fetch,
        render,
        page: Some(first),
        header_sent: jsonl,
    };
    let stream = futures_util::stream::unfold(walk, move |mut walk| async move {
        if !walk.header_sent {
            walk.header_sent = true;
            return Some((
                Ok::<_, std::io::Error>(axum::body::Bytes::from(csv_header)),
                walk,
            ));
        }
        let page = walk.page.take()?;
        if page.is_empty() {
            return None;
        }
        let full = page.len() == LOG_EXPORT_PAGE;
        let last = page.last().map(key);
        let mut chunk = String::new();
        for row in page {
            chunk.push_str(&(walk.render)(row));
        }
        match last.filter(|_| full) {
            Some(last) => match (walk.fetch)(last).await {
                Ok(next) => walk.page = Some(next),
                Err(e) => {
                    tracing::warn!(error = %e, what, "Log export failed part way");
                    return Some((
                        Err(std::io::Error::other(format!(
                            "{what} export failed part way"
                        ))),
                        walk,
                    ));
                }
            },
            None => walk.page = Some(Vec::new()),
        }
        Some((Ok(axum::body::Bytes::from(chunk)), walk))
    });
    let (content_type, extension) = if jsonl {
        ("application/x-ndjson", "jsonl")
    } else {
        ("text/csv; charset=utf-8", "csv")
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{what}.{extension}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        axum::body::Body::from_stream(stream),
    )
}

/// One row as a JSON Lines line.
fn json_line<T: Serialize>(row: &T) -> String {
    let mut line = serde_json::to_string(row).unwrap_or_default();
    line.push('\n');
    line
}

/// `GET /audit-log/export`: every event the listing's filters match,
/// newest first, streamed page by page as CSV or JSON Lines (D8, finding
/// 36). Who may see what is the listing's; the export is itself audited.
#[utoipa::path(
    get,
    path = "/api/v1/audit-log/export",
    tag = "compliance",
    operation_id = "compliance_export_audit_log",
    params(AuditLogQuery, AuditExportQuery),
    responses(
        (status = 200, description = "The events, streamed as CSV or JSON Lines", content_type = "text/csv", body = String),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn export_audit_log(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Query(q): Query<AuditLogQuery>,
    Query(export): Query<AuditExportQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::AuditView)?;
    let store = Arc::clone(require_store(&state)?);
    let jsonl = export.jsonl()?;
    let reach = audit_reach(&tenant, &identity, &headers, &q)?;
    let platform_reader = reach.platform_reader;
    let query = q.to_query(&reach, LOG_EXPORT_PAGE);
    let first = store
        .list_audit_events(&query)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(log_export(
        "audit-log",
        jsonl,
        AUDIT_CSV_HEADER,
        first,
        move |before| {
            let store = Arc::clone(&store);
            let mut query = query.clone();
            query.before = Some(before);
            async move { store.list_audit_events(&query).await }
        },
        |e: &AuditEvent| (e.occurred_at, e.id),
        move |e| {
            let e = as_seen(e, platform_reader);
            if jsonl {
                json_line(&e)
            } else {
                audit_csv_row(&e)
            }
        },
    ))
}

// ─── Deletion log ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeletionLogQuery {
    /// `cdr`, `call_recording`, `conference_recording`, `voicemail`,
    /// `sip_trace`, `flow_trace`, `audit_event`, `orphan_file`.
    pub class: Option<String>,
    /// Unix seconds.
    pub since: Option<u64>,
    /// Unix seconds: only deletions before it.
    pub until: Option<u64>,
    /// What one subject request deleted.
    pub request_id: Option<String>,
    /// What one API request deleted: its audit row's `api_request_id`.
    pub api_request_id: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeletionResponse {
    pub id: i64,
    pub tenant_id: Option<String>,
    pub class: String,
    pub item: String,
    pub detail: Option<serde_json::Value>,
    pub cause: String,
    pub actor: String,
    /// The subject request the deletion served.
    pub request_id: Option<String>,
    /// The API request that made it, as its audit row names it.
    pub api_request_id: Option<String>,
    /// How the actor reached it: the phone a mailbox's owner called from.
    pub via: Option<String>,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub deleted_at: u64,
}

impl From<fcp_core::compliance::DeletionRecord> for DeletionResponse {
    fn from(r: fcp_core::compliance::DeletionRecord) -> Self {
        Self {
            id: r.id,
            tenant_id: r.tenant_id.map(|t| t.to_string()),
            class: r.class,
            item: r.item,
            detail: r.detail,
            cause: r.cause,
            actor: r.actor,
            request_id: r.request_id,
            api_request_id: r.api_request_id,
            via: r.via,
            deleted_at: unix_secs(r.deleted_at),
        }
    }
}

/// One deletion as a CSV line; the header is [`DELETION_CSV_HEADER`].
fn deletion_csv_row(r: &DeletionResponse) -> String {
    let opt = |v: &Option<String>| csv_field(v.as_deref().unwrap_or(""));
    let deleted = crate::time::format(std::time::Duration::from_secs(r.deleted_at));
    let detail = r.detail.as_ref().map(|d| d.to_string()).unwrap_or_default();
    csv_line(vec![
        r.id.to_string(),
        csv_field(&deleted),
        opt(&r.tenant_id),
        csv_field(&r.class),
        csv_field(&r.item),
        csv_field(&r.cause),
        csv_field(&r.actor),
        opt(&r.via),
        opt(&r.request_id),
        opt(&r.api_request_id),
        csv_field(&detail),
    ])
}

const DELETION_CSV_HEADER: &str =
    "id,deleted_at,tenant_id,class,item,cause,actor,via,request_id,api_request_id,detail\n";

impl DeletionLogQuery {
    /// The store's query: a tenant's rows, or every tenant's and the
    /// platform's for a platform administrator who picked none.
    fn to_query(
        &self,
        tenant: &TenantContext,
        identity: &Option<Extension<AuthIdentity>>,
        headers: &HeaderMap,
        limit: usize,
    ) -> fcp_core::compliance::DeletionQuery {
        let at = |s: u64| UNIX_EPOCH + std::time::Duration::from_secs(s);
        let platform_wide = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0))
            && !headers.contains_key(PLATFORM_TENANT_HEADER);
        fcp_core::compliance::DeletionQuery {
            tenant_id: (!platform_wide).then_some(tenant.0),
            class: self.class.clone(),
            since: self.since.map(at),
            until: self.until.map(at),
            request_id: self.request_id.clone(),
            api_request_id: self.api_request_id.clone(),
            limit,
            before: None,
        }
    }
}

/// `GET /compliance/deletions`: what was deleted, when, why and by whom
/// (`docs/COMPLIANCE_FOLLOWUPS.md` §7). Tenant admins see their tenant; the
/// platform token sees every tenant, platform upkeep included, unless it
/// selected one.
#[utoipa::path(
    get,
    path = "/api/v1/compliance/deletions",
    tag = "compliance",
    operation_id = "compliance_list_deletions",
    params(DeletionLogQuery, crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<DeletionResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_deletions(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Query(q): Query<DeletionLogQuery>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::AuditView)?;
    let store = require_store(&state)?;
    let mut query = q.to_query(&tenant, &identity, &headers, page.limit()? + 1);
    query.before = page.after()?.as_deref().map(parse_log_key).transpose()?;
    let records = store
        .list_deletions(&query)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let limit = page.limit()?;
    let next_key = (records.len() > limit).then(|| {
        let last = &records[limit - 1];
        log_key(last.deleted_at, last.id)
    });
    let out: Vec<DeletionResponse> = records.into_iter().take(limit).map(Into::into).collect();
    Ok(Json(crate::page::Page::new(out, next_key, None)))
}

/// `GET /compliance/deletions/export`: every deletion the listing's
/// filters match, newest first, as CSV or JSON Lines (#342). Who may see
/// what is the listing's; the export is itself audited.
#[utoipa::path(
    get,
    path = "/api/v1/compliance/deletions/export",
    tag = "compliance",
    operation_id = "compliance_export_deletions",
    params(DeletionLogQuery, AuditExportQuery),
    responses(
        (status = 200, description = "The deletions, streamed as CSV or JSON Lines", content_type = "text/csv", body = String),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn export_deletions(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Query(q): Query<DeletionLogQuery>,
    Query(export): Query<AuditExportQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::AuditView)?;
    let store = Arc::clone(require_store(&state)?);
    let jsonl = export.jsonl()?;
    let query = q.to_query(&tenant, &identity, &headers, LOG_EXPORT_PAGE);
    let first = store
        .list_deletions(&query)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(log_export(
        "deletion-log",
        jsonl,
        DELETION_CSV_HEADER,
        first,
        move |before| {
            let store = Arc::clone(&store);
            let mut query = query.clone();
            query.before = Some(before);
            async move { store.list_deletions(&query).await }
        },
        |r: &fcp_core::compliance::DeletionRecord| (r.deleted_at, r.id),
        move |r| {
            let r = DeletionResponse::from(r);
            if jsonl {
                json_line(&r)
            } else {
                deletion_csv_row(&r)
            }
        },
    ))
}

// ─── Legal holds (docs/COMPLIANCE_FOLLOWUPS.md §5) ───────────────────────

use fcp_core::legal_hold::{DataClass, HoldRelease, HoldScope, LegalHold, SubjectIdentifier};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ReleaseHoldRequest {
    pub reason: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListHoldsQuery {
    /// Released holds too.
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
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
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: u64,
    pub active: bool,
    pub released_by: Option<String>,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
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
#[utoipa::path(
    post,
    path = "/api/v1/compliance/holds",
    tag = "compliance",
    operation_id = "compliance_place_hold",
    request_body = PlaceHoldRequest,
    responses(
        (status = 201, description = "OK", body = HoldResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
#[utoipa::path(
    get,
    path = "/api/v1/compliance/holds",
    tag = "compliance",
    operation_id = "compliance_list_holds",
    params(ListHoldsQuery, crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<HoldResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_holds(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Query(q): Query<ListHoldsQuery>,
    Query(page): Query<crate::page::PageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    permit(&state, &identity, Permission::LegalHoldManage)?;
    let store = require_store(&state)?;
    let holds = store
        .list_holds(&tenant.0, q.all)
        .await
        .map_err(ApiError::storage)?;
    Ok(Json(crate::page::paginate(
        holds.into_iter().map(HoldResponse::from).collect(),
        &page,
        crate::page::Order::Descending,
        |r| crate::page::time_key(r.created_at, &r.id),
    )?))
}

/// `GET /compliance/holds/:id`: a hold, with what it holds now.
#[utoipa::path(
    get,
    path = "/api/v1/compliance/holds/{id}",
    tag = "compliance",
    operation_id = "compliance_get_hold",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = HoldResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
#[utoipa::path(
    post,
    path = "/api/v1/compliance/holds/{id}/release",
    tag = "compliance",
    operation_id = "compliance_release_hold",
    params(("id" = String, Path)),
    request_body = ReleaseHoldRequest,
    responses(
        (status = 200, description = "OK", body = HoldResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FileSubjectRequest {
    /// `export` or `erasure`.
    pub kind: String,
    pub subjects: Vec<SubjectIdentifier>,
    /// The tenant's own reference: a ticket, a letter.
    #[serde(default)]
    pub reference: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListSubjectRequestsQuery {
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BundleResponse {
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: u64,
    pub removed: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
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
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub received_at: u64,
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub due_at: u64,
    /// Open and past `due_at`.
    pub overdue: bool,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub started_at: Option<u64>,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub completed_at: Option<u64>,
    pub report: Option<fcp_core::subject_request::ExportReport>,
    pub bundle: Option<BundleResponse>,
    pub error: Option<String>,
    #[serde(default, with = "crate::time::epoch_secs_opt")]
    #[schema(value_type = Option<String>, format = DateTime)]
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
#[utoipa::path(
    post,
    path = "/api/v1/compliance/subject-requests",
    tag = "compliance",
    operation_id = "compliance_file_subject_request",
    request_body = FileSubjectRequest,
    responses(
        (status = 201, description = "OK", body = SubjectRequestResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
#[utoipa::path(
    get,
    path = "/api/v1/compliance/subject-requests",
    tag = "compliance",
    operation_id = "compliance_list_subject_requests",
    params(ListSubjectRequestsQuery, crate::page::PageQuery),
    responses(
        (status = 200, description = "OK", body = crate::page::Page<SubjectRequestResponse>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_subject_requests(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Query(q): Query<ListSubjectRequestsQuery>,
    Query(page): Query<crate::page::PageQuery>,
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
        .list_subject_requests(&tenant.0, status, crate::page::MAX_LIMIT)
        .await
        .map_err(ApiError::storage)?;
    let requests: Vec<SubjectRequestResponse> = requests.into_iter().map(Into::into).collect();
    Ok(Json(crate::page::paginate(
        requests,
        &page,
        crate::page::Order::Descending,
        |r| crate::page::time_key(r.received_at, &r.id),
    )?))
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
#[utoipa::path(
    get,
    path = "/api/v1/compliance/subject-requests/{id}",
    tag = "compliance",
    operation_id = "compliance_get_subject_request",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = SubjectRequestResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
#[utoipa::path(
    post,
    path = "/api/v1/compliance/subject-requests/{id}/confirm",
    tag = "compliance",
    operation_id = "compliance_confirm_subject_request",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "OK", body = SubjectRequestResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
#[utoipa::path(
    get,
    path = "/api/v1/compliance/subject-requests/{id}/bundle",
    tag = "compliance",
    operation_id = "compliance_download_subject_bundle",
    params(("id" = String, Path)),
    responses(
        (status = 200, description = "The bundle", content_type = "application/gzip", body = Vec<u8>),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
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
