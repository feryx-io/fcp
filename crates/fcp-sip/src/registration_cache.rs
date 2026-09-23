//! A short-lived memory of registration lookups
//! (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1, phase 2).
//!
//! Registrations are read from the primary, and every other store a call
//! reads is either replica-served or cached. This gives them the cache the
//! credential, extension and device stores already have: the rows the store
//! returned for a tenant and address of record, kept for a TTL.
//!
//! What keeps it honest:
//!
//! - **This node's own writes** invalidate the entry as soon as the store
//!   has them, so a REGISTER, a re-REGISTER from a new address or a
//!   deregistration handled here is seen by the next call here.
//! - **Every other node's writes** reach it as registration events on the
//!   bus, which the binary turns into [`RegistrationCache::invalidate`].
//! - **The TTL** bounds whatever neither of those covers: an event lost on
//!   the way, a writer that publishes none.
//!
//! An empty answer is cached like any other. Most of what a domain alias
//! costs is empty answers — the same user looked up at domains they did not
//! register under — so a cache that skipped them would save little of it.
//!
//! A lookup that raced a write must not put the old rows back after the
//! write invalidated them. Each entry carries a generation: an invalidation
//! leaves a marker with a new one rather than removing the entry, and a fill
//! lands only if the generation it saw before its read is still there. A
//! marker outlives the TTL, and a read older than the TTL is not kept, so
//! no fill can outlast the marker that should stop it.

use dashmap::DashMap;
use fcp_core::{CacheStats, Registration, TenantId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

type Key = (TenantId, String);

#[derive(Clone)]
enum State {
    /// What the store returned, expired rows and all: expiry is judged at
    /// each read, not at the fill.
    Rows(Vec<Registration>),
    /// Invalidated: the next lookup reads the store, and a fill from a read
    /// that began before this does not land.
    Invalidated,
}

#[derive(Clone)]
struct Slot {
    state: State,
    generation: u64,
    at: Instant,
}

/// What a lookup that missed saw, so its fill can tell whether a write came
/// in while it was reading.
#[derive(Debug, Clone, Copy)]
pub struct Ticket {
    /// The entry's generation at the lookup; zero when there was none.
    generation: u64,
    at: Instant,
}

/// A lookup's outcome.
pub enum Lookup {
    /// The rows as the store last returned them — expired ones included.
    Hit(Vec<Registration>),
    /// Read the store, then [`RegistrationCache::fill`] with this.
    Miss(Ticket),
}

/// Registration rows by tenant and address of record, for a TTL.
pub struct RegistrationCache {
    ttl: Duration,
    max_entries: usize,
    slots: DashMap<Key, Slot>,
    /// When an address was last invalidated in every tenant at once. That
    /// reaches tenants with nothing held yet, whose reads a per-entry
    /// marker could not stop.
    everywhere: DashMap<String, Instant>,
    generation: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl RegistrationCache {
    /// Entries held before the oldest are dropped — the other hot-path
    /// caches' bound.
    pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

    /// A cache keeping each answer for `ttl`. A zero TTL is no cache; the
    /// caller does not build one.
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            max_entries,
            slots: DashMap::new(),
            everywhere: DashMap::new(),
            generation: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// How long an answer is kept.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// The rows for `aor` in `tenant_id`, when an answer is held and fresh.
    pub fn lookup(&self, tenant_id: &TenantId, aor: &str) -> Lookup {
        let key = (*tenant_id, aor.to_string());
        let generation = match self.slots.get(&key) {
            Some(slot) => {
                if let State::Rows(rows) = &slot.state {
                    if slot.at.elapsed() < self.ttl {
                        self.hits.fetch_add(1, Ordering::Relaxed);
                        return Lookup::Hit(rows.clone());
                    }
                }
                slot.generation
            }
            None => 0,
        };
        self.misses.fetch_add(1, Ordering::Relaxed);
        Lookup::Miss(Ticket {
            generation,
            at: Instant::now(),
        })
    }

    /// Keep what the store returned for a lookup that missed — unless the
    /// entry was invalidated, or filled by another lookup, while this one
    /// was reading, or the read took longer than the TTL.
    pub fn fill(&self, tenant_id: &TenantId, aor: &str, rows: &[Registration], ticket: Ticket) {
        if ticket.at.elapsed() >= self.ttl {
            return;
        }
        if let Some(at) = self.everywhere.get(aor) {
            if *at >= ticket.at {
                return;
            }
        }
        let key = (*tenant_id, aor.to_string());
        let slot = Slot {
            state: State::Rows(rows.to_vec()),
            generation: self.next_generation(),
            at: Instant::now(),
        };
        match self.slots.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(mut held) => {
                if held.get().generation == ticket.generation {
                    held.insert(slot);
                }
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                // Nothing held then and nothing now. An invalidation in
                // between would have left a marker, and a marker lives
                // longer than this read did.
                if ticket.generation == 0 {
                    vacant.insert(slot);
                }
            }
        }
        self.enforce_capacity();
    }

    /// Forget `aor` in `tenant_id`: the store has changed under it.
    pub fn invalidate(&self, tenant_id: &TenantId, aor: &str) {
        let generation = self.next_generation();
        self.slots.insert(
            (*tenant_id, aor.to_string()),
            Slot {
                state: State::Invalidated,
                generation,
                at: Instant::now(),
            },
        );
        self.enforce_capacity();
    }

    /// Forget `aor` in every tenant — for a write that names no tenant.
    pub fn invalidate_everywhere(&self, aor: &str) {
        let now = Instant::now();
        let marker_life = self.ttl * 2;
        self.everywhere
            .retain(|_, at| now.duration_since(*at) < marker_life);
        self.everywhere.insert(aor.to_string(), now);
        let tenants: Vec<TenantId> = self
            .slots
            .iter()
            .filter(|slot| slot.key().1 == aor)
            .map(|slot| slot.key().0)
            .collect();
        for tenant_id in tenants {
            self.invalidate(&tenant_id, aor);
        }
    }

    /// Hits, misses, evictions and size, as the other caches report them.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            size: self.slots.len(),
            max_entries: self.max_entries,
        }
    }

    /// Past the bound, drop what has gone stale, then the oldest answers
    /// down to three quarters of it. A marker younger than the TTL is never
    /// dropped: it is what stops an older read from landing.
    fn enforce_capacity(&self) {
        if self.slots.len() <= self.max_entries {
            return;
        }
        let before = self.slots.len();
        let marker_life = self.ttl * 2;
        self.slots.retain(|_, slot| match slot.state {
            State::Rows(_) => slot.at.elapsed() < self.ttl,
            State::Invalidated => slot.at.elapsed() < marker_life,
        });
        if self.slots.len() > self.max_entries {
            let target = self.max_entries * 3 / 4;
            let mut answers: Vec<(Key, Instant)> = self
                .slots
                .iter()
                .filter(|slot| matches!(slot.state, State::Rows(_)))
                .map(|slot| (slot.key().clone(), slot.at))
                .collect();
            answers.sort_by_key(|(_, at)| *at);
            let excess = self.slots.len().saturating_sub(target);
            for (key, _) in answers.into_iter().take(excess) {
                self.slots.remove(&key);
            }
        }
        let evicted = before.saturating_sub(self.slots.len());
        if evicted > 0 {
            self.evictions.fetch_add(evicted as u64, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    const AOR: &str = "sip:bob@e2e.local";

    fn row(contact: &str) -> Registration {
        Registration {
            tenant_id: TenantId::nil(),
            aor: AOR.to_string(),
            contact: contact.to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            call_id: "c@x".to_string(),
            cseq: 1,
            user_agent: None,
            q: None,
            registered_at: SystemTime::now(),
            path: None,
            flow_token: None,
            source_address: None,
        }
    }

    fn contacts(lookup: Lookup) -> Option<Vec<String>> {
        match lookup {
            Lookup::Hit(rows) => Some(rows.into_iter().map(|r| r.contact).collect()),
            Lookup::Miss(_) => None,
        }
    }

    fn ticket(lookup: Lookup) -> Ticket {
        match lookup {
            Lookup::Miss(ticket) => ticket,
            Lookup::Hit(_) => panic!("expected a miss"),
        }
    }

    fn cache() -> RegistrationCache {
        RegistrationCache::new(Duration::from_secs(30), 100)
    }

    #[test]
    fn an_answer_is_kept_and_served() {
        let cache = cache();
        let t = TenantId::nil();
        let first = ticket(cache.lookup(&t, AOR));
        cache.fill(&t, AOR, &[row("sip:bob@10.0.0.1")], first);

        assert_eq!(
            contacts(cache.lookup(&t, AOR)),
            Some(vec!["sip:bob@10.0.0.1".to_string()])
        );
        let stats = cache.stats();
        assert_eq!((stats.hits, stats.misses, stats.size), (1, 1, 1));
    }

    #[test]
    fn an_empty_answer_is_kept_too() {
        // Most of what an alias costs is lookups that find nobody.
        let cache = cache();
        let t = TenantId::nil();
        let first = ticket(cache.lookup(&t, AOR));
        cache.fill(&t, AOR, &[], first);
        assert_eq!(contacts(cache.lookup(&t, AOR)), Some(vec![]));
    }

    #[test]
    fn tenants_and_addresses_are_kept_apart() {
        let cache = cache();
        let t = TenantId::nil();
        let other = TenantId::new();
        let first = ticket(cache.lookup(&t, AOR));
        cache.fill(&t, AOR, &[row("sip:bob@10.0.0.1")], first);

        assert!(contacts(cache.lookup(&other, AOR)).is_none());
        assert!(contacts(cache.lookup(&t, "sip:alice@e2e.local")).is_none());
    }

    #[test]
    fn an_answer_older_than_the_ttl_is_read_again() {
        let cache = RegistrationCache::new(Duration::from_millis(20), 100);
        let t = TenantId::nil();
        let first = ticket(cache.lookup(&t, AOR));
        cache.fill(&t, AOR, &[row("sip:bob@10.0.0.1")], first);
        std::thread::sleep(Duration::from_millis(30));

        // Stale, so a miss — and the fill after it replaces the stale rows.
        let again = ticket(cache.lookup(&t, AOR));
        cache.fill(&t, AOR, &[row("sip:bob@10.0.0.2")], again);
        assert_eq!(
            contacts(cache.lookup(&t, AOR)),
            Some(vec!["sip:bob@10.0.0.2".to_string()])
        );
    }

    #[test]
    fn an_invalidated_answer_is_read_again() {
        let cache = cache();
        let t = TenantId::nil();
        let first = ticket(cache.lookup(&t, AOR));
        cache.fill(&t, AOR, &[row("sip:bob@10.0.0.1")], first);

        cache.invalidate(&t, AOR);
        let again = ticket(cache.lookup(&t, AOR));
        cache.fill(&t, AOR, &[row("sip:bob@10.0.0.2")], again);
        assert_eq!(
            contacts(cache.lookup(&t, AOR)),
            Some(vec!["sip:bob@10.0.0.2".to_string()])
        );
    }

    #[test]
    fn a_read_that_raced_a_write_does_not_put_the_old_rows_back() {
        // A lookup misses and reads the store; before it returns, a
        // REGISTER moves the phone and invalidates. The read's answer is
        // from before the write, so it must not land.
        let cache = cache();
        let t = TenantId::nil();
        let racing = ticket(cache.lookup(&t, AOR));
        cache.invalidate(&t, AOR);
        cache.fill(&t, AOR, &[row("sip:bob@old")], racing);
        assert!(
            contacts(cache.lookup(&t, AOR)).is_none(),
            "the old rows landed after the invalidation"
        );

        // The same race against an entry that was already held.
        let first = ticket(cache.lookup(&t, AOR));
        cache.fill(&t, AOR, &[row("sip:bob@a")], first);
        cache.invalidate(&t, AOR);
        let racing = ticket(cache.lookup(&t, AOR));
        cache.invalidate(&t, AOR);
        cache.fill(&t, AOR, &[row("sip:bob@old")], racing);
        assert!(contacts(cache.lookup(&t, AOR)).is_none());
    }

    #[test]
    fn a_read_slower_than_the_ttl_is_not_kept() {
        let cache = RegistrationCache::new(Duration::from_millis(20), 100);
        let t = TenantId::nil();
        let slow = ticket(cache.lookup(&t, AOR));
        std::thread::sleep(Duration::from_millis(30));
        cache.fill(&t, AOR, &[row("sip:bob@10.0.0.1")], slow);
        assert_eq!(cache.stats().size, 0);
    }

    #[test]
    fn invalidating_everywhere_reaches_every_tenant() {
        let cache = cache();
        let (a, b) = (TenantId::nil(), TenantId::new());
        for t in [a, b] {
            let first = ticket(cache.lookup(&t, AOR));
            cache.fill(&t, AOR, &[row("sip:bob@10.0.0.1")], first);
        }
        cache.invalidate_everywhere(AOR);
        assert!(contacts(cache.lookup(&a, AOR)).is_none());
        assert!(contacts(cache.lookup(&b, AOR)).is_none());
    }

    #[test]
    fn invalidating_everywhere_stops_a_read_in_a_tenant_with_nothing_held() {
        let cache = cache();
        let t = TenantId::new();
        let racing = ticket(cache.lookup(&t, AOR));
        cache.invalidate_everywhere(AOR);
        cache.fill(&t, AOR, &[row("sip:bob@old")], racing);
        assert!(contacts(cache.lookup(&t, AOR)).is_none());

        // A read that began after it lands as usual.
        let after = ticket(cache.lookup(&t, AOR));
        cache.fill(&t, AOR, &[row("sip:bob@new")], after);
        assert!(contacts(cache.lookup(&t, AOR)).is_some());
    }

    #[test]
    fn the_bound_drops_the_oldest_answers_and_keeps_the_markers() {
        let cache = RegistrationCache::new(Duration::from_secs(30), 8);
        let t = TenantId::nil();
        cache.invalidate(&t, "sip:marked@e2e.local");
        for n in 0..12 {
            let aor = format!("sip:u{n}@e2e.local");
            let first = ticket(cache.lookup(&t, &aor));
            cache.fill(&t, &aor, &[], first);
        }
        let stats = cache.stats();
        assert!(stats.size <= 8, "{} entries held", stats.size);
        assert!(stats.evictions > 0);
        // The newest answer survived, the oldest did not.
        assert!(contacts(cache.lookup(&t, "sip:u11@e2e.local")).is_some());
        assert!(contacts(cache.lookup(&t, "sip:u0@e2e.local")).is_none());
        // And the marker is still there to stop a racing fill.
        let racing = Ticket {
            generation: 0,
            at: Instant::now(),
        };
        cache.fill(&t, "sip:marked@e2e.local", &[row("sip:x@old")], racing);
        assert!(contacts(cache.lookup(&t, "sip:marked@e2e.local")).is_none());
    }
}
