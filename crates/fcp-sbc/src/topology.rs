//! SBC-specific topology hiding utilities.
//!
//! The primary topology hiding implementation lives in `fcp-sip/src/handler.rs`
//! (`apply_topology_hiding()`), which operates on SIP response messages.
//! This module provides additional SBC-specific helpers if needed.

use fcp_b2bua_core::topology::{TopologyHidingConfig, TopologyHidingMode};

/// Check whether topology hiding requires a specific external domain.
///
/// Returns `true` if topology hiding is active and will need an external
/// domain for header rewriting.
#[allow(dead_code)]
pub fn requires_external_domain(config: &TopologyHidingConfig) -> bool {
    config.mode != TopologyHidingMode::None
        && (config.hide_via || config.hide_record_route || config.hide_contact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_none_mode_no_domain_needed() {
        let cfg = TopologyHidingConfig::default();
        assert!(!requires_external_domain(&cfg));
    }

    #[test]
    fn test_sbc_defaults_need_domain() {
        let cfg = TopologyHidingConfig::sbc_defaults();
        assert!(requires_external_domain(&cfg));
    }

    #[test]
    fn test_partial_no_hide_flags_no_domain() {
        let cfg = TopologyHidingConfig {
            mode: TopologyHidingMode::Partial,
            hide_via: false,
            hide_record_route: false,
            hide_contact: false,
            hide_user_agent: true,
            ..Default::default()
        };
        assert!(!requires_external_domain(&cfg));
    }
}
