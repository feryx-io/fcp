//! Event publishing helpers for presence operations.

use fcp_core::TenantId;
use fcp_events::{EventPublisher, PlatformEvent, PresenceEvent};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::warn;

use crate::types::PresenceState;

/// Publish a presence event, logging a warning if it fails.
async fn publish(events: &Option<Arc<dyn EventPublisher>>, event: PresenceEvent) {
    if let Some(publisher) = events {
        if let Err(e) = publisher.publish(PlatformEvent::Presence(event)).await {
            warn!(error = %e, "Failed to publish presence event");
        }
    }
}

/// Publish a StatusChanged event.
pub async fn status_changed(
    events: &Option<Arc<dyn EventPublisher>>,
    aor: &str,
    old_state: PresenceState,
    new_state: PresenceState,
    tenant_id: &TenantId,
) {
    publish(
        events,
        PresenceEvent::StatusChanged {
            aor: aor.to_string(),
            old_state: old_state.to_string(),
            new_state: new_state.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a WatcherAdded event.
pub async fn watcher_added(
    events: &Option<Arc<dyn EventPublisher>>,
    presentity_aor: &str,
    watcher_aor: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        PresenceEvent::WatcherAdded {
            presentity_aor: presentity_aor.to_string(),
            watcher_aor: watcher_aor.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a WatcherRemoved event.
pub async fn watcher_removed(
    events: &Option<Arc<dyn EventPublisher>>,
    presentity_aor: &str,
    watcher_aor: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        PresenceEvent::WatcherRemoved {
            presentity_aor: presentity_aor.to_string(),
            watcher_aor: watcher_aor.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a PublishReceived event.
pub async fn publish_received(
    events: &Option<Arc<dyn EventPublisher>>,
    aor: &str,
    state: PresenceState,
    tenant_id: &TenantId,
) {
    publish(
        events,
        PresenceEvent::PublishReceived {
            aor: aor.to_string(),
            state: state.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a PublishExpiryWarning event.
pub async fn publish_expiry_warning(
    events: &Option<Arc<dyn EventPublisher>>,
    aor: &str,
    etag: &str,
    elapsed_pct: u8,
    tenant_id: &TenantId,
) {
    publish(
        events,
        PresenceEvent::PublishExpiryWarning {
            aor: aor.to_string(),
            etag: etag.to_string(),
            elapsed_pct,
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a PublishExpired event.
pub async fn publish_expired(
    events: &Option<Arc<dyn EventPublisher>>,
    aor: &str,
    etag: &str,
    state: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        PresenceEvent::PublishExpired {
            aor: aor.to_string(),
            etag: etag.to_string(),
            state: state.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a DialogChanged event (call started, answered, ended, etc.).
pub async fn dialog_changed(
    events: &Option<Arc<dyn EventPublisher>>,
    aor: &str,
    call_id: &str,
    dialog_state: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        PresenceEvent::DialogChanged {
            aor: aor.to_string(),
            call_id: call_id.to_string(),
            dialog_state: dialog_state.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}
