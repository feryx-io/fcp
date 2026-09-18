//! Header Manipulation Rules (HMR) runtime engine.
//!
//! Compiles HMR configuration rules into regex-powered matchers and applies
//! them to SIP requests and responses. Supports direction filtering (ingress/
//! egress), message type filtering (request/response), method filtering, trunk
//! scoping, conditional matching, and priority ordering.

use crate::hmr::{HeaderManipulationConfig, HmrActionType, HmrDirection, HmrMessageType};
use crate::TrunkConfig;
use regex::Regex;
use sip_core::uri::Uri;
use sip_core::{Request, Response};
use std::collections::HashMap;
use tracing::warn;

/// A compiled condition ready for evaluation.
struct CompiledCondition {
    header_name: String,
    regex: Regex,
    negate: bool,
}

/// A compiled action ready for execution.
struct CompiledAction {
    action_type: HmrActionType,
    header_name: String,
    value: Option<String>,
    pattern: Option<Regex>,
}

/// A compiled rule with pre-compiled regexes.
struct CompiledRule {
    name: String,
    priority: u32,
    direction: HmrDirection,
    message_type: HmrMessageType,
    methods: Vec<String>,
    conditions: Vec<CompiledCondition>,
    actions: Vec<CompiledAction>,
}

/// The HMR runtime engine. Holds compiled rules and trunk-to-rule mappings.
impl std::fmt::Debug for HeaderManipulationEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeaderManipulationEngine")
            .field("rules_count", &self.rules.len())
            .field(
                "trunk_rule_map_keys",
                &self.trunk_rule_map.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

pub struct HeaderManipulationEngine {
    rules: Vec<CompiledRule>,
    /// Maps trunk name → indices into `rules` that apply to that trunk.
    trunk_rule_map: HashMap<String, Vec<usize>>,
    /// Maps rule name → its index, so a trunk this engine never saw — a
    /// tenant's, created after the node started — can still name the rules
    /// it wants (`docs/TENANT_TRUNKS.md` §6).
    rule_index: HashMap<String, usize>,
}

impl HeaderManipulationEngine {
    /// Compile HMR config into the runtime engine.
    ///
    /// Compiles all regex patterns, builds trunk-rule mappings, and sorts
    /// rules by priority. Returns an error if any regex fails to compile.
    pub fn new(config: &HeaderManipulationConfig, trunks: &[TrunkConfig]) -> Result<Self, String> {
        let mut rules = Vec::new();

        for cfg_rule in &config.rules {
            if !cfg_rule.enabled {
                continue;
            }

            let mut conditions = Vec::new();
            for cond in &cfg_rule.conditions {
                let regex = Regex::new(&cond.pattern).map_err(|e| {
                    format!(
                        "rule {:?} condition on {:?}: invalid regex {:?}: {}",
                        cfg_rule.name, cond.header_name, cond.pattern, e
                    )
                })?;
                conditions.push(CompiledCondition {
                    header_name: cond.header_name.clone(),
                    regex,
                    negate: cond.negate,
                });
            }

            let mut actions = Vec::new();
            for act in &cfg_rule.actions {
                let pattern = if let Some(ref pat) = act.pattern {
                    let regex = Regex::new(pat).map_err(|e| {
                        format!(
                            "rule {:?} action on {:?}: invalid regex {:?}: {}",
                            cfg_rule.name, act.header_name, pat, e
                        )
                    })?;
                    Some(regex)
                } else {
                    None
                };
                actions.push(CompiledAction {
                    action_type: act.action_type,
                    header_name: act.header_name.clone(),
                    value: act.value.clone(),
                    pattern,
                });
            }

            let methods: Vec<String> = cfg_rule
                .methods
                .iter()
                .map(|m| m.to_ascii_uppercase())
                .collect();

            rules.push(CompiledRule {
                name: cfg_rule.name.clone(),
                priority: cfg_rule.priority,
                direction: cfg_rule.direction,
                message_type: cfg_rule.message_type,
                methods,
                conditions,
                actions,
            });
        }

        // Sort by priority (lower = first)
        rules.sort_by_key(|r| r.priority);

        // Build trunk → rule indices map
        let mut trunk_rule_map: HashMap<String, Vec<usize>> = HashMap::new();
        for trunk in trunks {
            if trunk.header_rules.is_empty() {
                continue;
            }
            let mut indices = Vec::new();
            for rule_name in &trunk.header_rules {
                if let Some(idx) = rules.iter().position(|r| r.name == *rule_name) {
                    indices.push(idx);
                } else {
                    warn!(
                        trunk = %trunk.name,
                        rule = %rule_name,
                        "trunk references unknown HMR rule"
                    );
                }
            }
            // Maintain priority ordering
            indices.sort_by_key(|&idx| rules[idx].priority);
            trunk_rule_map.insert(trunk.name.clone(), indices);
        }

        let rule_index = rules
            .iter()
            .enumerate()
            .map(|(idx, rule)| (rule.name.clone(), idx))
            .collect();

        Ok(Self {
            rules,
            trunk_rule_map,
            rule_index,
        })
    }

    /// Apply matching ingress rules to a request.
    pub fn apply_request_ingress(&self, req: &mut Request, trunk: Option<&str>) {
        self.apply_request(req, HmrDirection::Ingress, trunk);
    }

    /// Apply matching ingress rules to a response.
    pub fn apply_response_ingress(&self, resp: &mut Response, trunk: Option<&str>) {
        self.apply_response(resp, HmrDirection::Ingress, trunk);
    }

    /// Apply matching egress rules to a request.
    pub fn apply_request_egress(&self, req: &mut Request, trunk: Option<&str>) {
        self.apply_request(req, HmrDirection::Egress, trunk);
    }

    /// Apply matching egress rules to a response.
    pub fn apply_response_egress(&self, resp: &mut Response, trunk: Option<&str>) {
        self.apply_response(resp, HmrDirection::Egress, trunk);
    }

    /// Apply the egress rules `names` names, in the order this engine
    /// compiled them (by priority), to a request leaving on a trunk this
    /// engine was not built from — a tenant's trunk, which names its rules
    /// itself (`docs/TENANT_TRUNKS.md` §6). Names it does not know are
    /// logged and skipped.
    pub fn apply_request_egress_named(&self, req: &mut Request, names: &[String]) {
        self.apply_request_named(req, names, HmrDirection::Egress);
    }

    /// As [`Self::apply_request_egress_named`], for a request that arrived
    /// on such a trunk (`docs/TENANT_TRUNKS.md` §7).
    pub fn apply_request_ingress_named(&self, req: &mut Request, names: &[String]) {
        self.apply_request_named(req, names, HmrDirection::Ingress);
    }

    fn apply_request_named(&self, req: &mut Request, names: &[String], direction: HmrDirection) {
        if names.is_empty() {
            return;
        }
        let mut indices: Vec<usize> = names
            .iter()
            .filter_map(|name| match self.rule_index.get(name) {
                Some(idx) => Some(*idx),
                None => {
                    warn!(rule = %name, "trunk names an unknown HMR rule");
                    None
                }
            })
            .collect();
        indices.sort_unstable();
        indices.dedup();
        let rules: Vec<&CompiledRule> = indices
            .into_iter()
            .map(|idx| &self.rules[idx])
            .filter(|rule| rule.direction == HmrDirection::Both || rule.direction == direction)
            .collect();
        self.apply_rules_to_request(req, &rules);
    }

    /// Whether this engine knows any rule at all.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    fn apply_request(&self, req: &mut Request, direction: HmrDirection, trunk: Option<&str>) {
        let rules = self.matching_rules(direction, trunk);
        self.apply_rules_to_request(req, &rules);
    }

    fn apply_rules_to_request(&self, req: &mut Request, rules: &[&CompiledRule]) {
        let method_str = req.method().to_string();
        let uri_str = req.uri().to_string();

        for rule in rules {
            // Check message type filter
            if rule.message_type == HmrMessageType::Response {
                continue;
            }

            // Check method filter
            if !rule.methods.is_empty() && !rule.methods.iter().any(|m| m == &method_str) {
                continue;
            }

            // Evaluate conditions
            if !evaluate_conditions_request(&rule.conditions, req, &method_str, &uri_str) {
                continue;
            }

            // Execute actions
            for action in &rule.actions {
                execute_action_request(action, req);
            }
        }
    }

    fn apply_response(&self, resp: &mut Response, direction: HmrDirection, trunk: Option<&str>) {
        let status_str = resp.code().to_string();

        for rule in self.matching_rules(direction, trunk) {
            // Check message type filter
            if rule.message_type == HmrMessageType::Request {
                continue;
            }

            // Check method filter: responses don't have a method, so skip if method filter is set
            if !rule.methods.is_empty() {
                continue;
            }

            // Evaluate conditions
            if !evaluate_conditions_response(&rule.conditions, resp, &status_str) {
                continue;
            }

            // Execute actions
            for action in &rule.actions {
                execute_action_response(action, resp);
            }
        }
    }

    /// Get matching rules for a given direction and optional trunk, in priority order.
    fn matching_rules(&self, direction: HmrDirection, trunk: Option<&str>) -> Vec<&CompiledRule> {
        let indices: Box<dyn Iterator<Item = usize>> = if let Some(trunk_name) = trunk {
            if let Some(trunk_indices) = self.trunk_rule_map.get(trunk_name) {
                // Trunk has explicit rule list — use only those
                Box::new(trunk_indices.iter().copied())
            } else {
                // Trunk exists but has no explicit rule list — use all rules
                Box::new(0..self.rules.len())
            }
        } else {
            // No trunk context — use all rules
            Box::new(0..self.rules.len())
        };

        indices
            .filter_map(|idx| {
                let rule = &self.rules[idx];
                // Check direction filter
                match rule.direction {
                    HmrDirection::Both => Some(rule),
                    d if d == direction => Some(rule),
                    _ => None,
                }
            })
            .collect()
    }
}

/// The rules a trunk names, for a node that builds its own outbound
/// INVITEs (`docs/TENANT_TRUNKS.md` §6). The call manager hands its engine
/// to `fcp-sip` through this.
impl fcp_sip::trunk_egress::TrunkHeaderRules for HeaderManipulationEngine {
    fn apply_egress(&self, names: &[String], request: &mut Request) {
        self.apply_request_egress_named(request, names);
    }

    fn apply_ingress(&self, names: &[String], request: &mut Request) {
        self.apply_request_ingress_named(request, names);
    }
}

/// Evaluate all conditions against a request. Returns true if all pass (AND logic).
fn evaluate_conditions_request(
    conditions: &[CompiledCondition],
    req: &Request,
    method_str: &str,
    uri_str: &str,
) -> bool {
    for cond in conditions {
        let value = match cond.header_name.as_str() {
            "$request-uri" => Some(uri_str.to_string()),
            "$method" => Some(method_str.to_string()),
            "$status-code" => None, // Not applicable for requests
            header_name => req.headers().get(header_name).map(|v| v.to_string()),
        };

        let matched = match &value {
            Some(v) => cond.regex.is_match(v),
            None => false,
        };

        let passes = if cond.negate { !matched } else { matched };
        if !passes {
            return false;
        }
    }
    true
}

/// Evaluate all conditions against a response. Returns true if all pass (AND logic).
fn evaluate_conditions_response(
    conditions: &[CompiledCondition],
    resp: &Response,
    status_str: &str,
) -> bool {
    for cond in conditions {
        let value = match cond.header_name.as_str() {
            "$status-code" => Some(status_str.to_string()),
            "$request-uri" | "$method" => None, // Not applicable for responses
            header_name => resp.headers().get(header_name).map(|v| v.to_string()),
        };

        let matched = match &value {
            Some(v) => cond.regex.is_match(v),
            None => false,
        };

        let passes = if cond.negate { !matched } else { matched };
        if !passes {
            return false;
        }
    }
    true
}

/// Execute a single action on a request.
fn execute_action_request(action: &CompiledAction, req: &mut Request) {
    if action.header_name == "$request-uri" {
        match action.action_type {
            HmrActionType::Replace => {
                if let Some(ref val) = action.value {
                    if let Ok(uri) = Uri::parse(val) {
                        req.set_uri(uri);
                    } else {
                        warn!(value = %val, "HMR: failed to parse replacement URI");
                    }
                }
            }
            HmrActionType::Modify => {
                if let (Some(ref pattern), Some(ref replacement)) = (&action.pattern, &action.value)
                {
                    let current = req.uri().to_string();
                    let new_val = pattern.replace(&current, replacement);
                    if let Ok(uri) = Uri::parse(&new_val) {
                        req.set_uri(uri);
                    } else {
                        warn!(value = %new_val, "HMR: failed to parse modified URI");
                    }
                }
            }
            _ => {
                // Add/Remove don't apply to $request-uri
            }
        }
        return;
    }

    match action.action_type {
        HmrActionType::Add => {
            if let Some(ref val) = action.value {
                let _ = req.headers_mut().push(&action.header_name, val);
            }
        }
        HmrActionType::Remove => {
            req.headers_mut().remove(&action.header_name);
        }
        HmrActionType::Replace => {
            if let Some(ref val) = action.value {
                let _ = req.headers_mut().replace_first(&action.header_name, val);
            }
        }
        HmrActionType::Modify => {
            if let (Some(ref pattern), Some(ref replacement)) = (&action.pattern, &action.value) {
                // Read current value into owned string to avoid borrow conflict
                let current_val = req
                    .headers()
                    .get(&action.header_name)
                    .map(|v| v.to_string());
                if let Some(current) = current_val {
                    let new_val = pattern.replace(&current, replacement);
                    let _ = req
                        .headers_mut()
                        .replace_first(&action.header_name, new_val.as_ref());
                }
            }
        }
    }
}

/// Execute a single action on a response.
fn execute_action_response(action: &CompiledAction, resp: &mut Response) {
    // $request-uri actions are not applicable to responses
    if action.header_name == "$request-uri" {
        return;
    }

    match action.action_type {
        HmrActionType::Add => {
            if let Some(ref val) = action.value {
                let _ = resp.headers_mut().push(&action.header_name, val);
            }
        }
        HmrActionType::Remove => {
            resp.headers_mut().remove(&action.header_name);
        }
        HmrActionType::Replace => {
            if let Some(ref val) = action.value {
                let _ = resp.headers_mut().replace_first(&action.header_name, val);
            }
        }
        HmrActionType::Modify => {
            if let (Some(ref pattern), Some(ref replacement)) = (&action.pattern, &action.value) {
                // Read current value into owned string to avoid borrow conflict
                let current_val = resp
                    .headers()
                    .get(&action.header_name)
                    .map(|v| v.to_string());
                if let Some(current) = current_val {
                    let new_val = pattern.replace(&current, replacement);
                    let _ = resp
                        .headers_mut()
                        .replace_first(&action.header_name, new_val.as_ref());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hmr::*;
    use bytes::Bytes;
    use sip_core::{Headers, Method, RequestLine, StatusLine};

    fn make_request(method: Method, uri: &str) -> Request {
        let uri = Uri::parse(uri).unwrap();
        let rl = RequestLine::new(method, uri);
        let mut headers = Headers::new();
        headers
            .push("Via", "SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776")
            .unwrap();
        headers
            .push("From", "<sip:alice@example.com>;tag=abc")
            .unwrap();
        headers.push("To", "<sip:bob@example.com>").unwrap();
        headers.push("Call-ID", "test123@10.0.0.1").unwrap();
        headers.push("CSeq", "1 INVITE").unwrap();
        Request::new(rl, headers, Bytes::new()).unwrap()
    }

    fn make_response(code: u16) -> Response {
        let sl = StatusLine::new(code, "OK").unwrap();
        let mut headers = Headers::new();
        headers
            .push("Via", "SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK776")
            .unwrap();
        headers
            .push("From", "<sip:alice@example.com>;tag=abc")
            .unwrap();
        headers.push("To", "<sip:bob@example.com>;tag=xyz").unwrap();
        headers.push("Call-ID", "test123@10.0.0.1").unwrap();
        headers.push("CSeq", "1 INVITE").unwrap();
        Response::new(sl, headers, Bytes::new()).unwrap()
    }

    fn simple_config(rules: Vec<HmrRule>) -> HeaderManipulationConfig {
        HeaderManipulationConfig {
            enabled: true,
            rules,
        }
    }

    #[test]
    fn test_compile_valid_rules() {
        let cfg = simple_config(vec![HmrRule {
            name: "test".into(),
            priority: 10,
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
        }]);
        assert!(HeaderManipulationEngine::new(&cfg, &[]).is_ok());
    }

    #[test]
    fn test_compile_invalid_regex() {
        let cfg = simple_config(vec![HmrRule {
            name: "bad-regex".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![HmrCondition {
                header_name: "Via".into(),
                pattern: "[invalid".into(),
                negate: false,
            }],
            actions: vec![HmrAction {
                action_type: HmrActionType::Remove,
                header_name: "X-Test".into(),
                value: None,
                pattern: None,
            }],
        }]);
        let err = HeaderManipulationEngine::new(&cfg, &[]).unwrap_err();
        assert!(err.contains("invalid regex"));
    }

    #[test]
    fn test_add_header() {
        let cfg = simple_config(vec![HmrRule {
            name: "add".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Custom".into(),
                value: Some("hello".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        assert!(req.headers().get("X-Custom").is_none());

        engine.apply_request_ingress(&mut req, None);
        assert_eq!(req.headers().get("X-Custom"), Some("hello"));
    }

    #[test]
    fn test_remove_header() {
        let cfg = simple_config(vec![HmrRule {
            name: "remove".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Remove,
                header_name: "Call-ID".into(),
                value: None,
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        assert!(req.headers().get("Call-ID").is_some());

        engine.apply_request_ingress(&mut req, None);
        assert!(req.headers().get("Call-ID").is_none());
    }

    #[test]
    fn test_replace_header() {
        let cfg = simple_config(vec![HmrRule {
            name: "replace".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Replace,
                header_name: "Call-ID".into(),
                value: Some("new-call-id@10.0.0.2".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");

        engine.apply_request_ingress(&mut req, None);
        assert_eq!(req.headers().get("Call-ID"), Some("new-call-id@10.0.0.2"));
    }

    #[test]
    fn test_modify_header_regex() {
        let cfg = simple_config(vec![HmrRule {
            name: "modify".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Modify,
                header_name: "From".into(),
                value: Some("<sip:rewritten@new.com>;tag=abc".into()),
                pattern: Some("<sip:[^>]+>;tag=abc".into()),
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");

        engine.apply_request_ingress(&mut req, None);
        assert_eq!(
            req.headers().get("From"),
            Some("<sip:rewritten@new.com>;tag=abc")
        );
    }

    #[test]
    fn test_condition_matching() {
        let cfg = simple_config(vec![HmrRule {
            name: "conditional".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![HmrCondition {
                header_name: "From".into(),
                pattern: "alice".into(),
                negate: false,
            }],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Matched".into(),
                value: Some("yes".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();

        // Request with alice in From — should match
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req, None);
        assert_eq!(req.headers().get("X-Matched"), Some("yes"));
    }

    #[test]
    fn test_condition_no_match() {
        let cfg = simple_config(vec![HmrRule {
            name: "conditional".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![HmrCondition {
                header_name: "From".into(),
                pattern: "charlie".into(),
                negate: false,
            }],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Matched".into(),
                value: Some("yes".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req, None);
        // Should NOT have added the header since condition didn't match
        assert!(req.headers().get("X-Matched").is_none());
    }

    #[test]
    fn test_condition_negation() {
        let cfg = simple_config(vec![HmrRule {
            name: "negate".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![HmrCondition {
                header_name: "From".into(),
                pattern: "charlie".into(),
                negate: true, // passes when NOT charlie
            }],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Not-Charlie".into(),
                value: Some("true".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req, None);
        // From is alice, not charlie — negated condition passes
        assert_eq!(req.headers().get("X-Not-Charlie"), Some("true"));
    }

    #[test]
    fn test_method_filter() {
        let cfg = simple_config(vec![HmrRule {
            name: "invite-only".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Request,
            methods: vec!["INVITE".into()],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Invite".into(),
                value: Some("yes".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();

        // INVITE should match
        let mut invite_req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut invite_req, None);
        assert_eq!(invite_req.headers().get("X-Invite"), Some("yes"));

        // REGISTER should not match
        let mut reg_req = make_request(Method::Register, "sip:example.com");
        engine.apply_request_ingress(&mut reg_req, None);
        assert!(reg_req.headers().get("X-Invite").is_none());
    }

    #[test]
    fn test_direction_filter() {
        let cfg = simple_config(vec![HmrRule {
            name: "egress-only".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Egress,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Egress".into(),
                value: Some("yes".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();

        // Ingress should NOT apply the rule
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req, None);
        assert!(req.headers().get("X-Egress").is_none());

        // Egress SHOULD apply the rule
        let mut req2 = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_egress(&mut req2, None);
        assert_eq!(req2.headers().get("X-Egress"), Some("yes"));
    }

    #[test]
    fn test_message_type_filter() {
        let cfg = simple_config(vec![HmrRule {
            name: "response-only".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Response,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Resp".into(),
                value: Some("yes".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();

        // Request should NOT match
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req, None);
        assert!(req.headers().get("X-Resp").is_none());

        // Response SHOULD match
        let mut resp = make_response(200);
        engine.apply_response_ingress(&mut resp, None);
        assert_eq!(resp.headers().get("X-Resp"), Some("yes"));
    }

    #[test]
    fn test_trunk_scoped_rules() {
        let cfg = simple_config(vec![
            HmrRule {
                name: "rule-a".into(),
                priority: 10,
                enabled: true,
                direction: HmrDirection::Both,
                message_type: HmrMessageType::Any,
                methods: vec![],
                conditions: vec![],
                actions: vec![HmrAction {
                    action_type: HmrActionType::Add,
                    header_name: "X-Rule-A".into(),
                    value: Some("yes".into()),
                    pattern: None,
                }],
            },
            HmrRule {
                name: "rule-b".into(),
                priority: 20,
                enabled: true,
                direction: HmrDirection::Both,
                message_type: HmrMessageType::Any,
                methods: vec![],
                conditions: vec![],
                actions: vec![HmrAction {
                    action_type: HmrActionType::Add,
                    header_name: "X-Rule-B".into(),
                    value: Some("yes".into()),
                    pattern: None,
                }],
            },
        ]);

        // Trunk only uses rule-a
        let trunks = vec![TrunkConfig {
            name: "carrier-a".into(),
            hosts: vec!["sip.carrier.com".into()],
            header_rules: vec!["rule-a".into()],
            emergency_capable: false,
            ..Default::default()
        }];

        let engine = HeaderManipulationEngine::new(&cfg, &trunks).unwrap();

        // With trunk scoping — only rule-a should apply
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req, Some("carrier-a"));
        assert_eq!(req.headers().get("X-Rule-A"), Some("yes"));
        assert!(req.headers().get("X-Rule-B").is_none());

        // Without trunk scoping — both rules apply
        let mut req2 = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req2, None);
        assert_eq!(req2.headers().get("X-Rule-A"), Some("yes"));
        assert_eq!(req2.headers().get("X-Rule-B"), Some("yes"));
    }

    #[test]
    fn test_request_uri_manipulation() {
        let cfg = simple_config(vec![HmrRule {
            name: "rewrite-uri".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Request,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Modify,
                header_name: "$request-uri".into(),
                value: Some("sip:$1@new.domain.com".into()),
                pattern: Some("sip:(.+)@old\\.domain\\.com".into()),
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@old.domain.com");

        engine.apply_request_ingress(&mut req, None);
        assert_eq!(req.uri().to_string(), "sip:bob@new.domain.com");
    }

    #[test]
    fn test_request_uri_replace() {
        let cfg = simple_config(vec![HmrRule {
            name: "replace-uri".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Request,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Replace,
                header_name: "$request-uri".into(),
                value: Some("sip:new-target@example.com".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");

        engine.apply_request_ingress(&mut req, None);
        assert_eq!(req.uri().to_string(), "sip:new-target@example.com");
    }

    #[test]
    fn test_priority_ordering() {
        // Rule at priority 20 adds X-Order: second
        // Rule at priority 10 adds X-Order: first
        // Both should fire, and the lower priority (10) fires first
        let cfg = simple_config(vec![
            HmrRule {
                name: "second".into(),
                priority: 20,
                enabled: true,
                direction: HmrDirection::Both,
                message_type: HmrMessageType::Any,
                methods: vec![],
                conditions: vec![],
                actions: vec![HmrAction {
                    action_type: HmrActionType::Add,
                    header_name: "X-Order".into(),
                    value: Some("second".into()),
                    pattern: None,
                }],
            },
            HmrRule {
                name: "first".into(),
                priority: 10,
                enabled: true,
                direction: HmrDirection::Both,
                message_type: HmrMessageType::Any,
                methods: vec![],
                conditions: vec![],
                actions: vec![HmrAction {
                    action_type: HmrActionType::Add,
                    header_name: "X-Order".into(),
                    value: Some("first".into()),
                    pattern: None,
                }],
            },
        ]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req, None);

        // Both headers should be present, with "first" appearing before "second"
        let values: Vec<&str> = req.headers().get_all("X-Order").collect();
        assert_eq!(values, vec!["first", "second"]);
    }

    #[test]
    fn test_no_rules_noop() {
        let cfg = simple_config(vec![]);
        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();

        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        let orig_headers_count = req.headers().len();
        engine.apply_request_ingress(&mut req, None);
        assert_eq!(req.headers().len(), orig_headers_count);
    }

    #[test]
    fn test_disabled_rule_skipped() {
        let cfg = simple_config(vec![HmrRule {
            name: "disabled".into(),
            priority: 10,
            enabled: false,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Disabled".into(),
                value: Some("should-not-appear".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req, None);
        assert!(req.headers().get("X-Disabled").is_none());
    }

    #[test]
    fn test_response_add_header() {
        let cfg = simple_config(vec![HmrRule {
            name: "resp-add".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Response,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Server".into(),
                value: Some("fcp-sbc".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut resp = make_response(200);
        engine.apply_response_egress(&mut resp, None);
        assert_eq!(resp.headers().get("X-Server"), Some("fcp-sbc"));
    }

    #[test]
    fn test_response_condition_status_code() {
        let cfg = simple_config(vec![HmrRule {
            name: "on-200".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Response,
            methods: vec![],
            conditions: vec![HmrCondition {
                header_name: "$status-code".into(),
                pattern: "^200$".into(),
                negate: false,
            }],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Success".into(),
                value: Some("true".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();

        // 200 response should match
        let mut resp200 = make_response(200);
        engine.apply_response_ingress(&mut resp200, None);
        assert_eq!(resp200.headers().get("X-Success"), Some("true"));

        // 404 response should not match
        let mut resp404 = make_response(404);
        engine.apply_response_ingress(&mut resp404, None);
        assert!(resp404.headers().get("X-Success").is_none());
    }

    #[test]
    fn test_method_condition_on_request() {
        let cfg = simple_config(vec![HmrRule {
            name: "method-cond".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Request,
            methods: vec![],
            conditions: vec![HmrCondition {
                header_name: "$method".into(),
                pattern: "^INVITE$".into(),
                negate: false,
            }],
            actions: vec![HmrAction {
                action_type: HmrActionType::Add,
                header_name: "X-Is-Invite".into(),
                value: Some("yes".into()),
                pattern: None,
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();

        let mut invite = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut invite, None);
        assert_eq!(invite.headers().get("X-Is-Invite"), Some("yes"));

        let mut options = make_request(Method::Options, "sip:example.com");
        engine.apply_request_ingress(&mut options, None);
        assert!(options.headers().get("X-Is-Invite").is_none());
    }

    #[test]
    fn test_modify_with_capture_groups() {
        let cfg = simple_config(vec![HmrRule {
            name: "capture".into(),
            priority: 10,
            enabled: true,
            direction: HmrDirection::Both,
            message_type: HmrMessageType::Any,
            methods: vec![],
            conditions: vec![],
            actions: vec![HmrAction {
                action_type: HmrActionType::Modify,
                header_name: "From".into(),
                value: Some("<sip:$1@newdomain.com>;tag=abc".into()),
                pattern: Some("<sip:([^@]+)@[^>]+>;tag=abc".into()),
            }],
        }]);

        let engine = HeaderManipulationEngine::new(&cfg, &[]).unwrap();
        let mut req = make_request(Method::Invite, "sip:bob@example.com");
        engine.apply_request_ingress(&mut req, None);
        assert_eq!(
            req.headers().get("From"),
            Some("<sip:alice@newdomain.com>;tag=abc")
        );
    }
}
