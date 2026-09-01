//! XCAP-compatible pres-rules (RFC 4825/5025) XML generation and parsing.
//!
//! Maps between `PresenceAuthPolicy` and the RFC 5025 pres-rules XML format.
//! This is not a full XCAP server but provides document format compatibility.

use anyhow::{bail, Result};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Reader;
use quick_xml::Writer;
use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use crate::types::{PresenceAuthPolicy, TimeWindow};

const CR_NS: &str = "urn:ietf:params:xml:ns:common-policy";
const PR_NS: &str = "urn:ietf:params:xml:ns:pres-rules";
const RL_NS: &str = "urn:ietf:params:xml:ns:resource-lists";

// ============================================================================
// Resource Lists (RFC 4826)
// ============================================================================

/// A single resource list with optional name and entries.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceListXml {
    pub name: Option<String>,
    pub entries: Vec<ResourceListEntryXml>,
}

/// An entry within a resource list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResourceListEntryXml {
    pub uri: String,
    pub display_name: Option<String>,
}

/// Generate RFC 4826 resource-lists XML from a slice of lists.
pub fn generate_resource_lists_xml(lists: &[ResourceListXml]) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    writer
        .write_event(Event::Decl(quick_xml::events::BytesDecl::new(
            "1.0",
            Some("UTF-8"),
            None,
        )))
        .expect("write xml decl");

    let mut root = BytesStart::new("resource-lists");
    root.push_attribute(("xmlns", RL_NS));
    writer
        .write_event(Event::Start(root))
        .expect("write resource-lists start");

    for list in lists {
        let mut list_elem = BytesStart::new("list");
        if let Some(ref name) = list.name {
            list_elem.push_attribute(("name", name.as_str()));
        }
        writer
            .write_event(Event::Start(list_elem))
            .expect("write list start");

        for entry in &list.entries {
            let mut entry_elem = BytesStart::new("entry");
            entry_elem.push_attribute(("uri", entry.uri.as_str()));

            if let Some(ref dn) = entry.display_name {
                writer
                    .write_event(Event::Start(entry_elem))
                    .expect("write entry start");
                writer
                    .write_event(Event::Start(BytesStart::new("display-name")))
                    .expect("write display-name start");
                writer
                    .write_event(Event::Text(BytesText::new(dn)))
                    .expect("write display-name text");
                writer
                    .write_event(Event::End(BytesEnd::new("display-name")))
                    .expect("write display-name end");
                writer
                    .write_event(Event::End(BytesEnd::new("entry")))
                    .expect("write entry end");
            } else {
                writer
                    .write_event(Event::Empty(entry_elem))
                    .expect("write entry empty");
            }
        }

        writer
            .write_event(Event::End(BytesEnd::new("list")))
            .expect("write list end");
    }

    writer
        .write_event(Event::End(BytesEnd::new("resource-lists")))
        .expect("write resource-lists end");

    let result = writer.into_inner().into_inner();
    String::from_utf8(result).expect("valid utf-8")
}

/// Parse RFC 4826 resource-lists XML into a vector of lists.
pub fn parse_resource_lists_xml(xml: &str) -> Result<Vec<ResourceListXml>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut lists = Vec::new();
    let mut current_list: Option<ResourceListXml> = None;
    let mut current_entry_uri: Option<String> = None;
    let mut in_display_name = false;
    let mut current_display_name: Option<String> = None;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let ln = e.local_name();
                let local_name = std::str::from_utf8(ln.as_ref()).unwrap_or("");
                match local_name {
                    "list" => {
                        let mut name = None;
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"name" {
                                name = Some(
                                    std::str::from_utf8(&attr.value).unwrap_or("").to_string(),
                                );
                            }
                        }
                        current_list = Some(ResourceListXml {
                            name,
                            entries: Vec::new(),
                        });
                    }
                    "entry" => {
                        let mut uri = String::new();
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"uri" {
                                uri = std::str::from_utf8(&attr.value).unwrap_or("").to_string();
                            }
                        }
                        current_entry_uri = Some(uri);
                        current_display_name = None;
                    }
                    "display-name" => {
                        in_display_name = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let ln = e.local_name();
                let local_name = std::str::from_utf8(ln.as_ref()).unwrap_or("");
                if local_name == "entry" {
                    let mut uri = String::new();
                    for attr in e.attributes().flatten() {
                        if attr.key.local_name().as_ref() == b"uri" {
                            uri = std::str::from_utf8(&attr.value).unwrap_or("").to_string();
                        }
                    }
                    if let Some(ref mut list) = current_list {
                        list.entries.push(ResourceListEntryXml {
                            uri,
                            display_name: None,
                        });
                    }
                }
            }
            Ok(Event::Text(ref e)) => {
                if in_display_name {
                    current_display_name = Some(e.decode().unwrap_or_default().to_string());
                }
            }
            Ok(Event::End(ref e)) => {
                let ln = e.local_name();
                let local_name = std::str::from_utf8(ln.as_ref()).unwrap_or("");
                match local_name {
                    "list" => {
                        if let Some(list) = current_list.take() {
                            lists.push(list);
                        }
                    }
                    "entry" => {
                        if let (Some(ref mut list), Some(uri)) =
                            (&mut current_list, current_entry_uri.take())
                        {
                            list.entries.push(ResourceListEntryXml {
                                uri,
                                display_name: current_display_name.take(),
                            });
                        }
                    }
                    "display-name" => {
                        in_display_name = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => bail!("Error parsing resource-lists XML: {e}"),
            _ => {}
        }
        buf.clear();
    }

    Ok(lists)
}

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

    // Event-package-specific allow rules
    for (event, aors) in &policy.event_allow {
        for (i, aor) in aors.iter().enumerate() {
            write_event_identity_rule(
                &mut writer,
                &format!("pres_event_allow_{event}_{i}"),
                aor,
                event,
                "allow",
            );
        }
    }

    // Event-package-specific block rules
    for (event, aors) in &policy.event_block {
        for (i, aor) in aors.iter().enumerate() {
            write_event_identity_rule(
                &mut writer,
                &format!("pres_event_block_{event}_{i}"),
                aor,
                event,
                "block",
            );
        }
    }

    // Allow patterns
    for (i, pattern) in policy.allow_patterns.iter().enumerate() {
        write_pattern_rule(
            &mut writer,
            &format!("pres_pattern_allow_{i}"),
            pattern,
            "allow",
        );
    }

    // Block patterns
    for (i, pattern) in policy.block_patterns.iter().enumerate() {
        write_pattern_rule(
            &mut writer,
            &format!("pres_pattern_block_{i}"),
            pattern,
            "block",
        );
    }

    // Time windows
    if !policy.time_windows.is_empty() {
        write_time_window_rules(&mut writer, &policy.time_windows);
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
    writer.write_event(Event::Empty(one)).expect("write one");

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

/// Write a rule with an identity condition and event-package scope.
fn write_event_identity_rule(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    rule_id: &str,
    aor: &str,
    event_package: &str,
    action: &str,
) {
    let mut rule = BytesStart::new("cr:rule");
    rule.push_attribute(("id", rule_id));
    writer
        .write_event(Event::Start(rule))
        .expect("write rule start");

    // <cr:conditions><cr:identity><cr:one id="sip:aor"/></cr:identity>
    // <pr:event-package>event</pr:event-package></cr:conditions>
    writer
        .write_event(Event::Start(BytesStart::new("cr:conditions")))
        .expect("write conditions start");
    writer
        .write_event(Event::Start(BytesStart::new("cr:identity")))
        .expect("write identity start");
    let mut one = BytesStart::new("cr:one");
    one.push_attribute(("id", format!("sip:{aor}").as_str()));
    writer.write_event(Event::Empty(one)).expect("write one");
    writer
        .write_event(Event::End(BytesEnd::new("cr:identity")))
        .expect("write identity end");

    writer
        .write_event(Event::Start(BytesStart::new("pr:event-package")))
        .expect("write event-package start");
    writer
        .write_event(Event::Text(BytesText::new(event_package)))
        .expect("write event-package text");
    writer
        .write_event(Event::End(BytesEnd::new("pr:event-package")))
        .expect("write event-package end");

    writer
        .write_event(Event::End(BytesEnd::new("cr:conditions")))
        .expect("write conditions end");

    // <cr:actions><pr:sub-handling>action</pr:sub-handling></cr:actions>
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

/// Write a pattern-based rule (domain wildcard).
fn write_pattern_rule(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    rule_id: &str,
    pattern: &str,
    action: &str,
) {
    let mut rule = BytesStart::new("cr:rule");
    rule.push_attribute(("id", rule_id));
    writer
        .write_event(Event::Start(rule))
        .expect("write rule start");

    // <cr:conditions><pr:domain-pattern>pattern</pr:domain-pattern></cr:conditions>
    writer
        .write_event(Event::Start(BytesStart::new("cr:conditions")))
        .expect("write conditions start");
    writer
        .write_event(Event::Start(BytesStart::new("pr:domain-pattern")))
        .expect("write domain-pattern start");
    writer
        .write_event(Event::Text(BytesText::new(pattern)))
        .expect("write domain-pattern text");
    writer
        .write_event(Event::End(BytesEnd::new("pr:domain-pattern")))
        .expect("write domain-pattern end");
    writer
        .write_event(Event::End(BytesEnd::new("cr:conditions")))
        .expect("write conditions end");

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

/// Write time-window rules.
fn write_time_window_rules(writer: &mut Writer<Cursor<Vec<u8>>>, windows: &[TimeWindow]) {
    let mut rule = BytesStart::new("cr:rule");
    rule.push_attribute(("id", "pres_time_windows"));
    writer
        .write_event(Event::Start(rule))
        .expect("write rule start");

    writer
        .write_event(Event::Start(BytesStart::new("cr:conditions")))
        .expect("write conditions start");
    for (i, w) in windows.iter().enumerate() {
        let mut tw = BytesStart::new("pr:time-window");
        tw.push_attribute(("id", format!("tw_{i}").as_str()));
        tw.push_attribute(("start-hour", w.start_hour.to_string().as_str()));
        tw.push_attribute(("end-hour", w.end_hour.to_string().as_str()));
        if w.days.is_empty() {
            writer
                .write_event(Event::Empty(tw))
                .expect("write time-window");
        } else {
            writer
                .write_event(Event::Start(tw))
                .expect("write time-window start");
            let days_str = w
                .days
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(",");
            writer
                .write_event(Event::Start(BytesStart::new("pr:days")))
                .expect("write days start");
            writer
                .write_event(Event::Text(BytesText::new(&days_str)))
                .expect("write days text");
            writer
                .write_event(Event::End(BytesEnd::new("pr:days")))
                .expect("write days end");
            writer
                .write_event(Event::End(BytesEnd::new("pr:time-window")))
                .expect("write time-window end");
        }
    }
    writer
        .write_event(Event::End(BytesEnd::new("cr:conditions")))
        .expect("write conditions end");

    // Time windows don't have an action — they're conditions on the default rule
    writer
        .write_event(Event::Start(BytesStart::new("cr:actions")))
        .expect("write actions start");
    writer
        .write_event(Event::Start(BytesStart::new("pr:sub-handling")))
        .expect("write sub-handling start");
    writer
        .write_event(Event::Text(BytesText::new("allow")))
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
    let mut event_allow: HashMap<String, HashSet<String>> = HashMap::new();
    let mut event_block: HashMap<String, HashSet<String>> = HashMap::new();
    let mut allow_patterns = Vec::new();
    let mut block_patterns = Vec::new();
    let mut time_windows = Vec::new();

    // Track parser state
    let mut in_rule = false;
    let mut in_conditions = false;
    let mut in_identity = false;
    let mut in_actions = false;
    let mut in_sub_handling = false;
    let mut in_event_package = false;
    let mut in_domain_pattern = false;
    let mut in_days = false;
    let mut current_identity: Option<String> = None;
    let mut current_action: Option<String> = None;
    let mut current_event_package: Option<String> = None;
    let mut current_domain_pattern: Option<String> = None;
    let mut has_identity_condition = false;
    let mut has_domain_pattern = false;
    let mut has_time_window = false;
    let mut current_time_windows: Vec<TimeWindow> = Vec::new();
    // Track the current time window being parsed (for days)
    let mut pending_tw_start: Option<u8> = None;
    let mut pending_tw_end: Option<u8> = None;

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
                        current_event_package = None;
                        current_domain_pattern = None;
                        has_identity_condition = false;
                        has_domain_pattern = false;
                        has_time_window = false;
                        current_time_windows.clear();
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
                                let aor = val.strip_prefix("sip:").unwrap_or(val);
                                current_identity = Some(aor.to_string());
                            }
                        }
                    }
                    "event-package" if in_conditions => {
                        in_event_package = true;
                    }
                    "domain-pattern" if in_conditions => {
                        in_domain_pattern = true;
                        has_domain_pattern = true;
                    }
                    "time-window" if in_conditions => {
                        has_time_window = true;
                        let mut start = 0u8;
                        let mut end = 0u8;
                        for attr in e.attributes().flatten() {
                            let local = attr.key.local_name();
                            let key = std::str::from_utf8(local.as_ref()).unwrap_or("");
                            let val = std::str::from_utf8(&attr.value).unwrap_or("0");
                            match key {
                                "start-hour" => start = val.parse().unwrap_or(0),
                                "end-hour" => end = val.parse().unwrap_or(0),
                                _ => {}
                            }
                        }
                        pending_tw_start = Some(start);
                        pending_tw_end = Some(end);
                    }
                    "days" if has_time_window => {
                        in_days = true;
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
                let text = e.decode().unwrap_or_default().to_string();
                if in_sub_handling {
                    current_action = Some(text);
                } else if in_event_package {
                    current_event_package = Some(text);
                } else if in_domain_pattern {
                    current_domain_pattern = Some(text);
                } else if in_days {
                    let days: Vec<u8> = text
                        .split(',')
                        .filter_map(|d| d.trim().parse().ok())
                        .collect();
                    if let (Some(start), Some(end)) =
                        (pending_tw_start.take(), pending_tw_end.take())
                    {
                        current_time_windows.push(TimeWindow {
                            start_hour: start,
                            end_hour: end,
                            days,
                        });
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let ln = e.local_name();
                let local_name = std::str::from_utf8(ln.as_ref()).unwrap_or("");
                match local_name {
                    "rule" => {
                        let action = current_action.as_deref().unwrap_or("allow");

                        if has_time_window && !current_time_windows.is_empty() {
                            time_windows.append(&mut current_time_windows);
                        } else if has_time_window {
                            // Time windows with no days — finalize any pending
                            if let (Some(start), Some(end)) =
                                (pending_tw_start.take(), pending_tw_end.take())
                            {
                                time_windows.push(TimeWindow {
                                    start_hour: start,
                                    end_hour: end,
                                    days: vec![],
                                });
                            }
                        } else if has_domain_pattern {
                            if let Some(ref pattern) = current_domain_pattern {
                                match action {
                                    "allow" => allow_patterns.push(pattern.clone()),
                                    _ => block_patterns.push(pattern.clone()),
                                }
                            }
                        } else if !has_identity_condition {
                            default_allow = action == "allow";
                        } else if let Some(ref aor) = current_identity {
                            if let Some(ref event_pkg) = current_event_package {
                                // Event-package-specific rule
                                match action {
                                    "allow" => {
                                        event_allow
                                            .entry(event_pkg.clone())
                                            .or_default()
                                            .insert(aor.clone());
                                    }
                                    _ => {
                                        event_block
                                            .entry(event_pkg.clone())
                                            .or_default()
                                            .insert(aor.clone());
                                    }
                                }
                            } else {
                                // Simple identity rule
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
                        }
                        in_rule = false;
                        pending_tw_start = None;
                        pending_tw_end = None;
                    }
                    "conditions" => in_conditions = false,
                    "identity" => in_identity = false,
                    "actions" => in_actions = false,
                    "sub-handling" => in_sub_handling = false,
                    "event-package" => in_event_package = false,
                    "domain-pattern" => in_domain_pattern = false,
                    "time-window" => {
                        // Finalize time window if no days child was seen
                        if let (Some(start), Some(end)) =
                            (pending_tw_start.take(), pending_tw_end.take())
                        {
                            current_time_windows.push(TimeWindow {
                                start_hour: start,
                                end_hour: end,
                                days: vec![],
                            });
                        }
                    }
                    "days" => in_days = false,
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
        event_allow,
        event_block,
        time_windows,
        allow_patterns,
        block_patterns,
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        };

        let xml = generate_pres_rules(&policy);
        let parsed = parse_pres_rules(&xml).unwrap();

        assert!(!parsed.default_allow);
        assert_eq!(parsed.allow_list, policy.allow_list);
        assert!(parsed.block_list.is_empty());
    }

    #[test]
    fn test_parse_empty_policy() {
        let policy = PresenceAuthPolicy {
            default_allow: true,
            allow_list: HashSet::new(),
            block_list: HashSet::new(),
            ..Default::default()
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

    // ========================================================================
    // Resource Lists XML tests
    // ========================================================================

    #[test]
    fn test_generate_resource_lists_xml() {
        let lists = vec![ResourceListXml {
            name: Some("My Contacts".to_string()),
            entries: vec![
                ResourceListEntryXml {
                    uri: "sip:alice@example.com".to_string(),
                    display_name: Some("Alice".to_string()),
                },
                ResourceListEntryXml {
                    uri: "sip:bob@example.com".to_string(),
                    display_name: None,
                },
            ],
        }];
        let xml = generate_resource_lists_xml(&lists);
        assert!(xml.contains("urn:ietf:params:xml:ns:resource-lists"));
        assert!(xml.contains("My Contacts"));
        assert!(xml.contains("sip:alice@example.com"));
        assert!(xml.contains("<display-name>Alice</display-name>"));
        assert!(xml.contains("sip:bob@example.com"));
    }

    #[test]
    fn test_roundtrip_resource_lists() {
        let lists = vec![
            ResourceListXml {
                name: Some("Team".to_string()),
                entries: vec![
                    ResourceListEntryXml {
                        uri: "sip:alice@example.com".to_string(),
                        display_name: Some("Alice Smith".to_string()),
                    },
                    ResourceListEntryXml {
                        uri: "sip:bob@example.com".to_string(),
                        display_name: Some("Bob Jones".to_string()),
                    },
                ],
            },
            ResourceListXml {
                name: None,
                entries: vec![ResourceListEntryXml {
                    uri: "sip:carol@example.com".to_string(),
                    display_name: None,
                }],
            },
        ];

        let xml = generate_resource_lists_xml(&lists);
        let parsed = parse_resource_lists_xml(&xml).unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, Some("Team".to_string()));
        assert_eq!(parsed[0].entries.len(), 2);
        assert_eq!(parsed[0].entries[0].uri, "sip:alice@example.com");
        assert_eq!(
            parsed[0].entries[0].display_name,
            Some("Alice Smith".to_string())
        );
        assert_eq!(parsed[0].entries[1].uri, "sip:bob@example.com");
        assert_eq!(
            parsed[0].entries[1].display_name,
            Some("Bob Jones".to_string())
        );
        assert_eq!(parsed[1].name, None);
        assert_eq!(parsed[1].entries.len(), 1);
        assert_eq!(parsed[1].entries[0].uri, "sip:carol@example.com");
        assert_eq!(parsed[1].entries[0].display_name, None);
    }

    #[test]
    fn test_parse_empty_resource_lists() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<resource-lists xmlns="urn:ietf:params:xml:ns:resource-lists">
</resource-lists>"#;
        let parsed = parse_resource_lists_xml(xml).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_parse_resource_lists_invalid_xml() {
        let result = parse_resource_lists_xml("not valid xml <<<");
        assert!(result.is_err());
    }
}
