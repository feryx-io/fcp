//! Each person's presence, worked out by the admin (`docs/PRESENCE.md`
//! §3): the status they chose, laid under their calls, do not disturb,
//! their apps' idleness and whether any of their phones or apps is
//! registered ([`fcp_core::presence_status::effective`]).
//!
//! A tenant is read whole the first time it is asked for (its users and
//! devices, its registrations, chosen statuses and calling settings) and
//! kept current by events: registrations (read again from the store, so
//! one device of two leaving changes nothing), `call.initiated` for
//! ringing, `presence.user_call` from the call managers for calls, and
//! `user.calling_changed` for DND. It is read again every few minutes for
//! what no event says (a registration that lapsed, a user added or
//! renamed), and a status's `until` is checked as often. Each change of
//! what others see is announced as `presence.user_changed`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use dashmap::DashMap;
use fcp_core::calling::CallingStore;
use fcp_core::presence_status::{
    effective, ChosenStatus, Happening, Presence, PresenceStatusStore, Resolved, StatusState,
    IDLE_LASTS,
};
use fcp_core::storage::{DeviceStore, DynStorage, RegistrationStore, UserFilter};
use fcp_core::CredentialStore;
use fcp_core::{TenantId, UserId};
use fcp_events::{CallEvent, EventPublisher, PlatformEvent, PresenceEvent, RegistrationEvent};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// How long a tenant read whole lasts before it is read again.
pub const REREAD_EVERY: Duration = Duration::from_secs(5 * 60);

/// A person as presence sees them.
#[derive(Debug, Clone)]
struct Person {
    username: String,
    extension: Option<String>,
    /// May use services; a suspended or deprovisioned user is offline.
    active: bool,
    chosen: Option<ChosenStatus>,
    resolved: Resolved,
    dnd: bool,
    registered: bool,
    ringing: HashSet<String>,
    calls: HashSet<String>,
    /// Each app session's word that its person is idle, until when.
    idle: HashMap<String, Instant>,
    shown: Option<Presence>,
}

impl Person {
    fn happening(&self, now: Instant) -> Happening {
        Happening {
            registered: self.active && self.registered,
            ringing: !self.ringing.is_empty(),
            on_call: !self.calls.is_empty(),
            dnd: self.dnd,
            idle: !self.idle.is_empty() && self.idle.values().all(|until| *until > now),
        }
    }
}

#[derive(Default)]
struct Tenant {
    read_at: Option<Instant>,
    /// Lower-cased SIP names, extensions and devices' SIP names → person.
    names: HashMap<String, UserId>,
    people: HashMap<UserId, Person>,
    /// Who was asked after and was not among the tenant's users when it
    /// was read for them, so asking again does not read it again.
    strangers: HashSet<UserId>,
    /// Which people each ringing call rings.
    ringing: HashMap<String, Vec<UserId>>,
}

/// Every tenant's people's presence.
#[derive(Clone)]
pub struct PeoplePresence {
    storage: Arc<DynStorage>,
    events: Arc<dyn EventPublisher>,
    tenants: Arc<DashMap<TenantId, Arc<Mutex<Tenant>>>>,
}

/// The user part of a SIP address or a bare name, lower-cased.
fn name_of(address: &str) -> String {
    let a = address.trim().trim_start_matches('<');
    let a = a
        .strip_prefix("sips:")
        .or_else(|| a.strip_prefix("sip:"))
        .or_else(|| a.strip_prefix("tel:"))
        .unwrap_or(a);
    a.split(['@', ';', '>'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

impl PeoplePresence {
    /// Announcing changes on `events`.
    pub fn new(storage: Arc<DynStorage>, events: Arc<dyn EventPublisher>) -> Self {
        Self {
            storage,
            events,
            tenants: Arc::new(DashMap::new()),
        }
    }

    fn statuses(&self) -> Option<&Arc<dyn PresenceStatusStore>> {
        self.storage.presence_status_store()
    }

    fn tenant(&self, tenant: &TenantId) -> Arc<Mutex<Tenant>> {
        self.tenants.entry(*tenant).or_default().clone()
    }

    /// A person's presence; `None` when they are not one of the tenant's
    /// users.
    pub async fn of(&self, tenant: &TenantId, user: &UserId) -> Option<Presence> {
        let t = self.tenant(tenant);
        let mut t = t.lock().await;
        self.ensure_knows(tenant, &mut t, user).await;
        let now = Instant::now();
        t.people
            .get(user)
            .map(|p| effective(&p.resolved, p.happening(now)))
    }

    /// Every person's presence in a tenant (the directory).
    pub async fn tenant_presence(&self, tenant: &TenantId) -> HashMap<UserId, Presence> {
        let t = self.tenant(tenant);
        let mut t = t.lock().await;
        self.ensure_read(tenant, &mut t).await;
        let now = Instant::now();
        t.people
            .iter()
            .map(|(id, p)| (*id, effective(&p.resolved, p.happening(now))))
            .collect()
    }

    /// The person's chosen status changed (`/me/presence`): read it again.
    pub async fn chosen_changed(&self, tenant: &TenantId, user: &UserId) {
        let t = self.tenant(tenant);
        let mut t = t.lock().await;
        self.ensure_knows(tenant, &mut t, user).await;
        self.read_person_settings(tenant, &mut t, user).await;
        self.announce(tenant, &mut t).await;
    }

    /// An app session says its person is idle, or no longer.
    pub async fn set_idle(&self, tenant: &TenantId, user: &UserId, session: &str, idle: bool) {
        let t = self.tenant(tenant);
        let mut t = t.lock().await;
        self.ensure_knows(tenant, &mut t, user).await;
        if let Some(p) = t.people.get_mut(user) {
            if idle {
                p.idle
                    .insert(session.to_string(), Instant::now() + IDLE_LASTS);
            } else {
                p.idle.remove(session);
            }
        }
        self.announce(tenant, &mut t).await;
    }

    /// Take in one event.
    pub async fn on_event(&self, event: &PlatformEvent) {
        match event {
            PlatformEvent::Registration(
                RegistrationEvent::Registered { tenant_id, .. }
                | RegistrationEvent::Refreshed { tenant_id, .. }
                | RegistrationEvent::Unregistered { tenant_id, .. }
                | RegistrationEvent::Expired { tenant_id, .. },
            ) => {
                let t = self.tenant(tenant_id);
                let mut t = t.lock().await;
                if t.read_at.is_none() {
                    return;
                }
                self.read_registrations(tenant_id, &mut t).await;
                self.announce(tenant_id, &mut t).await;
            }
            PlatformEvent::Call(CallEvent::Initiated {
                call_id,
                tenant_id,
                to,
                ..
            }) => {
                let t = self.tenant(tenant_id);
                let mut t = t.lock().await;
                if t.read_at.is_none() {
                    return;
                }
                let Some(user) = t.names.get(&name_of(to)).copied() else {
                    return;
                };
                let call = call_id.to_string();
                if let Some(p) = t.people.get_mut(&user) {
                    p.ringing.insert(call.clone());
                }
                t.ringing.entry(call).or_default().push(user);
                self.announce(tenant_id, &mut t).await;
            }
            PlatformEvent::Call(
                CallEvent::Answered {
                    call_id,
                    tenant_id: Some(tenant_id),
                    ..
                }
                | CallEvent::Ended {
                    call_id,
                    tenant_id: Some(tenant_id),
                    ..
                }
                | CallEvent::Failed {
                    call_id,
                    tenant_id: Some(tenant_id),
                    ..
                },
            ) => {
                let t = self.tenant(tenant_id);
                let mut t = t.lock().await;
                let call = call_id.to_string();
                if let Some(users) = t.ringing.remove(&call) {
                    for u in users {
                        if let Some(p) = t.people.get_mut(&u) {
                            p.ringing.remove(&call);
                        }
                    }
                    self.announce(tenant_id, &mut t).await;
                }
            }
            PlatformEvent::Presence(PresenceEvent::UserCall {
                user_id,
                call_id,
                on_call,
                tenant_id,
                ..
            }) => {
                let Ok(user) = user_id.parse::<UserId>() else {
                    return;
                };
                let t = self.tenant(tenant_id);
                let mut t = t.lock().await;
                if t.read_at.is_none() {
                    return;
                }
                if let Some(p) = t.people.get_mut(&user) {
                    if *on_call {
                        p.calls.insert(call_id.clone());
                        p.ringing.remove(call_id);
                    } else {
                        p.calls.remove(call_id);
                    }
                }
                self.announce(tenant_id, &mut t).await;
            }
            PlatformEvent::User(fcp_events::UserEvent::CallingChanged {
                user_id,
                tenant_id,
                ..
            }) => {
                let Ok(user) = user_id.parse::<UserId>() else {
                    return;
                };
                let t = self.tenant(tenant_id);
                let mut t = t.lock().await;
                if t.read_at.is_none() {
                    return;
                }
                self.read_person_settings(tenant_id, &mut t, &user).await;
                self.dnd_status_lapses(&mut t, &user).await;
                self.announce(tenant_id, &mut t).await;
            }
            _ => {}
        }
    }

    /// Read again whatever no event says: tenants read longer ago than
    /// [`REREAD_EVERY`], statuses whose `until` has passed, idleness no
    /// app repeated.
    pub async fn tick(&self) {
        let tenants: Vec<TenantId> = self.tenants.iter().map(|e| *e.key()).collect();
        let now = SystemTime::now();
        for tenant in tenants {
            let t = self.tenant(&tenant);
            let mut t = t.lock().await;
            if t.read_at.is_none() {
                continue;
            }
            if t.read_at.is_some_and(|at| at.elapsed() >= REREAD_EVERY) {
                self.read_tenant(&tenant, &mut t).await;
            }
            let cleared: Vec<UserId> = t
                .people
                .iter()
                .filter(|(_, p)| p.chosen.as_ref().is_some_and(|c| c.cleared(now)))
                .map(|(id, _)| *id)
                .collect();
            for user in cleared {
                self.clear_status(&tenant, &mut t, &user).await;
            }
            let instant = Instant::now();
            for p in t.people.values_mut() {
                p.idle.retain(|_, until| *until > instant);
            }
            self.announce(&tenant, &mut t).await;
        }
    }

    /// A status whose time has come goes back to Available, and DND goes
    /// off if the status had turned it on.
    async fn clear_status(&self, tenant: &TenantId, t: &mut Tenant, user: &UserId) {
        let Some(p) = t.people.get_mut(user) else {
            return;
        };
        // What it was before its time came: a cleared status reads as
        // available.
        let was_dnd = match p.chosen.as_ref() {
            Some(c) => match StatusState::parse(&c.status) {
                Some(state) => state == StatusState::Dnd,
                None => match self.statuses() {
                    Some(store) => store
                        .list_own_statuses(user)
                        .await
                        .unwrap_or_default()
                        .iter()
                        .any(|s| s.id == c.status && s.state == StatusState::Dnd),
                    None => false,
                },
            },
            None => false,
        };
        p.chosen = None;
        p.resolved = Resolved::available();
        if let Some(store) = self.statuses() {
            if let Err(e) = store.delete_chosen_status(user).await {
                warn!(user_id = %user, error = %e, "A cleared status was not removed");
            }
        }
        if was_dnd && p.dnd {
            p.dnd = false;
            match crate::v1::me_presence::set_dnd(&self.storage, user, false).await {
                Ok(true) => {
                    let event = PlatformEvent::User(fcp_events::UserEvent::CallingChanged {
                        user_id: user.to_string(),
                        username: p.username.clone(),
                        tenant_id: *tenant,
                        timestamp: SystemTime::now(),
                    });
                    let _ = self.events.publish(event).await;
                }
                Ok(false) => {}
                Err(e) => {
                    warn!(user_id = %user, error = %e, "DND was not turned off with its status")
                }
            }
        }
    }

    /// DND turned off elsewhere ends a `dnd` status: they are available.
    async fn dnd_status_lapses(&self, t: &mut Tenant, user: &UserId) {
        let Some(p) = t.people.get_mut(user) else {
            return;
        };
        if p.dnd || p.resolved.state != StatusState::Dnd {
            return;
        }
        p.chosen = None;
        p.resolved = Resolved::available();
        if let Some(store) = self.statuses() {
            if let Err(e) = store.delete_chosen_status(user).await {
                warn!(user_id = %user, error = %e, "A DND status was not cleared");
            }
        }
    }

    async fn ensure_read(&self, tenant: &TenantId, t: &mut Tenant) {
        if t.read_at.is_none() {
            self.read_tenant(tenant, t).await;
        }
    }

    /// As [`Self::ensure_read`], and the tenant read again when `user` is
    /// not among its people: a user made since the last read. Without
    /// this they had no presence until [`REREAD_EVERY`] came round: their
    /// idleness was dropped and nothing they chose was announced.
    async fn ensure_knows(&self, tenant: &TenantId, t: &mut Tenant, user: &UserId) {
        self.ensure_read(tenant, t).await;
        if t.people.contains_key(user) || t.strangers.contains(user) {
            return;
        }
        self.read_tenant(tenant, t).await;
        if !t.people.contains_key(user) {
            t.strangers.insert(*user);
        }
    }

    /// Read the tenant whole, keeping what only events said (calls,
    /// ringing, idleness).
    async fn read_tenant(&self, tenant: &TenantId, t: &mut Tenant) {
        let users = match self
            .storage
            .list_users_in_tenant(tenant, &UserFilter::default())
            .await
        {
            Ok(u) => u,
            Err(e) => {
                warn!(tenant_id = %tenant, error = %e, "A tenant's people were not read");
                return;
            }
        };
        let devices = self.storage.list_devices(tenant).await.unwrap_or_default();
        let chosen: HashMap<UserId, ChosenStatus> = match self.statuses() {
            Some(s) => s
                .list_chosen_statuses()
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|c| c.tenant_id == *tenant)
                .map(|c| (c.user_id, c))
                .collect(),
            None => HashMap::new(),
        };
        let dnd: HashMap<UserId, bool> = self
            .storage
            .list_calling_settings(tenant)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.user_id, s.dnd))
            .collect();

        let mut names = HashMap::new();
        let mut people = HashMap::new();
        for u in &users {
            names.insert(u.sip_name().to_lowercase(), u.id);
            if let Some(ext) = &u.extension {
                names.entry(ext.to_lowercase()).or_insert(u.id);
            }
            let before = t.people.remove(&u.id);
            let chosen = chosen.get(&u.id).cloned();
            let own = match (&chosen, self.statuses()) {
                (Some(c), Some(s)) if StatusState::parse(&c.status).is_none() => {
                    s.list_own_statuses(&u.id).await.unwrap_or_default()
                }
                _ => Vec::new(),
            };
            let resolved = Resolved::of(chosen.as_ref(), &own, SystemTime::now());
            let (ringing, calls, idle, shown) = before
                .map(|b| (b.ringing, b.calls, b.idle, b.shown))
                .unwrap_or_default();
            people.insert(
                u.id,
                Person {
                    username: u.username.clone(),
                    extension: u.extension.clone(),
                    active: u.may_use_services(),
                    chosen,
                    resolved,
                    dnd: dnd.get(&u.id).copied().unwrap_or(false),
                    registered: false,
                    ringing,
                    calls,
                    idle,
                    shown,
                },
            );
        }
        for d in &devices {
            let Some(owner) = d.owner_user_id else {
                continue;
            };
            for name in [d.sip_username.as_deref(), d.aor_user()]
                .into_iter()
                .flatten()
            {
                names.entry(name.to_lowercase()).or_insert(owner);
            }
        }
        t.names = names;
        t.people = people;
        t.strangers.clear();
        t.read_at = Some(Instant::now());
        self.read_registrations(tenant, t).await;
        debug!(tenant_id = %tenant, people = t.people.len(), "A tenant's presence read");
    }

    /// Who has a registration that has not expired, from the store.
    async fn read_registrations(&self, tenant: &TenantId, t: &mut Tenant) {
        let registrations = match self.storage.get_all_registrations_in_tenant(tenant).await {
            Ok(r) => r,
            Err(e) => {
                warn!(tenant_id = %tenant, error = %e, "Registrations were not read for presence");
                return;
            }
        };
        let now = SystemTime::now();
        let registered: HashSet<UserId> = registrations
            .iter()
            .filter(|r| r.expires_at > now)
            .filter_map(|r| t.names.get(&name_of(&r.aor)).copied())
            .collect();
        for (id, p) in t.people.iter_mut() {
            p.registered = registered.contains(id);
        }
    }

    /// Read a person's chosen status and DND again.
    async fn read_person_settings(&self, tenant: &TenantId, t: &mut Tenant, user: &UserId) {
        let Some(p) = t.people.get_mut(user) else {
            return;
        };
        if let Some(store) = self.statuses() {
            p.chosen = store.get_chosen_status(user).await.unwrap_or(None);
            let own = match &p.chosen {
                Some(c) if StatusState::parse(&c.status).is_none() => {
                    store.list_own_statuses(user).await.unwrap_or_default()
                }
                _ => Vec::new(),
            };
            p.resolved = Resolved::of(p.chosen.as_ref(), &own, SystemTime::now());
        }
        p.dnd = self
            .storage
            .get_calling_settings(tenant, user)
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s.dnd);
    }

    /// Announce each person whose presence is no longer what was last
    /// announced.
    async fn announce(&self, tenant: &TenantId, t: &mut Tenant) {
        let now = Instant::now();
        let mut changed = Vec::new();
        for (id, p) in t.people.iter_mut() {
            let shown = effective(&p.resolved, p.happening(now));
            if p.shown.as_ref() == Some(&shown) {
                continue;
            }
            // The first reading of a person is not a change.
            let first = p.shown.is_none();
            p.shown = Some(shown.clone());
            if !first {
                changed.push((*id, p.username.clone(), p.extension.clone(), shown));
            }
        }
        for (id, username, extension, shown) in changed {
            let event = PlatformEvent::Presence(PresenceEvent::UserChanged {
                user_id: id.to_string(),
                username,
                extension,
                state: shown.state.as_str().to_string(),
                label: shown.label,
                note: shown.note,
                idle: shown.idle,
                tenant_id: *tenant,
                timestamp: SystemTime::now(),
            });
            if let Err(e) = self.events.publish(event).await {
                warn!(user_id = %id, error = %e, "A person's presence was not announced");
            }
        }
    }

    /// Take in events from `bus` and tick, for as long as the process runs.
    pub fn run(self, bus: Arc<dyn fcp_events::EventBus>) {
        use fcp_events::filter::EventType;
        use fcp_events::EventFilter;
        let ticker = self.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs(30));
            loop {
                every.tick().await;
                ticker.tick().await;
            }
        });
        tokio::spawn(async move {
            let filter = EventFilter::new().types(vec![
                EventType::Registration,
                EventType::Call,
                EventType::Presence,
                EventType::User,
            ]);
            loop {
                let mut stream = match bus.subscribe(filter.clone()).await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(error = %e, "People's presence could not hear events");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                while let Some(event) = stream.next().await {
                    self.on_event(&event).await;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::name_of;

    #[test]
    fn a_name_is_the_user_part_of_any_address() {
        assert_eq!(name_of("sip:Bob@example.com"), "bob");
        assert_eq!(name_of("<sips:1001@example.com;transport=tls>"), "1001");
        assert_eq!(name_of("tel:+15551234"), "+15551234");
        assert_eq!(name_of("1001"), "1001");
    }
}
