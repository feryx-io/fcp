//! Keeping a node's [`TrunkSet`] current (`docs/TENANT_TRUNKS.md` §4).
//!
//! A node reads its own `[[trunks]]` once — those are the platform's — and
//! the tenants' trunks from the database: at startup, whenever the API says
//! a trunk changed, and on a slow tick, so a node that missed the event is
//! never wrong for long.

use crate::trunk_health::TrunkHealthMonitor;
use crate::trunk_set::TrunkSet;
use fcp_core::storage::{TenantStore, TrunkStore};
use fcp_events::filter::EventType;
use fcp_events::{EventFilter, EventSubscriber, PlatformEvent, SystemEvent};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// The `entity_type` the trunks API puts on `system.config_changed` when a
/// trunk is created, changed or deleted.
pub const TRUNK_ENTITY: &str = "trunk";

/// How often a node re-reads the tenants' trunks by default, when nothing
/// told it to. The event is the fast path; this is the backstop.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Read every tenant's trunks into `set`, and let `health` watch the ones it
/// has not seen before.
pub async fn refresh_trunks<S>(set: &TrunkSet, storage: &S, health: &TrunkHealthMonitor)
where
    S: TrunkStore + TenantStore + ?Sized,
{
    match set.refresh(storage).await {
        Ok(count) => {
            health.track(&set.all());
            debug!(
                trunks = count,
                tenants = set.tenant_count(),
                "The tenants' trunks are current"
            );
        }
        Err(e) => {
            warn!(error = %e, "Could not read the tenants' trunks; routing on what is known")
        }
    }
}

/// Keep `set` current for as long as the node runs: on every trunk change
/// the API announces, and on the backstop tick.
///
/// The first read happens before this returns, so a node is routing by the
/// tenants' trunks before it answers its first call.
pub async fn spawn_trunk_refresh<S>(
    set: Arc<TrunkSet>,
    storage: Arc<S>,
    health: Arc<TrunkHealthMonitor>,
    events: Option<Arc<dyn EventSubscriber>>,
    backstop: Duration,
) -> tokio::task::JoinHandle<()>
where
    S: TrunkStore + TenantStore + Send + Sync + 'static + ?Sized,
{
    refresh_trunks(&set, storage.as_ref(), &health).await;
    info!(
        platform_trunks = set.platform().len(),
        tenants = set.tenant_count(),
        "Trunk routing: this node's own trunks and every tenant's"
    );

    tokio::spawn(async move {
        let filter = EventFilter::new().types(vec![EventType::System]);
        let mut stream = match &events {
            Some(bus) => match bus.subscribe(filter.clone()).await {
                Ok(stream) => Some(stream),
                Err(e) => {
                    warn!(error = %e, "Trunk refresh could not subscribe; the tick alone keeps it current");
                    None
                }
            },
            None => None,
        };

        let mut ticker = tokio::time::interval(backstop);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // the first tick is immediate, and the read above just happened

        loop {
            // A stream that ended is taken up again; until it is, the tick
            // is what keeps this node current.
            if stream.is_none() {
                if let Some(bus) = &events {
                    match bus.subscribe(filter.clone()).await {
                        Ok(open) => {
                            info!("Trunk refresh resubscribed");
                            stream = Some(open);
                        }
                        Err(e) => warn!(error = %e, "Trunk refresh could not resubscribe"),
                    }
                }
            }

            match &mut stream {
                Some(open) => {
                    let event = tokio::select! {
                        _ = ticker.tick() => None,
                        event = open.next() => match event {
                            Some(event) => Some(event),
                            None => {
                                warn!("Trunk refresh event stream ended");
                                stream = None;
                                continue;
                            }
                        },
                    };
                    match event {
                        Some(event) if is_trunk_change(&event) => {
                            debug!("A trunk changed; re-reading the tenants' trunks")
                        }
                        // Some other system event: nothing to re-read.
                        Some(_) => continue,
                        // The backstop tick.
                        None => {}
                    }
                }
                None => {
                    ticker.tick().await;
                }
            }
            refresh_trunks(&set, storage.as_ref(), &health).await;
        }
    })
}

/// Whether this event says a trunk changed.
fn is_trunk_change(event: &PlatformEvent) -> bool {
    matches!(
        event,
        PlatformEvent::System(SystemEvent::ConfigChanged { entity_type, .. })
            if entity_type == TRUNK_ENTITY
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::TenantId;
    use std::time::SystemTime;

    #[test]
    fn only_a_trunk_change_reloads_the_trunks() {
        let changed = |entity_type: &str| {
            PlatformEvent::System(SystemEvent::ConfigChanged {
                entity_type: entity_type.to_string(),
                entity_id: "trunk-1".to_string(),
                tenant_id: TenantId::new(),
                timestamp: SystemTime::now(),
            })
        };
        assert!(is_trunk_change(&changed("trunk")));
        assert!(!is_trunk_change(&changed("device")));
    }
}
