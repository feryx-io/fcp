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
use fcp_core::{TenantId, Trunk};
use ipnetwork::IpNetwork;
use rust_decimal::prelude::ToPrimitive;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tracing::{debug, warn};

/// How long one trunk host's name may take to resolve during a refresh.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// Each tenant's least-cost routes, keyed by trunk id
    /// (`docs/TENANT_TRUNKS.md` §12, phase 6c).
    lcr: RwLock<HashMap<TenantId, Arc<LcrRouteTable>>>,
    /// What each tenant may carry at once, as of the last refresh
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). Absent means no limit.
    call_limits: RwLock<HashMap<TenantId, u32>>,
}

impl TrunkSet {
    /// A node with only its own trunks — every deployment that has never
    /// created a trunk through the API, and the SBC, which is the
    /// platform's edge (§13.8).
    pub fn platform_only(trunks: Vec<Trunk>) -> Self {
        let set = Self {
            platform: trunks.into_iter().map(Arc::new).collect(),
            tenants: RwLock::new(HashMap::new()),
            inbound: RwLock::new(Vec::new()),
            resolved: RwLock::new(HashMap::new()),
            lcr: RwLock::new(HashMap::new()),
            call_limits: RwLock::new(HashMap::new()),
        };
        set.rebuild_inbound();
        set
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
        let mut index = Vec::new();
        for trunk in self.all() {
            if !trunk.takes_inbound() {
                continue;
            }
            let networks = trunk.inbound_networks();
            if !networks.is_empty() {
                index.extend(networks.into_iter().map(|n| (n, Arc::clone(&trunk))));
                continue;
            }
            for host in &trunk.hosts {
                let host = host_part(host);
                let host = host.as_str();
                let addrs = match host.parse::<IpAddr>() {
                    Ok(ip) => vec![ip],
                    Err(_) => resolved.get(host).cloned().unwrap_or_default(),
                };
                index.extend(
                    addrs
                        .into_iter()
                        .map(|ip| (IpNetwork::from(ip), Arc::clone(&trunk))),
                );
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
        self.resolve_hosts().await;
        self.rebuild_inbound();
    }

    /// Resolve every trunk host given by name, keeping what a name last
    /// resolved to when it fails now.
    async fn resolve_hosts(&self) {
        let names: HashSet<String> = self
            .all()
            .iter()
            .filter(|t| t.takes_inbound() && t.inbound_sources.is_empty())
            .flat_map(|t| t.hosts.iter().map(|h| host_part(h)))
            .filter(|h| !h.is_empty() && h.parse::<IpAddr>().is_err())
            .collect();
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
        let count = trunks.len();
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
