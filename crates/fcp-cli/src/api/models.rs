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

// ==================== User Models ====================

/// User response (excludes password hash)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserResponse {
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
}

/// Hangup response
#[derive(Debug, Deserialize, Serialize)]
pub struct HangupResponse {
    pub call_id: String,
    pub status: String,
}

/// Recording response
#[derive(Debug, Deserialize, Serialize)]
pub struct RecordingResponse {
    pub call_id: String,
    pub message: String,
}

/// Start recording request
#[derive(Debug, Serialize)]
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
        Ok(SystemTime::UNIX_EPOCH + std::time::Duration::new(ts.secs_since_epoch, ts.nanos_since_epoch))
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
        Ok(ts_opt.map(|ts| SystemTime::UNIX_EPOCH + std::time::Duration::new(ts.secs_since_epoch, ts.nanos_since_epoch)))
    }
}
