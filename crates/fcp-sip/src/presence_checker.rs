//! Presence-aware agent availability checker for queue distribution.
//!
//! Bridges what this node knows of people's presence
//! ([`PeopleOnPhones`]) with `fcp_queue::PresenceChecker`, so the queue
//! skips an agent in do not disturb or on a call (`docs/PRESENCE.md` §2):
//! not one who chose Busy or Away, and not one who is offline, whose ring
//! attempt fails on its own.

use crate::person_presence::PeopleOnPhones;
use std::sync::Arc;
use tracing::debug;

/// Lets the queue manager ask whether an agent can be offered a call.
pub struct PresenceCheckerAdapter {
    people: Arc<PeopleOnPhones>,
}

impl PresenceCheckerAdapter {
    pub fn new(people: Arc<PeopleOnPhones>) -> Self {
        Self { people }
    }
}

impl fcp_queue::PresenceChecker for PresenceCheckerAdapter {
    fn is_dnd(&self, agent_extension: &str) -> bool {
        let unavailable = self.people.unavailable(agent_extension);
        if unavailable {
            debug!(
                agent = %agent_extension,
                "Agent filtered from queue distribution: do not disturb or on a call"
            );
        }
        unavailable
    }
}
