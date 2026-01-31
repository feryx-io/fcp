//! Header Manipulation Rules (HMR) configuration for the SBC.
//!
//! Provides Oracle-style SIP header manipulation rules supporting ingress
//! (inbound), egress (outbound), or both directions. Rules can add, remove,
//! replace, or regex-modify SIP headers and the Request-URI.
//!
//! Rules are scoped globally or per-trunk, ordered by priority, and filtered
//! by direction, message type, and SIP method.
//!
//! This module contains only configuration types — regex patterns are stored
//! as strings and compiled at runtime in the `fcp-sbc` crate.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Direction when this rule applies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HmrDirection {
    /// Apply only to inbound (received) messages.
    Ingress,
    /// Apply only to outbound (sent) messages.
    Egress,
    /// Apply to both inbound and outbound messages.
    #[default]
    Both,
}

/// Which message types this rule applies to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HmrMessageType {
    /// Apply only to SIP requests.
    Request,
    /// Apply only to SIP responses.
    Response,
    /// Apply to both requests and responses.
    #[default]
    Any,
}

/// Condition that must match for a rule to fire (all conditions are ANDed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HmrCondition {
    /// Header to check. Special values: `$request-uri`, `$method`, `$status-code`.
    pub header_name: String,
    /// Regex pattern to match against the header value.
    pub pattern: String,
    /// If true, the condition passes when the pattern does NOT match.
    #[serde(default)]
    pub negate: bool,
}

/// Type of manipulation action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HmrActionType {
    /// Add a header (appends even if header already exists).
    Add,
    /// Remove all instances of the named header.
    Remove,
    /// Replace the first instance of the named header's value.
    Replace,
    /// Regex find/replace within the header value (requires `pattern` + `value`).
    Modify,
}

/// A single manipulation action within a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HmrAction {
    /// The type of manipulation to perform.
    pub action_type: HmrActionType,
    /// Target header name. Special values: `$request-uri`.
    pub header_name: String,
    /// New value (for Add/Replace) or replacement string (for Modify, supports `$1` captures).
    #[serde(default)]
    pub value: Option<String>,
    /// Regex pattern for Modify action (find portion).
    #[serde(default)]
    pub pattern: Option<String>,
}

fn default_priority() -> u32 {
    100
}

fn default_enabled() -> bool {
    true
}

/// A named header manipulation rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HmrRule {
    /// Unique rule name.
    pub name: String,
    /// Lower priority = runs first. Default: 100.
    #[serde(default = "default_priority")]
    pub priority: u32,
    /// Whether this rule is active. Default: true.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Direction filter.
    #[serde(default)]
    pub direction: HmrDirection,
    /// Message type filter.
    #[serde(default)]
    pub message_type: HmrMessageType,
    /// SIP methods this rule applies to (empty = all). E.g. `["INVITE", "REGISTER"]`.
    #[serde(default)]
    pub methods: Vec<String>,
    /// All conditions must match for the rule to fire (AND logic).
    #[serde(default)]
    pub conditions: Vec<HmrCondition>,
    /// Actions executed in order when the rule fires.
    pub actions: Vec<HmrAction>,
}

/// Top-level header manipulation configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderManipulationConfig {
    /// Whether header manipulation is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// The list of manipulation rules.
    #[serde(default)]
    pub rules: Vec<HmrRule>,
}

impl HeaderManipulationConfig {
    /// Validate the HMR configuration.
    ///
    /// Checks:
    /// - Rule names non-empty, no CRLF, unique
    /// - Actions have required fields (Add/Replace require `value`, Modify requires `pattern` + `value`)
    /// - Header names non-empty, no CRLF
    /// - Priority <= 10000
    /// - At least one action per rule
    pub fn validate(&self) -> Result<(), String> {
        let mut seen_names = HashSet::new();

        for (i, rule) in self.rules.iter().enumerate() {
            // Rule name validation
            if rule.name.is_empty() {
                return Err(format!("header_manipulation.rules[{i}]: name must not be empty"));
            }
            if rule.name.contains('\r') || rule.name.contains('\n') {
                return Err(format!(
                    "header_manipulation.rules[{i}]: name {:?} must not contain CR/LF",
                    rule.name
                ));
            }
            if !seen_names.insert(&rule.name) {
                return Err(format!(
                    "header_manipulation.rules[{i}]: duplicate rule name {:?}",
                    rule.name
                ));
            }

            // Priority
            if rule.priority > 10000 {
                return Err(format!(
                    "header_manipulation.rules[{i}] {:?}: priority must not exceed 10000",
                    rule.name
                ));
            }

            // Must have at least one action
            if rule.actions.is_empty() {
                return Err(format!(
                    "header_manipulation.rules[{i}] {:?}: must have at least one action",
                    rule.name
                ));
            }

            // Validate each action
            for (j, action) in rule.actions.iter().enumerate() {
                if action.header_name.is_empty() {
                    return Err(format!(
                        "header_manipulation.rules[{i}] {:?} actions[{j}]: header_name must not be empty",
                        rule.name
                    ));
                }
                if action.header_name.contains('\r') || action.header_name.contains('\n') {
                    return Err(format!(
                        "header_manipulation.rules[{i}] {:?} actions[{j}]: header_name must not contain CR/LF",
                        rule.name
                    ));
                }

                match action.action_type {
                    HmrActionType::Add | HmrActionType::Replace => {
                        if action.value.is_none() {
                            return Err(format!(
                                "header_manipulation.rules[{i}] {:?} actions[{j}]: {:?} requires value",
                                rule.name, action.action_type
                            ));
                        }
                    }
                    HmrActionType::Modify => {
                        if action.pattern.is_none() {
                            return Err(format!(
                                "header_manipulation.rules[{i}] {:?} actions[{j}]: Modify requires pattern",
                                rule.name
                            ));
                        }
                        if action.value.is_none() {
                            return Err(format!(
                                "header_manipulation.rules[{i}] {:?} actions[{j}]: Modify requires value",
                                rule.name
                            ));
                        }
                    }
                    HmrActionType::Remove => {
                        // Remove doesn't require value or pattern
                    }
                }
            }

            // Validate conditions
            for (j, cond) in rule.conditions.iter().enumerate() {
                if cond.header_name.is_empty() {
                    return Err(format!(
                        "header_manipulation.rules[{i}] {:?} conditions[{j}]: header_name must not be empty",
                        rule.name
                    ));
                }
                if cond.pattern.is_empty() {
                    return Err(format!(
                        "header_manipulation.rules[{i}] {:?} conditions[{j}]: pattern must not be empty",
                        rule.name
                    ));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_rule() -> HmrRule {
        HmrRule {
            name: "test-rule".into(),
            priority: 100,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Test".into(),
                value: Some("value".into()),
                pattern: None,
            }],
        }
    }

    fn valid_config() -> HeaderManipulationConfig {
        HeaderManipulationConfig {
            enabled: true,
            rules: vec![valid_rule()],
        }
    }

    #[test]
    fn test_default() {
        let cfg = HeaderManipulationConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.rules.is_empty());
    }

    #[test]
    fn test_validate_valid() {
        assert!(valid_config().validate().is_ok());
    }

    #[test]
    fn test_validate_empty_rule_name() {
        let mut cfg = valid_config();
        cfg.rules[0].name = String::new();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("name must not be empty"));
    }

    #[test]
    fn test_validate_crlf_rule_name() {
        let mut cfg = valid_config();
        cfg.rules[0].name = "bad\r\nname".into();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("CR/LF"));
    }

    #[test]
    fn test_validate_add_missing_value() {
        let mut cfg = valid_config();
        cfg.rules[0].actions[0].value = None;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("requires value"));
    }

    #[test]
    fn test_validate_replace_missing_value() {
        let mut cfg = valid_config();
        cfg.rules[0].actions[0].action_type = HmrActionType::Replace;
        cfg.rules[0].actions[0].value = None;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("requires value"));
    }

    #[test]
    fn test_validate_modify_missing_pattern() {
        let mut cfg = valid_config();
        cfg.rules[0].actions[0] = HmrAction {
            action_type: HmrActionType::Modify,
            header_name: "Via".into(),
            value: Some("replacement".into()),
            pattern: None,
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("Modify requires pattern"));
    }

    #[test]
    fn test_validate_modify_missing_value() {
        let mut cfg = valid_config();
        cfg.rules[0].actions[0] = HmrAction {
            action_type: HmrActionType::Modify,
            header_name: "Via".into(),
            value: None,
            pattern: Some(".*".into()),
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("Modify requires value"));
    }

    #[test]
    fn test_validate_duplicate_names() {
        let mut cfg = valid_config();
        let mut rule2 = valid_rule();
        rule2.name = "test-rule".into(); // same name
        cfg.rules.push(rule2);
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("duplicate rule name"));
    }

    #[test]
    fn test_validate_empty_actions() {
        let mut cfg = valid_config();
        cfg.rules[0].actions.clear();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("must have at least one action"));
    }

    #[test]
    fn test_validate_empty_header_name_in_action() {
        let mut cfg = valid_config();
        cfg.rules[0].actions[0].header_name = String::new();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("header_name must not be empty"));
    }

    #[test]
    fn test_validate_crlf_header_name_in_action() {
        let mut cfg = valid_config();
        cfg.rules[0].actions[0].header_name = "Bad\nHeader".into();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("header_name must not contain CR/LF"));
    }

    #[test]
    fn test_validate_priority_too_high() {
        let mut cfg = valid_config();
        cfg.rules[0].priority = 10001;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("priority must not exceed 10000"));
    }

    #[test]
    fn test_validate_empty_condition_header_name() {
        let mut cfg = valid_config();
        cfg.rules[0].conditions.push(HmrCondition {
            header_name: String::new(),
            pattern: ".*".into(),
            negate: false,
        });
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("header_name must not be empty"));
    }

    #[test]
    fn test_validate_empty_condition_pattern() {
        let mut cfg = valid_config();
        cfg.rules[0].conditions.push(HmrCondition {
            header_name: "Via".into(),
            pattern: String::new(),
            negate: false,
        });
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("pattern must not be empty"));
    }

    #[test]
    fn test_validate_remove_no_value_ok() {
        let mut cfg = valid_config();
        cfg.rules[0].actions[0] = HmrAction {
            action_type: HmrActionType::Remove,
            header_name: "X-Internal".into(),
            value: None,
            pattern: None,
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = HeaderManipulationConfig {
            enabled: true,
            rules: vec![
                HmrRule {
                    name: "strip-internal".into(),
                    priority: 10,
                    enabled: true,
                    direction: HmrDirection::Egress,
                    message_type: HmrMessageType::Any,
                    methods: vec![],
                    conditions: vec![],
                    actions: vec![HmrAction {
                        action_type: HmrActionType::Remove,
                        header_name: "X-Internal".into(),
                        value: None,
                        pattern: None,
                    }],
                },
                HmrRule {
                    name: "rewrite-uri".into(),
                    priority: 20,
                    enabled: true,
                    direction: HmrDirection::Ingress,
                    message_type: HmrMessageType::Request,
                    methods: vec!["INVITE".into()],
                    conditions: vec![HmrCondition {
                        header_name: "$request-uri".into(),
                        pattern: "sip:(.+)@old.com".into(),
                        negate: false,
                    }],
                    actions: vec![HmrAction {
                        action_type: HmrActionType::Modify,
                        header_name: "$request-uri".into(),
                        value: Some("sip:$1@new.com".into()),
                        pattern: Some("sip:(.+)@old.com".into()),
                    }],
                },
            ],
        };

        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: HeaderManipulationConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_serde_defaults() {
        // Minimal JSON with only required fields
        let json = r#"{
            "enabled": true,
            "rules": [{
                "name": "test",
                "actions": [{
                    "action_type": "Remove",
                    "header_name": "X-Test"
                }]
            }]
        }"#;
        let cfg: HeaderManipulationConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.rules[0].priority, 100); // default
        assert!(cfg.rules[0].enabled); // default true
        assert_eq!(cfg.rules[0].direction, HmrDirection::Both); // default
        assert_eq!(cfg.rules[0].message_type, HmrMessageType::Any); // default
        assert!(cfg.rules[0].methods.is_empty()); // default
        assert!(cfg.rules[0].conditions.is_empty()); // default
    }

    #[test]
    fn test_direction_variants() {
        assert_eq!(HmrDirection::default(), HmrDirection::Both);
    }

    #[test]
    fn test_message_type_variants() {
        assert_eq!(HmrMessageType::default(), HmrMessageType::Any);
    }
}
