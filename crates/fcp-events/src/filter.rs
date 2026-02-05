//! Event filtering for selective subscriptions.

use crate::PlatformEvent;
use fcp_core::{CallId, TenantId};
use serde::{Deserialize, Serialize};

/// Filter for event subscriptions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventFilter {
    /// Filter by tenant ID
    pub tenant_id: Option<TenantId>,

    /// Filter by call ID
    pub call_id: Option<CallId>,

    /// Filter by event types
    pub event_types: Option<Vec<EventType>>,
}

/// Event type categories for filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Call,
    Media,
    Ivr,
    Recording,
    Flow,
    System,
    User,
    Registration,
    Voicemail,
}

impl EventFilter {
    /// Create a new event filter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by tenant ID.
    pub fn tenant(mut self, tenant_id: TenantId) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    /// Filter by call ID.
    pub fn call(mut self, call_id: CallId) -> Self {
        self.call_id = Some(call_id);
        self
    }

    /// Filter by event types.
    pub fn types(mut self, event_types: Vec<EventType>) -> Self {
        self.event_types = Some(event_types);
        self
    }

    /// Check if an event matches this filter.
    pub fn matches(&self, event: &PlatformEvent) -> bool {
        // Filter by call ID if specified
        if let Some(filter_call_id) = &self.call_id {
            if let Some(event_call_id) = event.call_id() {
                if event_call_id != filter_call_id {
                    return false;
                }
            } else {
                // Event has no call_id but filter requires one
                return false;
            }
        }

        // Filter by event type if specified
        if let Some(ref types) = self.event_types {
            let event_type = match event {
                PlatformEvent::Call(_) => EventType::Call,
                PlatformEvent::Media(_) => EventType::Media,
                PlatformEvent::Ivr(_) => EventType::Ivr,
                PlatformEvent::Recording(_) => EventType::Recording,
                PlatformEvent::Flow(_) => EventType::Flow,
                PlatformEvent::System(_) => EventType::System,
                PlatformEvent::User(_) => EventType::User,
                PlatformEvent::Registration(_) => EventType::Registration,
                PlatformEvent::Voicemail(_) => EventType::Voicemail,
            };

            if !types.contains(&event_type) {
                return false;
            }
        }

        // All filters passed
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallDirection, CallEvent};
    use std::time::SystemTime;

    #[test]
    fn test_filter_no_restrictions() {
        let filter = EventFilter::new();
        let event = PlatformEvent::Call(CallEvent::Initiated {
            call_id: CallId::new(),
            tenant_id: TenantId::new(),
            from: "alice@example.com".to_string(),
            to: "bob@example.com".to_string(),
            direction: CallDirection::Inbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            node_id: None,
        });

        assert!(filter.matches(&event));
    }

    #[test]
    fn test_filter_by_call_id() {
        let call_id = CallId::new();
        let filter = EventFilter::new().call(call_id);

        let matching_event = PlatformEvent::Call(CallEvent::Initiated {
            call_id,
            tenant_id: TenantId::new(),
            from: "alice@example.com".to_string(),
            to: "bob@example.com".to_string(),
            direction: CallDirection::Inbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            node_id: None,
        });

        let non_matching_event = PlatformEvent::Call(CallEvent::Initiated {
            call_id: CallId::new(), // Different call ID
            tenant_id: TenantId::new(),
            from: "charlie@example.com".to_string(),
            to: "dave@example.com".to_string(),
            direction: CallDirection::Inbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            node_id: None,
        });

        assert!(filter.matches(&matching_event));
        assert!(!filter.matches(&non_matching_event));
    }

    #[test]
    fn test_filter_by_event_type() {
        let filter = EventFilter::new().types(vec![EventType::Call]);

        let call_event = PlatformEvent::Call(CallEvent::Initiated {
            call_id: CallId::new(),
            tenant_id: TenantId::new(),
            from: "alice@example.com".to_string(),
            to: "bob@example.com".to_string(),
            direction: CallDirection::Inbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            node_id: None,
        });

        let system_event = PlatformEvent::System(crate::SystemEvent::Started {
            node_id: "node-1".to_string(),
            timestamp: SystemTime::now(),
        });

        assert!(filter.matches(&call_event));
        assert!(!filter.matches(&system_event));
    }
}
