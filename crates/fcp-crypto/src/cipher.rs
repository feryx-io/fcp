//! AES-256-GCM field cipher over a ring of data encryption keys.

use crate::{CryptoError, Result};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::RngCore;
use std::collections::HashMap;
use std::fmt;
use zeroize::Zeroizing;

/// Prefix of every sealed value; anything else is treated as legacy
/// plaintext by [`FieldCipher::open`].
pub const SEALED_PREFIX: &str = "enc:v1:";

const NONCE_LEN: usize = 12;
/// DEK length in bytes (AES-256).
pub const DEK_LEN: usize = 32;

/// Result of opening a stored value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The stored value was not sealed; returned unchanged. Callers that
    /// want to migrate lazily can re-seal on their next write.
    Plaintext(String),
    /// The stored value was sealed with `key_id` and decrypted.
    Decrypted { key_id: String, value: String },
}

impl Opened {
    /// The usable value either way.
    pub fn into_value(self) -> String {
        match self {
            Opened::Plaintext(v) | Opened::Decrypted { value: v, .. } => v,
        }
    }

    /// Whether the value was stored sealed with the given key.
    pub fn is_sealed_with(&self, key_id: &str) -> bool {
        matches!(self, Opened::Decrypted { key_id: k, .. } if k == key_id)
    }
}

/// Seals and opens field values with a ring of DEKs. Cheap to clone via
/// `Arc`; construct once per process from the persisted key ring.
pub struct FieldCipher {
    keys: HashMap<String, Aes256Gcm>,
    active: String,
}

impl fmt::Debug for FieldCipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut ids: Vec<&String> = self.keys.keys().collect();
        ids.sort();
        f.debug_struct("FieldCipher")
            .field("active", &self.active)
            .field("keys", &ids)
            .finish()
    }
}

impl FieldCipher {
    /// Build a cipher from `(key id, raw 32-byte DEK)` pairs; `active` must
    /// be one of them and is used for every new seal.
    pub fn new<I, K>(keys: I, active: &str) -> Result<Self>
    where
        I: IntoIterator<Item = (String, K)>,
        K: AsRef<[u8]>,
    {
        let mut map = HashMap::new();
        for (id, raw) in keys {
            map.insert(id, Self::aead(raw.as_ref())?);
        }
        if !map.contains_key(active) {
            return Err(CryptoError::UnknownKey(active.to_string()));
        }
        Ok(Self {
            keys: map,
            active: active.to_string(),
        })
    }

    /// A single-key cipher.
    pub fn single(key_id: &str, raw: &[u8]) -> Result<Self> {
        Self::new([(key_id.to_string(), raw)], key_id)
    }

    fn aead(raw: &[u8]) -> Result<Aes256Gcm> {
        if raw.len() != DEK_LEN {
            return Err(CryptoError::Config(format!(
                "data encryption key must be {DEK_LEN} bytes, got {}",
                raw.len()
            )));
        }
        Aes256Gcm::new_from_slice(raw).map_err(|_| CryptoError::Config("bad key length".into()))
    }

    /// Id of the DEK used for new seals.
    pub fn active_key_id(&self) -> &str {
        &self.active
    }

    /// Ids of every DEK this cipher can open, sorted.
    pub fn key_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.keys.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Add a DEK to the ring (for reads); does not change the active key.
    pub fn add_key(&mut self, key_id: String, raw: &[u8]) -> Result<()> {
        self.keys.insert(key_id, Self::aead(raw)?);
        Ok(())
    }

    /// Make an already-held DEK the active one.
    pub fn set_active(&mut self, key_id: &str) -> Result<()> {
        if !self.keys.contains_key(key_id) {
            return Err(CryptoError::UnknownKey(key_id.to_string()));
        }
        self.active = key_id.to_string();
        Ok(())
    }

    /// Whether a stored value is sealed (as opposed to legacy plaintext).
    pub fn is_sealed(value: &str) -> bool {
        value.starts_with(SEALED_PREFIX)
    }

    /// Seal a value with the active DEK. The key id is bound as associated
    /// data, so a ciphertext cannot be re-labelled to another key.
    pub fn seal(&self, plaintext: &str) -> Result<String> {
        self.seal_with(&self.active, plaintext)
    }

    fn seal_with(&self, key_id: &str, plaintext: &str) -> Result<String> {
        let aead = self
            .keys
            .get(key_id)
            .ok_or_else(|| CryptoError::UnknownKey(key_id.to_string()))?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);
        let ciphertext = aead
            .encrypt(
                &nonce,
                aes_gcm::aead::Payload {
                    msg: plaintext.as_bytes(),
                    aad: key_id.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::Decrypt)?;
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);
        Ok(format!("{SEALED_PREFIX}{key_id}:{}", B64.encode(blob)))
    }

    /// Open a stored value: decrypt when sealed, pass through otherwise.
    pub fn open(&self, stored: &str) -> Result<Opened> {
        let Some(rest) = stored.strip_prefix(SEALED_PREFIX) else {
            return Ok(Opened::Plaintext(stored.to_string()));
        };
        let (key_id, b64) = rest
            .split_once(':')
            .ok_or_else(|| CryptoError::Malformed("missing key id separator".into()))?;
        if key_id.is_empty() {
            return Err(CryptoError::Malformed("empty key id".into()));
        }
        let aead = self
            .keys
            .get(key_id)
            .ok_or_else(|| CryptoError::UnknownKey(key_id.to_string()))?;
        let blob = B64
            .decode(b64)
            .map_err(|e| CryptoError::Malformed(format!("base64: {e}")))?;
        if blob.len() < NONCE_LEN + 16 {
            return Err(CryptoError::Malformed("ciphertext too short".into()));
        }
        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::from(<[u8; NONCE_LEN]>::try_from(nonce_bytes).expect("length checked"));
        let plaintext = aead
            .decrypt(
                &nonce,
                aes_gcm::aead::Payload {
                    msg: ciphertext,
                    aad: key_id.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::Decrypt)?;
        let value = String::from_utf8(plaintext)
            .map_err(|_| CryptoError::Malformed("plaintext is not UTF-8".into()))?;
        Ok(Opened::Decrypted {
            key_id: key_id.to_string(),
            value,
        })
    }

    /// Open a value and, if it is plaintext or sealed with a non-active key,
    /// return the re-sealed form. `None` means the stored value is already
    /// current.
    pub fn reseal_if_stale(&self, stored: &str) -> Result<Option<String>> {
        match self.open(stored)? {
            Opened::Decrypted { key_id, .. } if key_id == self.active => Ok(None),
            opened => Ok(Some(self.seal(&opened.into_value())?)),
        }
    }
}

/// Generate a random 256-bit DEK.
pub fn generate_dek() -> Zeroizing<Vec<u8>> {
    let mut raw = Zeroizing::new(vec![0u8; DEK_LEN]);
    rand::rngs::OsRng.fill_bytes(&mut raw);
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> FieldCipher {
        FieldCipher::single("k1", &[7u8; 32]).unwrap()
    }

    #[test]
    fn seal_open_round_trip() {
        let c = cipher();
        let sealed = c.seal("s3cret").unwrap();
        assert!(sealed.starts_with("enc:v1:k1:"));
        assert!(FieldCipher::is_sealed(&sealed));
        assert_eq!(
            c.open(&sealed).unwrap(),
            Opened::Decrypted {
                key_id: "k1".into(),
                value: "s3cret".into()
            }
        );
        assert_eq!(c.open(&sealed).unwrap().into_value(), "s3cret");
    }

    #[test]
    fn seals_are_randomized() {
        let c = cipher();
        assert_ne!(c.seal("x").unwrap(), c.seal("x").unwrap());
    }

    #[test]
    fn plaintext_passes_through() {
        let c = cipher();
        assert_eq!(
            c.open("legacy-password").unwrap(),
            Opened::Plaintext("legacy-password".into())
        );
        assert!(!FieldCipher::is_sealed("legacy-password"));
        assert_eq!(c.open("").unwrap(), Opened::Plaintext(String::new()));
    }

    #[test]
    fn tampering_is_detected() {
        let c = cipher();
        let sealed = c.seal("s3cret").unwrap();
        let mut bytes = sealed.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(matches!(
            c.open(&tampered),
            Err(CryptoError::Decrypt) | Err(CryptoError::Malformed(_))
        ));
    }

    #[test]
    fn key_id_is_bound_as_aad() {
        let mut c = cipher();
        c.add_key("k2".into(), &[7u8; 32]).unwrap(); // same raw key, other id
        let sealed = c.seal("s3cret").unwrap();
        let relabelled = sealed.replacen("enc:v1:k1:", "enc:v1:k2:", 1);
        assert!(matches!(c.open(&relabelled), Err(CryptoError::Decrypt)));
    }

    #[test]
    fn wrong_key_fails() {
        let a = cipher();
        let b = FieldCipher::single("k1", &[9u8; 32]).unwrap();
        let sealed = a.seal("s3cret").unwrap();
        assert!(matches!(b.open(&sealed), Err(CryptoError::Decrypt)));
    }

    #[test]
    fn unknown_key_is_reported() {
        let a = cipher();
        let b = FieldCipher::single("other", &[7u8; 32]).unwrap();
        let sealed = a.seal("s3cret").unwrap();
        assert!(matches!(b.open(&sealed), Err(CryptoError::UnknownKey(k)) if k == "k1"));
    }

    #[test]
    fn malformed_values_are_rejected() {
        let c = cipher();
        assert!(matches!(c.open("enc:v1:"), Err(CryptoError::Malformed(_))));
        assert!(matches!(
            c.open("enc:v1:k1"),
            Err(CryptoError::Malformed(_))
        ));
        assert!(matches!(
            c.open("enc:v1::abc"),
            Err(CryptoError::Malformed(_))
        ));
        assert!(matches!(
            c.open("enc:v1:k1:!!!"),
            Err(CryptoError::Malformed(_))
        ));
        assert!(matches!(
            c.open("enc:v1:k1:QUJD"),
            Err(CryptoError::Malformed(_))
        ));
    }

    #[test]
    fn rotation_reads_old_and_seals_with_new() {
        let mut c = cipher();
        let old = c.seal("s3cret").unwrap();
        c.add_key("k2".into(), &[3u8; 32]).unwrap();
        c.set_active("k2").unwrap();
        assert_eq!(c.active_key_id(), "k2");
        assert_eq!(c.key_ids(), vec!["k1", "k2"]);

        assert_eq!(c.open(&old).unwrap().into_value(), "s3cret");
        let fresh = c.seal("s3cret").unwrap();
        assert!(fresh.starts_with("enc:v1:k2:"));

        // Stale (old key or plaintext) values get re-sealed; current ones don't.
        let resealed = c.reseal_if_stale(&old).unwrap().expect("old key is stale");
        assert!(resealed.starts_with("enc:v1:k2:"));
        assert!(c
            .reseal_if_stale("plain")
            .unwrap()
            .unwrap()
            .starts_with("enc:v1:k2:"));
        assert!(c.reseal_if_stale(&fresh).unwrap().is_none());
    }

    #[test]
    fn constructor_validates_keys() {
        assert!(matches!(
            FieldCipher::single("k", &[0u8; 16]),
            Err(CryptoError::Config(_))
        ));
        assert!(matches!(
            FieldCipher::new([("k".to_string(), [0u8; 32])], "missing"),
            Err(CryptoError::UnknownKey(_))
        ));
        let mut c = cipher();
        assert!(matches!(
            c.set_active("nope"),
            Err(CryptoError::UnknownKey(_))
        ));
    }

    #[test]
    fn debug_does_not_leak_keys() {
        let dbg = format!("{:?}", cipher());
        assert!(dbg.contains("k1"));
        assert!(!dbg.contains("7, 7, 7"));
    }
}
