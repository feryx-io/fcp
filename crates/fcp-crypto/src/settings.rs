//! Deployment-level encryption settings, read from the environment so every
//! binary that opens the database configures encryption identically.
//!
//! | Variable | Meaning |
//! |---|---|
//! | `FCP_ENCRYPTION_PROVIDER` | `none` (default), `local`, or `aws-kms` |
//! | `FCP_MASTER_KEY` | `local`: base64 of 32 random bytes |
//! | `FCP_MASTER_KEY_FILE` | `local`: path to a file holding the base64 key (alternative) |
//! | `FCP_AWS_KMS_KEY_ID` | `aws-kms`: KMS key id, ARN, or alias |
//! | `FCP_PREVIOUS_MASTER_KEY` | while moving to a new KEK: the old local master key (base64) |
//! | `FCP_PREVIOUS_MASTER_KEY_FILE` | the same, from a file |
//! | `FCP_PREVIOUS_AWS_KMS_KEY_ID` | while moving to a new KEK: the old KMS key |
//!
//! Setting `FCP_MASTER_KEY` (or the file variant) without a provider selects
//! `local`. Generate a key with `openssl rand -base64 32`.

use crate::kek::{Kek, LocalKek, MASTER_KEY_LEN};
use crate::{CryptoError, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use std::fmt;
use std::sync::Arc;
use zeroize::Zeroizing;

/// Which key-encryption-key provider a deployment uses.
#[derive(Clone, Default)]
pub enum EncryptionSettings {
    /// Secrets are stored as-is. Sealed values that already exist cannot be
    /// read and surface as errors.
    #[default]
    Disabled,
    /// DEKs wrapped by an in-memory master key.
    Local { master_key: Zeroizing<Vec<u8>> },
    /// DEKs generated and unwrapped by AWS KMS (requires the `aws-kms`
    /// feature at build time).
    AwsKms { key_id: String },
}

impl fmt::Debug for EncryptionSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => write!(f, "Disabled"),
            Self::Local { .. } => write!(f, "Local {{ master_key: <redacted> }}"),
            Self::AwsKms { key_id } => write!(f, "AwsKms {{ key_id: {key_id:?} }}"),
        }
    }
}

impl EncryptionSettings {
    /// Read the settings from the process environment (see module docs).
    pub fn from_env() -> Result<Self> {
        let provider = std::env::var("FCP_ENCRYPTION_PROVIDER")
            .ok()
            .map(|p| p.trim().to_ascii_lowercase());
        let master = read_master_key_from_env()?;

        match provider.as_deref() {
            None | Some("") => match master {
                Some(master_key) => Ok(Self::Local { master_key }),
                None => Ok(Self::Disabled),
            },
            Some("none") | Some("disabled") | Some("off") => Ok(Self::Disabled),
            Some("local") => {
                let master_key = master.ok_or_else(|| {
                    CryptoError::Config(
                        "FCP_ENCRYPTION_PROVIDER=local requires FCP_MASTER_KEY or FCP_MASTER_KEY_FILE"
                            .into(),
                    )
                })?;
                Ok(Self::Local { master_key })
            }
            Some("aws-kms") | Some("aws_kms") | Some("aws") => {
                let key_id = std::env::var("FCP_AWS_KMS_KEY_ID")
                    .ok()
                    .filter(|k| !k.trim().is_empty())
                    .ok_or_else(|| {
                        CryptoError::Config(
                            "FCP_ENCRYPTION_PROVIDER=aws-kms requires FCP_AWS_KMS_KEY_ID".into(),
                        )
                    })?;
                Ok(Self::AwsKms { key_id })
            }
            Some(other) => Err(CryptoError::Config(format!(
                "unknown FCP_ENCRYPTION_PROVIDER {other:?} (expected none, local, or aws-kms)"
            ))),
        }
    }

    /// Build settings for a local master key given as base64.
    pub fn local_from_base64(encoded: &str) -> Result<Self> {
        Ok(Self::Local {
            master_key: decode_master_key(encoded)?,
        })
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// Short provider name for logs and status endpoints.
    pub fn provider_name(&self) -> &'static str {
        match self {
            Self::Disabled => "none",
            Self::Local { .. } => "local",
            Self::AwsKms { .. } => "aws-kms",
        }
    }

    /// Instantiate the provider. `None` when encryption is disabled.
    pub async fn kek(&self) -> Result<Option<Arc<dyn Kek>>> {
        match self {
            Self::Disabled => Ok(None),
            Self::Local { master_key } => Ok(Some(Arc::new(LocalKek::new(master_key)?))),
            #[cfg(feature = "aws-kms")]
            Self::AwsKms { key_id } => Ok(Some(Arc::new(
                crate::kek::aws::AwsKmsKek::from_env(key_id.clone()).await?,
            ))),
            #[cfg(not(feature = "aws-kms"))]
            Self::AwsKms { .. } => Err(CryptoError::Config(
                "FCP_ENCRYPTION_PROVIDER=aws-kms but this binary was built without the `aws-kms` feature"
                    .into(),
            )),
        }
    }
}

impl EncryptionSettings {
    /// The KEK a deployment is moving from, while it moves: what still
    /// unwraps the keys the old KEK wrapped, until they are rewrapped
    /// (`docs/ENCRYPTION.md`). `None` when no previous key is set.
    pub fn previous_from_env() -> Result<Option<Self>> {
        if let Some(master_key) =
            read_key_from_env("FCP_PREVIOUS_MASTER_KEY", "FCP_PREVIOUS_MASTER_KEY_FILE")?
        {
            return Ok(Some(Self::Local { master_key }));
        }
        Ok(std::env::var("FCP_PREVIOUS_AWS_KMS_KEY_ID")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .map(|key_id| Self::AwsKms { key_id }))
    }
}

fn read_master_key_from_env() -> Result<Option<Zeroizing<Vec<u8>>>> {
    read_key_from_env("FCP_MASTER_KEY", "FCP_MASTER_KEY_FILE")
}

/// A master key from `var`, else from the file `file_var` names.
fn read_key_from_env(var: &str, file_var: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
    if let Ok(encoded) = std::env::var(var) {
        if !encoded.trim().is_empty() {
            return decode_master_key(&encoded).map(Some);
        }
    }
    if let Ok(path) = std::env::var(file_var) {
        if !path.trim().is_empty() {
            let encoded = std::fs::read_to_string(&path)
                .map_err(|e| CryptoError::Config(format!("cannot read {file_var} {path}: {e}")))?;
            return decode_master_key(&encoded).map(Some);
        }
    }
    Ok(None)
}

fn decode_master_key(encoded: &str) -> Result<Zeroizing<Vec<u8>>> {
    let raw = B64
        .decode(encoded.trim())
        .map_err(|e| CryptoError::Config(format!("master key is not valid base64: {e}")))?;
    if raw.len() != MASTER_KEY_LEN {
        return Err(CryptoError::Config(format!(
            "master key must decode to {MASTER_KEY_LEN} bytes, got {}",
            raw.len()
        )));
    }
    Ok(Zeroizing::new(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_master_key_is_validated() {
        let ok = B64.encode([5u8; 32]);
        assert!(EncryptionSettings::local_from_base64(&ok)
            .unwrap()
            .is_enabled());
        assert!(matches!(
            EncryptionSettings::local_from_base64("not base64!"),
            Err(CryptoError::Config(_))
        ));
        assert!(matches!(
            EncryptionSettings::local_from_base64(&B64.encode([5u8; 16])),
            Err(CryptoError::Config(_))
        ));
    }

    #[tokio::test]
    async fn disabled_yields_no_kek() {
        assert!(EncryptionSettings::Disabled.kek().await.unwrap().is_none());
        assert_eq!(EncryptionSettings::default().provider_name(), "none");
    }

    #[tokio::test]
    async fn local_yields_local_kek() {
        let s = EncryptionSettings::local_from_base64(&B64.encode([5u8; 32])).unwrap();
        let kek = s.kek().await.unwrap().expect("kek");
        assert!(kek.id().starts_with("local:"));
        assert!(format!("{s:?}").contains("redacted"));
    }
}
