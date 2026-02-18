//! XCAP-compatible pres-rules (RFC 4825/5025) XML generation and parsing.
//!
//! Maps between `PresenceAuthPolicy` and the RFC 5025 pres-rules XML format.
//! This is not a full XCAP server but provides document format compatibility.

use anyhow::{bail, Result};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Reader;
use quick_xml::Writer;
use std::collections::HashSet;
use std::io::Cursor;

use crate::types::PresenceAuthPolicy;

const CR_NS: &str = "urn:ietf:params:xml:ns:common-policy";
const PR_NS: &str = "urn:ietf:params:xml:ns:pres-rules";

/// Generate pres-rules XML (RFC 5025) from a `PresenceAuthPolicy`.
pub fn generate_pres_rules(policy: &PresenceAuthPolicy) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    // XML declaration
    writer
        .write_event(Event::Decl(quick_xml::events::BytesDecl::new(
            "1.0",
            Some("UTF-8"),
            None,
        )))
        .expect("write xml decl");

    // <cr:ruleset xmlns:cr="..." xmlns:pr="...">
    let mut ruleset = BytesStart::new("cr:ruleset");
    ruleset.push_attribute(("xmlns:cr", CR_NS));
    ruleset.push_attribute(("xmlns:pr", PR_NS));
    writer
        .write_event(Event::Start(ruleset))
        .expect("write ruleset start");

    // Default rule
    let default_action = if policy.default_allow {
        "allow"
    } else {
        "block"
    };

    let mut rule = BytesStart::new("cr:rule");
    rule.push_attribute(("id", "pres_default"));
    writer
        .write_event(Event::Start(rule))
        .expect("write rule start");

    // <cr:conditions>
    writer
        .write_event(Event::Start(BytesStart::new("cr:conditions")))
        .expect("write conditions start");
    // Empty conditions = applies to all
    writer
        .write_event(Event::End(BytesEnd::new("cr:conditions")))
        .expect("write conditions end");

    // <cr:actions>
    writer
        .write_event(Event::Start(BytesStart::new("cr:actions")))
        .expect("write actions start");
    // <pr:sub-handling>allow|block</pr:sub-handling>
    writer
        .write_event(Event::Start(BytesStart::new("pr:sub-handling")))
        .expect("write sub-handling start");
    writer
        .write_event(Event::Text(BytesText::new(default_action)))
        .expect("write sub-handling text");
    writer
        .write_event(Event::End(BytesEnd::new("pr:sub-handling")))
        .expect("write sub-handling end");
    writer
        .write_event(Event::End(BytesEnd::new("cr:actions")))
        .expect("write actions end");

    // </cr:rule>
    writer
        .write_event(Event::End(BytesEnd::new("cr:rule")))
        .expect("write rule end");

    // Allow list entries
    for (i, aor) in policy.allow_list.iter().enumerate() {
        write_identity_rule(&mut writer, &format!("pres_allow_{i}"), aor, "allow");
    }

    // Block list entries
    for (i, aor) in policy.block_list.iter().enumerate() {
        write_identity_rule(&mut writer, &format!("pres_block_{i}"), aor, "block");
    }

    // </cr:ruleset>
    writer
        .write_event(Event::End(BytesEnd::new("cr:ruleset")))
        .expect("write ruleset end");

    let result = writer.into_inner().into_inner();
    String::from_utf8(result).expect("valid utf-8")
}

fn write_identity_rule(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    rule_id: &str,
    aor: &str,
    action: &str,
) {
    let mut rule = BytesStart::new("cr:rule");
    rule.push_attribute(("id", rule_id));
    writer
        .write_event(Event::Start(rule))
        .expect("write rule start");

    // <cr:conditions><cr:identity><cr:one id="sip:aor"/></cr:identity></cr:conditions>
    writer
        .write_event(Event::Start(BytesStart::new("cr:conditions")))
        .expect("write conditions start");
    writer
        .write_event(Event::Start(BytesStart::new("cr:identity")))
        .expect("write identity start");

    let mut one = BytesStart::new("cr:one");
    one.push_attribute(("id", format!("sip:{aor}").as_str()));
    writer
        .write_event(Event::Empty(one))
        .expect("write one");

    writer
        .write_event(Event::End(BytesEnd::new("cr:identity")))
        .expect("write identity end");
    writer
        .write_event(Event::End(BytesEnd::new("cr:conditions")))
        .expect("write conditions end");

    // <cr:actions><pr:sub-handling>allow|block</pr:sub-handling></cr:actions>
    writer
        .write_event(Event::Start(BytesStart::new("cr:actions")))
        .expect("write actions start");
    writer
        .write_event(Event::Start(BytesStart::new("pr:sub-handling")))
        .expect("write sub-handling start");
    writer
        .write_event(Event::Text(BytesText::new(action)))
        .expect("write sub-handling text");
    writer
        .write_event(Event::End(BytesEnd::new("pr:sub-handling")))
        .expect("write sub-handling end");
    writer
        .write_event(Event::End(BytesEnd::new("cr:actions")))
        .expect("write actions end");

    writer
        .write_event(Event::End(BytesEnd::new("cr:rule")))
        .expect("write rule end");
}

/// Parse pres-rules XML (RFC 5025) into a `PresenceAuthPolicy`.
pub fn parse_pres_rules(xml: &str) -> Result<PresenceAuthPolicy> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut default_allow = true;
    let mut allow_list = HashSet::new();
    let mut block_list = HashSet::new();

    // Track parser state
    let mut in_rule = false;
    let mut in_conditions = false;
    let mut in_identity = false;
    let mut in_actions = false;
    let mut in_sub_handling = false;
    let mut current_identity: Option<String> = None;
    let mut current_action: Option<String> = None;
    let mut has_identity_condition = false;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let ln = e.local_name();
                let local_name = std::str::from_utf8(ln.as_ref()).unwrap_or("");
                match local_name {
                    "rule" => {
                        in_rule = true;
                        current_identity = None;
                        current_action = None;
                        has_identity_condition = false;
                    }
                    "conditions" if in_rule => {
                        in_conditions = true;
                    }
                    "identity" if in_conditions => {
                        in_identity = true;
                        has_identity_condition = true;
                    }
                    "one" if in_identity => {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"id" {
                                let val = std::str::from_utf8(&attr.value).unwrap_or("");
                                // Strip "sip:" prefix
                                let aor = val.strip_prefix("sip:").unwrap_or(val);
                                current_identity = Some(aor.to_string());
                            }
                        }
                    }
                    "actions" if in_rule => {
                        in_actions = true;
                    }
                    "sub-handling" if in_actions => {
                        in_sub_handling = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                if in_sub_handling {
                    let text = e.unescape().unwrap_or_default().to_string();
                    current_action = Some(text);
                }
            }
            Ok(Event::End(ref e)) => {
                let ln = e.local_name();
                let local_name = std::str::from_utf8(ln.as_ref()).unwrap_or("");
                match local_name {
                    "rule" => {
                        // Process the completed rule
                        let action = current_action.as_deref().unwrap_or("allow");
                        if !has_identity_condition {
                            // Default rule (no identity condition)
                            default_allow = action == "allow";
                        } else if let Some(ref aor) = current_identity {
                            match action {
                                "allow" => {
                                    allow_list.insert(aor.clone());
                                }
                                "block" | "polite-block" | "confirm" => {
                                    block_list.insert(aor.clone());
                                }
                                _ => {}
                            }
                        }
                        in_rule = false;
                    }
                    "conditions" => in_conditions = false,
                    "identity" => in_identity = false,
                    "actions" => in_actions = false,
                    "sub-handling" => in_sub_handling = false,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => bail!("Error parsing pres-rules XML: {e}"),
            _ => {}
        }
        buf.clear();
    }

    Ok(PresenceAuthPolicy {
        default_allow,
        allow_list,
        block_list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_pres_rules_default_allow() {
        let policy = PresenceAuthPolicy {
            default_allow: true,
            allow_list: HashSet::new(),
            block_list: HashSet::from(["bob@example.com".to_string()]),
        };
        let xml = generate_pres_rules(&policy);
        assert!(xml.contains("urn:ietf:params:xml:ns:common-policy"));
        assert!(xml.contains("urn:ietf:params:xml:ns:pres-rules"));
        assert!(xml.contains("<pr:sub-handling>allow</pr:sub-handling>"));
        assert!(xml.contains("sip:bob@example.com"));
        assert!(xml.contains("<pr:sub-handling>block</pr:sub-handling>"));
    }

    #[test]
    fn test_generate_pres_rules_default_block() {
        let policy = PresenceAuthPolicy {
            default_allow: false,
            allow_list: HashSet::from(["alice@example.com".to_string()]),
            block_list: HashSet::new(),
        };
        let xml = generate_pres_rules(&policy);
        // Default rule should be "block"
        assert!(xml.contains("pres_default"));
        assert!(xml.contains("sip:alice@example.com"));
    }

    #[test]
    fn test_roundtrip_pres_rules() {
        let policy = PresenceAuthPolicy {
            default_allow: true,
            allow_list: HashSet::from(["charlie@example.com".to_string()]),
            block_list: HashSet::from(["bob@example.com".to_string()]),
        };

        let xml = generate_pres_rules(&policy);
        let parsed = parse_pres_rules(&xml).unwrap();

        assert_eq!(parsed.default_allow, policy.default_allow);
        assert_eq!(parsed.allow_list, policy.allow_list);
        assert_eq!(parsed.block_list, policy.block_list);
    }

    #[test]
    fn test_roundtrip_pres_rules_block_default() {
        let policy = PresenceAuthPolicy {
            default_allow: false,
            allow_list: HashSet::from(["alice@example.com".to_string()]),
            block_list: HashSet::new(),
        };

        let xml = generate_pres_rules(&policy);
        let parsed = parse_pres_rules(&xml).unwrap();

        assert_eq!(parsed.default_allow, false);
        assert_eq!(parsed.allow_list, policy.allow_list);
        assert!(parsed.block_list.is_empty());
    }

    #[test]
    fn test_parse_empty_policy() {
        let policy = PresenceAuthPolicy {
            default_allow: true,
            allow_list: HashSet::new(),
            block_list: HashSet::new(),
        };
        let xml = generate_pres_rules(&policy);
        let parsed = parse_pres_rules(&xml).unwrap();
        assert!(parsed.default_allow);
        assert!(parsed.allow_list.is_empty());
        assert!(parsed.block_list.is_empty());
    }

    #[test]
    fn test_parse_invalid_xml() {
        let result = parse_pres_rules("not xml at all <<<");
        assert!(result.is_err());
    }
}
