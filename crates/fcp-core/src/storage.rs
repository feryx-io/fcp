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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
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

    /// The name the user's phones register and authenticate as, when it
    /// is not their username (`docs/USERS.md` §6): a rename keeps it, since
    /// the SIP digest (HA1) is computed over it. `None` is the username.
    #[serde(default)]
    pub sip_username: Option<String>,

    /// Made by the tenant's SCIM provisioner (`docs/SCIM.md`): when the
    /// tenant's users come from SCIM, only these sign in by single sign-on.
    #[serde(default)]
    pub scim_managed: bool,

    /// The provisioner's own id for the user (SCIM `externalId`).
    #[serde(default)]
    pub scim_external_id: Option<String>,

    /// Display name (optional)
    pub display_name: Option<String>,

    /// Listed in the tenant's directory (`/me/directory`), as every user is
    /// unless an administrator, or the user where the tenant lets them,
    /// takes them out (an executive, say). An unlisted user still rings on
    /// their extension and is seen by administrators.
    #[serde(default = "default_directory_listed")]
    pub directory_listed: bool,

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

    /// When the user was deprovisioned; the purge follows the tenant's
    /// grace period later. Set only while `account_status` is
    /// `Deprovisioned`.
    #[serde(with = "optional_system_time_serde", default)]
    pub deprovisioned_at: Option<SystemTime>,
}

fn default_directory_listed() -> bool {
    true
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

/// Which of a tenant's users a listing wants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserFilter {
    /// A word in the username, display name or email (case aside).
    pub search: Option<String>,
    pub status: Option<AccountStatus>,
    /// Exactly this username, case aside (SCIM `userName eq`).
    pub username: Option<String>,
    /// Exactly this SCIM `externalId`.
    pub external_id: Option<String>,
    /// Exactly this email, case aside.
    pub email: Option<String>,
    /// Leave out deleted users waiting for their purge.
    pub exclude_deprovisioned: bool,
}

impl UserFilter {
    pub fn matches(&self, user: &User) -> bool {
        let status = self.status.is_none_or(|s| s == user.account_status);
        let search = self.search.as_deref().is_none_or(|q| {
            let q = q.to_lowercase();
            user.username.to_lowercase().contains(&q)
                || user
                    .display_name
                    .as_deref()
                    .is_some_and(|d| d.to_lowercase().contains(&q))
                || user
                    .email
                    .as_deref()
                    .is_some_and(|e| e.to_lowercase().contains(&q))
        });
        let eq = |want: &Option<String>, have: Option<&str>| {
            want.as_deref()
                .is_none_or(|w| have.is_some_and(|h| h.eq_ignore_ascii_case(w)))
        };
        status
            && search
            && eq(&self.username, Some(&user.username))
            && self
                .external_id
                .as_deref()
                .is_none_or(|w| user.scim_external_id.as_deref() == Some(w))
            && eq(&self.email, user.email.as_deref())
            && !(self.exclude_deprovisioned && user.is_deprovisioned())
    }
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
    /// Deleted by an admin or a provisioner: suspended, and purged when
    /// the tenant's grace period runs out (`docs/USERS.md` §4). Set only
    /// by a delete and cleared only by a restore.
    Deprovisioned,
}

impl std::fmt::Display for AccountStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Suspended => write!(f, "suspended"),
            Self::Locked => write!(f, "locked"),
            Self::Deprovisioned => write!(f, "deprovisioned"),
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
            "deprovisioned" => Ok(Self::Deprovisioned),
            // What the admin UI and the CLI called a suspension.
            "disabled" => Ok(Self::Suspended),
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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

    /// The user may record a call they are on, and stop that recording,
    /// from their app (`docs/RECORD_AND_CONTACTS.md` §2). Set by an
    /// administrator only.
    #[serde(default)]
    pub record_on_demand: bool,

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
            record_on_demand: false,
            preferred_codecs: default_codecs(),
        }
    }
}

fn default_codecs() -> Vec<String> {
    vec!["PCMU".to_string(), "PCMA".to_string()]
}

impl User {
    /// Whether the user is suspended: refused everywhere — SIP, voicemail,
    /// provisioning, hoteling, web sign-in and the API tokens they made
    /// (`docs/IDENTITY_AND_DEVICES.md` phase 5). `enabled: false` is the
    /// same thing, kept for one release.
    pub fn is_suspended(&self) -> bool {
        !self.enabled
            || matches!(
                self.account_status,
                AccountStatus::Suspended | AccountStatus::Deprovisioned
            )
    }

    /// The name the user's phones register and authenticate as: their
    /// SIP name, kept across renames, else their username. Every SIP
    /// lookup and every SIP address built for the user uses it.
    pub fn sip_name(&self) -> &str {
        self.sip_username.as_deref().unwrap_or(&self.username)
    }

    /// Whether the user is deleted and waiting for their purge.
    pub fn is_deprovisioned(&self) -> bool {
        self.account_status == AccountStatus::Deprovisioned
    }

    /// Whether the user's phones and services work. A Locked user's do:
    /// a lock is the web sign-in's guard against guessed passwords, and
    /// anyone could otherwise take a person's phones offline by guessing
    /// wrong five times.
    pub fn may_use_services(&self) -> bool {
        !self.is_suspended()
    }

    /// Make `enabled` and `account_status` say the same thing: a
    /// suspension is both. `enabled` follows the status; a write that
    /// clears `enabled` alone suspends.
    pub fn normalize_status(&mut self) {
        if !self.enabled
            && matches!(
                self.account_status,
                AccountStatus::Active | AccountStatus::Locked
            )
        {
            self.account_status = AccountStatus::Suspended;
        }
        self.enabled = !matches!(
            self.account_status,
            AccountStatus::Suspended | AccountStatus::Deprovisioned
        );
        if self.account_status != AccountStatus::Deprovisioned {
            self.deprovisioned_at = None;
        } else if self.deprovisioned_at.is_none() {
            self.deprovisioned_at = Some(SystemTime::now());
        }
    }

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

    /// The address the REGISTER came from (`ip:port`), which places a
    /// device that moves (`docs/E911_BLOCK_2.md` §6.2).
    #[serde(default)]
    pub source_address: Option<String>,

    /// The Contact's `+sip.instance` (RFC 5626): the same phone across
    /// addresses, so a new binding from it replaces the old.
    #[serde(default)]
    pub instance_id: Option<String>,

    /// The Contact's `reg-id` (RFC 5626).
    #[serde(default)]
    pub reg_id: Option<u32>,

    /// Where a sleeping app is woken (RFC 8599, `docs/PUSH.md`).
    #[serde(default)]
    pub push: Option<crate::push::PushAddress>,

    /// The node holding this binding's flow (`flow_token`), which a call on
    /// another node reaches it through.
    #[serde(default)]
    pub flow_node: Option<String>,
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

    /// The user whose phones register as `name` in a tenant
    /// (`User::sip_name`): what every SIP-side lookup asks. The default is
    /// the username, for a backend without SIP names.
    async fn get_user_by_sip_name(
        &self,
        name: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<User>> {
        self.get_user_in_tenant(name, tenant_id).await
    }

    /// Every tenant's user whose phones register as `name`.
    async fn list_users_by_sip_name(&self, name: &str) -> Result<Vec<User>> {
        self.list_users_by_username(name).await
    }

    /// Rename a user (`docs/USERS.md` §6): their username becomes
    /// `new_username`; their SIP name stays what it was. What else names
    /// them by username (meetings, legacy token makers) follows. The
    /// default rewrites the user under the new name and removes the old.
    async fn rename_user(&self, user: &User, new_username: &str) -> Result<User> {
        let mut renamed = user.clone();
        if renamed.sip_username.is_none() {
            renamed.sip_username = Some(user.username.clone());
        }
        renamed.username = new_username.to_string();
        renamed.updated_at = SystemTime::now();
        self.delete_user_in_tenant(&user.username, &user.tenant_id)
            .await?;
        self.put_user(renamed.clone()).await?;
        Ok(renamed)
    }

    /// A tenant's users that match `filter`, by username. The default
    /// filters every user in memory; the Postgres backend asks SQL.
    async fn list_users_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        filter: &UserFilter,
    ) -> Result<Vec<User>> {
        let mut users: Vec<User> = self
            .list_users()
            .await?
            .into_iter()
            .filter(|u| &u.tenant_id == tenant_id && filter.matches(u))
            .collect();
        users.sort_by(|a, b| a.username.cmp(&b.username));
        Ok(users)
    }

    /// How many users a tenant has (its `max_users`).
    /// The roles a user's groups give them (`docs/USERS.md` §5); none
    /// where the backend keeps no groups.
    async fn group_roles_of_user(&self, user_id: &crate::UserId) -> Result<Vec<crate::UserRole>> {
        let _ = user_id;
        Ok(Vec::new())
    }

    async fn count_users_in_tenant(&self, tenant_id: &crate::TenantId) -> Result<u64> {
        Ok(self
            .list_users_in_tenant(tenant_id, &UserFilter::default())
            .await?
            .len() as u64)
    }

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

    /// A tenant's users whose email is `email`, case aside
    /// (`docs/IDENTITY_AND_DEVICES.md` phase 2): who `jane@acme.com` is when
    /// it is not a username. Email is not unique, so there may be several.
    async fn find_users_by_email_in_tenant(
        &self,
        email: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<User>> {
        let email = email.trim();
        Ok(self
            .list_users()
            .await?
            .into_iter()
            .filter(|u| {
                u.tenant_id == *tenant_id
                    && u.email
                        .as_deref()
                        .is_some_and(|e| e.eq_ignore_ascii_case(email))
            })
            .collect())
    }

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

    /// The tenant claiming `domain` as a login domain
    /// (`docs/IDENTITY_AND_DEVICES.md` phase 2): where `someone@domain`
    /// signs in. A backend should use an index; this default reads them all.
    async fn get_tenant_by_login_domain(&self, domain: &str) -> Result<Option<crate::Tenant>> {
        let needle = domain.trim().to_ascii_lowercase();
        Ok(self
            .list_tenants()
            .await?
            .into_iter()
            .find(|t| t.login_domains.contains(&needle)))
    }

    /// List all tenants.
    async fn list_tenants(&self) -> Result<Vec<crate::Tenant>>;

    /// Create or update a tenant.
    async fn put_tenant(&self, tenant: crate::Tenant) -> Result<()>;

    /// Delete a tenant by ID.
    async fn delete_tenant(&self, id: &crate::TenantId) -> Result<()>;
}

/// How often a registration has been looked up in the store, and why
/// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1, phase 1).
///
/// Registrations are read from the primary, so in a deployment whose
/// primary is in another region every one of these is a round trip across
/// it. The estimate in the survey — two per call, times the domain
/// aliases, times the members of a hunt group — is arithmetic; this makes
/// it a number a deployment can read off `/status` and graph from
/// `/metrics`.
///
/// `direct`, `alias` and `housekeeping` are round trips to the store. A
/// lookup the registration cache answered (phase 2) is none of them: it is
/// counted as `cached`, so the round trips a cache saves are the
/// difference a deployment can see, not an estimate.
///
/// Counting only, deliberately: nothing here changes what a call does.
#[derive(Debug, Default)]
pub struct RegistrationReads {
    /// The address of record was looked up as asked.
    direct: AtomicU64,
    /// The same lookup again at one of the node's domain aliases. Every
    /// alias past the first is another round trip
    /// (`fcp-sip/src/registrar.rs`, `get_registration_rows_impl`).
    alias: AtomicU64,
    /// A read that is not on the call path: REGISTER handling, expiry
    /// sweeps, deregistration.
    housekeeping: AtomicU64,
    /// A lookup the registration cache answered without a round trip.
    cached: AtomicU64,
}

/// Why a registration was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationRead {
    /// The address of record as asked for.
    Direct,
    /// The same address at one of this node's domain aliases.
    Alias,
    /// Not the call path: REGISTER, expiry, deregistration.
    Housekeeping,
}

impl RegistrationReads {
    /// The counts this process has accumulated.
    pub fn global() -> &'static RegistrationReads {
        static READS: std::sync::OnceLock<RegistrationReads> = std::sync::OnceLock::new();
        READS.get_or_init(RegistrationReads::default)
    }

    /// Note one round trip to the registration store.
    pub fn note(&self, read: RegistrationRead) {
        let counter = match read {
            RegistrationRead::Direct => &self.direct,
            RegistrationRead::Alias => &self.alias,
            RegistrationRead::Housekeeping => &self.housekeeping,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Note one lookup answered from the registration cache — no round
    /// trip, so it is not in [`RegistrationReads::total`].
    pub fn note_cached(&self) {
        self.cached.fetch_add(1, Ordering::Relaxed);
    }

    /// Lookups the registration cache answered.
    pub fn cached(&self) -> u64 {
        self.cached.load(Ordering::Relaxed)
    }

    /// `(direct, alias, housekeeping)`.
    pub fn counts(&self) -> (u64, u64, u64) {
        (
            self.direct.load(Ordering::Relaxed),
            self.alias.load(Ordering::Relaxed),
            self.housekeeping.load(Ordering::Relaxed),
        )
    }

    /// Every round trip to the store, whatever the reason.
    pub fn total(&self) -> u64 {
        let (direct, alias, housekeeping) = self.counts();
        direct + alias + housekeeping
    }

    /// The counts as Prometheus series, so reads-per-call is a query
    /// against `fcp_calls_total` rather than arithmetic in a document.
    pub fn prometheus(&self, labels: &str) -> String {
        let (direct, alias, housekeeping) = self.counts();
        let sep = if labels.is_empty() { "" } else { "," };
        let mut out = String::from(
            "# HELP fcp_registration_reads_total Registration lookups in the store, by reason.\n\
             # TYPE fcp_registration_reads_total counter\n",
        );
        for (reason, n) in [
            ("direct", direct),
            ("alias", alias),
            ("housekeeping", housekeeping),
        ] {
            out.push_str(&format!(
                "fcp_registration_reads_total{{{labels}{sep}reason=\"{reason}\"}} {n}\n"
            ));
        }
        let hits_labels = if labels.is_empty() {
            String::new()
        } else {
            format!("{{{labels}}}")
        };
        out.push_str(&format!(
            "# HELP fcp_registration_cache_hits_total Registration lookups answered from the cache, without a round trip.\n\
             # TYPE fcp_registration_cache_hits_total counter\n\
             fcp_registration_cache_hits_total{hits_labels} {}\n",
            self.cached()
        ));
        out
    }
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

/// What a call detail record is a record of. One `Call` record exists per
/// call; the other kinds are records beside it, correlated by `call_id`
/// (`docs/CDR_PIPELINE.md` §5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CdrKind {
    /// The call as the B2BUA saw it: the winning leg, start to end.
    #[default]
    Call,
    /// A segment of a call spent in an ACD queue.
    Queue,
    /// A participant's leg on a conference server.
    Conference,
    /// A browser's session through the WebRTC gateway.
    Gateway,
}

impl CdrKind {
    /// The name stored in the database and shown by the API.
    pub fn as_str(&self) -> &'static str {
        match self {
            CdrKind::Call => "call",
            CdrKind::Queue => "queue",
            CdrKind::Conference => "conference",
            CdrKind::Gateway => "gateway",
        }
    }
}

impl std::str::FromStr for CdrKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "call" => Ok(CdrKind::Call),
            "queue" => Ok(CdrKind::Queue),
            "conference" => Ok(CdrKind::Conference),
            "gateway" => Ok(CdrKind::Gateway),
            other => Err(format!("unknown CDR kind: {other}")),
        }
    }
}

impl CdrKind {
    /// The kind filter a listing applies for a `?kind=` parameter
    /// (`docs/CDR_PIPELINE.md` §5): a kind lists that kind, `all` every
    /// kind, and no parameter the call records alone — or every kind when
    /// the listing asks for one call's records.
    pub fn listing_filter(
        param: Option<&str>,
        for_one_call: bool,
    ) -> std::result::Result<Option<CdrKind>, String> {
        match param.map(str::trim) {
            Some("all") => Ok(None),
            Some(kind) => kind.parse().map(Some).map_err(|_| {
                format!("Invalid kind: {kind}. Valid values: call, queue, conference, gateway, all")
            }),
            None if for_one_call => Ok(None),
            None => Ok(Some(CdrKind::Call)),
        }
    }
}

impl std::fmt::Display for CdrKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The schema version stamped on records built by this build of the
/// platform; the pipeline's normalizer decodes by it.
pub const CDR_SCHEMA_VERSION: u16 = 1;

fn default_cdr_schema_version() -> u16 {
    CDR_SCHEMA_VERSION
}

/// Which way a call went, as the tenant's numbers say: stored on the
/// record by the CDR pipeline's enricher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    /// Inbound call (from external to internal)
    Inbound,
    /// Outbound call (from internal to external)
    Outbound,
    /// Internal call (between internal endpoints)
    Internal,
}

/// Call Detail Record (CDR)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallDetailRecord {
    /// The record's own identity and the pipeline's idempotency key: minted
    /// by the node that built the record, so a replay of the same record
    /// stores nothing twice.
    #[serde(default = "uuid::Uuid::new_v4")]
    pub cdr_id: uuid::Uuid,

    /// What this is a record of; `Call` unless stated.
    #[serde(default)]
    pub kind: CdrKind,

    /// The record layout this record was built with.
    #[serde(default = "default_cdr_schema_version")]
    pub schema_version: u16,

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

    /// Which way the call went, as the tenant's numbers say; stored so the
    /// aggregates need not derive it per row. Filled by the pipeline's
    /// enricher (phase 2); `None` until then.
    #[serde(default)]
    pub direction: Option<CallDirection>,

    /// The node that owned the call and the region it advertised.
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    /// The site whose stack owned the call, at the cloud: set as the
    /// record comes up the site's link (`docs/HYBRID_CONTROL_PLANE.md`
    /// §3.3). Absent on a record this deployment's own nodes wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_id: Option<crate::SiteId>,

    /// The call this one replaced or was transferred from.
    #[serde(default)]
    pub correlation_id: Option<crate::CallId>,

    /// The trunk the call left or arrived on.
    #[serde(default)]
    pub trunk_name: Option<String>,

    /// Which trunk that was (`docs/TENANT_TRUNKS.md` §4): two tenants may
    /// both have a "carrier-a", so the name alone does not say.
    #[serde(default)]
    pub trunk_id: Option<crate::TrunkId>,

    /// Routing information (placeholders for future use)
    pub route_id: Option<String>,
    pub gateway_id: Option<String>,

    /// Media information. The RTP counters are totals over both legs as the
    /// media session saw them at the end of the call.
    pub media_anchored: bool,
    pub codec_a_leg: Option<String>,
    pub codec_b_leg: Option<String>,
    pub rtp_packets_sent: Option<u64>,
    pub rtp_packets_received: Option<u64>,
    pub rtp_bytes_sent: Option<u64>,
    pub rtp_bytes_received: Option<u64>,
    #[serde(default)]
    pub rtp_packets_lost: Option<u64>,
    #[serde(default)]
    pub rtp_jitter_ms: Option<f64>,

    /// Recording
    pub recording_url: Option<String>,
    /// What the parties were told about the call's recording; `None` when
    /// nothing recorded it (`docs/COMPLIANCE_FOLLOWUPS.md` §8).
    #[serde(default)]
    pub recording_consent: Option<crate::recording::RecordingConsent>,

    /// Cost (for billing), and what it came from.
    pub cost: Option<crate::billing::Decimal>,
    pub currency: Option<String>,
    #[serde(default)]
    pub rate_plan_id: Option<String>,
    #[serde(default)]
    pub matched_prefix: Option<String>,
    #[serde(default)]
    pub billable_seconds: Option<u32>,
    #[serde(default, with = "optional_system_time_serde")]
    pub rated_at: Option<SystemTime>,

    /// When the row was written by the pipeline (`end_time` is when the
    /// call ended).
    #[serde(default, with = "optional_system_time_serde")]
    pub created_at: Option<SystemTime>,

    /// Custom fields (JSON)
    pub custom_data: Option<serde_json::Value>,
}

/// A record the store refused for what it is rather than for being
/// unavailable: retrying it will not help (`docs/CDR_PIPELINE.md` §6).
#[derive(Debug, thiserror::Error)]
#[error("CDR rejected by the store: {0}")]
pub struct CdrRejected(pub String);

/// CDR query filter
#[derive(Debug, Clone, Default)]
pub struct CdrQuery {
    /// Filter by tenant ID
    pub tenant_id: Option<crate::TenantId>,

    /// Filter by what the record is a record of; `None` lists every kind
    pub kind: Option<CdrKind>,
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
    /// Keyset pagination: records strictly after this position in the
    /// listing's order (newest first). Takes the place of `offset`.
    pub cursor: Option<CdrCursor>,
    /// Records whose caller or callee has any of these canonical keys
    /// (`crate::subject::keys`): a person's calls. The keys lean towards
    /// matching; narrow with a `SubjectMatcher`.
    pub subject_keys: Option<Vec<String>>,
}

/// A position in a record listing, newest first: the last record of a page
/// (`docs/CDR_PIPELINE.md` §9). Opaque to API clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CdrCursor {
    /// The record's start time, in microseconds since the Unix epoch (the
    /// database's precision).
    pub start_micros: i64,
    pub cdr_id: uuid::Uuid,
}

impl CdrCursor {
    /// The cursor after `record`.
    pub fn after(record: &CallDetailRecord) -> Self {
        let micros = record
            .start_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0);
        Self {
            start_micros: micros,
            cdr_id: record.cdr_id,
        }
    }

    /// The opaque form an API hands out.
    pub fn encode(&self) -> String {
        format!("{:x}.{}", self.start_micros, self.cdr_id.simple())
    }

    /// Read a cursor an API handed out; `None` for anything else.
    pub fn decode(text: &str) -> Option<Self> {
        let (micros, id) = text.trim().split_once('.')?;
        Some(Self {
            start_micros: i64::from_str_radix(micros, 16).ok()?,
            cdr_id: uuid::Uuid::parse_str(id).ok()?,
        })
    }
}

/// A count of records, and whether it is exact or the planner's estimate
/// (`docs/CDR_PIPELINE.md` §9).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CdrCount {
    pub count: u64,
    pub exact: bool,
}

/// What a rollup rebuild did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollupRun {
    /// Hours recomputed.
    pub hours: u64,
    /// Rollup rows written.
    pub rows: u64,
}

/// A lock that keeps a maintenance job to one host at a time. Released on
/// `release`; a lock dropped without it is released when its connection
/// closes.
#[async_trait]
pub trait MaintenanceLock: Send {
    async fn release(self: Box<Self>);
}

/// The lock a store without coordination hands out: always granted.
pub struct UncoordinatedLock;

#[async_trait]
impl MaintenanceLock for UncoordinatedLock {
    async fn release(self: Box<Self>) {}
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
    pub total_cost: crate::billing::Decimal,
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

    /// Store a batch of records in one transaction and return the ids of
    /// the ones that were not already there; a record whose `cdr_id` is
    /// stored already is skipped. When the store refuses a record for what
    /// it is (a constraint or a value it cannot hold), nothing in the batch
    /// is stored and the error is a [`CdrRejected`], so the caller can
    /// store the records one at a time to find it; any other error is the
    /// store being unavailable. The default stores one record at a time and
    /// reports every record as new.
    async fn insert_cdrs(&self, cdrs: Vec<CallDetailRecord>) -> Result<Vec<uuid::Uuid>> {
        let mut ids = Vec::with_capacity(cdrs.len());
        for cdr in cdrs {
            let id = cdr.cdr_id;
            self.insert_cdr(cdr).await?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// Get a CDR by call ID
    async fn get_cdr(&self, call_id: &crate::CallId) -> Result<Option<CallDetailRecord>>;

    /// The records written after `after` in this store's shipping order
    /// (`ship_seq`, `docs/HYBRID_CONTROL_PLANE.md` §3.3), oldest first,
    /// each with its sequence, at most `limit`. A store that keeps no
    /// order has nothing to ship.
    async fn cdrs_after(&self, after: i64, limit: usize) -> Result<Vec<(i64, CallDetailRecord)>> {
        let _ = (after, limit);
        Ok(Vec::new())
    }

    /// Query CDRs with filters
    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>>;

    /// Get CDR count for a query
    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize>;

    /// Delete a tenant's CDRs that started before `before` (retention).
    /// Backends without retention support keep everything.
    async fn delete_cdrs_before(
        &self,
        _tenant_id: &crate::TenantId,
        _before: SystemTime,
    ) -> Result<u64> {
        Ok(0)
    }

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

    /// A count that gives up being exact after `budget`, answering with the
    /// query planner's estimate instead. The default counts exactly.
    async fn count_cdrs_within(&self, query: CdrQuery, budget: Duration) -> Result<CdrCount> {
        let _ = budget;
        Ok(CdrCount {
            count: self.count_cdrs(query).await? as u64,
            exact: true,
        })
    }

    /// Make sure the record table has partitions for the last month through
    /// the next three (`docs/CDR_PIPELINE.md` §8). Returns the partitions
    /// created. A backend without partitions does nothing.
    async fn ensure_cdr_partitions(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Drop the monthly record partitions that end before `before`, all of
    /// whose rows retention has already removed. Returns those dropped.
    async fn drop_cdr_partitions_before(&self, before: SystemTime) -> Result<Vec<String>> {
        let _ = before;
        Ok(Vec::new())
    }

    /// Recompute the hourly usage rollups for the last `window` (all of
    /// history on the first run) in one transaction.
    async fn rebuild_usage_rollups(&self, window: Duration) -> Result<RollupRun> {
        let _ = window;
        Ok(RollupRun::default())
    }

    /// Try to take the maintenance lock `name`; `None` when another host
    /// holds it.
    async fn try_maintenance_lock(&self, name: &str) -> Result<Option<Box<dyn MaintenanceLock>>> {
        let _ = name;
        Ok(Some(Box::new(UncoordinatedLock)))
    }

    // ── The archive (docs/CDR_PIPELINE.md §13, phase 5; crate::cdr_archive) ──
    // A backend without an archive confirms nothing, so a tenant whose
    // policy requires the archive keeps its records.

    /// The first day the archiver has not enumerated. Set on the first call
    /// to the day of the oldest record; `None` while there are none.
    async fn cdr_archive_next_day(&self) -> Result<Option<chrono::NaiveDate>> {
        Ok(None)
    }

    /// Record that every tenant with records on `day` has been archived:
    /// `next_day` moves past it, once records being stored for the day at
    /// that moment are committed (they are noted as late when they are
    /// not in the archive).
    async fn advance_cdr_archive(&self, day: chrono::NaiveDate) -> Result<()> {
        let _ = day;
        Ok(())
    }

    /// The tenants with records on `day`.
    async fn cdr_tenants_on(&self, day: chrono::NaiveDate) -> Result<Vec<crate::TenantId>> {
        let _ = day;
        Ok(Vec::new())
    }

    /// Tenant-days before `before` to write again: begun and never
    /// confirmed, or given late records. Oldest first.
    async fn pending_cdr_archives(
        &self,
        before: chrono::NaiveDate,
        limit: usize,
    ) -> Result<Vec<(crate::TenantId, chrono::NaiveDate)>> {
        let _ = (before, limit);
        Ok(Vec::new())
    }

    /// Start writing a tenant's day: the day stops counting as archived,
    /// its late-record notes are cleared, and the new generation is
    /// returned. Records stored after this are noted as late again.
    async fn begin_cdr_archive(
        &self,
        tenant_id: &crate::TenantId,
        day: chrono::NaiveDate,
    ) -> Result<u64> {
        let _ = (tenant_id, day);
        Err(anyhow::anyhow!("this store keeps no CDR archive"))
    }

    /// A page of a tenant's records on `day`, oldest first, strictly after
    /// `after` in that order (the cursor's fields, read ascending).
    async fn cdr_archive_page(
        &self,
        tenant_id: &crate::TenantId,
        day: chrono::NaiveDate,
        after: Option<CdrCursor>,
        limit: usize,
    ) -> Result<Vec<CallDetailRecord>> {
        let _ = (tenant_id, day, after, limit);
        Ok(Vec::new())
    }

    /// Confirm a write. `false` when a newer generation has begun since.
    async fn complete_cdr_archive(
        &self,
        write: &crate::cdr_archive::CdrArchiveWrite,
    ) -> Result<bool> {
        let _ = write;
        Ok(false)
    }

    /// What the archive holds for a tenant's day.
    async fn get_cdr_archive(
        &self,
        tenant_id: &crate::TenantId,
        day: chrono::NaiveDate,
    ) -> Result<Option<crate::cdr_archive::CdrArchiveEntry>> {
        let _ = (tenant_id, day);
        Ok(None)
    }

    /// The start of the tenant's first day that is not archived and
    /// confirmed: every record of the tenant's before it is in the
    /// archive. `None` when nothing is.
    async fn cdr_archive_frontier(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<SystemTime>> {
        let _ = tenant_id;
        Ok(None)
    }

    /// The archive as a whole.
    async fn cdr_archive_status(&self) -> Result<crate::cdr_archive::CdrArchiveStatus> {
        Ok(Default::default())
    }

    /// Archived days whose object lock should change: `true` for a day an
    /// active legal hold on records now covers and whose object is not
    /// locked, `false` for a locked day no hold covers. Oldest first.
    async fn cdr_archive_lock_changes(
        &self,
        limit: usize,
    ) -> Result<Vec<(crate::TenantId, chrono::NaiveDate, String, bool)>> {
        let _ = limit;
        Ok(Vec::new())
    }

    /// Record a day's object as locked or not.
    async fn set_cdr_archive_locked(
        &self,
        tenant_id: &crate::TenantId,
        day: chrono::NaiveDate,
        locked: bool,
    ) -> Result<()> {
        let _ = (tenant_id, day, locked);
        Ok(())
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

    /// The tenant's device that is `name` over SIP: the one whose own
    /// credentials are `name`, else one whose SIP URI's user is `name`
    /// (`docs/ENTERPRISE_CALLING.md` §3.8). Names are case-insensitive.
    async fn get_device_by_sip_name(
        &self,
        tenant_id: &crate::TenantId,
        name: &str,
    ) -> Result<Option<crate::Device>> {
        let devices = self.list_devices(tenant_id).await?;
        let by_credentials = devices.iter().find(|d| {
            d.sip_username
                .as_deref()
                .is_some_and(|u| u.eq_ignore_ascii_case(name))
        });
        Ok(by_credentials
            .or_else(|| {
                devices
                    .iter()
                    .find(|d| d.aor_user().is_some_and(|u| u.eq_ignore_ascii_case(name)))
            })
            .cloned())
    }

    /// Create or update a device
    async fn put_device(&self, device: crate::Device) -> Result<()>;

    /// Delete a device by ID
    async fn delete_device(&self, id: &crate::DeviceId) -> Result<()>;

    /// Write what a REGISTER said about a device, touching nothing else
    /// (`docs/PROVISIONING.md` §8).
    async fn record_device_registration(
        &self,
        id: &crate::DeviceId,
        update: &crate::device::DeviceRegistrationUpdate,
    ) -> Result<()> {
        if let Some(mut device) = self.get_device(id).await? {
            update.apply(&mut device);
            self.put_device(device).await?;
        }
        Ok(())
    }
}

/// SIP trunk storage operations.
#[async_trait::async_trait]
pub trait TrunkStore: Send + Sync {
    /// Get a trunk by ID
    async fn get_trunk(&self, id: &crate::TrunkId) -> Result<Option<crate::Trunk>>;

    /// List all trunks for a tenant
    async fn list_trunks(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Trunk>>;

    /// Every tenant's trunks, for a node building what it routes by
    /// (`docs/TENANT_TRUNKS.md` §4). Defaulted so a store that cannot do it
    /// simply has none, and the node routes by its own trunks.
    async fn list_all_trunks(&self) -> Result<Vec<crate::Trunk>> {
        Ok(Vec::new())
    }

    /// Create or update a trunk
    async fn put_trunk(&self, trunk: crate::Trunk) -> Result<()>;

    /// Delete a trunk by ID
    async fn delete_trunk(&self, id: &crate::TrunkId) -> Result<()>;

    /// Get aggregated stats for a tenant's trunk from its CDRs (last 24h).
    /// A record names its trunk by name, which is the tenant's alone: the
    /// tenant is part of the question, or a trunk of the same name in
    /// another tenant would count.
    async fn get_trunk_stats(
        &self,
        tenant_id: &crate::TenantId,
        trunk_name: &str,
    ) -> Result<crate::TrunkStats> {
        // Default: return empty stats (override in implementations with CDR access)
        let _ = (tenant_id, trunk_name);
        Ok(crate::TrunkStats::default())
    }

    // ── Least-cost routes (`docs/TENANT_TRUNKS.md` §12, phase 6c) ──────
    //
    // Defaulted like `list_all_trunks`: a store without them has none, and
    // every tenant's calls take the ordinary choice.

    async fn get_lcr_route(&self, id: &crate::LcrRouteId) -> Result<Option<crate::LcrRoute>> {
        let _ = id;
        Ok(None)
    }

    /// A tenant's routes, by prefix then cost.
    async fn list_lcr_routes(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::LcrRoute>> {
        let _ = tenant_id;
        Ok(Vec::new())
    }

    /// Every tenant's routes, for a node building what it routes by.
    async fn list_all_lcr_routes(&self) -> Result<Vec<crate::LcrRoute>> {
        Ok(Vec::new())
    }

    async fn put_lcr_route(&self, route: crate::LcrRoute) -> Result<()> {
        let _ = route;
        Err(anyhow::anyhow!("this store keeps no least-cost routes"))
    }

    async fn delete_lcr_route(&self, id: &crate::LcrRouteId) -> Result<()> {
        let _ = id;
        Err(anyhow::anyhow!("this store keeps no least-cost routes"))
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
    /// Every tenant's rules: what a node routing calls holds
    /// (`docs/TENANT_TRUNKS.md` §12, phase 6b).
    async fn list_all_routing_rules(&self) -> Result<Vec<crate::RoutingRule>>;
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

    /// Messages written or changed after `after` in shipping order
    /// (`ship_seq`, `docs/HYBRID_CONTROL_PLANE.md` §3.3), oldest first.
    async fn messages_after(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<Vec<(i64, crate::VoicemailMessage)>> {
        let _ = (after, limit);
        Ok(Vec::new())
    }

    /// Mailboxes written or changed after `after` in shipping order.
    async fn mailboxes_after(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<Vec<(i64, crate::VoicemailMailbox)>> {
        let _ = (after, limit);
        Ok(Vec::new())
    }

    /// Delete a message
    async fn delete_message(&self, id: &str) -> Result<()>;

    /// Delete a tenant's messages received before `before`, audio queued
    /// with them (retention). Returns how many.
    async fn delete_messages_before(
        &self,
        tenant_id: &crate::TenantId,
        before: SystemTime,
    ) -> Result<u64> {
        let _ = (tenant_id, before);
        Ok(0)
    }

    /// Delete the messages older than their own mailbox's `retention_days`.
    async fn apply_mailbox_retention(&self) -> Result<u64> {
        Ok(0)
    }

    /// Delete the messages marked deleted before `before`.
    async fn purge_deleted_messages(&self, before: SystemTime) -> Result<u64> {
        let _ = before;
        Ok(0)
    }

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
    /// The secret a rotation replaced, still signing deliveries beside the
    /// new one until `previous_secret_expires_at`, so a receiver can move to
    /// the new secret without refusing a genuine delivery
    /// (docs/DEVELOPER_SURFACE.md §3, phase 5).
    #[serde(default)]
    pub previous_secret: Option<String>,
    /// Until when the previous secret also signs.
    #[serde(default, with = "optional_system_time_serde")]
    pub previous_secret_expires_at: Option<SystemTime>,
}

impl WebhookEndpoint {
    /// The secrets a delivery made at `now` is signed with: the current
    /// one, and the one a rotation replaced while its overlap lasts.
    pub fn signing_secrets(&self, now: SystemTime) -> Vec<&str> {
        let mut secrets = vec![self.secret.as_str()];
        if let (Some(previous), Some(until)) =
            (&self.previous_secret, self.previous_secret_expires_at)
        {
            if now <= until && !previous.is_empty() {
                secrets.push(previous);
            }
        }
        secrets
    }
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

    /// Log a delivery unless one with its id is already logged; whether
    /// this one was. A delivery's id is derived from the event it carries,
    /// so an event handed over twice (an events server killed before it
    /// acknowledged it) is delivered once (`docs/DEEP_TESTING.md` D5b).
    async fn log_delivery_once(&self, log: WebhookDeliveryLog) -> Result<bool> {
        if self.get_delivery_log(&log.id).await?.is_some() {
            return Ok(false);
        }
        self.log_delivery(log).await?;
        Ok(true)
    }

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
    /// What the token reaches: the admin API under its permissions, or
    /// only its maker's own `/me` routes.
    #[serde(default)]
    pub scope: ApiTokenScope,
    /// When the token was revoked; a revoked token is kept, and refused.
    #[serde(default, with = "optional_system_time_serde")]
    pub revoked_at: Option<SystemTime>,
    /// Who revoked it: a username, a token id, or `static_token`.
    #[serde(default)]
    pub revoked_by: Option<String>,
    /// The hash of the secret a rotation replaced, honoured until
    /// `previous_expires_at` so a client can move to the new one.
    #[serde(default)]
    pub previous_token_hash: Option<String>,
    /// Until when the replaced secret still opens the token.
    #[serde(default, with = "optional_system_time_serde")]
    pub previous_expires_at: Option<SystemTime>,
    /// When the secret was last rotated.
    #[serde(default, with = "optional_system_time_serde")]
    pub rotated_at: Option<SystemTime>,
}

/// What an API token reaches (docs/DEVELOPER_SURFACE.md §3, phase 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiTokenScope {
    /// The admin API, limited to the token's permissions.
    #[default]
    Admin,
    /// Its maker's own `/me` routes, and nothing else.
    Personal,
}

impl ApiTokenScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Personal => "personal",
        }
    }
}

impl std::str::FromStr for ApiTokenScope {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "personal" => Ok(Self::Personal),
            other => Err(format!("unknown token scope: {other}")),
        }
    }
}

impl ApiToken {
    /// The stored form of a presented secret: hex SHA-256. The secret is
    /// 256 random bits, so a salt would add nothing.
    pub fn hash_secret(secret: &str) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(secret.as_bytes()))
    }

    /// Whether a secret hashing to `presented_hash` opens this token at
    /// `now`: not revoked, not expired, and either the current secret or the
    /// one a rotation replaced, inside its grace period.
    pub fn accepts(&self, presented_hash: &str, now: SystemTime) -> bool {
        if self.revoked_at.is_some() {
            return false;
        }
        if self.expires_at.is_some_and(|expires| now > expires) {
            return false;
        }
        if crate::secret::constant_time_eq(presented_hash, &self.token_hash) {
            return true;
        }
        match (&self.previous_token_hash, self.previous_expires_at) {
            (Some(previous), Some(until)) => {
                now <= until && crate::secret::constant_time_eq(presented_hash, previous)
            }
            _ => false,
        }
    }
}

/// A presented `fcp_…` secret that opened a token.
#[derive(Debug, Clone)]
pub struct AuthenticatedApiToken {
    pub token: ApiToken,
    /// The maker's role now, when the token names a maker and a user store
    /// was given; a token never does more than its maker could today.
    pub maker_role: Option<crate::UserRole>,
    /// The roles the maker's groups give them now.
    pub maker_group_roles: Vec<crate::UserRole>,
    /// The maker, by id and by their username now, when known.
    pub maker: Option<(crate::UserId, String)>,
}

/// Authenticate a presented API token secret: find it by hash, check it is
/// open (`ApiToken::accepts`), and check its maker is still an active user
/// of its tenant — a disabled, suspended or deleted maker's tokens are
/// refused. A lookup failure refuses (fails closed). A token without a maker
/// (made with the static token, or before makers were recorded) is judged
/// on itself alone.
pub async fn authenticate_api_token(
    tokens: &dyn ApiTokenStore,
    users: Option<&dyn CredentialStore>,
    secret: &str,
) -> Option<AuthenticatedApiToken> {
    let hash = ApiToken::hash_secret(secret);
    let token = match tokens.get_api_token_by_hash(&hash).await {
        Ok(Some(token)) => token,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to look up API token");
            return None;
        }
    };
    if !token.accepts(&hash, SystemTime::now()) {
        return None;
    }
    let mut maker_role = None;
    let mut maker_group_roles = Vec::new();
    let mut maker = None;
    if let (false, Some(users)) = (token.user_id.is_empty(), users) {
        // The maker by id (`docs/IDENTITY_AND_DEVICES.md` phase 1); a token
        // made before makers were ids names a username, matched once in its
        // tenant and rewritten to that user's id, so a user later made with
        // the same name is not its maker.
        let by_id = token
            .user_id
            .starts_with("user-")
            .then(|| token.user_id.parse::<crate::UserId>().ok())
            .flatten();
        let found = match by_id {
            Some(id) => users
                .get_user_by_id(&id)
                .await
                .map(|u| u.filter(|u| u.tenant_id == token.tenant_id)),
            None => {
                let found = users
                    .get_user_in_tenant(&token.user_id, &token.tenant_id)
                    .await;
                if let Ok(Some(user)) = &found {
                    if let Err(e) = tokens.set_api_token_maker(&token.id, &user.id).await {
                        tracing::warn!(token_id = %token.id, error = %e, "Failed to record an API token's maker by id");
                    }
                }
                found
            }
        };
        match found {
            Ok(Some(user)) => {
                if user.is_suspended() {
                    tracing::debug!(token_id = %token.id, maker = %token.user_id, "API token refused: its maker is inactive");
                    return None;
                }
                maker_role = Some(user.role);
                maker_group_roles = users.group_roles_of_user(&user.id).await.unwrap_or_else(|e| {
                    // Fewer roles, never more: the token does less until
                    // the groups can be read.
                    tracing::warn!(token_id = %token.id, error = %e, "Could not read the maker's groups");
                    Vec::new()
                });
                maker = Some((user.id, user.username));
            }
            Ok(None) => {
                tracing::debug!(token_id = %token.id, maker = %token.user_id, "API token refused: its maker is gone");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "API token maker check failed");
                return None;
            }
        }
    }
    Some(AuthenticatedApiToken {
        token,
        maker_role,
        maker_group_roles,
        maker,
    })
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

    /// Look up an API token by its hash (for authentication): the current
    /// secret's, or the previous one's while its grace period lasts.
    async fn get_api_token_by_hash(&self, token_hash: &str) -> Result<Option<ApiToken>>;

    /// Delete an API token by ID (subject erasure; revoking keeps the row).
    async fn delete_api_token(&self, id: &crate::ApiTokenId) -> Result<()>;

    /// Revoke a token: it is kept, marked, and refused from now on.
    async fn revoke_api_token(&self, id: &crate::ApiTokenId, revoked_by: &str) -> Result<()>;

    /// Give a token a new secret, the old one still opening it until
    /// `previous_valid_until`.
    async fn rotate_api_token(
        &self,
        id: &crate::ApiTokenId,
        new_hash: &str,
        new_prefix: &str,
        previous_valid_until: SystemTime,
    ) -> Result<()>;

    /// Update the last_used_at timestamp.
    async fn update_last_used(&self, id: &crate::ApiTokenId) -> Result<()>;

    /// Record a token's maker by id, in place of the username a token made
    /// before makers were ids carries.
    async fn set_api_token_maker(
        &self,
        id: &crate::ApiTokenId,
        maker: &crate::UserId,
    ) -> Result<()> {
        let _ = (id, maker);
        Ok(())
    }
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
    billing_store: Arc<dyn crate::billing::BillingStore>,
    phone_number_store: Arc<dyn PhoneNumberStore>,
    line_binding_store: Arc<dyn LineBindingStore>,
    extension_store: Arc<dyn ExtensionStore>,
    device_store: Arc<dyn DeviceStore>,
    voicemail_store: Arc<dyn VoicemailStore>,
    geo_routing_store: Arc<dyn GeoRoutingStore>,
    porting_store: Arc<dyn crate::porting::PortingStore>,
    site_store: Arc<dyn crate::site::SiteStore>,
    sync_store: Arc<dyn crate::sync::SyncStore>,
    calling_store: Arc<dyn crate::calling::CallingStore>,
    schedule_store: Arc<dyn crate::schedule::ScheduleStore>,
    class_of_service_store: Arc<dyn crate::class_of_service::ClassOfServiceStore>,
    hoteling_store: Arc<dyn crate::hoteling::HotelingStore>,
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
    /// Sign-in sessions (`docs/IDENTITY_AND_DEVICES.md` phase 1).
    login_session_store: Option<Arc<dyn crate::session::LoginSessionStore>>,
    /// Users' second factors (`docs/IDENTITY_AND_DEVICES.md` phase 2b).
    totp_store: Option<Arc<dyn crate::totp::TotpStore>>,
    /// Identity providers and users' identities (phase 3).
    identity_store: Option<Arc<dyn crate::identity::IdentityStore>>,
    prompt_store: Option<Arc<dyn crate::prompt::PromptStore>>,
    flow_trace_store: Option<Arc<dyn crate::flow_trace::FlowTraceStore>>,
    config_template_store: Option<Arc<dyn ConfigTemplateStore>>,
    encryption_store: Option<Arc<dyn crate::encryption::EncryptionStore>>,
    emergency_store: Option<Arc<dyn crate::e911::EmergencyStore>>,
    compliance_store: Option<Arc<dyn crate::compliance::ComplianceStore>>,
    recording_store: Option<Arc<dyn crate::recording::RecordingStore>>,
    call_recording_store: Option<Arc<dyn crate::recording::CallRecordingStore>>,
    subject_data_store: Option<Arc<dyn crate::subject_request::SubjectDataStore>>,
    subject_erase_store: Option<Arc<dyn crate::subject_erasure::SubjectEraseStore>>,
    offboarding_store: Option<Arc<dyn crate::offboarding::OffboardingStore>>,
    group_store: Option<Arc<dyn crate::group::GroupStore>>,
    brand_store: Option<Arc<dyn crate::branding::BrandStore>>,
    contact_store: Option<Arc<dyn crate::contact::ContactStore>>,
    provisioning_store: Option<Arc<dyn crate::provisioning::ProvisioningStore>>,
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
        billing_store: Arc<dyn crate::billing::BillingStore>,
        phone_number_store: Arc<dyn PhoneNumberStore>,
        line_binding_store: Arc<dyn LineBindingStore>,
        extension_store: Arc<dyn ExtensionStore>,
        device_store: Arc<dyn DeviceStore>,
        voicemail_store: Arc<dyn VoicemailStore>,
        geo_routing_store: Arc<dyn GeoRoutingStore>,
        porting_store: Arc<dyn crate::porting::PortingStore>,
        site_store: Arc<dyn crate::site::SiteStore>,
        sync_store: Arc<dyn crate::sync::SyncStore>,
        calling_store: Arc<dyn crate::calling::CallingStore>,
        schedule_store: Arc<dyn crate::schedule::ScheduleStore>,
        class_of_service_store: Arc<dyn crate::class_of_service::ClassOfServiceStore>,
        hoteling_store: Arc<dyn crate::hoteling::HotelingStore>,
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
            billing_store,
            phone_number_store,
            line_binding_store,
            extension_store,
            device_store,
            voicemail_store,
            geo_routing_store,
            porting_store,
            site_store,
            sync_store,
            calling_store,
            schedule_store,
            class_of_service_store,
            hoteling_store,
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
            login_session_store: None,
            totp_store: None,
            identity_store: None,
            prompt_store: None,
            flow_trace_store: None,
            config_template_store: None,
            encryption_store: None,
            emergency_store: None,
            compliance_store: None,
            subject_data_store: None,
            subject_erase_store: None,
            offboarding_store: None,
            group_store: None,
            brand_store: None,
            contact_store: None,
            provisioning_store: None,
            recording_store: None,
            call_recording_store: None,
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
            + crate::billing::BillingStore
            + PhoneNumberStore
            + LineBindingStore
            + ExtensionStore
            + DeviceStore
            + VoicemailStore
            + GeoRoutingStore
            + crate::porting::PortingStore
            + crate::site::SiteStore
            + crate::sync::SyncStore
            + crate::calling::CallingStore
            + crate::schedule::ScheduleStore
            + crate::class_of_service::ClassOfServiceStore
            + crate::hoteling::HotelingStore
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
            billing_store: storage.clone(),
            phone_number_store: storage.clone(),
            line_binding_store: storage.clone(),
            extension_store: storage.clone(),
            device_store: storage.clone(),
            voicemail_store: storage.clone(),
            geo_routing_store: storage.clone(),
            porting_store: storage.clone(),
            site_store: storage.clone(),
            sync_store: storage.clone(),
            calling_store: storage.clone(),
            schedule_store: storage.clone(),
            class_of_service_store: storage.clone(),
            hoteling_store: storage.clone(),
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
            login_session_store: None,
            totp_store: None,
            identity_store: None,
            prompt_store: None,
            flow_trace_store: None,
            config_template_store: Some(storage),
            encryption_store: None,
            emergency_store: None,
            compliance_store: None,
            subject_data_store: None,
            subject_erase_store: None,
            offboarding_store: None,
            group_store: None,
            brand_store: None,
            contact_store: None,
            provisioning_store: None,
            recording_store: None,
            call_recording_store: None,
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

    /// Attach the sign-in session store.
    pub fn with_login_session_store(
        mut self,
        store: Arc<dyn crate::session::LoginSessionStore>,
    ) -> Self {
        self.login_session_store = Some(store);
        self
    }

    /// Attach the second-factor store.
    pub fn with_totp_store(mut self, store: Arc<dyn crate::totp::TotpStore>) -> Self {
        self.totp_store = Some(store);
        self
    }

    /// Attach the identity-provider store.
    pub fn with_identity_store(mut self, store: Arc<dyn crate::identity::IdentityStore>) -> Self {
        self.identity_store = Some(store);
        self
    }

    /// The identity-provider store, if one is attached.
    pub fn identity_store(&self) -> Option<&Arc<dyn crate::identity::IdentityStore>> {
        self.identity_store.as_ref()
    }

    /// The second-factor store, if one is attached.
    pub fn totp_store(&self) -> Option<&Arc<dyn crate::totp::TotpStore>> {
        self.totp_store.as_ref()
    }

    /// The sign-in session store, if one is attached.
    pub fn login_session_store(&self) -> Option<&Arc<dyn crate::session::LoginSessionStore>> {
        self.login_session_store.as_ref()
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

    /// Attach the compliance store (retention/consent policy, admin audit log).
    pub fn with_compliance_store(
        mut self,
        store: Arc<dyn crate::compliance::ComplianceStore>,
    ) -> Self {
        self.compliance_store = Some(store);
        self
    }

    /// Attach the conference recording registry (`docs/VIDEO_CONFERENCING.md`
    /// §11). Without it a node still records, but the file is all there is.
    pub fn with_recording_store(
        mut self,
        store: Arc<dyn crate::recording::RecordingStore>,
    ) -> Self {
        self.recording_store = Some(store);
        self
    }

    /// The conference recording registry, when one is attached.
    pub fn recording_store(&self) -> Option<&Arc<dyn crate::recording::RecordingStore>> {
        self.recording_store.as_ref()
    }

    /// Attach the call recording registry (`docs/COMPLIANCE_FOLLOWUPS.md`
    /// §4). Without it a call is still recorded, but no retention, hold or
    /// erasure can find the file.
    pub fn with_call_recording_store(
        mut self,
        store: Arc<dyn crate::recording::CallRecordingStore>,
    ) -> Self {
        self.call_recording_store = Some(store);
        self
    }

    /// The call recording registry, when one is attached.
    pub fn call_recording_store(&self) -> Option<&Arc<dyn crate::recording::CallRecordingStore>> {
        self.call_recording_store.as_ref()
    }

    pub fn compliance_store(&self) -> Option<&Arc<dyn crate::compliance::ComplianceStore>> {
        self.compliance_store.as_ref()
    }

    /// Attach what subject exports read (`docs/COMPLIANCE_FOLLOWUPS.md` §6.3).
    pub fn with_subject_data_store(
        mut self,
        store: Arc<dyn crate::subject_request::SubjectDataStore>,
    ) -> Self {
        self.subject_data_store = Some(store);
        self
    }

    /// Attach where deprovisioned users are purged (`docs/USERS.md` §4).
    pub fn with_offboarding_store(
        mut self,
        store: Arc<dyn crate::offboarding::OffboardingStore>,
    ) -> Self {
        self.offboarding_store = Some(store);
        self
    }

    /// Attach the tenant's groups (`docs/USERS.md` §5).
    pub fn with_group_store(mut self, store: Arc<dyn crate::group::GroupStore>) -> Self {
        self.group_store = Some(store);
        self
    }

    /// Attach provisioning keys and the fetch log (`docs/PROVISIONING.md`).
    pub fn with_provisioning_store(
        mut self,
        store: Arc<dyn crate::provisioning::ProvisioningStore>,
    ) -> Self {
        self.provisioning_store = Some(store);
        self
    }

    /// Provisioning keys and the fetch log, when attached.
    pub fn provisioning_store(&self) -> Option<&Arc<dyn crate::provisioning::ProvisioningStore>> {
        self.provisioning_store.as_ref()
    }

    /// The tenant's groups, when attached.
    pub fn group_store(&self) -> Option<&Arc<dyn crate::group::GroupStore>> {
        self.group_store.as_ref()
    }

    /// Attach tenants' brands (`docs/BRANDING.md`).
    pub fn with_brand_store(mut self, store: Arc<dyn crate::branding::BrandStore>) -> Self {
        self.brand_store = Some(store);
        self
    }

    /// Attach the store of users' own contacts.
    pub fn with_contact_store(mut self, store: Arc<dyn crate::contact::ContactStore>) -> Self {
        self.contact_store = Some(store);
        self
    }

    /// Users' own contacts (`docs/RECORD_AND_CONTACTS.md` §3), when attached.
    pub fn contact_store(&self) -> Option<&Arc<dyn crate::contact::ContactStore>> {
        self.contact_store.as_ref()
    }

    /// Tenants' brands, when attached.
    pub fn brand_store(&self) -> Option<&Arc<dyn crate::branding::BrandStore>> {
        self.brand_store.as_ref()
    }

    /// Where deprovisioned users are purged, when attached.
    pub fn offboarding_store(&self) -> Option<&Arc<dyn crate::offboarding::OffboardingStore>> {
        self.offboarding_store.as_ref()
    }

    /// Attach where erasures act (`docs/COMPLIANCE_FOLLOWUPS.md` §6.3).
    pub fn with_subject_erase_store(
        mut self,
        store: Arc<dyn crate::subject_erasure::SubjectEraseStore>,
    ) -> Self {
        self.subject_erase_store = Some(store);
        self
    }

    /// Where erasures act, when attached.
    pub fn subject_erase_store(
        &self,
    ) -> Option<&Arc<dyn crate::subject_erasure::SubjectEraseStore>> {
        self.subject_erase_store.as_ref()
    }

    /// What subject exports read, when attached.
    pub fn subject_data_store(&self) -> Option<&Arc<dyn crate::subject_request::SubjectDataStore>> {
        self.subject_data_store.as_ref()
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

    async fn get_user_by_sip_name(
        &self,
        name: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<User>> {
        self.credential_store
            .get_user_by_sip_name(name, tenant_id)
            .await
    }

    async fn list_users_by_sip_name(&self, name: &str) -> Result<Vec<User>> {
        self.credential_store.list_users_by_sip_name(name).await
    }

    async fn rename_user(&self, user: &User, new_username: &str) -> Result<User> {
        self.credential_store.rename_user(user, new_username).await
    }

    async fn list_users_in_tenant(
        &self,
        tenant_id: &crate::TenantId,
        filter: &UserFilter,
    ) -> Result<Vec<User>> {
        self.credential_store
            .list_users_in_tenant(tenant_id, filter)
            .await
    }

    async fn count_users_in_tenant(&self, tenant_id: &crate::TenantId) -> Result<u64> {
        self.credential_store.count_users_in_tenant(tenant_id).await
    }

    async fn group_roles_of_user(&self, user_id: &crate::UserId) -> Result<Vec<crate::UserRole>> {
        self.credential_store.group_roles_of_user(user_id).await
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

    async fn find_users_by_email_in_tenant(
        &self,
        email: &str,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<User>> {
        self.credential_store
            .find_users_by_email_in_tenant(email, tenant_id)
            .await
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

    async fn insert_cdrs(&self, cdrs: Vec<CallDetailRecord>) -> Result<Vec<uuid::Uuid>> {
        self.cdr_store.insert_cdrs(cdrs).await
    }

    async fn get_cdr(&self, call_id: &crate::CallId) -> Result<Option<CallDetailRecord>> {
        self.cdr_store.get_cdr(call_id).await
    }

    async fn cdrs_after(&self, after: i64, limit: usize) -> Result<Vec<(i64, CallDetailRecord)>> {
        self.cdr_store.cdrs_after(after, limit).await
    }

    async fn query_cdrs(&self, query: CdrQuery) -> Result<Vec<CallDetailRecord>> {
        self.cdr_store.query_cdrs(query).await
    }

    async fn count_cdrs(&self, query: CdrQuery) -> Result<usize> {
        self.cdr_store.count_cdrs(query).await
    }

    async fn delete_cdrs_before(
        &self,
        tenant_id: &crate::TenantId,
        before: SystemTime,
    ) -> Result<u64> {
        self.cdr_store.delete_cdrs_before(tenant_id, before).await
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

    async fn count_cdrs_within(&self, query: CdrQuery, budget: Duration) -> Result<CdrCount> {
        self.cdr_store.count_cdrs_within(query, budget).await
    }

    async fn ensure_cdr_partitions(&self) -> Result<Vec<String>> {
        self.cdr_store.ensure_cdr_partitions().await
    }

    async fn drop_cdr_partitions_before(&self, before: SystemTime) -> Result<Vec<String>> {
        self.cdr_store.drop_cdr_partitions_before(before).await
    }

    async fn rebuild_usage_rollups(&self, window: Duration) -> Result<RollupRun> {
        self.cdr_store.rebuild_usage_rollups(window).await
    }

    async fn try_maintenance_lock(&self, name: &str) -> Result<Option<Box<dyn MaintenanceLock>>> {
        self.cdr_store.try_maintenance_lock(name).await
    }

    async fn cdr_archive_next_day(&self) -> Result<Option<chrono::NaiveDate>> {
        self.cdr_store.cdr_archive_next_day().await
    }

    async fn advance_cdr_archive(&self, day: chrono::NaiveDate) -> Result<()> {
        self.cdr_store.advance_cdr_archive(day).await
    }

    async fn cdr_tenants_on(&self, day: chrono::NaiveDate) -> Result<Vec<crate::TenantId>> {
        self.cdr_store.cdr_tenants_on(day).await
    }

    async fn pending_cdr_archives(
        &self,
        before: chrono::NaiveDate,
        limit: usize,
    ) -> Result<Vec<(crate::TenantId, chrono::NaiveDate)>> {
        self.cdr_store.pending_cdr_archives(before, limit).await
    }

    async fn begin_cdr_archive(
        &self,
        tenant_id: &crate::TenantId,
        day: chrono::NaiveDate,
    ) -> Result<u64> {
        self.cdr_store.begin_cdr_archive(tenant_id, day).await
    }

    async fn cdr_archive_page(
        &self,
        tenant_id: &crate::TenantId,
        day: chrono::NaiveDate,
        after: Option<CdrCursor>,
        limit: usize,
    ) -> Result<Vec<CallDetailRecord>> {
        self.cdr_store
            .cdr_archive_page(tenant_id, day, after, limit)
            .await
    }

    async fn complete_cdr_archive(
        &self,
        write: &crate::cdr_archive::CdrArchiveWrite,
    ) -> Result<bool> {
        self.cdr_store.complete_cdr_archive(write).await
    }

    async fn get_cdr_archive(
        &self,
        tenant_id: &crate::TenantId,
        day: chrono::NaiveDate,
    ) -> Result<Option<crate::cdr_archive::CdrArchiveEntry>> {
        self.cdr_store.get_cdr_archive(tenant_id, day).await
    }

    async fn cdr_archive_frontier(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<SystemTime>> {
        self.cdr_store.cdr_archive_frontier(tenant_id).await
    }

    async fn cdr_archive_status(&self) -> Result<crate::cdr_archive::CdrArchiveStatus> {
        self.cdr_store.cdr_archive_status().await
    }

    async fn cdr_archive_lock_changes(
        &self,
        limit: usize,
    ) -> Result<Vec<(crate::TenantId, chrono::NaiveDate, String, bool)>> {
        self.cdr_store.cdr_archive_lock_changes(limit).await
    }

    async fn set_cdr_archive_locked(
        &self,
        tenant_id: &crate::TenantId,
        day: chrono::NaiveDate,
        locked: bool,
    ) -> Result<()> {
        self.cdr_store
            .set_cdr_archive_locked(tenant_id, day, locked)
            .await
    }
}

#[async_trait]
impl crate::billing::BillingStore for DynStorage {
    async fn create_rate_plan(&self, plan: crate::billing::RatePlan) -> Result<()> {
        self.billing_store.create_rate_plan(plan).await
    }

    async fn get_rate_plan(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<Option<crate::billing::RatePlan>> {
        self.billing_store.get_rate_plan(tenant_id, plan_id).await
    }

    async fn list_rate_plans(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::billing::RatePlan>> {
        self.billing_store.list_rate_plans(tenant_id).await
    }

    async fn update_rate_plan(&self, plan: crate::billing::RatePlan) -> Result<()> {
        self.billing_store.update_rate_plan(plan).await
    }

    async fn delete_rate_plan(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<()> {
        self.billing_store
            .delete_rate_plan(tenant_id, plan_id)
            .await
    }

    async fn upsert_rate(
        &self,
        tenant_id: &crate::TenantId,
        rate: crate::billing::Rate,
    ) -> Result<()> {
        self.billing_store.upsert_rate(tenant_id, rate).await
    }

    async fn list_rates(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
    ) -> Result<Vec<crate::billing::Rate>> {
        self.billing_store.list_rates(tenant_id, plan_id).await
    }

    async fn delete_rate(
        &self,
        tenant_id: &crate::TenantId,
        plan_id: uuid::Uuid,
        rate_id: uuid::Uuid,
    ) -> Result<()> {
        self.billing_store
            .delete_rate(tenant_id, plan_id, rate_id)
            .await
    }

    async fn get_default_rate_plan_with_rates(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<(crate::billing::RatePlan, Vec<crate::billing::Rate>)>> {
        self.billing_store
            .get_default_rate_plan_with_rates(tenant_id)
            .await
    }

    async fn get_balance(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Option<crate::billing::TenantBalance>> {
        self.billing_store.get_balance(tenant_id).await
    }

    async fn upsert_balance(&self, balance: crate::billing::TenantBalance) -> Result<()> {
        self.billing_store.upsert_balance(balance).await
    }

    async fn apply_balance_transaction(
        &self,
        request: crate::billing::BalanceTransactionRequest,
    ) -> Result<crate::billing::BalanceTransaction> {
        self.billing_store.apply_balance_transaction(request).await
    }

    async fn list_balance_transactions(
        &self,
        tenant_id: &crate::TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::billing::BalanceTransaction>> {
        self.billing_store
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

    async fn get_device_by_sip_name(
        &self,
        tenant_id: &crate::TenantId,
        name: &str,
    ) -> Result<Option<crate::Device>> {
        self.device_store
            .get_device_by_sip_name(tenant_id, name)
            .await
    }

    async fn put_device(&self, device: crate::Device) -> Result<()> {
        self.device_store.put_device(device).await
    }

    async fn delete_device(&self, id: &crate::DeviceId) -> Result<()> {
        self.device_store.delete_device(id).await
    }

    async fn record_device_registration(
        &self,
        id: &crate::DeviceId,
        update: &crate::device::DeviceRegistrationUpdate,
    ) -> Result<()> {
        self.device_store
            .record_device_registration(id, update)
            .await
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

    async fn list_all_trunks(&self) -> Result<Vec<crate::Trunk>> {
        self.trunk_store.list_all_trunks().await
    }

    async fn put_trunk(&self, trunk: crate::Trunk) -> Result<()> {
        self.trunk_store.put_trunk(trunk).await
    }

    async fn delete_trunk(&self, id: &crate::TrunkId) -> Result<()> {
        self.trunk_store.delete_trunk(id).await
    }

    async fn get_trunk_stats(
        &self,
        tenant_id: &crate::TenantId,
        trunk_name: &str,
    ) -> Result<crate::TrunkStats> {
        self.trunk_store
            .get_trunk_stats(tenant_id, trunk_name)
            .await
    }

    async fn get_lcr_route(&self, id: &crate::LcrRouteId) -> Result<Option<crate::LcrRoute>> {
        self.trunk_store.get_lcr_route(id).await
    }

    async fn list_lcr_routes(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::LcrRoute>> {
        self.trunk_store.list_lcr_routes(tenant_id).await
    }

    async fn list_all_lcr_routes(&self) -> Result<Vec<crate::LcrRoute>> {
        self.trunk_store.list_all_lcr_routes().await
    }

    async fn put_lcr_route(&self, route: crate::LcrRoute) -> Result<()> {
        self.trunk_store.put_lcr_route(route).await
    }

    async fn delete_lcr_route(&self, id: &crate::LcrRouteId) -> Result<()> {
        self.trunk_store.delete_lcr_route(id).await
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
    async fn list_all_routing_rules(&self) -> Result<Vec<crate::RoutingRule>> {
        self.routing_rule_store.list_all_routing_rules().await
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

    async fn delete_messages_before(
        &self,
        tenant_id: &crate::TenantId,
        before: SystemTime,
    ) -> Result<u64> {
        self.voicemail_store
            .delete_messages_before(tenant_id, before)
            .await
    }

    async fn apply_mailbox_retention(&self) -> Result<u64> {
        self.voicemail_store.apply_mailbox_retention().await
    }

    async fn purge_deleted_messages(&self, before: SystemTime) -> Result<u64> {
        self.voicemail_store.purge_deleted_messages(before).await
    }

    async fn delete_message(&self, id: &str) -> Result<()> {
        self.voicemail_store.delete_message(id).await
    }

    async fn get_mwi_status(&self, mailbox_id: &str) -> Result<crate::MwiStatus> {
        self.voicemail_store.get_mwi_status(mailbox_id).await
    }

    async fn messages_after(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<Vec<(i64, crate::VoicemailMessage)>> {
        self.voicemail_store.messages_after(after, limit).await
    }

    async fn mailboxes_after(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<Vec<(i64, crate::VoicemailMailbox)>> {
        self.voicemail_store.mailboxes_after(after, limit).await
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
impl crate::sync::SyncStore for DynStorage {
    async fn journal_head(&self) -> Result<i64> {
        self.sync_store.journal_head().await
    }

    async fn journal_after(
        &self,
        tenant_id: &crate::TenantId,
        since: i64,
        limit: usize,
    ) -> Result<Vec<crate::JournalEntry>> {
        self.sync_store.journal_after(tenant_id, since, limit).await
    }
}

#[async_trait]
impl crate::class_of_service::ClassOfServiceStore for DynStorage {
    async fn get_class_of_service(
        &self,
        id: &crate::ClassOfServiceId,
    ) -> Result<Option<crate::ClassOfService>> {
        self.class_of_service_store.get_class_of_service(id).await
    }

    async fn list_classes_of_service(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::ClassOfService>> {
        self.class_of_service_store
            .list_classes_of_service(tenant_id)
            .await
    }

    async fn put_class_of_service(&self, class: crate::ClassOfService) -> Result<()> {
        self.class_of_service_store
            .put_class_of_service(class)
            .await
    }

    async fn delete_class_of_service(&self, id: &crate::ClassOfServiceId) -> Result<()> {
        self.class_of_service_store
            .delete_class_of_service(id)
            .await
    }
}

#[async_trait]
impl crate::hoteling::HotelingStore for DynStorage {
    async fn start_hoteling_session(
        &self,
        session: crate::hoteling::HotelingSession,
    ) -> Result<Vec<crate::hoteling::HotelingSession>> {
        self.hoteling_store.start_hoteling_session(session).await
    }

    async fn get_hoteling_session(
        &self,
        id: &crate::HotelingSessionId,
    ) -> Result<Option<crate::hoteling::HotelingSession>> {
        self.hoteling_store.get_hoteling_session(id).await
    }

    async fn hoteling_session_for_device(
        &self,
        device_id: &crate::DeviceId,
    ) -> Result<Option<crate::hoteling::HotelingSession>> {
        self.hoteling_store
            .hoteling_session_for_device(device_id)
            .await
    }

    async fn hoteling_session_for_user(
        &self,
        user_id: &crate::UserId,
    ) -> Result<Option<crate::hoteling::HotelingSession>> {
        self.hoteling_store.hoteling_session_for_user(user_id).await
    }

    async fn list_hoteling_sessions(
        &self,
        tenant_id: &crate::TenantId,
        include_ended: bool,
    ) -> Result<Vec<crate::hoteling::HotelingSession>> {
        self.hoteling_store
            .list_hoteling_sessions(tenant_id, include_ended)
            .await
    }

    async fn end_hoteling_session(
        &self,
        id: &crate::HotelingSessionId,
        by: crate::hoteling::SessionEnd,
    ) -> Result<Option<crate::hoteling::HotelingSession>> {
        self.hoteling_store.end_hoteling_session(id, by).await
    }

    async fn expire_hoteling_sessions(
        &self,
        now: SystemTime,
    ) -> Result<Vec<crate::hoteling::HotelingSession>> {
        self.hoteling_store.expire_hoteling_sessions(now).await
    }
}

#[async_trait]
impl crate::schedule::ScheduleStore for DynStorage {
    async fn get_schedule(&self, id: &crate::ScheduleId) -> Result<Option<crate::Schedule>> {
        self.schedule_store.get_schedule(id).await
    }

    async fn list_schedules(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Schedule>> {
        self.schedule_store.list_schedules(tenant_id).await
    }

    async fn put_schedule(&self, schedule: crate::Schedule) -> Result<()> {
        self.schedule_store.put_schedule(schedule).await
    }

    async fn delete_schedule(&self, id: &crate::ScheduleId) -> Result<()> {
        self.schedule_store.delete_schedule(id).await
    }
}

#[async_trait]
impl crate::calling::CallingStore for DynStorage {
    async fn get_calling_settings(
        &self,
        tenant_id: &crate::TenantId,
        user_id: &crate::UserId,
    ) -> Result<Option<crate::CallingSettings>> {
        self.calling_store
            .get_calling_settings(tenant_id, user_id)
            .await
    }

    async fn list_calling_settings(
        &self,
        tenant_id: &crate::TenantId,
    ) -> Result<Vec<crate::CallingSettings>> {
        self.calling_store.list_calling_settings(tenant_id).await
    }

    async fn put_calling_settings(&self, settings: crate::CallingSettings) -> Result<()> {
        self.calling_store.put_calling_settings(settings).await
    }

    async fn delete_calling_settings(
        &self,
        tenant_id: &crate::TenantId,
        user_id: &crate::UserId,
    ) -> Result<()> {
        self.calling_store
            .delete_calling_settings(tenant_id, user_id)
            .await
    }
}

#[async_trait]
impl crate::site::SiteStore for DynStorage {
    async fn get_site(&self, id: &crate::SiteId) -> Result<Option<crate::Site>> {
        self.site_store.get_site(id).await
    }

    async fn list_sites(&self, tenant_id: &crate::TenantId) -> Result<Vec<crate::Site>> {
        self.site_store.list_sites(tenant_id).await
    }

    async fn list_all_sites(&self) -> Result<Vec<crate::Site>> {
        self.site_store.list_all_sites().await
    }

    async fn put_site(&self, site: crate::Site) -> Result<()> {
        self.site_store.put_site(site).await
    }

    async fn delete_site(&self, id: &crate::SiteId) -> Result<()> {
        self.site_store.delete_site(id).await
    }

    async fn put_enrollment_token(&self, token: crate::EnrollmentToken) -> Result<()> {
        self.site_store.put_enrollment_token(token).await
    }

    async fn consume_enrollment_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<crate::EnrollmentToken>> {
        self.site_store.consume_enrollment_token(token_hash).await
    }

    async fn put_site_request(&self, request: crate::site::SiteRequest) -> Result<()> {
        self.site_store.put_site_request(request).await
    }

    async fn get_site_request(
        &self,
        id: &crate::SiteRequestId,
    ) -> Result<Option<crate::site::SiteRequest>> {
        self.site_store.get_site_request(id).await
    }

    async fn pending_site_requests(
        &self,
        site_id: &crate::SiteId,
        limit: usize,
    ) -> Result<Vec<crate::site::SiteRequest>> {
        self.site_store.pending_site_requests(site_id, limit).await
    }

    async fn get_site_by_secret_hash(&self, secret_hash: &str) -> Result<Option<crate::Site>> {
        self.site_store.get_site_by_secret_hash(secret_hash).await
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

    async fn get_tenant_by_login_domain(&self, domain: &str) -> Result<Option<crate::Tenant>> {
        self.tenant_store.get_tenant_by_login_domain(domain).await
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

    async fn log_delivery_once(&self, log: WebhookDeliveryLog) -> Result<bool> {
        self.webhook_store.log_delivery_once(log).await
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

    #[test]
    fn a_listing_shows_call_records_unless_asked_for_a_kind_all_or_one_call() {
        assert_eq!(
            CdrKind::listing_filter(None, false),
            Ok(Some(CdrKind::Call))
        );
        assert_eq!(CdrKind::listing_filter(None, true), Ok(None));
        assert_eq!(CdrKind::listing_filter(Some("all"), false), Ok(None));
        assert_eq!(
            CdrKind::listing_filter(Some("queue"), true),
            Ok(Some(CdrKind::Queue))
        );
        assert!(CdrKind::listing_filter(Some("calls"), false).is_err());
    }

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
            directory_listed: true,
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
            deprovisioned_at: None,
            sip_username: None,
            scim_managed: false,
            scim_external_id: None,
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
            source_address: None,
            instance_id: None,
            reg_id: None,
            push: None,
            flow_node: None,
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

#[cfg(test)]
mod registration_read_tests {
    use super::*;

    #[test]
    fn a_read_is_counted_under_the_reason_it_was_made_for() {
        // The global is shared, so this asserts on deltas rather than on
        // absolute values: another test in the binary may have counted too.
        let reads = RegistrationReads::default();
        assert_eq!(reads.counts(), (0, 0, 0));
        assert_eq!(reads.total(), 0);

        reads.note(RegistrationRead::Direct);
        reads.note(RegistrationRead::Alias);
        reads.note(RegistrationRead::Alias);
        reads.note(RegistrationRead::Housekeeping);

        assert_eq!(reads.counts(), (1, 2, 1));
        assert_eq!(reads.total(), 4, "every round trip counts once");
    }

    #[test]
    fn the_series_carry_the_reason_and_the_labels() {
        let reads = RegistrationReads::default();
        reads.note(RegistrationRead::Direct);
        reads.note(RegistrationRead::Alias);

        let body = reads.prometheus("node=\"cm-1\"");
        assert!(body.contains("# TYPE fcp_registration_reads_total counter"));
        assert!(
            body.contains("fcp_registration_reads_total{node=\"cm-1\",reason=\"direct\"} 1"),
            "{body}"
        );
        assert!(
            body.contains("fcp_registration_reads_total{node=\"cm-1\",reason=\"alias\"} 1"),
            "{body}"
        );
        // A reason with no reads is still published, so a graph does not
        // start existing only once something goes wrong.
        assert!(body.contains("reason=\"housekeeping\"} 0"), "{body}");
    }

    #[test]
    fn the_series_are_well_formed_without_labels() {
        let body = RegistrationReads::default().prometheus("");
        assert!(
            body.contains("fcp_registration_reads_total{reason=\"direct\"} 0"),
            "{body}"
        );
        assert!(!body.contains("{,"), "no stray separator: {body}");
        assert!(
            body.contains("fcp_registration_cache_hits_total 0"),
            "{body}"
        );
    }

    #[test]
    fn a_cached_lookup_is_not_a_round_trip() {
        let reads = RegistrationReads::default();
        reads.note(RegistrationRead::Direct);
        reads.note_cached();
        reads.note_cached();

        assert_eq!(reads.cached(), 2);
        assert_eq!(
            reads.total(),
            1,
            "the total is round trips to the store; a cache hit made none"
        );
        let body = reads.prometheus("node=\"cm-1\"");
        assert!(
            body.contains("fcp_registration_cache_hits_total{node=\"cm-1\"} 2"),
            "{body}"
        );
    }

    #[test]
    fn the_global_is_one_counter() {
        let before = RegistrationReads::global().total();
        RegistrationReads::global().note(RegistrationRead::Direct);
        assert_eq!(RegistrationReads::global().total(), before + 1);
    }
}
