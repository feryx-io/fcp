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

    /// Access voicemail. No suffix = own mailbox, with suffix = specific mailbox.
    VoicemailAccess { mailbox: Option<String> },

    /// A change to the caller's own calling settings
    /// (`docs/ENTERPRISE_CALLING.md` §3.1).
    Calling(CallingCode),
}

/// The calling settings a phone can change by dialing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallingCode {
    DndOn,
    DndOff,
    /// The target as dialed after the code: a number, an extension.
    ForwardAllOn {
        target: String,
    },
    ForwardAllOff,
    ForwardBusyOn {
        target: String,
    },
    ForwardBusyOff,
    ForwardNoAnswerOn {
        target: String,
    },
    ForwardNoAnswerOff,
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
    let is_enabled = |code: &str| !code.trim().is_empty();

    // Build prefix→action mapping, sorted longest-first
    #[allow(clippy::type_complexity)]
    let mut prefixes: Vec<(&str, fn(&str) -> FeatureCodeAction)> = Vec::new();
    if is_enabled(&config.intercom_twoway) {
        prefixes.push((&config.intercom_twoway, |suffix| {
            FeatureCodeAction::IntercomTwoWay {
                extension: suffix.to_string(),
            }
        }));
    }
    if is_enabled(&config.intercom_oneway) {
        prefixes.push((&config.intercom_oneway, |suffix| {
            FeatureCodeAction::IntercomOneWay {
                extension: suffix.to_string(),
            }
        }));
    }
    if is_enabled(&config.retrieve) {
        prefixes.push((&config.retrieve, |suffix| {
            FeatureCodeAction::RetrieveParkedCall {
                orbit: suffix.parse().unwrap_or(0),
            }
        }));
    }
    if is_enabled(&config.park) {
        prefixes.push((&config.park, |_| FeatureCodeAction::ParkCall));
    }
    if is_enabled(&config.directed_pickup) {
        prefixes.push((&config.directed_pickup, |suffix| {
            FeatureCodeAction::DirectedPickup {
                extension: suffix.to_string(),
            }
        }));
    }
    if is_enabled(&config.forward_all_on) {
        prefixes.push((&config.forward_all_on, |suffix| {
            FeatureCodeAction::Calling(CallingCode::ForwardAllOn {
                target: suffix.to_string(),
            })
        }));
    }
    if is_enabled(&config.forward_busy_on) {
        prefixes.push((&config.forward_busy_on, |suffix| {
            FeatureCodeAction::Calling(CallingCode::ForwardBusyOn {
                target: suffix.to_string(),
            })
        }));
    }
    if is_enabled(&config.forward_no_answer_on) {
        prefixes.push((&config.forward_no_answer_on, |suffix| {
            FeatureCodeAction::Calling(CallingCode::ForwardNoAnswerOn {
                target: suffix.to_string(),
            })
        }));
    }
    if is_enabled(&config.voicemail_access) {
        prefixes.push((&config.voicemail_access, |suffix| {
            FeatureCodeAction::VoicemailAccess {
                mailbox: if suffix.is_empty() {
                    None
                } else {
                    Some(suffix.to_string())
                },
            }
        }));
    }

    // Sort by prefix length descending so longer prefixes match first
    prefixes.sort_by_key(|p| std::cmp::Reverse(p.0.len()));

    for (prefix, action_fn) in &prefixes {
        if let Some(suffix) = user.strip_prefix(*prefix) {
            // Prefix-only codes (park, voicemail_access) work with or without suffix
            if *prefix == config.park || *prefix == config.voicemail_access {
                return Some(action_fn(suffix));
            }
            // Other codes require a non-empty suffix (the target extension/orbit)
            if !suffix.is_empty() {
                return Some(action_fn(suffix));
            }
        }
    }

    // Exact-match codes (no suffix)
    let exact: [(&str, FeatureCodeAction); 5] = [
        (
            &config.dnd_on,
            FeatureCodeAction::Calling(CallingCode::DndOn),
        ),
        (
            &config.dnd_off,
            FeatureCodeAction::Calling(CallingCode::DndOff),
        ),
        (
            &config.forward_all_off,
            FeatureCodeAction::Calling(CallingCode::ForwardAllOff),
        ),
        (
            &config.forward_busy_off,
            FeatureCodeAction::Calling(CallingCode::ForwardBusyOff),
        ),
        (
            &config.forward_no_answer_off,
            FeatureCodeAction::Calling(CallingCode::ForwardNoAnswerOff),
        ),
    ];
    for (code, action) in exact {
        if is_enabled(code) && user == code {
            return Some(action);
        }
    }
    if is_enabled(&config.group_pickup) && user == config.group_pickup {
        return Some(FeatureCodeAction::GroupPickup);
    }
    if is_enabled(&config.agent_login) && user == config.agent_login {
        return Some(FeatureCodeAction::AgentLogin);
    }
    if is_enabled(&config.agent_logout) && user == config.agent_logout {
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
        let mut cfg = default_config();
        cfg.group_pickup = "*8".to_string();
        assert_eq!(parse("*8", &cfg), Some(FeatureCodeAction::GroupPickup));
        assert_eq!(
            parse("sip:*8@10.0.0.1", &cfg),
            Some(FeatureCodeAction::GroupPickup)
        );
    }

    #[test]
    fn test_directed_pickup() {
        let mut cfg = default_config();
        cfg.directed_pickup = "*9".to_string();
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
        let mut cfg = default_config();
        cfg.intercom_oneway = "*80".to_string();
        cfg.intercom_twoway = "*81".to_string();
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
    fn calling_codes_parse_with_and_without_a_target() {
        let cfg = default_config();
        assert_eq!(
            parse("*78", &cfg),
            Some(FeatureCodeAction::Calling(CallingCode::DndOn))
        );
        assert_eq!(
            parse("sip:*79@10.0.0.1", &cfg),
            Some(FeatureCodeAction::Calling(CallingCode::DndOff))
        );
        assert_eq!(
            parse("*72+15551234567", &cfg),
            Some(FeatureCodeAction::Calling(CallingCode::ForwardAllOn {
                target: "+15551234567".to_string()
            }))
        );
        assert_eq!(
            parse("*73", &cfg),
            Some(FeatureCodeAction::Calling(CallingCode::ForwardAllOff))
        );
        assert_eq!(
            parse("*902002", &cfg),
            Some(FeatureCodeAction::Calling(CallingCode::ForwardBusyOn {
                target: "2002".to_string()
            }))
        );
        assert_eq!(
            parse("*91", &cfg),
            Some(FeatureCodeAction::Calling(CallingCode::ForwardBusyOff))
        );
        assert_eq!(
            parse("*922002", &cfg),
            Some(FeatureCodeAction::Calling(CallingCode::ForwardNoAnswerOn {
                target: "2002".to_string()
            }))
        );
        assert_eq!(
            parse("*93", &cfg),
            Some(FeatureCodeAction::Calling(CallingCode::ForwardNoAnswerOff))
        );
        // An "on" code without a target is not a code: nothing to forward to.
        assert_eq!(parse("*72", &cfg), None);
        // Park's `*70` and retrieve's `*71` are untouched by `*72`/`*73`.
        assert_eq!(parse("*70", &cfg), Some(FeatureCodeAction::ParkCall));
        assert_eq!(
            parse("*71701", &cfg),
            Some(FeatureCodeAction::RetrieveParkedCall { orbit: 701 })
        );
    }

    #[test]
    fn test_no_match() {
        let cfg = default_config();
        assert_eq!(parse("1001", &cfg), None);
        assert_eq!(parse("sip:1001@10.0.0.1", &cfg), None);
    }

    #[test]
    fn test_disabled_feature_codes_do_not_match() {
        let cfg = default_config();
        assert_eq!(parse("*8", &cfg), None);
        assert_eq!(parse("*91001", &cfg), None);
        assert_eq!(parse("*801001", &cfg), None);
        assert_eq!(parse("*811001", &cfg), None);
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
    fn test_voicemail_access_own_mailbox() {
        let cfg = default_config();
        assert_eq!(
            parse("*98", &cfg),
            Some(FeatureCodeAction::VoicemailAccess { mailbox: None })
        );
        assert_eq!(
            parse("sip:*98@10.0.0.1", &cfg),
            Some(FeatureCodeAction::VoicemailAccess { mailbox: None })
        );
    }

    #[test]
    fn test_voicemail_access_specific_mailbox() {
        let cfg = default_config();
        assert_eq!(
            parse("*981234", &cfg),
            Some(FeatureCodeAction::VoicemailAccess {
                mailbox: Some("1234".to_string())
            })
        );
        assert_eq!(
            parse("sip:*981001@10.0.0.1", &cfg),
            Some(FeatureCodeAction::VoicemailAccess {
                mailbox: Some("1001".to_string())
            })
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
