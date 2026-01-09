//! Storage abstraction layer for FCP.
//!
//! This module provides traits and implementations for storing:
//! - User credentials (for authentication)
//! - SIP registrations (location bindings)
//! - Call detail records (CDRs)
//!
//! The default implementation uses in-memory storage with JSON file persistence.

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
    }
}

/// A user account with SIP credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Username (SIP user part)
    pub username: String,

    /// Password hash (bcrypt)
    /// This should contain a bcrypt hash, not plaintext.
    /// Use `storage::password::hash_password()` to hash passwords before storing.
    pub password: String,

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
}

fn default_tenant_id() -> crate::TenantId {
    crate::TenantId::nil()
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

    /// Password (plaintext, will be hashed on load)
    pub password: String,

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
/// This is the default storage implementation that keeps data in memory
/// and periodically saves to JSON files.
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
}

impl DynStorage {
    /// Create a new DynStorage from individual trait objects.
    pub fn new(
        credential_store: Arc<dyn CredentialStore>,
        registration_store: Arc<dyn RegistrationStore>,
        cdr_store: Arc<dyn CdrStore>,
    ) -> Self {
        Self {
            credential_store,
            registration_store,
            cdr_store,
        }
    }

    /// Create a new DynStorage from a storage backend that implements all three traits.
    ///
    /// This is a convenience method for backends like FileBackedStorage that implement
    /// all storage traits.
    pub fn from_all_in_one<T>(storage: Arc<T>) -> Self
    where
        T: CredentialStore + RegistrationStore + CdrStore + 'static,
    {
        Self {
            credential_store: storage.clone(),
            registration_store: storage.clone(),
            cdr_store: storage,
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
            display_name: Some("Alice".to_string()),
            tenant_id: crate::TenantId::nil(), // Default tenant for test
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            media_preferences: MediaPreferences::default(),
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
