//! A person's presence on the phones (`docs/PRESENCE.md` §3, phase 2).
//! The admin works out what others see of each person and announces it as
//! `presence.user_changed`; this node lays it on the addresses watchers
//! subscribe to — the person's SIP name and extension on each of the
//! tenant's domains, and any other address of theirs a phone has asked
//! about — as PIDF: basic `closed` only when they are offline, the RPID
//! activity for away, busy (do not disturb too) and on the phone, and
//! their own status's label and note as the note. Busy lamps (the dialog
//! package) stay driven by calls.
//!
//! A person nobody has announced since this node started is read from the
//! database when a phone first subscribes to them, as the admin reads them
//! (their chosen status, do not disturb, whether a phone of theirs is
//! registered), without their calls: the admin's next word brings those.
//!
//! Queues and hunt groups skip a member in do not disturb or on a call
//! ([`PeopleOnPhones::unavailable`]), not one who chose Busy or Away.

use fcp_core::calling::CallingStore;
use fcp_core::presence_status::{effective, Happening, Presence, Resolved, Shown, StatusState};
use fcp_core::storage::{CredentialStore, DeviceStore, RegistrationStore, TenantStore};
use fcp_core::{DynStorage, TenantId, UserId};
use fcp_presence::{PresenceManager, PresenceState, PresenceUpdate};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, warn};

/// What this node knows of each person's presence, laid on their addresses.
pub struct PeopleOnPhones {
    presence: PresenceManager,
    storage: Arc<DynStorage>,
    shown: PeopleShown,
}

/// What others see of each person, by their SIP name and extension
/// (lowercase), per tenant.
#[derive(Default)]
pub struct PeopleShown(dashmap::DashMap<String, HashMap<TenantId, Presence>>);

impl PeopleShown {
    pub fn remember(&self, tenant: &TenantId, names: &[String], presence: &Presence) {
        for name in names {
            self.0
                .entry(name.to_lowercase())
                .or_default()
                .insert(*tenant, presence.clone());
        }
    }

    pub fn get(&self, tenant: &TenantId, name: &str) -> Option<Presence> {
        self.0
            .get(&name.to_lowercase())
            .and_then(|by_tenant| by_tenant.get(tenant).cloned())
    }

    /// Whether the member with this extension (or SIP name) is in do not
    /// disturb or on a call, in the tenant asking: extension 2001 in
    /// another tenant is someone else. Someone not heard of is available.
    pub fn unavailable(&self, tenant: &TenantId, extension: &str) -> bool {
        self.get(tenant, extension)
            .is_some_and(|p| matches!(p.state, Shown::Dnd | Shown::OnCall))
    }
}

/// A person's presence as PIDF says it: the state and the note.
pub fn pidf_of(p: &Presence) -> (PresenceState, Option<String>) {
    let own = match (&p.label, &p.note) {
        (Some(label), Some(note)) => Some(format!("{label} — {note}")),
        (label, note) => label.clone().or_else(|| note.clone()),
    };
    match p.state {
        Shown::Offline => (PresenceState::Closed, None),
        Shown::Available => (PresenceState::Open, own),
        Shown::Away if p.idle => (PresenceState::Away, own.or(Some("Idle".into()))),
        Shown::Away => (PresenceState::Away, own),
        Shown::Busy => (PresenceState::Busy, own),
        Shown::Dnd => (PresenceState::Busy, own.or(Some("Do not disturb".into()))),
        Shown::Ringing => (PresenceState::Ringing, None),
        Shown::OnCall => (
            PresenceState::OnThePhone,
            own.map(|o| format!("On the phone · {o}")),
        ),
    }
}

/// The user part of an address, lowercase.
fn name_of(address: &str) -> String {
    let a = address.trim().trim_start_matches('<');
    let a = a
        .strip_prefix("sips:")
        .or_else(|| a.strip_prefix("sip:"))
        .unwrap_or(a);
    a.split(['@', ';', '>'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

impl PeopleOnPhones {
    pub fn new(presence: PresenceManager, storage: Arc<DynStorage>) -> Self {
        Self {
            presence,
            storage,
            shown: PeopleShown::default(),
        }
    }

    /// Lay what the admin announced of a person on their addresses. The
    /// addresses returned changed only their note: no
    /// `presence.status_changed` tells their watchers, so the caller does.
    pub async fn apply(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
        extension: Option<&str>,
        presence: &Presence,
    ) -> Vec<String> {
        let user = match self.storage.get_user_by_id(user_id).await {
            Ok(Some(user)) => user,
            Ok(None) => return Vec::new(),
            Err(e) => {
                warn!(user_id = %user_id, error = %e, "A person's presence was not laid on their phones");
                return Vec::new();
            }
        };
        let mut names = vec![user.sip_name().to_lowercase()];
        for ext in [extension, user.extension.as_deref()].into_iter().flatten() {
            let ext = ext.to_lowercase();
            if !names.contains(&ext) {
                names.push(ext);
            }
        }
        self.lay(tenant, &names, presence, None).await
    }

    /// Record a person's presence under `names` and put it on their
    /// addresses (and on `also`, an address of theirs a phone asked
    /// about). Returns the addresses whose note alone changed.
    async fn lay(
        &self,
        tenant: &TenantId,
        names: &[String],
        presence: &Presence,
        also: Option<&str>,
    ) -> Vec<String> {
        self.shown.remember(tenant, names, presence);
        let domains = match self.storage.get_tenant(tenant).await {
            Ok(Some(t)) => t.sip_domains,
            _ => Vec::new(),
        };
        let mut aors: HashSet<String> = names
            .iter()
            .flat_map(|n| {
                domains
                    .iter()
                    .map(move |d| format!("{n}@{d}").to_lowercase())
            })
            .collect();
        // Addresses on other hosts (the node's address, as phones in a
        // single-tenant deployment often use) that a phone asked about.
        for p in self.presence.get_presences_by_tenant(tenant) {
            if names.contains(&name_of(&p.aor)) {
                aors.insert(p.aor);
            }
        }
        if let Some(also) = also {
            aors.insert(also.to_string());
        }

        let (state, note) = pidf_of(presence);
        let mut note_only = Vec::new();
        for aor in aors {
            let before = self.presence.get_presence_for_tenant(&aor, tenant);
            let (old_state, old_note) = before
                .map(|b| (b.state, b.note))
                .unwrap_or((PresenceState::Closed, None));
            if old_state == state && old_note == note {
                continue;
            }
            let update = PresenceUpdate {
                aor: aor.clone(),
                state,
                note: note.clone(),
            };
            if let Err(e) = self.presence.update_presence(update, tenant).await {
                warn!(aor = %aor, error = %e, "A person's presence was not laid on an address");
                continue;
            }
            if old_state == state {
                note_only.push(aor);
            }
        }
        note_only
    }

    /// Before a phone is first told about `aor`: lay its person's presence
    /// on it if nothing has yet. What the admin announced since this node
    /// started is used; else it is read from the database, without calls.
    pub async fn seed(&self, aor: &str, tenant: &TenantId) {
        if self.presence.get_presence_for_tenant(aor, tenant).is_some() {
            return;
        }
        let dialed = name_of(aor);
        let Some(user_id) = crate::ring_plan::user_of(&self.storage, tenant, &dialed).await else {
            return;
        };
        let Ok(Some(user)) = self.storage.get_user_by_id(&user_id).await else {
            return;
        };
        let mut names = vec![user.sip_name().to_lowercase()];
        if let Some(ext) = &user.extension {
            names.push(ext.to_lowercase());
        }
        let known = names.iter().find_map(|n| self.shown.get(tenant, n));
        let presence = match known {
            Some(p) => p,
            None => self.read(tenant, &user).await,
        };
        debug!(aor = %aor, state = presence.state.as_str(), "A watched person's presence laid");
        self.lay(tenant, &names, &presence, Some(aor)).await;
    }

    /// A person's presence from the database, as the admin works it out,
    /// without calls or idleness.
    async fn read(&self, tenant: &TenantId, user: &fcp_core::storage::User) -> Presence {
        if !user.may_use_services() {
            return effective(&Resolved::available(), Happening::default());
        }
        let now = SystemTime::now();
        let resolved = match self.storage.presence_status_store() {
            Some(store) => {
                let chosen = store.get_chosen_status(&user.id).await.unwrap_or(None);
                let own = match &chosen {
                    Some(c) if StatusState::parse(&c.status).is_none() => {
                        store.list_own_statuses(&user.id).await.unwrap_or_default()
                    }
                    _ => Vec::new(),
                };
                Resolved::of(chosen.as_ref(), &own, now)
            }
            None => Resolved::available(),
        };
        let dnd = self
            .storage
            .get_calling_settings(tenant, &user.id)
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s.dnd);

        let mut names: HashSet<String> = HashSet::from([user.sip_name().to_lowercase()]);
        if let Some(ext) = &user.extension {
            names.insert(ext.to_lowercase());
        }
        for d in self
            .storage
            .list_devices_by_user(&user.id)
            .await
            .unwrap_or_default()
        {
            for name in [d.sip_username.as_deref(), d.aor_user()]
                .into_iter()
                .flatten()
            {
                names.insert(name.to_lowercase());
            }
        }
        let registered = self
            .storage
            .get_all_registrations_in_tenant(tenant)
            .await
            .unwrap_or_default()
            .iter()
            .any(|r| r.expires_at > now && names.contains(&name_of(&r.aor)));

        effective(
            &resolved,
            Happening {
                registered,
                dnd,
                ..Happening::default()
            },
        )
    }

    /// Whether the member with this extension (or SIP name) is in do not
    /// disturb or on a call ([`PeopleShown::unavailable`]).
    pub fn unavailable(&self, tenant: &TenantId, extension: &str) -> bool {
        self.shown.unavailable(tenant, extension)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(state: Shown, label: Option<&str>, note: Option<&str>, idle: bool) -> Presence {
        Presence {
            state,
            label: label.map(String::from),
            note: note.map(String::from),
            idle,
        }
    }

    #[test]
    fn a_persons_presence_as_pidf() {
        let p = |s, l, n, i| pidf_of(&shown(s, l, n, i));
        assert_eq!(
            p(Shown::Offline, Some("WFH"), None, false),
            (PresenceState::Closed, None)
        );
        assert_eq!(
            p(Shown::Available, Some("WFH"), None, false),
            (PresenceState::Open, Some("WFH".into()))
        );
        assert_eq!(
            p(Shown::Available, None, None, false),
            (PresenceState::Open, None)
        );
        assert_eq!(
            p(Shown::Away, Some("At lunch"), Some("back at 2"), false),
            (PresenceState::Away, Some("At lunch — back at 2".into()))
        );
        assert_eq!(
            p(Shown::Away, None, None, true),
            (PresenceState::Away, Some("Idle".into()))
        );
        assert_eq!(
            p(Shown::Dnd, None, None, false),
            (PresenceState::Busy, Some("Do not disturb".into()))
        );
        assert_eq!(
            p(Shown::Dnd, Some("Focus"), None, false),
            (PresenceState::Busy, Some("Focus".into()))
        );
        assert_eq!(
            p(Shown::Ringing, Some("WFH"), None, false),
            (PresenceState::Ringing, None)
        );
        assert_eq!(
            p(Shown::OnCall, Some("WFH"), None, false),
            (PresenceState::OnThePhone, Some("On the phone · WFH".into()))
        );
        assert_eq!(
            p(Shown::OnCall, None, None, false),
            (PresenceState::OnThePhone, None)
        );
    }

    #[test]
    fn only_do_not_disturb_and_a_call_make_a_member_unavailable() {
        let people = PeopleShown::default();
        let tenant = TenantId::new();
        let say = |state| {
            people.remember(
                &tenant,
                &["dana".into(), "2001".into()],
                &shown(state, Some("Focus"), None, false),
            )
        };
        assert!(!people.unavailable(&tenant, "2001"), "nobody heard of is available");
        for (state, unavailable) in [
            (Shown::Busy, false),
            (Shown::Away, false),
            (Shown::Available, false),
            (Shown::Ringing, false),
            (Shown::Offline, false),
            (Shown::Dnd, true),
            (Shown::OnCall, true),
        ] {
            say(state);
            assert_eq!(people.unavailable(&tenant, "2001"), unavailable, "{state:?}");
            assert_eq!(people.unavailable(&tenant, "Dana"), unavailable, "by name too");
            assert!(
                !people.unavailable(&TenantId::new(), "2001"),
                "2001 in another tenant is someone else"
            );
        }
        assert_eq!(people.get(&tenant, "DANA").unwrap().state, Shown::OnCall);
        assert!(people.get(&TenantId::new(), "dana").is_none());
    }

    #[test]
    fn the_name_of_an_address() {
        assert_eq!(name_of("sip:Bob@globex.example"), "bob");
        assert_eq!(name_of("<sips:2102@10.0.0.1:5061;transport=tls>"), "2102");
        assert_eq!(name_of("alice@example.com"), "alice");
    }
}
