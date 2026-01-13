//! Registrar service for handling SIP REGISTER requests.
//!
//! This module provides integration between FCP storage and siphon-rs registrar.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use fcp_core::{CredentialStore, Registration, RegistrationStore};
use sip_core::{Request, Response};
use sip_registrar::{AsyncLocationStore, Binding};
use smol_str::SmolStr;
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, info};

use crate::auth::Ha1Authenticator;

/// Adapter that implements siphon-rs LocationStore using FCP RegistrationStore.
pub struct LocationStoreAdapter {
    store: Arc<dyn RegistrationStore>,
}

impl LocationStoreAdapter {
    pub fn new(store: Arc<dyn RegistrationStore>) -> Self {
        Self { store }
    }

    /// Convert FCP Registration to siphon-rs Binding.
    fn to_binding(reg: &Registration) -> anyhow::Result<Binding> {
        let expires_duration = reg
            .expires_at
            .duration_since(SystemTime::now())
            .unwrap_or_default();

        // Clamp to minimum expiry (60 seconds) to avoid validation errors on lookup.
        // The binding was already validated on insertion, and the actual expiration
        // is tracked via expires_at. This ensures conversions don't fail due to
        // time passing between storage and lookup.
        const MIN_EXPIRES_SECS: u64 = 60;
        let clamped_duration = if expires_duration.as_secs() < MIN_EXPIRES_SECS {
            std::time::Duration::from_secs(MIN_EXPIRES_SECS)
        } else {
            expires_duration
        };

        let mut binding = Binding::new(
            reg.aor.clone().into(),
            reg.contact.clone().into(),
            clamped_duration,
        )?
        .with_call_id(reg.call_id.clone().into())?
        .with_cseq(reg.cseq)?
        .with_q_value(reg.q.unwrap_or(1.0))?;

        // Add user_agent if present
        if let Some(ref ua) = reg.user_agent {
            binding = binding.with_user_agent(SmolStr::new(ua))?;
        }

        Ok(binding)
    }

    /// Convert siphon-rs Binding to FCP Registration.
    fn from_binding(binding: &Binding) -> Registration {
        Registration {
            aor: binding.aor().to_string(),
            contact: binding.contact().to_string(),
            expires_at: SystemTime::now() + binding.expires(),
            call_id: binding.call_id().to_string(),
            cseq: binding.cseq(),
            user_agent: binding.user_agent().map(|s| s.to_string()),
            q: Some(binding.q_value()),
            registered_at: SystemTime::now(),
        }
    }
}

#[async_trait]
impl AsyncLocationStore for LocationStoreAdapter {
    async fn upsert(&self, binding: Binding) -> Result<()> {
        let registration = Self::from_binding(&binding);
        self.store.put_registration(registration).await
    }

    async fn remove(&self, aor: &str, contact: &str) -> Result<()> {
        self.store.delete_registration(aor, contact).await
    }

    async fn remove_all(&self, aor: &str) -> Result<()> {
        self.store.delete_all_registrations(aor).await
    }

    async fn lookup(&self, aor: &str) -> Result<Vec<Binding>> {
        let registrations = self.store.get_registrations(aor).await?;

        // Filter out expired registrations
        let now = SystemTime::now();
        registrations
            .iter()
            .filter(|r| r.expires_at > now)
            .map(Self::to_binding)
            .collect()
    }

    async fn cleanup_expired(&self) -> Result<usize> {
        self.store.cleanup_expired().await
    }
}

/// Adapter that implements siphon-rs CredentialStore using FCP CredentialStore.
pub struct CredentialStoreAdapter {
    store: Arc<dyn CredentialStore>,
    realm: String,
}

impl CredentialStoreAdapter {
    pub fn new(store: Arc<dyn CredentialStore>, realm: String) -> Self {
        Self { store, realm }
    }
}

#[async_trait]
impl sip_auth::AsyncCredentialStore for CredentialStoreAdapter {
    async fn fetch(&self, username: &str, _realm: &str) -> Option<sip_auth::Credentials> {
        if let Ok(Some(user)) = self.store.get_user(username).await {
            if user.enabled {
                // Use pre-computed HA1 hash for SIP Digest authentication
                // We pass the HA1 hash as the "password" - it will be used directly without re-hashing
                // This is secure because we never store plaintext passwords
                if let Some(ref ha1_hash) = user.sip_ha1_hash {
                    return Some(sip_auth::Credentials::new(
                        username,
                        ha1_hash.as_str(),
                        self.realm.clone(),
                    ));
                }
            }
        }
        None
    }
}

/// FCP Registrar service that handles REGISTER requests.
///
/// This implements registration handling with custom HA1 authentication.
pub struct FcpRegistrar {
    /// Location store for managing registrations
    location_store: LocationStoreAdapter,

    /// Custom authenticator that uses pre-computed HA1 hashes
    authenticator: Option<Ha1Authenticator<CredentialStoreAdapter>>,

    /// Credential store for looking up user preferences
    credential_store: Option<Arc<dyn CredentialStore>>,

    /// Default registration expiry in seconds
    default_expires: u32,

    /// Minimum registration expiry in seconds
    min_expires: u32,

    /// Maximum registration expiry in seconds
    max_expires: u32,
}

impl FcpRegistrar {
    /// Create a new registrar.
    ///
    /// # Arguments
    /// * `registration_store` - Storage for SIP registrations
    /// * `credential_store` - Storage for user credentials (optional, for auth)
    /// * `realm` - Authentication realm (optional)
    pub fn new(
        registration_store: Arc<dyn RegistrationStore>,
        credential_store: Option<Arc<dyn CredentialStore>>,
        realm: Option<String>,
    ) -> Self {
        let location_store = LocationStoreAdapter::new(registration_store);
        let realm_str = realm.unwrap_or_else(|| "fcp.local".to_string());

        // Create our custom Ha1Authenticator if credentials are provided
        let authenticator = if let Some(ref cred_store) = credential_store {
            let cred_adapter =
                CredentialStoreAdapter::new(Arc::clone(cred_store), realm_str.clone());
            Some(Ha1Authenticator::new(realm_str, cred_adapter))
        } else {
            None
        };

        Self {
            location_store,
            authenticator,
            credential_store,
            default_expires: 3600,
            min_expires: 60,
            max_expires: 86400,
        }
    }

    /// Handle a REGISTER request.
    ///
    /// # Arguments
    /// * `request` - The REGISTER request
    ///
    /// # Returns
    /// The response to send (200 OK, 401 Unauthorized, etc.)
    pub async fn handle_register(&self, request: &Request) -> Response {
        // Handle authentication manually if enabled
        if let Some(ref authenticator) = self.authenticator {
            debug!("Handling REGISTER with authentication");

            // Check if request has Authorization header
            let has_auth = request
                .headers()
                .get_smol("Authorization")
                .or_else(|| request.headers().get_smol("Proxy-Authorization"))
                .is_some();

            if !has_auth {
                // No auth header - send challenge
                debug!("No Authorization header, sending 401 challenge");
                return match authenticator.challenge(request) {
                    Ok(response) => response,
                    Err(e) => {
                        tracing::error!("Failed to create auth challenge: {}", e);
                        return self.create_error_response(request, 500, "Internal Server Error");
                    }
                };
            }

            // Verify authentication
            match authenticator.verify(request).await {
                Ok(true) => {
                    debug!("Authentication successful");
                    // Continue to registration handling
                }
                Ok(false) => {
                    debug!("Authentication failed, sending 401 challenge");
                    return match authenticator.challenge(request) {
                        Ok(response) => response,
                        Err(e) => {
                            tracing::error!("Failed to create auth challenge: {}", e);
                            return self
                                .create_error_response(request, 500, "Internal Server Error");
                        }
                    };
                }
                Err(e) => {
                    tracing::error!("Authentication error: {}", e);
                    return self.create_error_response(request, 500, "Internal Server Error");
                }
            }
        } else {
            debug!("Handling REGISTER without authentication");
        }

        // Authentication passed or not required - handle registration
        self.process_registration(request).await
    }

    /// Process the actual registration logic
    async fn process_registration(&self, request: &Request) -> Response {
        // Extract To header for AOR
        let to_header = match request.headers().get_smol("To") {
            Some(to) => to,
            None => {
                return self.create_error_response(request, 400, "Bad Request - Missing To header");
            }
        };

        let to = match sip_parse::parse_to_header(to_header) {
            Some(t) => t,
            None => {
                return self
                    .create_error_response(request, 400, "Bad Request - Invalid To header");
            }
        };

        let aor = sip_registrar::normalize_aor(to.inner().uri())
            .unwrap_or_else(|_| to.inner().uri().to_string());

        // Extract Contact header
        let contact_header = request.headers().get_smol("Contact");

        // Handle deregistration (Contact: *)
        if let Some(contact) = contact_header {
            if contact.trim() == "*" {
                // Delete all bindings
                match self.location_store.remove_all(&aor).await {
                    Ok(_) => {
                        info!(aor = %aor, "Deregistered all contacts");
                        return self.create_ok_response(request, vec![]);
                    }
                    Err(e) => {
                        tracing::error!("Failed to deregister: {}", e);
                        return self
                            .create_error_response(request, 500, "Internal Server Error");
                    }
                }
            }
        }

        // Extract expiry
        let expires = self.extract_expires(request);

        // Handle query (Expires: 0 means deregistration)
        if expires == 0 {
            if let Some(contact_str) = contact_header {
                match self.location_store.remove(&aor, contact_str.as_str()).await {
                    Ok(_) => {
                        info!(aor = %aor, contact = %contact_str, "Deregistered contact");
                        return self.create_ok_response(request, vec![]);
                    }
                    Err(e) => {
                        tracing::error!("Failed to deregister contact: {}", e);
                        return self
                            .create_error_response(request, 500, "Internal Server Error");
                    }
                }
            }
        }

        // Handle registration (Expires > 0)
        if let Some(contact_str) = contact_header {
            let binding = match Self::create_binding(&aor, contact_str.as_str(), expires, request) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("Failed to create binding: {}", e);
                    return self.create_error_response(request, 400, "Bad Request");
                }
            };

            match self.location_store.upsert(binding).await {
                Ok(_) => {
                    info!(aor = %aor, contact = %contact_str, expires = expires, "Registration successful");
                }
                Err(e) => {
                    tracing::error!("Failed to store registration: {}", e);
                    return self.create_error_response(request, 500, "Internal Server Error");
                }
            }
        }

        // Retrieve all current bindings and return in response
        match self.location_store.lookup(&aor).await {
            Ok(bindings) => self.create_ok_response(request, bindings),
            Err(e) => {
                tracing::error!("Failed to lookup registrations: {}", e);
                self.create_error_response(request, 500, "Internal Server Error")
            }
        }
    }

    /// Extract expires value from request
    fn extract_expires(&self, request: &Request) -> u32 {
        // Check Contact header params first
        if let Some(contact) = request.headers().get("Contact") {
            if let Some(expires_str) = contact
                .split(';')
                .find(|p| p.trim().starts_with("expires="))
            {
                if let Some(val) = expires_str.trim().strip_prefix("expires=") {
                    if let Ok(exp) = val.parse::<u32>() {
                        return exp.clamp(self.min_expires, self.max_expires);
                    }
                }
            }
        }

        // Check Expires header
        if let Some(expires_str) = request.headers().get("Expires") {
            if let Ok(exp) = expires_str.parse::<u32>() {
                return exp.clamp(self.min_expires, self.max_expires);
            }
        }

        // Default
        self.default_expires
    }

    /// Create a binding from request parameters
    fn create_binding(
        aor: &str,
        contact: &str,
        expires: u32,
        request: &Request,
    ) -> Result<Binding> {
        let call_id = request
            .headers()
            .get("Call-ID")
            .ok_or_else(|| anyhow!("Missing Call-ID"))?;

        let cseq_str = request
            .headers()
            .get("CSeq")
            .ok_or_else(|| anyhow!("Missing CSeq"))?;
        let cseq = cseq_str
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or_else(|| anyhow!("Invalid CSeq"))?;

        let user_agent = request.headers().get("User-Agent");

        let mut binding = Binding::new(
            SmolStr::new(aor),
            SmolStr::new(contact),
            std::time::Duration::from_secs(expires as u64),
        )
        .map_err(|e| anyhow!("Binding creation failed: {}", e))?
        .with_call_id(SmolStr::new(call_id))
        .map_err(|e| anyhow!("Failed to set call_id: {}", e))?
        .with_cseq(cseq)
        .map_err(|e| anyhow!("Failed to set cseq: {}", e))?
        .with_q_value(1.0)
        .map_err(|e| anyhow!("Failed to set q_value: {}", e))?;

        // Add User-Agent if present
        if let Some(ua) = user_agent {
            binding = binding
                .with_user_agent(SmolStr::new(ua))
                .map_err(|e| anyhow!("Failed to set user_agent: {}", e))?;
        }

        Ok(binding)
    }

    /// Create a 200 OK response with Contact headers for all bindings
    fn create_ok_response(&self, request: &Request, bindings: Vec<Binding>) -> Response {
        let mut headers = sip_core::Headers::new();

        // Copy required headers from request
        if let Some(via) = request.headers().get_smol("Via") {
            let _ = headers.push(SmolStr::new("Via"), via.clone());
        }
        if let Some(from) = request.headers().get_smol("From") {
            let _ = headers.push(SmolStr::new("From"), from.clone());
        }
        if let Some(to) = request.headers().get_smol("To") {
            let to_with_tag = if !to.contains(";tag=") {
                SmolStr::new(format!("{};tag={}", to, self.generate_tag()))
            } else {
                to.clone()
            };
            let _ = headers.push(SmolStr::new("To"), to_with_tag);
        }
        if let Some(call_id) = request.headers().get_smol("Call-ID") {
            let _ = headers.push(SmolStr::new("Call-ID"), call_id.clone());
        }
        if let Some(cseq) = request.headers().get_smol("CSeq") {
            let _ = headers.push(SmolStr::new("CSeq"), cseq.clone());
        }

        // Add Contact headers for each binding
        for binding in bindings {
            let contact = format!(
                "<{}>;expires={}",
                binding.contact(),
                binding.expires().as_secs()
            );
            let _ = headers.push(SmolStr::new("Contact"), SmolStr::new(contact));
        }

        let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        Response::new(
            sip_core::StatusLine::new(200, "OK").expect("Failed to create status line"),
            headers,
            bytes::Bytes::new(),
        )
        .expect("Failed to create response")
    }

    fn generate_tag(&self) -> String {
        use rand::{distributions::Alphanumeric, thread_rng, Rng};
        thread_rng()
            .sample_iter(&Alphanumeric)
            .take(10)
            .map(char::from)
            .collect()
    }

    /// Create an error response with proper headers copied from request
    fn create_error_response(&self, request: &Request, code: u16, reason: &str) -> Response {
        let mut headers = sip_core::Headers::new();
        if let Some(via) = request.headers().get_smol("Via") {
            let _ = headers.push(SmolStr::new("Via"), via.clone());
        }
        if let Some(from) = request.headers().get_smol("From") {
            let _ = headers.push(SmolStr::new("From"), from.clone());
        }
        if let Some(to) = request.headers().get_smol("To") {
            let _ = headers.push(SmolStr::new("To"), to.clone());
        }
        if let Some(call_id) = request.headers().get_smol("Call-ID") {
            let _ = headers.push(SmolStr::new("Call-ID"), call_id.clone());
        }
        if let Some(cseq) = request.headers().get_smol("CSeq") {
            let _ = headers.push(SmolStr::new("CSeq"), cseq.clone());
        }
        let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        Response::new(
            sip_core::StatusLine::new(code, reason).expect("Failed to create status line"),
            headers,
            bytes::Bytes::new(),
        )
        .expect("Failed to create response")
    }

    /// Get all registrations for an Address-of-Record.
    pub async fn get_registrations(&self, aor: &str) -> Result<Vec<Binding>> {
        // Normalize AOR so lookups match stored keys even if clients send
        // params or percent-encoded users.
        let normalized = match sip_core::Uri::parse(aor) {
            Ok(uri) => sip_registrar::normalize_aor(&uri)
                .unwrap_or_else(|_| aor.split(';').next().unwrap_or(aor).to_string()),
            Err(_) => aor.split(';').next().unwrap_or(aor).to_string(),
        };

        self.location_store.lookup(&normalized).await
    }

    /// Get media preferences for a user.
    ///
    /// # Arguments
    /// * `username` - The username to look up
    ///
    /// # Returns
    /// The user's media preferences, or default preferences if user not found
    pub async fn get_user_media_preferences(&self, username: &str) -> fcp_core::MediaPreferences {
        if let Some(ref cred_store) = self.credential_store {
            match cred_store.get_user(username).await {
                Ok(Some(user)) => {
                    debug!(
                        username = %username,
                        force_anchor = %user.media_preferences.force_media_anchoring,
                        dtmf = %user.media_preferences.enable_dtmf_detection,
                        record = %user.media_preferences.record_all_calls,
                        "Retrieved user media preferences"
                    );
                    return user.media_preferences;
                }
                Ok(None) => {
                    debug!(username = %username, "User not found, using default media preferences");
                }
                Err(e) => {
                    tracing::error!(username = %username, error = %e, "Failed to look up user");
                }
            }
        }

        // Return default preferences if no credential store or user not found
        fcp_core::MediaPreferences::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::FileBackedStorage;
    use smol_str::SmolStr;

    #[tokio::test]
    async fn test_registrar_without_auth() {
        let temp_dir = std::env::temp_dir().join("fcp-test-registrar");
        let storage = Arc::new(FileBackedStorage::new(temp_dir.clone()).await.unwrap());

        let registrar = FcpRegistrar::new(storage.clone(), None, None);

        // Create a simple REGISTER request
        let mut headers = sip_core::Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK123"),
        );
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:alice@example.com>;tag=abc123"),
        );
        headers.push(SmolStr::new("To"), SmolStr::new("<sip:alice@example.com>"));
        headers.push(
            SmolStr::new("Call-ID"),
            SmolStr::new("test-call-id@example.com"),
        );
        headers.push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER"));
        headers.push(
            SmolStr::new("Contact"),
            SmolStr::new("<sip:alice@192.168.1.1:5060>;expires=3600"),
        );
        headers.push(SmolStr::new("Expires"), SmolStr::new("3600"));
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        let request = Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Register,
                sip_core::SipUri::parse("sip:example.com").unwrap(),
            ),
            headers,
            bytes::Bytes::new(),
        )
        .expect("Failed to create request");

        let response = registrar.handle_register(&request).await;
        assert_eq!(response.start_line().code(), 200);

        // Verify registration was stored
        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 1);

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir).await.ok();
    }

    #[tokio::test]
    async fn test_registrar_with_auth() {
        let temp_dir = std::env::temp_dir().join("fcp-test-registrar-auth");
        let storage = Arc::new(FileBackedStorage::new(temp_dir.clone()).await.unwrap());

        // Create a test user
        let user = fcp_core::User {
            username: "alice".to_string(),
            password: "secret123".to_string(),
            display_name: Some("Alice".to_string()),
            tenant_id: fcp_core::TenantId::nil(), // Default tenant for test
            enabled: true,
            created_at: std::time::SystemTime::now(),
            updated_at: std::time::SystemTime::now(),
            media_preferences: fcp_core::MediaPreferences::default(),
            email: None,
            password_last_reset: None,
            phone_number: None,
            mobile_number: None,
            pin: None,
            extension: None,
            department: None,
            job_title: None,
            location: None,
            language: "en-US".to_string(),
            timezone: "UTC".to_string(),
            account_status: fcp_core::AccountStatus::Active,
            last_login: None,
            emergency_contact: fcp_core::EmergencyContact::default(),
            digest_password: None,
            sip_ha1_hash: None,
        };
        storage.put_user(user).await.unwrap();

        // Create registrar WITH authentication
        let registrar = FcpRegistrar::new(
            storage.clone(),
            Some(storage.clone()), // Use storage as credential store
            Some("test.local".to_string()),
        );

        // First request without auth - should get 401
        let mut headers = sip_core::Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK123"),
        );
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:alice@example.com>;tag=abc123"),
        );
        headers.push(SmolStr::new("To"), SmolStr::new("<sip:alice@example.com>"));
        headers.push(
            SmolStr::new("Call-ID"),
            SmolStr::new("test-call-auth@example.com"),
        );
        headers.push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER"));
        headers.push(
            SmolStr::new("Contact"),
            SmolStr::new("<sip:alice@192.168.1.1:5060>;expires=3600"),
        );
        headers.push(SmolStr::new("Expires"), SmolStr::new("3600"));
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        let request = Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Register,
                sip_core::SipUri::parse("sip:example.com").unwrap(),
            ),
            headers,
            bytes::Bytes::new(),
        )
        .expect("Failed to create request");

        let response = registrar.handle_register(&request).await;

        // Should get 401 Unauthorized
        assert_eq!(response.start_line().code(), 401);
        assert!(response.headers().get("WWW-Authenticate").is_some());

        // Verify no registration was stored (auth required)
        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 0);

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir).await.ok();
    }
}
