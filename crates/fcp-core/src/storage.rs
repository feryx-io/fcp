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

/// Password hashing utilities using bcrypt.
pub mod password {
    use anyhow::Result;

    const BCRYPT_COST: u32 = 12; // Higher = more secure but slower

    /// Hash a plaintext password using bcrypt.
    pub fn hash_password(password: &str) -> Result<String> {
        bcrypt::hash(password, BCRYPT_COST)
            .map_err(|e| anyhow::anyhow!("Failed to hash password: {}", e))
    }

    /// Verify a plaintext password against a bcrypt hash.
    pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
        bcrypt::verify(password, hash)
            .map_err(|e| anyhow::anyhow!("Failed to verify password: {}", e))
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
}

fn default_tenant_id() -> crate::TenantId {
    crate::TenantId::nil()
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
    async fn verify_credentials(&self, username: &str, password: &str) -> Result<bool> {
        if let Some(user) = self.get_user(username).await? {
            if !user.enabled {
                return Ok(false);
            }
            // Use bcrypt to verify password against hash
            password::verify_password(password, &user.password)
        } else {
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
    async fn load_registrations(
        path: &PathBuf,
    ) -> Result<HashMap<String, Vec<Registration>>> {
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
        let reg_list: Vec<Registration> = registrations
            .values()
            .flatten()
            .cloned()
            .collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_file_backed_storage_users() {
        let temp_dir = std::env::temp_dir().join("fcp-test-storage");
        let storage = FileBackedStorage::new(temp_dir.clone()).await.unwrap();

        // Create user with hashed password
        let password_hash = password::hash_password("secret123").unwrap();
        let user = User {
            username: "alice".to_string(),
            password: password_hash,
            display_name: Some("Alice".to_string()),
            tenant_id: crate::TenantId::nil(), // Default tenant for test
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        storage.put_user(user.clone()).await.unwrap();

        // Get user
        let retrieved = storage.get_user("alice").await.unwrap().unwrap();
        assert_eq!(retrieved.username, "alice");

        // Verify credentials
        assert!(storage
            .verify_credentials("alice", "secret123")
            .await
            .unwrap());
        assert!(!storage
            .verify_credentials("alice", "wrong")
            .await
            .unwrap());

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
