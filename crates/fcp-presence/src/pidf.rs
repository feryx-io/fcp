//! RFC 3863 PIDF (Presence Information Data Format) XML generation and parsing.

use anyhow::Result;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;
use std::io::Cursor;

use crate::types::{PresenceState, PresenceUpdate};

const PIDF_NS: &str = "urn:ietf:params:xml:ns:pidf";

/// Map a PresenceState to the PIDF `<basic>` status value.
fn state_to_basic(state: &PresenceState) -> &'static str {
    match state {
        PresenceState::Closed => "closed",
        _ => "open",
    }
}

/// Derive a note from state if no explicit note is given.
fn state_to_default_note(state: &PresenceState) -> Option<&'static str> {
    match state {
        PresenceState::Open | PresenceState::Closed => None,
        PresenceState::Away => Some("Away"),
        PresenceState::Busy => Some("Busy"),
        PresenceState::OnThePhone => Some("On the phone"),
        PresenceState::Ringing => Some("Ringing"),
    }
}

/// Generate a PIDF XML document for a presentity.
pub fn generate_pidf(aor: &str, state: &PresenceState, note: Option<&str>) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    // XML declaration
    writer
        .write_event(Event::Decl(quick_xml::events::BytesDecl::new("1.0", Some("UTF-8"), None)))
        .expect("write xml decl");

    // <presence xmlns="..." entity="pres:user@example.com">
    let mut presence = BytesStart::new("presence");
    presence.push_attribute(("xmlns", PIDF_NS));
    presence.push_attribute(("entity", format!("pres:{aor}").as_str()));
    writer
        .write_event(Event::Start(presence))
        .expect("write presence start");

    // <tuple id="t1">
    let mut tuple = BytesStart::new("tuple");
    tuple.push_attribute(("id", "t1"));
    writer
        .write_event(Event::Start(tuple))
        .expect("write tuple start");

    // <status>
    writer
        .write_event(Event::Start(BytesStart::new("status")))
        .expect("write status start");

    // <basic>open|closed</basic>
    writer
        .write_event(Event::Start(BytesStart::new("basic")))
        .expect("write basic start");
    writer
        .write_event(Event::Text(BytesText::new(state_to_basic(state))))
        .expect("write basic text");
    writer
        .write_event(Event::End(BytesEnd::new("basic")))
        .expect("write basic end");

    // </status>
    writer
        .write_event(Event::End(BytesEnd::new("status")))
        .expect("write status end");

    // <note> — use explicit note, or derive from state
    let effective_note = note.or_else(|| state_to_default_note(state));
    if let Some(note_text) = effective_note {
        writer
            .write_event(Event::Start(BytesStart::new("note")))
            .expect("write note start");
        writer
            .write_event(Event::Text(BytesText::new(note_text)))
            .expect("write note text");
        writer
            .write_event(Event::End(BytesEnd::new("note")))
            .expect("write note end");
    }

    // </tuple>
    writer
        .write_event(Event::End(BytesEnd::new("tuple")))
        .expect("write tuple end");

    // </presence>
    writer
        .write_event(Event::End(BytesEnd::new("presence")))
        .expect("write presence end");

    let result = writer.into_inner().into_inner();
    String::from_utf8(result).expect("valid utf-8")
}

/// Parse a PIDF XML document into a PresenceUpdate.
///
/// Delegates to sip-core's hardened parser which enforces:
/// - Input size limit (1MB max)
/// - Field length validation
/// - Control character rejection
pub fn parse_pidf(xml: &str) -> Result<PresenceUpdate> {
    use sip_core::presence::{parse_pidf as sip_parse_pidf, BasicStatus};

    let doc = sip_parse_pidf(xml)
        .map_err(|e| anyhow::anyhow!("PIDF parse error: {e}"))?;

    let entity = doc.entity();
    // Strip "pres:" prefix if present
    let aor = entity.strip_prefix("pres:").unwrap_or(entity).to_string();

    // Get status from first tuple
    let mut tuples = doc.tuples();
    let first_tuple = tuples.next();
    let basic_status = first_tuple
        .and_then(|t| t.status())
        .unwrap_or(BasicStatus::Closed);

    let state = match basic_status {
        BasicStatus::Open => {
            // Infer richer state from note (same logic as before)
            let note_text = first_tuple.and_then(|t| t.notes().next());
            match note_text {
                Some("Away") => PresenceState::Away,
                Some("Busy") => PresenceState::Busy,
                Some("On the phone") => PresenceState::OnThePhone,
                Some("Ringing") => PresenceState::Ringing,
                _ => PresenceState::Open,
            }
        }
        BasicStatus::Closed => PresenceState::Closed,
    };

    let note = first_tuple
        .and_then(|t| t.notes().next())
        .map(|n| n.to_string())
        .or_else(|| doc.notes().next().map(|n| n.to_string()));

    Ok(PresenceUpdate { aor, state, note })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_pidf_open() {
        let xml = generate_pidf("alice@example.com", &PresenceState::Open, None);
        assert!(xml.contains("urn:ietf:params:xml:ns:pidf"));
        assert!(xml.contains("pres:alice@example.com"));
        assert!(xml.contains("<basic>open</basic>"));
        // Open state with no note should not have a <note> element
        assert!(!xml.contains("<note>"));
    }

    #[test]
    fn test_generate_pidf_closed() {
        let xml = generate_pidf("bob@example.com", &PresenceState::Closed, None);
        assert!(xml.contains("<basic>closed</basic>"));
        assert!(xml.contains("pres:bob@example.com"));
    }

    #[test]
    fn test_generate_pidf_with_note() {
        let xml =
            generate_pidf("alice@example.com", &PresenceState::Open, Some("In a meeting"));
        assert!(xml.contains("<basic>open</basic>"));
        assert!(xml.contains("<note>In a meeting</note>"));
    }

    #[test]
    fn test_generate_pidf_busy_default_note() {
        let xml = generate_pidf("alice@example.com", &PresenceState::Busy, None);
        assert!(xml.contains("<basic>open</basic>"));
        assert!(xml.contains("<note>Busy</note>"));
    }

    #[test]
    fn test_parse_pidf_basic() {
        let xml = generate_pidf("alice@example.com", &PresenceState::Open, None);
        let update = parse_pidf(&xml).unwrap();
        assert_eq!(update.aor, "alice@example.com");
        assert_eq!(update.state, PresenceState::Open);
        assert!(update.note.is_none());
    }

    #[test]
    fn test_parse_pidf_with_note() {
        let xml =
            generate_pidf("bob@example.com", &PresenceState::Open, Some("In a meeting"));
        let update = parse_pidf(&xml).unwrap();
        assert_eq!(update.aor, "bob@example.com");
        assert_eq!(update.state, PresenceState::Open);
        assert_eq!(update.note.as_deref(), Some("In a meeting"));
    }

    #[test]
    fn test_parse_pidf_roundtrip() {
        // Test that generate → parse preserves state
        let states: [(PresenceState, Option<&str>); 6] = [
            (PresenceState::Open, None),
            (PresenceState::Closed, None),
            (PresenceState::Away, None),
            (PresenceState::Busy, None),
            (PresenceState::OnThePhone, None),
            (PresenceState::Ringing, None),
        ];

        for (state, note) in &states {
            let xml = generate_pidf("user@example.com", state, note.as_deref());
            let update = parse_pidf(&xml).unwrap();
            assert_eq!(update.aor, "user@example.com");
            assert_eq!(
                update.state, *state,
                "Roundtrip failed for state {state}"
            );
        }
    }

    #[test]
    fn test_parse_pidf_closed_roundtrip() {
        let xml = generate_pidf("user@example.com", &PresenceState::Closed, None);
        let update = parse_pidf(&xml).unwrap();
        assert_eq!(update.state, PresenceState::Closed);
    }

    #[test]
    fn test_parse_invalid_xml() {
        let result = parse_pidf("not xml at all <<<<");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pidf_oversized_input() {
        // sip-core rejects inputs > 1MB
        let oversized = "x".repeat(1024 * 1024 + 1);
        let result = parse_pidf(&oversized);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pidf_control_chars_in_entity() {
        // Embed a raw control character (CR) directly in the entity attribute
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <presence xmlns=\"urn:ietf:params:xml:ns:pidf\" entity=\"pres:alice\r@example.com\">\n\
            <tuple id=\"t1\"><status><basic>open</basic></status></tuple>\n\
            </presence>";
        // The hardened parser should reject control characters
        let result = parse_pidf(xml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pidf_missing_entity() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<presence xmlns="urn:ietf:params:xml:ns:pidf">
  <tuple id="t1"><status><basic>open</basic></status></tuple>
</presence>"#;
        let result = parse_pidf(xml);
        assert!(result.is_err());
    }
}
