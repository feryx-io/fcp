//! Key-encryption-key providers.
//!
//! A [`Kek`] never exposes its own key material; it only generates data
//! encryption keys and wraps/unwraps them. The wrapped form is what the
//! storage layer persists.

use crate::cipher::{generate_dek, DEK_LEN};
use crate::{CryptoError, Result};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use async_trait::async_trait;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::fmt;
use zeroize::Zeroizing;

/// Length of the local master key in bytes.
pub const MASTER_KEY_LEN: usize = 32;

/// A key-encryption-key provider.
#[async_trait]
pub trait Kek: Send + Sync {
    /// Stable identifier recorded alongside every wrapped DEK, so a
    /// deployment can tell which KEK a persisted key belongs to
    /// (`local:<fingerprint>`, `aws-kms:<key id>`).
    fn id(&self) -> &str;

    /// Generate a fresh 256-bit DEK, returning `(plaintext, wrapped)`.
    async fn generate_dek(&self) -> Result<(Zeroizing<Vec<u8>>, Vec<u8>)>;

    /// Recover a DEK from its wrapped form.
    async fn unwrap_dek(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>>;
}

/// Wraps DEKs with a 32-byte master key held in memory (AES-256-GCM).
///
/// Suitable for development and single-operator deployments where the
/// master key is delivered through the environment or a mounted secret. Its
/// id is derived from a SHA-256 fingerprint of the key, so a mismatched
/// master key is detected at startup rather than as decrypt failures.
pub struct LocalKek {
    aead: Aes256Gcm,
    id: String,
}

impl fmt::Debug for LocalKek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalKek").field("id", &self.id).finish()
    }
}

impl LocalKek {
    pub fn new(master_key: &[u8]) -> Result<Self> {
        if master_key.len() != MASTER_KEY_LEN {
            return Err(CryptoError::Config(format!(
                "master key must be {MASTER_KEY_LEN} bytes, got {}",
                master_key.len()
            )));
        }
        let fingerprint = Sha256::digest(master_key);
        let id = format!("local:{}", hex(&fingerprint[..8]));
        let aead = Aes256Gcm::new_from_slice(master_key)
            .map_err(|_| CryptoError::Config("bad master key length".into()))?;
        Ok(Self { aead, id })
    }
}

#[async_trait]
impl Kek for LocalKek {
    fn id(&self) -> &str {
        &self.id
    }

    async fn generate_dek(&self) -> Result<(Zeroizing<Vec<u8>>, Vec<u8>)> {
        let dek = generate_dek();
        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);
        let ct = self
            .aead
            .encrypt(
                &nonce,
                aes_gcm::aead::Payload {
                    msg: &dek,
                    aad: self.id.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::Kek("wrap failed".into()))?;
        let mut wrapped = Vec::with_capacity(12 + ct.len());
        wrapped.extend_from_slice(&nonce_bytes);
        wrapped.extend_from_slice(&ct);
        Ok((dek, wrapped))
    }

    async fn unwrap_dek(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if wrapped.len() < 12 + 16 {
            return Err(CryptoError::Kek("wrapped key too short".into()));
        }
        let (nonce_bytes, ct) = wrapped.split_at(12);
        let nonce = Nonce::from(<[u8; 12]>::try_from(nonce_bytes).expect("length checked"));
        let dek = self
            .aead
            .decrypt(
                &nonce,
                aes_gcm::aead::Payload {
                    msg: ct,
                    aad: self.id.as_bytes(),
                },
            )
            .map_err(|_| {
                CryptoError::Kek(
                    "unwrap failed: master key does not match the key this DEK was wrapped with"
                        .into(),
                )
            })?;
        if dek.len() != DEK_LEN {
            return Err(CryptoError::Kek("unwrapped key has wrong length".into()));
        }
        Ok(Zeroizing::new(dek))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(feature = "aws-kms")]
pub mod aws {
    //! AWS KMS as the KEK: `GenerateDataKey` for new DEKs, `Decrypt` to
    //! unwrap. The KMS key itself never leaves AWS.

    use super::*;
    use aws_sdk_kms::primitives::Blob;
    use aws_sdk_kms::types::DataKeySpec;

    pub struct AwsKmsKek {
        client: aws_sdk_kms::Client,
        key_id: String,
        id: String,
    }

    impl fmt::Debug for AwsKmsKek {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("AwsKmsKek").field("id", &self.id).finish()
        }
    }

    impl AwsKmsKek {
        /// Connect using the default AWS credential/region chain.
        pub async fn from_env(key_id: impl Into<String>) -> Result<Self> {
            let config = aws_config::load_from_env().await;
            Ok(Self::with_client(aws_sdk_kms::Client::new(&config), key_id))
        }

        pub fn with_client(client: aws_sdk_kms::Client, key_id: impl Into<String>) -> Self {
            let key_id = key_id.into();
            let id = format!("aws-kms:{key_id}");
            Self { client, key_id, id }
        }
    }

    #[async_trait]
    impl Kek for AwsKmsKek {
        fn id(&self) -> &str {
            &self.id
        }

        async fn generate_dek(&self) -> Result<(Zeroizing<Vec<u8>>, Vec<u8>)> {
            let out = self
                .client
                .generate_data_key()
                .key_id(&self.key_id)
                .key_spec(DataKeySpec::Aes256)
                .send()
                .await
                .map_err(|e| CryptoError::Kek(format!("GenerateDataKey: {e}")))?;
            let plaintext = out
                .plaintext()
                .ok_or_else(|| CryptoError::Kek("GenerateDataKey returned no plaintext".into()))?
                .as_ref()
                .to_vec();
            let wrapped = out
                .ciphertext_blob()
                .ok_or_else(|| CryptoError::Kek("GenerateDataKey returned no ciphertext".into()))?
                .as_ref()
                .to_vec();
            Ok((Zeroizing::new(plaintext), wrapped))
        }

        async fn unwrap_dek(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
            let out = self
                .client
                .decrypt()
                .key_id(&self.key_id)
                .ciphertext_blob(Blob::new(wrapped))
                .send()
                .await
                .map_err(|e| CryptoError::Kek(format!("Decrypt: {e}")))?;
            let dek = out
                .plaintext()
                .ok_or_else(|| CryptoError::Kek("Decrypt returned no plaintext".into()))?
                .as_ref()
                .to_vec();
            Ok(Zeroizing::new(dek))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_kek_wraps_and_unwraps() {
        let kek = LocalKek::new(&[1u8; 32]).unwrap();
        assert!(kek.id().starts_with("local:"));
        let (dek, wrapped) = kek.generate_dek().await.unwrap();
        assert_eq!(dek.len(), DEK_LEN);
        assert_ne!(&wrapped[12..], &dek[..]);
        let back = kek.unwrap_dek(&wrapped).await.unwrap();
        assert_eq!(&back[..], &dek[..]);
    }

    #[tokio::test]
    async fn different_master_keys_have_different_ids_and_cannot_unwrap() {
        let a = LocalKek::new(&[1u8; 32]).unwrap();
        let b = LocalKek::new(&[2u8; 32]).unwrap();
        assert_ne!(a.id(), b.id());
        let (_, wrapped) = a.generate_dek().await.unwrap();
        assert!(matches!(
            b.unwrap_dek(&wrapped).await,
            Err(CryptoError::Kek(_))
        ));
    }

    #[test]
    fn master_key_length_is_validated() {
        assert!(matches!(
            LocalKek::new(&[0u8; 16]),
            Err(CryptoError::Config(_))
        ));
    }

    #[test]
    fn debug_does_not_leak_key() {
        let dbg = format!("{:?}", LocalKek::new(&[1u8; 32]).unwrap());
        assert!(dbg.starts_with("LocalKek"));
        assert!(!dbg.contains("1, 1, 1"));
    }
}
