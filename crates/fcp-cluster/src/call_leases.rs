//! Counting a tenant's calls, and a trunk's, across the cluster
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §7).
//!
//! A tenant's `max_concurrent_calls` is a contract with the platform, not
//! with one node of it, and a trunk's channel limit belongs to the
//! carrier. Counted per node — which is what phase 1 did and said it did
//! — a two-node cluster lets a tenant carry twice its contract. So each
//! call takes a **lease** here when it is admitted and gives it back when
//! it ends, and the count every node reads is the cluster's.
//!
//! One sorted set per subject, scored by when the lease expires:
//!
//! ```text
//! fcp:leases:tenant:<tenant-id>   call-id → expires-at (unix seconds)
//! fcp:leases:trunk:<trunk-id>     call-id → expires-at
//! ```
//!
//! The score is what makes this safe to leave alone: a node that dies
//! mid-call cannot release its leases, so every read drops the members
//! whose time has passed before it counts. The lease outlives any
//! reasonable call (`ttl`, three hours by default) and every node
//! refreshes what it still holds on a tick, so a long call keeps its
//! place and a dead node's calls fade rather than counting for ever.
//!
//! Redis being unreachable must never stop a call: every method says what
//! went wrong and the caller falls back to its own count, which is what
//! phase 1 enforced. The tenant is then capped per node rather than
//! across the cluster — the old behaviour, not no behaviour.

use fcp_storage_redis::RedisStorage;
use redis::AsyncCommands;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;

/// How long a lease stands without being refreshed. Longer than any call
/// anyone means to make; a call that outlives it is refreshed by its
/// node's tick long before.
pub const DEFAULT_LEASE_TTL_SECS: u64 = 3 * 60 * 60;

/// What a lease is counted against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseSubject<'a> {
    /// A tenant's concurrent calls, wherever they are placed.
    Tenant(&'a str),
    /// A trunk's channels, wherever the calls using them run.
    Trunk(&'a str),
    /// A user's calls, wherever they are (`<tenant>:<user>`): what call
    /// waiting reads (`docs/ENTERPRISE_CALLING.md` §3.4).
    User(&'a str),
}

impl LeaseSubject<'_> {
    fn key_suffix(&self) -> String {
        match self {
            Self::Tenant(id) => format!("leases:tenant:{id}"),
            Self::Trunk(id) => format!("leases:trunk:{id}"),
            Self::User(id) => format!("leases:user:{id}"),
        }
    }
}

/// The cluster's count of calls per tenant and per trunk.
#[derive(Clone)]
pub struct CallLeases {
    storage: RedisStorage,
    ttl_secs: u64,
}

impl CallLeases {
    pub fn new(storage: RedisStorage) -> Self {
        Self {
            storage,
            ttl_secs: DEFAULT_LEASE_TTL_SECS,
        }
    }

    /// A lease lifetime of this node's choosing, for a test that cannot
    /// wait three hours.
    pub fn with_ttl(mut self, ttl_secs: u64) -> Self {
        self.ttl_secs = ttl_secs.max(1);
        self
    }

    fn key(&self, subject: LeaseSubject<'_>) -> String {
        self.storage.key(&subject.key_suffix())
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Take a lease for `call_id`, and say how many are held afterwards —
    /// this one included.
    ///
    /// One round trip: the expired members are dropped, this call is
    /// written with its expiry, the set is counted, and the whole set is
    /// given an expiry of its own so an idle tenant leaves nothing
    /// behind. Taking a lease twice for the same call is not an error and
    /// does not count twice, which is what makes a retried INVITE safe.
    pub async fn take(&self, subject: LeaseSubject<'_>, call_id: &str) -> anyhow::Result<u64> {
        let key = self.key(subject);
        let now = Self::now();
        let mut conn = self.storage.connection();
        let (_, _, held): (i64, i64, u64) = redis::pipe()
            .atomic()
            .zrembyscore(&key, 0, now as isize)
            .zadd(&key, call_id, (now + self.ttl_secs) as isize)
            .zcard(&key)
            .expire(&key, self.ttl_secs as i64)
            .ignore()
            .query_async(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("lease take: {e}"))?;
        debug!(key = %key, call_id = %call_id, held, "lease taken");
        Ok(held)
    }

    /// Give a lease back. A call that never held one is not an error.
    pub async fn release(&self, subject: LeaseSubject<'_>, call_id: &str) -> anyhow::Result<()> {
        let key = self.key(subject);
        let mut conn = self.storage.connection();
        let _: i64 = conn
            .zrem(&key, call_id)
            .await
            .map_err(|e| anyhow::anyhow!("lease release: {e}"))?;
        Ok(())
    }

    /// How many leases are held, expired ones excluded. Read-only: it
    /// does not prune, so a reader cannot tidy up after a writer.
    pub async fn held(&self, subject: LeaseSubject<'_>) -> anyhow::Result<u64> {
        let key = self.key(subject);
        let mut conn = self.storage.connection();
        let held: u64 = conn
            .zcount(&key, Self::now() as isize + 1, "+inf")
            .await
            .map_err(|e| anyhow::anyhow!("lease count: {e}"))?;
        Ok(held)
    }

    /// Put this node's live calls back where they belong and push their
    /// expiry out: what a node runs on a tick so a call longer than the
    /// lease keeps its place, and so a lease lost to a Redis outage comes
    /// back when Redis does.
    ///
    /// Leases held here for calls that have ended are not this method's
    /// business — [`release`](Self::release) has them — but a lease whose
    /// call this node has forgotten will lapse on its own.
    pub async fn refresh(
        &self,
        subject: LeaseSubject<'_>,
        call_ids: &[String],
    ) -> anyhow::Result<()> {
        if call_ids.is_empty() {
            return Ok(());
        }
        let key = self.key(subject);
        let expires = (Self::now() + self.ttl_secs) as isize;
        let mut conn = self.storage.connection();
        let mut pipe = redis::pipe();
        pipe.atomic();
        for call_id in call_ids {
            pipe.zadd(&key, call_id, expires).ignore();
        }
        pipe.expire(&key, self.ttl_secs as i64).ignore();
        pipe.query_async::<()>(&mut conn)
            .await
            .map_err(|e| anyhow::anyhow!("lease refresh: {e}"))?;
        Ok(())
    }
}
