//! STIR/SHAKEN caller ID verification and signing types (RFC 8224/8225/8588).
//!
//! Provides configuration types, PASSporT token structures, identity header
//! parsing and generation, a TTL-based certificate cache, and helper functions
//! for base64url encoding.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

// ── Configuration types ─────────────────────────────────────────────────────

/// Verification mode for inbound calls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StirVerifyMode {
    #[default]
    Disabled,
    /// Verify + log result, allow call regardless.
    LogOnly,
    /// Verify, reject with 438 on failure.
    Enforce,
}

/// Signing mode for outbound calls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StirSignMode {
    #[default]
    Disabled,
    Enabled,
}

/// Attestation level (RFC 8588 §5.2.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AttestationLevel {
    /// Full — originator known + authorized for calling number.
    #[default]
    A,
    /// Partial — originator known, can't verify number authorization.
    B,
    /// Gateway — no relationship with caller.
    C,
}

impl AttestationLevel {
    /// Return the single-character string representation used in PASSporT.
    pub fn as_str(&self) -> &'static str {
        match self {
            AttestationLevel::A => "A",
            AttestationLevel::B => "B",
            AttestationLevel::C => "C",
        }
    }

    /// Parse from the single-character string in a PASSporT claim.
    pub fn from_passport_str(s: &str) -> Option<Self> {
        match s {
            "A" => Some(AttestationLevel::A),
            "B" => Some(AttestationLevel::B),
            "C" => Some(AttestationLevel::C),
            _ => None,
        }
    }
}

fn default_max_iat_age() -> u64 {
    60
}

fn default_cert_cache_ttl() -> u64 {
    3600
}

fn default_cert_cache_max() -> usize {
    1000
}

fn default_cert_fetch_timeout() -> u64 {
    5
}

/// Per-trunk STIR/SHAKEN configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrunkStirShakenConfig {
    /// Verification mode for inbound calls on this trunk.
    #[serde(default)]
    pub verify_mode: StirVerifyMode,

    /// Signing mode for outbound calls on this trunk.
    #[serde(default)]
    pub sign_mode: StirSignMode,

    /// Attestation level to use when signing outbound calls.
    #[serde(default)]
    pub attestation: AttestationLevel,

    /// Maximum age of `iat` claim in seconds before treating as expired.
    #[serde(default = "default_max_iat_age")]
    pub max_iat_age_secs: u64,
}

impl Default for TrunkStirShakenConfig {
    fn default() -> Self {
        Self {
            verify_mode: StirVerifyMode::default(),
            sign_mode: StirSignMode::default(),
            attestation: AttestationLevel::default(),
            max_iat_age_secs: default_max_iat_age(),
        }
    }
}

impl TrunkStirShakenConfig {
    /// Validate the per-trunk STIR/SHAKEN configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_iat_age_secs == 0 || self.max_iat_age_secs > 600 {
            return Err("stir_shaken.max_iat_age_secs must be between 1 and 600".into());
        }
        Ok(())
    }
}

/// Global STIR/SHAKEN configuration (in SbcConfig).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StirShakenConfig {
    /// Master enable/disable toggle.
    #[serde(default)]
    pub enabled: bool,

    /// Path to PEM ECDSA P-256 private key for signing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_key_path: Option<String>,

    /// Path to PEM X.509 certificate for signing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_cert_path: Option<String>,

    /// HTTPS URL where the signing certificate can be fetched (used as x5u in PASSporT).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_cert_url: Option<String>,

    /// TTL for cached verifier certificates, in seconds.
    #[serde(default = "default_cert_cache_ttl")]
    pub cert_cache_ttl_secs: u64,

    /// Maximum number of cached certificates.
    #[serde(default = "default_cert_cache_max")]
    pub cert_cache_max_entries: usize,

    /// Timeout for fetching remote certificates, in seconds.
    #[serde(default = "default_cert_fetch_timeout")]
    pub cert_fetch_timeout_secs: u64,
}

impl Default for StirShakenConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            signing_key_path: None,
            signing_cert_path: None,
            signing_cert_url: None,
            cert_cache_ttl_secs: default_cert_cache_ttl(),
            cert_cache_max_entries: default_cert_cache_max(),
            cert_fetch_timeout_secs: default_cert_fetch_timeout(),
        }
    }
}

impl StirShakenConfig {
    /// Validate the global STIR/SHAKEN configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.cert_cache_ttl_secs == 0 || self.cert_cache_ttl_secs > 86400 {
            return Err("stir_shaken.cert_cache_ttl_secs must be between 1 and 86400".into());
        }
        if self.cert_cache_max_entries == 0 || self.cert_cache_max_entries > 100_000 {
            return Err("stir_shaken.cert_cache_max_entries must be between 1 and 100000".into());
        }
        if self.cert_fetch_timeout_secs == 0 || self.cert_fetch_timeout_secs > 30 {
            return Err("stir_shaken.cert_fetch_timeout_secs must be between 1 and 30".into());
        }

        // If any signing field is set, all three must be present
        let has_key = self.signing_key_path.is_some();
        let has_cert = self.signing_cert_path.is_some();
        let has_url = self.signing_cert_url.is_some();
        let signing_count = [has_key, has_cert, has_url].iter().filter(|&&v| v).count();
        if signing_count > 0 && signing_count < 3 {
            return Err(
                "stir_shaken: signing requires all three fields: signing_key_path, \
                 signing_cert_path, and signing_cert_url"
                    .into(),
            );
        }

        Ok(())
    }
}

// ── PASSporT types ──────────────────────────────────────────────────────────

/// JOSE header for a SHAKEN PASSporT token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassportHeader {
    pub alg: String,
    pub ppt: String,
    pub typ: String,
    pub x5u: String,
}

/// Telephone number field in PASSporT claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassportTnField {
    pub tn: Vec<String>,
}

/// PASSporT payload claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassportPayload {
    pub attest: String,
    pub dest: PassportTnField,
    pub iat: u64,
    pub orig: PassportTnField,
    pub origid: String,
}

/// Parsed Identity header components.
#[derive(Debug, Clone)]
pub struct ParsedIdentity {
    /// The compact JWS token (header.payload.signature).
    pub token: String,
    /// The `info` parameter URL (certificate location).
    pub info_url: String,
    /// The decoded JOSE header.
    pub header: PassportHeader,
    /// The decoded PASSporT payload.
    pub payload: PassportPayload,
    /// Raw header bytes (base64url-encoded).
    pub raw_header: String,
    /// Raw payload bytes (base64url-encoded).
    pub raw_payload: String,
    /// Raw signature bytes (base64url-encoded).
    pub raw_signature: String,
}

// ── Error types ─────────────────────────────────────────────────────────────

/// Errors that can occur during STIR/SHAKEN verification or signing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum StirError {
    #[allow(dead_code)]
    #[error("no Identity header present")]
    NoIdentityHeader,

    #[error("malformed Identity header: {0}")]
    MalformedHeader(String),

    #[error("invalid PASSporT token: {0}")]
    InvalidToken(String),

    #[error("certificate fetch failed: {0}")]
    CertificateFetchFailed(String),

    #[error("certificate invalid: {0}")]
    CertificateInvalid(String),

    #[error("signature verification failed")]
    SignatureInvalid,

    #[error("claims invalid: {0}")]
    ClaimsInvalid(String),

    #[error("iat expired: age {age_secs}s exceeds max {max_secs}s")]
    IatExpired { age_secs: u64, max_secs: u64 },

    #[error("orig TN mismatch: passport={passport_tn}, from={from_tn}")]
    OrigTnMismatch {
        passport_tn: String,
        from_tn: String,
    },

    #[error("dest TN mismatch: passport={passport_tn}, to={to_tn}")]
    DestTnMismatch { passport_tn: String, to_tn: String },

    #[error("signing error: {0}")]
    SigningError(String),
}

// ── Base64url helpers ───────────────────────────────────────────────────────

/// Encode bytes as base64url without padding.
pub fn base64url_encode(data: &[u8]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD.encode(data)
}

/// Decode base64url (no padding) to bytes.
pub fn base64url_decode(data: &str) -> Result<Vec<u8>, StirError> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD
        .decode(data)
        .map_err(|e| StirError::InvalidToken(format!("base64url decode error: {e}")))
}

// ── TN cleaning ─────────────────────────────────────────────────────────────

/// Extract a clean telephone number from a SIP URI, tel URI, or raw string.
///
/// Examples:
/// - `sip:+15551234@host` → `+15551234`
/// - `tel:+15551234` → `+15551234`
/// - `+1 (555) 123-4567` → `+15551234567`
/// - `15551234567` → `15551234567`
pub fn clean_tn(input: &str) -> String {
    let input = input.trim();

    // Handle sip: or sips: URI — extract user part
    if let Some(rest) = input
        .strip_prefix("sip:")
        .or_else(|| input.strip_prefix("sips:"))
    {
        // user@host or just user
        let user = rest.split('@').next().unwrap_or(rest);
        return strip_non_tn_chars(user);
    }

    // Handle tel: URI
    if let Some(rest) = input.strip_prefix("tel:") {
        // Remove phone-context and other params
        let number = rest.split(';').next().unwrap_or(rest);
        return strip_non_tn_chars(number);
    }

    // Handle <sip:...> or <tel:...> with angle brackets
    if input.starts_with('<') && input.contains('>') {
        let inner = input
            .trim_start_matches('<')
            .split('>')
            .next()
            .unwrap_or(input);
        return clean_tn(inner);
    }

    // Raw number — strip non-digit/non-plus chars
    strip_non_tn_chars(input)
}

/// Keep only digits and leading '+'.
fn strip_non_tn_chars(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for (i, ch) in s.chars().enumerate() {
        if (ch == '+' && i == 0) || ch.is_ascii_digit() {
            result.push(ch);
        }
    }
    result
}

// ── Identity header parsing ─────────────────────────────────────────────────

/// Parse a SIP Identity header value into its components.
///
/// Expected format:
/// `<compact-jws>;info=<cert-url>;alg=ES256;ppt=shaken`
///
/// The compact JWS is three base64url segments separated by dots:
/// `header.payload.signature`
pub fn parse_identity_header(header_value: &str) -> Result<ParsedIdentity, StirError> {
    let header_value = header_value.trim();

    // Split on first ';' to separate token from parameters
    let (token, params_str) = header_value
        .split_once(';')
        .ok_or_else(|| StirError::MalformedHeader("missing parameters after token".into()))?;

    let token = token.trim().to_string();

    // Parse parameters
    let mut info_url = None;
    for param in params_str.split(';') {
        let param = param.trim();
        if let Some(value) = param.strip_prefix("info=") {
            // Remove angle brackets if present
            let url = value.trim_matches(|c| c == '<' || c == '>');
            info_url = Some(url.to_string());
        }
    }

    let info_url =
        info_url.ok_or_else(|| StirError::MalformedHeader("missing info parameter".into()))?;

    // Split JWS into three parts
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(StirError::MalformedHeader(format!(
            "JWS must have 3 parts, got {}",
            parts.len()
        )));
    }

    let raw_header = parts[0].to_string();
    let raw_payload = parts[1].to_string();
    let raw_signature = parts[2].to_string();

    // Decode and parse header
    let header_bytes = base64url_decode(&raw_header)?;
    let header: PassportHeader = serde_json::from_slice(&header_bytes)
        .map_err(|e| StirError::InvalidToken(format!("header JSON parse error: {e}")))?;

    // Decode and parse payload
    let payload_bytes = base64url_decode(&raw_payload)?;
    let payload: PassportPayload = serde_json::from_slice(&payload_bytes)
        .map_err(|e| StirError::InvalidToken(format!("payload JSON parse error: {e}")))?;

    Ok(ParsedIdentity {
        token,
        info_url,
        header,
        payload,
        raw_header,
        raw_payload,
        raw_signature,
    })
}

/// Generate a SIP Identity header value from components.
///
/// Returns the complete Identity header value including the JWS token and
/// the info/alg/ppt parameters.
pub fn generate_identity_header(
    orig_tn: &str,
    dest_tn: &str,
    attestation: AttestationLevel,
    cert_url: &str,
    signing_key: &p256::ecdsa::SigningKey,
) -> Result<String, StirError> {
    use p256::ecdsa::{signature::Signer, Signature};

    let header = PassportHeader {
        alg: "ES256".into(),
        ppt: "shaken".into(),
        typ: "passport".into(),
        x5u: cert_url.to_string(),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let payload = PassportPayload {
        attest: attestation.as_str().to_string(),
        dest: PassportTnField {
            tn: vec![dest_tn.to_string()],
        },
        iat: now,
        orig: PassportTnField {
            tn: vec![orig_tn.to_string()],
        },
        origid: uuid::Uuid::new_v4().to_string(),
    };

    let header_json = serde_json::to_string(&header)
        .map_err(|e| StirError::SigningError(format!("header serialization: {e}")))?;
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| StirError::SigningError(format!("payload serialization: {e}")))?;

    let header_b64 = base64url_encode(header_json.as_bytes());
    let payload_b64 = base64url_encode(payload_json.as_bytes());

    // Sign header.payload
    let signing_input = format!("{}.{}", header_b64, payload_b64);
    let signature: Signature = signing_key
        .try_sign(signing_input.as_bytes())
        .map_err(|e| StirError::SigningError(format!("ES256 signing failed: {e}")))?;

    let sig_b64 = base64url_encode(&signature.to_bytes());

    // Build Identity header value
    Ok(format!(
        "{}.{}.{};info=<{}>;alg=ES256;ppt=shaken",
        header_b64, payload_b64, sig_b64, cert_url
    ))
}

// ── Certificate cache ───────────────────────────────────────────────────────

/// A cached certificate entry.
struct CachedCert {
    /// The raw DER-encoded public key bytes (SPKI).
    pub_key_bytes: Vec<u8>,
    /// When this entry was inserted.
    inserted_at: Instant,
}

/// TTL-based cache for fetched verifier certificates.
///
/// Maps certificate URLs to their extracted public key bytes.
/// Thread-safe via `DashMap`.
pub struct CertificateCache {
    entries: DashMap<String, CachedCert>,
    ttl: Duration,
    max_entries: usize,
}

impl CertificateCache {
    /// Create a new certificate cache.
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: DashMap::new(),
            ttl,
            max_entries,
        }
    }

    /// Get cached public key bytes for a URL, if present and not expired.
    pub fn get(&self, url: &str) -> Option<Vec<u8>> {
        let entry = self.entries.get(url)?;
        if entry.inserted_at.elapsed() > self.ttl {
            drop(entry);
            self.entries.remove(url);
            None
        } else {
            Some(entry.pub_key_bytes.clone())
        }
    }

    /// Insert a certificate's public key bytes into the cache.
    ///
    /// If the cache is at capacity, expired entries are evicted first.
    /// If still at capacity after eviction, the oldest entry is removed.
    pub fn insert(&self, url: String, pub_key_bytes: Vec<u8>) {
        if self.entries.len() >= self.max_entries {
            self.evict_expired();
        }
        // If still at capacity, remove oldest
        if self.entries.len() >= self.max_entries {
            if let Some(oldest_key) = self.oldest_key() {
                self.entries.remove(&oldest_key);
            }
        }
        self.entries.insert(
            url,
            CachedCert {
                pub_key_bytes,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Remove all expired entries.
    pub fn evict_expired(&self) {
        self.entries
            .retain(|_, entry| entry.inserted_at.elapsed() <= self.ttl);
    }

    /// Return the number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Find the key of the oldest entry.
    fn oldest_key(&self) -> Option<String> {
        let mut oldest: Option<(String, Instant)> = None;
        for entry in self.entries.iter() {
            match &oldest {
                None => oldest = Some((entry.key().clone(), entry.value().inserted_at)),
                Some((_, oldest_time)) => {
                    if entry.value().inserted_at < *oldest_time {
                        oldest = Some((entry.key().clone(), entry.value().inserted_at));
                    }
                }
            }
        }
        oldest.map(|(key, _)| key)
    }
}

/// Quick helper to extract a header value from raw SIP payload bytes.
///
/// Scans lines for a header matching the given name (case-insensitive).
/// Returns the trimmed value after the colon.
pub fn extract_header_from_payload(payload: &[u8], header_name: &str) -> Option<String> {
    let text = std::str::from_utf8(payload).ok()?;
    let search_lower = format!("{}:", header_name.to_ascii_lowercase());

    for line in text.lines().skip(1) {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with(&search_lower) {
            let value = &trimmed[search_lower.len()..];
            return Some(value.trim().to_string());
        }
    }
    None
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- Config defaults and validation ---

    #[test]
    fn test_stir_config_defaults() {
        let global = StirShakenConfig::default();
        assert!(!global.enabled);
        assert!(global.signing_key_path.is_none());
        assert!(global.signing_cert_path.is_none());
        assert!(global.signing_cert_url.is_none());
        assert_eq!(global.cert_cache_ttl_secs, 3600);
        assert_eq!(global.cert_cache_max_entries, 1000);
        assert_eq!(global.cert_fetch_timeout_secs, 5);

        let trunk = TrunkStirShakenConfig::default();
        assert_eq!(trunk.verify_mode, StirVerifyMode::Disabled);
        assert_eq!(trunk.sign_mode, StirSignMode::Disabled);
        assert_eq!(trunk.attestation, AttestationLevel::A);
        assert_eq!(trunk.max_iat_age_secs, 60);
    }

    #[test]
    fn test_stir_config_validation_ranges() {
        let mut cfg = StirShakenConfig {
            cert_cache_ttl_secs: 0,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
        cfg.cert_cache_ttl_secs = 86401;
        assert!(cfg.validate().is_err());
        cfg.cert_cache_ttl_secs = 3600;
        assert!(cfg.validate().is_ok());

        // cert_cache_max_entries
        cfg.cert_cache_max_entries = 0;
        assert!(cfg.validate().is_err());
        cfg.cert_cache_max_entries = 100_001;
        assert!(cfg.validate().is_err());
        cfg.cert_cache_max_entries = 1000;
        assert!(cfg.validate().is_ok());

        // cert_fetch_timeout_secs
        cfg.cert_fetch_timeout_secs = 0;
        assert!(cfg.validate().is_err());
        cfg.cert_fetch_timeout_secs = 31;
        assert!(cfg.validate().is_err());
        cfg.cert_fetch_timeout_secs = 5;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_stir_config_signing_field_consistency() {
        let mut cfg = StirShakenConfig {
            signing_key_path: Some("/path/to/key.pem".into()),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());

        // Key + cert but no URL — should fail
        cfg.signing_cert_path = Some("/path/to/cert.pem".into());
        assert!(cfg.validate().is_err());

        // All three set — should pass
        cfg.signing_cert_url = Some("https://example.com/cert.pem".into());
        assert!(cfg.validate().is_ok());

        // None set — should pass
        let cfg = StirShakenConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_trunk_stir_config_validation() {
        let mut cfg = TrunkStirShakenConfig::default();
        assert!(cfg.validate().is_ok());

        cfg.max_iat_age_secs = 0;
        assert!(cfg.validate().is_err());

        cfg.max_iat_age_secs = 601;
        assert!(cfg.validate().is_err());

        cfg.max_iat_age_secs = 1;
        assert!(cfg.validate().is_ok());

        cfg.max_iat_age_secs = 600;
        assert!(cfg.validate().is_ok());
    }

    // --- Attestation serde ---

    #[test]
    fn test_attestation_serde() {
        let levels = vec![
            AttestationLevel::A,
            AttestationLevel::B,
            AttestationLevel::C,
        ];
        for level in levels {
            let json = serde_json::to_string(&level).unwrap();
            let deserialized: AttestationLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, deserialized);
        }
    }

    #[test]
    fn test_attestation_as_str() {
        assert_eq!(AttestationLevel::A.as_str(), "A");
        assert_eq!(AttestationLevel::B.as_str(), "B");
        assert_eq!(AttestationLevel::C.as_str(), "C");
    }

    #[test]
    fn test_attestation_from_passport_str() {
        assert_eq!(
            AttestationLevel::from_passport_str("A"),
            Some(AttestationLevel::A)
        );
        assert_eq!(
            AttestationLevel::from_passport_str("B"),
            Some(AttestationLevel::B)
        );
        assert_eq!(
            AttestationLevel::from_passport_str("C"),
            Some(AttestationLevel::C)
        );
        assert_eq!(AttestationLevel::from_passport_str("X"), None);
    }

    // --- Identity header parsing ---

    #[test]
    fn test_parse_identity_header_valid() {
        let header_json = serde_json::json!({
            "alg": "ES256",
            "ppt": "shaken",
            "typ": "passport",
            "x5u": "https://cert.example.com/cert.pem"
        });
        let payload_json = serde_json::json!({
            "attest": "A",
            "dest": { "tn": ["+15551002"] },
            "iat": 1700000000u64,
            "orig": { "tn": ["+15551001"] },
            "origid": "550e8400-e29b-41d4-a716-446655440000"
        });

        let header_b64 = base64url_encode(serde_json::to_string(&header_json).unwrap().as_bytes());
        let payload_b64 =
            base64url_encode(serde_json::to_string(&payload_json).unwrap().as_bytes());
        let sig_b64 = base64url_encode(b"fake-signature-bytes");

        let identity_value = format!(
            "{}.{}.{};info=<https://cert.example.com/cert.pem>;alg=ES256;ppt=shaken",
            header_b64, payload_b64, sig_b64
        );

        let parsed = parse_identity_header(&identity_value).unwrap();
        assert_eq!(parsed.info_url, "https://cert.example.com/cert.pem");
        assert_eq!(parsed.header.alg, "ES256");
        assert_eq!(parsed.header.ppt, "shaken");
        assert_eq!(parsed.payload.attest, "A");
        assert_eq!(parsed.payload.orig.tn, vec!["+15551001"]);
        assert_eq!(parsed.payload.dest.tn, vec!["+15551002"]);
        assert_eq!(parsed.payload.iat, 1700000000);
    }

    #[test]
    fn test_parse_identity_header_missing_info() {
        let header_b64 = base64url_encode(b"{}");
        let payload_b64 = base64url_encode(b"{}");
        let sig_b64 = base64url_encode(b"sig");

        let identity_value = format!(
            "{}.{}.{};alg=ES256;ppt=shaken",
            header_b64, payload_b64, sig_b64
        );

        let result = parse_identity_header(&identity_value);
        assert!(result.is_err());
        match result.unwrap_err() {
            StirError::MalformedHeader(msg) => assert!(msg.contains("info")),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn test_parse_identity_header_malformed_jwt() {
        // Only two parts instead of three
        let identity_value = "part1.part2;info=<https://example.com>;alg=ES256";
        let result = parse_identity_header(identity_value);
        assert!(result.is_err());
        match result.unwrap_err() {
            StirError::MalformedHeader(msg) => assert!(msg.contains("3 parts")),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn test_parse_identity_header_no_params() {
        let identity_value = "header.payload.signature";
        let result = parse_identity_header(identity_value);
        assert!(result.is_err());
        match result.unwrap_err() {
            StirError::MalformedHeader(msg) => assert!(msg.contains("missing parameters")),
            other => panic!("unexpected error: {other}"),
        }
    }

    // --- clean_tn ---

    #[test]
    fn test_clean_tn_sip_uri() {
        assert_eq!(clean_tn("sip:+15551234@host.example.com"), "+15551234");
    }

    #[test]
    fn test_clean_tn_sips_uri() {
        assert_eq!(clean_tn("sips:+15551234@host.example.com"), "+15551234");
    }

    #[test]
    fn test_clean_tn_tel_uri() {
        assert_eq!(clean_tn("tel:+15551234"), "+15551234");
    }

    #[test]
    fn test_clean_tn_tel_uri_with_context() {
        assert_eq!(
            clean_tn("tel:+15551234;phone-context=example.com"),
            "+15551234"
        );
    }

    #[test]
    fn test_clean_tn_digits_only() {
        assert_eq!(clean_tn("+1 (555) 123-4567"), "+15551234567");
    }

    #[test]
    fn test_clean_tn_raw_digits() {
        assert_eq!(clean_tn("15551234567"), "15551234567");
    }

    #[test]
    fn test_clean_tn_angle_brackets() {
        assert_eq!(clean_tn("<sip:+15551234@host.example.com>"), "+15551234");
    }

    // --- Base64url ---

    #[test]
    fn test_base64url_roundtrip() {
        let data = b"Hello, STIR/SHAKEN!";
        let encoded = base64url_encode(data);
        let decoded = base64url_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_base64url_no_padding() {
        let encoded = base64url_encode(b"abc");
        assert!(!encoded.contains('='));
    }

    // --- Certificate cache ---

    #[test]
    fn test_cert_cache_basic() {
        let cache = CertificateCache::new(Duration::from_secs(60), 10);
        assert!(cache.is_empty());

        cache.insert("https://example.com/cert".into(), vec![1, 2, 3]);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get("https://example.com/cert"), Some(vec![1, 2, 3]));
        assert_eq!(cache.get("https://other.com/cert"), None);
    }

    #[test]
    fn test_cert_cache_ttl_expiry() {
        let cache = CertificateCache::new(Duration::from_millis(1), 10);
        cache.insert("https://example.com/cert".into(), vec![1, 2, 3]);

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(cache.get("https://example.com/cert"), None);
    }

    #[test]
    fn test_cert_cache_max_entries() {
        let cache = CertificateCache::new(Duration::from_secs(60), 3);

        cache.insert("url1".into(), vec![1]);
        cache.insert("url2".into(), vec![2]);
        cache.insert("url3".into(), vec![3]);
        assert_eq!(cache.len(), 3);

        // Adding a 4th should evict one
        cache.insert("url4".into(), vec![4]);
        assert_eq!(cache.len(), 3);
        assert!(cache.get("url4").is_some());
    }

    #[test]
    fn test_cert_cache_clear() {
        let cache = CertificateCache::new(Duration::from_secs(60), 10);
        cache.insert("url1".into(), vec![1]);
        cache.insert("url2".into(), vec![2]);
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cert_cache_evict_expired() {
        let cache = CertificateCache::new(Duration::from_millis(1), 10);
        cache.insert("url1".into(), vec![1]);
        cache.insert("url2".into(), vec![2]);

        std::thread::sleep(Duration::from_millis(10));
        cache.evict_expired();
        assert!(cache.is_empty());
    }

    // --- Generate + parse roundtrip ---

    #[test]
    fn test_generate_and_parse_roundtrip() {
        use p256::ecdsa::SigningKey;
        use rand::rngs::OsRng;

        let signing_key = SigningKey::random(&mut OsRng);
        let cert_url = "https://cert.example.com/cert.pem";

        let header_value = generate_identity_header(
            "+15551001",
            "+15551002",
            AttestationLevel::A,
            cert_url,
            &signing_key,
        )
        .unwrap();

        let parsed = parse_identity_header(&header_value).unwrap();
        assert_eq!(parsed.info_url, cert_url);
        assert_eq!(parsed.header.alg, "ES256");
        assert_eq!(parsed.header.ppt, "shaken");
        assert_eq!(parsed.header.typ, "passport");
        assert_eq!(parsed.payload.attest, "A");
        assert_eq!(parsed.payload.orig.tn, vec!["+15551001"]);
        assert_eq!(parsed.payload.dest.tn, vec!["+15551002"]);
        assert!(!parsed.payload.origid.is_empty());
    }

    // --- Sign + verify roundtrip ---

    #[test]
    fn test_sign_verify_roundtrip() {
        use p256::ecdsa::{signature::Verifier, Signature, SigningKey, VerifyingKey};
        use rand::rngs::OsRng;

        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);
        let cert_url = "https://cert.example.com/cert.pem";

        let header_value = generate_identity_header(
            "+15551001",
            "+15551002",
            AttestationLevel::B,
            cert_url,
            &signing_key,
        )
        .unwrap();

        let parsed = parse_identity_header(&header_value).unwrap();

        // Reconstruct signing input
        let signing_input = format!("{}.{}", parsed.raw_header, parsed.raw_payload);

        // Decode signature
        let sig_bytes = base64url_decode(&parsed.raw_signature).unwrap();
        let signature = Signature::from_bytes((&sig_bytes[..]).into()).unwrap();

        // Verify
        assert!(verifying_key
            .verify(signing_input.as_bytes(), &signature)
            .is_ok());
    }

    // --- extract_header_from_payload ---

    #[test]
    fn test_extract_header_from_payload() {
        let payload = b"INVITE sip:bob@example.com SIP/2.0\r\n\
            Identity: eyJ0eXAiOiJwYXNzcG9ydCI.payload.sig;info=<url>;alg=ES256;ppt=shaken\r\n\
            From: <sip:+15551001@example.com>;tag=abc\r\n\
            To: <sip:+15551002@example.com>\r\n\
            Call-ID: test123\r\n\
            CSeq: 1 INVITE\r\n\
            \r\n";

        let identity = extract_header_from_payload(payload, "Identity");
        assert!(identity.is_some());
        assert!(identity.unwrap().starts_with("eyJ0eXAiOiJwYXNzcG9ydCI"));

        let from = extract_header_from_payload(payload, "From");
        assert!(from.is_some());
        assert!(from.unwrap().contains("+15551001"));

        let missing = extract_header_from_payload(payload, "X-Nonexistent");
        assert!(missing.is_none());
    }

    // --- Serde roundtrip for configs ---

    #[test]
    fn test_global_config_serde_roundtrip() {
        let cfg = StirShakenConfig {
            enabled: true,
            signing_key_path: Some("/path/key.pem".into()),
            signing_cert_path: Some("/path/cert.pem".into()),
            signing_cert_url: Some("https://example.com/cert.pem".into()),
            cert_cache_ttl_secs: 1800,
            cert_cache_max_entries: 500,
            cert_fetch_timeout_secs: 10,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: StirShakenConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_trunk_config_serde_roundtrip() {
        let cfg = TrunkStirShakenConfig {
            verify_mode: StirVerifyMode::Enforce,
            sign_mode: StirSignMode::Enabled,
            attestation: AttestationLevel::B,
            max_iat_age_secs: 120,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: TrunkStirShakenConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_verify_mode_serde() {
        let modes = vec![
            StirVerifyMode::Disabled,
            StirVerifyMode::LogOnly,
            StirVerifyMode::Enforce,
        ];
        for mode in modes {
            let json = serde_json::to_string(&mode).unwrap();
            let deserialized: StirVerifyMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, deserialized);
        }
    }

    #[test]
    fn test_sign_mode_serde() {
        let modes = vec![StirSignMode::Disabled, StirSignMode::Enabled];
        for mode in modes {
            let json = serde_json::to_string(&mode).unwrap();
            let deserialized: StirSignMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, deserialized);
        }
    }
}
