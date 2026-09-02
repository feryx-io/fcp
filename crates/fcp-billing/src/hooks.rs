//! External billing hooks.
//!
//! Every billing outcome is delivered as a [`BillingEvent`] to a
//! [`BillingHook`]. The platform default, [`EventBusHook`], publishes the
//! event on the event bus (`PlatformEvent::Billing`), where the events server
//! streams it to WebSocket subscribers and the webhook dispatcher forwards it
//! to endpoints subscribed to `billing.*`. In-process integrations (a
//! metering sidecar, a custom ledger) implement the trait directly and are
//! combined with [`CompositeHook`].

use async_trait::async_trait;
use fcp_events::{BillingEvent, EventPublisher, PlatformEvent};
use std::sync::Arc;

/// Receiver of billing outcomes. Implementations must never fail the call
/// path: log and swallow errors.
#[async_trait]
pub trait BillingHook: Send + Sync {
    async fn on_event(&self, event: BillingEvent);
}

/// Drops every event.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopHook;

#[async_trait]
impl BillingHook for NoopHook {
    async fn on_event(&self, _event: BillingEvent) {}
}

/// Publishes billing events on the platform event bus.
pub struct EventBusHook {
    events: Arc<dyn EventPublisher>,
}

impl EventBusHook {
    pub fn new(events: Arc<dyn EventPublisher>) -> Self {
        Self { events }
    }
}

#[async_trait]
impl BillingHook for EventBusHook {
    async fn on_event(&self, event: BillingEvent) {
        let name = PlatformEvent::Billing(event.clone()).event_type_name();
        if let Err(e) = self.events.publish(PlatformEvent::Billing(event)).await {
            tracing::error!(error = %e, event = name, "Failed to publish billing event");
        }
    }
}

/// Fans one event out to several hooks, in order.
#[derive(Default)]
pub struct CompositeHook {
    hooks: Vec<Arc<dyn BillingHook>>,
}

impl CompositeHook {
    pub fn new(hooks: Vec<Arc<dyn BillingHook>>) -> Self {
        Self { hooks }
    }

    pub fn push(mut self, hook: Arc<dyn BillingHook>) -> Self {
        self.hooks.push(hook);
        self
    }
}

#[async_trait]
impl BillingHook for CompositeHook {
    async fn on_event(&self, event: BillingEvent) {
        for hook in &self.hooks {
            hook.on_event(event.clone()).await;
        }
    }
}

/// Records every event; for tests.
#[derive(Default)]
pub struct RecordingHook {
    pub events: std::sync::Mutex<Vec<BillingEvent>>,
}

impl RecordingHook {
    pub fn events(&self) -> Vec<BillingEvent> {
        self.events.lock().expect("recording hook poisoned").clone()
    }
}

#[async_trait]
impl BillingHook for RecordingHook {
    async fn on_event(&self, event: BillingEvent) {
        self.events
            .lock()
            .expect("recording hook poisoned")
            .push(event);
    }
}
