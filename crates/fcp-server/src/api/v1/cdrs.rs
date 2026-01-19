//! CDR (Call Detail Record) API endpoints.

use crate::api::{error::ApiError, state::ApiState};
use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use fcp_core::storage::{CdrQuery, CdrStore};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, warn};

/// Helper macro to get storage or return error.
macro_rules! require_storage {
    ($state:expr) => {
        $state.storage.as_ref().ok_or_else(|| {
            ApiError::NotFound("CDR access requires storage to be enabled".to_string())
        })?
    };
}

/// CDR response model.
#[derive(Debug, Serialize)]
pub struct CdrResponse {
    pub call_id: String,
    pub tenant_id: String,
    pub sip_call_id: String,
    pub from_uri: String,
    pub from_display_name: Option<String>,
    pub to_uri: String,
    pub to_display_name: Option<String>,
    pub start_time: u64, // Unix timestamp (seconds since epoch)
    pub answer_time: Option<u64>,
    pub end_time: Option<u64>,
    pub setup_duration_ms: Option<u64>,
    pub call_duration_ms: Option<u64>,
    pub total_duration_ms: Option<u64>,
    pub disposition: String,
    pub hangup_cause: String,
    pub hangup_party: Option<String>,
    pub sip_status_code: Option<u16>,
    pub sip_reason: Option<String>,
    pub media_anchored: bool,
    pub codec_a_leg: Option<String>,
    pub codec_b_leg: Option<String>,
    pub recording_url: Option<String>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
}

impl From<fcp_core::storage::CallDetailRecord> for CdrResponse {
    fn from(cdr: fcp_core::storage::CallDetailRecord) -> Self {
        // Convert SystemTime to Unix timestamp (seconds since epoch)
        let to_timestamp = |st: SystemTime| -> u64 {
            st.duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        };

        // Serialize disposition and hangup_cause using serde
        let disposition = serde_json::to_string(&cdr.disposition)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let hangup_cause = serde_json::to_string(&cdr.hangup_cause)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let hangup_party = cdr.hangup_party.map(|hp| {
            serde_json::to_string(&hp)
                .map(|s| s.trim_matches('"').to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        });

        Self {
            call_id: cdr.call_id.to_string(),
            tenant_id: cdr.tenant_id.to_string(),
            sip_call_id: cdr.sip_call_id,
            from_uri: cdr.from_uri,
            from_display_name: cdr.from_display_name,
            to_uri: cdr.to_uri,
            to_display_name: cdr.to_display_name,
            start_time: to_timestamp(cdr.start_time),
            answer_time: cdr.answer_time.map(to_timestamp),
            end_time: cdr.end_time.map(to_timestamp),
            setup_duration_ms: cdr.setup_duration_ms,
            call_duration_ms: cdr.call_duration_ms,
            total_duration_ms: cdr.total_duration_ms,
            disposition,
            hangup_cause,
            hangup_party,
            sip_status_code: cdr.sip_status_code,
            sip_reason: cdr.sip_reason,
            media_anchored: cdr.media_anchored,
            codec_a_leg: cdr.codec_a_leg,
            codec_b_leg: cdr.codec_b_leg,
            recording_url: cdr.recording_url,
            cost: cdr.cost,
            currency: cdr.currency,
        }
    }
}

/// Query parameters for listing CDRs.
#[derive(Debug, Deserialize)]
pub struct ListCdrsQuery {
    /// Filter by tenant ID
    pub tenant_id: Option<String>,

    /// Filter by call ID
    pub call_id: Option<String>,

    /// Filter by from URI (caller)
    pub from_uri: Option<String>,

    /// Filter by to URI (callee)
    pub to_uri: Option<String>,

    /// Filter by disposition (answered, busy, no_answer, failed, cancelled)
    pub disposition: Option<String>,

    /// Filter by start time (Unix timestamp in seconds)
    pub start_time_from: Option<String>,

    /// Filter by start time (Unix timestamp in seconds)
    pub start_time_to: Option<String>,

    /// Maximum number of results (default: 100, max: 1000)
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Offset for pagination (default: 0)
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    100
}

/// Paginated list of CDRs.
#[derive(Debug, Serialize)]
pub struct ListCdrsResponse {
    pub cdrs: Vec<CdrResponse>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

/// List CDRs with filters and pagination.
///
/// GET /api/v1/cdrs
pub async fn list_cdrs(
    State(state): State<ApiState>,
    Query(params): Query<ListCdrsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    // Validate and cap limit
    let limit = if params.limit > 1000 {
        warn!(
            requested_limit = params.limit,
            "Requested limit exceeds maximum, capping at 1000"
        );
        1000
    } else if params.limit == 0 {
        default_limit()
    } else {
        params.limit
    };

    // Build CdrQuery from request parameters
    let mut query = CdrQuery {
        tenant_id: None,
        call_id: None,
        from_uri: params.from_uri.clone(),
        to_uri: params.to_uri.clone(),
        disposition: None,
        start_time_from: None,
        start_time_to: None,
        limit: Some(limit),
        offset: Some(params.offset),
    };

    // Parse optional tenant_id
    if let Some(ref tenant_id_str) = params.tenant_id {
        query.tenant_id = Some(
            tenant_id_str
                .parse::<fcp_core::TenantId>()
                .map_err(|e| ApiError::BadRequest(format!("Invalid tenant_id: {}", e)))?,
        );
    }

    // Parse optional call_id
    if let Some(ref call_id_str) = params.call_id {
        query.call_id = Some(
            call_id_str
                .parse::<fcp_core::CallId>()
                .map_err(|e| ApiError::BadRequest(format!("Invalid call_id: {}", e)))?,
        );
    }

    // Parse optional disposition
    if let Some(ref disp_str) = params.disposition {
        use fcp_core::storage::CallDisposition;

        // Try deserializing with serde (handles snake_case)
        let disposition = serde_json::from_str::<CallDisposition>(&format!("\"{}\"", disp_str))
            .map_err(|e| {
                ApiError::BadRequest(format!(
                    "Invalid disposition '{}': {}. Valid values: answered, busy, no_answer, failed, cancelled",
                    disp_str, e
                ))
            })?;
        query.disposition = Some(disposition);
    }

    // Parse optional timestamps (Unix timestamps in seconds)
    if let Some(ref start_from_str) = params.start_time_from {
        let timestamp = start_from_str
            .parse::<u64>()
            .map_err(|e| {
                ApiError::BadRequest(format!("Invalid start_time_from (expected Unix timestamp): {}", e))
            })?;
        query.start_time_from = Some(UNIX_EPOCH + std::time::Duration::from_secs(timestamp));
    }

    if let Some(ref start_to_str) = params.start_time_to {
        let timestamp = start_to_str
            .parse::<u64>()
            .map_err(|e| {
                ApiError::BadRequest(format!("Invalid start_time_to (expected Unix timestamp): {}", e))
            })?;
        query.start_time_to = Some(UNIX_EPOCH + std::time::Duration::from_secs(timestamp));
    }

    debug!(
        tenant_id = ?query.tenant_id,
        call_id = ?query.call_id,
        from_uri = ?query.from_uri,
        to_uri = ?query.to_uri,
        disposition = ?query.disposition,
        limit = query.limit,
        offset = query.offset,
        "Querying CDRs"
    );

    // Execute queries in parallel
    let (cdrs, total) = tokio::try_join!(
        storage.query_cdrs(query.clone()),
        storage.count_cdrs(query.clone())
    )
    .map_err(|e| {
        warn!(error = %e, "Failed to query CDRs");
        ApiError::Internal("Failed to query CDRs".to_string())
    })?;

    let cdr_responses: Vec<CdrResponse> = cdrs.into_iter().map(CdrResponse::from).collect();

    Ok(Json(ListCdrsResponse {
        cdrs: cdr_responses,
        total,
        limit,
        offset: params.offset,
    }))
}

/// Get a specific CDR by call ID.
///
/// GET /api/v1/cdrs/:call_id
pub async fn get_cdr(
    State(state): State<ApiState>,
    Path(call_id_str): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);

    // Parse call_id
    let call_id = call_id_str
        .parse::<fcp_core::CallId>()
        .map_err(|e| ApiError::BadRequest(format!("Invalid call_id: {}", e)))?;

    debug!(call_id = %call_id, "Retrieving CDR");

    // Retrieve CDR from storage
    let cdr = storage
        .get_cdr(&call_id)
        .await
        .map_err(|e| {
            warn!(call_id = %call_id, error = %e, "Failed to retrieve CDR");
            ApiError::Internal("Failed to retrieve CDR".to_string())
        })?
        .ok_or_else(|| ApiError::NotFound(format!("CDR not found for call_id: {}", call_id)))?;

    Ok(Json(CdrResponse::from(cdr)))
}
