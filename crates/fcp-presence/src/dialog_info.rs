//! RFC 4235 dialog-info XML generation for BLF (Busy Lamp Field).
//!
//! Desk phone BLF keys subscribe to `event: dialog` and receive NOTIFY with
//! `application/dialog-info+xml` bodies. This module generates the XML.

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;
use std::io::Cursor;

use crate::types::PresenceState;

const DIALOG_INFO_NS: &str = "urn:ietf:params:xml:ns:dialog-info";

/// Map a PresenceState to a dialog-info `<state>` value.
///
/// RFC 4235 dialog states: trying, proceeding, early, confirmed, terminated.
fn state_to_dialog_state(state: &PresenceState) -> &'static str {
    match state {
        PresenceState::Closed => "terminated",
        PresenceState::Open => "terminated",   // No active dialog
        PresenceState::Away => "terminated",    // No active dialog
        PresenceState::Busy => "confirmed",     // DND treated as occupied
        PresenceState::OnThePhone => "confirmed",
        PresenceState::Ringing => "early",
    }
}

/// Whether the presentity has an active dialog (for BLF lamp state).
fn has_active_dialog(state: &PresenceState) -> bool {
    matches!(
        state,
        PresenceState::Ringing | PresenceState::OnThePhone | PresenceState::Busy
    )
}

/// Generate a dialog-info XML document for BLF notifications.
///
/// Produces a `full` state document with zero or one `<dialog>` element
/// depending on whether the presentity has an active call.
pub fn generate_dialog_info(
    aor: &str,
    state: &PresenceState,
    dialog_id: Option<&str>,
    version: u64,
) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    // XML declaration
    writer
        .write_event(Event::Decl(quick_xml::events::BytesDecl::new(
            "1.0",
            Some("UTF-8"),
            None,
        )))
        .expect("write xml decl");

    // <dialog-info xmlns="..." version="N" state="full" entity="sip:user@domain">
    let version_str = version.to_string();
    let mut dialog_info = BytesStart::new("dialog-info");
    dialog_info.push_attribute(("xmlns", DIALOG_INFO_NS));
    dialog_info.push_attribute(("version", version_str.as_str()));
    dialog_info.push_attribute(("state", "full"));
    dialog_info.push_attribute(("entity", format!("sip:{aor}").as_str()));
    writer
        .write_event(Event::Start(dialog_info))
        .expect("write dialog-info start");

    // Only include a <dialog> element if the user has an active dialog
    if has_active_dialog(state) {
        let dlg_id = dialog_id.unwrap_or("1");

        // <dialog id="..." direction="recipient">
        let mut dialog = BytesStart::new("dialog");
        dialog.push_attribute(("id", dlg_id));
        dialog.push_attribute(("direction", "recipient"));
        writer
            .write_event(Event::Start(dialog))
            .expect("write dialog start");

        // <state>early|confirmed</state>
        writer
            .write_event(Event::Start(BytesStart::new("state")))
            .expect("write state start");
        writer
            .write_event(Event::Text(BytesText::new(state_to_dialog_state(state))))
            .expect("write state text");
        writer
            .write_event(Event::End(BytesEnd::new("state")))
            .expect("write state end");

        // </dialog>
        writer
            .write_event(Event::End(BytesEnd::new("dialog")))
            .expect("write dialog end");
    }

    // </dialog-info>
    writer
        .write_event(Event::End(BytesEnd::new("dialog-info")))
        .expect("write dialog-info end");

    let result = writer.into_inner().into_inner();
    String::from_utf8(result).expect("valid utf-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_dialog_info_idle() {
        let xml = generate_dialog_info("alice@example.com", &PresenceState::Open, None, 0);
        assert!(xml.contains(DIALOG_INFO_NS));
        assert!(xml.contains("sip:alice@example.com"));
        assert!(xml.contains("state=\"full\""));
        assert!(xml.contains("version=\"0\""));
        // No <dialog id=...> element for idle user (but <dialog-info> is present)
        assert!(!xml.contains("<dialog id="));
        assert!(!xml.contains("<state>"));
    }

    #[test]
    fn test_generate_dialog_info_ringing() {
        let xml = generate_dialog_info("alice@example.com", &PresenceState::Ringing, Some("call-42"), 5);
        assert!(xml.contains("<dialog id=\"call-42\""));
        assert!(xml.contains("<state>early</state>"));
        assert!(xml.contains("version=\"5\""));
    }

    #[test]
    fn test_generate_dialog_info_on_phone() {
        let xml = generate_dialog_info("alice@example.com", &PresenceState::OnThePhone, None, 1);
        assert!(xml.contains("<dialog id=\"1\""));
        assert!(xml.contains("<state>confirmed</state>"));
        assert!(xml.contains("version=\"1\""));
    }

    #[test]
    fn test_generate_dialog_info_busy() {
        let xml = generate_dialog_info("alice@example.com", &PresenceState::Busy, None, 2);
        assert!(xml.contains("<state>confirmed</state>"));
    }

    #[test]
    fn test_generate_dialog_info_closed() {
        let xml = generate_dialog_info("alice@example.com", &PresenceState::Closed, None, 0);
        // Closed = no active dialog, no <dialog id=...> element
        assert!(!xml.contains("<dialog id="));
    }

    #[test]
    fn test_generate_dialog_info_away() {
        let xml = generate_dialog_info("alice@example.com", &PresenceState::Away, None, 0);
        // Away = no active dialog
        assert!(!xml.contains("<dialog id="));
    }

    #[test]
    fn test_version_increments() {
        let xml_v0 = generate_dialog_info("alice@example.com", &PresenceState::Ringing, None, 0);
        let xml_v1 = generate_dialog_info("alice@example.com", &PresenceState::OnThePhone, None, 1);
        let xml_v42 = generate_dialog_info("alice@example.com", &PresenceState::Closed, None, 42);
        assert!(xml_v0.contains("version=\"0\""));
        assert!(xml_v1.contains("version=\"1\""));
        assert!(xml_v42.contains("version=\"42\""));
    }
}
