//! Encryption-at-rest management contract.
//!
//! The cipher itself lives in `fcp-crypto` and is applied inside the storage
//! backend; this trait is what the admin API uses to inspect the key ring,
//! rotate the active data encryption key, and re-seal stored secrets.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Snapshot of a backend's encryption state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionStatus {
    /// Whether secret columns are sealed on write.
    pub enabled: bool,
    /// `none`, `local`, or `aws-kms`.
    pub provider: String,
    /// Identifier of the key-encryption key wrapping the DEKs.
    pub kek_id: Option<String>,
    /// DEK used for new writes.
    pub active_key_id: Option<String>,
    /// Every DEK the backend can read with, oldest first.
    pub key_ids: Vec<String>,
    /// `table.column` names that are sealed.
    pub sealed_columns: Vec<String>,
}

impl EncryptionStatus {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            provider: "none".to_string(),
            kek_id: None,
            active_key_id: None,
            key_ids: Vec::new(),
            sealed_columns: Vec::new(),
        }
    }
}

/// Outcome of a bulk re-seal pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReencryptReport {
    /// Rows rewritten with the active DEK (were plaintext or an older DEK).
    pub rewritten: u64,
    /// Rows already sealed with the active DEK.
    pub already_current: u64,
    /// Per-column breakdown of rewritten rows (`table.column` → count).
    pub by_column: Vec<(String, u64)>,
}

/// Key-ring management for a storage backend.
#[async_trait]
pub trait EncryptionStore: Send + Sync {
    async fn encryption_status(&self) -> anyhow::Result<EncryptionStatus>;

    /// Create a new DEK, make it active for new writes, and keep the old
    /// ones for reads. Returns the new status. Fails when encryption is
    /// disabled.
    async fn rotate_encryption_key(&self) -> anyhow::Result<EncryptionStatus>;

    /// Rewrite every stored secret that is plaintext or sealed with a
    /// non-active DEK using the active one. Fails when encryption is
    /// disabled.
    async fn reencrypt_secrets(&self) -> anyhow::Result<ReencryptReport>;
}
