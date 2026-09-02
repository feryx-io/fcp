//! Storage abstraction layer for FCP.
//!
//! This module provides traits and implementations for storing:
//! - User credentials (for authentication)
//! - SIP registrations (location bindings)
//! - Call detail records (CDRs)
//!
//! ## Storage Backends
//!
//! FCP supports the following storage backends:
//! - **PostgreSQL** (production): ACID-compliant storage with JSONB, connection pooling, and migrations
//! - **Redis**: Caching, event bus, distributed state (parking, circuit breaker)
//! - **FileBackedStorage** (development): In-memory with JSON file persistence
//!
//! PostgreSQL is the recommended backend for production deployments. FileBackedStorage
//! is suitable for development and testing only.

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
            bail!("PIN must be at least {} digits long", MIN_PIN_LENGTH);
        }

        if pin.len() > MAX_PIN_LENGTH {
            bail!("PIN must not exceed {} digits", MAX_PIN_LENGTH);
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
        bcrypt::hash(pin, BCRYPT_COST).map_err(|e| anyhow::anyhow!("Failed to hash PIN: {}", e))
    }

    /// Verify a plaintext PIN against a bcrypt hash.
    pub fn verify_pin(pin: &str, hash: &str) -> Result<bool> {
        bcrypt::verify(pin, hash).map_err(|e| anyhow::anyhow!("Failed to verify PIN: {}", e))
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
    /// Unique user ID
    #[serde(default = "crate::UserId::new")]
    pub id: crate::UserId,

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

    /// Plaintext SIP password reserved for auto-provisioning templates.
    ///
    /// This is optional and only set when an administrator explicitly creates
    /// or rotates SIP credentials for provisioning. It is not used for API/web
    /// authentication and should be protected as sensitive credential material.
    #[serde(default)]
    pub sip_provisioning_password: Option<String>,

    /// Display name (optional)
    pub display_name: Option<String>,

    /// Tenant ID for multi-tenant isolation
    #[serde(default = "default_tenant_id")]
    pub tenant_id: crate::TenantId,

    /// User role for permissions
    #[serde(default)]
    pub role: crate::UserRole,

    /// Department ID for department-scoped admins
    pub department_id: Option<String>,

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

    /// Reference to a SiteLocation entity
    pub location_id: Option<crate::LocationId>,

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

    /// Number of consecutive failed login attempts
    #[serde(default)]
    pub failed_login_attempts: i32,

    /// Account locked until this time (None = not time-locked)
    #[serde(with = "optional_system_time_serde", default)]
    pub account_locked_until: Option<SystemTime>,

    /// Emergency contact information
    #[serde(default)]
    pub emergency_contact: EmergencyContact,

    // ===== Phone Number Support =====
    /// Primary phone number for outbound caller ID
    pub primary_phone_number_id: Option<crate::PhoneNumberId>,

    /// Voicemail box ID
    pub voicemail_box_id: Option<crate::VoicemailBoxId>,

    /// Profile picture filename (e.g. "avatar.png")
    pub profile_picture: Option<String>,
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
#[derive(Default)]
pub enum AccountStatus {
    /// Account is active and can authenticate
    #[default]
    Active,
    /// Account is temporarily suspended
    Suspended,
    /// Account is locked (e.g., after too many failed login attempts)
    Locked,
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

impl User {
    /// Check if user has a specific permission in the given deployment mode
    pub fn has_permission(
        &self,
        permission: crate::Permission,
        mode: crate::DeploymentMode,
    ) -> bool {
        self.role.has_permission(permission, mode)
    }

    /// Check if user can access a specific tenant
    pub fn can_access_tenant(&self, tenant_id: &crate::TenantId) -> bool {
        match self.role {
            crate::UserRole::SuperAdmin => true,
            _ => &self.tenant_id == tenant_id,
        }
    }

    /// Check if user can access a specific department
    pub fn can_access_department(&self, dept_id: &str) -> bool {
        match self.role {
            crate::UserRole::SuperAdmin
            | crate::UserRole::TelecomAdmin
            | crate::UserRole::UserAdmin => true,
            crate::UserRole::DepartmentAdmin => self.department_id.as_deref() == Some(dept_id),
            _ => false,
        }
    }

    /// Get all permissions this user has in the given mode
    pub fn permissions(&self, mode: crate::DeploymentMode) -> Vec<crate::Permission> {
        self.role.permissions(mode)
    }
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

    /// Plaintext SIP password reserved for auto-provisioning templates.
    #[serde(default)]
    pub sip_provisioning_password: Option<String>,

    /// Display name (optional)
    pub display_name: Option<String>,

    /// Tenant ID (optional, defaults to nil tenant)
    #[serde(default = "default_tenant_id")]
    pub tenant_id: crate::TenantId,

    /// User role (optional, defaults to EndUser)
    #[serde(default)]
    pub role: Option<crate::UserRole>,

    /// Department ID (optional)
    pub department_id: Option<String>,

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

    /// Reference to a SiteLocation entity
    pub location_id: Option<crate::LocationId>,

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
    /// Tenant ID for multi-tenant isolation.
    #[serde(default = "default_tenant_id")]
    pub tenant_id: crate::TenantId,

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

    /// Path header from REGISTER request (RFC 3327)
    /// Stored as serialized string for persistence.
    /// The registrar uses this to build route sets for requests sent to the UA.
    #[serde(default)]
    pub path: Option<String>,

    /// Flow token for RFC 5626 Outbound support.
    /// Identifies the TLS/TCP connection for reusing inbound connections
    /// when routing requests to NAT'd clients.
    #[serde(default)]
    pub flow_token: Option<String>,
}

/// Storage backend for user credentials.
#[async_trait]
pub trait CredentialStore: Send + Sync {
    /// Get a globally-unique user by username.
    ///
    /// Backends must fail closed if the username exists in multiple tenants.
    async fn get_user(&self, username: &str) -> Result<Option<User>>;

    /// Get a user by username within a tenant.
    async fn get_user_in_tenant(
        &self,
        username: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<User>>;

    /// List all users that share a username across tenants.
    async fn list_users_by_username(&self, username: &str) -> Result<Vec<User>>;

    /// Create or update a user.
    async fn put_user(&self, user: User) -> Result<()>;

    /// Delete a globally-unique user by username.
    ///
    /// Backends must fail closed if the username exists in multiple tenants.
    async fn delete_user(&self, username: &str) -> Result<()>;

    /// Delete a user by username within a tenant.
    async fn delete_user_in_tenant(
        &self,
        username: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<()>;

    /// List all users.
    async fn list_users(&self) -> Result<Vec<User>>;

    /// Get a user by their ID.
    ///
    /// Default implementation iterates `list_users()`. Backends should override
    /// this with a direct query for better performance.
    async fn get_user_by_id(&self, user_id: &crate::UserId) -> Result<Option<User>> {
        let users = self.list_users().await?;
        Ok(users.into_iter().find(|u| u.id == *user_id))
    }

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

/// Storage backend for tenants.
#[async_trait]
pub trait TenantStore: Send + Sync {
    /// Get a tenant by ID.
    async fn get_tenant(&self, id: &crate::TenantId) -> Result<Option<crate::Tenant>>;

    /// Get a tenant by display name (for uniqueness checks on the admin UI).
    ///
    /// NOTE: Tenant names are human-readable and SHOULD NOT be used as SIP
    /// routing identifiers. Use [`get_tenant_by_sip_domain`] for SIP lookup.
    async fn get_tenant_by_name(&self, name: &str) -> Result<Option<crate::Tenant>>;

    /// Get a tenant that claims the given SIP domain.
    ///
    /// Matches `domain` (case-insensitive, whitespace-trimmed) against every
    /// tenant's `sip_domains` array. The application layer enforces that no
    /// two tenants claim the same domain, so this returns at most one tenant.
    /// Returns `Ok(None)` when no tenant owns the domain.
    ///
    /// Implementations should use an index (e.g. Postgres GIN on the
    /// `sip_domains` column) so that the lookup is O(log n) in the number
    /// of tenants.
    async fn get_tenant_by_sip_domain(&self, domain: &str) -> Result<Option<crate::Tenant>>;

    /// List all tenants.
    async fn list_tenants(&self) -> Result<Vec<crate::Tenant>>;

    /// Create or update a tenant.
    async fn put_tenant(&self, tenant: crate::Tenant) -> Result<()>;

    /// Delete a tenant by ID.
    async fn delete_tenant(&self, id: &crate::TenantId) -> Result<()>;
}

/// Storage backend for SIP registrations.
#[async_trait]
pub trait RegistrationStore: Send + Sync {
    /// Get all registrations for an Address-of-Record.
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>>;

    /// Get all registrations for an Address-of-Record scoped to a tenant.
    async fn get_registrations_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
    ) -> Result<Vec<Registration>>;

    /// Add or update a registration.
    async fn put_registration(&self, registration: Registration) -> Result<()>;

    /// Remove a specific registration by Contact URI.
    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()>;

    /// Remove a specific registration by Contact URI within a tenant.
    async fn delete_registration_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
        contact: &str,
    ) -> Result<()>;

    /// Remove all registrations for an AOR.
    async fn delete_all_registrations(&self, aor: &str) -> Result<()>;

    /// Remove all registrations for an AOR within a tenant.
    async fn delete_all_registrations_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
    ) -> Result<()>;

    /// Remove expired registrations.
    async fn cleanup_expired(&self) -> Result<usize>;

    /// Count total active registrations across all AORs.
    async fn registration_count(&self) -> Result<usize> {
        Ok(0)
    }

    /// Get all active registrations across all AORs.
    async fn get_all_registrations(&self) -> Result<Vec<Registration>> {
        Ok(Vec::new())
    }

    /// Get all active registrations for a tenant.
    async fn get_all_registrations_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<Registration>> {
        let registrations = self.get_all_registrations().await?;
        Ok(registrations
            .into_iter()
            .filter(|registration| registration.tenant_id == *tenant_id)
            .collect())
    }
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

/// Aggregated CDR usage statistics for a tenant within a time range.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageAggregation {
    /// Total number of calls (all dispositions).
    pub total_calls: u64,
    /// Number of answered (connected) calls.
    pub answered_calls: u64,
    /// Sum of call_duration_ms across all answered calls.
    pub total_duration_ms: u64,
    /// Number of inbound calls (to_uri matches a tenant number).
    pub inbound_calls: u64,
    /// Number of outbound calls.
    pub outbound_calls: u64,
    /// Sum of rated CDR costs in the period (0.0 when billing is not
    /// configured). All of a tenant's rated calls share the currency of the
    /// tenant's default rate plan.
    #[serde(default)]
    pub total_cost: f64,
}

/// Per-number CDR usage breakdown.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NumberUsage {
    /// The canonical tenant-owned E.164 phone number.
    pub number: String,
    /// Total calls involving this number.
    pub total_calls: u64,
    /// Answered calls involving this number.
    pub answered_calls: u64,
    /// Total duration in milliseconds.
    pub total_duration_ms: u64,
    /// Inbound calls to this number.
    pub inbound_calls: u64,
    /// Outbound calls from this number.
    pub outbound_calls: u64,
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

    /// Aggregate CDR usage statistics for a tenant within a time range.
    async fn aggregate_usage(
        &self,
        tenant_id: &crate::TenantId,
        from: SystemTime,
        to: SystemTime,
    ) -> Result<UsageAggregation> {
        let _ = (tenant_id, from, to);
        Ok(UsageAggregation::default())
    }

    /// Aggregate CDR usage statistics per phone number for a tenant within a time range.
    async fn aggregate_usage_by_number(
        &self,
        tenant_id: &crate::TenantId,
        from: SystemTime,
        to: SystemTime,
    ) -> Result<Vec<NumberUsage>> {
        let _ = (tenant_id, from, to);
        Ok(Vec::new())
    }

    // ── Rate plans (billing MVP) ─────────────────────────────────────────
    // Defaulted so existing backends and test doubles keep compiling; the
    // defaults fail loudly rather than pretending to persist anything.

    /// Create a rate plan. When `plan.is_default` is set, any previous
    /// default plan for the tenant loses its default flag.
    async fn create_rate_plan(&self, plan: crate::billing::RatePlan) -> Result<()> {
        let _ = plan;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Get a rate plan by id, scoped to a tenant.
    async fn get_rate_plan(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<Option<crate::billing::RatePlan>> {
        let _ = (tenant_id, plan_id);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// List a tenant's rate plans.
    async fn list_rate_plans(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::billing::RatePlan>> {
        let _ = tenant_id;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Replace a rate plan's mutable fields (name, currency, rates config,
    /// default/enabled flags). When `plan.is_default` is set, any previous
    /// default plan for the tenant loses its default flag.
    async fn update_rate_plan(&self, plan: crate::billing::RatePlan) -> Result<()> {
        let _ = plan;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Delete a rate plan and its rates.
    async fn delete_rate_plan(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<()> {
        let _ = (tenant_id, plan_id);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Add or replace a rate on a plan (upsert on `(rate_plan_id, prefix)`).
    async fn upsert_rate(
        &self,
        tenant_id: &crate::TenantId,
        rate: crate::billing::Rate,
    ) -> Result<()> {
        let _ = (tenant_id, rate);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// List a plan's rates.
    async fn list_rates(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<Vec<crate::billing::Rate>> {
        let _ = (tenant_id, plan_id);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Delete one rate from a plan.
    async fn delete_rate(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
        rate_id: uuid::Uuid,
    ) -> Result<()> {
        let _ = (tenant_id, plan_id, rate_id);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "rate plans are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// The tenant's enabled default rate plan with its rates, if any — the
    /// single lookup the CDR rating path performs at call end.
    async fn get_default_rate_plan_with_rates(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<(crate::billing::RatePlan, Vec<crate::billing::Rate>)>> {
        // Default: billing not configured — rating is skipped silently.
        let _ = tenant_id;
        Ok(None)
    }

    // ── Balances (billing follow-up) ─────────────────────────────────────

    /// The tenant's balance row, if one has been created. `None` means no
    /// balance is tracked for the tenant: settlement skips the debit and
    /// admission never blocks.
    async fn get_balance(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<crate::billing::TenantBalance>> {
        let _ = tenant_id;
        Ok(None)
    }

    /// Create or replace the balance *settings* (currency, credit limit,
    /// enforcement, low-balance threshold). The stored `balance` amount is
    /// preserved for an existing row and taken from `balance.balance` only
    /// when the row is created.
    async fn upsert_balance(&self, balance: crate::billing::TenantBalance) -> Result<()> {
        let _ = balance;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "balances are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// Atomically apply a signed amount to the tenant's balance and append
    /// the ledger entry, returning that entry (with `balance_after`). When
    /// the tenant has no balance row yet, one is created with default
    /// settings (enforcement off) in the request's currency.
    async fn apply_balance_transaction(
        &self,
        request: crate::billing::BalanceTransactionRequest,
    ) -> Result<crate::billing::BalanceTransaction> {
        let _ = request;
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "balances are not supported by this storage backend".to_string(),
            ),
        )))
    }

    /// The tenant's ledger, newest first.
    async fn list_balance_transactions(
        &self,
        tenant_id: &crate::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::billing::BalanceTransaction>> {
        let _ = (tenant_id, limit, offset);
        Err(anyhow::Error::new(crate::Error::Storage(
            crate::error::StorageError::Database(
                "balances are not supported by this storage backend".to_string(),
            ),
        )))
    }
}

/// Query parameters for searching available pool phone numbers.
#[derive(Debug, Clone, Default)]
pub struct AvailableNumberQuery {
    /// Filter by area code (NPA)
    pub area_code: Option<String>,
    /// Glob-style pattern match on E.164 (e.g., "*7034")
    pub pattern: Option<String>,
    /// Filter by state/region
    pub region: Option<String>,
    /// Filter by rate center
    pub rate_center: Option<String>,
    /// Maximum results to return (default 50)
    pub limit: usize,
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

    /// Atomically purchase an available pool number for a tenant.
    ///
    /// Returns `Ok(Some(number))` when the number was claimed, `Ok(None)` when
    /// it was missing or no longer available, and `Err(...)` on storage errors.
    async fn purchase_available_phone_number(
        &self,
        id: &crate::PhoneNumberId,
        tenant_id: &crate::TenantId,
        acquired_at: SystemTime,
    ) -> Result<Option<crate::PhoneNumber>> {
        let _ = (id, tenant_id, acquired_at);
        Ok(None)
    }

    /// Delete a phone number by ID
    async fn delete_phone_number(&self, id: &crate::PhoneNumberId) -> Result<()>;

    /// Search available (pool) phone numbers matching filters.
    async fn search_available_numbers(
        &self,
        query: AvailableNumberQuery,
    ) -> Result<Vec<crate::PhoneNumber>> {
        let _ = query;
        Ok(vec![])
    }
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

    /// Get a device by MAC address (for auto-provisioning).
    async fn get_device_by_mac_address(&self, mac_address: &str) -> Result<Option<crate::Device>> {
        let _ = mac_address;
        Ok(None)
    }

    /// Create or update a device
    async fn put_device(&self, device: crate::Device) -> Result<()>;

    /// Delete a device by ID
    async fn delete_device(&self, id: &crate::DeviceId) -> Result<()>;
}

/// SIP trunk storage operations.
#[async_trait::async_trait]
pub trait TrunkStore: Send + Sync {
    /// Get a trunk by ID
    async fn get_trunk(&self, id: &crate::TrunkId) -> Result<Option<crate::Trunk>>;

    /// List all trunks for a tenant
    async fn list_trunks(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Trunk>>;

    /// Create or update a trunk
    async fn put_trunk(&self, trunk: crate::Trunk) -> Result<()>;

    /// Delete a trunk by ID
    async fn delete_trunk(&self, id: &crate::TrunkId) -> Result<()>;

    /// Get aggregated stats for a trunk from CDRs (last 24h).
    async fn get_trunk_stats(&self, trunk_name: &str) -> Result<crate::TrunkStats> {
        // Default: return empty stats (override in implementations with CDR access)
        let _ = trunk_name;
        Ok(crate::TrunkStats::default())
    }
}

/// Routing rule storage operations.
#[async_trait::async_trait]
pub trait RoutingRuleStore: Send + Sync {
    async fn get_routing_rule(
        &self,
        id: &crate::RoutingRuleId,
    ) -> Result<Option<crate::RoutingRule>>;
    async fn list_routing_rules(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::RoutingRule>>;
    async fn put_routing_rule(&self, rule: crate::RoutingRule) -> Result<()>;
    async fn delete_routing_rule(&self, id: &crate::RoutingRuleId) -> Result<()>;
}

/// Queue configuration storage operations.
#[async_trait::async_trait]
pub trait QueueConfigStore: Send + Sync {
    /// Get a queue config by ID
    async fn get_queue_config(
        &self,
        id: &crate::QueueConfigId,
    ) -> Result<Option<crate::QueueConfigRecord>>;

    /// List all queue configs for a tenant
    async fn list_queue_configs(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::QueueConfigRecord>>;

    /// Create or update a queue config
    async fn put_queue_config(&self, config: crate::QueueConfigRecord) -> Result<()>;

    /// Delete a queue config by ID
    async fn delete_queue_config(&self, id: &crate::QueueConfigId) -> Result<()>;
}

/// Storage for device provisioning configuration templates.
#[async_trait]
pub trait ConfigTemplateStore: Send + Sync {
    /// Get a config template by ID.
    async fn get_config_template(
        &self,
        id: &crate::ConfigTemplateId,
    ) -> Result<Option<crate::config_template::ConfigTemplate>>;

    /// List all config templates for a tenant.
    async fn list_config_templates(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::config_template::ConfigTemplate>>;

    /// Create or update a config template.
    async fn put_config_template(
        &self,
        template: crate::config_template::ConfigTemplate,
    ) -> Result<()>;

    /// Delete a config template by ID.
    async fn delete_config_template(&self, id: &crate::ConfigTemplateId) -> Result<()>;
}

/// Site location storage operations.
#[async_trait::async_trait]
pub trait SiteLocationStore: Send + Sync {
    /// Get a location by ID
    async fn get_location(&self, id: &crate::LocationId) -> Result<Option<crate::SiteLocation>>;

    /// List all locations for a tenant
    async fn list_locations(&self, tenant_id: &crate::TenantId)
        -> Result<Vec<crate::SiteLocation>>;

    /// Create or update a location
    async fn put_location(&self, location: crate::SiteLocation) -> Result<()>;

    /// Delete a location by ID
    async fn delete_location(&self, id: &crate::LocationId) -> Result<()>;
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
    async fn list_messages(&self, mailbox_id: &str) -> Result<Vec<crate::VoicemailMessage>>;

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
    async fn put_phone_number_location(&self, location: crate::PhoneNumberLocation) -> Result<()>;

    /// Delete phone number location
    async fn delete_phone_number_location(
        &self,
        phone_number_id: &crate::PhoneNumberId,
    ) -> Result<()>;
}

/// Persisted agent state for queue system restart recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAgent {
    pub id: String,
    pub extension: String,
    pub queues: Vec<String>,
    pub skills: Vec<String>,
    pub priority: u32,
    /// Agent state: "available", "away", "paused", "logged_out"
    pub state: String,
    /// Per-queue priority overrides (queue_name → priority).
    #[serde(default)]
    pub queue_priorities: HashMap<String, u32>,
    /// Maximum concurrent calls this agent can handle. Default 1.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_calls: u32,
}

fn default_max_concurrent() -> u32 {
    1
}

/// A timestamped snapshot of queue statistics for historical analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredQueueSnapshot {
    pub queue_name: String,
    /// Unix timestamp (seconds since epoch).
    pub timestamp_epoch: u64,
    pub calls_waiting: usize,
    pub agents_available: usize,
    pub agents_on_call: usize,
    pub agents_total: usize,
    pub avg_wait_time_secs: f64,
    pub longest_wait_secs: u64,
    pub calls_handled_today: u64,
    pub calls_abandoned_today: u64,
    pub service_level_pct: f64,
}

/// Persistence for ACD queue agent state.
#[async_trait]
pub trait QueueStore: Send + Sync {
    /// Save all logged-in agents for a tenant.
    async fn save_agents(
        &self,
        tenant_id: &crate::TenantId,
        agents: Vec<StoredAgent>,
    ) -> Result<()>;

    /// Load all agents for a tenant.
    async fn load_agents(&self, tenant_id: &crate::TenantId) -> Result<Vec<StoredAgent>>;

    /// Remove an agent record.
    async fn remove_agent(&self, tenant_id: &crate::TenantId, agent_id: &str) -> Result<()>;

    /// Clear all agent records for a tenant.
    async fn clear_agents(&self, tenant_id: &crate::TenantId) -> Result<()>;

    /// Save a queue stats snapshot for historical analysis.
    /// Default implementation is a no-op for stores that don't support it.
    async fn save_stats_snapshot(
        &self,
        _tenant_id: &crate::TenantId,
        _snapshot: StoredQueueSnapshot,
    ) -> Result<()> {
        Ok(())
    }

    /// Query historical stats snapshots for a queue within a time range.
    /// Returns snapshots ordered by timestamp ascending.
    async fn query_stats_snapshots(
        &self,
        _tenant_id: &crate::TenantId,
        _queue_name: &str,
        _from_epoch: Option<u64>,
        _to_epoch: Option<u64>,
        _limit: usize,
    ) -> Result<Vec<StoredQueueSnapshot>> {
        Ok(Vec::new())
    }
}

/// Storage backend for conference meetings.
#[async_trait::async_trait]
pub trait ConferenceStore: Send + Sync {
    /// Get a meeting by ID
    async fn get_meeting(&self, id: &crate::MeetingId) -> Result<Option<crate::Meeting>>;

    /// Get a meeting by room number within a tenant
    async fn get_meeting_by_room_number(
        &self,
        tenant_id: &crate::TenantId,
        room_number: &str,
    ) -> Result<Option<crate::Meeting>>;

    /// List all meetings for a tenant
    async fn list_meetings(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Meeting>>;

    /// List meetings owned by a specific user within a tenant
    async fn list_meetings_by_owner(
        &self,
        tenant_id: &crate::TenantId,
        owner_id: &str,
    ) -> Result<Vec<crate::Meeting>>;

    /// Create or update a meeting
    async fn put_meeting(&self, meeting: crate::Meeting) -> Result<()>;

    /// Delete a meeting by ID
    async fn delete_meeting(&self, id: &crate::MeetingId) -> Result<()>;
}

/// Check whether a meeting room number already exists across **all** tenants.
///
/// The conference bridge keys rooms globally by room number, so each room
/// number must be unique across tenants. This helper centralises the
/// cross-tenant uniqueness check used by both the conference-server HTTP API
/// and the admin API.
///
/// * `storage` — any storage that implements both [`ConferenceStore`] and
///   [`TenantStore`].
/// * `default_tenant_id` — always included in the tenant sweep so that the
///   default tenant is checked even when it has no row in `list_tenants`.
/// * `room_number` — the candidate room number to search for.
/// * `exclude_tenant` — when updating an existing meeting, pass
///   `Some(meeting.tenant_id)` to avoid a false positive against the meeting
///   being updated.
///
/// Returns `Ok(true)` as soon as a matching room is found.
pub async fn room_number_exists_globally<S>(
    storage: &S,
    default_tenant_id: &crate::TenantId,
    room_number: &str,
    exclude_tenant: Option<crate::TenantId>,
) -> Result<bool>
where
    S: ConferenceStore + TenantStore,
{
    use std::collections::HashSet;
    let mut tenant_ids: HashSet<crate::TenantId> = HashSet::from([*default_tenant_id]);
    for tenant in storage.list_tenants().await? {
        tenant_ids.insert(tenant.id);
    }
    for tenant_id in tenant_ids {
        if exclude_tenant == Some(tenant_id) {
            continue;
        }
        if storage
            .get_meeting_by_room_number(&tenant_id, room_number)
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Persisted presentity state for restart recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPresentity {
    /// Address of Record (user@domain)
    pub aor: String,
    /// Presence state as string (e.g. "open", "busy")
    pub state: String,
    /// Optional status note
    pub note: Option<String>,
    /// Tenant ID for multi-tenancy
    pub tenant_id: crate::TenantId,
    /// Unix epoch seconds when this was last updated
    pub updated_at_epoch: u64,
}

/// Persisted presence subscription for restart recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSubscription {
    pub subscription_id: String,
    pub watcher_aor: String,
    pub presentity_aor: String,
    pub tenant_id: crate::TenantId,
    pub contact_uri: String,
    pub event_package: String,
    /// Unix epoch seconds when this subscription expires (Instant is not serializable).
    pub expires_at_epoch: u64,
    /// Call-ID from SUBSCRIBE (for in-dialog NOTIFY).
    pub sip_call_id: Option<String>,
    /// From-tag from SUBSCRIBE (becomes To-tag in NOTIFY).
    pub sip_from_tag: Option<String>,
    /// To-tag from our 200 OK (becomes From-tag in NOTIFY).
    pub sip_to_tag: Option<String>,
}

/// Persisted presence authorization policy for restart recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuthPolicy {
    pub aor: String,
    pub default_allow: bool,
    /// JSON-encoded Vec<String> of allowed watcher AORs
    pub allow_list_json: String,
    /// JSON-encoded Vec<String> of blocked watcher AORs
    pub block_list_json: String,
    pub tenant_id: crate::TenantId,
    /// JSON-encoded HashMap<String, HashSet<String>> for per-event allow lists
    #[serde(default)]
    pub event_allow_json: String,
    /// JSON-encoded HashMap<String, HashSet<String>> for per-event block lists
    #[serde(default)]
    pub event_block_json: String,
    /// JSON-encoded Vec<TimeWindow> for time-based restrictions
    #[serde(default)]
    pub time_windows_json: String,
    /// JSON-encoded Vec<String> for domain wildcard allow patterns
    #[serde(default)]
    pub allow_patterns_json: String,
    /// JSON-encoded Vec<String> for domain wildcard block patterns
    #[serde(default)]
    pub block_patterns_json: String,
}

/// Persistence for presence watcher subscriptions and presentity state.
#[async_trait]
pub trait PresenceStore: Send + Sync {
    /// Save or update a subscription.
    async fn save_subscription(&self, sub: StoredSubscription) -> Result<()>;

    /// Load all non-expired subscriptions for a tenant.
    async fn load_subscriptions(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<StoredSubscription>>;

    /// Remove a single subscription by ID.
    async fn remove_subscription(&self, subscription_id: &str) -> Result<()>;

    /// Remove all subscriptions for a presentity AOR within a tenant.
    async fn remove_subscriptions_for_presentity(
        &self,
        tenant_id: &crate::TenantId,
        presentity_aor: &str,
    ) -> Result<()>;

    /// Delete expired subscriptions and return how many were removed.
    async fn cleanup_expired_subscriptions(&self) -> Result<usize>;

    /// Save or update a presentity's state.
    async fn save_presentity(&self, presentity: StoredPresentity) -> Result<()> {
        let _ = presentity;
        Ok(())
    }

    /// Load all presentities for a tenant.
    async fn load_presentities(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<StoredPresentity>> {
        let _ = tenant_id;
        Ok(Vec::new())
    }

    /// Remove a presentity by AOR within a tenant.
    async fn remove_presentity(&self, tenant_id: &crate::TenantId, aor: &str) -> Result<()> {
        let _ = tenant_id;
        let _ = aor;
        Ok(())
    }

    /// Save or update an auth policy for a presentity.
    async fn save_auth_policy(&self, policy: StoredAuthPolicy) -> Result<()> {
        let _ = policy;
        Ok(())
    }

    /// Load all auth policies for a tenant.
    async fn load_auth_policies(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<StoredAuthPolicy>> {
        let _ = tenant_id;
        Ok(vec![])
    }

    /// Remove an auth policy by AOR within a tenant.
    async fn remove_auth_policy(&self, tenant_id: &crate::TenantId, aor: &str) -> Result<()> {
        let _ = tenant_id;
        let _ = aor;
        Ok(())
    }
}

/// Persisted resource list for RFC 4826 Resource List Subscriptions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredResourceList {
    /// The resource list URI (e.g., `sip:my-team@example.com`).
    pub uri: String,
    /// Optional display name.
    pub name: Option<String>,
    /// JSON-encoded array of `{uri, display_name}` entries.
    pub entries_json: String,
    /// Tenant ID for multi-tenancy.
    pub tenant_id: crate::TenantId,
}

/// Storage backend for RFC 4826 resource lists.
#[async_trait]
pub trait ResourceListStore: Send + Sync {
    /// Save or update a resource list.
    async fn save_resource_list(&self, list: &StoredResourceList) -> Result<()>;

    /// Get a resource list by tenant and URI.
    async fn get_resource_list(
        &self,
        tenant_id: &crate::TenantId,
        uri: &str,
    ) -> Result<Option<StoredResourceList>>;

    /// Get all resource lists for a tenant.
    async fn get_resource_lists_by_tenant(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<StoredResourceList>>;

    /// Delete a resource list by tenant and URI.
    async fn delete_resource_list(&self, tenant_id: &crate::TenantId, uri: &str) -> Result<()>;
}

/// A stored presence state change for analytics/history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPresenceChange {
    /// Unique ID for this change record.
    pub id: String,
    /// Address of Record (user@domain).
    pub aor: String,
    /// Previous presence state.
    pub old_state: String,
    /// New presence state.
    pub new_state: String,
    /// Tenant ID for multi-tenancy.
    pub tenant_id: crate::TenantId,
    /// Unix epoch seconds when this change occurred.
    pub timestamp_epoch: u64,
}

/// Summary of presence activity for an AOR over a time range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceSummary {
    /// Address of Record.
    pub aor: String,
    /// Total number of state changes in the period.
    pub total_changes: u64,
    /// Seconds spent in Open state.
    pub time_in_open_secs: u64,
    /// Seconds spent in Busy state.
    pub time_in_busy_secs: u64,
    /// Seconds spent in Away state.
    pub time_in_away_secs: u64,
    /// Seconds spent in Closed state.
    pub time_in_closed_secs: u64,
}

/// Storage backend for presence history/analytics.
#[async_trait]
pub trait PresenceHistoryStore: Send + Sync {
    /// Record a presence state change.
    async fn record_change(&self, change: StoredPresenceChange) -> Result<()>;

    /// Query history for a specific AOR within a tenant and time range.
    async fn query_history(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        limit: usize,
    ) -> Result<Vec<StoredPresenceChange>>;

    /// Get a summary of presence activity across all AORs for a tenant.
    async fn query_summary(
        &self,
        tenant_id: &crate::TenantId,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
    ) -> Result<Vec<PresenceSummary>>;
}

/// A registered webhook endpoint for receiving platform event notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEndpoint {
    /// Unique webhook endpoint identifier
    pub id: crate::WebhookId,
    /// Tenant ID for multi-tenancy
    pub tenant_id: crate::TenantId,
    /// HTTP(S) URL to POST events to
    pub url: String,
    /// HMAC-SHA256 secret for signing payloads
    pub secret: String,
    /// Event type filter (e.g., `["call.initiated", "voicemail.new_message"]`). Empty = all events.
    pub events: Vec<String>,
    /// Whether this endpoint is currently enabled
    pub enabled: bool,
    /// Optional description for this endpoint
    pub description: Option<String>,
    /// Creation timestamp
    #[serde(with = "system_time_serde")]
    pub created_at: SystemTime,
    /// Last modified timestamp
    #[serde(with = "system_time_serde")]
    pub updated_at: SystemTime,
}

/// A delivery log entry for a webhook invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDeliveryLog {
    /// Unique delivery log ID (UUID string)
    pub id: String,
    /// The webhook endpoint this delivery is for
    pub endpoint_id: crate::WebhookId,
    /// The event type that triggered this delivery
    pub event_type: String,
    /// The serialized JSON payload
    pub payload: String,
    /// HTTP status code from the endpoint (if received)
    pub status_code: Option<u16>,
    /// Number of delivery attempts so far
    pub attempts: u32,
    /// When the last attempt was made
    #[serde(with = "optional_system_time_serde")]
    pub last_attempt_at: Option<SystemTime>,
    /// When the next retry is scheduled
    #[serde(with = "optional_system_time_serde")]
    pub next_retry_at: Option<SystemTime>,
    /// Whether the delivery was successful
    pub delivered: bool,
    /// Error message from the last failed attempt
    pub error: Option<String>,
    /// HTTP response body from the endpoint (truncated to 64KB)
    #[serde(default)]
    pub response_body: Option<String>,
    /// HTTP response headers (JSON-serialized map)
    #[serde(default)]
    pub response_headers: Option<String>,
    /// When this delivery log was created
    #[serde(with = "system_time_serde")]
    pub created_at: SystemTime,
}

/// Storage backend for webhook endpoints and delivery logs.
#[async_trait]
pub trait WebhookStore: Send + Sync {
    /// Create a new webhook endpoint.
    async fn create_endpoint(&self, endpoint: WebhookEndpoint) -> Result<()>;

    /// Get a webhook endpoint by ID.
    async fn get_endpoint(&self, id: &crate::WebhookId) -> Result<Option<WebhookEndpoint>>;

    /// List all webhook endpoints for a tenant.
    async fn list_endpoints(&self, tenant_id: &crate::TenantId) -> Result<Vec<WebhookEndpoint>>;

    /// Update an existing webhook endpoint.
    async fn update_endpoint(&self, endpoint: WebhookEndpoint) -> Result<()>;

    /// Delete a webhook endpoint by ID.
    async fn delete_endpoint(&self, id: &crate::WebhookId) -> Result<()>;

    /// Log a webhook delivery attempt.
    async fn log_delivery(&self, log: WebhookDeliveryLog) -> Result<()>;

    /// Get pending retries (undelivered, next_retry_at <= now, ordered by next_retry_at).
    async fn get_pending_retries(&self, limit: usize) -> Result<Vec<WebhookDeliveryLog>>;

    /// Update a delivery log entry (e.g., after retry attempt).
    async fn update_delivery_log(&self, log: WebhookDeliveryLog) -> Result<()>;

    /// Get recent delivery logs for an endpoint.
    async fn get_delivery_logs(
        &self,
        endpoint_id: &crate::WebhookId,
        limit: usize,
    ) -> Result<Vec<WebhookDeliveryLog>>;

    /// Get a single delivery log by ID.
    async fn get_delivery_log(&self, id: &str) -> Result<Option<WebhookDeliveryLog>> {
        // Default implementation returns None — override in Postgres backend.
        let _ = id;
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// API Tokens
// ---------------------------------------------------------------------------

/// An API token for programmatic access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    /// Unique token identifier
    pub id: crate::ApiTokenId,
    /// Human-readable name for this token
    pub name: String,
    /// SHA-256 hash of the secret (hex-encoded)
    pub token_hash: String,
    /// First 8 characters of the plaintext token for display
    pub prefix: String,
    /// Owner user ID (may be empty for system tokens)
    pub user_id: String,
    /// Tenant this token belongs to
    pub tenant_id: crate::TenantId,
    /// Permissions granted to this token
    pub permissions: Vec<String>,
    /// When this token expires (None = never)
    #[serde(with = "optional_system_time_serde")]
    pub expires_at: Option<SystemTime>,
    /// When this token was created
    #[serde(with = "system_time_serde")]
    pub created_at: SystemTime,
    /// When this token was last used
    #[serde(with = "optional_system_time_serde")]
    pub last_used_at: Option<SystemTime>,
}

/// Storage backend for API tokens.
#[async_trait]
pub trait ApiTokenStore: Send + Sync {
    /// Store a new API token.
    async fn create_api_token(&self, token: ApiToken) -> Result<()>;

    /// List all API tokens for a tenant.
    async fn list_api_tokens(&self, tenant_id: &crate::TenantId) -> Result<Vec<ApiToken>>;

    /// Look up an API token by ID.
    async fn get_api_token(&self, id: &crate::ApiTokenId) -> Result<Option<ApiToken>>;

    /// Look up an API token by its hash (for authentication).
    async fn get_api_token_by_hash(&self, token_hash: &str) -> Result<Option<ApiToken>>;

    /// Delete an API token by ID.
    async fn delete_api_token(&self, id: &crate::ApiTokenId) -> Result<()>;

    /// Update the last_used_at timestamp.
    async fn update_last_used(&self, id: &crate::ApiTokenId) -> Result<()>;
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

// ---------------------------------------------------------------------------
// Cluster parking (cross-node park/pickup)
// ---------------------------------------------------------------------------

/// A parked call entry stored in the cluster (Redis) for cross-node pickup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterParkedCall {
    /// Internal call ID (string form of fcp_b2bua_core::CallId).
    pub call_id: String,
    /// Node that owns the parked A-leg.
    pub node_id: String,
    /// Extension of the user who parked the call.
    pub parker_extension: String,
    /// Tenant ID (string form).
    pub tenant_id: String,
    /// Unix timestamp (seconds) when the call was parked.
    pub parked_at_unix: u64,
}

/// Distributed parking store for cross-node park/pickup.
///
/// The local `ParkingManager` (in-memory DashMap) handles the primary parking
/// state on each node. This trait provides a cluster-wide secondary store
/// (backed by Redis) so that any node can discover parked calls and route
/// cross-node pickups.
#[async_trait]
pub trait ClusterParkingStore: Send + Sync {
    /// Store a parked call in the cluster.
    async fn cluster_park(
        &self,
        orbit: u16,
        info: ClusterParkedCall,
        timeout_secs: u64,
    ) -> Result<()>;

    /// Look up a parked call by orbit across the cluster.
    async fn cluster_lookup(&self, orbit: u16) -> Result<Option<ClusterParkedCall>>;

    /// Remove a parked call from the cluster store. Returns it if it existed.
    async fn cluster_unpark(&self, orbit: u16) -> Result<Option<ClusterParkedCall>>;
}

/// In-memory storage with JSON file persistence.
///
/// **Note:** This implementation is intended for development and testing only.
/// For production deployments, use the PostgreSQL backend (fcp-storage-postgres).
///
/// This implementation keeps data in memory and saves to JSON files. It does not
/// persist CDRs (Call Detail Records) - they are discarded. For full CDR support
/// and ACID compliance, use the PostgreSQL backend (fcp-storage-postgres).
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
    fn user_key(tenant_id: &crate::TenantId, username: &str) -> String {
        format!("{}:{}", tenant_id, username)
    }

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
            Ok(users
                .into_iter()
                .map(|u| (Self::user_key(&u.tenant_id, &u.username), u))
                .collect())
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
        let mut matches = users
            .values()
            .filter(|user| user.username == username)
            .cloned()
            .collect::<Vec<_>>();

        if matches.len() > 1 {
            anyhow::bail!("username '{}' exists in multiple tenants", username);
        }

        Ok(matches.pop())
    }

    async fn get_user_in_tenant(
        &self,
        username: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<User>> {
        let users = self.users.read().await;
        Ok(users.get(&Self::user_key(tenant_id, username)).cloned())
    }

    async fn list_users_by_username(&self, username: &str) -> Result<Vec<User>> {
        let users = self.users.read().await;
        Ok(users
            .values()
            .filter(|user| user.username == username)
            .cloned()
            .collect())
    }

    async fn put_user(&self, user: User) -> Result<()> {
        let mut users = self.users.write().await;
        users.insert(Self::user_key(&user.tenant_id, &user.username), user);
        drop(users);

        // Save to file
        self.save_users().await?;
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        let mut users = self.users.write().await;
        let matching_keys = users
            .iter()
            .filter(|(_, user)| user.username == username)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();

        if matching_keys.len() > 1 {
            anyhow::bail!("username '{}' exists in multiple tenants", username);
        }

        if let Some(key) = matching_keys.first() {
            users.remove(key);
        }
        drop(users);

        // Save to file
        self.save_users().await?;
        Ok(())
    }

    async fn delete_user_in_tenant(
        &self,
        username: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<()> {
        let mut users = self.users.write().await;
        users.remove(&Self::user_key(tenant_id, username));
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

    async fn get_registrations_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
    ) -> Result<Vec<Registration>> {
        let registrations = self.registrations.read().await;
        Ok(registrations
            .get(aor)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|registration| registration.tenant_id == *tenant_id)
            .collect())
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        let mut registrations = self.registrations.write().await;
        let aor_regs = registrations.entry(registration.aor.clone()).or_default();

        // Remove existing registration with the same contact in the same tenant
        aor_regs.retain(|r| {
            !(r.contact == registration.contact && r.tenant_id == registration.tenant_id)
        });

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

    async fn delete_registration_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
        contact: &str,
    ) -> Result<()> {
        let mut registrations = self.registrations.write().await;
        if let Some(aor_regs) = registrations.get_mut(aor) {
            aor_regs.retain(|r| !(r.contact == contact && r.tenant_id == *tenant_id));
            if aor_regs.is_empty() {
                registrations.remove(aor);
            }
        }
        drop(registrations);

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

    async fn delete_all_registrations_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
    ) -> Result<()> {
        let mut registrations = self.registrations.write().await;
        if let Some(aor_regs) = registrations.get_mut(aor) {
            aor_regs.retain(|registration| registration.tenant_id != *tenant_id);
            if aor_regs.is_empty() {
                registrations.remove(aor);
            }
        }
        drop(registrations);

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

    async fn registration_count(&self) -> Result<usize> {
        let regs = self.registrations.read().await;
        Ok(regs.values().map(|v| v.len()).sum())
    }

    async fn get_all_registrations(&self) -> Result<Vec<Registration>> {
        let regs = self.registrations.read().await;
        let now = SystemTime::now();
        Ok(regs
            .values()
            .flatten()
            .filter(|r| r.expires_at > now)
            .cloned()
            .collect())
    }

    async fn get_all_registrations_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<Registration>> {
        let regs = self.registrations.read().await;
        let now = SystemTime::now();
        Ok(regs
            .values()
            .flatten()
            .filter(|registration| {
                registration.expires_at > now && registration.tenant_id == *tenant_id
            })
            .cloned()
            .collect())
    }
}

#[async_trait]
impl CdrStore for FileBackedStorage {
    async fn insert_cdr(&self, _cdr: CallDetailRecord) -> Result<()> {
        // FileBackedStorage doesn't persist CDRs - this is a no-op
        // For production use cases, use the PostgreSQL backend (fcp-storage-postgres)
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
/// This allows the runtime to work with different storage backends (PostgreSQL, Redis,
/// FileBackedStorage) without being tied to a specific implementation at compile time.
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
    tenant_store: Arc<dyn TenantStore>,
    conference_store: Arc<dyn ConferenceStore>,
    queue_store: Arc<dyn QueueStore>,
    presence_store: Arc<dyn PresenceStore>,
    resource_list_store: Arc<dyn ResourceListStore>,
    webhook_store: Arc<dyn WebhookStore>,
    site_location_store: Arc<dyn SiteLocationStore>,
    trunk_store: Arc<dyn TrunkStore>,
    routing_rule_store: Arc<dyn RoutingRuleStore>,
    queue_config_store: Arc<dyn QueueConfigStore>,
    config_store: Option<Arc<dyn crate::config_store::ConfigStore>>,
    sip_trace_store: Option<Arc<dyn crate::sip_trace::SipTraceStore>>,
    api_token_store: Option<Arc<dyn ApiTokenStore>>,
    prompt_store: Option<Arc<dyn crate::prompt::PromptStore>>,
    flow_trace_store: Option<Arc<dyn crate::flow_trace::FlowTraceStore>>,
    config_template_store: Option<Arc<dyn ConfigTemplateStore>>,
    encryption_store: Option<Arc<dyn crate::encryption::EncryptionStore>>,
    emergency_store: Option<Arc<dyn crate::e911::EmergencyStore>>,
}

impl DynStorage {
    /// Create a new DynStorage from individual trait objects.
    ///
    /// This is an aggregate constructor over every store trait, so the argument
    /// count tracks the number of stores rather than any avoidable complexity.
    #[allow(clippy::too_many_arguments)]
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
        tenant_store: Arc<dyn TenantStore>,
        conference_store: Arc<dyn ConferenceStore>,
        queue_store: Arc<dyn QueueStore>,
        presence_store: Arc<dyn PresenceStore>,
        resource_list_store: Arc<dyn ResourceListStore>,
        webhook_store: Arc<dyn WebhookStore>,
        site_location_store: Arc<dyn SiteLocationStore>,
        trunk_store: Arc<dyn TrunkStore>,
        routing_rule_store: Arc<dyn RoutingRuleStore>,
        queue_config_store: Arc<dyn QueueConfigStore>,
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
            tenant_store,
            conference_store,
            queue_store,
            presence_store,
            resource_list_store,
            webhook_store,
            site_location_store,
            trunk_store,
            routing_rule_store,
            queue_config_store,
            config_store: None,
            sip_trace_store: None,
            api_token_store: None,
            prompt_store: None,
            flow_trace_store: None,
            config_template_store: None,
            encryption_store: None,
            emergency_store: None,
        }
    }

    /// Create a new DynStorage from a storage backend that implements all traits.
    ///
    /// This is a convenience method for backends like PostgresStorage that implement
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
            + TenantStore
            + ConferenceStore
            + QueueStore
            + PresenceStore
            + ResourceListStore
            + WebhookStore
            + SiteLocationStore
            + TrunkStore
            + RoutingRuleStore
            + QueueConfigStore
            + ConfigTemplateStore
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
            porting_store: storage.clone(),
            tenant_store: storage.clone(),
            conference_store: storage.clone(),
            queue_store: storage.clone(),
            presence_store: storage.clone(),
            resource_list_store: storage.clone(),
            webhook_store: storage.clone(),
            site_location_store: storage.clone(),
            trunk_store: storage.clone(),
            routing_rule_store: storage.clone(),
            queue_config_store: storage.clone(),
            config_store: None,
            sip_trace_store: None,
            api_token_store: None,
            prompt_store: None,
            flow_trace_store: None,
            config_template_store: Some(storage),
            encryption_store: None,
            emergency_store: None,
        }
    }

    /// Replace the registration store with a different implementation.
    ///
    /// This supports split-backend deployments where registrations are stored
    /// in Redis while other data remains in the primary backend.
    pub fn with_registration_store(mut self, store: Arc<dyn RegistrationStore>) -> Self {
        self.registration_store = store;
        self
    }

    /// Replace the credential store with a different implementation (e.g. cached).
    pub fn with_credential_store(mut self, store: Arc<dyn CredentialStore>) -> Self {
        self.credential_store = store;
        self
    }

    /// Replace the extension store with a different implementation (e.g. cached).
    pub fn with_extension_store(mut self, store: Arc<dyn ExtensionStore>) -> Self {
        self.extension_store = store;
        self
    }

    /// Replace the device store with a different implementation (e.g. cached).
    pub fn with_device_store(mut self, store: Arc<dyn DeviceStore>) -> Self {
        self.device_store = store;
        self
    }

    /// Get the presence store.
    pub fn presence_store(&self) -> &Arc<dyn PresenceStore> {
        &self.presence_store
    }

    /// Get the resource list store.
    pub fn resource_list_store(&self) -> &Arc<dyn ResourceListStore> {
        &self.resource_list_store
    }

    /// Get the webhook store.
    pub fn webhook_store(&self) -> &Arc<dyn WebhookStore> {
        &self.webhook_store
    }

    /// Replace the config store with a specific implementation.
    pub fn with_config_store(mut self, store: Arc<dyn crate::config_store::ConfigStore>) -> Self {
        self.config_store = Some(store);
        self
    }

    /// Get the config store, if one has been set.
    pub fn config_store(&self) -> Option<&Arc<dyn crate::config_store::ConfigStore>> {
        self.config_store.as_ref()
    }

    /// Replace the SIP trace store with a specific implementation.
    pub fn with_sip_trace_store(mut self, store: Arc<dyn crate::sip_trace::SipTraceStore>) -> Self {
        self.sip_trace_store = Some(store);
        self
    }

    /// Get the SIP trace store, if one has been set.
    pub fn sip_trace_store(&self) -> Option<&Arc<dyn crate::sip_trace::SipTraceStore>> {
        self.sip_trace_store.as_ref()
    }

    /// Replace the API token store with a specific implementation.
    pub fn with_api_token_store(mut self, store: Arc<dyn ApiTokenStore>) -> Self {
        self.api_token_store = Some(store);
        self
    }

    /// Get the API token store, if one has been set.
    pub fn api_token_store(&self) -> Option<&Arc<dyn ApiTokenStore>> {
        self.api_token_store.as_ref()
    }

    /// Get a clone of the credential store Arc.
    pub fn credential_store_arc(&self) -> Arc<dyn CredentialStore> {
        Arc::clone(&self.credential_store)
    }

    /// Replace the prompt store with a specific implementation.
    pub fn with_prompt_store(mut self, store: Arc<dyn crate::prompt::PromptStore>) -> Self {
        self.prompt_store = Some(store);
        self
    }

    /// Get the prompt store, if one has been set.
    pub fn prompt_store(&self) -> Option<&Arc<dyn crate::prompt::PromptStore>> {
        self.prompt_store.as_ref()
    }

    /// Replace the flow trace store with a specific implementation.
    pub fn with_flow_trace_store(
        mut self,
        store: Arc<dyn crate::flow_trace::FlowTraceStore>,
    ) -> Self {
        self.flow_trace_store = Some(store);
        self
    }

    /// Get the flow trace store, if one has been set.
    pub fn flow_trace_store(&self) -> Option<&Arc<dyn crate::flow_trace::FlowTraceStore>> {
        self.flow_trace_store.as_ref()
    }

    /// Get the site location store.
    pub fn site_location_store(&self) -> &Arc<dyn SiteLocationStore> {
        &self.site_location_store
    }

    /// Replace the config template store with a specific implementation.
    pub fn with_config_template_store(mut self, store: Arc<dyn ConfigTemplateStore>) -> Self {
        self.config_template_store = Some(store);
        self
    }

    /// Attach the backend's encryption-at-rest management surface.
    pub fn with_encryption_store(
        mut self,
        store: Arc<dyn crate::encryption::EncryptionStore>,
    ) -> Self {
        self.encryption_store = Some(store);
        self
    }

    /// The encryption-at-rest management surface, if the backend has one.
    pub fn encryption_store(&self) -> Option<&Arc<dyn crate::encryption::EncryptionStore>> {
        self.encryption_store.as_ref()
    }

    /// Attach the emergency (E911) configuration and audit store.
    pub fn with_emergency_store(mut self, store: Arc<dyn crate::e911::EmergencyStore>) -> Self {
        self.emergency_store = Some(store);
        self
    }

    pub fn emergency_store(&self) -> Option<&Arc<dyn crate::e911::EmergencyStore>> {
        self.emergency_store.as_ref()
    }

    /// Get the config template store, if one has been set.
    pub fn config_template_store(&self) -> Option<&Arc<dyn ConfigTemplateStore>> {
        self.config_template_store.as_ref()
    }
}

#[async_trait]
impl CredentialStore for DynStorage {
    async fn get_user(&self, username: &str) -> Result<Option<User>> {
        self.credential_store.get_user(username).await
    }

    async fn get_user_in_tenant(
        &self,
        username: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<User>> {
        self.credential_store
            .get_user_in_tenant(username, tenant_id)
            .await
    }

    async fn list_users_by_username(&self, username: &str) -> Result<Vec<User>> {
        self.credential_store.list_users_by_username(username).await
    }

    async fn put_user(&self, user: User) -> Result<()> {
        self.credential_store.put_user(user).await
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        self.credential_store.delete_user(username).await
    }

    async fn delete_user_in_tenant(
        &self,
        username: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<()> {
        self.credential_store
            .delete_user_in_tenant(username, tenant_id)
            .await
    }

    async fn list_users(&self) -> Result<Vec<User>> {
        self.credential_store.list_users().await
    }

    async fn get_user_by_id(&self, user_id: &crate::UserId) -> Result<Option<User>> {
        self.credential_store.get_user_by_id(user_id).await
    }
}

#[async_trait]
impl RegistrationStore for DynStorage {
    async fn get_registrations(&self, aor: &str) -> Result<Vec<Registration>> {
        self.registration_store.get_registrations(aor).await
    }

    async fn get_registrations_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
    ) -> Result<Vec<Registration>> {
        self.registration_store
            .get_registrations_in_tenant(tenant_id, aor)
            .await
    }

    async fn put_registration(&self, registration: Registration) -> Result<()> {
        self.registration_store.put_registration(registration).await
    }

    async fn delete_registration(&self, aor: &str, contact: &str) -> Result<()> {
        self.registration_store
            .delete_registration(aor, contact)
            .await
    }

    async fn delete_registration_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
        contact: &str,
    ) -> Result<()> {
        self.registration_store
            .delete_registration_in_tenant(tenant_id, aor, contact)
            .await
    }

    async fn delete_all_registrations(&self, aor: &str) -> Result<()> {
        self.registration_store.delete_all_registrations(aor).await
    }

    async fn delete_all_registrations_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        aor: &str,
    ) -> Result<()> {
        self.registration_store
            .delete_all_registrations_in_tenant(tenant_id, aor)
            .await
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        self.registration_store.cleanup_expired().await
    }

    async fn registration_count(&self) -> Result<usize> {
        self.registration_store.registration_count().await
    }

    async fn get_all_registrations(&self) -> Result<Vec<Registration>> {
        self.registration_store.get_all_registrations().await
    }

    async fn get_all_registrations_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<Registration>> {
        self.registration_store
            .get_all_registrations_in_tenant(tenant_id)
            .await
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

    async fn aggregate_usage(
        &self,
        tenant_id: &crate::TenantId,
        from: SystemTime,
        to: SystemTime,
    ) -> Result<UsageAggregation> {
        self.cdr_store.aggregate_usage(tenant_id, from, to).await
    }

    async fn aggregate_usage_by_number(
        &self,
        tenant_id: &crate::TenantId,
        from: SystemTime,
        to: SystemTime,
    ) -> Result<Vec<NumberUsage>> {
        self.cdr_store
            .aggregate_usage_by_number(tenant_id, from, to)
            .await
    }

    async fn create_rate_plan(&self, plan: crate::billing::RatePlan) -> Result<()> {
        self.cdr_store.create_rate_plan(plan).await
    }

    async fn get_rate_plan(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<Option<crate::billing::RatePlan>> {
        self.cdr_store.get_rate_plan(tenant_id, plan_id).await
    }

    async fn list_rate_plans(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::billing::RatePlan>> {
        self.cdr_store.list_rate_plans(tenant_id).await
    }

    async fn update_rate_plan(&self, plan: crate::billing::RatePlan) -> Result<()> {
        self.cdr_store.update_rate_plan(plan).await
    }

    async fn delete_rate_plan(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<()> {
        self.cdr_store.delete_rate_plan(tenant_id, plan_id).await
    }

    async fn upsert_rate(
        &self,
        tenant_id: &crate::TenantId,
        rate: crate::billing::Rate,
    ) -> Result<()> {
        self.cdr_store.upsert_rate(tenant_id, rate).await
    }

    async fn list_rates(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<Vec<crate::billing::Rate>> {
        self.cdr_store.list_rates(tenant_id, plan_id).await
    }

    async fn delete_rate(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
        rate_id: uuid::Uuid,
    ) -> Result<()> {
        self.cdr_store
            .delete_rate(tenant_id, plan_id, rate_id)
            .await
    }

    async fn get_default_rate_plan_with_rates(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<(crate::billing::RatePlan, Vec<crate::billing::Rate>)>> {
        self.cdr_store
            .get_default_rate_plan_with_rates(tenant_id)
            .await
    }

    async fn get_balance(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<crate::billing::TenantBalance>> {
        self.cdr_store.get_balance(tenant_id).await
    }

    async fn upsert_balance(&self, balance: crate::billing::TenantBalance) -> Result<()> {
        self.cdr_store.upsert_balance(balance).await
    }

    async fn apply_balance_transaction(
        &self,
        request: crate::billing::BalanceTransactionRequest,
    ) -> Result<crate::billing::BalanceTransaction> {
        self.cdr_store.apply_balance_transaction(request).await
    }

    async fn list_balance_transactions(
        &self,
        tenant_id: &crate::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::billing::BalanceTransaction>> {
        self.cdr_store
            .list_balance_transactions(tenant_id, limit, offset)
            .await
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

    async fn purchase_available_phone_number(
        &self,
        id: &crate::PhoneNumberId,
        tenant_id: &crate::TenantId,
        acquired_at: SystemTime,
    ) -> Result<Option<crate::PhoneNumber>> {
        self.phone_number_store
            .purchase_available_phone_number(id, tenant_id, acquired_at)
            .await
    }

    async fn delete_phone_number(&self, id: &crate::PhoneNumberId) -> Result<()> {
        self.phone_number_store.delete_phone_number(id).await
    }

    async fn search_available_numbers(
        &self,
        query: AvailableNumberQuery,
    ) -> Result<Vec<crate::PhoneNumber>> {
        self.phone_number_store
            .search_available_numbers(query)
            .await
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

    async fn get_device_by_mac_address(&self, mac_address: &str) -> Result<Option<crate::Device>> {
        self.device_store
            .get_device_by_mac_address(mac_address)
            .await
    }

    async fn put_device(&self, device: crate::Device) -> Result<()> {
        self.device_store.put_device(device).await
    }

    async fn delete_device(&self, id: &crate::DeviceId) -> Result<()> {
        self.device_store.delete_device(id).await
    }
}

#[async_trait]
impl TrunkStore for DynStorage {
    async fn get_trunk(&self, id: &crate::TrunkId) -> Result<Option<crate::Trunk>> {
        self.trunk_store.get_trunk(id).await
    }

    async fn list_trunks(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Trunk>> {
        self.trunk_store.list_trunks(tenant_id).await
    }

    async fn put_trunk(&self, trunk: crate::Trunk) -> Result<()> {
        self.trunk_store.put_trunk(trunk).await
    }

    async fn delete_trunk(&self, id: &crate::TrunkId) -> Result<()> {
        self.trunk_store.delete_trunk(id).await
    }

    async fn get_trunk_stats(&self, trunk_name: &str) -> Result<crate::TrunkStats> {
        self.trunk_store.get_trunk_stats(trunk_name).await
    }
}

#[async_trait]
impl RoutingRuleStore for DynStorage {
    async fn get_routing_rule(
        &self,
        id: &crate::RoutingRuleId,
    ) -> Result<Option<crate::RoutingRule>> {
        self.routing_rule_store.get_routing_rule(id).await
    }
    async fn list_routing_rules(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::RoutingRule>> {
        self.routing_rule_store.list_routing_rules(tenant_id).await
    }
    async fn put_routing_rule(&self, rule: crate::RoutingRule) -> Result<()> {
        self.routing_rule_store.put_routing_rule(rule).await
    }
    async fn delete_routing_rule(&self, id: &crate::RoutingRuleId) -> Result<()> {
        self.routing_rule_store.delete_routing_rule(id).await
    }
}

#[async_trait]
impl QueueConfigStore for DynStorage {
    async fn get_queue_config(
        &self,
        id: &crate::QueueConfigId,
    ) -> Result<Option<crate::QueueConfigRecord>> {
        self.queue_config_store.get_queue_config(id).await
    }
    async fn list_queue_configs(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::QueueConfigRecord>> {
        self.queue_config_store.list_queue_configs(tenant_id).await
    }
    async fn put_queue_config(&self, config: crate::QueueConfigRecord) -> Result<()> {
        self.queue_config_store.put_queue_config(config).await
    }
    async fn delete_queue_config(&self, id: &crate::QueueConfigId) -> Result<()> {
        self.queue_config_store.delete_queue_config(id).await
    }
}

#[async_trait]
impl SiteLocationStore for DynStorage {
    async fn get_location(&self, id: &crate::LocationId) -> Result<Option<crate::SiteLocation>> {
        self.site_location_store.get_location(id).await
    }

    async fn list_locations(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::SiteLocation>> {
        self.site_location_store.list_locations(tenant_id).await
    }

    async fn put_location(&self, location: crate::SiteLocation) -> Result<()> {
        self.site_location_store.put_location(location).await
    }

    async fn delete_location(&self, id: &crate::LocationId) -> Result<()> {
        self.site_location_store.delete_location(id).await
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

    async fn put_phone_number_location(&self, location: crate::PhoneNumberLocation) -> Result<()> {
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

#[async_trait]
impl TenantStore for DynStorage {
    async fn get_tenant(&self, id: &crate::TenantId) -> Result<Option<crate::Tenant>> {
        self.tenant_store.get_tenant(id).await
    }

    async fn get_tenant_by_name(&self, name: &str) -> Result<Option<crate::Tenant>> {
        self.tenant_store.get_tenant_by_name(name).await
    }

    async fn get_tenant_by_sip_domain(&self, domain: &str) -> Result<Option<crate::Tenant>> {
        self.tenant_store.get_tenant_by_sip_domain(domain).await
    }

    async fn list_tenants(&self) -> Result<Vec<crate::Tenant>> {
        self.tenant_store.list_tenants().await
    }

    async fn put_tenant(&self, tenant: crate::Tenant) -> Result<()> {
        self.tenant_store.put_tenant(tenant).await
    }

    async fn delete_tenant(&self, id: &crate::TenantId) -> Result<()> {
        self.tenant_store.delete_tenant(id).await
    }
}

#[async_trait]
impl ConferenceStore for DynStorage {
    async fn get_meeting(&self, id: &crate::MeetingId) -> Result<Option<crate::Meeting>> {
        self.conference_store.get_meeting(id).await
    }

    async fn get_meeting_by_room_number(
        &self,
        tenant_id: &crate::TenantId,
        room_number: &str,
    ) -> Result<Option<crate::Meeting>> {
        self.conference_store
            .get_meeting_by_room_number(tenant_id, room_number)
            .await
    }

    async fn list_meetings(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Meeting>> {
        self.conference_store.list_meetings(tenant_id).await
    }

    async fn list_meetings_by_owner(
        &self,
        tenant_id: &crate::TenantId,
        owner_id: &str,
    ) -> Result<Vec<crate::Meeting>> {
        self.conference_store
            .list_meetings_by_owner(tenant_id, owner_id)
            .await
    }

    async fn put_meeting(&self, meeting: crate::Meeting) -> Result<()> {
        self.conference_store.put_meeting(meeting).await
    }

    async fn delete_meeting(&self, id: &crate::MeetingId) -> Result<()> {
        self.conference_store.delete_meeting(id).await
    }
}

#[async_trait]
impl QueueStore for DynStorage {
    async fn save_agents(
        &self,
        tenant_id: &crate::TenantId,
        agents: Vec<StoredAgent>,
    ) -> Result<()> {
        self.queue_store.save_agents(tenant_id, agents).await
    }

    async fn load_agents(&self, tenant_id: &crate::TenantId) -> Result<Vec<StoredAgent>> {
        self.queue_store.load_agents(tenant_id).await
    }

    async fn remove_agent(&self, tenant_id: &crate::TenantId, agent_id: &str) -> Result<()> {
        self.queue_store.remove_agent(tenant_id, agent_id).await
    }

    async fn clear_agents(&self, tenant_id: &crate::TenantId) -> Result<()> {
        self.queue_store.clear_agents(tenant_id).await
    }

    async fn save_stats_snapshot(
        &self,
        tenant_id: &crate::TenantId,
        snapshot: StoredQueueSnapshot,
    ) -> Result<()> {
        self.queue_store
            .save_stats_snapshot(tenant_id, snapshot)
            .await
    }

    async fn query_stats_snapshots(
        &self,
        tenant_id: &crate::TenantId,
        queue_name: &str,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        limit: usize,
    ) -> Result<Vec<StoredQueueSnapshot>> {
        self.queue_store
            .query_stats_snapshots(tenant_id, queue_name, from_epoch, to_epoch, limit)
            .await
    }
}

#[async_trait]
impl PresenceStore for DynStorage {
    async fn save_subscription(&self, sub: StoredSubscription) -> Result<()> {
        self.presence_store.save_subscription(sub).await
    }

    async fn load_subscriptions(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<StoredSubscription>> {
        self.presence_store.load_subscriptions(tenant_id).await
    }

    async fn remove_subscription(&self, subscription_id: &str) -> Result<()> {
        self.presence_store
            .remove_subscription(subscription_id)
            .await
    }

    async fn remove_subscriptions_for_presentity(
        &self,
        tenant_id: &crate::TenantId,
        presentity_aor: &str,
    ) -> Result<()> {
        self.presence_store
            .remove_subscriptions_for_presentity(tenant_id, presentity_aor)
            .await
    }

    async fn cleanup_expired_subscriptions(&self) -> Result<usize> {
        self.presence_store.cleanup_expired_subscriptions().await
    }

    async fn save_presentity(&self, presentity: StoredPresentity) -> Result<()> {
        self.presence_store.save_presentity(presentity).await
    }

    async fn load_presentities(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<StoredPresentity>> {
        self.presence_store.load_presentities(tenant_id).await
    }

    async fn remove_presentity(&self, tenant_id: &crate::TenantId, aor: &str) -> Result<()> {
        self.presence_store.remove_presentity(tenant_id, aor).await
    }

    async fn save_auth_policy(&self, policy: StoredAuthPolicy) -> Result<()> {
        self.presence_store.save_auth_policy(policy).await
    }

    async fn load_auth_policies(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<StoredAuthPolicy>> {
        self.presence_store.load_auth_policies(tenant_id).await
    }

    async fn remove_auth_policy(&self, tenant_id: &crate::TenantId, aor: &str) -> Result<()> {
        self.presence_store.remove_auth_policy(tenant_id, aor).await
    }
}

#[async_trait]
impl ResourceListStore for DynStorage {
    async fn save_resource_list(&self, list: &StoredResourceList) -> Result<()> {
        self.resource_list_store.save_resource_list(list).await
    }

    async fn get_resource_list(
        &self,
        tenant_id: &crate::TenantId,
        uri: &str,
    ) -> Result<Option<StoredResourceList>> {
        self.resource_list_store
            .get_resource_list(tenant_id, uri)
            .await
    }

    async fn get_resource_lists_by_tenant(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<StoredResourceList>> {
        self.resource_list_store
            .get_resource_lists_by_tenant(tenant_id)
            .await
    }

    async fn delete_resource_list(&self, tenant_id: &crate::TenantId, uri: &str) -> Result<()> {
        self.resource_list_store
            .delete_resource_list(tenant_id, uri)
            .await
    }
}

#[async_trait]
impl WebhookStore for DynStorage {
    async fn create_endpoint(&self, endpoint: WebhookEndpoint) -> Result<()> {
        self.webhook_store.create_endpoint(endpoint).await
    }

    async fn get_endpoint(&self, id: &crate::WebhookId) -> Result<Option<WebhookEndpoint>> {
        self.webhook_store.get_endpoint(id).await
    }

    async fn list_endpoints(&self, tenant_id: &crate::TenantId) -> Result<Vec<WebhookEndpoint>> {
        self.webhook_store.list_endpoints(tenant_id).await
    }

    async fn update_endpoint(&self, endpoint: WebhookEndpoint) -> Result<()> {
        self.webhook_store.update_endpoint(endpoint).await
    }

    async fn delete_endpoint(&self, id: &crate::WebhookId) -> Result<()> {
        self.webhook_store.delete_endpoint(id).await
    }

    async fn log_delivery(&self, log: WebhookDeliveryLog) -> Result<()> {
        self.webhook_store.log_delivery(log).await
    }

    async fn get_pending_retries(&self, limit: usize) -> Result<Vec<WebhookDeliveryLog>> {
        self.webhook_store.get_pending_retries(limit).await
    }

    async fn update_delivery_log(&self, log: WebhookDeliveryLog) -> Result<()> {
        self.webhook_store.update_delivery_log(log).await
    }

    async fn get_delivery_logs(
        &self,
        endpoint_id: &crate::WebhookId,
        limit: usize,
    ) -> Result<Vec<WebhookDeliveryLog>> {
        self.webhook_store
            .get_delivery_logs(endpoint_id, limit)
            .await
    }

    async fn get_delivery_log(&self, id: &str) -> Result<Option<WebhookDeliveryLog>> {
        self.webhook_store.get_delivery_log(id).await
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
            id: crate::UserId::new(),
            username: "alice".to_string(),
            password: password_hash,
            digest_password: None,
            sip_ha1_hash: None,
            sip_provisioning_password: None,
            display_name: Some("Alice".to_string()),
            tenant_id: crate::TenantId::nil(), // Default tenant for test
            enabled: true,
            role: crate::UserRole::EndUser,
            department_id: None,
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
            location_id: None,
            language: "en-US".to_string(),
            timezone: "UTC".to_string(),
            account_status: AccountStatus::Active,
            last_login: None,
            emergency_contact: EmergencyContact::default(),
            primary_phone_number_id: None,
            voicemail_box_id: None,
            profile_picture: None,
            failed_login_attempts: 0,
            account_locked_until: None,
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
            tenant_id: crate::TenantId::nil(),
            aor: "sip:alice@example.com".to_string(),
            contact: "sip:alice@192.168.1.100:5060".to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            call_id: "test-call-id".to_string(),
            cseq: 1,
            user_agent: Some("FCP Test".to_string()),
            q: Some(1.0),
            registered_at: SystemTime::now(),
            path: None,
            flow_token: None,
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
