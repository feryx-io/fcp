//! Encryption at rest for the Ferrous Communications Platform.
//!
//! Secrets that must be recoverable in plaintext (SIP digest and provisioning
//! passwords, trunk credentials, conference PINs, webhook signing secrets)
//! are stored as AES-256-GCM ciphertext produced by a *data encryption key*
//! (DEK). DEKs never live in the database in the clear: each one is wrapped
//! by a *key encryption key* (KEK) held by a [`Kek`] provider and only the
//! wrapped form is persisted. This is classic envelope encryption:
//!
//! ```text
//!   KEK (local master key or AWS KMS)  ──wraps──▶  DEK  ──seals──▶  column value
//! ```
//!
//! - [`FieldCipher`] seals/opens individual values. Sealed values are
//!   self-describing strings (`enc:v1:<dek id>:<base64>`), so a column can
//!   hold a mix of legacy plaintext and ciphertext and be migrated lazily.
//! - [`Kek`] is the provider contract; [`LocalKek`] wraps DEKs with a
//!   32-byte master key from the environment (development and single-node
//!   deployments), `AwsKmsKek` (feature `aws-kms`) delegates to AWS KMS.
//! - [`EncryptionSettings`] reads the deployment's choice from the
//!   environment so every binary configures encryption the same way.
//!
//! Key rotation is a matter of adding a new DEK to the ring and marking it
//! active; old DEKs stay available for reads until every row has been
//! re-sealed.

pub mod cipher;
pub mod kek;
pub mod settings;

pub use cipher::{FieldCipher, Opened, SEALED_PREFIX};
pub use kek::{Kek, LocalKek, MASTER_KEY_LEN};
pub use settings::EncryptionSettings;

#[cfg(feature = "aws-kms")]
pub use kek::aws::AwsKmsKek;

/// Errors from sealing, opening and key management.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// The value is sealed with a DEK this cipher does not hold.
    #[error("unknown data encryption key: {0}")]
    UnknownKey(String),
    /// The value claims to be sealed but is not well-formed.
    #[error("malformed sealed value: {0}")]
    Malformed(String),
    /// Authentication failed: wrong key, or the ciphertext was tampered with.
    #[error("decryption failed (wrong key or corrupted ciphertext)")]
    Decrypt,
    /// The key-encryption-key provider failed.
    #[error("key provider error: {0}")]
    Kek(String),
    /// Bad configuration (missing or malformed key material).
    #[error("encryption configuration error: {0}")]
    Config(String),
    /// A sealed value was read but no cipher is configured.
    #[error("value is encrypted but encryption at rest is not configured")]
    NotConfigured,
}

pub type Result<T> = std::result::Result<T, CryptoError>;
