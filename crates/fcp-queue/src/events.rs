//! Event publishing helpers for queue operations.

use fcp_core::TenantId;
use fcp_events::{EventPublisher, PlatformEvent, QueueEvent, QueueStatsSnapshot};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::warn;

use crate::types::QueueStats;

/// Publish a queue event, logging a warning if it fails.
async fn publish(events: &Option<Arc<dyn EventPublisher>>, event: QueueEvent) {
    if let Some(publisher) = events {
        if let Err(e) = publisher.publish(PlatformEvent::Queue(event)).await {
            warn!(error = %e, "Failed to publish queue event");
        }
    }
}

/// Publish a CallerEnqueued event.
pub async fn caller_enqueued(
    events: &Option<Arc<dyn EventPublisher>>,
    queue_name: &str,
    call_id: &str,
    caller: &str,
    position: usize,
    tenant_id: &TenantId,
) {
    publish(
        events,
        QueueEvent::CallerEnqueued {
            queue_name: queue_name.to_string(),
            call_id: call_id.to_string(),
            caller: caller.to_string(),
            position,
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a CallerDequeued event (agent answered).
pub async fn caller_dequeued(
    events: &Option<Arc<dyn EventPublisher>>,
    queue_name: &str,
    call_id: &str,
    agent_id: &str,
    wait_time_secs: u64,
    tenant_id: &TenantId,
) {
    publish(
        events,
        QueueEvent::CallerDequeued {
            queue_name: queue_name.to_string(),
            call_id: call_id.to_string(),
            agent_id: agent_id.to_string(),
            wait_time_secs,
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a CallerAbandoned event (caller hung up while waiting).
pub async fn caller_abandoned(
    events: &Option<Arc<dyn EventPublisher>>,
    queue_name: &str,
    call_id: &str,
    wait_time_secs: u64,
    position: usize,
    tenant_id: &TenantId,
) {
    publish(
        events,
        QueueEvent::CallerAbandoned {
            queue_name: queue_name.to_string(),
            call_id: call_id.to_string(),
            wait_time_secs,
            position,
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a CallerOverflowed event (max wait time or queue full).
pub async fn caller_overflowed(
    events: &Option<Arc<dyn EventPublisher>>,
    queue_name: &str,
    call_id: &str,
    destination: &str,
    reason: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        QueueEvent::CallerOverflowed {
            queue_name: queue_name.to_string(),
            call_id: call_id.to_string(),
            destination: destination.to_string(),
            reason: reason.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish an AgentLoggedIn event.
pub async fn agent_logged_in(
    events: &Option<Arc<dyn EventPublisher>>,
    queue_name: &str,
    agent_id: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        QueueEvent::AgentLoggedIn {
            queue_name: queue_name.to_string(),
            agent_id: agent_id.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish an AgentLoggedOut event.
pub async fn agent_logged_out(
    events: &Option<Arc<dyn EventPublisher>>,
    queue_name: &str,
    agent_id: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        QueueEvent::AgentLoggedOut {
            queue_name: queue_name.to_string(),
            agent_id: agent_id.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish an AgentStateChanged event.
pub async fn agent_state_changed(
    events: &Option<Arc<dyn EventPublisher>>,
    agent_id: &str,
    old_state: &str,
    new_state: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        QueueEvent::AgentStateChanged {
            agent_id: agent_id.to_string(),
            old_state: old_state.to_string(),
            new_state: new_state.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish an AgentKicked event (admin forced logout).
pub async fn agent_kicked(
    events: &Option<Arc<dyn EventPublisher>>,
    agent_id: &str,
    reason: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        QueueEvent::AgentKicked {
            agent_id: agent_id.to_string(),
            reason: reason.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a CallbackInitiated event (caller requested virtual queueing).
pub async fn callback_initiated(
    events: &Option<Arc<dyn EventPublisher>>,
    queue_name: &str,
    call_id: &str,
    callback_uri: &str,
    tenant_id: &TenantId,
) {
    publish(
        events,
        QueueEvent::CallbackInitiated {
            queue_name: queue_name.to_string(),
            call_id: call_id.to_string(),
            callback_uri: callback_uri.to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}

/// Publish a StatsSnapshot event for wallboard consumers.
pub async fn stats_snapshot(
    events: &Option<Arc<dyn EventPublisher>>,
    stats: Vec<QueueStats>,
    tenant_id: &TenantId,
) {
    let snapshots: Vec<QueueStatsSnapshot> = stats
        .into_iter()
        .map(|s| QueueStatsSnapshot {
            queue_name: s.queue_name,
            calls_waiting: s.calls_waiting,
            agents_available: s.agents_available,
            agents_on_call: s.agents_on_call,
            agents_total: s.agents_total,
            avg_wait_time_secs: s.avg_wait_time_secs,
            longest_wait_secs: s.longest_wait_secs,
            calls_handled_today: s.calls_handled_today,
            calls_abandoned_today: s.calls_abandoned_today,
            service_level_pct: s.service_level_pct,
        })
        .collect();

    publish(
        events,
        QueueEvent::StatsSnapshot {
            stats: snapshots,
            tenant_id: tenant_id.clone(),
            timestamp: SystemTime::now(),
        },
    )
    .await;
}
