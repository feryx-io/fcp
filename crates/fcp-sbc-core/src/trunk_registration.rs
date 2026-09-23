//! Outbound trunk registration.
//!
//! A trunk with `register: true` holds a REGISTER binding at its carrier:
//! most carriers will not take a call from an address they have no binding
//! for. This module keeps those bindings for every trunk the node routes
//! by — its own `[[trunks]]` and every tenant's — and for each one:
//!
//! - **registers the trunk as itself**, when it has credentials: `From`
//!   and `To` name the carrier account and `Contact` names this node, so
//!   the carrier's calls for that account arrive here and the trunk they
//!   arrive on says whose they are (`docs/TENANT_TRUNKS.md` §7). A trunk
//!   with no credentials has no account to be, and registers under this
//!   node's own address of record as it always did.
//! - **holds it on one node.** Several nodes load the same trunk, and
//!   several bindings for one account is not what a carrier expects: the
//!   owner ([`TrunkOwnership`]) registers and refreshes, and a node that
//!   stops being the owner gives its binding up. Without a cluster every
//!   node owns what it loaded, which is one node.
//!
//! [`TrunkRegistrar`] keeps the state, [`run_trunk_registration`] the loop.

use crate::trunk_health::TrunkHealthMonitor;
use crate::trunk_set::TrunkSet;
use dashmap::DashMap;
use fcp_core::transport::TransportType;
use fcp_core::{TenantId, Trunk};
use fcp_events::{EventPublisher, PlatformEvent, TrunkEvent};
use fcp_sip::uac::SipUserAgentClient;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, info, warn};

/// Registration state for a trunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationState {
    /// Not yet registered, de-registered, or held by another node.
    Unregistered,
    /// REGISTER request in flight.
    Pending,
    /// Successfully registered with the remote peer.
    Registered,
    /// Last registration attempt failed.
    Failed,
}

impl std::fmt::Display for RegistrationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unregistered => write!(f, "unregistered"),
            Self::Pending => write!(f, "pending"),
            Self::Registered => write!(f, "registered"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Which node holds a registering trunk's binding.
///
/// A cluster answers from what every node has published about itself
/// (`fcp_cluster::ClusterManager::registers_trunk`); a node on its own
/// answers [`AloneOnThisNode`].
pub trait TrunkOwnership: Send + Sync {
    /// Say which registering trunks this node has loaded, so its peers can
    /// work out who owns each of them.
    fn advertise(&self, trunk_ids: Vec<String>);

    /// Whether this node is the one that registers `trunk_id`.
    fn registers(&self, trunk_id: &str) -> bool;
}

/// A node with no cluster: everything it loaded is its own to register,
/// which is what a single node did before trunks were shared.
pub struct AloneOnThisNode;

impl TrunkOwnership for AloneOnThisNode {
    fn advertise(&self, _trunk_ids: Vec<String>) {}

    fn registers(&self, _trunk_id: &str) -> bool {
        true
    }
}

/// A node in a cluster: it says what it has loaded in its heartbeat, and
/// the lowest live node id that loaded a trunk registers it.
///
/// A node claims a trunk only once its own claim to it has had time to
/// reach its peers. Ownership is decided from what every node has said
/// about itself, so a node that has just loaded a trunk — at startup, or
/// because a tenant created one — has said nothing about it yet, and
/// neither has it necessarily read what a peer said a moment ago. Acting
/// on that silence puts a second binding at the carrier beside the one
/// another node is already holding. A trunk is therefore claimable a
/// couple of heartbeats after this node first advertised it, which costs
/// seconds and is the difference between one binding and two.
pub struct ClusterOwnership {
    cluster: Arc<fcp_cluster::ClusterManager>,
    /// When each trunk this node loaded becomes claimable.
    claimable: DashMap<String, Instant>,
    settle: Duration,
}

impl ClusterOwnership {
    pub fn new(cluster: Arc<fcp_cluster::ClusterManager>) -> Self {
        // Two beats: one to publish what this node has loaded, one to read
        // what its peers have.
        let settle = Duration::from_secs(cluster.heartbeat_interval_secs() * 2 + 1);
        Self {
            cluster,
            claimable: DashMap::new(),
            settle,
        }
    }
}

impl TrunkOwnership for ClusterOwnership {
    fn advertise(&self, trunk_ids: Vec<String>) {
        let now = Instant::now();
        for id in &trunk_ids {
            self.claimable
                .entry(id.clone())
                .or_insert_with(|| now + self.settle);
        }
        self.claimable.retain(|id, _| trunk_ids.contains(id));
        if self.cluster.set_registering_trunks(trunk_ids) {
            // A claim its peers cannot see is a claim they will override:
            // out with it now rather than on the next beat.
            let cluster = Arc::clone(&self.cluster);
            tokio::spawn(async move { cluster.publish_now().await });
        }
    }

    fn registers(&self, trunk_id: &str) -> bool {
        let claimable = self.claimable.get(trunk_id).map(|at| *at);
        match claimable {
            Some(at) if Instant::now() >= at => self.cluster.registers_trunk(trunk_id),
            // Advertised a moment ago, or not advertised at all.
            _ => false,
        }
    }
}

/// What this node is doing about one trunk's binding.
struct TrunkRegEntry {
    name: String,
    tenant_id: Option<TenantId>,
    reg_state: RegistrationState,
    expires_at: Option<Instant>,
    configured_expiry: u32,
    last_attempt: Option<Instant>,
    last_error: Option<String>,
    registrar_uri: String,
    /// The carrier account this trunk binds, when it has credentials.
    account: Option<String>,
    credentials: Option<(String, String)>,
}

/// Tracks outbound registration state for the trunks that need it.
///
/// Keyed by trunk id ([`Trunk::key`]): two tenants may both have a
/// "carrier-a", and they are two registrations.
pub struct TrunkRegistrar {
    state: DashMap<String, TrunkRegEntry>,
}

impl Default for TrunkRegistrar {
    fn default() -> Self {
        Self::new()
    }
}

impl TrunkRegistrar {
    /// An empty registrar; [`Self::track`] fills it from the trunk set.
    pub fn new() -> Self {
        Self {
            state: DashMap::new(),
        }
    }

    /// Take the registering trunks of `trunks`, forgetting any this node no
    /// longer routes by. Returns their ids, which is what the node
    /// advertises to its peers.
    ///
    /// A trunk already tracked keeps its state — a refresh in progress is
    /// not restarted because the set was re-read — but takes the current
    /// account, credentials and expiry, so an edit reaches the next
    /// refresh.
    pub fn track(&self, trunks: &[Arc<Trunk>]) -> Vec<String> {
        let mut ids = Vec::new();
        for trunk in trunks {
            if !trunk.register || !trunk.enabled {
                continue;
            }
            let Some(uri) = build_registrar_uri(trunk) else {
                warn!(
                    trunk = %trunk.name,
                    "Trunk has register=true but no hosts configured, skipping"
                );
                continue;
            };
            let id = trunk.key();
            ids.push(id.clone());
            let account = account_uri(trunk);
            let credentials = trunk
                .credentials
                .as_ref()
                .map(|c| (c.username.clone(), c.password.clone()));
            let expiry = trunk.register_expiry.unwrap_or(3600);
            match self.state.get_mut(&id) {
                Some(mut held) => {
                    held.name = trunk.name.clone();
                    held.tenant_id = trunk.tenant_id;
                    held.configured_expiry = expiry;
                    held.registrar_uri = uri;
                    held.account = account;
                    held.credentials = credentials;
                }
                None => {
                    self.state.insert(
                        id,
                        TrunkRegEntry {
                            name: trunk.name.clone(),
                            tenant_id: trunk.tenant_id,
                            reg_state: RegistrationState::Unregistered,
                            expires_at: None,
                            configured_expiry: expiry,
                            last_attempt: None,
                            last_error: None,
                            registrar_uri: uri,
                            account,
                            credentials,
                        },
                    );
                }
            }
        }
        self.state.retain(|id, _| ids.contains(id));
        ids
    }

    /// The registration state of a trunk, by id.
    pub fn registration_state(&self, trunk: &str) -> Option<RegistrationState> {
        self.state.get(trunk).map(|e| e.reg_state)
    }

    /// Whether the trunk needs a REGISTER sent now: never registered, the
    /// last attempt failed long enough ago to try again, or the binding is
    /// within a refresh window of expiring.
    pub fn needs_registration(&self, trunk: &str) -> bool {
        let Some(entry) = self.state.get(trunk) else {
            return false;
        };
        match entry.reg_state {
            RegistrationState::Unregistered => true,
            RegistrationState::Pending => false,
            RegistrationState::Failed => entry
                .last_attempt
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(30)),
            RegistrationState::Registered => entry
                .expires_at
                .is_none_or(|at| at.saturating_duration_since(Instant::now()).as_secs() < 60),
        }
    }

    /// Whether this node holds a binding it should now give up.
    fn holds_binding(&self, trunk: &str) -> bool {
        matches!(
            self.registration_state(trunk),
            Some(RegistrationState::Registered) | Some(RegistrationState::Pending)
        )
    }

    /// Record a successful registration.
    pub fn record_registered(&self, trunk: &str, expires_secs: u32) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.reg_state = RegistrationState::Registered;
            entry.expires_at = Some(Instant::now() + Duration::from_secs(expires_secs as u64));
            entry.last_attempt = Some(Instant::now());
            entry.last_error = None;
        }
    }

    /// Record a failed registration.
    pub fn record_failed(&self, trunk: &str, reason: String) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.reg_state = RegistrationState::Failed;
            entry.last_attempt = Some(Instant::now());
            entry.last_error = Some(reason);
        }
    }

    /// Mark a registration as in flight.
    pub fn mark_pending(&self, trunk: &str) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.reg_state = RegistrationState::Pending;
        }
    }

    /// Mark a trunk as no longer registered here.
    pub fn mark_unregistered(&self, trunk: &str) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.reg_state = RegistrationState::Unregistered;
            entry.expires_at = None;
        }
    }

    /// The registrar URI for a trunk.
    pub fn registrar_uri(&self, trunk: &str) -> Option<String> {
        self.state.get(trunk).map(|e| e.registrar_uri.clone())
    }

    /// The carrier account a trunk binds, when it has one.
    pub fn account(&self, trunk: &str) -> Option<String> {
        self.state.get(trunk).and_then(|e| e.account.clone())
    }

    /// The trunk's own credentials, when it has them.
    fn credentials(&self, trunk: &str) -> Option<(String, String)> {
        self.state.get(trunk).and_then(|e| e.credentials.clone())
    }

    /// The configured expiry for a trunk.
    pub fn configured_expiry(&self, trunk: &str) -> Option<u32> {
        self.state.get(trunk).map(|e| e.configured_expiry)
    }

    /// The last error recorded for a trunk.
    pub fn last_error(&self, trunk: &str) -> Option<String> {
        self.state.get(trunk).and_then(|e| e.last_error.clone())
    }

    /// The name of a trunk, for logs and the admin API.
    pub fn name_of(&self, trunk: &str) -> Option<String> {
        self.state.get(trunk).map(|e| e.name.clone())
    }

    /// The tenant whose trunk this is; `None` for the platform's own.
    pub fn tenant_of(&self, trunk: &str) -> Option<TenantId> {
        self.state.get(trunk).and_then(|e| e.tenant_id)
    }

    /// The ids of every trunk being tracked.
    pub fn trunk_ids(&self) -> Vec<String> {
        self.state.iter().map(|e| e.key().clone()).collect()
    }

    /// Whether any trunk needs registering at all.
    pub fn has_registerable_trunks(&self) -> bool {
        !self.state.is_empty()
    }
}

/// The address of record a trunk binds at its carrier: its account, at the
/// domain the carrier knows it by.
///
/// The realm a carrier challenges with names the domain it wants in the
/// account, so it is preferred; then the trunk's own `from_domain`, which
/// is the domain its calls go out as; then the host the REGISTER is sent
/// to. A trunk with no credentials has no account.
fn account_uri(trunk: &Trunk) -> Option<String> {
    let credentials = trunk.credentials.as_ref()?;
    let domain = credentials
        .realm
        .clone()
        .or_else(|| trunk.from_domain.clone())
        .or_else(|| trunk.hosts.first().cloned())?;
    Some(format!("sip:{}@{}", credentials.username, domain))
}

/// Build a registrar URI from a trunk's configuration.
///
/// Uses `sips:` for TLS transports and `sip:` otherwise. Only includes
/// the port if it differs from the transport's default.
/// The registrar a trunk registers at: its first host, with the port it
/// names or the trunk's, and — when the port is the transport's default,
/// so RFC 3263 is asked — the transport named, so the SRV service asked
/// for is the trunk's own rather than TCP-then-UDP
/// (`docs/DEPLOYMENT_GAPS.md` §1.8).
fn build_registrar_uri(trunk: &Trunk) -> Option<String> {
    let host = fcp_core::trunk::TrunkHost::parse(trunk.hosts.first()?).ok()?;
    let default_port = trunk.transport.protocol.default_port();
    let port = host.port_or(&trunk.transport);
    let uri_host = host.uri_host();
    if trunk.transport.protocol == TransportType::Tls {
        return Some(if port == default_port {
            format!("sips:{uri_host}")
        } else {
            format!("sips:{uri_host}:{port}")
        });
    }
    let transport = trunk.transport.protocol.as_str();
    Some(if port == default_port {
        format!("sip:{uri_host};transport={transport}")
    } else {
        format!("sip:{uri_host}:{port};transport={transport}")
    })
}

/// What the registration loop needs beside the trunks themselves.
pub struct TrunkRegistrationContext {
    pub registrar: Arc<TrunkRegistrar>,
    pub trunks: Arc<TrunkSet>,
    pub health: Arc<TrunkHealthMonitor>,
    pub uac: Arc<SipUserAgentClient>,
    pub ownership: Arc<dyn TrunkOwnership>,
    pub events: Option<Arc<dyn EventPublisher>>,
    pub node_id: String,
}

/// Background task that keeps the REGISTER bindings of every registering
/// trunk this node routes by.
///
/// Ticks every 5 seconds. On each tick it re-reads the trunk set (a trunk
/// created or edited while the node runs is picked up here), tells its
/// peers what it has loaded, and then for each trunk either keeps the
/// binding — this node owns it — or gives up one it is holding, because
/// another node owns it now.
///
/// Spawn with `tokio::spawn`. Runs until shutdown is signalled, then
/// de-registers what it holds.
pub async fn run_trunk_registration(
    ctx: TrunkRegistrationContext,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    info!(node_id = %ctx.node_id, "Starting trunk registration task");

    let mut ticker = tokio::time::interval(Duration::from_secs(5));

    loop {
        ticker.tick().await;

        if *shutdown.borrow_and_update() {
            info!("Shutdown signalled, de-registering trunks");
            deregister_all(&ctx).await;
            return;
        }

        // What this node has loaded, and what its peers should know of it.
        let tracked = ctx.registrar.track(&ctx.trunks.all());
        ctx.ownership.advertise(tracked.clone());

        for id in tracked {
            if !ctx.ownership.registers(&id) {
                // Another node holds this one now. Give ours up rather
                // than leave the carrier with two bindings for one
                // account, and stop refreshing.
                if ctx.registrar.holds_binding(&id) {
                    info!(
                        trunk = %ctx.registrar.name_of(&id).unwrap_or_default(),
                        "Another node registers this trunk now, releasing the binding"
                    );
                    deregister(&ctx, &id).await;
                    ctx.registrar.mark_unregistered(&id);
                }
                continue;
            }
            if !ctx.registrar.needs_registration(&id) {
                continue;
            }
            register_one(&ctx, &id).await;
        }
    }
}

/// Send one trunk's REGISTER and record what came back.
async fn register_one(ctx: &TrunkRegistrationContext, id: &str) {
    let Some(uri) = ctx.registrar.registrar_uri(id) else {
        return;
    };
    let name = ctx.registrar.name_of(id).unwrap_or_else(|| id.to_string());
    let expires = ctx.registrar.configured_expiry(id).unwrap_or(3600);
    let account = ctx.registrar.account(id);

    ctx.registrar.mark_pending(id);
    debug!(trunk = %name, uri = %uri, expires, account = ?account, "Sending REGISTER");

    let sent = tokio::time::timeout(
        Duration::from_secs(10),
        send_register(ctx, id, &uri, expires),
    )
    .await;

    match sent {
        Ok(Ok(response)) => {
            let code = response.start_line().code();
            if (200..300).contains(&code) {
                info!(trunk = %name, code, account = ?account, "Trunk registered successfully");
                ctx.registrar.record_registered(id, expires);
                ctx.health.record_success(id);
                publish(
                    ctx,
                    TrunkEvent::Registered {
                        trunk_id: id.to_string(),
                        trunk_name: name,
                        tenant_id: ctx.registrar.tenant_of(id),
                        account,
                        expires_secs: expires,
                        node_id: ctx.node_id.clone(),
                        timestamp: SystemTime::now(),
                    },
                )
                .await;
            } else {
                let reason = format!("{} {}", code, response.start_line().reason());
                warn!(trunk = %name, code, "Trunk registration rejected");
                fail(ctx, id, &name, account, reason).await;
            }
        }
        Ok(Err(e)) => {
            warn!(trunk = %name, error = %e, "Trunk registration failed");
            fail(ctx, id, &name, account, e.to_string()).await;
        }
        Err(_) => {
            warn!(trunk = %name, "Trunk registration timed out (10s)");
            fail(ctx, id, &name, account, "timeout".to_string()).await;
        }
    }
}

/// The REGISTER itself: as the trunk's own account when it has one, and
/// as this node otherwise.
async fn send_register(
    ctx: &TrunkRegistrationContext,
    id: &str,
    uri: &str,
    expires: u32,
) -> anyhow::Result<sip_core::Response> {
    match (ctx.registrar.account(id), ctx.registrar.credentials(id)) {
        (Some(account), credentials) => {
            ctx.uac
                .register_as(&account, uri, expires, credentials)
                .await
        }
        (None, _) => ctx.uac.register(uri, expires).await,
    }
}

async fn fail(
    ctx: &TrunkRegistrationContext,
    id: &str,
    name: &str,
    account: Option<String>,
    reason: String,
) {
    ctx.registrar.record_failed(id, reason.clone());
    ctx.health.record_failure(id);
    publish(
        ctx,
        TrunkEvent::RegistrationFailed {
            trunk_id: id.to_string(),
            trunk_name: name.to_string(),
            tenant_id: ctx.registrar.tenant_of(id),
            account,
            reason,
            node_id: ctx.node_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

async fn publish(ctx: &TrunkRegistrationContext, event: TrunkEvent) {
    if let Some(events) = &ctx.events {
        if let Err(e) = events.publish(PlatformEvent::Trunk(event)).await {
            debug!(error = %e, "Could not publish trunk event");
        }
    }
}

/// Best-effort de-registration of one trunk: REGISTER with expires=0.
async fn deregister(ctx: &TrunkRegistrationContext, id: &str) {
    let Some(uri) = ctx.registrar.registrar_uri(id) else {
        return;
    };
    let name = ctx.registrar.name_of(id).unwrap_or_else(|| id.to_string());
    debug!(trunk = %name, "Sending de-registration (expires=0)");

    match tokio::time::timeout(Duration::from_secs(3), send_register(ctx, id, &uri, 0)).await {
        Ok(Ok(response)) => {
            info!(trunk = %name, code = response.start_line().code(), "Trunk de-registered");
        }
        Ok(Err(e)) => warn!(trunk = %name, error = %e, "Trunk de-registration failed"),
        Err(_) => warn!(trunk = %name, "Trunk de-registration timed out (3s)"),
    }
}

/// Best-effort de-registration of everything this node holds.
async fn deregister_all(ctx: &TrunkRegistrationContext) {
    for id in ctx.registrar.trunk_ids() {
        if ctx.registrar.registration_state(&id) != Some(RegistrationState::Registered) {
            continue;
        }
        deregister(ctx, &id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::trunk::TrunkCredentials;
    use fcp_core::{Trunk, TrunkId};

    fn trunk(name: &str, register: bool) -> Trunk {
        let mut t = Trunk::new(name.to_string(), TenantId::new());
        t.tenant_id = None;
        t.id = TrunkId::platform(name);
        t.hosts = vec!["carrier.example".to_string()];
        t.register = register;
        t
    }

    fn with_credentials(mut t: Trunk, user: &str, realm: Option<&str>) -> Trunk {
        t.credentials = Some(TrunkCredentials {
            username: user.to_string(),
            password: "secret".to_string(),
            realm: realm.map(str::to_owned),
        });
        t
    }

    /// Only the trunks that ask to register are tracked, and a trunk the
    /// node no longer routes by is forgotten.
    #[test]
    fn it_tracks_the_registering_trunks_and_forgets_the_rest() {
        let registrar = TrunkRegistrar::new();
        let registering = Arc::new(trunk("carrier-a", true));
        let plain = Arc::new(trunk("carrier-b", false));

        let ids = registrar.track(&[Arc::clone(&registering), Arc::clone(&plain)]);
        assert_eq!(ids, vec![registering.key()]);
        assert!(registrar.has_registerable_trunks());

        let ids = registrar.track(&[plain]);
        assert!(ids.is_empty());
        assert!(!registrar.has_registerable_trunks());
    }

    /// A disabled trunk holds no binding.
    #[test]
    fn a_disabled_trunk_is_not_registered() {
        let registrar = TrunkRegistrar::new();
        let mut t = trunk("carrier-a", true);
        t.enabled = false;
        assert!(registrar.track(&[Arc::new(t)]).is_empty());
    }

    /// The account is the trunk's own: its credentials at the realm the
    /// carrier challenges with, then its from-domain, then the host.
    #[test]
    fn the_account_is_the_trunks_own() {
        let realm = with_credentials(trunk("a", true), "acct-4471", Some("sip.carrier.net"));
        assert_eq!(
            account_uri(&realm).as_deref(),
            Some("sip:acct-4471@sip.carrier.net")
        );

        let mut domain = with_credentials(trunk("b", true), "acct-9", None);
        domain.from_domain = Some("tenant.example".to_string());
        assert_eq!(
            account_uri(&domain).as_deref(),
            Some("sip:acct-9@tenant.example")
        );

        let host = with_credentials(trunk("c", true), "acct-1", None);
        assert_eq!(
            account_uri(&host).as_deref(),
            Some("sip:acct-1@carrier.example")
        );
    }

    /// A trunk with no credentials has no account to be, and registers as
    /// this node did before.
    #[test]
    fn a_trunk_without_credentials_has_no_account() {
        assert_eq!(account_uri(&trunk("a", true)), None);
    }

    /// Two tenants' "carrier-a" are two registrations, not one.
    #[test]
    fn two_tenants_trunks_of_one_name_are_two_registrations() {
        let registrar = TrunkRegistrar::new();
        let mut one = trunk("carrier-a", true);
        one.id = TrunkId::new();
        one.tenant_id = Some(TenantId::new());
        let mut two = trunk("carrier-a", true);
        two.id = TrunkId::new();
        two.tenant_id = Some(TenantId::new());

        let ids = registrar.track(&[Arc::new(one), Arc::new(two)]);
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert_ne!(ids[0], ids[1]);
    }

    /// Re-reading the set keeps a binding in place but takes the edit: a
    /// trunk whose expiry changed does not lose its registration for it.
    #[test]
    fn re_reading_the_set_keeps_the_binding_and_takes_the_edit() {
        let registrar = TrunkRegistrar::new();
        let mut t = trunk("carrier-a", true);
        t.register_expiry = Some(3600);
        let id = t.key();
        registrar.track(&[Arc::new(t.clone())]);
        registrar.record_registered(&id, 3600);

        t.register_expiry = Some(600);
        registrar.track(&[Arc::new(t)]);

        assert_eq!(
            registrar.registration_state(&id),
            Some(RegistrationState::Registered)
        );
        assert_eq!(registrar.configured_expiry(&id), Some(600));
    }

    /// A fresh binding is not refreshed; one near its expiry is.
    #[test]
    fn a_binding_is_refreshed_near_its_expiry() {
        let registrar = TrunkRegistrar::new();
        let t = Arc::new(trunk("carrier-a", true));
        let id = t.key();
        registrar.track(&[t]);

        assert!(registrar.needs_registration(&id), "never registered");
        registrar.record_registered(&id, 3600);
        assert!(!registrar.needs_registration(&id), "fresh");
        registrar.record_registered(&id, 30);
        assert!(registrar.needs_registration(&id), "about to expire");
    }

    /// A node on its own registers everything it loaded.
    #[test]
    fn alone_a_node_owns_what_it_loaded() {
        let alone = AloneOnThisNode;
        assert!(alone.registers("trunk-anything"));
    }
}

#[cfg(test)]
mod registrar_uri_tests {
    use super::*;

    #[test]
    fn the_registrar_uri_names_the_transport_when_rfc_3263_is_asked() {
        let mut trunk = Trunk {
            name: "carrier".to_string(),
            ..Default::default()
        };
        trunk.hosts = vec!["sip.carrier.example".to_string()];
        trunk.transport.protocol = TransportType::Udp;
        trunk.transport.port = None;
        assert_eq!(
            build_registrar_uri(&trunk).as_deref(),
            Some("sip:sip.carrier.example;transport=udp")
        );
        trunk.transport.port = Some(5080);
        assert_eq!(
            build_registrar_uri(&trunk).as_deref(),
            Some("sip:sip.carrier.example:5080;transport=udp")
        );
        trunk.hosts = vec!["sip.carrier.example:5090".to_string()];
        assert_eq!(
            build_registrar_uri(&trunk).as_deref(),
            Some("sip:sip.carrier.example:5090;transport=udp")
        );
        trunk.hosts = vec!["sip.carrier.example".to_string()];
        trunk.transport.protocol = TransportType::Tls;
        trunk.transport.port = None;
        assert_eq!(
            build_registrar_uri(&trunk).as_deref(),
            Some("sips:sip.carrier.example")
        );
    }
}
