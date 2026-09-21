//! What a drain means to a presence node: the watchers subscribed to it
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
//!
//! The sequence itself — refuse, wait, end, or call the whole thing off —
//! is [`fcp_sip::drain`], which every routing binary runs (§13, 2b-2). What
//! is here is what "what is left" means to a presence server: no call to
//! hang up, but a subscription each, which this node has been notifying.
//!
//! A draining node grants short subscriptions, from
//! `[admission] drain_register_expiry_secs`, so each watcher refreshes
//! within the minute and is placed on a node that is serving. The wait
//! watches them go. Whatever is still subscribed at the deadline gets the
//! terminated NOTIFY with `reason=probation` that this binary has always
//! sent at shutdown — which tells a watcher to subscribe again shortly,
//! rather than that its presentity is gone.

use async_trait::async_trait;
use fcp_sip::drain::{DrainAnnouncer, DrainTarget};
use fcp_sip::presence::{PresenceHandler, SipPresenceHandler};
use std::sync::Arc;
use tracing::warn;

/// The watchers this node is notifying.
pub struct Subscriptions {
    manager: fcp_presence::PresenceManager,
    handler: Arc<SipPresenceHandler>,
    announcer: Option<Arc<dyn DrainAnnouncer>>,
}

impl Subscriptions {
    pub fn new(
        manager: fcp_presence::PresenceManager,
        handler: Arc<SipPresenceHandler>,
        announcer: Option<Arc<dyn DrainAnnouncer>>,
    ) -> Self {
        Self {
            manager,
            handler,
            announcer,
        }
    }
}

#[async_trait]
impl DrainTarget for Subscriptions {
    /// Every watcher, the watcher-info ones included: each is a dialog
    /// this node owns and would stop notifying if it vanished.
    async fn carrying(&self) -> usize {
        self.manager.get_all_watchers_including_winfo().len()
    }

    /// Tell every watcher still here that its subscription is over and to
    /// come back — `terminated;reason=probation`, which is what a watcher
    /// needs to resubscribe somewhere that is serving.
    async fn end_remaining(&self) -> usize {
        let watchers = self.manager.get_all_watchers_including_winfo();
        if watchers.is_empty() {
            return 0;
        }
        if let Err(e) = self
            .handler
            .send_terminated_notifies(&watchers, "probation")
            .await
        {
            warn!(error = %e, "Could not tell every watcher the node is leaving");
        }
        watchers.len()
    }

    async fn advertise(&self, draining: bool) {
        if let Some(announcer) = &self.announcer {
            announcer.set_draining(draining).await;
        }
    }
}
