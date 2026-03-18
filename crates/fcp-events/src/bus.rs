//! Event bus implementation for publishing and subscribing to events.

use crate::{EventFilter, PlatformEvent};
use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Event publisher trait for publishing events to the bus.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish an event to the bus.
    async fn publish(&self, event: PlatformEvent) -> Result<()>;
}

/// Event subscriber trait for subscribing to events from the bus.
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    /// Subscribe to events matching the given filter.
    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream>;
}

/// Combined event bus trait.
pub trait EventBus: EventPublisher + EventSubscriber + Send + Sync {}

/// Event stream for receiving events.
pub struct EventStream {
    receiver: mpsc::Receiver<PlatformEvent>,
    _subscription_id: SubscriptionId,
}

impl EventStream {
    /// Receive the next event from the stream.
    pub async fn next(&mut self) -> Option<PlatformEvent> {
        self.receiver.recv().await
    }

    /// Try to receive an event without blocking.
    pub fn try_next(&mut self) -> Option<PlatformEvent> {
        self.receiver.try_recv().ok()
    }
}

/// Subscription ID for tracking subscriptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SubscriptionId(u64);

/// Subscription details.
struct Subscription {
    filter: EventFilter,
    sender: mpsc::Sender<PlatformEvent>,
    /// Running count of events dropped because the channel was full.
    drops: AtomicU64,
}

/// In-memory event bus implementation.
///
/// This implementation stores all subscriptions in memory and broadcasts
/// events to matching subscribers. It is suitable for single-node deployments.
#[derive(Clone)]
pub struct InMemoryEventBus {
    subscriptions: Arc<DashMap<SubscriptionId, Subscription>>,
    next_subscription_id: Arc<parking_lot::Mutex<u64>>,
}

impl InMemoryEventBus {
    /// Create a new in-memory event bus.
    pub fn new() -> Self {
        Self {
            subscriptions: Arc::new(DashMap::new()),
            next_subscription_id: Arc::new(parking_lot::Mutex::new(0)),
        }
    }

    /// Get the number of active subscriptions.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// Clean up closed subscriptions.
    pub fn cleanup_closed_subscriptions(&self) -> usize {
        let mut removed = 0;

        self.subscriptions.retain(|_, subscription| {
            if subscription.sender.is_closed() {
                removed += 1;
                false
            } else {
                true
            }
        });

        if removed > 0 {
            debug!(removed, "Cleaned up closed event subscriptions");
        }

        removed
    }
}

impl Default for InMemoryEventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventPublisher for InMemoryEventBus {
    async fn publish(&self, event: PlatformEvent) -> Result<()> {
        let call_id = event.call_id().map(|id| id.to_string());

        debug!(
            ?event,
            call_id = ?call_id,
            subscriber_count = self.subscriptions.len(),
            "Publishing event"
        );

        let mut sent = 0;
        let mut failed = 0;

        // Broadcast to all matching subscriptions
        for entry in self.subscriptions.iter() {
            let subscription = entry.value();

            // Check if event matches filter
            if subscription.filter.matches(&event) {
                // Try to send (non-blocking)
                match subscription.sender.try_send(event.clone()) {
                    Ok(_) => {
                        // If we previously had drops, log the summary now that we're catching up
                        let prev_drops = subscription.drops.swap(0, Ordering::Relaxed);
                        if prev_drops > 0 {
                            warn!(
                                subscription_id = ?entry.key(),
                                dropped_events = prev_drops,
                                "Subscriber recovered after dropping events (channel was full)"
                            );
                        }
                        sent += 1;
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        // Increment drop counter; log only on the first drop in a burst
                        let prev = subscription.drops.fetch_add(1, Ordering::Relaxed);
                        if prev == 0 {
                            warn!(
                                subscription_id = ?entry.key(),
                                "Event dropped: subscriber channel full (further drops suppressed until recovery)"
                            );
                        }
                        failed += 1;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        // Subscription closed, will be cleaned up later
                        failed += 1;
                    }
                }
            }
        }

        debug!(sent, failed, "Event published to subscribers");

        Ok(())
    }
}

#[async_trait]
impl EventSubscriber for InMemoryEventBus {
    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream> {
        // Generate subscription ID
        let subscription_id = {
            let mut next_id = self.next_subscription_id.lock();
            let id = SubscriptionId(*next_id);
            *next_id += 1;
            id
        };

        // Create channel for events (buffer of 10 000 events).
        // A larger buffer absorbs transient bursts from high-frequency
        // publishers (presence heartbeats, media stats, etc.) without
        // dropping events when a subscriber briefly stalls.
        let (sender, receiver) = mpsc::channel(10_000);

        // Store subscription
        let subscription = Subscription {
            filter,
            sender,
            drops: AtomicU64::new(0),
        };
        self.subscriptions.insert(subscription_id, subscription);

        debug!(
            ?subscription_id,
            total_subscriptions = self.subscriptions.len(),
            "New event subscription created"
        );

        Ok(EventStream {
            receiver,
            _subscription_id: subscription_id,
        })
    }
}

impl EventBus for InMemoryEventBus {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallDirection, CallEvent};
    use fcp_core::{CallId, TenantId};
    use std::time::SystemTime;

    #[tokio::test]
    async fn test_publish_and_subscribe() {
        let bus = InMemoryEventBus::new();

        // Create subscription
        let mut stream = bus.subscribe(EventFilter::new()).await.unwrap();

        // Publish event
        let call_id = CallId::new();
        let event = PlatformEvent::Call(CallEvent::Initiated {
            call_id,
            tenant_id: TenantId::new(),
            from: "alice@example.com".to_string(),
            to: "bob@example.com".to_string(),
            direction: CallDirection::Inbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            node_id: None,
        });

        bus.publish(event.clone()).await.unwrap();

        // Receive event
        let received = stream.next().await.unwrap();

        match received {
            PlatformEvent::Call(CallEvent::Initiated {
                call_id: received_call_id,
                ..
            }) => {
                assert_eq!(received_call_id, call_id);
            }
            _ => panic!("Expected Call::Initiated event"),
        }
    }

    #[tokio::test]
    async fn test_filtered_subscription() {
        let bus = InMemoryEventBus::new();

        let call_id_1 = CallId::new();
        let call_id_2 = CallId::new();

        // Subscribe to call_id_1 only
        let mut stream = bus
            .subscribe(EventFilter::new().call(call_id_1))
            .await
            .unwrap();

        // Publish event for call_id_1
        let event1 = PlatformEvent::Call(CallEvent::Initiated {
            call_id: call_id_1,
            tenant_id: TenantId::new(),
            from: "alice@example.com".to_string(),
            to: "bob@example.com".to_string(),
            direction: CallDirection::Inbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            node_id: None,
        });

        // Publish event for call_id_2 (should be filtered out)
        let event2 = PlatformEvent::Call(CallEvent::Initiated {
            call_id: call_id_2,
            tenant_id: TenantId::new(),
            from: "charlie@example.com".to_string(),
            to: "dave@example.com".to_string(),
            direction: CallDirection::Inbound,
            timestamp: SystemTime::now(),
            trunk_name: None,
            node_id: None,
        });

        bus.publish(event2).await.unwrap();
        bus.publish(event1.clone()).await.unwrap();

        // Should only receive event1
        let received = stream.next().await.unwrap();
        match received {
            PlatformEvent::Call(CallEvent::Initiated {
                call_id: received_call_id,
                ..
            }) => {
                assert_eq!(received_call_id, call_id_1);
            }
            _ => panic!("Expected Call::Initiated event for call_id_1"),
        }

        // Should not receive event2
        assert!(stream.try_next().is_none());
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = InMemoryEventBus::new();

        // Create two subscriptions
        let mut stream1 = bus.subscribe(EventFilter::new()).await.unwrap();
        let mut stream2 = bus.subscribe(EventFilter::new()).await.unwrap();

        // Publish event
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

        bus.publish(event).await.unwrap();

        // Both subscribers should receive the event
        assert!(stream1.next().await.is_some());
        assert!(stream2.next().await.is_some());
    }
}
