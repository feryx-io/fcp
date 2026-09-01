//! Registrar service for handling SIP REGISTER requests.
//!
//! This module provides integration between FCP storage and siphon-rs registrar.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use fcp_core::{
    is_host_literal, CredentialStore, Registration, RegistrationStore, TenantId, TenantStore,
};
use fcp_events::{EventPublisher, PlatformEvent, RegistrationEvent};
use sip_core::{Request, Response};
use sip_registrar::{AsyncLocationStore, Binding};
use smol_str::SmolStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, info, warn};

use crate::auth::{Ha1Authenticator, Ha1CredentialStore};

fn extract_aor_from_request(request: &Request) -> Option<String> {
    let to_header = request.headers().get_smol("To")?;
    let to = sip_parse::parse_to_header(to_header)?;
    Some(
        sip_registrar::normalize_aor(to.inner().uri())
            .unwrap_or_else(|_| to.inner().uri().to_string()),
    )
}

fn extract_username_from_aor(aor: &str) -> Option<String> {
    let without_scheme = aor
        .strip_prefix("sip:")
        .or_else(|| aor.strip_prefix("sips:"))
        .unwrap_or(aor);
    let username = without_scheme.split('@').next()?.trim();
    if username.is_empty() {
        None
    } else {
        Some(username.to_string())
    }
}

fn extract_domain_from_aor(aor: &str) -> Option<String> {
    let (_, domain) = FcpRegistrar::split_aor(aor)?;
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_string())
    }
}

/// Resolve an inbound SIP request to a tenant using a tiered strategy.
///
/// The resolver walks through progressively weaker signals and uses the
/// first one that confidently identifies a tenant:
///
/// 1. **Hostname domain → `sip_domains` lookup.** If the AOR host is a
///    real hostname, look it up against the tenant allowlist. A match
///    returns immediately; a miss fails closed so typos / spoofed
///    hostnames can't silently route to the default tenant.
///
/// 2. **Legacy name-based fallback.** Older deployments used the tenant
///    display name as the SIP domain. To avoid breaking those while they
///    migrate to `sip_domains`, we still try `get_tenant_by_name` on a
///    miss. (This fallback can be removed once all live tenants have
///    at least one entry in `sip_domains`.)
///
/// 3. **IP / localhost → skip domain lookup.** Numeric IP literals are
///    never tenant names; fall through to the next tier.
///
/// 4. **Username-based lookup.** If the credential store sees exactly one
///    user with this username across every tenant, route the request to
///    that user's tenant. Ambiguous usernames (two tenants with the same
///    name) fail loudly so the operator must disambiguate.
///
/// 5. **Default tenant.** Last-resort compatibility for single-tenant
///    deployments. Only used when nothing above provided a match.
async fn resolve_tenant_for_inputs(
    tenant_store: Option<&Arc<dyn TenantStore>>,
    credential_store: Option<&Arc<dyn CredentialStore>>,
    default_tenant_id: Option<TenantId>,
    aor: Option<&str>,
    username_hint: Option<&str>,
) -> Result<TenantId> {
    if let Some(domain) = aor.and_then(extract_domain_from_aor) {
        if is_host_literal(&domain) {
            // IP literal or localhost — skip the tenant-store lookup and
            // let the username / default fallbacks handle it.
            debug!(
                domain = %domain,
                "Host literal in AOR; skipping tenant domain lookup"
            );
        } else if let Some(store) = tenant_store {
            // Tier 1: sip_domains allowlist.
            if let Some(tenant) = store.get_tenant_by_sip_domain(&domain).await? {
                return Ok(tenant.id);
            }

            // Tier 2: legacy fallback — tenant display name used as SIP
            // domain. Deployments should migrate off this by populating
            // sip_domains on every tenant.
            if let Some(tenant) = store.get_tenant_by_name(&domain).await? {
                warn!(
                    domain = %domain,
                    tenant_id = %tenant.id,
                    "Tenant resolved via legacy name match; add '{}' to the tenant's sip_domains to silence this warning",
                    domain
                );
                return Ok(tenant.id);
            }

            // Hostname present but nothing claims it — fail closed.
            return Err(anyhow!("unknown tenant domain '{}'", domain));
        } else if let Some(default_tenant_id) = default_tenant_id {
            // No tenant store configured at all — single-tenant deployment.
            return Ok(default_tenant_id);
        } else {
            return Err(anyhow!(
                "unknown tenant domain '{}' and no default tenant configured",
                domain
            ));
        }
    }

    // Tier 3 (or follow-through from an IP literal): username lookup.
    let username = username_hint
        .map(str::to_string)
        .or_else(|| aor.and_then(extract_username_from_aor));

    if let (Some(store), Some(username)) = (credential_store, username) {
        let users = store.list_users_by_username(&username).await?;
        match users.len() {
            1 => return Ok(users[0].tenant_id),
            0 => {}
            _ => {
                return Err(anyhow!(
                    "ambiguous username '{}' across multiple tenants",
                    username
                ))
            }
        }
    }

    // Tier 4: default tenant.  IP-literal domains, unknown usernames,
    // and internal service-to-service requests all land here.  The
    // default tenant is safe to use because:
    //   - Real hostnames already failed closed in the domain tiers above.
    //   - Trunk admission control (for INVITEs) still enforces per-IP
    //     access for external sources.
    if let Some(default_tenant_id) = default_tenant_id {
        Ok(default_tenant_id)
    } else {
        Err(anyhow!("tenant context required"))
    }
}

/// Adapter that implements siphon-rs LocationStore using FCP RegistrationStore.
pub struct LocationStoreAdapter {
    store: Arc<dyn RegistrationStore>,
}

impl LocationStoreAdapter {
    pub fn new(store: Arc<dyn RegistrationStore>) -> Self {
        Self { store }
    }

    async fn upsert_in_tenant(&self, binding: Binding, tenant_id: TenantId) -> Result<()> {
        let mut registration = Self::from_binding(&binding);
        registration.tenant_id = tenant_id;
        registration.contact =
            crate::registrar::FcpRegistrar::normalize_contact_for_matching(&registration.contact);
        self.store.put_registration(registration).await
    }

    async fn remove_in_tenant(&self, tenant_id: &TenantId, aor: &str, contact: &str) -> Result<()> {
        let registrations = self
            .store
            .get_registrations_in_tenant(tenant_id, aor)
            .await?;

        let normalized_requested =
            crate::registrar::FcpRegistrar::normalize_contact_for_matching(contact);

        let mut removed_any = false;
        for reg in registrations {
            let normalized_stored =
                crate::registrar::FcpRegistrar::normalize_contact_for_matching(&reg.contact);
            if normalized_stored == normalized_requested {
                self.store
                    .delete_registration_in_tenant(tenant_id, aor, &reg.contact)
                    .await?;
                removed_any = true;
            }
        }

        if !removed_any {
            info!(aor = %aor, contact = %contact, normalized = %normalized_requested, "No matching registration to remove");
        }

        Ok(())
    }

    async fn remove_all_in_tenant(&self, tenant_id: &TenantId, aor: &str) -> Result<()> {
        self.store
            .delete_all_registrations_in_tenant(tenant_id, aor)
            .await
    }

    async fn lookup_in_tenant(&self, tenant_id: &TenantId, aor: &str) -> Result<Vec<Binding>> {
        let registrations = self
            .store
            .get_registrations_in_tenant(tenant_id, aor)
            .await?;
        let now = SystemTime::now();
        registrations
            .iter()
            .filter(|r| r.expires_at > now)
            .map(Self::to_binding)
            .collect()
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

        // Add Path header if present (RFC 3327)
        if let Some(ref path_str) = reg.path {
            // Parse the serialized Path header back into PathHeader
            // Path headers are stored as Display format: "<uri1>, <uri2>"
            if let Ok(path) = Self::parse_path_from_string(path_str) {
                binding = binding.with_path(path);
            }
        }

        // Add flow token if present (RFC 5626)
        if let Some(ref flow_token) = reg.flow_token {
            binding = binding.with_flow_token(SmolStr::new(flow_token));
        }

        Ok(binding)
    }

    /// Parse a Path header from its serialized string representation.
    fn parse_path_from_string(path_str: &str) -> anyhow::Result<sip_core::PathHeader> {
        // Create a temporary Headers object to use sip_parse::parse_path
        let mut headers = sip_core::Headers::new();
        let _ = headers.push(SmolStr::new("Path"), SmolStr::new(path_str));
        sip_parse::parse_path(&headers)
            .map_err(|e| anyhow::anyhow!("Failed to parse Path: {:?}", e))
    }

    /// Convert siphon-rs Binding to FCP Registration.
    fn from_binding(binding: &Binding) -> Registration {
        Registration {
            tenant_id: TenantId::nil(),
            aor: binding.aor().to_string(),
            contact: binding.contact().to_string(),
            expires_at: SystemTime::now() + binding.expires(),
            call_id: binding.call_id().to_string(),
            cseq: binding.cseq(),
            user_agent: binding.user_agent().map(|s| s.to_string()),
            q: Some(binding.q_value()),
            registered_at: SystemTime::now(),
            // Serialize Path header to string for storage (RFC 3327)
            path: binding.path().map(|p| p.to_string()),
            // Store flow token for RFC 5626 connection reuse
            flow_token: binding.flow_token().map(|s| s.to_string()),
        }
    }
}

#[async_trait]
impl AsyncLocationStore for LocationStoreAdapter {
    async fn upsert(&self, binding: Binding) -> Result<()> {
        let mut registration = Self::from_binding(&binding);
        // Normalize the contact to strip transient parameters (rinstance, gr).
        // This ensures re-registrations from the same device update the existing
        // row via ON CONFLICT(aor, contact) rather than creating duplicates
        // when rinstance changes between client restarts.
        registration.contact =
            crate::registrar::FcpRegistrar::normalize_contact_for_matching(&registration.contact);
        self.store.put_registration(registration).await
    }

    async fn remove(&self, aor: &str, contact: &str) -> Result<()> {
        // For removal, we need to normalize contacts for matching since rinstance changes
        // Get all registrations for this AOR
        let registrations = self.store.get_registrations(aor).await?;

        debug!(
            aor = %aor,
            contact = %contact,
            stored_count = registrations.len(),
            "Attempting to remove registration"
        );

        // Normalize the requested contact
        let normalized_requested =
            crate::registrar::FcpRegistrar::normalize_contact_for_matching(contact);
        debug!(
            aor = %aor,
            original_contact = %contact,
            normalized_contact = %normalized_requested,
            "Normalized requested contact for removal"
        );

        // Find and remove registrations that match when normalized
        let mut removed_any = false;
        for reg in registrations {
            let normalized_stored =
                crate::registrar::FcpRegistrar::normalize_contact_for_matching(&reg.contact);
            debug!(
                aor = %aor,
                stored_contact = %reg.contact,
                normalized_stored = %normalized_stored,
                normalized_requested = %normalized_requested,
                matches = (normalized_stored == normalized_requested),
                "Comparing stored registration"
            );

            if normalized_stored == normalized_requested {
                info!(
                    aor = %aor,
                    contact = %reg.contact,
                    "Removing matching registration"
                );
                self.store.delete_registration(aor, &reg.contact).await?;
                removed_any = true;
            }
        }

        if !removed_any {
            // No matching registration found - this is not an error per RFC 3261
            info!(aor = %aor, contact = %contact, normalized = %normalized_requested, "No matching registration to remove");
        } else {
            info!(aor = %aor, "Successfully removed matching registrations");
        }

        Ok(())
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
    tenant_store: Option<Arc<dyn TenantStore>>,
    default_tenant_id: Option<TenantId>,
}

impl CredentialStoreAdapter {
    pub fn new(store: Arc<dyn CredentialStore>, realm: String) -> Self {
        Self {
            store,
            realm,
            tenant_store: None,
            default_tenant_id: None,
        }
    }

    pub fn with_tenant_context(
        mut self,
        tenant_store: Option<Arc<dyn TenantStore>>,
        default_tenant_id: Option<TenantId>,
    ) -> Self {
        self.tenant_store = tenant_store;
        self.default_tenant_id = default_tenant_id;
        self
    }

    async fn resolve_tenant(&self, request: &Request, username: &str) -> Result<TenantId> {
        let aor = extract_aor_from_request(request);
        resolve_tenant_for_inputs(
            self.tenant_store.as_ref(),
            Some(&self.store),
            self.default_tenant_id,
            aor.as_deref(),
            Some(username),
        )
        .await
    }
}

#[async_trait]
impl Ha1CredentialStore for CredentialStoreAdapter {
    async fn fetch(
        &self,
        username: &str,
        _realm: &str,
        request: &Request,
    ) -> Option<sip_auth::Credentials> {
        let tenant_id = match self.resolve_tenant(request, username).await {
            Ok(tenant_id) => tenant_id,
            Err(e) => {
                debug!(username = %username, error = %e, "Failed to resolve tenant for SIP digest auth");
                return None;
            }
        };

        if let Ok(Some(user)) = self.store.get_user_in_tenant(username, &tenant_id).await {
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

    /// Custom authenticator that uses pre-computed HA1 hashes.
    /// Always created if credential store is available; auth_enabled controls whether it's used.
    authenticator: Option<Ha1Authenticator<CredentialStoreAdapter>>,

    /// Whether authentication is currently enabled (toggleable at runtime).
    auth_enabled: AtomicBool,

    /// Credential store for looking up user preferences
    credential_store: Option<Arc<dyn CredentialStore>>,

    /// Default registration expiry in seconds
    default_expires: u32,

    /// Minimum registration expiry in seconds
    min_expires: u32,

    /// Maximum registration expiry in seconds
    max_expires: u32,

    /// Event publisher for emitting registration events
    event_publisher: Option<Arc<dyn EventPublisher>>,

    /// Realm for extracting tenant context (used in events)
    realm: String,

    /// Tenant store used to resolve REGISTER requests to tenant scope.
    tenant_store: Option<Arc<dyn TenantStore>>,

    /// Explicit default tenant for single-tenant deployments without tenant domains.
    default_tenant_id: Option<TenantId>,

    /// Domain aliases - list of domains that should be treated as equivalent.
    /// When looking up registrations, all aliased domains are checked.
    domain_aliases: Vec<String>,
}

impl FcpRegistrar {
    /// Create a new registrar.
    ///
    /// # Arguments
    /// * `registration_store` - Storage for SIP registrations
    /// * `credential_store` - Storage for user credentials (always pass if available)
    /// * `realm` - Authentication realm (optional)
    /// * `auth_enabled` - Whether authentication is initially enabled
    pub fn new(
        registration_store: Arc<dyn RegistrationStore>,
        credential_store: Option<Arc<dyn CredentialStore>>,
        realm: Option<String>,
        auth_enabled: bool,
    ) -> Self {
        let location_store = LocationStoreAdapter::new(registration_store);
        let realm_str = realm.unwrap_or_else(|| "fcp.local".to_string());

        let mut registrar = Self {
            location_store,
            authenticator: None,
            auth_enabled: AtomicBool::new(auth_enabled),
            credential_store,
            default_expires: 3600,
            min_expires: 60,
            max_expires: 86400,
            event_publisher: None,
            realm: realm_str,
            tenant_store: None,
            default_tenant_id: None,
            domain_aliases: Vec::new(),
        };
        registrar.refresh_authenticator();
        registrar
    }

    /// Set domain aliases for registration lookups.
    ///
    /// All domains in the list are treated as equivalent. When looking up
    /// registrations for alice@domain1, we also check alice@domain2, etc.
    pub fn with_domain_aliases(mut self, aliases: Vec<String>) -> Self {
        self.domain_aliases = aliases;
        self
    }

    /// Attach a tenant store for tenant-aware REGISTER auth and persistence.
    pub fn with_tenant_store(mut self, tenant_store: Arc<dyn TenantStore>) -> Self {
        self.tenant_store = Some(tenant_store);
        self.refresh_authenticator();
        self
    }

    /// Set the explicit default tenant used when SIP addressing carries no tenant domain.
    pub fn with_default_tenant_id(mut self, tenant_id: TenantId) -> Self {
        self.default_tenant_id = Some(tenant_id);
        self.refresh_authenticator();
        self
    }

    /// Create a new registrar with event publishing support.
    ///
    /// # Arguments
    /// * `registration_store` - Storage for SIP registrations
    /// * `credential_store` - Storage for user credentials (always pass if available)
    /// * `realm` - Authentication realm (optional)
    /// * `event_publisher` - Event publisher for emitting registration events
    /// * `auth_enabled` - Whether authentication is initially enabled
    pub fn with_events(
        registration_store: Arc<dyn RegistrationStore>,
        credential_store: Option<Arc<dyn CredentialStore>>,
        realm: Option<String>,
        event_publisher: Arc<dyn EventPublisher>,
        auth_enabled: bool,
    ) -> Self {
        let mut registrar = Self::new(registration_store, credential_store, realm, auth_enabled);
        registrar.event_publisher = Some(event_publisher);
        registrar
    }

    fn refresh_authenticator(&mut self) {
        self.authenticator = self.credential_store.as_ref().map(|cred_store| {
            let cred_adapter =
                CredentialStoreAdapter::new(Arc::clone(cred_store), self.realm.clone())
                    .with_tenant_context(self.tenant_store.clone(), self.default_tenant_id);
            Ha1Authenticator::new(self.realm.clone(), cred_adapter)
        });
    }

    /// Enable authentication at runtime.
    pub fn enable_auth(&self) {
        self.auth_enabled.store(true, Ordering::Relaxed);
        info!("Registration authentication enabled");
    }

    /// Disable authentication at runtime.
    pub fn disable_auth(&self) {
        self.auth_enabled.store(false, Ordering::Relaxed);
        info!("Registration authentication disabled");
    }

    /// Check whether authentication is currently enabled.
    pub fn is_auth_enabled(&self) -> bool {
        self.auth_enabled.load(Ordering::Relaxed)
    }

    /// Publish a registration event (fire-and-forget).
    fn publish_event(&self, event: RegistrationEvent) {
        if let Some(ref publisher) = self.event_publisher {
            let publisher = Arc::clone(publisher);
            let platform_event = PlatformEvent::Registration(event);
            tokio::spawn(async move {
                if let Err(e) = publisher.publish(platform_event).await {
                    tracing::debug!(%e, "Failed to publish registration event");
                }
            });
        }
    }

    /// Get the realm for this registrar.
    #[allow(dead_code)]
    pub fn realm(&self) -> &str {
        &self.realm
    }

    /// Handle a REGISTER request.
    ///
    /// # Arguments
    /// * `request` - The REGISTER request
    ///
    /// # Returns
    /// The response to send (200 OK, 401 Unauthorized, etc.)
    pub async fn handle_register(&self, request: &Request) -> Response {
        self.handle_register_with_flow(request, None).await
    }

    /// Handle a REGISTER request with optional flow token for RFC 5626.
    ///
    /// # Arguments
    /// * `request` - The REGISTER request
    /// * `flow_token` - Optional flow token for TLS connection reuse
    ///
    /// # Returns
    /// The response to send (200 OK, 401 Unauthorized, etc.)
    pub async fn handle_register_with_flow(
        &self,
        request: &Request,
        flow_token: Option<&str>,
    ) -> Response {
        // Extract AOR early for event emission
        let aor = self.extract_aor_from_request(request).unwrap_or_default();
        let username_hint = extract_username_from_aor(&aor);
        let tenant_id = self
            .resolve_tenant(request, Some(&aor), username_hint.as_deref())
            .await
            .ok();

        // Handle authentication if enabled (toggled at runtime via AtomicBool)
        if self.auth_enabled.load(Ordering::Relaxed) {
            if let Some(ref authenticator) = self.authenticator {
                debug!("Handling REGISTER with authentication");

                // Check if request has Authorization header
                let has_auth = request
                    .headers()
                    .get_smol("Authorization")
                    .or_else(|| request.headers().get_smol("Proxy-Authorization"))
                    .is_some();

                if !has_auth {
                    // No auth header - send challenge (not a failure, just initial challenge)
                    debug!("No Authorization header, sending 401 challenge");
                    return match authenticator.challenge(request) {
                        Ok(response) => response,
                        Err(e) => {
                            tracing::error!("Failed to create auth challenge: {}", e);
                            return self.create_error_response(
                                request,
                                500,
                                "Internal Server Error",
                            );
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
                        // Emit registration failed event
                        self.publish_event(RegistrationEvent::Failed {
                            aor: aor.clone(),
                            tenant_id: tenant_id.unwrap_or_else(TenantId::nil),
                            reason: "authentication_failed".to_string(),
                            timestamp: SystemTime::now(),
                        });
                        return match authenticator.challenge(request) {
                            Ok(response) => response,
                            Err(e) => {
                                tracing::error!("Failed to create auth challenge: {}", e);
                                return self.create_error_response(
                                    request,
                                    500,
                                    "Internal Server Error",
                                );
                            }
                        };
                    }
                    Err(e) => {
                        tracing::error!("Authentication error: {}", e);
                        // Emit registration failed event
                        self.publish_event(RegistrationEvent::Failed {
                            aor,
                            tenant_id: tenant_id.unwrap_or_else(TenantId::nil),
                            reason: format!("authentication_error: {}", e),
                            timestamp: SystemTime::now(),
                        });
                        return self.create_error_response(request, 500, "Internal Server Error");
                    }
                }
            } else {
                debug!("Auth enabled but no credential store available, skipping authentication");
            }
        } else {
            debug!("Handling REGISTER without authentication");
        }

        // Authentication passed or not required - handle registration
        self.process_registration(request, flow_token).await
    }

    /// Extract AOR from a REGISTER request's To header.
    fn extract_aor_from_request(&self, request: &Request) -> Option<String> {
        extract_aor_from_request(request)
    }

    async fn resolve_tenant(
        &self,
        _request: &Request,
        aor_hint: Option<&str>,
        username_hint: Option<&str>,
    ) -> Result<TenantId> {
        resolve_tenant_for_inputs(
            self.tenant_store.as_ref(),
            self.credential_store.as_ref(),
            self.default_tenant_id,
            aor_hint,
            username_hint,
        )
        .await
    }

    /// Process the actual registration logic
    async fn process_registration(&self, request: &Request, flow_token: Option<&str>) -> Response {
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
                return self.create_error_response(request, 400, "Bad Request - Invalid To header");
            }
        };

        let aor = sip_registrar::normalize_aor(to.inner().uri())
            .unwrap_or_else(|_| to.inner().uri().to_string());
        let username_hint = extract_username_from_aor(&aor);
        let tenant_id = match self
            .resolve_tenant(request, Some(&aor), username_hint.as_deref())
            .await
        {
            Ok(tenant_id) => tenant_id,
            Err(e) => {
                warn!(aor = %aor, error = %e, "Rejecting REGISTER: tenant resolution failed");
                let code = if e.to_string().contains("unknown tenant domain")
                    || e.to_string().contains("ambiguous username")
                    || e.to_string().contains("tenant context required")
                {
                    403
                } else {
                    503
                };
                let reason = if code == 403 {
                    "Forbidden - tenant resolution failed"
                } else {
                    "Tenant lookup unavailable"
                };
                return self.create_error_response(request, code, reason);
            }
        };

        // Extract Contact header
        let contact_header = request.headers().get_smol("Contact");

        // Extract User-Agent for event emission
        let user_agent = request.headers().get("User-Agent").map(|s| s.to_string());

        // Handle deregistration (Contact: *)
        if let Some(contact) = contact_header {
            if contact.trim() == "*" {
                // Delete all bindings
                match self
                    .location_store
                    .remove_all_in_tenant(&tenant_id, &aor)
                    .await
                {
                    Ok(_) => {
                        info!(aor = %aor, "Deregistered all contacts");
                        // Emit unregistration event for wildcard deregistration
                        self.publish_event(RegistrationEvent::Unregistered {
                            aor: aor.clone(),
                            contact: "*".to_string(),
                            tenant_id,
                            reason: "wildcard_deregistration".to_string(),
                            timestamp: SystemTime::now(),
                        });
                        return self.create_ok_response(request, vec![]);
                    }
                    Err(e) => {
                        tracing::error!("Failed to deregister: {}", e);
                        return self.create_error_response(request, 500, "Internal Server Error");
                    }
                }
            }
        }

        // Extract expiry
        let expires = self.extract_expires(request);

        // Handle query (Expires: 0 means deregistration)
        if expires == 0 {
            if let Some(contact_str) = contact_header {
                // Strip contact-params (;expires=0, etc.) for matching
                let contact_uri = Self::extract_contact_uri(contact_str.as_str());
                // LocationStoreAdapter.remove() handles normalization internally
                match self
                    .location_store
                    .remove_in_tenant(&tenant_id, &aor, contact_uri)
                    .await
                {
                    Ok(_) => {
                        info!(aor = %aor, contact = %contact_uri, "Deregistered contact");
                        // Emit unregistration event
                        self.publish_event(RegistrationEvent::Unregistered {
                            aor: aor.clone(),
                            contact: contact_uri.to_string(),
                            tenant_id,
                            reason: "explicit".to_string(),
                            timestamp: SystemTime::now(),
                        });
                        return self.create_ok_response(request, vec![]);
                    }
                    Err(e) => {
                        tracing::error!("Failed to deregister contact: {}", e);
                        return self.create_error_response(request, 500, "Internal Server Error");
                    }
                }
            }
        }

        // Handle registration (Expires > 0)
        if let Some(contact_str) = contact_header {
            // Strip contact-params (like ;expires=0)
            let contact_uri = Self::extract_contact_uri(contact_str.as_str());
            let binding =
                match Self::create_binding(&aor, contact_uri, expires, request, flow_token) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::error!("Failed to create binding: {}", e);
                        return self.create_error_response(request, 400, "Bad Request");
                    }
                };

            match self
                .location_store
                .upsert_in_tenant(binding, tenant_id)
                .await
            {
                Ok(_) => {
                    info!(aor = %aor, contact = %contact_uri, expires = expires, "Registration successful");
                    // Emit registration event
                    self.publish_event(RegistrationEvent::Registered {
                        aor: aor.clone(),
                        contact: contact_uri.to_string(),
                        tenant_id,
                        expires_secs: expires,
                        user_agent: user_agent.clone(),
                        timestamp: SystemTime::now(),
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to store registration: {}", e);
                    return self.create_error_response(request, 500, "Internal Server Error");
                }
            }
        }

        // Retrieve all current bindings and return in response
        match self.location_store.lookup_in_tenant(&tenant_id, &aor).await {
            Ok(bindings) => self.create_ok_response(request, bindings),
            Err(e) => {
                tracing::error!("Failed to lookup registrations: {}", e);
                self.create_error_response(request, 500, "Internal Server Error")
            }
        }
    }

    /// Extract the Contact URI without contact-params (strips ;expires=N, etc.)
    ///
    /// RFC 3261: Contact header = name-addr / addr-spec [SEMI contact-params]
    /// We need to strip contact-params for matching registrations
    fn extract_contact_uri(contact_str: &str) -> &str {
        let trimmed = contact_str.trim();

        // Find the end of the URI (before contact-params like ;expires=0)
        // We need to handle angle brackets: <sip:user@host>;expires=0
        if let Some(gt_pos) = trimmed.find('>') {
            // Has angle brackets - return everything up to and including '>'
            &trimmed[..=gt_pos]
        } else {
            // No angle brackets - find first semicolon (start of contact-params)
            if let Some(semi_pos) = trimmed.find(';') {
                &trimmed[..semi_pos]
            } else {
                // No parameters, return as-is
                trimmed
            }
        }
    }

    /// Normalize Contact URI for matching by removing transient SIP URI parameters.
    ///
    /// RFC 5626 rinstance is a transient parameter that changes between client restarts.
    /// For matching purposes, we should compare contacts based on stable identity:
    /// user@host:port and stable parameters like transport.
    ///
    /// This ensures unregistration works even if rinstance changes.
    pub fn normalize_contact_for_matching(contact: &str) -> String {
        let trimmed = contact.trim();

        // Extract the part inside angle brackets
        if let Some(start) = trimmed.find('<') {
            if let Some(end) = trimmed.find('>') {
                let uri_part = &trimmed[start + 1..end];

                // Split on semicolons to get URI parameters
                let parts: Vec<&str> = uri_part.split(';').collect();
                if parts.is_empty() {
                    return format!("<{}>", uri_part);
                }

                // Keep the base URI and stable parameters (transport, user, etc.)
                // Remove transient parameters (rinstance, gr)
                let base_uri = parts[0];
                let mut stable_params: Vec<&str> = Vec::new();

                for param in &parts[1..] {
                    let param_name = param.split('=').next().unwrap_or("");
                    // Keep transport and other stable params, skip rinstance and gr
                    if param_name != "rinstance" && param_name != "gr" {
                        stable_params.push(param);
                    }
                }

                // Rebuild the contact URI
                if stable_params.is_empty() {
                    format!("<{}>", base_uri)
                } else {
                    format!("<{};{}>", base_uri, stable_params.join(";"))
                }
            } else {
                // Malformed, return as-is
                trimmed.to_string()
            }
        } else {
            // No angle brackets - still try to strip rinstance
            let parts: Vec<&str> = trimmed.split(';').collect();
            if parts.is_empty() {
                return trimmed.to_string();
            }

            let base_uri = parts[0];
            let stable_params: Vec<&str> = parts[1..]
                .iter()
                .filter(|param| {
                    let param_name = param.split('=').next().unwrap_or("");
                    param_name != "rinstance" && param_name != "gr"
                })
                .copied()
                .collect();

            if stable_params.is_empty() {
                base_uri.to_string()
            } else {
                format!("{};{}", base_uri, stable_params.join(";"))
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
                        // Don't clamp expires=0 (unregister requests)
                        if exp == 0 {
                            debug!("Client requested expires=0 (deregistration)");
                            return 0;
                        }
                        let clamped = exp.clamp(self.min_expires, self.max_expires);
                        debug!(
                            requested = exp,
                            accepted = clamped,
                            source = "Contact header",
                            "Client requested expires value"
                        );
                        return clamped;
                    }
                }
            }
        }

        // Check Expires header
        if let Some(expires_str) = request.headers().get("Expires") {
            if let Ok(exp) = expires_str.parse::<u32>() {
                // Don't clamp expires=0 (unregister requests)
                if exp == 0 {
                    debug!("Client requested expires=0 (deregistration)");
                    return 0;
                }
                let clamped = exp.clamp(self.min_expires, self.max_expires);
                debug!(
                    requested = exp,
                    accepted = clamped,
                    source = "Expires header",
                    "Client requested expires value"
                );
                return clamped;
            }
        }

        // Default
        debug!(
            default = self.default_expires,
            "No expires specified by client, using default"
        );
        self.default_expires
    }

    /// Create a binding from request parameters
    fn create_binding(
        aor: &str,
        contact: &str,
        expires: u32,
        request: &Request,
        flow_token: Option<&str>,
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

        // Extract and add Path header if present (RFC 3327)
        if let Ok(path) = sip_parse::parse_path(request.headers()) {
            if !path.is_empty() {
                debug!(path = %path, "Storing Path header from REGISTER");
                binding = binding.with_path(path);
            }
        }

        // Add flow token if present (RFC 5626)
        if let Some(token) = flow_token {
            debug!(flow_token = %token, "Storing flow token from TLS REGISTER");
            binding = binding.with_flow_token(SmolStr::new(token));
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

        // Add Contact headers for each binding and collect Service-Route info
        for binding in &bindings {
            let contact_str = binding.contact();
            // Check if contact already has angle brackets
            let contact = if contact_str.starts_with('<') {
                // Already has angle brackets, just add expires parameter
                format!("{};expires={}", contact_str, binding.expires().as_secs())
            } else {
                // No angle brackets, wrap it
                format!("<{}>;expires={}", contact_str, binding.expires().as_secs())
            };
            let _ = headers.push(SmolStr::new("Contact"), SmolStr::new(contact));
        }

        // Add Service-Route header for bindings with Path (RFC 3608)
        // The Service-Route informs the UA of the route set to use for subsequent requests.
        // We derive it from the stored Path headers.
        for binding in &bindings {
            if let Some(path) = binding.path() {
                // Path and Service-Route have the same format
                let _ = headers.push(
                    SmolStr::new("Service-Route"),
                    SmolStr::new(path.to_string()),
                );
                debug!(service_route = %path, "Added Service-Route to 200 OK");
            }
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
    ///
    /// If domain aliases are configured, this will also look up registrations
    /// for the same user at aliased domains.
    pub async fn get_registrations(&self, aor: &str) -> Result<Vec<Binding>> {
        let tenant_id = resolve_tenant_for_inputs(
            self.tenant_store.as_ref(),
            self.credential_store.as_ref(),
            self.default_tenant_id,
            Some(aor),
            None,
        )
        .await?;

        self.get_registrations_impl(&tenant_id, aor).await
    }

    /// Get registrations using an already-known tenant ID.
    ///
    /// Use this when the caller already has the tenant context (e.g., from
    /// an INVITE that already passed tenant resolution). This avoids a
    /// second tenant resolution pass that can fail for IP-literal AOR
    /// domains in multi-tenant deployments.
    pub async fn get_registrations_for_tenant(
        &self,
        tenant_id: &TenantId,
        aor: &str,
    ) -> Result<Vec<Binding>> {
        self.get_registrations_impl(tenant_id, aor).await
    }

    async fn get_registrations_impl(
        &self,
        tenant_id: &TenantId,
        aor: &str,
    ) -> Result<Vec<Binding>> {
        // Normalize AOR so lookups match stored keys even if clients send
        // params or percent-encoded users.
        let normalized = match sip_core::Uri::parse(aor) {
            Ok(uri) => sip_registrar::normalize_aor(&uri)
                .unwrap_or_else(|_| aor.split(';').next().unwrap_or(aor).to_string()),
            Err(_) => aor.split(';').next().unwrap_or(aor).to_string(),
        };

        // Look up the primary AOR
        let mut all_bindings = self
            .location_store
            .lookup_in_tenant(tenant_id, &normalized)
            .await?;

        // If domain aliases are configured, check for registrations at aliased domains
        if !self.domain_aliases.is_empty() {
            if let Some((user, domain)) = Self::split_aor(&normalized) {
                // Check if the domain is in our alias list
                let domain_lower = domain.to_lowercase();
                let is_aliased_domain = self
                    .domain_aliases
                    .iter()
                    .any(|a| a.to_lowercase() == domain_lower);

                if is_aliased_domain {
                    // Look up all aliased domains
                    for alias in &self.domain_aliases {
                        if alias.to_lowercase() != domain_lower {
                            let aliased_aor = format!("sip:{}@{}", user, alias);
                            if let Ok(mut bindings) = self
                                .location_store
                                .lookup_in_tenant(tenant_id, &aliased_aor)
                                .await
                            {
                                debug!(
                                    original_aor = %normalized,
                                    aliased_aor = %aliased_aor,
                                    found = bindings.len(),
                                    "Found registrations at aliased domain"
                                );
                                all_bindings.append(&mut bindings);
                            }
                        }
                    }
                }
            }
        }

        Ok(all_bindings)
    }

    /// Look up device registrations with domain fallback.
    ///
    /// First tries the device's stored `sip_uri` (e.g. `sip:polycom@192.168.1.225`).
    /// If no bindings are found, extracts the username and retries with
    /// `fallback_domain` (e.g. `sip:polycom@acme.feryx.io`).  This handles the
    /// common case where the device record uses the server IP but the phone
    /// registered under the tenant's SIP domain.
    pub async fn get_registrations_for_device(
        &self,
        tenant_id: &TenantId,
        sip_uri: &str,
        fallback_domain: &str,
    ) -> Result<Vec<Binding>> {
        let primary = self.get_registrations_impl(tenant_id, sip_uri).await?;
        if !primary.is_empty() {
            return Ok(primary);
        }

        // Extract username from the device sip_uri and retry with the
        // fallback domain (typically the INVITE's domain).
        if let Some((user, device_domain)) = Self::split_aor(sip_uri) {
            let fb_lower = fallback_domain.trim().to_ascii_lowercase();
            if !fb_lower.is_empty() && fb_lower != device_domain.to_ascii_lowercase() {
                let fallback_aor = format!("sip:{}@{}", user, fallback_domain);
                debug!(
                    device_uri = %sip_uri,
                    fallback_aor = %fallback_aor,
                    "Device AOR returned no bindings, trying INVITE-domain fallback"
                );
                return self.get_registrations_impl(tenant_id, &fallback_aor).await;
            }
        }

        Ok(Vec::new())
    }

    /// Split an AOR into user and domain parts.
    /// Returns None if the AOR doesn't have the expected format.
    fn split_aor(aor: &str) -> Option<(&str, &str)> {
        // Parse `sip:` or `sips:` prefix.
        let without_scheme = aor
            .strip_prefix("sip:")
            .or_else(|| aor.strip_prefix("sips:"))?;
        let at_pos = without_scheme.find('@')?;
        let user = &without_scheme[..at_pos];
        let hostport_and_params = &without_scheme[at_pos + 1..];

        // RFC 3261 URI layout: scheme:user@hostport;uri-params?headers
        // Strip anything after the host/port — URI parameters (`;...`) and
        // any trailing headers (`?...`) are NOT part of the host and must
        // not be included when matching against the tenant `sip_domains`
        // allowlist or when detecting IP literals.
        let hostport = hostport_and_params
            .split([';', '?'])
            .next()
            .unwrap_or(hostport_and_params);

        // Strip the port from the host. IPv6 literals are wrapped in
        // square brackets, so keep the brackets and only strip the port
        // that follows the closing bracket. For IPv4 / hostname forms,
        // the port is after the single `:`.
        let host = if hostport.starts_with('[') {
            hostport
                .find(']')
                .map(|end| &hostport[..=end])
                .unwrap_or(hostport)
        } else {
            hostport.split(':').next().unwrap_or(hostport)
        };

        Some((user, host))
    }

    /// Get media preferences for a user identified by AOR.
    ///
    /// # Arguments
    /// * `aor` - The SIP AOR used to resolve tenant and username
    ///
    /// # Returns
    /// The user's media preferences, or default preferences if user not found
    pub async fn get_user_media_preferences(&self, aor: &str) -> fcp_core::MediaPreferences {
        if let Some(ref cred_store) = self.credential_store {
            let Some(username) = extract_username_from_aor(aor) else {
                debug!(aor = %aor, "Invalid AOR for media preference lookup, using defaults");
                return fcp_core::MediaPreferences::default();
            };

            let tenant_id = match resolve_tenant_for_inputs(
                self.tenant_store.as_ref(),
                self.credential_store.as_ref(),
                self.default_tenant_id,
                Some(aor),
                Some(&username),
            )
            .await
            {
                Ok(tenant_id) => tenant_id,
                Err(e) => {
                    warn!(aor = %aor, error = %e, "Failed to resolve tenant for media preferences");
                    return fcp_core::MediaPreferences::default();
                }
            };

            match cred_store.get_user_in_tenant(&username, &tenant_id).await {
                Ok(Some(user)) => {
                    debug!(
                        aor = %aor,
                        tenant_id = %tenant_id,
                        force_anchor = %user.media_preferences.force_media_anchoring,
                        dtmf = %user.media_preferences.enable_dtmf_detection,
                        record = %user.media_preferences.record_all_calls,
                        "Retrieved user media preferences"
                    );
                    return user.media_preferences;
                }
                Ok(None) => {
                    debug!(
                        aor = %aor,
                        tenant_id = %tenant_id,
                        "User not found in tenant, using default media preferences"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        aor = %aor,
                        tenant_id = %tenant_id,
                        error = %e,
                        "Failed to look up user for media preferences"
                    );
                }
            }
        }

        // Return default preferences if no credential store or user not found
        fcp_core::MediaPreferences::default()
    }
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    use fcp_core::FileBackedStorage;
    use smol_str::SmolStr;

    #[test]
    fn split_aor_strips_uri_parameters_from_host() {
        // URI parameters after the host must not leak into the host slice
        // — otherwise IP literal detection and tenant allowlist lookups
        // both break.
        let (user, host) =
            FcpRegistrar::split_aor("sip:jerry@192.168.1.225;transport=tcp").unwrap();
        assert_eq!(user, "jerry");
        assert_eq!(host, "192.168.1.225");
    }

    #[test]
    fn split_aor_strips_port_and_parameters() {
        let (user, host) =
            FcpRegistrar::split_aor("sip:alice@192.168.1.80:56720;transport=tcp;lr").unwrap();
        assert_eq!(user, "alice");
        assert_eq!(host, "192.168.1.80");
    }

    #[test]
    fn split_aor_strips_headers_after_host() {
        let (user, host) = FcpRegistrar::split_aor("sip:bob@acme.fcp.net?Subject=Hi").unwrap();
        assert_eq!(user, "bob");
        assert_eq!(host, "acme.fcp.net");
    }

    #[test]
    fn split_aor_keeps_ipv6_brackets_and_strips_port() {
        let (user, host) = FcpRegistrar::split_aor("sip:carol@[2001:db8::1]:5060").unwrap();
        assert_eq!(user, "carol");
        assert_eq!(host, "[2001:db8::1]");
    }

    #[test]
    fn split_aor_plain_hostname_is_unchanged() {
        let (user, host) = FcpRegistrar::split_aor("sip:dave@acme.fcp.net").unwrap();
        assert_eq!(user, "dave");
        assert_eq!(host, "acme.fcp.net");
    }

    #[test]
    fn is_host_literal_detects_ipv4() {
        assert!(is_host_literal("192.168.1.225"));
        assert!(is_host_literal("10.0.0.1"));
        assert!(is_host_literal("127.0.0.1"));
    }

    #[test]
    fn is_host_literal_detects_localhost() {
        assert!(is_host_literal("localhost"));
        assert!(is_host_literal("LOCALHOST"));
    }

    #[test]
    fn is_host_literal_detects_bracketed_ipv6() {
        assert!(is_host_literal("[::1]"));
        assert!(is_host_literal("[2001:db8::1]"));
    }

    #[test]
    fn is_host_literal_rejects_real_hostnames() {
        assert!(!is_host_literal("acme.fcp.net"));
        assert!(!is_host_literal("sip.example.com"));
    }

    #[tokio::test]
    async fn test_registrar_without_auth() {
        let temp_dir = std::env::temp_dir().join("fcp-test-registrar");
        let storage = Arc::new(FileBackedStorage::new(temp_dir.clone()).await.unwrap());

        let registrar = FcpRegistrar::new(storage.clone(), None, None, false)
            .with_default_tenant_id(fcp_core::TenantId::nil());

        // Create a simple REGISTER request
        let mut headers = sip_core::Headers::new();
        headers
            .push(
                SmolStr::new("Via"),
                SmolStr::new("SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK123"),
            )
            .unwrap();
        headers
            .push(
                SmolStr::new("From"),
                SmolStr::new("<sip:alice@example.com>;tag=abc123"),
            )
            .unwrap();
        headers
            .push(SmolStr::new("To"), SmolStr::new("<sip:alice@example.com>"))
            .unwrap();
        headers
            .push(
                SmolStr::new("Call-ID"),
                SmolStr::new("test-call-id@example.com"),
            )
            .unwrap();
        headers
            .push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER"))
            .unwrap();
        headers
            .push(
                SmolStr::new("Contact"),
                SmolStr::new("<sip:alice@192.168.1.1:5060>;expires=3600"),
            )
            .unwrap();
        headers
            .push(SmolStr::new("Expires"), SmolStr::new("3600"))
            .unwrap();
        headers
            .push(SmolStr::new("Content-Length"), SmolStr::new("0"))
            .unwrap();

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
            id: fcp_core::UserId::new(),
            username: "alice".to_string(),
            password: "secret123".to_string(),
            digest_password: None,
            sip_ha1_hash: None,
            sip_provisioning_password: None,
            display_name: Some("Alice".to_string()),
            tenant_id: fcp_core::TenantId::nil(), // Default tenant for test
            role: fcp_core::UserRole::default(),
            department_id: None,
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
            location_id: None,
            language: "en-US".to_string(),
            timezone: "UTC".to_string(),
            last_login: None,
            emergency_contact: fcp_core::EmergencyContact::default(),
            primary_phone_number_id: None,
            voicemail_box_id: None,
            profile_picture: None,
            account_status: fcp_core::AccountStatus::Active,
            failed_login_attempts: 0,
            account_locked_until: None,
        };
        storage.put_user(user).await.unwrap();

        // Create registrar WITH authentication
        let registrar = FcpRegistrar::new(
            storage.clone(),
            Some(storage.clone()), // Use storage as credential store
            Some("test.local".to_string()),
            true, // auth enabled
        )
        .with_default_tenant_id(fcp_core::TenantId::nil());

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

    #[tokio::test]
    async fn test_reregister_different_rinstance_no_duplicates() {
        let temp_dir = std::env::temp_dir().join("fcp-test-registrar-rinstance");
        let _ = tokio::fs::remove_dir_all(&temp_dir).await; // Clean up from previous runs
        let storage = Arc::new(FileBackedStorage::new(temp_dir.clone()).await.unwrap());

        let registrar = FcpRegistrar::new(storage.clone(), None, None, false)
            .with_default_tenant_id(fcp_core::TenantId::nil());

        // First REGISTER with rinstance=aaa
        let mut headers = sip_core::Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/TCP 192.168.1.206:60312;branch=z9hG4bK001"),
        );
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:bob@example.com>;tag=t1"),
        );
        headers.push(SmolStr::new("To"), SmolStr::new("<sip:bob@example.com>"));
        headers.push(SmolStr::new("Call-ID"), SmolStr::new("call-1@example.com"));
        headers.push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER"));
        headers.push(
            SmolStr::new("Contact"),
            SmolStr::new(
                "<sip:bob@192.168.1.206:60312;rinstance=aaa111;transport=tcp>;expires=3600",
            ),
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
        .unwrap();

        let response = registrar.handle_register(&request).await;
        assert_eq!(response.start_line().code(), 200);

        let regs = storage
            .get_registrations("sip:bob@example.com")
            .await
            .unwrap();
        assert_eq!(
            regs.len(),
            1,
            "Should have exactly 1 registration after first REGISTER"
        );

        // Second REGISTER with different rinstance=bbb (same device restarted)
        let mut headers2 = sip_core::Headers::new();
        headers2.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/TCP 192.168.1.206:60312;branch=z9hG4bK002"),
        );
        headers2.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:bob@example.com>;tag=t2"),
        );
        headers2.push(SmolStr::new("To"), SmolStr::new("<sip:bob@example.com>"));
        headers2.push(SmolStr::new("Call-ID"), SmolStr::new("call-2@example.com"));
        headers2.push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER"));
        headers2.push(
            SmolStr::new("Contact"),
            SmolStr::new(
                "<sip:bob@192.168.1.206:60312;rinstance=bbb222;transport=tcp>;expires=3600",
            ),
        );
        headers2.push(SmolStr::new("Expires"), SmolStr::new("3600"));
        headers2.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        let request2 = Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Register,
                sip_core::SipUri::parse("sip:example.com").unwrap(),
            ),
            headers2,
            bytes::Bytes::new(),
        )
        .unwrap();

        let response2 = registrar.handle_register(&request2).await;
        assert_eq!(response2.start_line().code(), 200);

        let regs = storage
            .get_registrations("sip:bob@example.com")
            .await
            .unwrap();
        assert_eq!(
            regs.len(),
            1,
            "Should still have exactly 1 registration after re-REGISTER with different rinstance"
        );

        // Verify the stored contact is normalized (no rinstance)
        assert!(
            !regs[0].contact.contains("rinstance"),
            "Stored contact should not contain rinstance: {}",
            regs[0].contact
        );
        assert!(
            regs[0].contact.contains("transport=tcp"),
            "Stored contact should keep transport param: {}",
            regs[0].contact
        );

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir).await.ok();
    }

    #[tokio::test]
    async fn test_unregister_removes_registration() {
        let temp_dir = std::env::temp_dir().join("fcp-test-registrar-unreg");
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        let storage = Arc::new(FileBackedStorage::new(temp_dir.clone()).await.unwrap());

        let registrar = FcpRegistrar::new(storage.clone(), None, None, false)
            .with_default_tenant_id(fcp_core::TenantId::nil());

        // Register first
        let mut headers = sip_core::Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/TCP 192.168.1.206:60312;branch=z9hG4bK001"),
        );
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:bob@example.com>;tag=t1"),
        );
        headers.push(SmolStr::new("To"), SmolStr::new("<sip:bob@example.com>"));
        headers.push(SmolStr::new("Call-ID"), SmolStr::new("call-1@example.com"));
        headers.push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER"));
        headers.push(
            SmolStr::new("Contact"),
            SmolStr::new("<sip:bob@192.168.1.206:60312;rinstance=abc;transport=tcp>;expires=3600"),
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
        .unwrap();

        let response = registrar.handle_register(&request).await;
        assert_eq!(response.start_line().code(), 200);

        let regs = storage
            .get_registrations("sip:bob@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 1);

        // Now unregister with Expires: 0 and a different rinstance
        let mut headers2 = sip_core::Headers::new();
        headers2.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/TCP 192.168.1.206:60312;branch=z9hG4bK002"),
        );
        headers2.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:bob@example.com>;tag=t2"),
        );
        headers2.push(SmolStr::new("To"), SmolStr::new("<sip:bob@example.com>"));
        headers2.push(SmolStr::new("Call-ID"), SmolStr::new("call-2@example.com"));
        headers2.push(SmolStr::new("CSeq"), SmolStr::new("2 REGISTER"));
        headers2.push(
            SmolStr::new("Contact"),
            SmolStr::new("<sip:bob@192.168.1.206:60312;rinstance=xyz;transport=tcp>;expires=0"),
        );
        headers2.push(SmolStr::new("Expires"), SmolStr::new("0"));
        headers2.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        let request2 = Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Register,
                sip_core::SipUri::parse("sip:example.com").unwrap(),
            ),
            headers2,
            bytes::Bytes::new(),
        )
        .unwrap();

        let response2 = registrar.handle_register(&request2).await;
        assert_eq!(response2.start_line().code(), 200);

        // Verify registration was removed
        let regs = storage
            .get_registrations("sip:bob@example.com")
            .await
            .unwrap();
        assert_eq!(
            regs.len(),
            0,
            "Registration should be removed after Expires: 0"
        );

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir).await.ok();
    }

    #[tokio::test]
    async fn test_wildcard_deregister_removes_all() {
        let temp_dir = std::env::temp_dir().join("fcp-test-registrar-wildcard");
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        let storage = Arc::new(FileBackedStorage::new(temp_dir.clone()).await.unwrap());

        let registrar = FcpRegistrar::new(storage.clone(), None, None, false)
            .with_default_tenant_id(fcp_core::TenantId::nil());

        // Register a contact
        let mut headers = sip_core::Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK001"),
        );
        headers.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:alice@example.com>;tag=t1"),
        );
        headers.push(SmolStr::new("To"), SmolStr::new("<sip:alice@example.com>"));
        headers.push(SmolStr::new("Call-ID"), SmolStr::new("call-1@example.com"));
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
        .unwrap();

        registrar.handle_register(&request).await;
        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(regs.len(), 1);

        // Wildcard deregister (Contact: *)
        let mut headers2 = sip_core::Headers::new();
        headers2.push(
            SmolStr::new("Via"),
            SmolStr::new("SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK002"),
        );
        headers2.push(
            SmolStr::new("From"),
            SmolStr::new("<sip:alice@example.com>;tag=t2"),
        );
        headers2.push(SmolStr::new("To"), SmolStr::new("<sip:alice@example.com>"));
        headers2.push(SmolStr::new("Call-ID"), SmolStr::new("call-2@example.com"));
        headers2.push(SmolStr::new("CSeq"), SmolStr::new("2 REGISTER"));
        headers2.push(SmolStr::new("Contact"), SmolStr::new("*"));
        headers2.push(SmolStr::new("Expires"), SmolStr::new("0"));
        headers2.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

        let request2 = Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Register,
                sip_core::SipUri::parse("sip:example.com").unwrap(),
            ),
            headers2,
            bytes::Bytes::new(),
        )
        .unwrap();

        let response = registrar.handle_register(&request2).await;
        assert_eq!(response.start_line().code(), 200);

        let regs = storage
            .get_registrations("sip:alice@example.com")
            .await
            .unwrap();
        assert_eq!(
            regs.len(),
            0,
            "All registrations should be removed after Contact: *"
        );

        // Cleanup
        tokio::fs::remove_dir_all(temp_dir).await.ok();
    }

    #[test]
    fn test_extract_contact_uri() {
        // With angle brackets and expires parameter
        assert_eq!(
            FcpRegistrar::extract_contact_uri("<sip:bob@192.168.1.206:5060>;expires=0"),
            "<sip:bob@192.168.1.206:5060>"
        );

        // With angle brackets, rinstance, and expires
        assert_eq!(
            FcpRegistrar::extract_contact_uri(
                "<sip:bob@192.168.1.206:60312;rinstance=f4d3c22d28a963ab;transport=tcp>;expires=0"
            ),
            "<sip:bob@192.168.1.206:60312;rinstance=f4d3c22d28a963ab;transport=tcp>"
        );

        // Without angle brackets
        assert_eq!(
            FcpRegistrar::extract_contact_uri("sip:bob@192.168.1.206:5060;expires=3600"),
            "sip:bob@192.168.1.206:5060"
        );

        // No parameters
        assert_eq!(
            FcpRegistrar::extract_contact_uri("<sip:alice@example.com>"),
            "<sip:alice@example.com>"
        );

        // Multiple parameters
        assert_eq!(
            FcpRegistrar::extract_contact_uri("<sip:user@host:5060>;expires=3600;q=0.8"),
            "<sip:user@host:5060>"
        );

        // With whitespace
        assert_eq!(
            FcpRegistrar::extract_contact_uri("  <sip:user@host>;expires=0  "),
            "<sip:user@host>"
        );
    }

    #[test]
    fn test_normalize_contact_for_matching() {
        // Strip rinstance but keep transport
        assert_eq!(
            FcpRegistrar::normalize_contact_for_matching(
                "<sip:bob@192.168.1.206:60312;rinstance=f4d3c22d28a963ab;transport=tcp>"
            ),
            "<sip:bob@192.168.1.206:60312;transport=tcp>"
        );

        // Different rinstance should normalize to same result
        assert_eq!(
            FcpRegistrar::normalize_contact_for_matching(
                "<sip:bob@192.168.1.206:60312;rinstance=5c6c24656fccfc49;transport=tcp>"
            ),
            "<sip:bob@192.168.1.206:60312;transport=tcp>"
        );

        // Strip gr parameter (GRUU instance ID) but keep transport
        assert_eq!(
            FcpRegistrar::normalize_contact_for_matching(
                "<sip:bob@example.com;gr=abc123;transport=tls>"
            ),
            "<sip:bob@example.com;transport=tls>"
        );

        // No transient parameters - return normalized
        assert_eq!(
            FcpRegistrar::normalize_contact_for_matching("<sip:alice@example.com>"),
            "<sip:alice@example.com>"
        );

        // Only stable parameters
        assert_eq!(
            FcpRegistrar::normalize_contact_for_matching("<sip:alice@example.com;transport=udp>"),
            "<sip:alice@example.com;transport=udp>"
        );

        // Without angle brackets
        assert_eq!(
            FcpRegistrar::normalize_contact_for_matching(
                "sip:bob@host;rinstance=xyz;transport=tcp"
            ),
            "sip:bob@host;transport=tcp"
        );

        // Multiple stable parameters
        assert_eq!(
            FcpRegistrar::normalize_contact_for_matching(
                "<sip:user@host;user=phone;transport=tcp;rinstance=abc>"
            ),
            "<sip:user@host;user=phone;transport=tcp>"
        );
    }
}
