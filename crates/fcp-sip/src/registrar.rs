//! Registrar service for handling SIP REGISTER requests.
//!
//! This module provides integration between FCP storage and siphon-rs registrar.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use fcp_core::{
    is_host_literal, CredentialStore, Registration, RegistrationStore, TenantId, TenantStore,
};
use fcp_events::{EventPublisher, PlatformEvent, RegistrationEvent};
use sip_core::{Method, Request, Response};
use sip_registrar::{AsyncLocationStore, Binding};
use smol_str::SmolStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, info, warn};

use crate::auth::{Ha1Authenticator, Ha1CredentialStore};
use crate::registration_cache::{Lookup, RegistrationCache};

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
pub(crate) async fn resolve_tenant_for_inputs(
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
        let users = store.list_users_by_sip_name(&username).await?;
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

/// Whether a live registration places its device at `peer`.
///
/// The address the REGISTER arrived from wins wherever the registrar kept
/// one (`docs/E911_BLOCK_2.md` §6.2): it is what the network saw, not what
/// the device claimed. Otherwise the contact's host is parsed and compared
/// as an address. A contact whose host is a name, not an address, places
/// nobody — resolving it would let a device's DNS decide who may call.
///
/// Searching the contact's *text* for the peer's address, as this check
/// once did, authorises a stranger: `sip:bob@10.0.0.10:5060` contains the
/// string `10.0.0.1` (`docs/EDGE_SECURITY.md` §2.2).
pub fn registration_is_at(registration: &Registration, peer: std::net::IpAddr) -> bool {
    if let Some(source) = registration
        .source_address
        .as_deref()
        .and_then(fcp_core::e911::parse_address)
    {
        return same_address(source, peer);
    }
    contact_host(&registration.contact)
        .map(|host| same_address(host, peer))
        .unwrap_or(false)
}

/// The host of a stored contact, when it is an address.
///
/// Accepts the forms the store holds: bare, in angle brackets, with URI
/// parameters, with a port, and IPv6 in brackets.
fn contact_host(contact: &str) -> Option<std::net::IpAddr> {
    let uri = contact.trim();
    let uri = match (uri.find('<'), uri.find('>')) {
        (Some(start), Some(end)) if start < end => &uri[start + 1..end],
        _ => uri,
    };
    let uri = uri.split(';').next()?.trim();
    let uri = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .unwrap_or(uri);
    // The user part may itself contain an `@`, so take the last one.
    let host = match uri.rfind('@') {
        Some(at) => &uri[at + 1..],
        None => uri,
    };
    let host = host.split('?').next().unwrap_or(host);
    fcp_core::e911::parse_address(host)
}

/// What an INVITE's credentials amount to
/// (`FcpRegistrar::authenticate_invite`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteAuth {
    /// Nothing to prove: `[registrar] auth_invites` is off, or this node
    /// has no credential store to check a digest against.
    NotRequired,
    /// Placed by a user who proved their password on a nonce this node
    /// issued, and whose `From` is their own.
    Admitted { username: String },
    /// No credentials, or a nonce that is not usable. Answered `401` with
    /// a challenge, never `403`: a phone that has lost its credentials has
    /// to be able to recover (`docs/EDGE_SECURITY.md` §4, Q2).
    Challenge { stale: bool },
    /// Real credentials wearing somebody else's `From`. No retry fixes
    /// that, so it is refused rather than challenged.
    WrongUser {
        authenticated: String,
        claimed: String,
    },
}

/// The address of record a request's `From` names — who it says placed
/// the request.
fn from_aor(request: &Request) -> Option<String> {
    let from = request.headers().get_smol("From")?;
    let from = sip_parse::parse_from_header(from)?;
    Some(
        sip_registrar::normalize_aor(from.inner().uri())
            .unwrap_or_else(|_| from.inner().uri().to_string()),
    )
}

/// The user part of a request's `From` — who the request says placed it —
/// or `None` when it names no user.
pub(crate) fn from_user(request: &Request) -> Option<String> {
    extract_username_from_aor(&from_aor(request)?)
}

/// Two addresses are the same device address; an IPv4 peer on a
/// dual-stack socket arrives mapped into IPv6.
fn same_address(a: std::net::IpAddr, b: std::net::IpAddr) -> bool {
    fn unmap(ip: std::net::IpAddr) -> std::net::IpAddr {
        match ip {
            std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map(std::net::IpAddr::V4).unwrap_or(ip),
            v4 => v4,
        }
    }
    unmap(a) == unmap(b)
}

/// Adapter that implements siphon-rs LocationStore using FCP RegistrationStore.
pub struct LocationStoreAdapter {
    store: Arc<dyn RegistrationStore>,
    /// Lookups kept for a TTL (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1,
    /// phase 2). `None` reads the store every time.
    cache: Option<Arc<RegistrationCache>>,
}

impl LocationStoreAdapter {
    pub fn new(store: Arc<dyn RegistrationStore>) -> Self {
        Self { store, cache: None }
    }

    /// The store has changed under `aor` in `tenant_id`.
    fn invalidate(&self, tenant_id: &TenantId, aor: &str) {
        if let Some(cache) = &self.cache {
            cache.invalidate(tenant_id, aor);
        }
    }

    /// The store has changed under `aor`, in whichever tenant holds it.
    fn invalidate_everywhere(&self, aor: &str) {
        if let Some(cache) = &self.cache {
            cache.invalidate_everywhere(aor);
        }
    }

    /// Store a binding; one from a phone that names its instance replaces
    /// that instance's other bindings (RFC 5626 §6: a woken app's new
    /// address, a phone that moved), which are returned.
    async fn upsert_in_tenant(
        &self,
        binding: Binding,
        tenant_id: TenantId,
        source_address: Option<std::net::SocketAddr>,
        details: BindingDetails,
    ) -> Result<Vec<String>> {
        let mut registration = Self::from_binding(&binding);
        registration.tenant_id = tenant_id;
        // Where the device registered from, which places a device that
        // moves (`docs/E911_BLOCK_2.md` §6.2).
        registration.source_address = source_address.map(|a| a.to_string());
        registration.contact =
            crate::registrar::FcpRegistrar::normalize_contact_for_matching(&registration.contact);
        if let Some(expires) = details.expires {
            registration.expires_at = SystemTime::now() + expires;
        }
        registration.instance_id = details.instance_id;
        registration.reg_id = details.reg_id;
        registration.push = details.push;
        registration.flow_node = details.flow_node;
        let aor = registration.aor.clone();
        let contact = registration.contact.clone();
        let instance = registration.instance_id.clone();
        let reg_id = registration.reg_id;
        let stored = self.store.put_registration(registration).await;
        self.invalidate(&tenant_id, &aor);
        stored?;

        let mut replaced = Vec::new();
        if let Some(instance) = instance {
            fcp_core::storage::RegistrationReads::global()
                .note(fcp_core::storage::RegistrationRead::Housekeeping);
            for old in self
                .store
                .get_registrations_in_tenant(&tenant_id, &aor)
                .await?
            {
                let same_phone = old.instance_id.as_deref() == Some(instance.as_str())
                    && (old.reg_id.is_none() || reg_id.is_none() || old.reg_id == reg_id);
                if same_phone && old.contact != contact {
                    self.store
                        .delete_registration_in_tenant(&tenant_id, &aor, &old.contact)
                        .await?;
                    replaced.push(old.contact);
                }
            }
            if !replaced.is_empty() {
                self.invalidate(&tenant_id, &aor);
            }
        }
        Ok(replaced)
    }

    async fn remove_in_tenant(&self, tenant_id: &TenantId, aor: &str, contact: &str) -> Result<()> {
        fcp_core::storage::RegistrationReads::global()
            .note(fcp_core::storage::RegistrationRead::Housekeeping);
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
                let deleted = self
                    .store
                    .delete_registration_in_tenant(tenant_id, aor, &reg.contact)
                    .await;
                self.invalidate(tenant_id, aor);
                deleted?;
                removed_any = true;
            }
        }

        if !removed_any {
            info!(aor = %aor, contact = %contact, normalized = %normalized_requested, "No matching registration to remove");
        }

        Ok(())
    }

    async fn remove_all_in_tenant(&self, tenant_id: &TenantId, aor: &str) -> Result<()> {
        let deleted = self
            .store
            .delete_all_registrations_in_tenant(tenant_id, aor)
            .await;
        self.invalidate(tenant_id, aor);
        deleted
    }

    /// The bindings a REGISTER's answer lists. Read from the store, never
    /// the cache — the answer is the store's word on what it now holds —
    /// and counted as housekeeping, since no call is waiting on it.
    async fn lookup_in_tenant(&self, tenant_id: &TenantId, aor: &str) -> Result<Vec<Registration>> {
        fcp_core::storage::RegistrationReads::global()
            .note(fcp_core::storage::RegistrationRead::Housekeeping);
        Ok(Self::live(
            self.store
                .get_registrations_in_tenant(tenant_id, aor)
                .await?,
        ))
    }

    /// The live registrations for an AOR, which carry the address the
    /// REGISTER came from — every lookup a call makes comes through here.
    ///
    /// From the cache when it holds a fresh answer, otherwise one round
    /// trip to the store; `read` says why that trip was made, so `/status`
    /// can tell a lookup from the alias lookups that follow it
    /// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1). Expiry is judged here on
    /// every call, so a binding that lapses while cached is not used.
    async fn rows_in_tenant(
        &self,
        tenant_id: &TenantId,
        aor: &str,
        read: fcp_core::storage::RegistrationRead,
    ) -> Result<Vec<Registration>> {
        let reads = fcp_core::storage::RegistrationReads::global();
        let ticket = match self.cache.as_ref().map(|c| c.lookup(tenant_id, aor)) {
            Some(Lookup::Hit(rows)) => {
                reads.note_cached();
                return Ok(Self::live(rows));
            }
            Some(Lookup::Miss(ticket)) => Some(ticket),
            None => None,
        };
        reads.note(read);
        let registrations = self
            .store
            .get_registrations_in_tenant(tenant_id, aor)
            .await?;
        if let (Some(cache), Some(ticket)) = (&self.cache, ticket) {
            cache.fill(tenant_id, aor, &registrations, ticket);
        }
        Ok(Self::live(registrations))
    }

    fn live(registrations: Vec<Registration>) -> Vec<Registration> {
        let now = SystemTime::now();
        registrations
            .into_iter()
            .filter(|r| r.expires_at > now)
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
        // And to siphon-rs's day at most: a push binding is kept longer
        // (`docs/PUSH.md` §2.1), and its row says how long.
        const MAX_EXPIRES_SECS: u64 = 86_400;
        let clamped_duration = std::time::Duration::from_secs(
            expires_duration
                .as_secs()
                .clamp(MIN_EXPIRES_SECS, MAX_EXPIRES_SECS),
        );

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
            source_address: None,
            instance_id: None,
            reg_id: None,
            push: None,
            flow_node: None,
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
        let aor = registration.aor.clone();
        let stored = self.store.put_registration(registration).await;
        self.invalidate_everywhere(&aor);
        stored
    }

    async fn remove(&self, aor: &str, contact: &str) -> Result<()> {
        // For removal, we need to normalize contacts for matching since rinstance changes
        // Get all registrations for this AOR
        fcp_core::storage::RegistrationReads::global()
            .note(fcp_core::storage::RegistrationRead::Housekeeping);
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
                let deleted = self.store.delete_registration(aor, &reg.contact).await;
                self.invalidate_everywhere(aor);
                deleted?;
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
        let deleted = self.store.delete_all_registrations(aor).await;
        self.invalidate_everywhere(aor);
        deleted
    }

    async fn lookup(&self, aor: &str) -> Result<Vec<Binding>> {
        fcp_core::storage::RegistrationReads::global()
            .note(fcp_core::storage::RegistrationRead::Housekeeping);
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
    /// Devices with credentials of their own (`docs/ENTERPRISE_CALLING.md`
    /// §3.8); without it only users authenticate.
    identity: Option<Arc<fcp_core::DynStorage>>,
}

impl CredentialStoreAdapter {
    pub fn new(store: Arc<dyn CredentialStore>, realm: String) -> Self {
        Self {
            store,
            realm,
            tenant_store: None,
            default_tenant_id: None,
            identity: None,
        }
    }

    pub fn with_identity(mut self, identity: Option<Arc<fcp_core::DynStorage>>) -> Self {
        self.identity = identity;
        self
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
        // Which address of record names the party proving a password. A
        // REGISTER's `To` is the registering user, but on any other
        // request — an INVITE — `To` is the *callee*, and looking the
        // caller's credentials up in the callee's tenant finds nobody
        // (`docs/EDGE_SECURITY.md` §2.1).
        let aor = if *request.method() == Method::Register {
            extract_aor_from_request(request)
        } else {
            from_aor(request)
        };
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

        if let Ok(Some(user)) = self.store.get_user_by_sip_name(username, &tenant_id).await {
            // A suspended user's phones stop; a Locked one's (the web
            // sign-in's lockout) do not.
            if user.may_use_services() {
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
            return None;
        }
        // A device's own credentials.
        let identity = self.identity.as_ref()?;
        match crate::hoteling::credential(identity, &tenant_id, username).await {
            Some(crate::hoteling::Credential::Device(device)) => {
                let ha1_hash = device.sip_ha1_hash?;
                Some(sip_auth::Credentials::new(
                    username,
                    ha1_hash.as_str(),
                    self.realm.clone(),
                ))
            }
            _ => None,
        }
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

    /// Whether an INVITE must carry a digest as well as come from a
    /// plausible address (`[registrar] auth_invites`,
    /// `docs/EDGE_SECURITY.md` §2.1).
    auth_invites: AtomicBool,

    /// Credential store for looking up user preferences
    credential_store: Option<Arc<dyn CredentialStore>>,

    /// Devices, extensions and lines: who a credential may register as,
    /// and devices' own credentials (`docs/ENTERPRISE_CALLING.md` §3.8).
    /// Without it any authenticated credential registers any address, as
    /// before.
    identity: Option<Arc<fcp_core::DynStorage>>,

    /// Registrations written on devices (`docs/PROVISIONING.md` §8).
    device_registrations: Option<Arc<crate::device_sync::DeviceRegistrations>>,

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

    /// Where the admin is, told to every phone a REGISTER succeeds for in
    /// `X-FCP-Provisioning-Url` (`docs/SOFTPHONE.md` §2.5): how the softphone
    /// finds its branding and settings without being told.
    provisioning_url: Option<String>,

    /// `[push]`: which apps this node can wake, and how long their bindings
    /// live (`docs/PUSH.md` §2.1). Without it a push address is refused
    /// with 555.
    push: Option<Arc<fcp_core::push::PushConfig>>,

    /// This node, named on the bindings whose flows it holds.
    node_id: Option<String>,
}

/// What a REGISTER says about its phone beyond the binding siphon-rs keeps.
#[derive(Debug, Clone, Default)]
pub(crate) struct BindingDetails {
    /// How long it is kept, where that is longer than siphon-rs's binding
    /// holds (a push binding's week).
    pub expires: Option<std::time::Duration>,
    pub instance_id: Option<String>,
    pub reg_id: Option<u32>,
    pub push: Option<fcp_core::push::PushAddress>,
    pub flow_node: Option<String>,
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
            auth_invites: AtomicBool::new(false),
            credential_store,
            identity: None,
            device_registrations: None,
            default_expires: 3600,
            min_expires: 60,
            max_expires: 86400,
            event_publisher: None,
            realm: realm_str,
            tenant_store: None,
            default_tenant_id: None,
            domain_aliases: Vec::new(),
            provisioning_url: None,
            push: None,
            node_id: None,
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

    /// Keep registration lookups for `ttl`
    /// (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1, phase 2). A zero TTL keeps
    /// nothing, which is how this registrar always behaved.
    ///
    /// This registrar's own writes invalidate at once. Another node's
    /// reach this one as registration events, which the binary hands to
    /// [`FcpRegistrar::forget_registration`]; the TTL bounds what neither
    /// covers.
    pub fn with_registration_cache(mut self, ttl: std::time::Duration) -> Self {
        self.location_store.cache = (!ttl.is_zero()).then(|| {
            Arc::new(RegistrationCache::new(
                ttl,
                RegistrationCache::DEFAULT_MAX_ENTRIES,
            ))
        });
        self
    }

    /// The registration cache, when there is one.
    pub fn registration_cache(&self) -> Option<&Arc<RegistrationCache>> {
        self.location_store.cache.as_ref()
    }

    /// Another node changed the registrations for `aor`: the next lookup
    /// here reads the store. A nil tenant — a writer that did not know
    /// the tenant — forgets the address in every tenant.
    pub fn forget_registration(&self, tenant_id: &TenantId, aor: &str) {
        if tenant_id.is_nil() {
            self.location_store.invalidate_everywhere(aor);
        } else {
            self.location_store.invalidate(tenant_id, aor);
        }
    }

    /// Attach a tenant store for tenant-aware REGISTER auth and persistence.
    pub fn with_tenant_store(mut self, tenant_store: Arc<dyn TenantStore>) -> Self {
        self.tenant_store = Some(tenant_store);
        self.refresh_authenticator();
        self
    }

    /// Check who a credential registers and calls as, and accept devices'
    /// own credentials (`docs/ENTERPRISE_CALLING.md` §3.8).
    pub fn with_identity(mut self, identity: Arc<fcp_core::DynStorage>) -> Self {
        self.device_registrations = Some(Arc::new(crate::device_sync::DeviceRegistrations::new(
            Arc::clone(&identity),
            self.event_publisher.clone(),
        )));
        self.identity = Some(identity);
        self.refresh_authenticator();
        self
    }

    /// Whether the credentials `username` may be `aor_user` in the tenant:
    /// always, without identity storage.
    async fn may_act_as(
        &self,
        tenant_id: Option<&TenantId>,
        username: &str,
        aor_user: &str,
    ) -> bool {
        if username.eq_ignore_ascii_case(aor_user) {
            return true;
        }
        let Some(identity) = &self.identity else {
            return true;
        };
        let Some(tenant_id) = tenant_id else {
            return false;
        };
        crate::hoteling::may_act_as(identity, tenant_id, username, aor_user).await
    }

    /// Set the explicit default tenant used when SIP addressing carries no tenant domain.
    /// Tell phones where the admin is in each REGISTER's 200 OK
    /// (`X-FCP-Provisioning-Url`).
    pub fn with_provisioning_url(mut self, url: Option<String>) -> Self {
        self.provisioning_url = url.filter(|u| !u.trim().is_empty());
        self
    }

    /// Wake apps with these keys (`[push]`, `docs/PUSH.md` §2.1).
    pub fn with_push(mut self, push: fcp_core::push::PushConfig) -> Self {
        self.push = Some(Arc::new(push));
        self
    }

    /// This node's id, recorded on the bindings whose flows it holds.
    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

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
                    .with_tenant_context(self.tenant_store.clone(), self.default_tenant_id)
                    .with_identity(self.identity.clone());
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

    /// Require a digest on INVITE as well as a plausible source address
    /// (`[registrar] auth_invites`, `docs/EDGE_SECURITY.md` §2.1).
    pub fn with_auth_invites(self, required: bool) -> Self {
        self.auth_invites.store(required, Ordering::Relaxed);
        self
    }

    /// Whether an INVITE must prove who it is placed by.
    ///
    /// False when the flag is off, and false when this registrar has no
    /// credential store to check a digest against — a node that cannot
    /// authenticate anyone must not refuse everyone.
    pub fn invite_auth_required(&self) -> bool {
        self.auth_invites.load(Ordering::Relaxed) && self.authenticator.is_some()
    }

    /// A 401 challenge for an INVITE, carrying a nonce this node issued.
    ///
    /// `stale` says the credentials were right and only the nonce was not,
    /// so the phone retries without prompting anyone (RFC 7616 §3.5).
    pub fn invite_challenge(&self, request: &Request, stale: bool) -> Result<Response> {
        let authenticator = self
            .authenticator
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no credential store to challenge against"))?;
        if stale {
            authenticator.challenge_stale(request)
        } else {
            authenticator.challenge(request)
        }
    }

    /// Who an INVITE is placed by, on the credentials it carries
    /// (`docs/EDGE_SECURITY.md` §2.1).
    ///
    /// The digest proves a password; it does not say whose `From` the
    /// request may wear. A user with valid credentials placing a call as
    /// their neighbour is refused outright rather than challenged, since
    /// no retry fixes it.
    pub async fn authenticate_invite(&self, request: &Request) -> InviteAuth {
        if !self.invite_auth_required() {
            return InviteAuth::NotRequired;
        }
        let Some(authenticator) = self.authenticator.as_ref() else {
            return InviteAuth::NotRequired;
        };

        match authenticator.authenticate(request).await {
            crate::auth::AuthOutcome::Failed => InviteAuth::Challenge { stale: false },
            crate::auth::AuthOutcome::Stale => InviteAuth::Challenge { stale: true },
            crate::auth::AuthOutcome::Authenticated { username } => {
                match from_user(request) {
                    // A request with no usable `From` user names nobody to
                    // be, so the credentials stand for themselves.
                    None => InviteAuth::Admitted { username },
                    Some(claimed) if claimed.eq_ignore_ascii_case(&username) => {
                        InviteAuth::Admitted { username }
                    }
                    // A device's address, its owner's, a line it is on.
                    Some(claimed)
                        if self.identity.is_some() && {
                            let tenant = self.invite_tenant(request, &claimed).await;
                            self.may_act_as(tenant.as_ref(), &username, &claimed).await
                        } =>
                    {
                        InviteAuth::Admitted { username }
                    }
                    Some(claimed) => InviteAuth::WrongUser {
                        authenticated: username,
                        claimed,
                    },
                }
            }
        }
    }

    /// The tenant of the party an INVITE's `From` names.
    async fn invite_tenant(&self, request: &Request, claimed: &str) -> Option<TenantId> {
        let aor = from_aor(request);
        resolve_tenant_for_inputs(
            self.tenant_store.as_ref(),
            self.credential_store.as_ref(),
            self.default_tenant_id,
            aor.as_deref(),
            Some(claimed),
        )
        .await
        .ok()
    }

    /// Publish a registration event (fire-and-forget).
    /// Write a REGISTER on the device it is for (`docs/PROVISIONING.md` §8).
    #[allow(clippy::too_many_arguments)]
    fn note_device(
        &self,
        tenant_id: TenantId,
        aor: &str,
        auth_username: Option<String>,
        address: Option<String>,
        source: Option<std::net::SocketAddr>,
        user_agent: Option<String>,
        registered: bool,
        push_provider: Option<String>,
    ) {
        let (Some(devices), Some(aor_user)) =
            (&self.device_registrations, extract_username_from_aor(aor))
        else {
            return;
        };
        devices.note(
            tenant_id,
            aor_user,
            auth_username,
            address,
            source.map(|s| s.ip()),
            user_agent,
            registered,
            push_provider,
        );
    }

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

    /// [`Self::handle_register_with_flow`], remembering the address the
    /// REGISTER came from with the binding.
    pub async fn handle_register_from(
        &self,
        request: &Request,
        flow_token: Option<&str>,
        source_address: Option<std::net::SocketAddr>,
    ) -> Response {
        self.handle_register_inner(request, flow_token, source_address)
            .await
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
        self.handle_register_inner(request, flow_token, None).await
    }

    async fn handle_register_inner(
        &self,
        request: &Request,
        flow_token: Option<&str>,
        source_address: Option<std::net::SocketAddr>,
    ) -> Response {
        // Extract AOR early for event emission
        let aor = self.extract_aor_from_request(request).unwrap_or_default();
        let username_hint = extract_username_from_aor(&aor);
        let tenant_id = self
            .resolve_tenant(request, Some(&aor), username_hint.as_deref())
            .await
            .ok();

        // Who the credentials are, for the device the registration is
        // written on.
        let mut auth_username: Option<String> = None;

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
                match authenticator.authenticate(request).await {
                    crate::auth::AuthOutcome::Authenticated { username } => {
                        debug!("Authentication successful");
                        auth_username = Some(username.clone());
                        // The credentials may register only what is theirs
                        // (`docs/ENTERPRISE_CALLING.md` §3.8): no retry fixes
                        // that, so it is refused rather than challenged.
                        if let Some(aor_user) = username_hint.as_deref() {
                            if !self
                                .may_act_as(tenant_id.as_ref(), &username, aor_user)
                                .await
                            {
                                warn!(
                                    aor = %aor,
                                    username = %username,
                                    "REGISTER refused: the credentials may not register this address"
                                );
                                self.publish_event(RegistrationEvent::Failed {
                                    aor: aor.clone(),
                                    tenant_id: tenant_id.unwrap_or_else(TenantId::nil),
                                    reason: "not_permitted".to_string(),
                                    timestamp: SystemTime::now(),
                                });
                                return self.create_error_response(request, 403, "Forbidden");
                            }
                        }
                    }
                    // The credentials are right and only the nonce was not
                    // usable, so the phone retries without prompting anyone.
                    outcome @ (crate::auth::AuthOutcome::Stale
                    | crate::auth::AuthOutcome::Failed) => {
                        let stale = outcome == crate::auth::AuthOutcome::Stale;
                        debug!(stale, "Authentication failed, sending 401 challenge");
                        if !stale {
                            self.publish_event(RegistrationEvent::Failed {
                                aor: aor.clone(),
                                tenant_id: tenant_id.unwrap_or_else(TenantId::nil),
                                reason: "authentication_failed".to_string(),
                                timestamp: SystemTime::now(),
                            });
                        }
                        let challenge = if stale {
                            authenticator.challenge_stale(request)
                        } else {
                            authenticator.challenge(request)
                        };
                        return match challenge {
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
                }
            } else {
                debug!("Auth enabled but no credential store available, skipping authentication");
            }
        } else {
            debug!("Handling REGISTER without authentication");
        }

        // Authentication passed or not required - handle registration
        self.process_registration(request, flow_token, source_address, auth_username)
            .await
    }

    /// The tenant of whoever a request's `From` says placed it, as digest
    /// authentication looks their credentials up.
    pub async fn caller_tenant(&self, request: &Request) -> Option<TenantId> {
        let aor = from_aor(request)?;
        let username = extract_username_from_aor(&aor);
        self.resolve_tenant(request, Some(&aor), username.as_deref())
            .await
            .ok()
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
    async fn process_registration(
        &self,
        request: &Request,
        flow_token: Option<&str>,
        source_address: Option<std::net::SocketAddr>,
        auth_username: Option<String>,
    ) -> Response {
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
                        self.note_device(
                            tenant_id,
                            &aor,
                            auth_username.clone(),
                            None,
                            source_address,
                            user_agent.clone(),
                            false,
                            None,
                        );
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
                        return self.store_error_response(request);
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
                        info!(
                            aor = %aor,
                            contact = %Self::normalize_contact_for_matching(contact_uri),
                            "Deregistered contact"
                        );
                        self.note_device(
                            tenant_id,
                            &aor,
                            auth_username.clone(),
                            None,
                            source_address,
                            user_agent.clone(),
                            false,
                            None,
                        );
                        // Emit unregistration event, naming the contact as
                        // it was stored (and announced registered): without
                        // its transient and push parameters.
                        self.publish_event(RegistrationEvent::Unregistered {
                            aor: aor.clone(),
                            contact: Self::normalize_contact_for_matching(contact_uri),
                            tenant_id,
                            reason: "explicit".to_string(),
                            timestamp: SystemTime::now(),
                        });
                        return self.create_ok_response(request, vec![]);
                    }
                    Err(e) => {
                        tracing::error!("Failed to deregister contact: {}", e);
                        return self.store_error_response(request);
                    }
                }
            }
        }

        // Handle registration (Expires > 0)
        let mut echo: Option<(String, String)> = None;
        let mut feature_caps = Vec::new();
        if let Some(contact_str) = contact_header {
            // Strip contact-params (like ;expires=0)
            let contact_uri = Self::extract_contact_uri(contact_str.as_str());
            // Its instance and reg-id, and where to wake it (`docs/PUSH.md`
            // §2.1): a push address this node cannot use is refused, so the
            // app registers without one.
            let parts = crate::push_address::parse_contact(contact_str.as_str());
            let push = match parts.push {
                crate::push_address::PushRequest::None => None,
                crate::push_address::PushRequest::Incomplete => {
                    return self.create_error_response(
                        request,
                        400,
                        "Bad Request - pn-provider needs pn-param and pn-prid",
                    );
                }
                crate::push_address::PushRequest::Address(address) => {
                    match self.push.as_deref().map(|p| p.check(&address)) {
                        Some(Ok(())) => Some(address),
                        _ => {
                            info!(
                                aor = %aor,
                                provider = %address.provider,
                                param = %address.param,
                                "REGISTER asks for a push this node cannot send"
                            );
                            return self.push_not_supported(request);
                        }
                    }
                }
            };
            // A push binding outlives its connection: its app re-registers
            // when it runs and is woken when a call needs it.
            let expires = match (&push, &self.push) {
                (Some(_), Some(config)) => self.expires_up_to(request, config.max_expires()),
                _ => expires,
            };
            let stored_contact = Self::normalize_contact_for_matching(contact_uri);
            let binding = match Self::create_binding(
                &aor,
                contact_uri,
                expires.min(86_400),
                request,
                flow_token,
            ) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("Failed to create binding: {}", e);
                    return self.create_error_response(request, 400, "Bad Request");
                }
            };
            let details = BindingDetails {
                expires: Some(std::time::Duration::from_secs(u64::from(expires))),
                instance_id: parts.instance.clone(),
                reg_id: parts.reg_id,
                push: push.clone(),
                flow_node: flow_token.and(self.node_id.clone()),
            };

            match self
                .location_store
                .upsert_in_tenant(binding, tenant_id, source_address, details)
                .await
            {
                Ok(replaced) => {
                    info!(
                        aor = %aor,
                        contact = %stored_contact,
                        expires = expires,
                        push = ?push.as_ref().map(|p| p.provider.as_str()),
                        "Registration successful"
                    );
                    self.note_device(
                        tenant_id,
                        &aor,
                        auth_username.clone(),
                        Some(contact_host_port(contact_uri)),
                        source_address,
                        user_agent.clone(),
                        true,
                        push.as_ref().map(|p| p.provider.clone()),
                    );
                    for old in replaced {
                        info!(aor = %aor, contact = %old, "Binding replaced by its instance's new one");
                        self.publish_event(RegistrationEvent::Unregistered {
                            aor: aor.clone(),
                            contact: old,
                            tenant_id,
                            reason: "replaced".to_string(),
                            timestamp: SystemTime::now(),
                        });
                    }
                    // Emit registration event
                    self.publish_event(RegistrationEvent::Registered {
                        aor: aor.clone(),
                        contact: stored_contact.clone(),
                        tenant_id,
                        expires_secs: expires,
                        user_agent: user_agent.clone(),
                        timestamp: SystemTime::now(),
                        instance_id: parts.instance,
                    });
                    if let Some(push) = &push {
                        feature_caps = crate::push_address::feature_caps(&[push.provider.as_str()]);
                    }
                    echo = Some((stored_contact, contact_uri.to_string()));
                }
                Err(e) => {
                    tracing::error!("Failed to store registration: {}", e);
                    return self.store_error_response(request);
                }
            }
        }

        // Retrieve all current bindings and return in response
        match self.location_store.lookup_in_tenant(&tenant_id, &aor).await {
            Ok(bindings) => {
                let mut response = self.create_ok_response_echoing(request, bindings, echo);
                for caps in feature_caps {
                    let _ = response
                        .headers_mut()
                        .push(SmolStr::new("Feature-Caps"), SmolStr::new(caps));
                }
                response
            }
            Err(e) => {
                tracing::error!("Failed to lookup registrations: {}", e);
                self.store_error_response(request)
            }
        }
    }

    /// 555 Push Notification Service Not Supported (RFC 8599 §5.2), naming
    /// the providers this node can push through.
    fn push_not_supported(&self, request: &Request) -> Response {
        let mut response =
            self.create_error_response(request, 555, "Push Notification Service Not Supported");
        let providers = self
            .push
            .as_deref()
            .map(|p| p.providers())
            .unwrap_or_default();
        for caps in crate::push_address::feature_caps(&providers) {
            let _ = response
                .headers_mut()
                .push(SmolStr::new("Feature-Caps"), SmolStr::new(caps));
        }
        response
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
                    // Keep transport and other stable params, skip rinstance
                    // and gr, and RFC 8599's push address, which is kept on
                    // its own (`docs/PUSH.md` §2.1) and is not where the
                    // phone is.
                    if param_name != "rinstance"
                        && param_name != "gr"
                        && !crate::push_address::is_push_param(param_name)
                    {
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
                    param_name != "rinstance"
                        && param_name != "gr"
                        && !crate::push_address::is_push_param(param_name)
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
        self.expires_up_to(request, self.max_expires)
    }

    /// The expiry a REGISTER asks for, kept between the minimum and `max`.
    fn expires_up_to(&self, request: &Request, max: u32) -> u32 {
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
                        let clamped = exp.clamp(self.min_expires, max.max(self.min_expires));
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
                let clamped = exp.clamp(self.min_expires, max.max(self.min_expires));
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
    fn create_ok_response(&self, request: &Request, bindings: Vec<Registration>) -> Response {
        self.create_ok_response_echoing(request, bindings, None)
    }

    /// A 200 OK whose Contact for the binding just made (stored as
    /// `echo.0`) is the phone's own (`echo.1`): a phone finds its binding in
    /// the answer by the URI it sent, push parameters and all (RFC 8599
    /// keeps those out of the stored contact).
    fn create_ok_response_echoing(
        &self,
        request: &Request,
        bindings: Vec<Registration>,
        echo: Option<(String, String)>,
    ) -> Response {
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
        let now = SystemTime::now();
        for binding in &bindings {
            let contact_str = match &echo {
                Some((stored, sent)) if binding.contact == *stored => sent.as_str(),
                _ => binding.contact.as_str(),
            };
            // To the nearest second: a binding just made reads as granted.
            let left = (binding
                .expires_at
                .duration_since(now)
                .unwrap_or_default()
                .as_millis()
                + 500)
                / 1000;
            // Check if contact already has angle brackets
            let contact = if contact_str.starts_with('<') {
                // Already has angle brackets, just add expires parameter
                format!("{};expires={}", contact_str, left)
            } else {
                // No angle brackets, wrap it
                format!("<{}>;expires={}", contact_str, left)
            };
            let _ = headers.push(SmolStr::new("Contact"), SmolStr::new(contact));
        }

        // Add Service-Route header for bindings with Path (RFC 3608)
        // The Service-Route informs the UA of the route set to use for subsequent requests.
        // We derive it from the stored Path headers.
        for binding in &bindings {
            if let Some(path) = binding.path.as_deref().filter(|p| !p.is_empty()) {
                // Path and Service-Route have the same format
                let _ = headers.push(SmolStr::new("Service-Route"), SmolStr::new(path));
                debug!(service_route = %path, "Added Service-Route to 200 OK");
            }
        }

        if let Some(url) = &self.provisioning_url {
            let _ = headers.push(
                SmolStr::new("X-FCP-Provisioning-Url"),
                SmolStr::new(url.as_str()),
            );
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
    /// A registration the store failed: 503 with `Retry-After`, so the
    /// phone tries again shortly, here or at another node, rather than a
    /// 500 it may not retry. The store fails like this when it is away
    /// before the node's probe has noticed, or when the primary's address
    /// leads to a standby that refuses writes (`docs/DEEP_TESTING.md` D5c).
    fn store_error_response(&self, request: &Request) -> Response {
        let mut response =
            self.create_error_response(request, 503, "Service Unavailable - store error");
        let _ = response
            .headers_mut()
            .push(SmolStr::new("Retry-After"), SmolStr::new("5"));
        response
    }

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
        self.get_registration_rows_impl(tenant_id, aor)
            .await?
            .iter()
            .map(LocationStoreAdapter::to_binding)
            .collect()
    }

    /// The live registrations for an Address-of-Record as stored.
    ///
    /// The rows behind [`FcpRegistrar::get_registrations`], which carry the
    /// address each REGISTER came from — what places a device
    /// (`docs/EDGE_SECURITY.md` §2.2).
    pub async fn get_registration_rows(&self, aor: &str) -> Result<Vec<Registration>> {
        let tenant_id = resolve_tenant_for_inputs(
            self.tenant_store.as_ref(),
            self.credential_store.as_ref(),
            self.default_tenant_id,
            Some(aor),
            None,
        )
        .await?;

        self.get_registration_rows_impl(&tenant_id, aor).await
    }

    /// An AOR's live registrations in a tenant, as stored.
    pub async fn get_registration_rows_in_tenant(
        &self,
        tenant_id: &TenantId,
        aor: &str,
    ) -> Result<Vec<Registration>> {
        self.get_registration_rows_impl(tenant_id, aor).await
    }

    /// A push the provider refused for good (`docs/PUSH.md` §2.3): the
    /// binding stays — the app may be awake on it — without its push
    /// address, and the tenant is told (`device.push_failed`).
    pub async fn forget_push(
        &self,
        row: &Registration,
        device_id: Option<String>,
        reason: &str,
    ) -> Result<()> {
        let Some(push) = row.push.clone() else {
            return Ok(());
        };
        let mut kept = row.clone();
        kept.push = None;
        self.location_store.store.put_registration(kept).await?;
        self.location_store.invalidate(&row.tenant_id, &row.aor);
        warn!(
            aor = %row.aor,
            contact = %row.contact,
            provider = %push.provider,
            token = %push.shown_prid(),
            %reason,
            "Push token gone: the binding keeps no push address"
        );
        if let Some(ref publisher) = self.event_publisher {
            let publisher = Arc::clone(publisher);
            let event = PlatformEvent::System(fcp_events::SystemEvent::DevicePushFailed {
                tenant_id: row.tenant_id,
                device_id,
                aor: row.aor.clone(),
                contact: row.contact.clone(),
                provider: push.provider,
                reason: reason.to_string(),
                timestamp: SystemTime::now(),
            });
            tokio::spawn(async move {
                if let Err(e) = publisher.publish(event).await {
                    tracing::debug!(%e, "Failed to publish device.push_failed");
                }
            });
        }
        Ok(())
    }

    async fn get_registration_rows_impl(
        &self,
        tenant_id: &TenantId,
        aor: &str,
    ) -> Result<Vec<Registration>> {
        // Normalize AOR so lookups match stored keys even if clients send
        // params or percent-encoded users.
        let normalized = match sip_core::Uri::parse(aor) {
            Ok(uri) => sip_registrar::normalize_aor(&uri)
                .unwrap_or_else(|_| aor.split(';').next().unwrap_or(aor).to_string()),
            Err(_) => aor.split(';').next().unwrap_or(aor).to_string(),
        };

        // Look up the primary AOR
        let mut all_rows = self
            .location_store
            .rows_in_tenant(
                tenant_id,
                &normalized,
                fcp_core::storage::RegistrationRead::Direct,
            )
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
                            if let Ok(mut rows) = self
                                .location_store
                                .rows_in_tenant(
                                    tenant_id,
                                    &aliased_aor,
                                    // The multiplication §2.1 names: every
                                    // alias past the first is another trip.
                                    fcp_core::storage::RegistrationRead::Alias,
                                )
                                .await
                            {
                                debug!(
                                    original_aor = %normalized,
                                    aliased_aor = %aliased_aor,
                                    found = rows.len(),
                                    "Found registrations at aliased domain"
                                );
                                all_rows.append(&mut rows);
                            }
                        }
                    }
                }
            }
        }

        Ok(all_rows)
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

            match cred_store.get_user_by_sip_name(&username, &tenant_id).await {
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

/// A contact URI's host and port (`sip:alice@192.0.2.7:5060;transport=tcp`
/// → `192.0.2.7:5060`).
fn contact_host_port(contact: &str) -> String {
    let rest = contact
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_start_matches("sips:")
        .trim_start_matches("sip:");
    let host = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
    host.split([';', '>']).next().unwrap_or(host).to_string()
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    use fcp_core::FileBackedStorage;
    use smol_str::SmolStr;

    fn registration_at(contact: &str, source: Option<&str>) -> Registration {
        Registration {
            tenant_id: TenantId::nil(),
            aor: "sip:bob@example.com".to_string(),
            contact: contact.to_string(),
            expires_at: SystemTime::now() + std::time::Duration::from_secs(3600),
            call_id: "call-1".to_string(),
            cseq: 1,
            user_agent: None,
            q: None,
            registered_at: SystemTime::now(),
            path: None,
            flow_token: None,
            source_address: source.map(str::to_string),
            instance_id: None,
            reg_id: None,
            push: None,
            flow_node: None,
        }
    }

    fn ip(s: &str) -> std::net::IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_neighbour_is_not_the_registered_device() {
        // The everyday shape of a LAN: the contact's text contains the
        // neighbour's address, so the old substring check authorised it
        // (`docs/EDGE_SECURITY.md` §2.2).
        let reg = registration_at("sip:bob@10.0.0.10:5060", None);
        assert!(reg.contact.contains("10.0.0.1"));
        assert!(!registration_is_at(&reg, ip("10.0.0.1")));
        assert!(registration_is_at(&reg, ip("10.0.0.10")));
    }

    #[test]
    fn an_address_in_a_port_or_parameter_authorises_nobody() {
        let reg = registration_at("<sip:bob@10.0.0.10:5060;transport=tcp>", None);
        // The port spells 5060; a peer at 5.0.6.0 is not the device, and
        // neither is a peer whose address appears only as a parameter.
        assert!(!registration_is_at(&reg, ip("5.0.6.0")));
        let named = registration_at("<sip:bob@10.0.0.10;fcp-note=192.168.1.5>", None);
        assert!(!registration_is_at(&named, ip("192.168.1.5")));
        assert!(registration_is_at(&named, ip("10.0.0.10")));
    }

    #[test]
    fn the_source_address_wins_over_the_contact() {
        // A device behind NAT registers a contact naming an address it
        // does not own; what places it is where the REGISTER came from.
        let reg = registration_at("sip:bob@192.168.1.50:5060", Some("203.0.113.7:1234"));
        assert!(registration_is_at(&reg, ip("203.0.113.7")));
        assert!(!registration_is_at(&reg, ip("192.168.1.50")));
    }

    #[test]
    fn a_contact_host_that_is_a_name_places_nobody() {
        let reg = registration_at("sip:bob@phone.example.com:5060", None);
        assert!(!registration_is_at(&reg, ip("10.0.0.10")));
    }

    #[test]
    fn an_ipv4_peer_mapped_into_ipv6_still_matches() {
        let reg = registration_at("sip:bob@10.0.0.10:5060", None);
        assert!(registration_is_at(&reg, ip("::ffff:10.0.0.10")));
        let mapped = registration_at("sip:bob@example.com", Some("[::ffff:10.0.0.10]:5060"));
        assert!(registration_is_at(&mapped, ip("10.0.0.10")));
    }

    #[test]
    fn ipv6_contacts_are_compared_as_addresses() {
        let reg = registration_at("<sip:bob@[2001:db8::10]:5060;transport=tcp>", None);
        assert!(registration_is_at(&reg, ip("2001:db8::10")));
        assert!(!registration_is_at(&reg, ip("2001:db8::1")));
    }

    #[test]
    fn contact_host_reads_the_forms_the_store_holds() {
        assert_eq!(contact_host("sip:bob@10.0.0.10"), Some(ip("10.0.0.10")));
        assert_eq!(
            contact_host("<sip:bob@10.0.0.10:5060>"),
            Some(ip("10.0.0.10"))
        );
        assert_eq!(
            contact_host("\"Bob\" <sips:bob@10.0.0.10;transport=tls>"),
            Some(ip("10.0.0.10"))
        );
        // A user part with its own `@`.
        assert_eq!(
            contact_host("sip:bob@work@10.0.0.10"),
            Some(ip("10.0.0.10"))
        );
        assert_eq!(
            contact_host("sip:bob@10.0.0.10?Subject=Hi"),
            Some(ip("10.0.0.10"))
        );
        assert_eq!(contact_host("sip:bob@example.com"), None);
        assert_eq!(contact_host(""), None);
    }

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
        // Not configured: no provisioning URL told.
        assert!(response.headers().get("X-FCP-Provisioning-Url").is_none());

        // Configured: every 200 OK tells the phone where the admin is.
        let told = FcpRegistrar::new(storage.clone(), None, None, false)
            .with_default_tenant_id(fcp_core::TenantId::nil())
            .with_provisioning_url(Some("https://pbx.example.com".to_string()))
            .handle_register(&request)
            .await;
        assert_eq!(told.start_line().code(), 200);
        assert_eq!(
            told.headers().get("X-FCP-Provisioning-Url"),
            Some("https://pbx.example.com")
        );

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
            deprovisioned_at: None,
            sip_username: None,
            scim_managed: false,
            scim_external_id: None,
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

    /// A REGISTER for bob from `contact`, with `expires`.
    fn bob_register(contact: &str, expires: u32, cseq: u32) -> Request {
        let mut headers = sip_core::Headers::new();
        headers.push(
            SmolStr::new("Via"),
            SmolStr::new(format!(
                "SIP/2.0/UDP 192.168.1.206:5060;branch=z9hG4bKcache{cseq}"
            )),
        );
        headers.push(
            SmolStr::new("From"),
            SmolStr::new(format!("<sip:bob@example.com>;tag=c{cseq}")),
        );
        headers.push(SmolStr::new("To"), SmolStr::new("<sip:bob@example.com>"));
        headers.push(SmolStr::new("Call-ID"), SmolStr::new("cache@example.com"));
        headers.push(
            SmolStr::new("CSeq"),
            SmolStr::new(format!("{cseq} REGISTER")),
        );
        headers.push(
            SmolStr::new("Contact"),
            SmolStr::new(format!("<{contact}>;expires={expires}")),
        );
        headers.push(SmolStr::new("Expires"), SmolStr::new(expires.to_string()));
        headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));
        Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Register,
                sip_core::SipUri::parse("sip:example.com").unwrap(),
            ),
            headers,
            bytes::Bytes::new(),
        )
        .unwrap()
    }

    /// A registrar keeping lookups for 30 seconds, on a store of its own.
    async fn cached_registrar(name: &str) -> (FcpRegistrar, Arc<FileBackedStorage>) {
        let dir = std::env::temp_dir().join(format!("fcp-test-registration-cache-{name}"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let storage = Arc::new(FileBackedStorage::new(dir).await.unwrap());
        let registrar = FcpRegistrar::new(storage.clone(), None, None, false)
            .with_default_tenant_id(TenantId::nil())
            .with_registration_cache(std::time::Duration::from_secs(30));
        (registrar, storage)
    }

    async fn bob_contacts(registrar: &FcpRegistrar) -> Vec<String> {
        registrar
            .get_registration_rows("sip:bob@example.com")
            .await
            .unwrap()
            .into_iter()
            .map(|r| {
                r.contact
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_string()
            })
            .collect()
    }

    /// Another node's REGISTER: straight into the store, past this
    /// registrar and its cache.
    async fn registered_elsewhere(storage: &FileBackedStorage, contact: &str) {
        let mut row = registration_at(contact, None);
        row.aor = "sip:bob@example.com".to_string();
        storage.put_registration(row).await.unwrap();
    }

    #[tokio::test]
    async fn a_lookup_is_answered_from_the_cache() {
        // `docs/MULTI_REGION_FOLLOWUPS.md` §2.1, phase 2.
        let (registrar, storage) = cached_registrar("answered").await;
        let reg = registrar
            .handle_register(&bob_register("sip:bob@10.0.0.1:5060", 3600, 1))
            .await;
        assert_eq!(reg.start_line().code(), 200);

        assert_eq!(
            bob_contacts(&registrar).await,
            vec!["sip:bob@10.0.0.1:5060"]
        );
        let misses = registrar.registration_cache().unwrap().stats().misses;

        // A write this node did not make is not seen until the cache is
        // told — which is what shows the second lookup never reached the
        // store.
        registered_elsewhere(&storage, "sip:bob@10.0.0.9:5060").await;
        assert_eq!(
            bob_contacts(&registrar).await,
            vec!["sip:bob@10.0.0.1:5060"]
        );
        let stats = registrar.registration_cache().unwrap().stats();
        assert_eq!(stats.misses, misses, "the second lookup read the store");
        assert!(stats.hits >= 1);
    }

    #[tokio::test]
    async fn this_nodes_own_register_is_seen_by_the_next_lookup() {
        // A phone that moves re-REGISTERs from its new address; a call
        // placed here straight after must reach it there.
        let (registrar, _storage) = cached_registrar("own-register").await;
        registrar
            .handle_register(&bob_register("sip:bob@10.0.0.1:5060", 3600, 1))
            .await;
        assert_eq!(
            bob_contacts(&registrar).await,
            vec!["sip:bob@10.0.0.1:5060"]
        );

        registrar
            .handle_register(&bob_register("sip:bob@10.0.0.1:5060", 0, 2))
            .await;
        assert!(
            bob_contacts(&registrar).await.is_empty(),
            "a deregistration here is seen at once"
        );

        registrar
            .handle_register(&bob_register("sip:bob@10.0.0.2:5062", 3600, 3))
            .await;
        assert_eq!(
            bob_contacts(&registrar).await,
            vec!["sip:bob@10.0.0.2:5062"],
            "a registration here is seen at once, even after an empty answer was kept"
        );
    }

    #[tokio::test]
    async fn another_nodes_register_is_seen_once_its_event_arrives() {
        // Nobody is registered, and the empty answer is kept. Another node
        // takes bob's REGISTER; its event is what tells this one.
        let (registrar, storage) = cached_registrar("event").await;
        assert!(bob_contacts(&registrar).await.is_empty());
        registered_elsewhere(&storage, "sip:bob@10.0.0.9:5060").await;
        assert!(
            bob_contacts(&registrar).await.is_empty(),
            "the empty answer is kept until something says otherwise"
        );

        registrar.forget_registration(&TenantId::nil(), "sip:bob@example.com");
        assert_eq!(
            bob_contacts(&registrar).await,
            vec!["sip:bob@10.0.0.9:5060"]
        );
    }

    #[tokio::test]
    async fn alias_lookups_are_kept_as_well() {
        // Most of what an alias costs is lookups that find nobody; a cache
        // that skipped empty answers would save little of it.
        let (registrar, _storage) = cached_registrar("aliases").await;
        let registrar = registrar.with_domain_aliases(vec![
            "example.com".to_string(),
            "alias-one.test".to_string(),
            "alias-two.test".to_string(),
        ]);
        registrar
            .handle_register(&bob_register("sip:bob@10.0.0.1:5060", 3600, 1))
            .await;

        assert_eq!(bob_contacts(&registrar).await.len(), 1);
        let first = registrar.registration_cache().unwrap().stats();
        assert_eq!(bob_contacts(&registrar).await.len(), 1);
        let second = registrar.registration_cache().unwrap().stats();
        assert_eq!(
            second.misses, first.misses,
            "the second lookup read the store at some alias"
        );
        assert_eq!(
            second.hits - first.hits,
            3,
            "one hit for the address and one for each other alias"
        );
    }

    #[tokio::test]
    async fn a_binding_that_lapses_while_kept_is_not_used() {
        let (registrar, storage) = cached_registrar("lapse").await;
        let mut row = registration_at("sip:bob@10.0.0.1:5060", None);
        row.expires_at = SystemTime::now() + std::time::Duration::from_millis(50);
        storage.put_registration(row).await.unwrap();

        assert_eq!(bob_contacts(&registrar).await.len(), 1);
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert!(
            bob_contacts(&registrar).await.is_empty(),
            "expiry is judged at each lookup, not when the answer was kept"
        );
    }

    #[tokio::test]
    async fn without_a_ttl_every_lookup_reads_the_store() {
        let dir = std::env::temp_dir().join("fcp-test-registration-cache-none");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let storage = Arc::new(FileBackedStorage::new(dir).await.unwrap());
        let registrar = FcpRegistrar::new(storage.clone(), None, None, false)
            .with_default_tenant_id(TenantId::nil())
            .with_registration_cache(std::time::Duration::ZERO);
        assert!(registrar.registration_cache().is_none());

        assert!(bob_contacts(&registrar).await.is_empty());
        registered_elsewhere(&storage, "sip:bob@10.0.0.9:5060").await;
        assert_eq!(bob_contacts(&registrar).await.len(), 1);
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

    // ── INVITE authentication (`docs/EDGE_SECURITY.md` §2.1) ────────────

    /// A user whose HA1 the credential store holds.
    async fn store_with_alice(dir: &str) -> Arc<FileBackedStorage> {
        let temp_dir = std::env::temp_dir().join(dir);
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        let storage = Arc::new(FileBackedStorage::new(temp_dir).await.unwrap());
        let user = fcp_core::User {
            id: fcp_core::UserId::new(),
            username: "alice".to_string(),
            password: "secret".to_string(),
            digest_password: None,
            sip_ha1_hash: Some(fcp_core::storage::password::compute_ha1(
                "alice",
                "fcp.local",
                "secret",
            )),
            sip_provisioning_password: None,
            display_name: Some("Alice".to_string()),
            tenant_id: fcp_core::TenantId::nil(),
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
            deprovisioned_at: None,
            sip_username: None,
            scim_managed: false,
            scim_external_id: None,
            account_status: fcp_core::AccountStatus::Active,
            failed_login_attempts: 0,
            account_locked_until: None,
        };
        storage.put_user(user).await.unwrap();
        storage
    }

    fn invite_registrar(storage: Arc<FileBackedStorage>, auth_invites: bool) -> FcpRegistrar {
        FcpRegistrar::new(
            storage.clone(),
            Some(storage as _),
            Some("fcp.local".to_string()),
            true,
        )
        .with_default_tenant_id(fcp_core::TenantId::nil())
        .with_auth_invites(auth_invites)
    }

    /// An INVITE from `from`, carrying a digest over `nonce` when one is
    /// given.
    fn invite(from: &str, credentials: Option<(&str, &str)>) -> Request {
        let uri = "sip:bob@fcp.local";
        let authorization = credentials.map(|(user, nonce)| {
            let ha1 = fcp_core::storage::password::compute_ha1(user, "fcp.local", "secret");
            let ha2 = format!("{:x}", md5::compute(format!("INVITE:{uri}")));
            let response = format!(
                "{:x}",
                md5::compute(format!("{ha1}:{nonce}:00000001:abc123:auth:{ha2}"))
            );
            format!(
                "Digest username=\"{user}\", realm=\"fcp.local\", nonce=\"{nonce}\", \
                 uri=\"{uri}\", response=\"{response}\", qop=auth, nc=00000001, cnonce=\"abc123\""
            )
        });
        let auth_line = authorization
            .map(|a| format!("Authorization: {a}\r\n"))
            .unwrap_or_default();
        let raw = format!(
            "INVITE {uri} SIP/2.0\r\n\
             Via: SIP/2.0/UDP 10.0.0.9:5060;branch=z9hG4bK1\r\n\
             From: <sip:{from}@fcp.local>;tag=1\r\n\
             To: <{uri}>\r\n\
             Call-ID: call-invite-1\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:{from}@10.0.0.9>\r\n\
             {auth_line}\
             Content-Length: 0\r\n\r\n"
        );
        sip_parse::parse_request(&bytes::Bytes::from(raw)).expect("test INVITE parses")
    }

    /// The nonce from a challenge this registrar issued.
    fn issued_nonce(registrar: &FcpRegistrar, request: &Request) -> String {
        let challenge = registrar
            .invite_challenge(request, false)
            .expect("challenge");
        let header = challenge
            .headers()
            .get_smol("WWW-Authenticate")
            .expect("challenge carries WWW-Authenticate");
        let (_, rest) = header.as_str().split_once("nonce=\"").expect("a nonce");
        rest.split('"').next().expect("a closed nonce").to_string()
    }

    #[tokio::test]
    async fn an_invite_without_credentials_is_challenged() {
        // Today's defect: a call is authorised by the address it came
        // from and never asked who placed it.
        let storage = store_with_alice("fcp-test-invite-auth-none").await;
        let registrar = invite_registrar(storage, true);
        assert_eq!(
            registrar.authenticate_invite(&invite("alice", None)).await,
            InviteAuth::Challenge { stale: false }
        );
    }

    #[tokio::test]
    async fn an_invite_with_the_callers_own_credentials_is_admitted() {
        let storage = store_with_alice("fcp-test-invite-auth-ok").await;
        let registrar = invite_registrar(storage, true);
        let nonce = issued_nonce(&registrar, &invite("alice", None));
        assert_eq!(
            registrar
                .authenticate_invite(&invite("alice", Some(("alice", &nonce))))
                .await,
            InviteAuth::Admitted {
                username: "alice".to_string()
            }
        );
    }

    #[tokio::test]
    async fn a_nonce_this_node_never_issued_is_challenged_as_stale() {
        let storage = store_with_alice("fcp-test-invite-auth-stale").await;
        let registrar = invite_registrar(storage, true);
        assert_eq!(
            registrar
                .authenticate_invite(&invite("alice", Some(("alice", "invented"))))
                .await,
            InviteAuth::Challenge { stale: true }
        );
    }

    #[tokio::test]
    async fn credentials_do_not_let_a_caller_wear_another_users_from() {
        // Alice's password proves Alice. It does not let her place calls
        // as Bob, at Bob's cost and under Bob's caller id — and no retry
        // fixes that, so it is refused rather than challenged.
        let storage = store_with_alice("fcp-test-invite-auth-wrong-user").await;
        let registrar = invite_registrar(storage, true);
        let nonce = issued_nonce(&registrar, &invite("bob", None));
        assert_eq!(
            registrar
                .authenticate_invite(&invite("bob", Some(("alice", &nonce))))
                .await,
            InviteAuth::WrongUser {
                authenticated: "alice".to_string(),
                claimed: "bob".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn nothing_is_required_while_the_flag_is_off() {
        // One release of source-based admission, warned about at startup
        // (`docs/EDGE_SECURITY.md` §4, Q1).
        let storage = store_with_alice("fcp-test-invite-auth-off").await;
        let registrar = invite_registrar(storage, false);
        assert_eq!(
            registrar.authenticate_invite(&invite("alice", None)).await,
            InviteAuth::NotRequired
        );
    }

    #[tokio::test]
    async fn a_node_that_cannot_check_a_digest_does_not_refuse_everyone() {
        let temp_dir = std::env::temp_dir().join("fcp-test-invite-auth-no-store");
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        let storage = Arc::new(FileBackedStorage::new(temp_dir).await.unwrap());
        let registrar = FcpRegistrar::new(storage, None, Some("fcp.local".to_string()), true)
            .with_default_tenant_id(fcp_core::TenantId::nil())
            .with_auth_invites(true);
        assert!(!registrar.invite_auth_required());
        assert_eq!(
            registrar.authenticate_invite(&invite("alice", None)).await,
            InviteAuth::NotRequired
        );
    }

    #[test]
    fn the_from_user_is_who_the_request_says_placed_it() {
        assert_eq!(from_user(&invite("alice", None)).as_deref(), Some("alice"));
    }

    // ===== Push (`docs/PUSH.md` §2.1) =====

    const APP: &str = "sip:app-3f9c@10.0.0.7:52311;transport=tls";
    const INSTANCE: &str = "<urn:uuid:00000000-0000-0000-0000-0000000000a1>";
    const TOKEN: &str = "8f2c4d6e8a0b2c4d6e8a91";

    /// A REGISTER whose Contact header is `contact` as written.
    fn bob_register_contact(contact: &str, cseq: u32) -> Request {
        let mut request = bob_register("sip:placeholder@invalid", 3600, cseq);
        let mut headers = sip_core::Headers::new();
        for header in request.headers().iter() {
            if header.name_eq("Contact") {
                let _ = headers.push(SmolStr::new("Contact"), SmolStr::new(contact));
            } else if !header.name_eq("Expires") {
                let _ = headers.push(header.name_smol().clone(), header.value_smol().clone());
            }
        }
        *request.headers_mut() = headers;
        request
    }

    fn app_contact(push: &str, extra: &str) -> String {
        format!("<{APP}{push}>;+sip.instance=\"{INSTANCE}\";reg-id=1{extra}")
    }

    fn apns_push() -> String {
        format!(";pn-provider=apns;pn-param=ABCDE12345.com.thevoiceguy.anvil.voip;pn-prid={TOKEN}")
    }

    fn push_config() -> fcp_core::push::PushConfig {
        toml::from_str(
            r#"
            [[apns]]
            team_id = "ABCDE12345"
            key_id = "XYZ987"
            key = "k"
            topics = ["com.thevoiceguy.anvil.voip"]
            "#,
        )
        .unwrap()
    }

    async fn push_registrar(name: &str, push: Option<fcp_core::push::PushConfig>) -> FcpRegistrar {
        let dir = std::env::temp_dir().join(format!("fcp-test-push-{name}"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let storage = Arc::new(FileBackedStorage::new(dir).await.unwrap());
        let registrar = FcpRegistrar::new(storage, None, None, false)
            .with_default_tenant_id(TenantId::nil())
            .with_node_id("node-a");
        match push {
            Some(push) => registrar.with_push(push),
            None => registrar,
        }
    }

    fn headers_named(response: &Response, name: &str) -> Vec<String> {
        response
            .headers()
            .get_all(name)
            .map(str::to_string)
            .collect()
    }

    #[tokio::test]
    async fn an_app_registers_where_to_wake_it_for_a_week() {
        let registrar = push_registrar("week", Some(push_config())).await;
        let response = registrar
            .handle_register_with_flow(
                &bob_register_contact(&app_contact(&apns_push(), ";expires=604800"), 1),
                Some("flow-1"),
            )
            .await;
        assert_eq!(response.start_line().code(), 200);
        assert_eq!(
            headers_named(&response, "Feature-Caps"),
            ["*;+sip.pns=\"apns\""]
        );
        // The app finds its binding by the URI it sent, push address and all.
        let contacts = headers_named(&response, "Contact");
        assert_eq!(contacts.len(), 1);
        assert!(
            contacts[0].contains(&format!("pn-prid={TOKEN}")),
            "{contacts:?}"
        );
        assert!(contacts[0].ends_with(";expires=604800"), "{contacts:?}");

        let rows = registrar
            .get_registration_rows("sip:bob@example.com")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        // Where the phone is, without where to push it.
        assert_eq!(row.contact, format!("<{APP}>"));
        assert_eq!(row.instance_id.as_deref(), Some(INSTANCE));
        assert_eq!(row.reg_id, Some(1));
        assert_eq!(row.flow_node.as_deref(), Some("node-a"));
        let push = row.push.as_ref().expect("a push address");
        assert_eq!(push.provider, "apns");
        assert_eq!(push.param, "ABCDE12345.com.thevoiceguy.anvil.voip");
        assert_eq!(push.prid, TOKEN);
        let left = row
            .expires_at
            .duration_since(SystemTime::now())
            .unwrap()
            .as_secs();
        assert!(left > 86_400, "a push binding outlives a day: {left}");
    }

    #[tokio::test]
    async fn an_ordinary_phone_keeps_a_day_at_most_and_no_feature_caps() {
        let registrar = push_registrar("ordinary", Some(push_config())).await;
        let response = registrar
            .handle_register(&bob_register_contact(
                &app_contact("", ";expires=604800"),
                1,
            ))
            .await;
        assert_eq!(response.start_line().code(), 200);
        assert!(headers_named(&response, "Feature-Caps").is_empty());
        assert!(headers_named(&response, "Contact")[0].ends_with(";expires=86400"));
        let rows = registrar
            .get_registration_rows("sip:bob@example.com")
            .await
            .unwrap();
        assert_eq!(rows[0].push, None);
        assert_eq!(rows[0].instance_id.as_deref(), Some(INSTANCE));
        // No flow, so no node holding one.
        assert_eq!(rows[0].flow_node, None);
    }

    #[tokio::test]
    async fn a_push_this_node_cannot_send_is_555_naming_what_it_can() {
        let registrar = push_registrar("unsupported", Some(push_config())).await;
        let fcm = format!(";pn-provider=fcm;pn-param=another-project;pn-prid={TOKEN}");
        let response = registrar
            .handle_register(&bob_register_contact(&app_contact(&fcm, ""), 1))
            .await;
        assert_eq!(response.start_line().code(), 555);
        assert_eq!(
            headers_named(&response, "Feature-Caps"),
            ["*;+sip.pns=\"apns\""]
        );
        assert!(registrar
            .get_registration_rows("sip:bob@example.com")
            .await
            .unwrap()
            .is_empty());

        // Without [push] every push address is refused, naming none.
        let none = push_registrar("unconfigured", None).await;
        let response = none
            .handle_register(&bob_register_contact(&app_contact(&apns_push(), ""), 1))
            .await;
        assert_eq!(response.start_line().code(), 555);
        assert!(headers_named(&response, "Feature-Caps").is_empty());
    }

    #[tokio::test]
    async fn a_provider_without_its_token_is_a_bad_request() {
        let registrar = push_registrar("incomplete", Some(push_config())).await;
        let response = registrar
            .handle_register(&bob_register_contact(
                &app_contact(";pn-provider=apns", ""),
                1,
            ))
            .await;
        assert_eq!(response.start_line().code(), 400);
    }

    #[tokio::test]
    async fn a_phone_that_moves_replaces_its_binding() {
        let registrar = push_registrar("moves", Some(push_config())).await;
        let first = registrar
            .handle_register(&bob_register_contact(&app_contact(&apns_push(), ""), 1))
            .await;
        assert_eq!(first.start_line().code(), 200);
        // Woken on another network: the same instance at a new address.
        let moved = format!(
            "<sip:app-3f9c@198.51.100.4:40001;transport=tls{}>;+sip.instance=\"{INSTANCE}\";reg-id=1",
            apns_push()
        );
        let second = registrar
            .handle_register(&bob_register_contact(&moved, 2))
            .await;
        assert_eq!(second.start_line().code(), 200);
        assert_eq!(
            bob_contacts(&registrar).await,
            ["sip:app-3f9c@198.51.100.4:40001;transport=tls"]
        );
        // Another phone of Bob's, with an instance of its own, stays.
        let other = "<sip:bob@192.0.2.20:5060>;+sip.instance=\"<urn:uuid:desk>\"";
        registrar
            .handle_register(&bob_register_contact(other, 3))
            .await;
        assert_eq!(bob_contacts(&registrar).await.len(), 2);
    }

    #[tokio::test]
    async fn an_app_unregisters_with_its_push_address_in_the_contact() {
        let registrar = push_registrar("unregister", Some(push_config())).await;
        registrar
            .handle_register(&bob_register_contact(&app_contact(&apns_push(), ""), 1))
            .await;
        let response = registrar
            .handle_register(&bob_register_contact(
                &app_contact(&apns_push(), ";expires=0"),
                2,
            ))
            .await;
        assert_eq!(response.start_line().code(), 200);
        assert!(bob_contacts(&registrar).await.is_empty());
    }

    /// Keeps what the registrar announces.
    #[derive(Default)]
    struct Announced(std::sync::Mutex<Vec<PlatformEvent>>);

    #[async_trait]
    impl EventPublisher for Announced {
        async fn publish(&self, event: PlatformEvent) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_phone_is_announced_gone_by_the_contact_it_was_announced_by() {
        // Registered and unregistered name the same contact, without the
        // parameters that are not where the phone is (`rinstance`, `pn-*`),
        // so whoever keeps registrations by contact sees it go.
        let dir = std::env::temp_dir().join("fcp-test-push-announced");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let storage = Arc::new(FileBackedStorage::new(dir).await.unwrap());
        let announced = Arc::new(Announced::default());
        let registrar = FcpRegistrar::with_events(storage, None, None, announced.clone(), false)
            .with_default_tenant_id(TenantId::nil())
            .with_push(push_config());
        let contact = app_contact(&format!(";rinstance=abc{}", apns_push()), "");
        registrar
            .handle_register(&bob_register_contact(&contact, 1))
            .await;
        registrar
            .handle_register(&bob_register_contact(
                &app_contact(&format!(";rinstance=abc{}", apns_push()), ";expires=0"),
                2,
            ))
            .await;
        // Published on tasks of their own.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let events = announced.0.lock().unwrap().clone();
        let registered = events.iter().find_map(|e| match e {
            PlatformEvent::Registration(RegistrationEvent::Registered {
                contact,
                instance_id,
                ..
            }) => Some((contact.clone(), instance_id.clone())),
            _ => None,
        });
        let unregistered = events.iter().find_map(|e| match e {
            PlatformEvent::Registration(RegistrationEvent::Unregistered { contact, .. }) => {
                Some(contact.clone())
            }
            _ => None,
        });
        let (registered, instance) = registered.expect("registered");
        assert_eq!(registered, format!("<{APP}>"));
        assert_eq!(instance.as_deref(), Some(INSTANCE));
        assert_eq!(unregistered.as_deref(), Some(registered.as_str()));
    }
}
