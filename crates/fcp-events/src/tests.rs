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
    });

    // Serialize to JSON
    let json = serde_json::to_string(&event).unwrap();
    println!("Event JSON: {}", json);

    // Deserialize back
    let deserialized: PlatformEvent = serde_json::from_str(&json).unwrap();

    // Should have the same call_id
    assert_eq!(event.call_id(), deserialized.call_id());
}
