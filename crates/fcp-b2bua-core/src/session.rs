//! B2BUA session and unified configuration.
//!
//! Combines all B2BUA policy modules (topology hiding, media anchoring,
//! forking, transfer) into a single configuration struct, and provides
//! a lightweight session type that associates a config with a call.
//!
//! `B2buaConfig` is the single entry point for configuring a B2BUA instance.
//! `B2buaSession` is a config holder for a specific call — it does **not**
//! contain async actors or SIP message handling logic.

use serde::{Deserialize, Serialize};

use crate::call::CallId;
use crate::forking::ForkingConfig;
use crate::media_anchoring::MediaAnchoringConfig;
use crate::topology::TopologyHidingConfig;
use crate::transfer::TransferConfig;

/// Unified B2BUA configuration.
///
/// Aggregates all sub-configurations that control B2BUA behavior. Each
/// service type (SBC, PBX, Session Manager) customizes these via the
/// named constructors on each sub-config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct B2buaConfig {
    /// Topology hiding settings.
    pub topology_hiding: TopologyHidingConfig,

    /// Media anchoring policy.
    pub media_anchoring: MediaAnchoringConfig,

    /// Call forking policy.
    pub forking: ForkingConfig,

    /// Call transfer policy.
    pub transfer: TransferConfig,

    /// Max-Forwards value for outbound requests (RFC 3261 §8.1.1.6).
    pub max_forwards: u8,

    /// Whether to relay provisional responses (1xx) to the caller.
    pub relay_provisional_responses: bool,

    /// Whether to insert Record-Route headers for mid-dialog routing.
    pub insert_record_route: bool,
}

impl Default for B2buaConfig {
    fn default() -> Self {
        Self {
            topology_hiding: TopologyHidingConfig::default(),
            media_anchoring: MediaAnchoringConfig::default(),
            forking: ForkingConfig::default(),
            transfer: TransferConfig::default(),
            max_forwards: 70,
            relay_provisional_responses: true,
            insert_record_route: true,
        }
    }
}

/// A B2BUA session associated with a specific call.
///
/// This is a lightweight config holder — not an async actor. SIP message
/// handling remains in `handler.rs`; this struct captures the policy
/// configuration and minimal state for a call.
#[derive(Debug, Clone)]
pub struct B2buaSession {
    /// The call this session is associated with.
    pub call_id: CallId,

    /// Configuration for this session.
    pub config: B2buaConfig,

    /// Number of currently active B-legs.
    pub active_b_leg_count: usize,

    /// Whether the call has been established (200 OK exchanged).
    pub established: bool,
}

impl B2buaSession {
    /// Create a new session with default configuration.
    pub fn new(call_id: CallId) -> Self {
        Self {
            call_id,
            config: B2buaConfig::default(),
            active_b_leg_count: 0,
            established: false,
        }
    }

    /// Create a new session with custom configuration.
    pub fn with_config(call_id: CallId, config: B2buaConfig) -> Self {
        Self {
            call_id,
            config,
            active_b_leg_count: 0,
            established: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_b2bua_config_default() {
        let cfg = B2buaConfig::default();
        assert_eq!(cfg.max_forwards, 70);
        assert!(cfg.relay_provisional_responses);
        assert!(cfg.insert_record_route);
        // Sub-configs should also be at their defaults
        assert_eq!(cfg.topology_hiding, TopologyHidingConfig::default());
        assert_eq!(cfg.media_anchoring, MediaAnchoringConfig::default());
        assert_eq!(cfg.forking, ForkingConfig::default());
        assert_eq!(cfg.transfer, TransferConfig::default());
    }

    #[test]
    fn test_session_new() {
        let call_id = CallId::new();
        let session = B2buaSession::new(call_id);
        assert_eq!(session.call_id, call_id);
        assert_eq!(session.active_b_leg_count, 0);
        assert!(!session.established);
        assert_eq!(session.config, B2buaConfig::default());
    }

    #[test]
    fn test_session_with_config() {
        let call_id = CallId::new();
        let config = B2buaConfig {
            max_forwards: 50,
            relay_provisional_responses: false,
            insert_record_route: false,
            topology_hiding: TopologyHidingConfig::sbc_defaults(),
            media_anchoring: MediaAnchoringConfig::sbc_defaults(),
            forking: ForkingConfig::sbc_defaults(),
            transfer: TransferConfig::sbc_defaults(),
        };
        let session = B2buaSession::with_config(call_id, config.clone());
        assert_eq!(session.call_id, call_id);
        assert_eq!(session.config, config);
        assert_eq!(session.active_b_leg_count, 0);
        assert!(!session.established);
    }

    #[test]
    fn test_session_state_mutation() {
        let mut session = B2buaSession::new(CallId::new());
        assert!(!session.established);
        assert_eq!(session.active_b_leg_count, 0);

        session.established = true;
        session.active_b_leg_count = 2;

        assert!(session.established);
        assert_eq!(session.active_b_leg_count, 2);
    }

    #[test]
    fn test_b2bua_config_serde_roundtrip() {
        let cfg = B2buaConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: B2buaConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_b2bua_config_custom_serde_roundtrip() {
        let cfg = B2buaConfig {
            max_forwards: 50,
            relay_provisional_responses: false,
            insert_record_route: false,
            topology_hiding: TopologyHidingConfig::sbc_defaults(),
            media_anchoring: MediaAnchoringConfig::sbc_defaults(),
            forking: ForkingConfig::pbx_parallel(),
            transfer: TransferConfig::sbc_defaults(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: B2buaConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
