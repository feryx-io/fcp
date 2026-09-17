//! What a node routes by: its own trunks, and every tenant's
//! (`docs/TENANT_TRUNKS.md` §4).
//!
//! A node reads its `[[trunks]]` once, at startup — those are the
//! platform's, shared by every tenant that has none of its own — and reads
//! the tenants' trunks from the store, refreshing them when they change.
//! Selection asks for one tenant's trunks and gets that tenant's first,
//! then the platform's, unless the tenant has said its calls may not leave
//! on a shared carrier.

use fcp_core::storage::{TenantStore, TrunkStore};
use fcp_core::{TenantId, Trunk};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, warn};

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
}

impl TrunkSet {
    /// A node with only its own trunks — every deployment that has never
    /// created a trunk through the API, and the SBC, which is the
    /// platform's edge (§13.8).
    pub fn platform_only(trunks: Vec<Trunk>) -> Self {
        Self {
            platform: trunks.into_iter().map(Arc::new).collect(),
            tenants: RwLock::new(HashMap::new()),
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

    /// This node's own trunks.
    pub fn platform(&self) -> &[Arc<Trunk>] {
        &self.platform
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
        count
    }

    /// Read every tenant's trunks, and what each tenant says about the
    /// platform's.
    pub async fn refresh<S>(&self, store: &S) -> anyhow::Result<usize>
    where
        S: TrunkStore + TenantStore + ?Sized,
    {
        let trunks = store.list_all_trunks().await?;
        let fallback: HashMap<TenantId, bool> = store
            .list_tenants()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|t| (t.id, t.platform_trunk_fallback))
            .collect();
        let count = self.set_tenant_trunks(trunks, &fallback);
        debug!(
            trunks = count,
            tenants = self.tenant_count(),
            "Tenant trunks refreshed"
        );
        Ok(count)
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
}
