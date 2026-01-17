//! Storage abstraction layer for FCP.
//!
//! This module provides traits and implementations for storing:
//! - User credentials (for authentication)
//! - SIP registrations (location bindings)
//! - Call detail records (CDRs)
//!
//! ## Storage Backends
//!
//! FCP supports multiple storage backends:
//! - **SQLite** (default): Production-ready, ACID-compliant storage for single-node and small deployments
//! - **PostgreSQL**: For larger multi-node deployments requiring advanced features
//! - **MySQL**: For existing MySQL infrastructure
//! - **MongoDB**: For large-scale NoSQL deployments
//!
//! The default backend is SQLite, which provides excellent performance for most use cases
//! while requiring no external database server. For development and testing only, a
//! FileBackedStorage implementation is also available that uses in-memory storage with JSON file persistence.

#[cfg(feature = "sqlite")]
pub mod sqlite;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Password hashing utilities using bcrypt.
pub mod password {
    use anyhow::{bail, Result};

    const BCRYPT_COST: u32 = 12; // Higher = more secure but slower
    const MIN_PASSWORD_LENGTH: usize = 8;
    const MAX_PASSWORD_LENGTH: usize = 128;

    /// Validate password strength requirements.
    ///
    /// Requirements:
    /// - Minimum 8 characters
    /// - Maximum 128 characters
    /// - At least one uppercase letter
    /// - At least one lowercase letter
    /// - At least one digit
    /// - At least one special character
    pub fn validate_password_strength(password: &str) -> Result<()> {
        if password.len() < MIN_PASSWORD_LENGTH {
            bail!(
                "Password must be at least {} characters long",
                MIN_PASSWORD_LENGTH
            );
        }

        if password.len() > MAX_PASSWORD_LENGTH {
            bail!(
                "Password must not exceed {} characters",
                MAX_PASSWORD_LENGTH
            );
        }

        let has_uppercase = password.chars().any(|c| c.is_uppercase());
        let has_lowercase = password.chars().any(|c| c.is_lowercase());
        let has_digit = password.chars().any(|c| c.is_ascii_digit());
        let has_special = password.chars().any(|c| !c.is_alphanumeric());

        if !has_uppercase {
            bail!("Password must contain at least one uppercase letter");
        }

        if !has_lowercase {
            bail!("Password must contain at least one lowercase letter");
        }

        if !has_digit {
            bail!("Password must contain at least one digit");
        }

        if !has_special {
            bail!("Password must contain at least one special character");
        }

        Ok(())
    }

    /// Hash a plaintext password using bcrypt.
    ///
    /// This function validates password strength before hashing.
    pub fn hash_password(password: &str) -> Result<String> {
        validate_password_strength(password)?;
        bcrypt::hash(password, BCRYPT_COST)
            .map_err(|e| anyhow::anyhow!("Failed to hash password: {}", e))
    }

    /// Verify a plaintext password against a bcrypt hash.
    pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
        bcrypt::verify(password, hash)
            .map_err(|e| anyhow::anyhow!("Failed to verify password: {}", e))
    }

    /// PIN validation constants
    const MIN_PIN_LENGTH: usize = 4;
    const MAX_PIN_LENGTH: usize = 8;

    /// Validate PIN requirements.
    ///
    /// Requirements:
    /// - Minimum 4 digits
    /// - Maximum 8 digits
    /// - Only numeric characters (0-9)
    pub fn validate_pin(pin: &str) -> Result<()> {
        if pin.len() < MIN_PIN_LENGTH {
            bail!(
                "PIN must be at least {} digits long",
                MIN_PIN_LENGTH
            );
        }

        if pin.len() > MAX_PIN_LENGTH {
            bail!(
                "PIN must not exceed {} digits",
                MAX_PIN_LENGTH
            );
        }

        // Ensure PIN contains only digits
        if !pin.chars().all(|c| c.is_ascii_digit()) {
            bail!("PIN must contain only numeric digits (0-9)");
        }

        Ok(())
    }

    /// Hash a PIN using bcrypt.
    ///
    /// This function validates PIN requirements before hashing.
    /// PINs are typically 4-8 digit numeric codes used for voicemail access.
    pub fn hash_pin(pin: &str) -> Result<String> {
        validate_pin(pin)?;
        bcrypt::hash(pin, BCRYPT_COST)
            .map_err(|e| anyhow::anyhow!("Failed to hash PIN: {}", e))
    }

    /// Verify a plaintext PIN against a bcrypt hash.
    pub fn verify_pin(pin: &str, hash: &str) -> Result<bool> {
        bcrypt::verify(pin, hash)
            .map_err(|e| anyhow::anyhow!("Failed to verify PIN: {}", e))
    }

    /// Compute SIP Digest HA1 hash for authentication.
    ///
    /// The HA1 hash is: MD5(username:realm:password)
    /// This is used by SIP Digest authentication (RFC 2617/7616).
    ///
    /// # Arguments
    /// * `username` - The SIP username
    /// * `realm` - The authentication realm (e.g., "fcp.local")
    /// * `password` - The plaintext password
    ///
    /// # Returns
    /// Lowercase hex-encoded MD5 hash
    pub fn compute_ha1(username: &str, realm: &str, password: &str) -> String {
        let input = format!("{}:{}:{}", username, realm, password);
        let digest = md5::compute(input.as_bytes());
        format!("{:x}", digest)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_password_strength_validation() {
            // Valid passwords
            assert!(validate_password_strength("Passw0rd!").is_ok());
            assert!(validate_password_strength("MyS3cur3P@ss").is_ok());
            assert!(validate_password_strength("Tr0ng#Password").is_ok());

            // Too short
            assert!(validate_password_strength("Pass1!").is_err());

            // Too long
            let long_password = "A1!".to_string() + &"a".repeat(MAX_PASSWORD_LENGTH);
            assert!(validate_password_strength(&long_password).is_err());

            // Missing uppercase
            assert!(validate_password_strength("passw0rd!").is_err());

            // Missing lowercase
            assert!(validate_password_strength("PASSW0RD!").is_err());

            // Missing digit
            assert!(validate_password_strength("Password!").is_err());

            // Missing special character
            assert!(validate_password_strength("Passw0rd").is_err());
        }

        #[test]
        fn test_hash_password_validates_strength() {
            // Weak password should be rejected
            assert!(hash_password("weak").is_err());

            // Strong password should be accepted
            assert!(hash_password("Str0ng!Pass").is_ok());
        }

        #[test]
        fn test_pin_validation() {
            // Valid PINs
            assert!(validate_pin("1234").is_ok());
            assert!(validate_pin("123456").is_ok());
            assert!(validate_pin("12345678").is_ok());

            // Too short
            assert!(validate_pin("123").is_err());

            // Too long
            assert!(validate_pin("123456789").is_err());

            // Contains non-numeric characters
            assert!(validate_pin("12a4").is_err());
            assert!(validate_pin("12-4").is_err());
            assert!(validate_pin("PIN1").is_err());
        }

        #[test]
        fn test_hash_pin_validates() {
            // Valid PIN should be accepted
            assert!(hash_pin("1234").is_ok());
            assert!(hash_pin("567890").is_ok());

            // Invalid PIN should be rejected
            assert!(hash_pin("123").is_err()); // Too short
            assert!(hash_pin("abcd").is_err()); // Not numeric
        }
    }
}

/// A user account with SIP credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Username (SIP user part)
    pub username: String,

    /// Password hash (bcrypt) for API/web authentication
    /// This should contain a bcrypt hash, not plaintext.
    /// Use `storage::password::hash_password()` to hash passwords before storing.
    pub password: String,

    /// Separate digest password hash (bcrypt) for SIP authentication
    /// If None, the main password is used for SIP authentication.
    /// This provides Cisco-style security isolation between API and SIP credentials.
    /// Use `storage::password::hash_password()` to hash passwords before storing.
    pub digest_password: Option<String>,

    /// SIP HA1 hash for Digest authentication
    /// Format: MD5(username:realm:password)
    /// This is the ONLY credential needed for SIP authentication - we never store plaintext SIP passwords.
    /// The HA1 hash is pre-computed when setting the password and is sufficient for SIP Digest auth.
    /// Computed from digest_password (if set) or password (if digest_password is None).
    /// Use `storage::password::compute_ha1()` to generate this hash.
    pub sip_ha1_hash: Option<String>,

    /// Display name (optional)
    pub display_name: Option<String>,

    /// Tenant ID for multi-tenant isolation
    #[serde(default = "default_tenant_id")]
    pub tenant_id: crate::TenantId,

    /// Enabled flag
    pub enabled: bool,

    /// Creation timestamp
    #[serde(with = "system_time_serde")]
    pub created_at: SystemTime,

    /// Last modified timestamp
    #[serde(with = "system_time_serde")]
    pub updated_at: SystemTime,

    /// Media preferences for this user
    #[serde(default)]
    pub media_preferences: MediaPreferences,

    // ===== Enterprise-grade fields =====

    /// Email address
    pub email: Option<String>,

    /// Password last reset timestamp
    #[serde(with = "optional_system_time_serde", default)]
    pub password_last_reset: Option<SystemTime>,

    /// Primary phone number
    pub phone_number: Option<String>,

    /// Mobile phone number
    pub mobile_number: Option<String>,

    /// PIN for voicemail access (hashed)
    pub pin: Option<String>,

    /// Extension number
    pub extension: Option<String>,

    /// Department
    pub department: Option<String>,

    /// Job title
    pub job_title: Option<String>,

    /// Location/Office
    pub location: Option<String>,

    /// Language/Locale (e.g., "en-US", "es-ES")
    #[serde(default = "default_language")]
    pub language: String,

    /// Timezone (e.g., "America/New_York", "UTC")
    #[serde(default = "default_timezone")]
    pub timezone: String,

    /// Account status
    #[serde(default)]
    pub account_status: AccountStatus,

    /// Last login timestamp
    #[serde(with = "optional_system_time_serde", default)]
    pub last_login: Option<SystemTime>,

    /// Emergency contact information
    #[serde(default)]
    pub emergency_contact: EmergencyContact,

    // ===== Phone Number Support =====
    /// Primary phone number for outbound caller ID
    pub primary_phone_number_id: Option<crate::PhoneNumberId>,

    /// Voicemail box ID
    pub voicemail_box_id: Option<crate::VoicemailBoxId>,
}

fn default_language() -> String {
    "en-US".to_string()
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_tenant_id() -> crate::TenantId {
    crate::TenantId::nil()
}

/// Account status for user accounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    /// Account is active and can authenticate
    Active,
    /// Account is temporarily suspended
    Suspended,
    /// Account is locked (e.g., after too many failed login attempts)
    Locked,
}

impl Default for AccountStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl std::fmt::Display for AccountStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Suspended => write!(f, "suspended"),
            Self::Locked => write!(f, "locked"),
        }
    }
}

impl std::str::FromStr for AccountStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "suspended" => Ok(Self::Suspended),
            "locked" => Ok(Self::Locked),
            _ => anyhow::bail!("Invalid account status: {}", s),
        }
    }
}

/// Emergency contact information.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmergencyContact {
    /// Emergency contact name
    pub name: Option<String>,
    /// Emergency contact phone number
    pub phone: Option<String>,
}

/// Media preferences for a user account.
/// These settings control default behavior for calls to/from this user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaPreferences {
    /// Force media to always flow through server (for testing/debugging)
    #[serde(default)]
    pub force_media_anchoring: bool,

    /// Enable DTMF detection for this user's calls
    #[serde(default = "default_true")]
    pub enable_dtmf_detection: bool,

    /// Record all calls for this user
    #[serde(default)]
    pub record_all_calls: bool,

    /// Preferred audio codecs in priority order
    #[serde(default = "default_codecs")]
    pub preferred_codecs: Vec<String>,
}

impl Default for MediaPreferences {
    fn default() -> Self {
        Self {
            force_media_anchoring: false,
            enable_dtmf_detection: true,
            record_all_calls: false,
            preferred_codecs: default_codecs(),
        }
    }
}

fn default_codecs() -> Vec<String> {
    vec!["PCMU".to_string(), "PCMA".to_string()]
}

/// User definition for config files (passwords are plaintext and will be hashed on load).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFileEntry {
    /// Username
    pub username: String,

    /// Password (plaintext, will be hashed on load) for API/web authentication
    pub password: String,

    /// Separate digest password (plaintext, will be hashed on load) for SIP authentication
    /// If None, the main password will be used for SIP authentication.
    /// This provides Cisco-style security isolation.
    pub digest_password: Option<String>,

    /// Display name (optional)
    pub display_name: Option<String>,

    /// Tenant ID (optional, defaults to nil tenant)
    #[serde(default = "default_tenant_id")]
    pub tenant_id: crate::TenantId,

    /// Enabled flag (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Media preferences for this user
    #[serde(default)]
    pub media_preferences: MediaPreferences,

    // ===== Enterprise-grade fields =====

    /// Email address
    pub email: Option<String>,

    /// Primary phone number
    pub phone_number: Option<String>,

    /// Mobile phone number
    pub mobile_number: Option<String>,

    /// PIN for voicemail access (plaintext, will be hashed on load)
    pub pin: Option<String>,

    /// Extension number
    pub extension: Option<String>,

    /// Department
    pub department: Option<String>,

    /// Job title
    pub job_title: Option<String>,

    /// Location/Office
    pub location: Option<String>,

    /// Language/Locale (e.g., "en-US", "es-ES")
    #[serde(default = "default_language")]
    pub language: String,

    /// Timezone (e.g., "America/New_York", "UTC")
    #[serde(default = "default_timezone")]
    pub timezone: String,

    /// Account status
    #[serde(default)]
    pub account_status: AccountStatus,

    /// Emergency contact information
    #[serde(default)]
    pub emergency_contact: EmergencyContact,

    // ===== Phone Number Support =====
    /// Primary phone number for outbound caller ID
    pub primary_phone_number_id: Option<crate::PhoneNumberId>,

    /// Voicemail box ID
    pub voicemail_box_id: Option<crate::VoicemailBoxId>,
}

fn default_true() -> bool {
    true
}

/// A SIP registration binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registration {
    /// Address-of-Record (To URI)
    pub aor: String,

    /// Contact URI
    pub contact: String,

    /// Expiration time
    #[serde(with = "system_time_serde")]
    pub expires_at: SystemTime,

    /// Call-ID from REGISTER request
    pub call_id: String,

    /// CSeq from REGISTER request
    pub cseq: u32,

    /// User-Agent header value
    pub user_agent: Option<String>,

    /// Q-value for priority (0.0-1.0)
    pub q: Option<f32>,

    /// Registration timestamp
    #[serde(with = "system_time_serde")]
    pub registered_at: SystemTime,
}

/// Storage backend for user credentials.
#[async_trait]
pub trait CredentialStore: Send + Sync {
    /// Get a user by username.
    async fn get_user(&self, username: &str) -> Result<Option<User>>;

    /// Create or update a user.
    async fn put_user(&self, user: User) -> Result<()>;

    /// Delete a user by username.
    async fn delete_user(&self, username: &str) -> Result<()>;

    /// List all users.
    async fn list_users(&self) -> Result<Vec<User>>;

    /// Check if a username/password combination is valid.
    /// Uses bcrypt to verify the password against the stored hash.
    ///
    /// **Security Note**: This implementation performs constant-time verification
    /// to prevent user enumeration timing attacks. Even when a user doesn't exist,
    /// we perform a dummy bcrypt verification to match the timing of real verifications.
    async fn verify_credentials(&self, username: &str, password: &str) -> Result<bool> {
        // Dummy hash for constant-time comparison when user doesn't exist
        // This is a valid bcrypt hash that will always fail verification
        const DUMMY_HASH: &str = "$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/LewY5jtRKm5T6fHGC";

        if let Some(user) = self.get_user(username).await? {
            if !user.enabled {
                // Still perform bcrypt check for constant time
                let _ = password::verify_password(password, DUMMY_HASH);
                // Security audit log
                warn!(
                    event = "auth_failure_disabled",
                    username = %username,
                    "Authentication failed: account disabled"
                );
                return Ok(false);
            }
            // Use bcrypt to verify password against hash
            let result = password::verify_password(password, &user.password)?;

            if result {
                // Security audit log for successful authentication
                info!(
                    event = "auth_success",
                    username = %username,
                    tenant_id = %user.tenant_id,
                    "Authentication successful"
                );
            } else {
                // Security audit log for failed authentication
                warn!(
                    event = "auth_failure_invalid_password",
                    username = %username,
                    "Authentication failed: invalid password"
                );
            }

            Ok(result)
        } else {
            // User doesn't exist - perform dummy bcrypt verification
            // to match timing of real verification
            let _ = password::verify_password(password, DUMMY_HASH);

            // Security audit log (no username reveal)
            warn!(
                event = "auth_failure_user_not_found",
                "Authentication failed: user not found"
            );

            Ok(false)
        }
    }
}

/// Storage backend for SIP registrations.
#[async_trait]
pub trait RegistrationStore: Send + Sync {
    /// Get all registrations for an Address-of-Record.
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>>;

    /// Add or update a registration.
    async fn put_registration(&self, registration: Registration) -> Result<()>;

    /// Remove a specific registration by Contact URI.
    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()>;

    /// Remove all registrations for an AOR.
    async fn delete_all_registrations(&self, aor: &str) -> Result<()>;

    /// Remove expired registrations.
    async fn cleanup_expired(&self) -> Result<usize>;
}

/// Call disposition (answered, busy, failed, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallDisposition {
    /// Call was answered (200 OK)
    Answered,
    /// Callee was busy (486 Busy)
    Busy,
    /// Call was not answered (408/480)
    NoAnswer,
    /// Call failed (4xx/5xx/6xx)
    Failed,
    /// Call was cancelled (487/CANCEL)
    Cancelled,
}

/// Hangup cause codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HangupCause {
    /// Normal call clearing (BYE)
    NormalClearing,
    /// User busy (486)
    UserBusy,
    /// No answer (408/480)
    NoAnswer,
    /// Call rejected (603/488/603)
    CallRejected,
    /// Request timeout (408)
    Timeout,
    /// Network/transport error
    NetworkError,
    /// Internal server error (500)
    InternalError,
    /// Other cause with SIP status code
    Other(u16),
}

/// Which party hung up the call
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HangupParty {
    /// Caller (A-leg) hung up
    Caller,
    /// Callee (B-leg) hung up
    Callee,
    /// System terminated the call
    System,
}

/// Call Detail Record (CDR)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallDetailRecord {
    /// Unique call identifier (FCP internal)
    pub call_id: crate::CallId,

    /// Tenant ID for multi-tenancy
    pub tenant_id: crate::TenantId,

    /// SIP Call-ID header
    pub sip_call_id: String,

    /// Caller information
    pub from_uri: String,
    pub from_display_name: Option<String>,

    /// Callee information
    pub to_uri: String,
    pub to_display_name: Option<String>,

    /// Call timestamps
    #[serde(with = "system_time_serde")]
    pub start_time: SystemTime,
    #[serde(with = "optional_system_time_serde")]
    pub answer_time: Option<SystemTime>,
    #[serde(with = "optional_system_time_serde")]
    pub end_time: Option<SystemTime>,

    /// Call duration (milliseconds)
    pub setup_duration_ms: Option<u64>,
    pub call_duration_ms: Option<u64>,
    pub total_duration_ms: Option<u64>,

    /// Call disposition
    pub disposition: CallDisposition,
    pub hangup_cause: HangupCause,
    pub hangup_party: Option<HangupParty>,

    /// SIP response codes
    pub sip_status_code: Option<u16>,
    pub sip_reason: Option<String>,

    /// Routing information (placeholders for future use)
    pub route_id: Option<String>,
    pub gateway_id: Option<String>,

    /// Media information
    pub media_anchored: bool,
    pub codec_a_leg: Option<String>,
    pub codec_b_leg: Option<String>,
    pub rtp_packets_sent: Option<u64>,
    pub rtp_packets_received: Option<u64>,
    pub rtp_bytes_sent: Option<u64>,
    pub rtp_bytes_received: Option<u64>,

    /// Recording
    pub recording_url: Option<String>,

    /// Cost (for billing)
    pub cost: Option<f64>,
    pub currency: Option<String>,

    /// Custom fields (JSON)
    pub custom_data: Option<serde_json::Value>,
}

/// CDR query filter
#[derive(Debug, Clone, Default)]
pub struct CdrQuery {
    /// Filter by tenant ID
    pub tenant_id: Option<crate::TenantId>,
    /// Filter by call ID
    pub call_id: Option<crate::CallId>,
    /// Filter by from URI (caller)
    pub from_uri: Option<String>,
    /// Filter by to URI (callee)
    pub to_uri: Option<String>,
    /// Filter by start time range
    pub start_time_from: Option<SystemTime>,
    pub start_time_to: Option<SystemTime>,
    /// Filter by disposition
    pub disposition: Option<CallDisposition>,
    /// Limit results
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

/// Storage backend for Call Detail Records
#[async_trait]
pub trait CdrStore: Send + Sync {
    /// Insert a new CDR
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()>;

    /// Get a CDR by call ID
    async fn get_cdr(&self, call_id: &crate::CallId) -> Result<Option<CallDetailRecord>>;

    /// Query CDRs with filters
    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>>;

    /// Get CDR count for a query
    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize>;
}

/// Storage backend for phone numbers
#[async_trait]
pub trait PhoneNumberStore: Send + Sync {
    /// Get a phone number by ID
    async fn get_phone_number(
        &self,
        id: &crate::PhoneNumberId,
    ) -> Result<Option<crate::PhoneNumber>>;

    /// Get a phone number by E.164 string
    async fn get_phone_number_by_e164(&self, e164: &str) -> Result<Option<crate::PhoneNumber>>;

    /// List all phone numbers for a tenant
    async fn list_phone_numbers(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::PhoneNumber>>;

    /// Create or update a phone number
    async fn put_phone_number(&self, phone_number: crate::PhoneNumber) -> Result<()>;

    /// Delete a phone number by ID
    async fn delete_phone_number(&self, id: &crate::PhoneNumberId) -> Result<()>;
}

/// Storage backend for line bindings
#[async_trait]
pub trait LineBindingStore: Send + Sync {
    /// Get a line binding by ID
    async fn get_line_binding(
        &self,
        id: &crate::LineBindingId,
    ) -> Result<Option<crate::LineBinding>>;

    /// Get line binding for a phone number
    async fn get_line_binding_by_phone_number(
        &self,
        phone_number_id: &crate::PhoneNumberId,
    ) -> Result<Option<crate::LineBinding>>;

    /// List all line bindings for a tenant
    async fn list_line_bindings(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::LineBinding>>;

    /// List line bindings for a user
    async fn list_line_bindings_by_user(
        &self,
        user_id: &crate::UserId,
    ) -> Result<Vec<crate::LineBinding>>;

    /// List line bindings for a device
    async fn list_line_bindings_by_device(
        &self,
        device_id: &crate::DeviceId,
    ) -> Result<Vec<crate::LineBinding>>;

    /// Create or update a line binding
    async fn put_line_binding(&self, binding: crate::LineBinding) -> Result<()>;

    /// Delete a line binding by ID
    async fn delete_line_binding(&self, id: &crate::LineBindingId) -> Result<()>;
}

/// Storage backend for extensions
#[async_trait]
pub trait ExtensionStore: Send + Sync {
    /// Get an extension by ID
    async fn get_extension(&self, id: &crate::ExtensionId) -> Result<Option<crate::Extension>>;

    /// Get an extension by number within a tenant
    async fn get_extension_by_number(
        &self,
        tenant_id: &crate::TenantId,
        number: &str,
    ) -> Result<Option<crate::Extension>>;

    /// List all extensions for a tenant
    async fn list_extensions(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Extension>>;

    /// Create or update an extension
    async fn put_extension(&self, extension: crate::Extension) -> Result<()>;

    /// Delete an extension by ID
    async fn delete_extension(&self, id: &crate::ExtensionId) -> Result<()>;
}

/// Storage backend for devices
#[async_trait]
pub trait DeviceStore: Send + Sync {
    /// Get a device by ID
    async fn get_device(&self, id: &crate::DeviceId) -> Result<Option<crate::Device>>;

    /// Get a device by SIP URI
    async fn get_device_by_sip_uri(&self, sip_uri: &str) -> Result<Option<crate::Device>>;

    /// List all devices for a tenant
    async fn list_devices(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Device>>;

    /// List devices for a user
    async fn list_devices_by_user(&self, user_id: &crate::UserId) -> Result<Vec<crate::Device>>;

    /// Create or update a device
    async fn put_device(&self, device: crate::Device) -> Result<()>;

    /// Delete a device by ID
    async fn delete_device(&self, id: &crate::DeviceId) -> Result<()>;
}

/// Voicemail storage operations.
#[async_trait::async_trait]
pub trait VoicemailStore: Send + Sync {
    /// Get a voicemail mailbox by ID
    async fn get_mailbox(&self, id: &str) -> Result<Option<crate::VoicemailMailbox>>;

    /// Get a mailbox by line binding ID
    async fn get_mailbox_by_line_binding(
        &self,
        line_binding_id: &crate::LineBindingId,
    ) -> Result<Option<crate::VoicemailMailbox>>;

    /// Get a mailbox by user ID
    async fn get_mailbox_by_user(
        &self,
        user_id: &crate::UserId,
    ) -> Result<Option<crate::VoicemailMailbox>>;

    /// Get a mailbox by extension
    async fn get_mailbox_by_extension(
        &self,
        tenant_id: &crate::TenantId,
        extension: &str,
    ) -> Result<Option<crate::VoicemailMailbox>>;

    /// List all mailboxes for a tenant
    async fn list_mailboxes(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::VoicemailMailbox>>;

    /// Create or update a mailbox
    async fn put_mailbox(&self, mailbox: crate::VoicemailMailbox) -> Result<()>;

    /// Delete a mailbox
    async fn delete_mailbox(&self, id: &str) -> Result<()>;

    /// Get a voicemail message by ID
    async fn get_message(&self, id: &str) -> Result<Option<crate::VoicemailMessage>>;

    /// List messages for a mailbox
    async fn list_messages(
        &self,
        mailbox_id: &str,
    ) -> Result<Vec<crate::VoicemailMessage>>;

    /// List messages by status
    async fn list_messages_by_status(
        &self,
        mailbox_id: &str,
        status: crate::MessageStatus,
    ) -> Result<Vec<crate::VoicemailMessage>>;

    /// Create or update a message
    async fn put_message(&self, message: crate::VoicemailMessage) -> Result<()>;

    /// Delete a message
    async fn delete_message(&self, id: &str) -> Result<()>;

    /// Get MWI status for a mailbox
    async fn get_mwi_status(&self, mailbox_id: &str) -> Result<crate::MwiStatus>;

    /// Get mailbox statistics
    async fn get_mailbox_stats(&self, mailbox_id: &str) -> Result<crate::VoicemailStats>;
}

/// Geographic routing storage operations.
#[async_trait::async_trait]
pub trait GeoRoutingStore: Send + Sync {
    /// Get a geographic routing rule by ID
    async fn get_geo_rule(&self, id: &str) -> Result<Option<crate::GeoRoutingRule>>;

    /// List all geographic routing rules for a tenant
    async fn list_geo_rules(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::GeoRoutingRule>>;

    /// Create or update a geographic routing rule
    async fn put_geo_rule(&self, rule: crate::GeoRoutingRule) -> Result<()>;

    /// Delete a geographic routing rule
    async fn delete_geo_rule(&self, id: &str) -> Result<()>;

    /// Get phone number location information
    async fn get_phone_number_location(
        &self,
        phone_number_id: &crate::PhoneNumberId,
    ) -> Result<Option<crate::PhoneNumberLocation>>;

    /// List all phone number locations for a tenant
    async fn list_phone_number_locations(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::PhoneNumberLocation>>;

    /// Create or update phone number location
    async fn put_phone_number_location(
        &self,
        location: crate::PhoneNumberLocation,
    ) -> Result<()>;

    /// Delete phone number location
    async fn delete_phone_number_location(
        &self,
        phone_number_id: &crate::PhoneNumberId,
    ) -> Result<()>;
}

/// Serde helper for Optional<SystemTime>
mod optional_system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &Option<SystemTime>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match time {
            Some(t) => {
                let duration = t
                    .duration_since(UNIX_EPOCH)
                    .map_err(serde::ser::Error::custom)?;
                Some(duration.as_secs()).serialize(serializer)
            }
            None => None::<u64>.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SystemTime>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = Option::<u64>::deserialize(deserializer)?;
        Ok(secs.map(|s| UNIX_EPOCH + std::time::Duration::from_secs(s)))
    }
}

/// In-memory storage with JSON file persistence.
///
/// **Note:** This implementation is intended for development and testing only.
/// For production deployments, use SQLite or another database backend.
///
/// This implementation keeps data in memory and saves to JSON files. It does not
/// persist CDRs (Call Detail Records) - they are discarded. For full CDR support
/// and ACID compliance, use the SQLite backend (fcp-storage-sqlite).
pub struct FileBackedStorage {
    /// In-memory user store
    users: Arc<RwLock<HashMap<String, User>>>,

    /// In-memory registration store (keyed by AOR)
    registrations: Arc<RwLock<HashMap<String, Vec<Registration>>>>,

    /// Path to users JSON file
    users_file: PathBuf,

    /// Path to registrations JSON file
    registrations_file: PathBuf,
}

impl FileBackedStorage {
    /// Create a new file-backed storage.
    ///
    /// # Arguments
    /// * `data_dir` - Directory to store JSON files
    pub async fn new(data_dir: PathBuf) -> Result<Self> {
        // Ensure data directory exists
        tokio::fs::create_dir_all(&data_dir).await?;

        let users_file = data_dir.join("users.json");
        let registrations_file = data_dir.join("registrations.json");

        // Load existing data
        let users = Self::load_users(&users_file).await?;
        let registrations = Self::load_registrations(&registrations_file).await?;

        Ok(Self {
            users: Arc::new(RwLock::new(users)),
            registrations: Arc::new(RwLock::new(registrations)),
            users_file,
            registrations_file,
        })
    }

    /// Load users from JSON file.
    async fn load_users(path: &PathBuf) -> Result<HashMap<String, User>> {
        if path.exists() {
            let data = tokio::fs::read_to_string(path).await?;
            let users: Vec<User> = serde_json::from_str(&data)?;
            Ok(users.into_iter().map(|u| (u.username.clone(), u)).collect())
        } else {
            Ok(HashMap::new())
        }
    }

    /// Load registrations from JSON file.
    async fn load_registrations(path: &PathBuf) -> Result<HashMap<String, Vec<Registration>>> {
        if path.exists() {
            let data = tokio::fs::read_to_string(path).await?;
            let regs: Vec<Registration> = serde_json::from_str(&data)?;

            // Group by AOR
            let mut map: HashMap<String, Vec<Registration>> = HashMap::new();
            for reg in regs {
                map.entry(reg.aor.clone()).or_default().push(reg);
            }
            Ok(map)
        } else {
            Ok(HashMap::new())
        }
    }

    /// Save users to JSON file.
    async fn save_users(&self) -> Result<()> {
        let users = self.users.read().await;
        let user_list: Vec<User> = users.values().cloned().collect();
        let data = serde_json::to_string_pretty(&user_list)?;
        tokio::fs::write(&self.users_file, data).await?;
        Ok(())
    }

    /// Save registrations to JSON file.
    async fn save_registrations(&self) -> Result<()> {
        let registrations = self.registrations.read().await;
        let reg_list: Vec<Registration> = registrations.values().flatten().cloned().collect();
        let data = serde_json::to_string_pretty(&reg_list)?;
        tokio::fs::write(&self.registrations_file, data).await?;
        Ok(())
    }
}

#[async_trait]
impl CredentialStore for FileBackedStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        let users = self.users.read().await;
        Ok(users.get(username).cloned())
    }

    async fn put_user(&self, user: User) -> Result<()> {
        let mut users = self.users.write().await;
        users.insert(user.username.clone(), user);
        drop(users);

        // Save to file
        self.save_users().await?;
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        let mut users = self.users.write().await;
        users.remove(username);
        drop(users);

        // Save to file
        self.save_users().await?;
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        let users = self.users.read().await;
        Ok(users.values().cloned().collect())
    }
}

#[async_trait]
impl RegistrationStore for FileBackedStorage {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        let registrations = self.registrations.read().await;
        Ok(registrations.get(aor).cloned().unwrap_or_default())
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        let mut registrations = self.registrations.write().await;
        let aor_regs = registrations.entry(registration.aor.clone()).or_default();

        // Remove existing registration with same contact
        aor_regs.retain(|r| r.contact != registration.contact);

        // Add new registration
        aor_regs.push(registration);
        drop(registrations);

        // Save to file
        self.save_registrations().await?;
        Ok(())
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        let mut registrations = self.registrations.write().await;
        if let Some(aor_regs) = registrations.get_mut(aor) {
            aor_regs.retain(|r| r.contact != contact);
            if aor_regs.is_empty() {
                registrations.remove(aor);
            }
        }
        drop(registrations);

        // Save to file
        self.save_registrations().await?;
        Ok(())
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        let mut registrations = self.registrations.write().await;
        registrations.remove(aor);
        drop(registrations);

        // Save to file
        self.save_registrations().await?;
        Ok(())
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        let mut registrations = self.registrations.write().await;
        let now = SystemTime::now();
        let mut removed = 0;

        // Remove expired registrations
        for aor_regs in registrations.values_mut() {
            let before = aor_regs.len();
            aor_regs.retain(|r| r.expires_at > now);
            removed += before - aor_regs.len();
        }

        // Remove empty AORs
        registrations.retain(|_, regs| !regs.is_empty());
        drop(registrations);

        // Save to file if anything was removed
        if removed > 0 {
            self.save_registrations().await?;
        }

        Ok(removed)
    }
}

#[async_trait]
impl CdrStore for FileBackedStorage {
    async fn insert_cdr(&self, _cdr: CallDetailRecord) -> Result<()> {
        // FileBackedStorage doesn't persist CDRs - this is a no-op
        // For production use cases, use a proper database backend (PostgreSQL, MySQL, MongoDB)
        warn!("FileBackedStorage does not persist CDRs - use a database backend for production");
        Ok(())
    }

    async fn get_cdr(&self, _call_id: &crate::CallId) -> Result<Option<CallDetailRecord>> {
        // FileBackedStorage doesn't store CDRs
        Ok(None)
    }

    async fn query_cdrs(&self, _query: CdrQuery) -> Result<Vec<CallDetailRecord>> {
        // FileBackedStorage doesn't store CDRs
        Ok(Vec::new())
    }

    async fn count_cdrs(&self, _query: CdrQuery) -> Result<usize> {
        // FileBackedStorage doesn't store CDRs
        Ok(0)
    }
}

/// Serde helper for SystemTime
mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time
            .duration_since(UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?;
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }
}

/// Dynamic storage wrapper that can hold any storage backend implementing the required traits.
///
/// This allows the runtime to work with different storage backends (SQLite, PostgreSQL, MySQL, MongoDB)
/// without being tied to a specific implementation at compile time.
#[derive(Clone)]
pub struct DynStorage {
    credential_store: Arc<dyn CredentialStore>,
    registration_store: Arc<dyn RegistrationStore>,
    cdr_store: Arc<dyn CdrStore>,
    phone_number_store: Arc<dyn PhoneNumberStore>,
    line_binding_store: Arc<dyn LineBindingStore>,
    extension_store: Arc<dyn ExtensionStore>,
    device_store: Arc<dyn DeviceStore>,
    voicemail_store: Arc<dyn VoicemailStore>,
    geo_routing_store: Arc<dyn GeoRoutingStore>,
    porting_store: Arc<dyn crate::porting::PortingStore>,
}

impl DynStorage {
    /// Create a new DynStorage from individual trait objects.
    pub fn new(
        credential_store: Arc<dyn CredentialStore>,
        registration_store: Arc<dyn RegistrationStore>,
        cdr_store: Arc<dyn CdrStore>,
        phone_number_store: Arc<dyn PhoneNumberStore>,
        line_binding_store: Arc<dyn LineBindingStore>,
        extension_store: Arc<dyn ExtensionStore>,
        device_store: Arc<dyn DeviceStore>,
        voicemail_store: Arc<dyn VoicemailStore>,
        geo_routing_store: Arc<dyn GeoRoutingStore>,
        porting_store: Arc<dyn crate::porting::PortingStore>,
    ) -> Self {
        Self {
            credential_store,
            registration_store,
            cdr_store,
            phone_number_store,
            line_binding_store,
            extension_store,
            device_store,
            voicemail_store,
            geo_routing_store,
            porting_store,
        }
    }

    /// Create a new DynStorage from a storage backend that implements all traits.
    ///
    /// This is a convenience method for backends like SqliteStorage that implement
    /// all storage traits.
    pub fn from_all_in_one<T>(storage: Arc<T>) -> Self
    where
        T: CredentialStore
            + RegistrationStore
            + CdrStore
            + PhoneNumberStore
            + LineBindingStore
            + ExtensionStore
            + DeviceStore
            + VoicemailStore
            + GeoRoutingStore
            + crate::porting::PortingStore
            + 'static,
    {
        Self {
            credential_store: storage.clone(),
            registration_store: storage.clone(),
            cdr_store: storage.clone(),
            phone_number_store: storage.clone(),
            line_binding_store: storage.clone(),
            extension_store: storage.clone(),
            device_store: storage.clone(),
            voicemail_store: storage.clone(),
            geo_routing_store: storage.clone(),
            porting_store: storage,
        }
    }
}

#[async_trait]
impl CredentialStore for DynStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        self.credential_store.get_user(username).await
    }

    async fn put_user(&self, user: User) -> Result<()> {
        self.credential_store.put_user(user).await
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        self.credential_store.delete_user(username).await
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        self.credential_store.list_users().await
    }
}

#[async_trait]
impl RegistrationStore for DynStorage {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        self.registration_store.get_registrations(aor).await
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        self.registration_store.put_registration(registration).await
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        self.registration_store
            .delete_registration(aor, contact)
            .await
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        self.registration_store
            .delete_all_registrations(aor)
            .await
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        self.registration_store.cleanup_expired().await
    }
}

#[async_trait]
impl CdrStore for DynStorage {
    async fn insert_cdr(&self, cdr: CallDetailRecord) -> Result<()> {
        self.cdr_store.insert_cdr(cdr).await
    }

    async fn get_cdr(&self, call_id: &crate::CallId) -> Result<Option<CallDetailRecord>> {
        self.cdr_store.get_cdr(call_id).await
    }

    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>> {
        self.cdr_store.query_cdrs(query).await
    }

    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize> {
        self.cdr_store.count_cdrs(query).await
    }
}

#[async_trait]
impl PhoneNumberStore for DynStorage {
    async fn get_phone_number(
        &self,
        id: &crate::PhoneNumberId,
    ) -> Result<Option<crate::PhoneNumber>> {
        self.phone_number_store.get_phone_number(id).await
    }

    async fn get_phone_number_by_e164(&self, e164: &str) -> Result<Option<crate::PhoneNumber>> {
        self.phone_number_store.get_phone_number_by_e164(e164).await
    }

    async fn list_phone_numbers(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::PhoneNumber>> {
        self.phone_number_store.list_phone_numbers(tenant_id).await
    }

    async fn put_phone_number(&self, phone_number: crate::PhoneNumber) -> Result<()> {
        self.phone_number_store.put_phone_number(phone_number).await
    }

    async fn delete_phone_number(&self, id: &crate::PhoneNumberId) -> Result<()> {
        self.phone_number_store.delete_phone_number(id).await
    }
}

#[async_trait]
impl LineBindingStore for DynStorage {
    async fn get_line_binding(
        &self,
        id: &crate::LineBindingId,
    ) -> Result<Option<crate::LineBinding>> {
        self.line_binding_store.get_line_binding(id).await
    }

    async fn get_line_binding_by_phone_number(
        &self,
        phone_number_id: &crate::PhoneNumberId,
    ) -> Result<Option<crate::LineBinding>> {
        self.line_binding_store
            .get_line_binding_by_phone_number(phone_number_id)
            .await
    }

    async fn list_line_bindings(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::LineBinding>> {
        self.line_binding_store.list_line_bindings(tenant_id).await
    }

    async fn list_line_bindings_by_user(
        &self,
        user_id: &crate::UserId,
    ) -> Result<Vec<crate::LineBinding>> {
        self.line_binding_store
            .list_line_bindings_by_user(user_id)
            .await
    }

    async fn list_line_bindings_by_device(
        &self,
        device_id: &crate::DeviceId,
    ) -> Result<Vec<crate::LineBinding>> {
        self.line_binding_store
            .list_line_bindings_by_device(device_id)
            .await
    }

    async fn put_line_binding(&self, binding: crate::LineBinding) -> Result<()> {
        self.line_binding_store.put_line_binding(binding).await
    }

    async fn delete_line_binding(&self, id: &crate::LineBindingId) -> Result<()> {
        self.line_binding_store.delete_line_binding(id).await
    }
}

#[async_trait]
impl ExtensionStore for DynStorage {
    async fn get_extension(&self, id: &crate::ExtensionId) -> Result<Option<crate::Extension>> {
        self.extension_store.get_extension(id).await
    }

    async fn get_extension_by_number(
        &self,
        tenant_id: &crate::TenantId,
        number: &str,
    ) -> Result<Option<crate::Extension>> {
        self.extension_store
            .get_extension_by_number(tenant_id, number)
            .await
    }

    async fn list_extensions(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Extension>> {
        self.extension_store.list_extensions(tenant_id).await
    }

    async fn put_extension(&self, extension: crate::Extension) -> Result<()> {
        self.extension_store.put_extension(extension).await
    }

    async fn delete_extension(&self, id: &crate::ExtensionId) -> Result<()> {
        self.extension_store.delete_extension(id).await
    }
}

#[async_trait]
impl DeviceStore for DynStorage {
    async fn get_device(&self, id: &crate::DeviceId) -> Result<Option<crate::Device>> {
        self.device_store.get_device(id).await
    }

    async fn get_device_by_sip_uri(&self, sip_uri: &str) -> Result<Option<crate::Device>> {
        self.device_store.get_device_by_sip_uri(sip_uri).await
    }

    async fn list_devices(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Device>> {
        self.device_store.list_devices(tenant_id).await
    }

    async fn list_devices_by_user(&self, user_id: &crate::UserId) -> Result<Vec<crate::Device>> {
        self.device_store.list_devices_by_user(user_id).await
    }

    async fn put_device(&self, device: crate::Device) -> Result<()> {
        self.device_store.put_device(device).await
    }

    async fn delete_device(&self, id: &crate::DeviceId) -> Result<()> {
        self.device_store.delete_device(id).await
    }
}

#[async_trait]
impl VoicemailStore for DynStorage {
    async fn get_mailbox(&self, id: &str) -> Result<Option<crate::VoicemailMailbox>> {
        self.voicemail_store.get_mailbox(id).await
    }

    async fn get_mailbox_by_line_binding(
        &self,
        line_binding_id: &crate::LineBindingId,
    ) -> Result<Option<crate::VoicemailMailbox>> {
        self.voicemail_store
            .get_mailbox_by_line_binding(line_binding_id)
            .await
    }

    async fn get_mailbox_by_user(
        &self,
        user_id: &crate::UserId,
    ) -> Result<Option<crate::VoicemailMailbox>> {
        self.voicemail_store.get_mailbox_by_user(user_id).await
    }

    async fn get_mailbox_by_extension(
        &self,
        tenant_id: &crate::TenantId,
        extension: &str,
    ) -> Result<Option<crate::VoicemailMailbox>> {
        self.voicemail_store
            .get_mailbox_by_extension(tenant_id, extension)
            .await
    }

    async fn list_mailboxes(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::VoicemailMailbox>> {
        self.voicemail_store.list_mailboxes(tenant_id).await
    }

    async fn put_mailbox(&self, mailbox: crate::VoicemailMailbox) -> Result<()> {
        self.voicemail_store.put_mailbox(mailbox).await
    }

    async fn delete_mailbox(&self, id: &str) -> Result<()> {
        self.voicemail_store.delete_mailbox(id).await
    }

    async fn get_message(&self, id: &str) -> Result<Option<crate::VoicemailMessage>> {
        self.voicemail_store.get_message(id).await
    }

    async fn list_messages(&self, mailbox_id: &str) -> Result<Vec<crate::VoicemailMessage>> {
        self.voicemail_store.list_messages(mailbox_id).await
    }

    async fn list_messages_by_status(
        &self,
        mailbox_id: &str,
        status: crate::MessageStatus,
    ) -> Result<Vec<crate::VoicemailMessage>> {
        self.voicemail_store
            .list_messages_by_status(mailbox_id, status)
            .await
    }

    async fn put_message(&self, message: crate::VoicemailMessage) -> Result<()> {
        self.voicemail_store.put_message(message).await
    }

    async fn delete_message(&self, id: &str) -> Result<()> {
        self.voicemail_store.delete_message(id).await
    }

    async fn get_mwi_status(&self, mailbox_id: &str) -> Result<crate::MwiStatus> {
        self.voicemail_store.get_mwi_status(mailbox_id).await
    }

    async fn get_mailbox_stats(&self, mailbox_id: &str) -> Result<crate::VoicemailStats> {
        self.voicemail_store.get_mailbox_stats(mailbox_id).await
    }
}

#[async_trait]
impl GeoRoutingStore for DynStorage {
    async fn get_geo_rule(&self, id: &str) -> Result<Option<crate::GeoRoutingRule>> {
        self.geo_routing_store.get_geo_rule(id).await
    }

    async fn list_geo_rules(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::GeoRoutingRule>> {
        self.geo_routing_store.list_geo_rules(tenant_id).await
    }

    async fn put_geo_rule(&self, rule: crate::GeoRoutingRule) -> Result<()> {
        self.geo_routing_store.put_geo_rule(rule).await
    }

    async fn delete_geo_rule(&self, id: &str) -> Result<()> {
        self.geo_routing_store.delete_geo_rule(id).await
    }

    async fn get_phone_number_location(
        &self,
        phone_number_id: &crate::PhoneNumberId,
    ) -> Result<Option<crate::PhoneNumberLocation>> {
        self.geo_routing_store
            .get_phone_number_location(phone_number_id)
            .await
    }

    async fn list_phone_number_locations(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::PhoneNumberLocation>> {
        self.geo_routing_store
            .list_phone_number_locations(tenant_id)
            .await
    }

    async fn put_phone_number_location(
        &self,
        location: crate::PhoneNumberLocation,
    ) -> Result<()> {
        self.geo_routing_store
            .put_phone_number_location(location)
            .await
    }

    async fn delete_phone_number_location(
        &self,
        phone_number_id: &crate::PhoneNumberId,
    ) -> Result<()> {
        self.geo_routing_store
            .delete_phone_number_location(phone_number_id)
            .await
    }
}

#[async_trait]
impl crate::porting::PortingStore for DynStorage {
    async fn get_port_request(
        &self,
        id: &crate::PortRequestId,
    ) -> Result<Option<crate::PortRequest>> {
        self.porting_store.get_port_request(id).await
    }

    async fn list_port_requests(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::PortRequest>> {
        self.porting_store.list_port_requests(tenant_id).await
    }

    async fn list_port_requests_by_status(
        &self,
        tenant_id: &crate::TenantId,
        status: crate::PortStatus,
    ) -> Result<Vec<crate::PortRequest>> {
        self.porting_store
            .list_port_requests_by_status(tenant_id, status)
            .await
    }

    async fn put_port_request(&self, request: crate::PortRequest) -> Result<()> {
        self.porting_store.put_port_request(request).await
    }

    async fn delete_port_request(&self, id: &crate::PortRequestId) -> Result<()> {
        self.porting_store.delete_port_request(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_file_backed_storage_users() {
        let temp_dir = std::env::temp_dir().join("fcp-test-storage");
        let storage = FileBackedStorage::new(temp_dir.clone()).await.unwrap();

        // Create user with hashed password (must meet strength requirements)
        let password_hash = password::hash_password("Secr3t!Pass").unwrap();
        let user = User {
            username: "alice".to_string(),
            password: password_hash,
            digest_password: None,
            sip_ha1_hash: None,
            display_name: Some("Alice".to_string()),
            tenant_id: crate::TenantId::nil(), // Default tenant for test
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            media_preferences: MediaPreferences::default(),
            email: None,
            password_last_reset: None,
            phone_number: None,
            mobile_number: None,
            pin: None,
            extension: None,
            department: None,
            job_title: None,
            location: None,
            language: "en-US".to_string(),
            timezone: "UTC".to_string(),
            account_status: AccountStatus::Active,
            last_login: None,
            emergency_contact: EmergencyContact::default(),
            primary_phone_number_id: None,
            voicemail_box_id: None,
        };

        storage.put_user(user.clone()).await.unwrap();

        // Get user
        let retrieved = storage.get_user("alice").await.unwrap().unwrap();
        assert_eq!(retrieved.username, "alice");

        // Verify credentials
        assert!(storage
            .verify_credentials("alice", "Secr3t!Pass")
            .await
            .unwrap());
        assert!(!storage.verify_credentials("alice", "wrong").await.unwrap());

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir).await.ok();
    }

    #[tokio::test]
    async fn test_file_backed_storage_registrations() {
        let temp_dir = std::env::temp_dir().join("fcp-test-storage-reg");
        let storage = FileBackedStorage::new(temp_dir.clone()).await.unwrap();

        // Create registration
        let reg = Registration {
            aor: "sip:alice@example.com".to_string(),
            contact: "sip:alice@192.168.1.100:5060".to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            call_id: "test-call-id".to_string(),
            cseq: 1,
            user_agent: Some("FCP Test".to_string()),
            q: Some(1.0),
            registered_at: SystemTime::now(),
        };

        storage.put_registration(reg.clone()).await.unwrap();

        // Get registrations
        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].contact, "sip:alice@192.168.1.100:5060");

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir).await.ok();
    }
}
