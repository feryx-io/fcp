//! SIP trace API endpoints.
//!
//! Provides access to captured SIP messages for call debugging and
//! ladder diagram visualization in the admin UI.

use crate::{
    auth::{can_access_all_tenants, AuthIdentity, TenantContext, PLATFORM_TENANT_HEADER},
    error::ApiError,
    state::ApiState,
};
use axum::{
    extract::{Extension, Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use fcp_core::CdrStore;
use serde::{Deserialize, Serialize};
use std::time::UNIX_EPOCH;
use tracing::{debug, warn};

/// Response model for a single SIP trace message.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SipTraceResponse {
    pub id: i64,
    #[serde(with = "crate::time::epoch_millis")]
    #[schema(value_type = String, format = DateTime)]
    pub timestamp: u64, // Unix timestamp in milliseconds
    pub direction: String,
    pub method_or_status: String,
    pub cseq: String,
    pub from_uri: String,
    pub to_uri: String,
    pub source_addr: String,
    pub dest_addr: String,
    pub transport: String,
    pub raw_message: String,
    pub node_id: Option<String>,
}

impl From<fcp_core::SipTraceMessage> for SipTraceResponse {
    fn from(msg: fcp_core::SipTraceMessage) -> Self {
        let timestamp = msg
            .timestamp
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            id: msg.id,
            timestamp,
            direction: msg.direction,
            method_or_status: msg.method_or_status,
            cseq: msg.cseq,
            from_uri: msg.from_uri,
            to_uri: msg.to_uri,
            source_addr: msg.source_addr,
            dest_addr: msg.dest_addr,
            transport: msg.transport,
            raw_message: msg.raw_message,
            node_id: msg.node_id,
        }
    }
}

/// Response for SIP trace messages for a call.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SipTraceListResponse {
    pub call_id: String,
    pub messages: Vec<SipTraceResponse>,
}

/// Query parameters for SIP trace lookup.
#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SipTraceQuery {
    /// Alternative lookup by SIP Call-ID header instead of FCP call_id
    pub sip_call_id: Option<String>,
}

/// Get all SIP trace messages for a call, ordered by timestamp.
///
/// GET /api/v1/calls/:call_id/sip-messages
///
/// Query parameters:
/// - sip_call_id: Alternative lookup by SIP Call-ID header value
#[utoipa::path(
    get,
    path = "/api/v1/calls/{call_id}/sip-messages",
    tag = "sip_trace",
    operation_id = "sip_trace_get_sip_messages",
    params(("call_id" = String, Path), SipTraceQuery),
    responses(
        (status = 200, description = "OK", body = SipTraceListResponse),
        (status = "default", description = "An error", body = crate::error::ErrorResponse),
    ),
)]
pub async fn get_sip_messages(
    State(state): State<ApiState>,
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    headers: HeaderMap,
    Path(call_id): Path<String>,
    Query(params): Query<SipTraceQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = state.storage.as_ref().ok_or_else(|| {
        ApiError::NotFound("SIP trace access requires storage to be enabled".to_string())
    })?;

    let sip_trace_store = storage
        .sip_trace_store()
        .ok_or_else(|| ApiError::NotFound("SIP trace store is not configured".to_string()))?;

    // Normalize call_id: strip "call-" prefix if present (CDR uses fcp_core::CallId
    // format "call-{uuid}" but traces store fcp_b2bua_core::CallId format "{uuid}")
    let normalized_call_id = call_id
        .strip_prefix("call-")
        .unwrap_or(&call_id)
        .to_string();

    debug!(call_id = %normalized_call_id, sip_call_id = ?params.sip_call_id, "Retrieving SIP trace messages");

    // Platform admins see every tenant unless they selected one with the
    // override header (then `tenant` already carries it).
    let allow_all_tenants = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0))
        && !headers.contains_key(PLATFORM_TENANT_HEADER);
    let tenant_id = tenant.0.to_string();
    // Traces are written with the canonical `call-<uuid>` form; accept the
    // bare uuid from callers too.
    let call_id_with_prefix = if call_id.starts_with("call-") {
        call_id.clone()
    } else {
        format!("call-{}", call_id)
    };

    let messages = if let Some(ref sip_call_id) = params.sip_call_id {
        // Look up by SIP Call-ID header
        sip_trace_store
            .get_sip_traces_by_sip_call_id(sip_call_id)
            .await
            .map_err(|e| {
                warn!(sip_call_id = %sip_call_id, error = %e, "Failed to retrieve SIP traces");
                ApiError::Internal("Failed to retrieve SIP trace messages".to_string())
            })?
    } else {
        // Look up by FCP call_id. The B2BUA writes the canonical
        // `call-<uuid>` form (CallId's Display), so query that first; the
        // bare-uuid form covers rows written before the id was canonicalised.
        let mut traces = sip_trace_store
            .get_sip_traces(&call_id_with_prefix)
            .await
            .map_err(|e| {
                warn!(call_id = %normalized_call_id, error = %e, "Failed to retrieve SIP traces");
                ApiError::Internal("Failed to retrieve SIP trace messages".to_string())
            })?;
        if traces.is_empty() {
            traces = sip_trace_store
                .get_sip_traces(&normalized_call_id)
                .await
                .unwrap_or_default();
        }

        // Fallback: if no traces found by call_id, look up the CDR to get sip_call_id
        // and try querying by that. This handles the initial INVITE and messages that
        // were stored before the FCP CallId mapping was established.
        if traces.is_empty() {
            if let Ok(call_id_parsed) = call_id_with_prefix.parse::<fcp_core::CallId>() {
                if let Some(cdr) = storage.get_cdr(&call_id_parsed).await.unwrap_or(None) {
                    debug!(
                        call_id = %normalized_call_id,
                        sip_call_id = %cdr.sip_call_id,
                        "No traces by call_id, falling back to CDR sip_call_id lookup"
                    );
                    traces = sip_trace_store
                        .get_sip_traces_by_sip_call_id(&cdr.sip_call_id)
                        .await
                        .unwrap_or_default();
                }
            }
        }

        traces
    };

    let messages: Vec<_> = messages
        .into_iter()
        .filter(|msg| allow_all_tenants || msg.tenant_id == tenant_id)
        .collect();

    let responses: Vec<SipTraceResponse> =
        messages.into_iter().map(SipTraceResponse::from).collect();

    Ok(Json(SipTraceListResponse {
        call_id: normalized_call_id,
        messages: responses,
    }))
}
