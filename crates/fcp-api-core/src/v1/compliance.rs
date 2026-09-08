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
    run_retention, AuditEvent, AuditQuery, CompliancePolicy, ComplianceStore, RecordingConsentMode,
    RetentionReport,
};
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
    pub recording_consent: String,
    pub recording_consent_prompt_id: Option<String>,
    /// False until the tenant has saved a policy (defaults shown).
    pub configured: bool,
    pub updated_at: u64,
}

impl PolicyResponse {
    fn from_policy(p: CompliancePolicy, configured: bool) -> Self {
        Self {
            tenant_id: p.tenant_id.to_string(),
            cdr_retention_days: p.cdr_retention_days,
            sip_trace_retention_days: p.sip_trace_retention_days,
            flow_trace_retention_days: p.flow_trace_retention_days,
            recording_retention_days: p.recording_retention_days,
            audit_retention_days: p.audit_retention_days,
            recording_consent: p.recording_consent.to_string(),
            recording_consent_prompt_id: p.recording_consent_prompt_id,
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
    pub recording_consent: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub recording_consent_prompt_id: Option<Option<String>>,
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
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    let stored = store
        .get_compliance_policy(&tenant.0)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let configured = stored.is_some();
    let policy = stored.unwrap_or_else(|| CompliancePolicy::defaults(tenant.0));
    Ok(Json(PolicyResponse::from_policy(policy, configured)))
}

pub async fn update_policy(
    State(state): State<ApiState>,
    tenant: TenantContext,
    Json(req): Json<UpdatePolicyRequest>,
) -> Result<impl IntoResponse, ApiError> {
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
    Ok(Json(PolicyResponse::from_policy(policy, true)))
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
            audit_events_deleted: r.audit_events_deleted,
            errors: r.errors,
        }
    }
}

/// Run the tenant's retention policy now and record the outcome.
pub async fn run_retention_now(
    State(state): State<ApiState>,
    tenant: TenantContext,
) -> Result<impl IntoResponse, ApiError> {
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
    let report = run_retention(storage, &policy, state.call_recordings_dir.as_deref()).await;
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
    Query(q): Query<LimitQuery>,
) -> Result<impl IntoResponse, ApiError> {
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
