//! Presence-aware agent availability checker for queue distribution.
//!
//! Extracted adapter that bridges `fcp_presence::PresenceManager` with
//! `fcp_queue::PresenceChecker` so the queue system can skip DND agents.

use fcp_presence::PresenceManager;
use tracing::debug;

/// Adapter that lets the queue manager check presence DND state.
///
/// Only explicit `Busy` (DND) causes an agent to be skipped.
/// `Closed` (offline) agents are NOT skipped — the ring attempt will
/// fail naturally, which is the correct behavior for hunt groups.
pub struct PresenceCheckerAdapter {
    presence: PresenceManager,
}

impl PresenceCheckerAdapter {
    /// Create a new adapter wrapping the given presence manager.
    pub fn new(presence: PresenceManager) -> Self {
        Self { presence }
    }
}

impl fcp_queue::PresenceChecker for PresenceCheckerAdapter {
    fn is_dnd(&self, agent_extension: &str) -> bool {
        for info in self.presence.get_all_presences() {
            let user_part = info.aor.split('@').next().unwrap_or("");
            if user_part == agent_extension {
                let is_dnd = info.state.is_dnd_state();
                if is_dnd {
                    debug!(
                        agent = %agent_extension,
                        state = %info.state,
                        "Agent filtered from queue distribution: DND"
                    );
                }
                return is_dnd;
            }
        }
        false
    }
}
