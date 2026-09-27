//! API request and response models

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ==================== System Models ====================

/// Health check response
#[derive(Debug, Deserialize, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// System status response
#[derive(Debug, Deserialize, Serialize)]
pub struct StatusResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub users_count: usize,
    pub active_registrations_count: usize,
}

// ==================== Tenant Models ====================

/// Tenant response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TenantResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub max_users: Option<u32>,
    pub max_phone_numbers: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub billing_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub trial_expires_at: Option<u64>,
    pub metadata: Option<String>,
}

/// Create tenant request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub max_users: Option<u32>,
    pub max_phone_numbers: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub billing_id: Option<String>,
    pub trial_expires_at: Option<u64>,
    pub metadata: Option<String>,
}

/// Update tenant request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub max_users: Option<u32>,
    pub max_phone_numbers: Option<u32>,
    pub max_concurrent_calls: Option<u32>,
    pub billing_id: Option<String>,
    pub trial_expires_at: Option<u64>,
    pub metadata: Option<String>,
}

// ==================== User Models ====================

/// User response (excludes password hash)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub tenant_id: String,
    pub enabled: bool,
    pub role: String, // Role name (e.g., "super_admin", "telecom_admin")
    pub department_id: Option<String>, // For department-scoped admins
    #[serde(with = "system_time_as_string")]
    pub created_at: std::time::SystemTime,
    #[serde(with = "system_time_as_string")]
    pub updated_at: std::time::SystemTime,

    // Enterprise fields
    pub email: Option<String>,
    #[serde(with = "optional_system_time_as_string", default)]
    pub password_last_reset: Option<std::time::SystemTime>,
    pub phone_number: Option<String>,
    pub mobile_number: Option<String>,
    pub extension: Option<String>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    pub location: Option<String>,
    pub language: String,
    pub timezone: String,
    pub account_status: String,
    #[serde(with = "optional_system_time_as_string", default)]
    pub last_login: Option<std::time::SystemTime>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_phone: Option<String>,
}

/// Create user request
#[derive(Debug, Serialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>, // Role name (e.g., "super_admin", "telecom_admin")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department_id: Option<String>, // For department-scoped admins

    // Enterprise fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emergency_contact_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emergency_contact_phone: Option<String>,
}

/// Update user request
#[derive(Debug, Serialize)]
pub struct UpdateUserRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department_id: Option<String>,

    // Enterprise fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emergency_contact_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emergency_contact_phone: Option<String>,
}

/// Update password request
#[derive(Debug, Serialize)]
pub struct UpdatePasswordRequest {
    pub password: String,
}

// ==================== Registration Models ====================

/// Registration response
#[derive(Debug, Deserialize, Serialize)]
pub struct RegistrationResponse {
    pub aor: String,
    pub contact: String,
    #[serde(with = "system_time_as_string")]
    pub expires_at: std::time::SystemTime,
    pub call_id: String,
    pub cseq: u32,
    pub user_agent: Option<String>,
    pub q: Option<f32>,
    #[serde(with = "system_time_as_string")]
    pub registered_at: std::time::SystemTime,
    pub expires_in_seconds: u64,
}

/// Simple registration response from call-manager /registrations endpoint
#[derive(Debug, Deserialize, Serialize)]
pub struct SimpleRegistrationResponse {
    pub aor: String,
    pub contact: String,
    pub expires_at_epoch: u64,
    pub user_agent: Option<String>,
}

/// Wrapper for all registrations list response
#[derive(Debug, Deserialize, Serialize)]
pub struct AllRegistrationsResponse {
    pub count: usize,
    pub registrations: Vec<SimpleRegistrationResponse>,
}

impl SimpleRegistrationResponse {
    /// Convert to full RegistrationResponse with default values for missing fields
    pub fn to_registration_response(&self) -> RegistrationResponse {
        let expires_at =
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(self.expires_at_epoch);
        let now = std::time::SystemTime::now();
        let expires_in_seconds = expires_at
            .duration_since(now)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        RegistrationResponse {
            aor: self.aor.clone(),
            contact: self.contact.clone(),
            expires_at,
            call_id: String::new(), // Not available from simple endpoint
            cseq: 0,                // Not available from simple endpoint
            user_agent: self.user_agent.clone(),
            q: None,            // Not available from simple endpoint
            registered_at: now, // Approximate
            expires_in_seconds,
        }
    }
}

/// Aggregated registration response from events server (/registrations)
#[derive(Debug, Deserialize, Serialize)]
pub struct AggregatedRegistrationResponse {
    pub aor: String,
    pub contact: String,
    pub tenant_id: String,
    pub node_id: Option<String>,
    pub user_agent: Option<String>,
    pub expires_secs: u32,
    pub registered_at: u64,
    pub last_updated: u64,
}

impl AggregatedRegistrationResponse {
    /// Convert to RegistrationResponse for TUI compatibility
    pub fn to_registration_response(&self) -> RegistrationResponse {
        use std::time::{Duration, UNIX_EPOCH};

        let now = std::time::SystemTime::now();
        let registered_at = UNIX_EPOCH + Duration::from_secs(self.registered_at);
        let last_updated = UNIX_EPOCH + Duration::from_secs(self.last_updated);

        // Calculate expires_at from last_updated + expires_secs
        let expires_at = last_updated + Duration::from_secs(self.expires_secs as u64);
        let expires_in_seconds = expires_at
            .duration_since(now)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        RegistrationResponse {
            aor: self.aor.clone(),
            contact: self.contact.clone(),
            expires_at,
            call_id: String::new(), // Not available from aggregated endpoint
            cseq: 0,                // Not available from aggregated endpoint
            user_agent: self.user_agent.clone(),
            q: None, // Not available from aggregated endpoint
            registered_at,
            expires_in_seconds,
        }
    }
}

/// Response wrapper for aggregated registrations from events server
#[derive(Debug, Deserialize, Serialize)]
pub struct AggregatedRegistrationsResponse {
    pub registrations: Vec<AggregatedRegistrationResponse>,
    pub count: usize,
}

// ==================== Call Models ====================

/// Call information response (from /api/v1/calls)
#[derive(Debug, Deserialize, Serialize)]
pub struct CallResponse {
    pub id: String,
    pub state: String,
    pub from_uri: String,
    pub to_uri: String,
    pub from_display_name: Option<String>,
    pub to_display_name: Option<String>,
    pub created_at: String,
    pub established_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_seconds: Option<u64>,
    pub media_anchored: bool,
    pub failure_reason: Option<String>,
    /// Node ID (only present for aggregated calls from events server)
    #[serde(default)]
    pub node_id: Option<String>,
}

/// Aggregated call response from events server (/calls)
#[derive(Debug, Deserialize, Serialize)]
pub struct AggregatedCallResponse {
    pub call_id: String,
    pub node_id: Option<String>,
    pub tenant_id: String,
    pub from: String,
    pub to: String,
    pub direction: String,
    pub state: String,
    pub initiated_at: u64,
    pub ringing_at: Option<u64>,
    pub answered_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub codec: Option<String>,
    pub trunk_name: Option<String>,
    pub duration_ms: Option<u64>,
    pub hangup_cause: Option<String>,
    pub failure_reason: Option<String>,
    pub last_updated: u64,
}

impl AggregatedCallResponse {
    /// Convert to CallResponse for TUI compatibility
    pub fn to_call_response(&self) -> CallResponse {
        use std::time::{Duration, UNIX_EPOCH};

        // Convert Unix timestamp to ISO 8601 string
        let created_at = UNIX_EPOCH + Duration::from_secs(self.initiated_at);
        let created_at_str = chrono::DateTime::<chrono::Utc>::from(created_at)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let established_at_str = self.answered_at.map(|ts| {
            let t = UNIX_EPOCH + Duration::from_secs(ts);
            chrono::DateTime::<chrono::Utc>::from(t)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string()
        });

        let ended_at_str = self.ended_at.map(|ts| {
            let t = UNIX_EPOCH + Duration::from_secs(ts);
            chrono::DateTime::<chrono::Utc>::from(t)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string()
        });

        CallResponse {
            id: self.call_id.clone(),
            state: self.state.clone(),
            from_uri: self.from.clone(),
            to_uri: self.to.clone(),
            from_display_name: None,
            to_display_name: None,
            created_at: created_at_str,
            established_at: established_at_str,
            ended_at: ended_at_str,
            duration_seconds: self.duration_ms.map(|ms| ms / 1000),
            media_anchored: self.codec.is_some(),
            failure_reason: self.failure_reason.clone(),
            node_id: self.node_id.clone(),
        }
    }
}

/// Response wrapper for aggregated calls from events server
#[derive(Debug, Deserialize, Serialize)]
pub struct AggregatedCallsResponse {
    pub calls: Vec<AggregatedCallResponse>,
    pub count: usize,
}

/// Events server statistics response
#[derive(Debug, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct EventsStatsResponse {
    pub calls: CallStatsResponse,
    pub nodes: NodesStatsResponse,
}

/// Call statistics from events server
#[derive(Debug, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct CallStatsResponse {
    pub total: usize,
    pub initiated: usize,
    pub ringing: usize,
    pub answered: usize,
    pub ended: usize,
    pub failed: usize,
}

/// Node statistics from events server
#[derive(Debug, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct NodesStatsResponse {
    pub total: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub offline: usize,
}

/// Node information from events server
#[derive(Debug, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct NodeInfoResponse {
    pub node_id: String,
    pub node_type: String,
    pub first_seen: u64,
    pub last_seen: u64,
    pub active_calls: usize,
    pub status: String,
}

/// Hangup response
#[derive(Debug, Deserialize, Serialize)]
pub struct HangupResponse {
    pub call_id: String,
    pub status: String,
}

/// Recording response
#[derive(Debug, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct RecordingResponse {
    pub call_id: String,
    pub message: String,
}

/// Start recording request
#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct StartRecordingRequest {
    pub path: String,
}

// ==================== Debug Models ====================

/// Debug summary response
#[derive(Debug, Deserialize, Serialize)]
pub struct DebugSummaryResponse {
    pub active_dialogs: usize,
    pub active_subscriptions: usize,
    pub active_calls: usize,
    pub rate_limit_entries: usize,
}

/// Dialog information response
#[derive(Debug, Deserialize, Serialize)]
pub struct DialogResponse {
    pub call_id: String,
    pub local_tag: String,
    pub remote_tag: String,
    pub state: String,
    pub local_uri: String,
    pub remote_uri: String,
    pub remote_target: String,
    pub local_cseq: u32,
    pub remote_cseq: u32,
    pub route_set: Vec<String>,
}

/// Subscription information response
#[derive(Debug, Deserialize, Serialize)]
pub struct SubscriptionResponse {
    pub call_id: String,
    pub from_tag: String,
    pub to_tag: String,
    pub event: String,
    pub state: String,
    pub contact: String,
    pub expires_at: String,
}

/// Rate limit state response
#[derive(Debug, Deserialize, Serialize)]
pub struct RateLimitResponse {
    pub dialog_id: String,
    pub window_start: String,
    pub counts: HashMap<String, u32>,
}

/// Debug calls information (from /api/v1/debug/calls)
#[derive(Debug, Deserialize, Serialize)]
pub struct DebugCallResponse {
    pub call_id: String,
    pub media_session_id: Option<String>,
    pub a_leg_dialog_id: Option<String>,
    pub b_leg_dialog_id: Option<String>,
    pub state: String,
}

// ==================== Phone Number Models ====================

/// Phone number response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PhoneNumberResponse {
    pub id: String,
    pub e164: String,
    pub country_code: String,
    pub npa: String,
    pub nxx: String,
    pub line: String,
    pub state: String,
    pub tenant_id: String,
    pub e911_address: Option<E911AddressResponse>,
    pub e911_callback: Option<String>,
    pub cnam: Option<String>,
    pub spam_score: Option<f32>,
    pub carrier: Option<String>,
    pub lrn: Option<String>,
    pub rate_center: Option<String>,
    #[serde(with = "system_time_as_string")]
    pub acquired_at: std::time::SystemTime,
    #[serde(with = "optional_system_time_as_string", default)]
    pub activated_at: Option<std::time::SystemTime>,
    pub recording_enabled: bool,
    pub block_anonymous: bool,
    pub tags: Vec<String>,
    #[serde(with = "system_time_as_string")]
    pub created_at: std::time::SystemTime,
    #[serde(with = "system_time_as_string")]
    pub updated_at: std::time::SystemTime,
}

/// E911 address response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct E911AddressResponse {
    pub address1: String,
    pub address2: Option<String>,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
    pub validated: bool,
    #[serde(with = "optional_system_time_as_string", default)]
    pub validated_at: Option<std::time::SystemTime>,
}

// ==================== Device Models ====================

/// Device response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceResponse {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub mac_address: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub tenant_id: String,
    pub owner_user_id: Option<String>,
    pub sip_uri: Option<String>,
    pub registration_state: String,
    #[serde(with = "optional_system_time_as_string", default)]
    pub last_registration: Option<std::time::SystemTime>,
    pub last_ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub max_lines: u8,
    pub supports_video: bool,
    pub supports_blf: bool,
    pub codec_support: Vec<String>,
    pub location: Option<String>,
    pub e911_override: Option<E911AddressResponse>,
    pub provisioning_url: Option<String>,
    pub config_template_id: Option<String>,
    pub enabled: bool,
    #[serde(with = "system_time_as_string")]
    pub created_at: std::time::SystemTime,
    #[serde(with = "system_time_as_string")]
    pub updated_at: std::time::SystemTime,
}

// ==================== Line Binding Models ====================

/// Line binding response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LineBindingResponse {
    pub id: String,
    pub phone_number_id: String,
    pub owner_user_id: Option<String>,
    pub binding_type: String,
    pub devices: Vec<DeviceBindingResponse>,
    pub ring_strategy: String,
    pub tenant_id: String,
    pub outbound_caller_id: Option<String>,
    pub outbound_cnam: Option<String>,
    #[serde(with = "system_time_as_string")]
    pub created_at: std::time::SystemTime,
    #[serde(with = "system_time_as_string")]
    pub updated_at: std::time::SystemTime,
}

/// Device binding response (device attached to a line)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceBindingResponse {
    pub device_id: String,
    pub priority: u8,
    pub line_appearance: u8,
    pub shared_line: bool,
    pub enabled: bool,
}

// ==================== Serde Helpers ====================

/// Serialize/deserialize SystemTime as string for display
mod system_time_as_string {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::SystemTime;

    #[derive(Deserialize)]
    struct TimeStamp {
        secs_since_epoch: u64,
        nanos_since_epoch: u32,
    }

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?;
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ts = TimeStamp::deserialize(deserializer)?;
        Ok(SystemTime::UNIX_EPOCH
            + std::time::Duration::new(ts.secs_since_epoch, ts.nanos_since_epoch))
    }
}

/// Serialize/deserialize Option<SystemTime> as string for display
mod optional_system_time_as_string {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::SystemTime;

    #[derive(Deserialize)]
    struct TimeStamp {
        secs_since_epoch: u64,
        nanos_since_epoch: u32,
    }

    pub fn serialize<S>(time: &Option<SystemTime>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match time {
            Some(t) => {
                let duration = t
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(serde::ser::Error::custom)?;
                serializer.serialize_some(&duration.as_secs())
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SystemTime>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ts_opt = Option::<TimeStamp>::deserialize(deserializer)?;
        Ok(ts_opt.map(|ts| {
            SystemTime::UNIX_EPOCH
                + std::time::Duration::new(ts.secs_since_epoch, ts.nanos_since_epoch)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregated_calls_deserialization() {
        let json = r#"{"calls":[{"answered_at":1770341243,"call_id":"call-0bb0d31e-2449-4466-9af1-959d5fe0e37b","codec":"TELEPHONE-EVENT","direction":"inbound","from":"sip:alice@192.168.1.225;transport=TCP","initiated_at":1770341240,"last_updated":1770341243,"node_id":"cm-docker-01","ringing_at":1770341240,"state":"answered","tenant_id":"tenant-00000000-0000-0000-0000-000000000000","to":"<sip:bob@192.168.1.225>"}],"count":1}"#;

        let result: Result<AggregatedCallsResponse, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "Failed to deserialize: {:?}", result.err());

        let response = result.unwrap();
        assert_eq!(response.count, 1);
        assert_eq!(response.calls.len(), 1);
        assert_eq!(
            response.calls[0].call_id,
            "call-0bb0d31e-2449-4466-9af1-959d5fe0e37b"
        );
        assert_eq!(response.calls[0].state, "answered");
        assert_eq!(response.calls[0].node_id, Some("cm-docker-01".to_string()));
    }

    #[test]
    fn test_aggregated_call_to_call_response() {
        let aggregated = AggregatedCallResponse {
            call_id: "call-123".to_string(),
            node_id: Some("node-1".to_string()),
            tenant_id: "tenant-abc".to_string(),
            from: "sip:alice@example.com".to_string(),
            to: "sip:bob@example.com".to_string(),
            direction: "inbound".to_string(),
            state: "answered".to_string(),
            initiated_at: 1770341240,
            ringing_at: Some(1770341240),
            answered_at: Some(1770341243),
            ended_at: None,
            codec: Some("PCMU".to_string()),
            trunk_name: None,
            duration_ms: Some(5000),
            hangup_cause: None,
            failure_reason: None,
            last_updated: 1770341243,
        };

        let call = aggregated.to_call_response();
        assert_eq!(call.id, "call-123");
        assert_eq!(call.state, "answered");
        assert_eq!(call.node_id, Some("node-1".to_string()));
        assert_eq!(call.duration_seconds, Some(5)); // 5000ms / 1000 = 5s
        assert!(call.media_anchored); // codec is Some
    }
}
