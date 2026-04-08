//! CDR (Call Detail Record) API endpoints.

use crate::{
    auth::{can_access_all_tenants, AuthIdentity, TenantContext},
    error::ApiError,
    state::ApiState,
};
use axum::{
    extract::{Extension, Path, Query, State},
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
        let to_timestamp =
            |st: SystemTime| -> u64 { st.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() };

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
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
    Query(params): Query<ListCdrsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let storage = require_storage!(state);
    let allow_all_tenants = can_access_all_tenants(identity.as_ref().map(|ext| &ext.0));

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
        tenant_id: Some(tenant.0.clone()),
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
    if allow_all_tenants {
        query.tenant_id = None;
    }
    if allow_all_tenants {
        if let Some(ref tenant_id_str) = params.tenant_id {
            query.tenant_id = Some(
                tenant_id_str
                    .parse::<fcp_core::TenantId>()
                    .map_err(|e| ApiError::BadRequest(format!("Invalid tenant_id: {}", e)))?,
            );
        }
    } else if let Some(ref tenant_id_str) = params.tenant_id {
        let requested_tenant_id = tenant_id_str
            .parse::<fcp_core::TenantId>()
            .map_err(|e| ApiError::BadRequest(format!("Invalid tenant_id: {}", e)))?;
        if requested_tenant_id != tenant.0 {
            return Err(ApiError::NotFound("CDRs not found".to_string()));
        }
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
        let timestamp = start_from_str.parse::<u64>().map_err(|e| {
            ApiError::BadRequest(format!(
                "Invalid start_time_from (expected Unix timestamp): {}",
                e
            ))
        })?;
        query.start_time_from = Some(UNIX_EPOCH + std::time::Duration::from_secs(timestamp));
    }

    if let Some(ref start_to_str) = params.start_time_to {
        let timestamp = start_to_str.parse::<u64>().map_err(|e| {
            ApiError::BadRequest(format!(
                "Invalid start_time_to (expected Unix timestamp): {}",
                e
            ))
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
    tenant: TenantContext,
    identity: Option<Extension<AuthIdentity>>,
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

    if !can_access_all_tenants(identity.as_ref().map(|ext| &ext.0)) && cdr.tenant_id != tenant.0 {
        return Err(ApiError::NotFound(format!(
            "CDR not found for call_id: {}",
            call_id
        )));
    }

    Ok(Json(CdrResponse::from(cdr)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::storage::{CallDetailRecord, CallDisposition, HangupCause, HangupParty};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_test_cdr() -> CallDetailRecord {
        CallDetailRecord {
            call_id: fcp_core::CallId::new(),
            tenant_id: fcp_core::TenantId::nil(),
            sip_call_id: "test-call-id@example.com".to_string(),
            from_uri: "sip:alice@example.com".to_string(),
            from_display_name: Some("Alice".to_string()),
            to_uri: "sip:bob@example.com".to_string(),
            to_display_name: Some("Bob".to_string()),
            start_time: SystemTime::now(),
            answer_time: Some(SystemTime::now()),
            end_time: Some(SystemTime::now()),
            setup_duration_ms: Some(2000),
            call_duration_ms: Some(30000),
            total_duration_ms: Some(32000),
            disposition: CallDisposition::Answered,
            hangup_cause: HangupCause::NormalClearing,
            hangup_party: Some(HangupParty::Caller),
            sip_status_code: Some(200),
            sip_reason: Some("OK".to_string()),
            route_id: None,
            gateway_id: None,
            media_anchored: true,
            codec_a_leg: Some("PCMU".to_string()),
            codec_b_leg: Some("PCMU".to_string()),
            rtp_packets_sent: None,
            rtp_packets_received: None,
            rtp_bytes_sent: None,
            rtp_bytes_received: None,
            recording_url: None,
            cost: None,
            currency: None,
            custom_data: None,
        }
    }

    #[test]
    fn test_cdr_response_conversion() {
        let cdr = create_test_cdr();
        let response = CdrResponse::from(cdr.clone());

        assert_eq!(response.call_id, cdr.call_id.to_string());
        assert_eq!(response.tenant_id, cdr.tenant_id.to_string());
        assert_eq!(response.sip_call_id, cdr.sip_call_id);
        assert_eq!(response.from_uri, cdr.from_uri);
        assert_eq!(response.from_display_name, cdr.from_display_name);
        assert_eq!(response.to_uri, cdr.to_uri);
        assert_eq!(response.to_display_name, cdr.to_display_name);
        assert_eq!(response.setup_duration_ms, Some(2000));
        assert_eq!(response.call_duration_ms, Some(30000));
        assert_eq!(response.total_duration_ms, Some(32000));
        assert_eq!(response.disposition, "answered");
        assert_eq!(response.hangup_cause, "normal_clearing");
        assert_eq!(response.hangup_party, Some("caller".to_string()));
        assert_eq!(response.sip_status_code, Some(200));
        assert_eq!(response.sip_reason, Some("OK".to_string()));
        assert_eq!(response.media_anchored, true);
        assert_eq!(response.codec_a_leg, Some("PCMU".to_string()));
        assert_eq!(response.codec_b_leg, Some("PCMU".to_string()));
    }

    #[test]
    fn test_cdr_response_timestamps() {
        let now = SystemTime::now();
        let now_secs = now.duration_since(UNIX_EPOCH).unwrap().as_secs();

        let cdr = CallDetailRecord {
            start_time: now,
            answer_time: Some(now),
            end_time: Some(now),
            ..create_test_cdr()
        };

        let response = CdrResponse::from(cdr);

        assert_eq!(response.start_time, now_secs);
        assert_eq!(response.answer_time, Some(now_secs));
        assert_eq!(response.end_time, Some(now_secs));
    }

    #[test]
    fn test_cdr_response_all_dispositions() {
        // Test all disposition variants serialize correctly
        let dispositions = vec![
            (CallDisposition::Answered, "answered"),
            (CallDisposition::Busy, "busy"),
            (CallDisposition::NoAnswer, "no_answer"),
            (CallDisposition::Failed, "failed"),
            (CallDisposition::Cancelled, "cancelled"),
        ];

        for (disposition, expected) in dispositions {
            let cdr = CallDetailRecord {
                disposition,
                ..create_test_cdr()
            };
            let response = CdrResponse::from(cdr);
            assert_eq!(response.disposition, expected);
        }
    }

    #[test]
    fn test_cdr_response_all_hangup_causes() {
        // Test all common hangup cause variants serialize correctly
        let causes = vec![
            (HangupCause::NormalClearing, "normal_clearing"),
            (HangupCause::UserBusy, "user_busy"),
            (HangupCause::NoAnswer, "no_answer"),
            (HangupCause::CallRejected, "call_rejected"),
            (HangupCause::InternalError, "internal_error"),
        ];

        for (cause, expected) in causes {
            let cdr = CallDetailRecord {
                hangup_cause: cause,
                ..create_test_cdr()
            };
            let response = CdrResponse::from(cdr);
            assert_eq!(response.hangup_cause, expected);
        }
    }

    #[test]
    fn test_cdr_response_hangup_parties() {
        // Test all hangup party variants serialize correctly
        let parties = vec![
            (Some(HangupParty::Caller), Some("caller".to_string())),
            (Some(HangupParty::Callee), Some("callee".to_string())),
            (Some(HangupParty::System), Some("system".to_string())),
            (None, None),
        ];

        for (party, expected) in parties {
            let cdr = CallDetailRecord {
                hangup_party: party,
                ..create_test_cdr()
            };
            let response = CdrResponse::from(cdr);
            assert_eq!(response.hangup_party, expected);
        }
    }

    // TODO: Implement extract_sip_status_code function and enable this test
    // #[test]
    // fn test_extract_sip_status_code() {
    //     use super::extract_sip_status_code;
    //
    //     assert_eq!(extract_sip_status_code("486 Busy Here"), Some(486));
    //     assert_eq!(extract_sip_status_code("200 OK"), Some(200));
    //     assert_eq!(extract_sip_status_code("404 Not Found"), Some(404));
    //     assert_eq!(extract_sip_status_code("503 Service Unavailable"), Some(503));
    //
    //     // Out of range
    //     assert_eq!(extract_sip_status_code("99 Invalid"), None);
    //     assert_eq!(extract_sip_status_code("700 Out of Range"), None);
    //
    //     // Not a number
    //     assert_eq!(extract_sip_status_code("ABC Invalid"), None);
    //     assert_eq!(extract_sip_status_code("Unknown error"), None);
    //
    //     // Empty
    //     assert_eq!(extract_sip_status_code(""), None);
    // }

    #[test]
    fn test_default_limit() {
        assert_eq!(default_limit(), 100);
    }

    // TODO: Add serde_urlencoded dependency and enable these tests
    // #[test]
    // fn test_list_cdrs_query_defaults() {
    //     let query_str = "";
    //     let query: Result<ListCdrsQuery, _> = serde_urlencoded::from_str(query_str);
    //     assert!(query.is_ok());
    //     let query = query.unwrap();
    //     assert_eq!(query.limit, 100);
    //     assert_eq!(query.offset, 0);
    //     assert!(query.tenant_id.is_none());
    //     assert!(query.call_id.is_none());
    //     assert!(query.from_uri.is_none());
    //     assert!(query.to_uri.is_none());
    //     assert!(query.disposition.is_none());
    // }
    //
    // #[test]
    // fn test_list_cdrs_query_with_filters() {
    //     let query_str = "tenant_id=tenant-123&from_uri=sip:alice@example.com&limit=50&offset=10";
    //     let query: Result<ListCdrsQuery, _> = serde_urlencoded::from_str(query_str);
    //     assert!(query.is_ok());
    //     let query = query.unwrap();
    //     assert_eq!(query.limit, 50);
    //     assert_eq!(query.offset, 10);
    //     assert_eq!(query.tenant_id, Some("tenant-123".to_string()));
    //     assert_eq!(query.from_uri, Some("sip:alice@example.com".to_string()));
    // }
}
