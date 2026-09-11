//! Integration tests for the event system.

use crate::*;
use fcp_core::{CallId, TenantId};
use std::time::SystemTime;

#[tokio::test]
async fn test_event_serialization() {
    let event = PlatformEvent::Call(CallEvent::Initiated {
        call_id: CallId::new(),
        tenant_id: TenantId::new(),
        from: "alice@example.com".to_string(),
        to: "bob@example.com".to_string(),
        direction: CallDirection::Inbound,
        timestamp: SystemTime::now(),
        trunk_name: None,
        node_id: Some("test-node-1".to_string()),
    });

    // Serialize to JSON
    let json = serde_json::to_string(&event).unwrap();
    println!("Event JSON: {}", json);

    // Deserialize back
    let deserialized: PlatformEvent = serde_json::from_str(&json).unwrap();

    // Should have the same call_id
    assert_eq!(event.call_id(), deserialized.call_id());
}

#[test]
fn video_overload_says_what_was_given_up_and_still_reads_older_nodes() {
    // A shed: one composite smaller, everyone's frame rate untouched.
    let shed = PlatformEvent::Conference(ConferenceEvent::VideoOverload {
        room_number: "1000".to_string(),
        action: "shed".to_string(),
        from_fps: 15,
        to_fps: 15,
        output: Some("all".to_string()),
        from_resolution: Some("1280x720".to_string()),
        to_resolution: Some("640x360".to_string()),
        tenant_id: TenantId::new(),
        timestamp: SystemTime::now(),
    });
    let json = serde_json::to_value(&shed).unwrap();
    assert_eq!(json["type"], "video_overload");
    assert_eq!(json["action"], "shed");
    assert_eq!(json["output"], "all");
    assert_eq!(json["from_resolution"], "1280x720");
    assert_eq!(json["to_resolution"], "640x360");
    assert_eq!(json["from_fps"], json["to_fps"]);

    // A halved frame rate names no composite, so the fields are left off
    // rather than sent as nulls.
    let halved = PlatformEvent::Conference(ConferenceEvent::VideoOverload {
        room_number: "1000".to_string(),
        action: "fps".to_string(),
        from_fps: 15,
        to_fps: 7,
        output: None,
        from_resolution: None,
        to_resolution: None,
        tenant_id: TenantId::new(),
        timestamp: SystemTime::now(),
    });
    let mut json = serde_json::to_value(&halved).unwrap();
    for key in ["output", "from_resolution", "to_resolution"] {
        assert!(json.get(key).is_none(), "{key} sent for a halved rate");
    }

    // An event relayed from a region still running a node that predates
    // shedding has no `action`: it reads as the halving it was.
    json.as_object_mut().unwrap().remove("action");
    let old: PlatformEvent = serde_json::from_value(json).unwrap();
    match old {
        PlatformEvent::Conference(ConferenceEvent::VideoOverload {
            action,
            to_fps,
            output,
            ..
        }) => {
            assert_eq!(action, "fps");
            assert_eq!(to_fps, 7);
            assert!(output.is_none());
        }
        other => panic!("not a video_overload: {other:?}"),
    }
}
