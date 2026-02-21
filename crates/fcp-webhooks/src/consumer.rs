//! Event bus consumer that feeds events into the webhook dispatcher.

use crate::dispatcher::WebhookDispatcher;
use std::sync::Arc;
use tracing::{error, info};

/// Spawn a background task that subscribes to all events from the event bus
/// and dispatches matching events to webhook endpoints.
pub fn spawn_webhook_consumer(
    event_bus: Arc<dyn fcp_events::EventSubscriber>,
    dispatcher: Arc<WebhookDispatcher>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let filter = fcp_events::EventFilter::new();

        let mut stream = match event_bus.subscribe(filter).await {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "Failed to subscribe to event bus for webhook dispatch");
                return;
            }
        };

        info!("Webhook event consumer started");

        while let Some(event) = stream.next().await {
            dispatcher.dispatch(&event).await;
        }

        error!("Webhook event stream ended unexpectedly");
    })
}
