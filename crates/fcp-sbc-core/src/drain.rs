//! Telling the cluster that this node is leaving
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
//!
//! The drain sequence is `fcp_sip::drain`, and what it advertises with is
//! `fcp_sip::drain::DrainAnnouncer`. `fcp-sip` does not depend on
//! `fcp-cluster`, so the adapter over `ClusterManager` lives here, where
//! `ClusterOwnership` already does — one implementation for every binary
//! that has both a cluster manager and a drain.

use async_trait::async_trait;
use fcp_sip::drain::DrainAnnouncer;
use std::sync::Arc;
use tracing::warn;

/// This node's heartbeat, as the drain's announcer.
pub struct ClusterAnnouncer {
    cluster: Arc<fcp_cluster::ClusterManager>,
}

impl ClusterAnnouncer {
    pub fn new(cluster: Arc<fcp_cluster::ClusterManager>) -> Self {
        Self { cluster }
    }

    /// The announcer for a node that may or may not be in a cluster.
    pub fn maybe(
        cluster: Option<Arc<fcp_cluster::ClusterManager>>,
    ) -> Option<Arc<dyn DrainAnnouncer>> {
        cluster.map(|cm| Arc::new(Self::new(cm)) as Arc<dyn DrainAnnouncer>)
    }
}

#[async_trait]
impl DrainAnnouncer for ClusterAnnouncer {
    async fn set_draining(&self, draining: bool) {
        if let Err(e) = self.cluster.set_draining(draining).await {
            warn!(error = %e, "Could not advertise draining to the cluster");
        }
    }
}
