//! Topology hiding configuration for B2BUA.
//!
//! Controls how the B2BUA conceals internal network topology from external
//! parties by rewriting or stripping SIP headers (Via, Record-Route, Contact,
//! User-Agent/Server).
//!
//! - **SBC**: Full hiding to protect internal topology from external networks
//! - **PBX**: None (transparent) since all parties are internal
//! - **Session Manager**: Partial hiding for inter-cluster routing

use serde::{Deserialize, Serialize};

/// How aggressively the B2BUA hides internal topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TopologyHidingMode {
    /// No topology hiding — headers pass through unmodified.
    None,

    /// Selective hiding — only some headers are rewritten.
    Partial,

    /// Full hiding — all topology-revealing headers are rewritten or stripped.
    Full,
}

impl Default for TopologyHidingMode {
    fn default() -> Self {
        Self::None
    }
}

/// Policy-level configuration for topology hiding.
///
/// This struct controls *what* gets hidden. The actual header rewriting is
/// performed by the SIP handler layer, which reads this config to decide
/// which transformations to apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyHidingConfig {
    /// Overall hiding mode.
    pub mode: TopologyHidingMode,

    /// Strip or rewrite Via headers to hide internal hops.
    pub hide_via: bool,

    /// Rewrite Record-Route headers with the local domain.
    pub hide_record_route: bool,

    /// Rewrite Contact headers with the local domain.
    pub hide_contact: bool,

    /// Domain used for rewritten headers (e.g. "sbc.example.com").
    /// Required when any hiding is enabled.
    pub local_domain: Option<String>,

    /// Remove User-Agent and Server headers from relayed messages.
    pub hide_user_agent: bool,

    /// Custom User-Agent/Server value to insert (only when `hide_user_agent` is true).
    pub custom_user_agent: Option<String>,
}

impl Default for TopologyHidingConfig {
    fn default() -> Self {
        Self {
            mode: TopologyHidingMode::None,
            hide_via: false,
            hide_record_route: false,
            hide_contact: false,
            local_domain: None,
            hide_user_agent: false,
            custom_user_agent: None,
        }
    }
}

impl TopologyHidingConfig {
    /// SBC defaults: full topology hiding with all header transformations enabled.
    ///
    /// Caller must set `local_domain` before use.
    pub fn sbc_defaults() -> Self {
        Self {
            mode: TopologyHidingMode::Full,
            hide_via: true,
            hide_record_route: true,
            hide_contact: true,
            local_domain: None,
            hide_user_agent: true,
            custom_user_agent: None,
        }
    }

    /// PBX defaults: no topology hiding (transparent mode).
    pub fn pbx_defaults() -> Self {
        Self::default()
    }

    /// Returns `true` if any topology hiding is active.
    pub fn is_active(&self) -> bool {
        self.mode != TopologyHidingMode::None
    }

    /// Validate the configuration.
    ///
    /// Returns `Err` if:
    /// - `local_domain` is set but empty or contains CR/LF characters
    /// - `custom_user_agent` is set but contains CR/LF characters
    pub fn validate(&self) -> Result<(), String> {
        if let Some(ref domain) = self.local_domain {
            if domain.is_empty() {
                return Err("local_domain must not be empty when set".into());
            }
            if domain.contains('\r') || domain.contains('\n') {
                return Err("local_domain must not contain CR/LF characters".into());
            }
        }
        if let Some(ref ua) = self.custom_user_agent {
            if ua.contains('\r') || ua.contains('\n') {
                return Err("custom_user_agent must not contain CR/LF characters".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let cfg = TopologyHidingConfig::default();
        assert_eq!(cfg.mode, TopologyHidingMode::None);
        assert!(!cfg.hide_via);
        assert!(!cfg.hide_record_route);
        assert!(!cfg.hide_contact);
        assert!(cfg.local_domain.is_none());
        assert!(!cfg.hide_user_agent);
        assert!(cfg.custom_user_agent.is_none());
    }

    #[test]
    fn test_sbc_defaults() {
        let cfg = TopologyHidingConfig::sbc_defaults();
        assert_eq!(cfg.mode, TopologyHidingMode::Full);
        assert!(cfg.hide_via);
        assert!(cfg.hide_record_route);
        assert!(cfg.hide_contact);
        assert!(cfg.hide_user_agent);
        assert!(cfg.is_active());
    }

    #[test]
    fn test_pbx_defaults() {
        let cfg = TopologyHidingConfig::pbx_defaults();
        assert_eq!(cfg.mode, TopologyHidingMode::None);
        assert!(!cfg.is_active());
    }

    #[test]
    fn test_is_active() {
        let mut cfg = TopologyHidingConfig::default();
        assert!(!cfg.is_active());

        cfg.mode = TopologyHidingMode::Partial;
        assert!(cfg.is_active());

        cfg.mode = TopologyHidingMode::Full;
        assert!(cfg.is_active());
    }

    #[test]
    fn test_validate_valid() {
        let mut cfg = TopologyHidingConfig::sbc_defaults();
        cfg.local_domain = Some("sbc.example.com".into());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_empty_domain() {
        let mut cfg = TopologyHidingConfig::default();
        cfg.local_domain = Some(String::new());
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_crlf_domain() {
        let mut cfg = TopologyHidingConfig::default();
        cfg.local_domain = Some("bad\r\ndomain".into());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_crlf_user_agent() {
        let mut cfg = TopologyHidingConfig::default();
        cfg.custom_user_agent = Some("bad\nagent".into());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_no_domain_ok() {
        let cfg = TopologyHidingConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = TopologyHidingConfig::sbc_defaults();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: TopologyHidingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
