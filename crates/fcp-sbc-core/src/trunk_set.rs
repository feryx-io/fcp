//! What a node routes by: its own trunks, and every tenant's
//! (`docs/TENANT_TRUNKS.md` §4).
//!
//! A node reads its `[[trunks]]` once, at startup — those are the
//! platform's, shared by every tenant that has none of its own — and reads
//! the tenants' trunks from the store, refreshing them when they change.
//! Selection asks for one tenant's trunks and gets that tenant's first,
//! then the platform's, unless the tenant has said its calls may not leave
//! on a shared carrier.
//!
//! Inbound, the set answers the other question (§7): which trunks a call
//! from this address may have arrived on, and so whose call it is.

use crate::lcr::{LcrRouteEntry, LcrRouteTable};
use fcp_core::storage::{TenantStore, TrunkStore};
use fcp_core::transport::TransportType;
use fcp_core::{SiteId, TenantId, Trunk};
use ipnetwork::IpNetwork;
use rust_decimal::prelude::ToPrimitive;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tracing::{debug, warn};

/// How long one trunk host's name may take to resolve during a refresh.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);

/// What each trunk host given by name resolves to, per transport: the
/// dial targets (host and port) in the order RFC 3263 gave them.
type SrvTargets = HashMap<(String, TransportType), Vec<(String, u16)>>;

/// The URI RFC 3263 is asked about for a trunk host on `transport`:
/// `sips:` for TLS, else `sip:` with the transport named, so the SRV
/// service asked for is the trunk's own rather than TCP-then-UDP.
pub fn srv_question(name: &str, transport: TransportType) -> String {
    match transport {
        TransportType::Tls => format!("sips:{name}"),
        other => format!("sip:{name};transport={}", other.as_str()),
    }
}

/// The address part of a trunk host — `10.0.0.1:5060`, `[2001:db8::1]:5060`
/// and `sip.carrier.example` all name a host — read the one way every
/// trunk host is (`fcp_core::trunk::TrunkHost`).
fn host_part(host: &str) -> String {
    fcp_core::trunk::TrunkHost::parse(host)
        .map(|h| h.host)
        .unwrap_or_else(|_| host.trim().to_string())
}

/// One tenant's trunks, and whether the platform's may carry its calls.
#[derive(Debug, Default, Clone)]
struct TenantTrunks {
    trunks: Vec<Arc<Trunk>>,
    /// False when this tenant's calls must never leave on a shared carrier.
    platform_fallback: bool,
}

/// The trunks a node can route by.
pub struct TrunkSet {
    /// This node's own `[[trunks]]`, in the order it read them.
    platform: Vec<Arc<Trunk>>,
    /// Every tenant's trunks, as of the last refresh.
    tenants: RwLock<HashMap<TenantId, TenantTrunks>>,
    /// Where each trunk that takes inbound calls takes them from, most
    /// specific range first.
    inbound: RwLock<Vec<(IpNetwork, Arc<Trunk>)>>,
    /// The addresses of trunk hosts given by name, as of the last refresh.
    resolved: RwLock<HashMap<String, Vec<IpAddr>>>,
    /// What RFC 3263 says a host given by name with no port resolves to,
    /// per transport, as of the last refresh: the SRV targets in priority
    /// order (host and port), or the A/AAAA addresses on the default port
    /// when the name has no SRV records (`docs/DEPLOYMENT_GAPS.md` §1.8).
    /// Each is one route a call tries in turn.
    srv: RwLock<SrvTargets>,
    /// Who answers the RFC 3263 question; the system's resolver, or a
    /// test's.
    resolver: Option<Arc<dyn sip_dns::Resolver>>,
    /// Each tenant's least-cost routes, keyed by trunk id
    /// (`docs/TENANT_TRUNKS.md` §12, phase 6c).
    lcr: RwLock<HashMap<TenantId, Arc<LcrRouteTable>>>,
    /// What each tenant may carry at once, as of the last refresh
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). Absent means no limit.
    call_limits: RwLock<HashMap<TenantId, u32>>,
    /// Which site this deployment is, in a hybrid deployment
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.6): the site whose end of a link
    /// trunk this node holds. `None` at the cloud, and everywhere else.
    site: RwLock<Option<SiteId>>,
    /// Keep, of the tenants' trunks, only the cloud's ends of sites' links:
    /// the cloud SBC, which routes by the platform's trunks alone (§13.8)
    /// and takes the sites' registrations.
    link_peers_only: bool,
}

impl TrunkSet {
    /// A node with only its own trunks — every deployment that has never
    /// created a trunk through the API, and the SBC, which is the
    /// platform's edge (§13.8).
    pub fn platform_only(trunks: Vec<Trunk>) -> Self {
        let resolver: Option<Arc<dyn sip_dns::Resolver>> = match sip_dns::SipResolver::from_system()
        {
            Ok(r) => Some(Arc::new(r.with_query_timeout(RESOLVE_TIMEOUT))),
            Err(e) => {
                warn!(error = %e, "No system DNS resolver; trunk hosts given by name are dialled on their default port without SRV");
                None
            }
        };
        let set = Self {
            platform: trunks.into_iter().map(Arc::new).collect(),
            tenants: RwLock::new(HashMap::new()),
            inbound: RwLock::new(Vec::new()),
            resolved: RwLock::new(HashMap::new()),
            srv: RwLock::new(HashMap::new()),
            resolver,
            lcr: RwLock::new(HashMap::new()),
            call_limits: RwLock::new(HashMap::new()),
            site: RwLock::new(None),
            link_peers_only: false,
        };
        set.rebuild_inbound();
        set
    }

    /// Of the tenants' trunks, keep only the cloud's ends of sites' links
    /// (`docs/HYBRID_CONTROL_PLANE.md` §3.6): what the cloud SBC needs of
    /// them, and nothing that would make it route a tenant's calls.
    pub fn link_peers_only(mut self) -> Self {
        self.link_peers_only = true;
        self
    }

    /// Which site this deployment is: the site of the link trunk it holds
    /// the site's end of, or `None` at the cloud and in a deployment that
    /// is neither.
    pub fn site(&self) -> Option<SiteId> {
        self.site.read().ok().and_then(|s| *s)
    }

    /// The cloud's ends of sites' links this node knows: the peers whose
    /// registrations it takes.
    pub fn link_peers(&self) -> Vec<Arc<Trunk>> {
        self.all()
            .into_iter()
            .filter(|t| t.is_link_peer())
            .collect()
    }

    /// Answer the RFC 3263 question with `resolver` instead of the
    /// system's (tests).
    pub fn with_resolver(mut self, resolver: Arc<dyn sip_dns::Resolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// The dial targets RFC 3263 gave `name` on `transport` at the last
    /// refresh, in order; `None` when the name was not resolved (no
    /// resolver, not yet refreshed, or every lookup failed), when a call
    /// dials the name itself on the default port.
    pub fn srv_targets(&self, name: &str, transport: TransportType) -> Option<Vec<(String, u16)>> {
        self.srv
            .read()
            .ok()?
            .get(&(name.to_string(), transport))
            .filter(|t| !t.is_empty())
            .cloned()
    }

    /// What a refresh would have resolved `name` to (tests).
    #[cfg(test)]
    pub(crate) fn set_srv_for_test(
        &self,
        name: &str,
        transport: TransportType,
        targets: Vec<(String, u16)>,
    ) {
        if let Ok(mut srv) = self.srv.write() {
            srv.insert((name.to_string(), transport), targets);
        }
    }

    /// The trunks a call from `source` may have arrived on: enabled, not
    /// outbound-only, and taking calls from that address — by their
    /// `inbound_sources`, or by their hosts when they name none. Most
    /// specific range first.
    pub fn inbound_from(&self, source: IpAddr) -> Vec<Arc<Trunk>> {
        let Ok(inbound) = self.inbound.read() else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        inbound
            .iter()
            .filter(|(net, _)| net.contains(source))
            .filter(|(_, trunk)| seen.insert(trunk.trunk_id()))
            .map(|(_, trunk)| Arc::clone(trunk))
            .collect()
    }

    /// Rebuild the inbound index from the trunks and the names resolved so
    /// far.
    fn rebuild_inbound(&self) {
        let resolved = self.resolved.read().map(|r| r.clone()).unwrap_or_default();
        let srv = self.srv.read().map(|r| r.clone()).unwrap_or_default();
        let mut index = Vec::new();
        for trunk in self.all() {
            // A site's link peer takes calls from wherever the site
            // registered from, which the handler knows; its hosts are this
            // cloud's own address.
            if !trunk.takes_inbound() || trunk.is_link_peer() {
                continue;
            }
            let networks = trunk.inbound_networks();
            if !networks.is_empty() {
                index.extend(networks.into_iter().map(|n| (n, Arc::clone(&trunk))));
                continue;
            }
            for entry in &trunk.hosts {
                let Ok(host) = fcp_core::trunk::TrunkHost::parse(entry) else {
                    continue;
                };
                // A name with no port takes calls from wherever RFC 3263
                // sends them: its SRV targets, resolved to addresses.
                let names: Vec<String> = match (host.host.parse::<IpAddr>(), host.port) {
                    (Ok(_), _) => vec![host.host.clone()],
                    (Err(_), None) => srv
                        .get(&(host.host.clone(), trunk.transport.protocol))
                        .filter(|t| !t.is_empty())
                        .map(|t| {
                            t.iter()
                                .map(|(h, _)| h.trim_matches(['[', ']']).to_string())
                                .collect()
                        })
                        .unwrap_or_else(|| vec![host.host.clone()]),
                    (Err(_), Some(_)) => vec![host.host.clone()],
                };
                for name in names {
                    let addrs = match name.parse::<IpAddr>() {
                        Ok(ip) => vec![ip],
                        Err(_) => resolved.get(&name).cloned().unwrap_or_default(),
                    };
                    index.extend(
                        addrs
                            .into_iter()
                            .map(|ip| (IpNetwork::from(ip), Arc::clone(&trunk))),
                    );
                }
            }
        }
        index.sort_by_key(|(net, _)| std::cmp::Reverse(net.prefix()));
        match self.inbound.write() {
            Ok(mut inbound) => *inbound = index,
            Err(e) => warn!(error = %e, "Trunk set lock poisoned; inbound index not rebuilt"),
        }
    }

    /// Resolve the trunk hosts given by name and take calls from what they
    /// resolve to. Done off the call path: at startup, and on each refresh.
    pub async fn resolve_names(&self) {
        self.resolve_srv().await;
        self.resolve_hosts().await;
        self.rebuild_inbound();
    }

    /// Ask RFC 3263 about every trunk host given by name with no port, on
    /// its trunk's transport: NAPTR, then SRV, then A/AAAA on the default
    /// port. What a name last resolved to is kept when it fails now. A
    /// host with a port, or an address, is dialled as written (RFC 3263
    /// §4.1) and is not asked about.
    async fn resolve_srv(&self) {
        let Some(resolver) = &self.resolver else {
            return;
        };
        let mut questions: HashSet<(String, TransportType)> = HashSet::new();
        for trunk in self.all() {
            for entry in &trunk.hosts {
                if let Ok(host) = fcp_core::trunk::TrunkHost::parse(entry) {
                    if host.port.is_none() && host.host.parse::<IpAddr>().is_err() {
                        questions.insert((host.host, trunk.transport.protocol));
                    }
                }
            }
        }
        for (name, transport) in questions {
            let uri = srv_question(&name, transport);
            let Ok(uri) = sip_core::SipUri::parse(&uri) else {
                debug!(host = %name, "Trunk host is not a SIP host; not resolved");
                continue;
            };
            match tokio::time::timeout(RESOLVE_TIMEOUT, resolver.resolve(&uri)).await {
                Ok(Ok(targets)) => {
                    let targets: Vec<(String, u16)> = targets
                        .iter()
                        .map(|t| (t.host().to_string(), t.port()))
                        .collect();
                    debug!(host = %name, transport = %transport, targets = ?targets, "Trunk host resolved");
                    if let Ok(mut srv) = self.srv.write() {
                        srv.insert((name, transport), targets);
                    }
                }
                Ok(Err(e)) => {
                    debug!(host = %name, error = %e, "Trunk host did not resolve (RFC 3263)")
                }
                Err(_) => debug!(host = %name, "Trunk host took too long to resolve (RFC 3263)"),
            }
        }
    }

    /// Resolve every trunk host given by name, and every SRV target given
    /// by name, keeping what a name last resolved to when it fails now.
    async fn resolve_hosts(&self) {
        let srv = self.srv.read().map(|r| r.clone()).unwrap_or_default();
        let mut names: HashSet<String> = self
            .all()
            .iter()
            .filter(|t| t.takes_inbound() && t.inbound_sources.is_empty())
            .flat_map(|t| t.hosts.iter().map(|h| host_part(h)))
            .filter(|h| !h.is_empty() && h.parse::<IpAddr>().is_err())
            .collect();
        names.extend(
            srv.values()
                .flatten()
                .map(|(h, _)| h.trim_matches(['[', ']']).to_string())
                .filter(|h| h.parse::<IpAddr>().is_err()),
        );
        for name in names {
            let lookup = tokio::net::lookup_host((name.clone(), 5060));
            match tokio::time::timeout(RESOLVE_TIMEOUT, lookup).await {
                Ok(Ok(addrs)) => {
                    let addrs: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
                    if let Ok(mut resolved) = self.resolved.write() {
                        resolved.insert(name, addrs);
                    }
                }
                Ok(Err(e)) => debug!(host = %name, error = %e, "Trunk host did not resolve"),
                Err(_) => debug!(host = %name, "Trunk host took too long to resolve"),
            }
        }
    }

    /// The trunks that may carry `tenant`'s calls, best claim first: the
    /// tenant's own, then the platform's. Order within each is left to the
    /// router, which knows about priority, weight, health and cost.
    pub fn for_tenant(&self, tenant: Option<TenantId>) -> Vec<Arc<Trunk>> {
        let Some(tenant) = tenant else {
            // A call with no tenant is the platform's own.
            return self.platform.clone();
        };
        let (mut trunks, fallback) = match self.tenants.read() {
            Ok(tenants) => match tenants.get(&tenant) {
                Some(entry) => (entry.trunks.clone(), entry.platform_fallback),
                None => (Vec::new(), true),
            },
            Err(e) => {
                warn!(error = %e, "Trunk set lock poisoned; routing on this node's own trunks");
                (Vec::new(), true)
            }
        };
        // A trunk runs where its site says, and a link peer is reached by
        // its registration, not routed to (`docs/HYBRID_CONTROL_PLANE.md`
        // §3.6).
        let site = self.site();
        trunks.retain(|t| t.routes_from(site));
        if fallback {
            trunks.extend(self.platform.iter().cloned());
        }
        trunks
    }

    /// Every trunk this node routes by, for health probing and status.
    pub fn all(&self) -> Vec<Arc<Trunk>> {
        let mut all = self.platform.clone();
        if let Ok(tenants) = self.tenants.read() {
            for entry in tenants.values() {
                all.extend(entry.trunks.iter().cloned());
            }
        }
        all
    }

    /// The trunk with `key` ([`Trunk::key`]), if this node knows it.
    pub fn find(&self, key: &str) -> Option<Arc<Trunk>> {
        self.all().into_iter().find(|t| t.key() == key)
    }

    /// This node's own trunks.
    pub fn platform(&self) -> &[Arc<Trunk>] {
        &self.platform
    }

    /// `tenant`'s least-cost routes, when it has any in force. Each entry's
    /// `trunk_name` is a trunk id: a tenant's routes name its trunks by id.
    pub fn lcr_for(&self, tenant: TenantId) -> Option<Arc<LcrRouteTable>> {
        self.lcr.read().ok()?.get(&tenant).cloned()
    }

    /// Replace what is known of the tenants' least-cost routes. A route
    /// that is disabled, or names a trunk that is not its own tenant's, is
    /// not in force. Returns how many are.
    pub fn set_tenant_lcr(&self, routes: Vec<fcp_core::LcrRoute>) -> usize {
        let owned: HashSet<(TenantId, String)> = match self.tenants.read() {
            Ok(tenants) => tenants
                .iter()
                .flat_map(|(tenant, entry)| {
                    entry
                        .trunks
                        .iter()
                        .map(move |t| (*tenant, t.trunk_id().to_string()))
                })
                .collect(),
            Err(_) => HashSet::new(),
        };
        let mut by_tenant: HashMap<TenantId, Vec<LcrRouteEntry>> = HashMap::new();
        for route in routes.into_iter().filter(|r| r.enabled) {
            if !owned.contains(&(route.tenant_id, route.trunk_id.to_string())) {
                debug!(
                    route = %route.id,
                    trunk = %route.trunk_id,
                    "A least-cost route names a trunk that is not its tenant's; not in force"
                );
                continue;
            }
            by_tenant
                .entry(route.tenant_id)
                .or_default()
                .push(LcrRouteEntry {
                    prefix: route.prefix,
                    trunk_name: route.trunk_id.to_string(),
                    cost_per_minute: route.cost_per_minute.to_f64().unwrap_or(f64::MAX),
                    priority: route.priority,
                    enabled: true,
                });
        }
        let count = by_tenant.values().map(Vec::len).sum();
        let tables = by_tenant
            .into_iter()
            .map(|(tenant, entries)| (tenant, Arc::new(LcrRouteTable::new(&entries))))
            .collect();
        match self.lcr.write() {
            Ok(mut lcr) => *lcr = tables,
            Err(e) => warn!(error = %e, "Trunk set lock poisoned; least-cost routes not refreshed"),
        }
        count
    }

    /// How many tenants have trunks here.
    pub fn tenant_count(&self) -> usize {
        self.tenants.read().map(|t| t.len()).unwrap_or(0)
    }

    /// Replace what is known about the tenants' trunks. Returns how many
    /// trunks were read.
    pub fn set_tenant_trunks(
        &self,
        trunks: Vec<Trunk>,
        fallback: &HashMap<TenantId, bool>,
    ) -> usize {
        let trunks: Vec<Trunk> = if self.link_peers_only {
            trunks.into_iter().filter(|t| t.is_link_peer()).collect()
        } else {
            trunks
        };
        let count = trunks.len();
        // The site this deployment is: what the link wrote as the site's
        // end of its link says so.
        let site = trunks
            .iter()
            .find(|t| t.is_link_to_cloud())
            .and_then(|t| t.site_id);
        if let Ok(mut held) = self.site.write() {
            *held = site;
        }
        let mut by_tenant: HashMap<TenantId, TenantTrunks> = HashMap::new();
        for trunk in trunks {
            let Some(tenant) = trunk.tenant_id else {
                continue;
            };
            let entry = by_tenant.entry(tenant).or_insert_with(|| TenantTrunks {
                trunks: Vec::new(),
                platform_fallback: fallback.get(&tenant).copied().unwrap_or(true),
            });
            entry.trunks.push(Arc::new(trunk));
        }
        // A tenant with no trunks of its own still has a say about the
        // platform's.
        for (tenant, allowed) in fallback {
            by_tenant
                .entry(*tenant)
                .or_insert_with(|| TenantTrunks {
                    trunks: Vec::new(),
                    platform_fallback: *allowed,
                })
                .platform_fallback = *allowed;
        }
        match self.tenants.write() {
            Ok(mut tenants) => *tenants = by_tenant,
            Err(e) => warn!(error = %e, "Trunk set lock poisoned; tenant trunks not refreshed"),
        }
        self.rebuild_inbound();
        count
    }

    /// Replace what is known about each tenant's concurrent call limit.
    /// A tenant that sets none is left out, and so has no limit.
    pub fn set_tenant_call_limits(&self, limits: HashMap<TenantId, u32>) {
        match self.call_limits.write() {
            Ok(mut slot) => *slot = limits,
            Err(e) => {
                warn!(error = %e, "Trunk set lock poisoned; tenant call limits not refreshed")
            }
        }
    }

    /// Read every tenant's trunks, and what each tenant says about the
    /// platform's.
    pub async fn refresh<S>(&self, store: &S) -> anyhow::Result<usize>
    where
        S: TrunkStore + TenantStore + ?Sized,
    {
        let trunks = store.list_all_trunks().await?;
        let tenants = store.list_tenants().await.unwrap_or_default();
        let fallback: HashMap<TenantId, bool> = tenants
            .iter()
            .map(|t| (t.id, t.platform_trunk_fallback))
            .collect();
        // What each tenant may carry at once comes from the same read
        // (`docs/CAPACITY_AND_SURVIVABILITY.md` §5).
        self.set_tenant_call_limits(
            tenants
                .iter()
                .filter_map(|t| t.max_concurrent_calls.map(|limit| (t.id, limit)))
                .collect(),
        );
        let count = self.set_tenant_trunks(trunks, &fallback);
        // After the trunks: a route is in force only for a trunk of its own
        // tenant's that this node now knows.
        // A table that cannot be read leaves the last one in force: the
        // trunks are current either way.
        match store.list_all_lcr_routes().await {
            Ok(routes) => {
                self.set_tenant_lcr(routes);
            }
            Err(e) => {
                warn!(error = %e, "Could not read the tenants' least-cost routes; keeping the last")
            }
        }
        self.resolve_names().await;
        debug!(
            trunks = count,
            tenants = self.tenant_count(),
            "Tenant trunks refreshed"
        );
        Ok(count)
    }
}

impl fcp_sip::trunk_ingress::InboundTrunks for TrunkSet {
    fn link_peer_by_account(&self, username: &str) -> Option<Arc<Trunk>> {
        self.link_peers().into_iter().find(|t| {
            t.enabled
                && t.credentials
                    .as_ref()
                    .is_some_and(|c| c.username.eq_ignore_ascii_case(username))
        })
    }

    fn link_peer_for_site(&self, site: SiteId) -> Option<Arc<Trunk>> {
        self.link_peers()
            .into_iter()
            .find(|t| t.enabled && t.site_id == Some(site))
    }

    fn site(&self) -> Option<SiteId> {
        TrunkSet::site(self)
    }

    fn inbound_from(&self, source: IpAddr) -> Vec<Arc<Trunk>> {
        TrunkSet::inbound_from(self, source)
    }
}

/// A tenant's concurrent call limit, for the admission controller
/// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). The trunk set already reads
/// every tenant on its refresh tick, so nothing else has to.
impl crate::admission::TenantCallLimits for TrunkSet {
    fn max_concurrent_calls(&self, tenant: &TenantId) -> Option<u32> {
        self.call_limits
            .read()
            .ok()
            .and_then(|limits| limits.get(tenant).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trunk(name: &str, tenant: Option<TenantId>) -> Trunk {
        let mut trunk = match tenant {
            Some(tenant) => Trunk::new(name.to_string(), tenant),
            None => Trunk {
                name: name.to_string(),
                ..Default::default()
            },
        };
        trunk.hosts = vec!["10.0.0.9".to_string()];
        trunk
    }

    #[test]
    fn a_tenants_trunks_come_first_then_the_platforms() {
        let (a, b) = (TenantId::new(), TenantId::new());
        let set = TrunkSet::platform_only(vec![trunk("shared", None)]);
        set.set_tenant_trunks(
            vec![trunk("acme-carrier", Some(a)), trunk("other", Some(b))],
            &HashMap::new(),
        );

        let names: Vec<String> = set
            .for_tenant(Some(a))
            .iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(
            names,
            vec!["acme-carrier".to_string(), "shared".to_string()]
        );

        // A tenant with no trunks of its own still calls out.
        let unknown = TenantId::new();
        let names: Vec<String> = set
            .for_tenant(Some(unknown))
            .iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(names, vec!["shared".to_string()]);

        // A call with no tenant is the platform's own.
        assert_eq!(set.for_tenant(None).len(), 1);
        assert_eq!(set.all().len(), 3);
    }

    #[test]
    fn a_tenant_can_refuse_the_platforms_trunks() {
        let a = TenantId::new();
        let set = TrunkSet::platform_only(vec![trunk("shared", None)]);
        set.set_tenant_trunks(
            vec![trunk("acme-carrier", Some(a))],
            &HashMap::from([(a, false)]),
        );

        let names: Vec<String> = set
            .for_tenant(Some(a))
            .iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(
            names,
            vec!["acme-carrier".to_string()],
            "this tenant's calls never leave on a shared carrier"
        );

        // And one that refuses the platform's trunks without having any of
        // its own has nowhere to go, which the router reports as such.
        let b = TenantId::new();
        set.set_tenant_trunks(Vec::new(), &HashMap::from([(b, false)]));
        assert!(set.for_tenant(Some(b)).is_empty());
    }

    fn inbound(name: &str, tenant: Option<TenantId>, sources: &[&str]) -> Trunk {
        let mut t = trunk(name, tenant);
        t.direction = fcp_core::trunk::TrunkDirection::Bidirectional;
        t.inbound_sources = sources.iter().map(|s| s.to_string()).collect();
        t
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_trunk_takes_calls_from_its_ranges_or_else_its_hosts() {
        let a = TenantId::new();
        let mut by_host = inbound("by-host", Some(a), &[]);
        by_host.hosts = vec![
            "10.0.0.9:5060".to_string(),
            "[2001:db8::7]:5061".to_string(),
        ];
        let mut outbound_only = inbound("outbound", Some(a), &["192.0.2.0/24"]);
        outbound_only.direction = fcp_core::trunk::TrunkDirection::Upstream;
        let mut disabled = inbound("disabled", Some(a), &["192.0.2.0/24"]);
        disabled.enabled = false;
        let set = TrunkSet::platform_only(Vec::new());
        set.set_tenant_trunks(
            vec![
                by_host,
                inbound("by-range", Some(a), &["198.51.100.0/24"]),
                outbound_only,
                disabled,
            ],
            &HashMap::new(),
        );

        let names = |from: &str| -> Vec<String> {
            set.inbound_from(ip(from))
                .iter()
                .map(|t| t.name.clone())
                .collect()
        };
        assert_eq!(names("10.0.0.9"), vec!["by-host"]);
        assert_eq!(names("2001:db8::7"), vec!["by-host"]);
        assert_eq!(names("198.51.100.77"), vec!["by-range"]);
        assert!(
            names("192.0.2.1").is_empty(),
            "an outbound-only or disabled trunk takes no calls"
        );
        assert!(names("203.0.113.1").is_empty());
    }

    #[test]
    fn the_most_specific_range_comes_first() {
        let (a, b) = (TenantId::new(), TenantId::new());
        let set = TrunkSet::platform_only(Vec::new());
        set.set_tenant_trunks(
            vec![
                inbound("wide", Some(a), &["198.51.100.0/24"]),
                inbound("narrow", Some(b), &["198.51.100.8/29"]),
            ],
            &HashMap::new(),
        );
        let names: Vec<String> = set
            .inbound_from(ip("198.51.100.9"))
            .iter()
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(names, vec!["narrow", "wide"]);
    }

    #[test]
    fn host_part_strips_ports_and_brackets() {
        assert_eq!(host_part("10.0.0.1:5060"), "10.0.0.1");
        assert_eq!(host_part("sip.carrier.example"), "sip.carrier.example");
        assert_eq!(host_part("sip.carrier.example:5080"), "sip.carrier.example");
        assert_eq!(host_part("[2001:db8::1]:5060"), "2001:db8::1");
        assert_eq!(host_part("2001:db8::1"), "2001:db8::1");
    }
}

#[cfg(test)]
mod srv_tests {
    use super::*;
    use sip_dns::{DnsTarget, Resolver};

    /// Answers every question with the targets it was given.
    struct FixedResolver(Vec<DnsTarget>);

    #[async_trait::async_trait]
    impl Resolver for FixedResolver {
        async fn resolve(&self, _uri: &sip_core::SipUri) -> anyhow::Result<Vec<DnsTarget>> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn a_host_given_by_name_with_no_port_is_what_rfc_3263_says() {
        let mut trunk = Trunk {
            name: "carrier".to_string(),
            ..Default::default()
        };
        trunk.enabled = true;
        trunk.direction = fcp_core::TrunkDirection::Bidirectional;
        trunk.hosts = vec![
            "sip.carrier.example".to_string(),
            "10.0.0.9:5080".to_string(),
        ];
        let resolver = Arc::new(FixedResolver(vec![
            DnsTarget::unchecked_new("192.0.2.10", 5062, sip_dns::Transport::Udp),
            DnsTarget::unchecked_new("192.0.2.11", 5062, sip_dns::Transport::Udp),
        ]));
        let set = TrunkSet::platform_only(vec![trunk]).with_resolver(resolver);
        assert!(set
            .srv_targets("sip.carrier.example", TransportType::Udp)
            .is_none());
        set.resolve_names().await;
        assert_eq!(
            set.srv_targets("sip.carrier.example", TransportType::Udp),
            Some(vec![
                ("192.0.2.10".to_string(), 5062),
                ("192.0.2.11".to_string(), 5062)
            ])
        );
        // A host with its own port is not asked about.
        assert!(set.srv_targets("10.0.0.9", TransportType::Udp).is_none());
        // The SRV targets are where the carrier's calls come from.
        assert_eq!(set.inbound_from("192.0.2.11".parse().unwrap()).len(), 1);
        assert_eq!(set.inbound_from("10.0.0.9".parse().unwrap()).len(), 1);
        assert!(set.inbound_from("192.0.2.12".parse().unwrap()).is_empty());
    }

    #[test]
    fn the_question_names_the_trunks_transport() {
        assert_eq!(
            srv_question("sip.carrier.example", TransportType::Udp),
            "sip:sip.carrier.example;transport=udp"
        );
        assert_eq!(
            srv_question("sip.carrier.example", TransportType::Tls),
            "sips:sip.carrier.example"
        );
    }
}
