//! The two signed tokens push needs: APNs's provider token (ES256, the
//! team's `.p8` key) and the JWT a Google service account trades for an
//! access token (RS256).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;

fn segment(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(value.to_string())
}

/// `header.claims.signature`, signed by `sign` over `header.claims`.
fn token(
    header: &Value,
    claims: &Value,
    sign: impl FnOnce(&[u8]) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<String> {
    let signing_input = format!("{}.{}", segment(header), segment(claims));
    let signature = sign(signing_input.as_bytes())?;
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

/// An ES256 token: `key` a P-256 key in PKCS#8 PEM (Apple's `.p8`).
pub(crate) struct Es256 {
    key: p256::ecdsa::SigningKey,
}

impl Es256 {
    pub(crate) fn from_pem(pem: &str) -> anyhow::Result<Self> {
        use p256::pkcs8::DecodePrivateKey;
        let key = p256::ecdsa::SigningKey::from_pkcs8_pem(pem.trim())
            .map_err(|e| anyhow::anyhow!("not a P-256 key in PKCS#8 PEM: {e}"))?;
        Ok(Self { key })
    }

    pub(crate) fn sign(&self, header: &Value, claims: &Value) -> anyhow::Result<String> {
        use p256::ecdsa::signature::Signer;
        token(header, claims, |input| {
            let signature: p256::ecdsa::Signature = self.key.sign(input);
            // JOSE wants r and s, 32 bytes each, not DER.
            Ok(signature.to_bytes().to_vec())
        })
    }
}

/// An RS256 token: `key` an RSA key in PKCS#8 PEM (a service account's
/// `private_key`).
pub(crate) struct Rs256 {
    key: rsa::pkcs1v15::SigningKey<sha2::Sha256>,
}

impl Rs256 {
    pub(crate) fn from_pem(pem: &str) -> anyhow::Result<Self> {
        use rsa::pkcs8::DecodePrivateKey;
        let key = rsa::RsaPrivateKey::from_pkcs8_pem(pem.trim())
            .map_err(|e| anyhow::anyhow!("not an RSA key in PKCS#8 PEM: {e}"))?;
        Ok(Self {
            key: rsa::pkcs1v15::SigningKey::new(key),
        })
    }

    pub(crate) fn sign(&self, header: &Value, claims: &Value) -> anyhow::Result<String> {
        use rsa::signature::{SignatureEncoding, Signer};
        token(header, claims, |input| Ok(self.key.sign(input).to_vec()))
    }
}

/// Seconds since the epoch.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}
