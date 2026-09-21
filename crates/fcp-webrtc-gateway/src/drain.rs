//! What a drain means to the gateway: the browsers it is carrying
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
//!
//! The sequence itself — refuse, wait, end, or call the whole thing off —
//! is [`fcp_sip::drain`], which every routing binary runs (§13, 2b-3).
//! What is here is the gateway's own two pieces: what "what is left"
//! means, and where a browser is told to go.
//!
//! A gateway session is anchored in this process — a peer connection, a
//! SIP leg and the relay between them — so unlike a conference
//! participant it cannot be moved by the platform: the browser has to
//! join again somewhere else. So a draining gateway does both things at
//! once. It refuses new sessions, and it tells every browser in a room,
//! through the status it already polls, to join another node — named from
//! the cluster ([`ClusterHandover`]), or left unnamed, which means "come
//! back where you are" and is what a load balancer in front of the pool
//! answers. A call to a SIP phone is told nothing: the far end is a phone
//! on another network, and redialling it would be a second call, so those
//! are waited for and hung up at the deadline like any other call.

use crate::session::{HandoverAdvice, SessionManager, SessionState};
use async_trait::async_trait;
use fcp_sip::drain::{DrainAnnouncer, DrainTarget};
use std::sync::Arc;
use tracing::warn;

/// The browsers this node is carrying.
pub struct GatewaySessions {
    sessions: Arc<SessionManager>,
    announcer: Option<Arc<dyn DrainAnnouncer>>,
}

impl GatewaySessions {
    pub fn new(sessions: Arc<SessionManager>, announcer: Option<Arc<dyn DrainAnnouncer>>) -> Self {
        Self {
            sessions,
            announcer,
        }
    }
}

#[async_trait]
impl DrainTarget for GatewaySessions {
    async fn carrying(&self) -> usize {
        self.sessions.live_count().await
    }

    /// End every session still here: a BYE to the far end — the
    /// conference node or the phone — and the browser's peer connection
    /// closed, each with the record the gateway writes for a session that
    /// ends. This is `DELETE /v1/rooms/{id}` and `/v1/calls/{id}` by
    /// another name.
    async fn end_remaining(&self) -> usize {
        let mut ended = 0;
        for info in self
            .sessions
            .list(true)
            .await
            .into_iter()
            .chain(self.sessions.list(false).await)
            // A session that is over lingers in the list for a minute so
            // a browser can read why; there is nothing to end there.
            .filter(|info| {
                !matches!(
                    info.state,
                    SessionState::Terminated | SessionState::Failed(_)
                )
            })
        {
            match self.sessions.hangup(&info.id).await {
                Ok(()) => ended += 1,
                // One that ended while the drain was reading the list is
                // not an error; anything else is worth saying.
                Err(e) => warn!(session = %info.id, error = %e, "could not end a session on drain"),
            }
        }
        ended
    }

    async fn advertise(&self, draining: bool) {
        if let Some(announcer) = &self.announcer {
            announcer.set_draining(draining).await;
        }
    }
}

/// Where a browser goes when this node is leaving: the least loaded
/// serving gateway in this node's region that advertises a browser-facing
/// address (`[http] public_url`), read from the cluster each time it is
/// asked, since a peer may itself start draining
/// (`fcp_cluster::choose_handover_node`).
#[cfg(feature = "storage-redis")]
pub struct ClusterHandover {
    cluster: Arc<fcp_cluster::ClusterManager>,
}

#[cfg(feature = "storage-redis")]
impl ClusterHandover {
    pub fn new(cluster: Arc<fcp_cluster::ClusterManager>) -> Self {
        Self { cluster }
    }
}

#[cfg(feature = "storage-redis")]
impl HandoverAdvice for ClusterHandover {
    fn peer_url(&self) -> Option<String> {
        self.cluster
            .handover_peer()
            .and_then(|peer| peer.public_url)
    }
}

/// A gateway with no cluster: there is no peer to name, so a browser is
/// told to come back where it is.
pub struct NoHandover;

impl HandoverAdvice for NoHandover {
    fn peer_url(&self) -> Option<String> {
        None
    }
}
