//! What a drain means to a conference node: the callers in its rooms
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
//!
//! This node is the one that had a drain before the platform did
//! (`docs/DISTRIBUTED_CONFERENCING.md` phase 5), and it keeps the part
//! that is its own: before waiting for anyone, it asks the holder of each
//! caller's leg to move that caller to another node, so a room survives a
//! restart rather than merely outliving it. What it no longer keeps is
//! the sequence around that, which is now [`fcp_sip::drain`] like every
//! other binary's — which is what makes `POST /drain` do the same thing
//! SIGTERM does, `/ready` answer `503`, and `DELETE /drain` put the node
//! back in service.
//!
//! Whoever is not moved — no other node has room, a leg's holder refuses
//! the REFER, a caller arrives after the migration — is waited for until
//! `drain_wait_secs` and then hung up with a `Reason` that says why.

use crate::conference_handler::ConferenceCallHandler;
use crate::migration;
use async_trait::async_trait;
use fcp_sip::drain::{DrainAnnouncer, DrainTarget};
use std::sync::Arc;
use std::time::Duration;
#[cfg(feature = "storage-redis")]
use std::time::Instant;
use tracing::{info, warn};

/// Where this node's callers can be moved, when it is in a cluster with a
/// `cluster.secret` to mint migration tickets under.
#[cfg(feature = "storage-redis")]
pub struct Migration {
    pub registry: fcp_cluster::ConferenceRegistry,
    pub members: Arc<dashmap::DashMap<String, fcp_cluster::ClusterMember>>,
    pub local_region: Option<String>,
    pub timeout: Duration,
}

/// The callers this node is carrying.
pub struct Callers {
    handler: Arc<ConferenceCallHandler>,
    #[cfg(feature = "storage-redis")]
    migration: Option<Migration>,
    announcer: Option<Arc<dyn DrainAnnouncer>>,
}

impl Callers {
    pub fn new(handler: Arc<ConferenceCallHandler>) -> Self {
        Self {
            handler,
            #[cfg(feature = "storage-redis")]
            migration: None,
            announcer: None,
        }
    }

    /// Where the callers can go, and how the cluster is told.
    #[cfg(feature = "storage-redis")]
    pub fn with_cluster(
        mut self,
        migration: Option<Migration>,
        announcer: Option<Arc<dyn DrainAnnouncer>>,
    ) -> Self {
        self.migration = migration;
        self.announcer = announcer;
        self
    }
}

#[async_trait]
impl DrainTarget for Callers {
    /// The callers in this node's rooms: trunks and the virtual
    /// participants of a cascade are not people waiting to be let go.
    async fn carrying(&self) -> usize {
        self.handler.node_callers()
    }

    /// Move every caller that can be moved before anyone is waited for
    /// (`docs/DISTRIBUTED_CONFERENCING.md` phase 5). A node with no
    /// cluster, or no `cluster.secret`, has nowhere to send them and says
    /// so once.
    async fn on_drain_start(&self) {
        #[cfg(feature = "storage-redis")]
        {
            let Some(migration) = &self.migration else {
                if self.handler.session_count() > 0 {
                    info!("no cluster here; callers are not migrated off this node");
                }
                return;
            };
            if !self.handler.migration_enabled() {
                if self.handler.session_count() > 0 {
                    info!("cluster.secret is not set; callers are not migrated off this node");
                }
                return;
            }
            migrate_callers(
                &self.handler,
                &migration.registry,
                &migration.members,
                migration.local_region.as_deref(),
                migration.timeout,
            )
            .await;
        }
    }

    /// Hang up whoever is left, each told the node is draining, each with
    /// its record. The rooms themselves are torn down by the shutdown
    /// that follows — a drain is not a shutdown, and a node whose drain
    /// is called off keeps its rooms.
    async fn end_remaining(&self) -> usize {
        self.handler.hangup_callers_draining().await
    }

    async fn advertise(&self, draining: bool) {
        if let Some(announcer) = &self.announcer {
            announcer.set_draining(draining).await;
        }
    }
}

/// Ask the holder of every caller's leg to move the caller to another
/// conference node (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5): plan
/// with the placement rule, REFER a batch at a time, then wait for the
/// reports, up to `timeout`. Whoever is not moved by then stays for the
/// drain wait and the hang-up.
#[cfg(feature = "storage-redis")]
async fn migrate_callers(
    handler: &Arc<ConferenceCallHandler>,
    registry: &fcp_cluster::ConferenceRegistry,
    members: &Arc<dashmap::DashMap<String, fcp_cluster::ClusterMember>>,
    local_region: Option<&str>,
    timeout: Duration,
) {
    // Two passes: a caller admitted between the first snapshot and its
    // REFERs (an INVITE answered just before the drain) would otherwise
    // be hung up with the leftovers.
    for pass in 0..2 {
        let candidates = handler.migration_candidates().await;
        if candidates.is_empty() {
            return;
        }
        if pass > 0 {
            info!(
                callers = candidates.len(),
                "Draining: callers admitted since the first pass"
            );
        }
        migrate_once(
            handler,
            registry,
            members,
            local_region,
            timeout,
            candidates,
        )
        .await;
    }
}

/// One round of the above: plan, REFER, wait for the reports.
#[cfg(feature = "storage-redis")]
async fn migrate_once(
    handler: &Arc<ConferenceCallHandler>,
    registry: &fcp_cluster::ConferenceRegistry,
    members: &Arc<dashmap::DashMap<String, fcp_cluster::ClusterMember>>,
    local_region: Option<&str>,
    timeout: Duration,
    candidates: Vec<migration::Candidate>,
) {
    use futures::StreamExt;
    use std::collections::HashMap;

    let mut hosts = HashMap::new();
    for c in &candidates {
        let key = (c.tenant_id.to_string(), c.room_number.clone());
        if hosts.contains_key(&key) {
            continue;
        }
        match registry.hosts(&key.0, &key.1).await {
            Ok(h) => {
                hosts.insert(key, h);
            }
            Err(e) => {
                warn!(room = %c.room_number, error = %e, "Registry read failed; the room's hosts are unknown")
            }
        }
    }
    let members: Vec<fcp_cluster::ClusterMember> =
        members.iter().map(|e| e.value().clone()).collect();
    let plan = migration::plan(candidates, &hosts, &members, local_region);
    info!(
        callers = plan.assignments.len(),
        unplaced = plan.unplaced,
        "Draining: moving callers to other nodes"
    );
    if plan.assignments.is_empty() {
        return;
    }

    let referred = std::sync::atomic::AtomicUsize::new(0);
    futures::stream::iter(plan.assignments)
        .for_each_concurrent(migration::REFER_CONCURRENCY, |a| {
            let referred = &referred;
            async move {
                match handler.refer_to_node(&a.call_id, &a.target).await {
                    Ok(()) => {
                        referred.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(e) => warn!(
                        call_id = %a.call_id,
                        to = %a.target.node_id,
                        error = %e,
                        "Caller could not be referred; staying"
                    ),
                }
            }
        })
        .await;
    let referred = referred.into_inner();
    if referred == 0 {
        return;
    }

    // Wait for the reports.
    let deadline = Instant::now() + timeout;
    while handler.migrations_pending() > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let expired = handler.expire_migrations().len();
    // The legs reported moved are being dropped; give the BYEs a moment.
    tokio::time::sleep(Duration::from_millis(500)).await;
    info!(
        referred,
        unreported = expired,
        remaining = handler.session_count(),
        "Draining: migration finished"
    );
}
