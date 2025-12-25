//! Registrar service for handling SIP REGISTER requests.
//!
//! This module provides integration between FCP storage and siphon-rs registrar.

use anyhow::Result;
use async_trait::async_trait;
use fcp_core::{CredentialStore, Registration, RegistrationStore};
use sip_auth::DigestAuthenticator;
use sip_core::{Request, Response};
use sip_registrar::{AsyncLocationStore, BasicRegistrar, Binding};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, info};

/// Adapter that implements siphon-rs LocationStore using FCP RegistrationStore.
pub struct LocationStoreAdapter {
    store: Arc<dyn RegistrationStore>,
}

impl LocationStoreAdapter {
    pub fn new(store: Arc<dyn RegistrationStore>) -> Self {
        Self { store }
    }

    /// Convert FCP Registration to siphon-rs Binding.
    fn to_binding(reg: &Registration) -> Binding {
        let expires_duration = reg
            .expires_at
            .duration_since(SystemTime::now())
            .unwrap_or_default();

        Binding {
            aor: reg.aor.clone().into(),
            contact: reg.contact.clone().into(),
            expires: expires_duration,
            call_id: reg.call_id.clone().into(),
            cseq: reg.cseq,
            q_value: reg.q.unwrap_or(1.0),
        }
    }

    /// Convert siphon-rs Binding to FCP Registration.
    fn from_binding(binding: &Binding) -> Registration {
        Registration {
            aor: binding.aor.to_string(),
            contact: binding.contact.to_string(),
            expires_at: SystemTime::now() + binding.expires,
            call_id: binding.call_id.to_string(),
            cseq: binding.cseq,
            user_agent: None,
            q: Some(binding.q_value),
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
        Ok(registrations
            .iter()
            .filter(|r| r.expires_at > now)
            .map(Self::to_binding)
            .collect())
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
                return Some(sip_auth::Credentials {
                    username: username.into(),
                    password: user.password.into(),
                    realm: self.realm.clone().into(),
                });
            }
        }
        None
    }
}

/// FCP Registrar service that handles REGISTER requests.
///
/// This wraps siphon-rs BasicRegistrar with FCP storage adapters.
pub struct FcpRegistrar {
    /// Siphon-rs registrar with async location store
    registrar: BasicRegistrar<
        LocationStoreAdapter,
        DigestAuthenticator<sip_auth::AsyncToSyncCredentialAdapter<CredentialStoreAdapter>>,
    >,

    /// Enable authentication
    auth_enabled: bool,

    /// Credential store for looking up user preferences
    credential_store: Option<Arc<dyn CredentialStore>>,
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

        let (registrar, auth_enabled) = if let Some(ref cred_store) = credential_store {
            // Create authenticator with async credential store adapter
            let cred_adapter = CredentialStoreAdapter::new(Arc::clone(cred_store), realm_str.clone());
            let sync_adapter = sip_auth::AsyncToSyncCredentialAdapter::new(cred_adapter);
            let authenticator = DigestAuthenticator::new(&realm_str, sync_adapter);

            let registrar = BasicRegistrar::new(location_store, Some(authenticator));

            (registrar, true)
        } else {
            // No authentication
            let registrar = BasicRegistrar::new(location_store, None);

            (registrar, false)
        };

        Self {
            registrar,
            auth_enabled,
            credential_store,
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
        if self.auth_enabled {
            debug!("Handling REGISTER with authentication");
        } else {
            debug!("Handling REGISTER without authentication");
        }

        // Use siphon-rs registrar to handle the request
        match self.registrar.handle_register_async(request).await {
            Ok(response) => {
                if response.start.code == 200 {
                    info!(
                        aor = %sip_parse::header(&request.headers, "To").map(|s| s.as_str()).unwrap_or(""),
                        "Registration successful"
                    );
                } else if response.start.code == 401 {
                    debug!("Sent 401 Unauthorized challenge");
                }
                response
            }
            Err(e) => {
                tracing::error!("Failed to handle REGISTER: {}", e);
                // Return 500 Internal Server Error
                let mut headers = sip_core::Headers::new();
                if let Some(via) = request.headers.get("Via") {
                    headers.push("Via".into(), via.clone());
                }
                if let Some(from) = request.headers.get("From") {
                    headers.push("From".into(), from.clone());
                }
                if let Some(to) = request.headers.get("To") {
                    headers.push("To".into(), to.clone());
                }
                if let Some(call_id) = request.headers.get("Call-ID") {
                    headers.push("Call-ID".into(), call_id.clone());
                }
                if let Some(cseq) = request.headers.get("CSeq") {
                    headers.push("CSeq".into(), cseq.clone());
                }
                headers.push("Content-Length".into(), "0".into());

                Response::new(
                    sip_core::StatusLine::new(500, "Internal Server Error".into()),
                    headers,
                    bytes::Bytes::new(),
                )
            }
        }
    }

    /// Get all registrations for an Address-of-Record.
    pub async fn get_registrations(&self, aor: &str) -> Result<Vec<Binding>> {
        // Normalize AOR so lookups match stored keys even if clients send
        // params or percent-encoded users.
        let normalized = match sip_core::Uri::parse(aor) {
            Some(uri) => sip_registrar::normalize_aor(&uri)
                .unwrap_or_else(|_| aor.split(';').next().unwrap_or(aor).to_string()),
            None => aor.split(';').next().unwrap_or(aor).to_string(),
        };

        self.registrar.location_store().lookup(&normalized).await
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

    #[tokio::test]
    async fn test_registrar_without_auth() {
        let temp_dir = std::env::temp_dir().join("fcp-test-registrar");
        let storage = Arc::new(FileBackedStorage::new(temp_dir.clone()).await.unwrap());

        let registrar = FcpRegistrar::new(storage.clone(), None, None);

        // Create a simple REGISTER request
        let mut headers = sip_core::Headers::new();
        headers.push("Via".into(), "SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK123".into());
        headers.push("From".into(), "<sip:alice@example.com>;tag=abc123".into());
        headers.push("To".into(), "<sip:alice@example.com>".into());
        headers.push("Call-ID".into(), "test-call-id@example.com".into());
        headers.push("CSeq".into(), "1 REGISTER".into());
        headers.push("Contact".into(), "<sip:alice@192.168.1.1:5060>;expires=3600".into());
        headers.push("Expires".into(), "3600".into());
        headers.push("Content-Length".into(), "0".into());

        let request = Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Register,
                sip_core::SipUri::parse("sip:example.com").unwrap(),
            ),
            headers,
            bytes::Bytes::new(),
        );

        let response = registrar.handle_register(&request).await;
        assert_eq!(response.start.code, 200);

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
        headers.push("Via".into(), "SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK123".into());
        headers.push("From".into(), "<sip:alice@example.com>;tag=abc123".into());
        headers.push("To".into(), "<sip:alice@example.com>".into());
        headers.push("Call-ID".into(), "test-call-auth@example.com".into());
        headers.push("CSeq".into(), "1 REGISTER".into());
        headers.push("Contact".into(), "<sip:alice@192.168.1.1:5060>;expires=3600".into());
        headers.push("Expires".into(), "3600".into());
        headers.push("Content-Length".into(), "0".into());

        let request = Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Register,
                sip_core::SipUri::parse("sip:example.com").unwrap(),
            ),
            headers,
            bytes::Bytes::new(),
        );

        let response = registrar.handle_register(&request).await;

        // Should get 401 Unauthorized
        assert_eq!(response.start.code, 401);
        assert!(response.headers.get("WWW-Authenticate").is_some());

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
