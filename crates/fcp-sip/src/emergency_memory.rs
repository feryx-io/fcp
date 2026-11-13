//! What a node keeps so that an emergency call is placed while its store
//! is away (#326, `docs/E911.md` "With the database away").
//!
//! With its Postgres away a call manager can place no call: who the caller
//! is, their tenant, the tenant's emergency numbers and where the caller is
//! are all read from it. A 503 is no answer to 911. So each REGISTER this
//! node takes while the store answers leaves here what an emergency call
//! from that phone would read:
//!
//! - the binding: the address of record, its tenant, the address it
//!   registered from and until when;
//! - the caller: their HA1, while they may use services, and what places
//!   them ([`CallerPlacement`]: device, sites, number, declared location);
//! - the tenant: its emergency configuration and its networks with their
//!   sites.
//!
//! The caller and the tenant are read again at most every [`REFRESH`]; the
//! binding is renewed by every REGISTER. While the store is away, a
//! REGISTER that proves the remembered password renews the binding here
//! (it is still answered 503: nothing was stored).
//!
//! A caller is trusted for as long as a binding of theirs stands — the
//! registration this node took. A phone this node never registered, or one
//! whose binding lapsed, is refused 503 as before: there is nobody to
//! believe and nowhere to place them.

use crate::registrar::same_address;
use dashmap::DashMap;
use fcp_b2bua_core::EmergencyCallInfo;
use fcp_core::e911::{
    CallerContext, CallerPlacement, EmergencyConfig, EmergencyKind, PlacedNetworks,
};
use fcp_core::storage::CredentialStore;
use fcp_core::TenantId;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, info};

/// How often a caller's and a tenant's records are read again while their
/// phones keep registering.
pub const REFRESH: Duration = Duration::from_secs(300);
/// How often bindings that lapsed are swept out.
const SWEEP: Duration = Duration::from_secs(60);

/// One registration of a caller's, as this node took it.
#[derive(Debug, Clone)]
struct Binding {
    source: IpAddr,
    until: SystemTime,
}

#[derive(Debug, Clone)]
struct Caller {
    tenant: TenantId,
    /// The user part of the address of record: the SIP name.
    name: String,
    /// The user's HA1, while they may use services.
    ha1: Option<String>,
    placement: CallerPlacement,
    read_at: Option<Instant>,
    bindings: Vec<Binding>,
}

impl Caller {
    fn live(&self, now: SystemTime) -> bool {
        self.bindings.iter().any(|b| b.until > now)
    }

    fn at(&self, source: IpAddr, now: SystemTime) -> bool {
        self.bindings
            .iter()
            .any(|b| b.until > now && same_address(b.source, source))
    }
}

#[derive(Debug, Clone)]
struct Tenant {
    config: EmergencyConfig,
    networks: PlacedNetworks,
    read_at: Instant,
}

/// See the module's documentation.
#[derive(Default)]
pub struct EmergencyMemory {
    /// By address of record, `user@host` in lower case.
    callers: DashMap<String, Caller>,
    tenants: DashMap<TenantId, Tenant>,
    swept: std::sync::Mutex<Option<Instant>>,
}

/// `sip:User@Host;x` → `user@host`.
pub fn aor_key(aor: &str) -> String {
    let aor = aor.trim();
    let aor = aor
        .strip_prefix("sips:")
        .or_else(|| aor.strip_prefix("sip:"))
        .unwrap_or(aor);
    aor.split(';').next().unwrap_or(aor).to_ascii_lowercase()
}

/// How long a REGISTER asks, or its 200 OK binds, by `headers`: the
/// longest `expires` among the contacts, else `Expires`. Zero is a removal.
pub fn granted_secs(headers: &sip_core::Headers) -> Option<u64> {
    let from_contacts = headers
        .get_all("Contact")
        .flat_map(|c| {
            c.split(',')
                .filter_map(|one| {
                    one.split(';').skip(1).find_map(|p| {
                        let (k, v) = p.split_once('=')?;
                        k.trim()
                            .eq_ignore_ascii_case("expires")
                            .then(|| v.trim().trim_matches('"').parse::<u64>().ok())
                            .flatten()
                    })
                })
                .collect::<Vec<_>>()
        })
        .max();
    from_contacts.or_else(|| {
        sip_parse::header(headers, "Expires").and_then(|e| e.as_str().trim().parse::<u64>().ok())
    })
}

impl EmergencyMemory {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A REGISTER for `aor` from `source` was taken here, binding for
    /// `secs` (none: a removal). Renews what is known at once; reads the
    /// caller and their tenant again, on its own task, when they are older
    /// than [`REFRESH`] or not known.
    pub fn registered(
        self: &Arc<Self>,
        storage: Arc<fcp_core::DynStorage>,
        registrar: Arc<crate::registrar::FcpRegistrar>,
        aor: &str,
        source: IpAddr,
        secs: u64,
    ) {
        self.sweep();
        let key = aor_key(aor);
        let now = SystemTime::now();
        if secs == 0 {
            if let Some(mut caller) = self.callers.get_mut(&key) {
                caller.bindings.retain(|b| !same_address(b.source, source));
            }
            return;
        }
        let until = now + Duration::from_secs(secs);
        let fresh = self.renew(&key, source, until)
            && self.callers.get(&key).is_some_and(|c| {
                c.read_at.is_some_and(|at| at.elapsed() < REFRESH)
                    && self
                        .tenants
                        .get(&c.tenant)
                        .is_some_and(|t| t.read_at.elapsed() < REFRESH)
            });
        if fresh {
            return;
        }
        let memory = Arc::clone(self);
        let aor = aor.to_string();
        tokio::spawn(async move {
            memory
                .read(&storage, &registrar, &aor, &key, source, until)
                .await;
        });
    }

    /// Renew a known caller's binding at `source`; whether they are known.
    fn renew(&self, key: &str, source: IpAddr, until: SystemTime) -> bool {
        let Some(mut caller) = self.callers.get_mut(key) else {
            return false;
        };
        match caller
            .bindings
            .iter_mut()
            .find(|b| same_address(b.source, source))
        {
            Some(b) => b.until = until,
            None => caller.bindings.push(Binding { source, until }),
        }
        true
    }

    /// A REGISTER proved the remembered password while the store is away:
    /// the caller's binding at `source` stands for `secs` more here.
    pub fn renewed_offline(&self, aor: &str, source: IpAddr, secs: u64) {
        if secs == 0 {
            return;
        }
        let until = SystemTime::now() + Duration::from_secs(secs);
        if self.renew(&aor_key(aor), source, until) {
            debug!(aor = %aor, %source, secs, "A remembered caller's binding renewed with the store away");
        }
    }

    async fn read(
        &self,
        storage: &fcp_core::DynStorage,
        registrar: &crate::registrar::FcpRegistrar,
        aor: &str,
        key: &str,
        source: IpAddr,
        until: SystemTime,
    ) {
        // The tenant the REGISTER was resolved in; reading the binding
        // back would fill the registration cache a call is to pay for.
        let Ok(tenant) = registrar.tenant_of_aor(aor).await else {
            return;
        };
        let name = key.split('@').next().unwrap_or_default().to_string();
        let user = match storage.get_user_by_sip_name(&name, &tenant).await {
            Ok(user) => user,
            Err(e) => {
                debug!(aor = %aor, error = %e, "Not remembered for emergency calls: the user could not be read");
                return;
            }
        };
        let (ha1, placement) = match &user {
            Some(user) => (
                user.may_use_services()
                    .then(|| user.sip_ha1_hash.clone())
                    .flatten(),
                fcp_core::e911::caller_placement(storage, user, Some(key)).await,
            ),
            // A device's own credentials: not a user, nothing to place.
            None => (None, CallerPlacement::default()),
        };
        let stale_tenant = self
            .tenants
            .get(&tenant)
            .is_none_or(|t| t.read_at.elapsed() >= REFRESH);
        if stale_tenant {
            let config = match storage.emergency_store() {
                Some(store) => store.get_emergency_config(&tenant).await.ok().flatten(),
                None => None,
            }
            .unwrap_or_else(|| EmergencyConfig::defaults(tenant));
            let networks = fcp_core::e911::placed_networks(storage, &tenant).await;
            self.tenants.insert(
                tenant,
                Tenant {
                    config,
                    networks,
                    read_at: Instant::now(),
                },
            );
        }
        let mut entry = self.callers.entry(key.to_string()).or_insert(Caller {
            tenant,
            name: name.clone(),
            ha1: None,
            placement: CallerPlacement::default(),
            read_at: None,
            bindings: Vec::new(),
        });
        let caller = entry.value_mut();
        caller.tenant = tenant;
        caller.name = name;
        caller.ha1 = ha1;
        caller.placement = placement;
        caller.read_at = Some(Instant::now());
        match caller
            .bindings
            .iter_mut()
            .find(|b| same_address(b.source, source))
        {
            Some(b) => b.until = b.until.max(until),
            None => caller.bindings.push(Binding { source, until }),
        }
        debug!(aor = %aor, tenant_id = %tenant, "Remembered for emergency calls");
    }

    fn sweep(&self) {
        let Ok(mut swept) = self.swept.lock() else {
            return;
        };
        if swept.is_some_and(|at| at.elapsed() < SWEEP) {
            return;
        }
        *swept = Some(Instant::now());
        drop(swept);
        let now = SystemTime::now();
        self.callers.retain(|_, c| {
            c.bindings.retain(|b| b.until > now);
            !c.bindings.is_empty()
        });
        let tenants: std::collections::HashSet<TenantId> =
            self.callers.iter().map(|c| c.tenant).collect();
        self.tenants.retain(|t, _| tenants.contains(t));
    }

    /// Whether `dialed` is a live emergency number for any tenant known
    /// here, or by the defaults: the one call let past a store that does
    /// not answer.
    pub fn may_be_emergency(&self, dialed: &str) -> bool {
        let live = |config: &EmergencyConfig| config.classify(dialed) == Some(EmergencyKind::Live);
        live(&EmergencyConfig::defaults(TenantId::new()))
            || self.tenants.iter().any(|t| live(&t.config))
    }

    /// The tenant of a caller registered here as `aor` from `source`, while
    /// that binding stands.
    pub fn registered_tenant(&self, aor: &str, source: IpAddr) -> Option<TenantId> {
        let now = SystemTime::now();
        self.callers
            .get(&aor_key(aor))
            .filter(|c| c.at(source, now))
            .map(|c| c.tenant)
    }

    /// The HA1 of `username` in `tenant`, while a binding of theirs
    /// stands and they may use services.
    pub fn ha1(&self, tenant: &TenantId, username: &str) -> Option<String> {
        let now = SystemTime::now();
        self.callers.iter().find_map(|c| {
            (c.tenant == *tenant && c.name.eq_ignore_ascii_case(username) && c.live(now))
                .then(|| c.ha1.clone())
                .flatten()
        })
    }

    /// The HA1 of whoever registered `aor` here, while a binding stands:
    /// for a REGISTER while the store is away.
    pub fn ha1_for_aor(&self, aor: &str, username: &str) -> Option<String> {
        let now = SystemTime::now();
        self.callers
            .get(&aor_key(aor))
            .filter(|c| c.live(now) && c.name.eq_ignore_ascii_case(username))
            .and_then(|c| c.ha1.clone())
    }

    /// The tenant's emergency configuration as last read, or the defaults.
    pub fn config(&self, tenant: &TenantId) -> EmergencyConfig {
        self.tenants
            .get(tenant)
            .map(|t| t.config.clone())
            .unwrap_or_else(|| EmergencyConfig::defaults(*tenant))
    }

    /// [`crate::emergency::detect`] from what is kept: the call's kind by
    /// the tenant's numbers, the caller placed as their records were last
    /// read, by the network the call came from, or not at all.
    pub fn detect(
        &self,
        tenant: &TenantId,
        destination: &str,
        username: Option<&str>,
        aor: Option<&str>,
        client_address: Option<IpAddr>,
        supplied: Option<&fcp_core::E911Address>,
    ) -> Option<EmergencyCallInfo> {
        let config = self.config(tenant);
        let kind = config.classify(destination)?;
        let now = SystemTime::now();
        let caller = aor
            .and_then(|a| self.callers.get(&aor_key(a)).map(|c| c.clone()))
            .filter(|c| c.tenant == *tenant)
            .or_else(|| {
                let name = username?;
                self.callers.iter().find_map(|c| {
                    (c.tenant == *tenant && c.name.eq_ignore_ascii_case(name)).then(|| c.clone())
                })
            });
        let placement = caller.as_ref().map(|c| &c.placement);
        let networks = self
            .tenants
            .get(tenant)
            .map(|t| t.networks.clone())
            .unwrap_or_default();
        let empty = CallerPlacement::default();
        let location = placement.unwrap_or(&empty).resolve(
            &networks,
            CallerContext {
                prefer_device_aor: aor,
                client_address,
                supplied,
                config: Some(&config),
            },
            now,
        );
        let callback_number = location
            .as_ref()
            .and_then(|l| l.callback_number.clone())
            .or_else(|| placement.and_then(CallerPlacement::callback));
        info!(
            tenant_id = %tenant,
            caller = username.unwrap_or("unknown"),
            remembered = caller.is_some(),
            located = location.is_some(),
            "An emergency call placed from what this node remembers: the store is away"
        );
        Some(EmergencyCallInfo {
            dialed: fcp_core::e911::dialed_user_part(destination).to_string(),
            kind,
            callback_number,
            location,
            leg: None,
            delivered: Vec::new(),
            geolocation_error: None,
            read_back: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(tenant: TenantId, name: &str, source: IpAddr, until: SystemTime) -> Caller {
        Caller {
            tenant,
            name: name.into(),
            ha1: Some("ha1".into()),
            placement: CallerPlacement::default(),
            read_at: Some(Instant::now()),
            bindings: vec![Binding { source, until }],
        }
    }

    #[test]
    fn a_caller_is_believed_while_their_binding_stands() {
        let memory = EmergencyMemory::default();
        let tenant = TenantId::new();
        let at: IpAddr = "192.0.2.10".parse().unwrap();
        let later = SystemTime::now() + Duration::from_secs(60);
        memory
            .callers
            .insert("alice@e2e.local".into(), caller(tenant, "alice", at, later));

        assert_eq!(
            memory.registered_tenant("sip:Alice@e2e.local;transport=udp", at),
            Some(tenant)
        );
        assert_eq!(
            memory.registered_tenant("sip:alice@e2e.local", "192.0.2.11".parse().unwrap()),
            None,
            "only from where it registered"
        );
        assert_eq!(memory.ha1(&tenant, "ALICE").as_deref(), Some("ha1"));
        assert_eq!(memory.ha1(&TenantId::new(), "alice"), None);

        // Lapsed: nobody to believe.
        memory.callers.get_mut("alice@e2e.local").unwrap().bindings[0].until =
            SystemTime::now() - Duration::from_secs(1);
        assert_eq!(memory.registered_tenant("sip:alice@e2e.local", at), None);
        assert_eq!(memory.ha1(&tenant, "alice"), None);

        // Renewed while the store is away.
        memory.renewed_offline("sip:alice@e2e.local", at, 120);
        assert_eq!(
            memory.registered_tenant("sip:alice@e2e.local", at),
            Some(tenant)
        );
    }

    #[test]
    fn only_a_live_emergency_number_passes() {
        let memory = EmergencyMemory::default();
        assert!(memory.may_be_emergency("sip:911@e2e.local"));
        assert!(!memory.may_be_emergency("sip:1002@e2e.local"));
        let tenant = TenantId::new();
        let mut config = EmergencyConfig::defaults(tenant);
        config.emergency_numbers.push("999".into());
        memory.tenants.insert(
            tenant,
            Tenant {
                config,
                networks: Vec::new(),
                read_at: Instant::now(),
            },
        );
        assert!(memory.may_be_emergency("sip:999@e2e.local"));
        let info = memory
            .detect(&tenant, "sip:999@e2e.local", Some("bob"), None, None, None)
            .expect("an emergency call");
        assert_eq!(info.kind, EmergencyKind::Live);
        assert!(info.location.is_none() && info.callback_number.is_none());
        assert!(memory
            .detect(&tenant, "sip:1002@e2e.local", None, None, None, None)
            .is_none());
    }

    #[test]
    fn a_registers_grant_is_its_longest_contact() {
        let response = |headers: &[(&str, &str)]| {
            let mut h = sip_core::Headers::new();
            for (k, v) in headers {
                let _ = h.push(smol_str::SmolStr::new(*k), smol_str::SmolStr::new(*v));
            }
            sip_core::Response::new(
                sip_core::StatusLine::new(200, "OK").unwrap(),
                h,
                bytes::Bytes::new(),
            )
            .unwrap()
        };
        assert_eq!(
            granted_secs(
                response(&[
                    ("Contact", "<sip:a@192.0.2.1>;expires=60"),
                    ("Contact", "<sip:a@192.0.2.2>;expires=3600"),
                ])
                .headers()
            ),
            Some(3600)
        );
        assert_eq!(
            granted_secs(response(&[("Expires", "300")]).headers()),
            Some(300)
        );
        assert_eq!(granted_secs(response(&[]).headers()), None);
        assert_eq!(
            aor_key("sips:Bob@Example.COM;transport=tls"),
            "bob@example.com"
        );
    }
}
