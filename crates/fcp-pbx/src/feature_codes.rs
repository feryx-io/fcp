//! Star code (feature code) parsing.
//!
//! Extracts the user-part from a SIP URI and matches it against configured
//! feature code prefixes to determine the intended PBX action.

use crate::config::FeatureCodeConfig;

/// Action identified from a dialed feature code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureCodeAction {
    /// Pick up a ringing call within the caller's pickup group.
    GroupPickup,

    /// Pick up a ringing call targeting a specific extension.
    DirectedPickup { extension: String },

    /// Park the current active call (orbit auto-assigned).
    ParkCall,

    /// Retrieve a parked call from a specific orbit.
    RetrieveParkedCall { orbit: u16 },

    /// One-way intercom (listen-only for the target).
    IntercomOneWay { extension: String },

    /// Two-way intercom (full duplex auto-answer).
    IntercomTwoWay { extension: String },

    /// Agent login to queue(s).
    AgentLogin,

    /// Agent logout from queue(s).
    AgentLogout,
}

/// Extract the user-part from a SIP URI or raw dialed string.
///
/// Examples:
///   "sip:*701@192.168.1.1" → "*701"
///   "*701" → "*701"
fn extract_user_part(dialed: &str) -> &str {
    let without_scheme = dialed
        .strip_prefix("sip:")
        .or_else(|| dialed.strip_prefix("sips:"))
        .unwrap_or(dialed);
    // Take everything before '@' (if present)
    without_scheme.split('@').next().unwrap_or(without_scheme)
}

/// Parse a dialed string against configured feature codes.
///
/// Prefixes are matched longest-first to avoid ambiguity (e.g., `*81` before `*8`).
pub fn parse(dialed: &str, config: &FeatureCodeConfig) -> Option<FeatureCodeAction> {
    let user = extract_user_part(dialed);

    // Build prefix→action mapping, sorted longest-first
    #[allow(clippy::type_complexity)]
    let mut prefixes: Vec<(&str, fn(&str) -> FeatureCodeAction)> = vec![
        (&config.intercom_twoway, |suffix| {
            FeatureCodeAction::IntercomTwoWay {
                extension: suffix.to_string(),
            }
        }),
        (&config.intercom_oneway, |suffix| {
            FeatureCodeAction::IntercomOneWay {
                extension: suffix.to_string(),
            }
        }),
        (&config.retrieve, |suffix| {
            FeatureCodeAction::RetrieveParkedCall {
                orbit: suffix.parse().unwrap_or(0),
            }
        }),
        (&config.park, |_| FeatureCodeAction::ParkCall),
        (&config.directed_pickup, |suffix| {
            FeatureCodeAction::DirectedPickup {
                extension: suffix.to_string(),
            }
        }),
    ];

    // Sort by prefix length descending so longer prefixes match first
    prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    for (prefix, action_fn) in &prefixes {
        if let Some(suffix) = user.strip_prefix(*prefix) {
            // Prefix-only codes (park) have no suffix requirement
            if *prefix == config.park {
                return Some(action_fn(suffix));
            }
            // Other codes require a non-empty suffix (the target extension/orbit)
            if !suffix.is_empty() {
                return Some(action_fn(suffix));
            }
        }
    }

    // Exact-match codes (no suffix)
    if user == config.group_pickup {
        return Some(FeatureCodeAction::GroupPickup);
    }
    if user == config.agent_login {
        return Some(FeatureCodeAction::AgentLogin);
    }
    if user == config.agent_logout {
        return Some(FeatureCodeAction::AgentLogout);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> FeatureCodeConfig {
        FeatureCodeConfig::default()
    }

    #[test]
    fn test_group_pickup() {
        let cfg = default_config();
        assert_eq!(parse("*8", &cfg), Some(FeatureCodeAction::GroupPickup));
        assert_eq!(
            parse("sip:*8@10.0.0.1", &cfg),
            Some(FeatureCodeAction::GroupPickup)
        );
    }

    #[test]
    fn test_directed_pickup() {
        let cfg = default_config();
        assert_eq!(
            parse("*91001", &cfg),
            Some(FeatureCodeAction::DirectedPickup {
                extension: "1001".to_string()
            })
        );
        assert_eq!(
            parse("sip:*91001@10.0.0.1", &cfg),
            Some(FeatureCodeAction::DirectedPickup {
                extension: "1001".to_string()
            })
        );
    }

    #[test]
    fn test_park_call() {
        let cfg = default_config();
        assert_eq!(parse("*70", &cfg), Some(FeatureCodeAction::ParkCall));
        assert_eq!(
            parse("sip:*70@10.0.0.1", &cfg),
            Some(FeatureCodeAction::ParkCall)
        );
    }

    #[test]
    fn test_retrieve_parked_call() {
        let cfg = default_config();
        assert_eq!(
            parse("*71701", &cfg),
            Some(FeatureCodeAction::RetrieveParkedCall { orbit: 701 })
        );
    }

    #[test]
    fn test_intercom_twoway_before_oneway() {
        let cfg = default_config();
        // *81 must match intercom_twoway, not intercom_oneway + "1..."
        assert_eq!(
            parse("*811001", &cfg),
            Some(FeatureCodeAction::IntercomTwoWay {
                extension: "1001".to_string()
            })
        );
        assert_eq!(
            parse("*801001", &cfg),
            Some(FeatureCodeAction::IntercomOneWay {
                extension: "1001".to_string()
            })
        );
    }

    #[test]
    fn test_no_match() {
        let cfg = default_config();
        assert_eq!(parse("1001", &cfg), None);
        assert_eq!(parse("sip:1001@10.0.0.1", &cfg), None);
    }

    #[test]
    fn test_agent_login() {
        let cfg = default_config();
        assert_eq!(parse("*50", &cfg), Some(FeatureCodeAction::AgentLogin));
        assert_eq!(
            parse("sip:*50@10.0.0.1", &cfg),
            Some(FeatureCodeAction::AgentLogin)
        );
    }

    #[test]
    fn test_agent_logout() {
        let cfg = default_config();
        assert_eq!(parse("*51", &cfg), Some(FeatureCodeAction::AgentLogout));
        assert_eq!(
            parse("sip:*51@10.0.0.1", &cfg),
            Some(FeatureCodeAction::AgentLogout)
        );
    }

    #[test]
    fn test_prefix_without_suffix_no_match() {
        let cfg = default_config();
        // "*9" alone (directed pickup prefix but no extension) → no match
        assert_eq!(parse("*9", &cfg), None);
        // "*71" alone (retrieve prefix but no orbit) → no match
        assert_eq!(parse("*71", &cfg), None);
    }
}
