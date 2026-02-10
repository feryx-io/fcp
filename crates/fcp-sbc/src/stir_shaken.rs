//! STIR/SHAKEN verification engine, signing engine, transport dispatcher,
//! and metrics for the SBC (RFC 8224/8225/8588).

use bytes::Bytes;
use fcp_sbc_core::stir_shaken::{
    self, AttestationLevel, CertificateCache, ParsedIdentity, StirError, StirShakenConfig,
    TrunkStirShakenConfig,
};
use fcp_sbc_core::TrunkConfig;
use p256::ecdsa::{signature::Verifier, Signature, SigningKey, VerifyingKey};
use sip_transaction::{TransportContext, TransportDispatcher};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

// ── Verification result ─────────────────────────────────────────────────────

/// Result of verifying an inbound Identity header.
#[derive(Debug)]
pub enum VerificationResult {
    /// Verification succeeded with the given attestation level.
    Verified { attestation: AttestationLevel },
    /// No Identity header was present.
    #[allow(dead_code)]
    NoIdentity,
    /// Verification failed.
    Failed(StirError),
}

// ── StirVerifier ────────────────────────────────────────────────────────────

/// Async verification engine for inbound STIR/SHAKEN Identity headers.
pub struct StirVerifier {
    http_client: reqwest::Client,
    cert_cache: CertificateCache,
}

impl StirVerifier {
    /// Create a new verifier with an HTTP client and certificate cache.
    pub fn new(config: &StirShakenConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.cert_fetch_timeout_secs))
            .build()
            .unwrap_or_default();

        let cert_cache = CertificateCache::new(
            Duration::from_secs(config.cert_cache_ttl_secs),
            config.cert_cache_max_entries,
        );

        Self {
            http_client,
            cert_cache,
        }
    }

    /// Verify an Identity header value against the From and To telephone numbers.
    pub async fn verify(
        &self,
        identity_header: &str,
        from_tn: &str,
        to_tn: &str,
        max_iat_age_secs: u64,
    ) -> VerificationResult {
        // 1. Parse Identity header
        let parsed = match stir_shaken::parse_identity_header(identity_header) {
            Ok(p) => p,
            Err(e) => return VerificationResult::Failed(e),
        };

        // 2. Fetch/cache certificate → extract VerifyingKey
        let verifying_key = match self.fetch_verifying_key(&parsed.info_url).await {
            Ok(key) => key,
            Err(e) => return VerificationResult::Failed(e),
        };

        // 3. Verify ES256 signature
        if let Err(e) = self.verify_signature(&parsed, &verifying_key) {
            return VerificationResult::Failed(e);
        }

        // 4. Validate claims
        if let Err(e) = self.validate_claims(&parsed, from_tn, to_tn, max_iat_age_secs) {
            return VerificationResult::Failed(e);
        }

        // 5. Extract attestation level
        let attestation =
            AttestationLevel::from_passport_str(&parsed.payload.attest).unwrap_or(AttestationLevel::C);

        VerificationResult::Verified { attestation }
    }

    /// Fetch (or retrieve from cache) the VerifyingKey for a certificate URL.
    async fn fetch_verifying_key(&self, url: &str) -> Result<VerifyingKey, StirError> {
        // Check cache
        if let Some(key_bytes) = self.cert_cache.get(url) {
            return VerifyingKey::from_sec1_bytes(&key_bytes).map_err(|e| {
                StirError::CertificateInvalid(format!("cached key decode failed: {e}"))
            });
        }

        // Fetch certificate
        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| StirError::CertificateFetchFailed(format!("{e}")))?;

        if !response.status().is_success() {
            return Err(StirError::CertificateFetchFailed(format!(
                "HTTP {}",
                response.status()
            )));
        }

        let cert_bytes = response
            .bytes()
            .await
            .map_err(|e| StirError::CertificateFetchFailed(format!("body read: {e}")))?;

        // Extract public key
        let verifying_key = extract_verifying_key_from_cert(&cert_bytes)?;

        // Cache the public key bytes
        let key_bytes = verifying_key.to_encoded_point(false);
        self.cert_cache
            .insert(url.to_string(), key_bytes.as_bytes().to_vec());

        Ok(verifying_key)
    }

    /// Verify the ES256 signature over the JWS signing input.
    fn verify_signature(
        &self,
        parsed: &ParsedIdentity,
        verifying_key: &VerifyingKey,
    ) -> Result<(), StirError> {
        let signing_input = format!("{}.{}", parsed.raw_header, parsed.raw_payload);

        let sig_bytes = stir_shaken::base64url_decode(&parsed.raw_signature)?;
        let signature = Signature::from_bytes((&sig_bytes[..]).into())
            .map_err(|_| StirError::SignatureInvalid)?;

        verifying_key
            .verify(signing_input.as_bytes(), &signature)
            .map_err(|_| StirError::SignatureInvalid)
    }

    /// Validate PASSporT claims (iat freshness, orig/dest TN).
    fn validate_claims(
        &self,
        parsed: &ParsedIdentity,
        from_tn: &str,
        to_tn: &str,
        max_iat_age_secs: u64,
    ) -> Result<(), StirError> {
        // Check iat freshness
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let age = now.saturating_sub(parsed.payload.iat);
        if age > max_iat_age_secs {
            return Err(StirError::IatExpired {
                age_secs: age,
                max_secs: max_iat_age_secs,
            });
        }

        // Check orig TN
        let from_clean = stir_shaken::clean_tn(from_tn);
        if let Some(passport_orig) = parsed.payload.orig.tn.first() {
            let passport_clean = stir_shaken::clean_tn(passport_orig);
            if passport_clean != from_clean {
                return Err(StirError::OrigTnMismatch {
                    passport_tn: passport_clean,
                    from_tn: from_clean,
                });
            }
        } else {
            return Err(StirError::ClaimsInvalid("orig.tn is empty".into()));
        }

        // Check dest TN
        let to_clean = stir_shaken::clean_tn(to_tn);
        if let Some(passport_dest) = parsed.payload.dest.tn.first() {
            let passport_clean = stir_shaken::clean_tn(passport_dest);
            if passport_clean != to_clean {
                return Err(StirError::DestTnMismatch {
                    passport_tn: passport_clean,
                    to_tn: to_clean,
                });
            }
        } else {
            return Err(StirError::ClaimsInvalid("dest.tn is empty".into()));
        }

        Ok(())
    }

    /// Access the internal certificate cache (for admin API).
    pub fn cache(&self) -> &CertificateCache {
        &self.cert_cache
    }
}

// ── Certificate key extraction ──────────────────────────────────────────────

/// Extract a P-256 VerifyingKey from a PEM or DER certificate.
///
/// Handles PEM-encoded X.509 certificates by finding the CERTIFICATE block,
/// parsing the DER content with x509-parser, and extracting the ECDSA P-256
/// public key from the SubjectPublicKeyInfo.
fn extract_verifying_key_from_cert(cert_bytes: &[u8]) -> Result<VerifyingKey, StirError> {
    // Check if PEM
    let der_bytes = if let Ok(text) = std::str::from_utf8(cert_bytes) {
        if text.contains("-----BEGIN CERTIFICATE-----") {
            // Extract base64 content between PEM markers
            let start = text
                .find("-----BEGIN CERTIFICATE-----")
                .ok_or_else(|| StirError::CertificateInvalid("missing PEM begin".into()))?
                + "-----BEGIN CERTIFICATE-----".len();
            let end = text
                .find("-----END CERTIFICATE-----")
                .ok_or_else(|| StirError::CertificateInvalid("missing PEM end".into()))?;

            let b64_content: String = text[start..end]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();

            use base64::engine::general_purpose::STANDARD;
            use base64::Engine;
            STANDARD
                .decode(&b64_content)
                .map_err(|e| StirError::CertificateInvalid(format!("PEM decode: {e}")))?
        } else {
            cert_bytes.to_vec()
        }
    } else {
        cert_bytes.to_vec()
    };

    // Parse X.509 with x509-parser
    let (_, cert) = x509_parser::parse_x509_certificate(&der_bytes)
        .map_err(|e| StirError::CertificateInvalid(format!("X.509 parse: {e}")))?;

    // Extract public key from SPKI
    let spki = cert.public_key();
    let key_bytes = &spki.subject_public_key.data;

    VerifyingKey::from_sec1_bytes(key_bytes.as_ref())
        .map_err(|e| StirError::CertificateInvalid(format!("P-256 key extract: {e}")))
}

// ── StirSigner ──────────────────────────────────────────────────────────────

/// Signing engine for outbound STIR/SHAKEN Identity headers.
pub struct StirSigner {
    signing_key: SigningKey,
    cert_url: String,
}

impl StirSigner {
    /// Create a new signer from the global STIR/SHAKEN config.
    ///
    /// Returns `Ok(None)` if signing is not configured (no key/cert/url).
    /// Returns `Err` if configuration is incomplete or key loading fails.
    pub fn new(config: &StirShakenConfig) -> Result<Option<Self>, StirError> {
        let (key_path, cert_url) = match (&config.signing_key_path, &config.signing_cert_url) {
            (Some(key_path), Some(cert_url)) => (key_path.clone(), cert_url.clone()),
            (None, None) => return Ok(None),
            _ => {
                return Err(StirError::SigningError(
                    "incomplete signing config: need both signing_key_path and signing_cert_url"
                        .into(),
                ));
            }
        };

        let key_pem = std::fs::read_to_string(&key_path)
            .map_err(|e| StirError::SigningError(format!("reading {key_path}: {e}")))?;

        let signing_key = Self::parse_signing_key(&key_pem)?;

        Ok(Some(Self {
            signing_key,
            cert_url,
        }))
    }

    /// Parse a PEM-encoded ECDSA P-256 private key (SEC1 or PKCS8).
    fn parse_signing_key(pem_str: &str) -> Result<SigningKey, StirError> {
        // Try SEC1 format first (-----BEGIN EC PRIVATE KEY-----)
        if pem_str.contains("EC PRIVATE KEY") {
            use p256::pkcs8::DecodePrivateKey;
            // Try PKCS8 decode first for EC keys (some PEM files use PKCS8 format even
            // with EC PRIVATE KEY label)
            if let Ok(key) = SigningKey::from_pkcs8_pem(pem_str) {
                return Ok(key);
            }
            // Fall through to SEC1
            use p256::SecretKey;
            let secret = SecretKey::from_sec1_pem(pem_str)
                .map_err(|e| StirError::SigningError(format!("SEC1 PEM parse: {e}")))?;
            return Ok(SigningKey::from(&secret));
        }

        // Try PKCS8 format (-----BEGIN PRIVATE KEY-----)
        if pem_str.contains("PRIVATE KEY") {
            use p256::pkcs8::DecodePrivateKey;
            let key = SigningKey::from_pkcs8_pem(pem_str)
                .map_err(|e| StirError::SigningError(format!("PKCS8 PEM parse: {e}")))?;
            return Ok(key);
        }

        Err(StirError::SigningError(
            "unrecognized PEM key format".into(),
        ))
    }

    /// Generate an Identity header value for the given orig/dest TNs and attestation.
    pub fn sign(
        &self,
        orig_tn: &str,
        dest_tn: &str,
        attestation: AttestationLevel,
    ) -> Result<String, StirError> {
        stir_shaken::generate_identity_header(
            orig_tn,
            dest_tn,
            attestation,
            &self.cert_url,
            &self.signing_key,
        )
    }
}

// ── StirTransportDispatcher ─────────────────────────────────────────────────

/// Transport dispatcher wrapper that adds STIR/SHAKEN Identity headers
/// to outbound INVITE requests on signing-enabled trunks.
pub struct StirTransportDispatcher {
    inner: Arc<dyn TransportDispatcher>,
    signer: Arc<StirSigner>,
    /// Maps peer IP to trunk name.
    peer_trunk_map: HashMap<IpAddr, String>,
    /// Maps trunk name to per-trunk STIR/SHAKEN config.
    trunk_configs: HashMap<String, TrunkStirShakenConfig>,
    metrics: Arc<StirMetrics>,
}

impl StirTransportDispatcher {
    /// Create a new STIR signing transport dispatcher.
    pub fn new(
        inner: Arc<dyn TransportDispatcher>,
        signer: Arc<StirSigner>,
        trunks: &[TrunkConfig],
        metrics: Arc<StirMetrics>,
    ) -> Self {
        let peer_trunk_map = crate::hmr_dispatcher::build_peer_trunk_map(trunks);
        let trunk_configs: HashMap<String, TrunkStirShakenConfig> = trunks
            .iter()
            .map(|t| (t.name.clone(), t.stir_shaken.clone()))
            .collect();

        Self {
            inner,
            signer,
            peer_trunk_map,
            trunk_configs,
            metrics,
        }
    }

    /// Check if the trunk for this peer has signing enabled.
    fn should_sign(&self, peer_ip: IpAddr) -> Option<(&str, &TrunkStirShakenConfig)> {
        let trunk_name = self.peer_trunk_map.get(&peer_ip)?;
        let config = self.trunk_configs.get(trunk_name)?;
        if config.sign_mode == fcp_sbc_core::StirSignMode::Enabled {
            Some((trunk_name.as_str(), config))
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl TransportDispatcher for StirTransportDispatcher {
    async fn dispatch(&self, ctx: &TransportContext, payload: Bytes) -> anyhow::Result<()> {
        // Only intercept outbound INVITE requests for signing-enabled trunks
        if let Some((_trunk_name, trunk_config)) = self.should_sign(ctx.peer().ip()) {
            // Quick check: is this an INVITE request?
            if payload.starts_with(b"INVITE ") {
                if let Some(mut req) = sip_parse::parse_request(&payload) {
                    // Extract orig/dest TNs from From/To headers
                    let from_tn = req
                        .headers()
                        .get("From")
                        .or_else(|| req.headers().get("f"))
                        .map(|v| stir_shaken::clean_tn(v))
                        .unwrap_or_default();
                    let to_tn = req
                        .headers()
                        .get("To")
                        .or_else(|| req.headers().get("t"))
                        .map(|v| stir_shaken::clean_tn(v))
                        .unwrap_or_default();

                    if !from_tn.is_empty() && !to_tn.is_empty() {
                        match self
                            .signer
                            .sign(&from_tn, &to_tn, trunk_config.attestation)
                        {
                            Ok(identity_value) => {
                                if let Ok(()) =
                                    req.headers_mut().push("Identity", &identity_value)
                                {
                                    let rewritten = sip_parse::serialize_request(&req);
                                    self.metrics.record_signing_success();
                                    return self.inner.dispatch(ctx, rewritten).await;
                                }
                                // If push fails (too many headers), fall through
                            }
                            Err(e) => {
                                warn!(error = %e, "STIR/SHAKEN signing failed");
                                self.metrics.record_signing_failure();
                                // Fall through — send without Identity header
                            }
                        }
                    }
                }
            }
        }

        // Pass through unchanged
        self.inner.dispatch(ctx, payload).await
    }
}

// ── StirMetrics ─────────────────────────────────────────────────────────────

/// Atomic counters for STIR/SHAKEN operations.
pub struct StirMetrics {
    verifications_success: AtomicU64,
    verifications_failed: AtomicU64,
    verifications_no_identity: AtomicU64,
    signings_success: AtomicU64,
    signings_failed: AtomicU64,
}

/// Snapshot of STIR/SHAKEN metrics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StirMetricsSnapshot {
    pub verifications_success: u64,
    pub verifications_failed: u64,
    pub verifications_no_identity: u64,
    pub signings_success: u64,
    pub signings_failed: u64,
}

impl StirMetrics {
    pub fn new() -> Self {
        Self {
            verifications_success: AtomicU64::new(0),
            verifications_failed: AtomicU64::new(0),
            verifications_no_identity: AtomicU64::new(0),
            signings_success: AtomicU64::new(0),
            signings_failed: AtomicU64::new(0),
        }
    }

    pub fn record_verification_success(&self) {
        self.verifications_success.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_verification_failure(&self) {
        self.verifications_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_verification_no_identity(&self) {
        self.verifications_no_identity
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_signing_success(&self) {
        self.signings_success.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_signing_failure(&self) {
        self.signings_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> StirMetricsSnapshot {
        StirMetricsSnapshot {
            verifications_success: self.verifications_success.load(Ordering::Relaxed),
            verifications_failed: self.verifications_failed.load(Ordering::Relaxed),
            verifications_no_identity: self.verifications_no_identity.load(Ordering::Relaxed),
            signings_success: self.signings_success.load(Ordering::Relaxed),
            signings_failed: self.signings_failed.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stir_metrics_counters() {
        let metrics = StirMetrics::new();
        let snap = metrics.snapshot();
        assert_eq!(snap.verifications_success, 0);
        assert_eq!(snap.verifications_failed, 0);
        assert_eq!(snap.verifications_no_identity, 0);
        assert_eq!(snap.signings_success, 0);
        assert_eq!(snap.signings_failed, 0);

        metrics.record_verification_success();
        metrics.record_verification_success();
        metrics.record_verification_failure();
        metrics.record_verification_no_identity();
        metrics.record_signing_success();
        metrics.record_signing_success();
        metrics.record_signing_success();
        metrics.record_signing_failure();

        let snap = metrics.snapshot();
        assert_eq!(snap.verifications_success, 2);
        assert_eq!(snap.verifications_failed, 1);
        assert_eq!(snap.verifications_no_identity, 1);
        assert_eq!(snap.signings_success, 3);
        assert_eq!(snap.signings_failed, 1);
    }

    #[test]
    fn test_verification_result_variants() {
        let result = VerificationResult::Verified {
            attestation: AttestationLevel::A,
        };
        assert!(matches!(result, VerificationResult::Verified { .. }));

        let result = VerificationResult::NoIdentity;
        assert!(matches!(result, VerificationResult::NoIdentity));

        let result = VerificationResult::Failed(StirError::SignatureInvalid);
        assert!(matches!(result, VerificationResult::Failed(_)));
    }
}
