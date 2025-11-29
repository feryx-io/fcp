//! Registrar service for handling SIP REGISTER requests.
//!
//! This module provides integration between FCP storage and siphon-rs registrar.

use anyhow::Result;
use async_trait::async_trait;
use fcp_core::{CredentialStore, Registration, RegistrationStore, User};
use sip_auth::{CredentialStore as SipCredentialStore, DigestAuthenticator};
use sip_core::{Request, Response};
use sip_registrar::{BasicRegistrar, Binding, LocationStore};
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
        let expires_in = reg
            .expires_at
            .duration_since(SystemTime::now())
            .unwrap_or_default()
            .as_secs() as u32;

        Binding {
            aor: reg.aor.clone().into(),
            contact: reg.contact.clone().into(),
            expires: expires_in,
            call_id: reg.call_id.clone().into(),
            cseq: reg.cseq,
            q: reg.q,
        }
    }

    /// Convert siphon-rs Binding to FCP Registration.
    fn from_binding(binding: &Binding) -> Registration {
        Registration {
            aor: binding.aor.to_string(),
            contact: binding.contact.to_string(),
            expires_at: SystemTime::now() + std::time::Duration::from_secs(binding.expires as u64),
            call_id: binding.call_id.to_string(),
            cseq: binding.cseq,
            user_agent: None,
            q: binding.q,
            registered_at: SystemTime::now(),
        }
    }
}

#[async_trait]
impl LocationStore for LocationStoreAdapter {
    async fn upsert(&self, binding: Binding) -> Result<()> {
        let registration = Self::from_binding(&binding);
        self.store.put_registration(registration).await
    }

    async fn remove(&self, aor: &str, contact: Option<&str>) -> Result<Vec<Binding>> {
        // Get existing registrations before removal
        let existing = self.store.get_registrations(aor).await?;

        if let Some(contact_uri) = contact {
            // Remove specific contact
            self.store.delete_registration(aor, contact_uri).await?;
            Ok(existing
                .iter()
                .filter(|r| r.contact == contact_uri)
                .map(Self::to_binding)
                .collect())
        } else {
            // Remove all (wildcard)
            self.store.delete_all_registrations(aor).await?;
            Ok(existing.iter().map(Self::to_binding).collect())
        }
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
}

/// Adapter that implements siphon-rs CredentialStore using FCP CredentialStore.
pub struct CredentialStoreAdapter {
    store: Arc<dyn CredentialStore>,
}

impl CredentialStoreAdapter {
    pub fn new(store: Arc<dyn CredentialStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl SipCredentialStore for CredentialStoreAdapter {
    async fn get_password(&self, username: &str) -> Option<String> {
        if let Ok(Some(user)) = self.store.get_user(username).await {
            if user.enabled {
                return Some(user.password);
            }
        }
        None
    }
}

/// FCP Registrar service that handles REGISTER requests.
///
/// This wraps siphon-rs BasicRegistrar with FCP storage adapters.
pub struct FcpRegistrar {
    /// Siphon-rs registrar
    registrar: BasicRegistrar<LocationStoreAdapter, DigestAuthenticator<CredentialStoreAdapter>>,

    /// Enable authentication
    auth_enabled: bool,
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

        let (registrar, auth_enabled) = if let Some(cred_store) = credential_store {
            // Create authenticator
            let cred_adapter = CredentialStoreAdapter::new(cred_store);
            let authenticator = DigestAuthenticator::new(
                realm.unwrap_or_else(|| "fcp.local".to_string()),
                cred_adapter,
            );

            let registrar = BasicRegistrar::builder()
                .location_store(location_store)
                .authenticator(Some(authenticator))
                .build();

            (registrar, true)
        } else {
            // No authentication
            let registrar = BasicRegistrar::builder()
                .location_store(location_store)
                .build();

            (registrar, false)
        };

        Self {
            registrar,
            auth_enabled,
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
        match self.registrar.handle(request).await {
            Ok(response) => {
                if response.start.code == 200 {
                    info!(
                        aor = %sip_parse::header(&request.headers, "To").unwrap_or_default(),
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
        self.registrar.location_store().lookup(aor).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::FileBackedStorage;
    use std::time::Duration;

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
}
