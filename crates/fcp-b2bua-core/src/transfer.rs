//! Call transfer policy configuration for B2BUA.
//!
//! Controls whether and how call transfers (REFER, Replaces) are handled
//! by the B2BUA, including rate limiting, loop detection, and policy
//! enforcement.
//!
//! - **SBC**: Allow transfers with moderate rate limiting
//! - **PBX**: Allow all transfer types (blind, attended, consultative)
//! - **Session Manager**: Deny transfers (routing only)

use serde::{Deserialize, Serialize};

/// Type of call transfer being attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransferType {
    /// Blind transfer: REFER without prior consultation.
    Blind,

    /// Attended transfer: consultation call established before REFER.
    Attended,

    /// Consultative transfer: similar to attended but with explicit
    /// consultation step and Replaces header.
    Consultative,
}

/// Policy governing which transfer types are permitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TransferPolicy {
    /// Allow all transfer types.
    #[default]
    Allow,

    /// Deny all transfers.
    Deny,

    /// Allow only blind transfers.
    BlindOnly,

    /// Allow only attended/consultative transfers.
    AttendedOnly,
}

/// Outcome of a transfer attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferResult {
    /// Transfer was accepted and is in progress.
    Accepted,

    /// Transfer was rejected by the remote party.
    Rejected {
        /// SIP status code from the rejection.
        status_code: u16,
        /// Reason phrase.
        reason: String,
    },

    /// Transfer failed due to a local error.
    Failed {
        /// Description of the failure.
        reason: String,
    },

    /// Transfer was denied by local policy before being attempted.
    PolicyDenied,
}

/// Policy-level configuration for call transfers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TransferConfig {
    /// Which transfer types are permitted.
    pub policy: TransferPolicy,

    /// Maximum number of transfers allowed per dialog.
    pub rate_limit_per_dialog: u32,

    /// Validate the Refer-To URI before forwarding.
    pub validate_refer_to: bool,

    /// Detect and prevent transfer loops (A→B→A).
    pub loop_detection: bool,

    /// Maximum depth of chained transfers (A→B→C→...).
    pub max_transfer_depth: u8,

    /// Send NOTIFY with transfer progress (sipfrag) back to the transferor.
    pub send_notify_progress: bool,

    /// Clean up the original dialog when a transfer fails.
    pub cleanup_on_failure: bool,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            policy: TransferPolicy::Allow,
            rate_limit_per_dialog: 3,
            validate_refer_to: true,
            loop_detection: true,
            max_transfer_depth: 5,
            send_notify_progress: true,
            cleanup_on_failure: true,
        }
    }
}

impl TransferConfig {
    /// SBC defaults: allow transfers with higher rate limit, shallower depth.
    pub fn sbc_defaults() -> Self {
        Self {
            policy: TransferPolicy::Allow,
            rate_limit_per_dialog: 5,
            validate_refer_to: true,
            loop_detection: true,
            max_transfer_depth: 3,
            send_notify_progress: true,
            cleanup_on_failure: true,
        }
    }

    /// PBX defaults: allow all transfers with standard limits.
    pub fn pbx_defaults() -> Self {
        Self::default()
    }

    /// Session Manager defaults: deny all transfers.
    pub fn session_manager_defaults() -> Self {
        Self {
            policy: TransferPolicy::Deny,
            rate_limit_per_dialog: 0,
            validate_refer_to: false,
            loop_detection: false,
            max_transfer_depth: 0,
            send_notify_progress: false,
            cleanup_on_failure: false,
        }
    }

    /// Check whether the given transfer type is allowed by policy.
    pub fn is_allowed(&self, transfer_type: TransferType) -> bool {
        match self.policy {
            TransferPolicy::Allow => true,
            TransferPolicy::Deny => false,
            TransferPolicy::BlindOnly => transfer_type == TransferType::Blind,
            TransferPolicy::AttendedOnly => matches!(
                transfer_type,
                TransferType::Attended | TransferType::Consultative
            ),
        }
    }

    /// Validate the configuration.
    ///
    /// Returns `Err` if:
    /// - `policy` is not `Deny` and `rate_limit_per_dialog` is zero
    /// - `policy` is not `Deny` and `max_transfer_depth` is zero or exceeds 20
    pub fn validate(&self) -> Result<(), String> {
        if self.policy == TransferPolicy::Deny {
            // No further validation needed when transfers are denied
            return Ok(());
        }
        if self.rate_limit_per_dialog == 0 {
            return Err(
                "rate_limit_per_dialog must be greater than zero when transfers are allowed".into(),
            );
        }
        if self.max_transfer_depth == 0 {
            return Err(
                "max_transfer_depth must be greater than zero when transfers are allowed".into(),
            );
        }
        if self.max_transfer_depth > 20 {
            return Err("max_transfer_depth must not exceed 20".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let cfg = TransferConfig::default();
        assert_eq!(cfg.policy, TransferPolicy::Allow);
        assert_eq!(cfg.rate_limit_per_dialog, 3);
        assert!(cfg.validate_refer_to);
        assert!(cfg.loop_detection);
        assert_eq!(cfg.max_transfer_depth, 5);
        assert!(cfg.send_notify_progress);
        assert!(cfg.cleanup_on_failure);
    }

    #[test]
    fn test_sbc_defaults() {
        let cfg = TransferConfig::sbc_defaults();
        assert_eq!(cfg.policy, TransferPolicy::Allow);
        assert_eq!(cfg.rate_limit_per_dialog, 5);
        assert_eq!(cfg.max_transfer_depth, 3);
    }

    #[test]
    fn test_pbx_defaults() {
        let cfg = TransferConfig::pbx_defaults();
        assert_eq!(cfg.policy, TransferPolicy::Allow);
        assert_eq!(cfg.rate_limit_per_dialog, 3);
    }

    #[test]
    fn test_session_manager_defaults() {
        let cfg = TransferConfig::session_manager_defaults();
        assert_eq!(cfg.policy, TransferPolicy::Deny);
        assert_eq!(cfg.rate_limit_per_dialog, 0);
        assert_eq!(cfg.max_transfer_depth, 0);
    }

    #[test]
    fn test_is_allowed_allow() {
        let cfg = TransferConfig::default();
        assert!(cfg.is_allowed(TransferType::Blind));
        assert!(cfg.is_allowed(TransferType::Attended));
        assert!(cfg.is_allowed(TransferType::Consultative));
    }

    #[test]
    fn test_is_allowed_deny() {
        let cfg = TransferConfig {
            policy: TransferPolicy::Deny,
            ..Default::default()
        };
        assert!(!cfg.is_allowed(TransferType::Blind));
        assert!(!cfg.is_allowed(TransferType::Attended));
        assert!(!cfg.is_allowed(TransferType::Consultative));
    }

    #[test]
    fn test_is_allowed_blind_only() {
        let cfg = TransferConfig {
            policy: TransferPolicy::BlindOnly,
            ..Default::default()
        };
        assert!(cfg.is_allowed(TransferType::Blind));
        assert!(!cfg.is_allowed(TransferType::Attended));
        assert!(!cfg.is_allowed(TransferType::Consultative));
    }

    #[test]
    fn test_is_allowed_attended_only() {
        let cfg = TransferConfig {
            policy: TransferPolicy::AttendedOnly,
            ..Default::default()
        };
        assert!(!cfg.is_allowed(TransferType::Blind));
        assert!(cfg.is_allowed(TransferType::Attended));
        assert!(cfg.is_allowed(TransferType::Consultative));
    }

    #[test]
    fn test_validate_valid() {
        assert!(TransferConfig::default().validate().is_ok());
        assert!(TransferConfig::sbc_defaults().validate().is_ok());
        assert!(TransferConfig::pbx_defaults().validate().is_ok());
    }

    #[test]
    fn test_validate_deny_always_ok() {
        // Deny policy skips numeric validation
        assert!(TransferConfig::session_manager_defaults()
            .validate()
            .is_ok());
    }

    #[test]
    fn test_validate_zero_rate_limit() {
        let cfg = TransferConfig {
            rate_limit_per_dialog: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("rate_limit_per_dialog"));
    }

    #[test]
    fn test_validate_zero_max_depth() {
        let cfg = TransferConfig {
            max_transfer_depth: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_transfer_depth"));
    }

    #[test]
    fn test_validate_excessive_max_depth() {
        let cfg = TransferConfig {
            max_transfer_depth: 21,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("20"));
    }

    #[test]
    fn test_validate_boundary_values() {
        // max_transfer_depth = 20 is valid
        let mut cfg = TransferConfig {
            max_transfer_depth: 20,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());

        // rate_limit = 1 is valid
        cfg.rate_limit_per_dialog = 1;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_transfer_result_variants() {
        let accepted = TransferResult::Accepted;
        assert_eq!(accepted, TransferResult::Accepted);

        let rejected = TransferResult::Rejected {
            status_code: 603,
            reason: "Decline".into(),
        };
        assert!(matches!(
            rejected,
            TransferResult::Rejected {
                status_code: 603,
                ..
            }
        ));

        let failed = TransferResult::Failed {
            reason: "Timeout".into(),
        };
        assert!(matches!(failed, TransferResult::Failed { .. }));

        let denied = TransferResult::PolicyDenied;
        assert_eq!(denied, TransferResult::PolicyDenied);
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = TransferConfig::sbc_defaults();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: TransferConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
