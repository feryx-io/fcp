//! `docs/API_EVENTS.md` shows events as FCP sends them: every example with
//! an `event_type` is a `PlatformEvent` as it parses.

use fcp_events::PlatformEvent;

#[test]
fn the_documented_events_are_the_events() {
    let doc = include_str!("../../../docs/API_EVENTS.md");
    let mut checked = 0;
    let mut wrong = Vec::new();
    for block in doc.split("```json\n").skip(1) {
        let body = block.split("```").next().unwrap_or_default();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
            continue;
        };
        if value.get("event_type").is_none() {
            continue;
        }
        if let Err(e) = serde_json::from_value::<PlatformEvent>(value.clone()) {
            wrong.push(format!("({e}):\n{body}"));
        }
        checked += 1;
    }
    assert!(
        wrong.is_empty(),
        "documented events that do not parse:\n{}",
        wrong.join("\n")
    );
    assert!(checked >= 10, "only {checked} examples found");
}
