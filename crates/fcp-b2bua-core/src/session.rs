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

fn default_max_extension_digits() -> u8 {
    6
}
use crate::topology::TopologyHidingConfig;
use crate::transfer::TransferConfig;

/// RFC 4028 session timer enforcement configuration.
///
/// When enabled, the B2BUA forces session timers on all confirmed dialogs
/// even if neither endpoint negotiated them. The SBC acts as refresher and
/// periodically re-INVITEs/UPDATEs both legs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTimerConfig {
    /// Enable session timer enforcement. When true, all confirmed dialogs
    /// get a session timer even if neither endpoint negotiated one.
    #[serde(default)]
    pub enabled: bool,

    /// Session interval in seconds. Both legs are refreshed at interval/2.
    /// Range: 90–86400. Default: 1800 (30 minutes).
    #[serde(default = "default_session_expires_secs")]
    pub session_expires_secs: u32,

    /// Minimum acceptable Session-Expires from remote endpoints (Min-SE).
    /// INVITEs with Session-Expires below this value are rejected with 422.
    /// Range: 90–7200. Default: 90.
    #[serde(default = "default_min_se_secs")]
    pub min_se_secs: u32,
}

fn default_session_expires_secs() -> u32 {
    1800
}

fn default_min_se_secs() -> u32 {
    90
}

impl Default for SessionTimerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            session_expires_secs: default_session_expires_secs(),
            min_se_secs: default_min_se_secs(),
        }
    }
}

impl SessionTimerConfig {
    /// Validate the session timer configuration.
    ///
    /// - `session_expires_secs` must be in 90..=86400
    /// - `min_se_secs` must be in 90..=7200
    /// - `session_expires_secs` must be >= `min_se_secs`
    pub fn validate(&self) -> Result<(), String> {
        if self.session_expires_secs < 90 || self.session_expires_secs > 86400 {
            return Err(format!(
                "session_expires_secs must be 90..=86400, got {}",
                self.session_expires_secs
            ));
        }
        if self.min_se_secs < 90 || self.min_se_secs > 7200 {
            return Err(format!(
                "min_se_secs must be 90..=7200, got {}",
                self.min_se_secs
            ));
        }
        if self.session_expires_secs < self.min_se_secs {
            return Err(format!(
                "session_expires_secs ({}) must be >= min_se_secs ({})",
                self.session_expires_secs, self.min_se_secs
            ));
        }
        Ok(())
    }
}

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

    /// RFC 4028 session timer enforcement.
    #[serde(default)]
    pub session_timer: SessionTimerConfig,

    /// Maximum number of digits treated as a short extension rather than a
    /// phone number.  Dialed strings with this many digits or fewer are
    /// routed as extensions; longer all-digit strings are normalized to
    /// E.164.  Default is 6 (i.e. up to 6-digit extensions; 7+ digits
    /// trigger phone-number normalization).
    #[serde(default = "default_max_extension_digits")]
    pub max_extension_digits: u8,
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
            session_timer: SessionTimerConfig::default(),
            max_extension_digits: default_max_extension_digits(),
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
        assert_eq!(cfg.session_timer, SessionTimerConfig::default());
        assert!(!cfg.session_timer.enabled);
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
            session_timer: SessionTimerConfig::default(),
            max_extension_digits: 6,
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
            session_timer: SessionTimerConfig {
                enabled: true,
                session_expires_secs: 900,
                min_se_secs: 120,
            },
            max_extension_digits: 6,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: B2buaConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_session_timer_config_defaults() {
        let cfg = SessionTimerConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.session_expires_secs, 1800);
        assert_eq!(cfg.min_se_secs, 90);
    }

    #[test]
    fn test_session_timer_config_validate_valid() {
        let cfg = SessionTimerConfig::default();
        assert!(cfg.validate().is_ok());

        let cfg = SessionTimerConfig {
            enabled: true,
            session_expires_secs: 1800,
            min_se_secs: 90,
        };
        assert!(cfg.validate().is_ok());

        // Edge cases: minimum valid values
        let cfg = SessionTimerConfig {
            enabled: true,
            session_expires_secs: 90,
            min_se_secs: 90,
        };
        assert!(cfg.validate().is_ok());

        // Edge cases: maximum valid values
        let cfg = SessionTimerConfig {
            enabled: true,
            session_expires_secs: 86400,
            min_se_secs: 7200,
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_session_timer_config_validate_session_expires_out_of_range() {
        let cfg = SessionTimerConfig {
            enabled: true,
            session_expires_secs: 89,
            min_se_secs: 90,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("session_expires_secs"));

        let cfg = SessionTimerConfig {
            enabled: true,
            session_expires_secs: 86401,
            min_se_secs: 90,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("session_expires_secs"));
    }

    #[test]
    fn test_session_timer_config_validate_min_se_out_of_range() {
        let cfg = SessionTimerConfig {
            enabled: true,
            session_expires_secs: 1800,
            min_se_secs: 89,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("min_se_secs"));

        let cfg = SessionTimerConfig {
            enabled: true,
            session_expires_secs: 7201,
            min_se_secs: 7201,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("min_se_secs"));
    }

    #[test]
    fn test_session_timer_config_validate_expires_less_than_min_se() {
        let cfg = SessionTimerConfig {
            enabled: true,
            session_expires_secs: 100,
            min_se_secs: 200,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("session_expires_secs"));
        assert!(err.contains("min_se_secs"));
    }

    #[test]
    fn test_session_timer_config_serde_roundtrip() {
        let cfg = SessionTimerConfig {
            enabled: true,
            session_expires_secs: 900,
            min_se_secs: 120,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SessionTimerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_session_timer_config_serde_defaults() {
        // When fields are missing in JSON, serde defaults should apply
        let json = "{}";
        let cfg: SessionTimerConfig = serde_json::from_str(json).unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.session_expires_secs, 1800);
        assert_eq!(cfg.min_se_secs, 90);
    }
}
