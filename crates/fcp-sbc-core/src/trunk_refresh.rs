//! Keeping a node's [`TrunkSet`] current (`docs/TENANT_TRUNKS.md` §4),
//! and its [`TenantRuleSet`] (phase 6b).
//!
//! A node reads its own `[[trunks]]` once — those are the platform's — and
//! the tenants' trunks and routing rules from the database: at startup,
//! whenever the API says one changed, and on a slow tick, so a node that
//! missed the event is never wrong for long.

use crate::trunk_health::TrunkHealthMonitor;
use crate::trunk_set::TrunkSet;
use fcp_core::storage::{RoutingRuleStore, TenantStore, TrunkStore};
use fcp_events::filter::EventType;
use fcp_events::{EventFilter, EventSubscriber, PlatformEvent, SystemEvent};
use fcp_policy::TenantRuleSet;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// The `entity_type` the trunks API puts on `system.config_changed` when a
/// trunk is created, changed or deleted.
pub const TRUNK_ENTITY: &str = "trunk";

/// The `entity_type` the routing rules API puts on `system.config_changed`
/// when a rule is created, changed or deleted.
pub const ROUTING_RULE_ENTITY: &str = "routing_rule";

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
    watch_config_changes(
        "Trunk refresh",
        &[TRUNK_ENTITY],
        events,
        backstop,
        move || {
            let (set, storage, health) =
                (Arc::clone(&set), Arc::clone(&storage), Arc::clone(&health));
            async move { refresh_trunks(&set, storage.as_ref(), &health).await }
        },
    )
}

/// Read every tenant's routing rules into `rules`.
pub async fn refresh_routing_rules<S>(rules: &TenantRuleSet, storage: &S)
where
    S: RoutingRuleStore + ?Sized,
{
    match rules.refresh(storage).await {
        Ok(count) => debug!(rules = count, "The tenants' routing rules are current"),
        Err(e) => {
            warn!(error = %e, "Could not read the tenants' routing rules; deciding on what is known")
        }
    }
}

/// Keep `rules` current as [`spawn_trunk_refresh`] keeps the trunks: read
/// before this returns, then on every rule change the API announces and on
/// the backstop tick (`docs/TENANT_TRUNKS.md` §12, phase 6b).
pub async fn spawn_routing_rule_refresh<S>(
    rules: Arc<TenantRuleSet>,
    storage: Arc<S>,
    events: Option<Arc<dyn EventSubscriber>>,
    backstop: Duration,
) -> tokio::task::JoinHandle<()>
where
    S: RoutingRuleStore + Send + Sync + 'static + ?Sized,
{
    refresh_routing_rules(&rules, storage.as_ref()).await;
    info!("Routing rules: every tenant's, decided on this node");
    watch_config_changes(
        "Routing rule refresh",
        &[ROUTING_RULE_ENTITY],
        events,
        backstop,
        move || {
            let (rules, storage) = (Arc::clone(&rules), Arc::clone(&storage));
            async move { refresh_routing_rules(&rules, storage.as_ref()).await }
        },
    )
}

/// Run `refresh` whenever the API announces that an entity of one of
/// `entities` changed, and on the backstop tick, so a node that missed the
/// event is never wrong for long. The caller has already done the first
/// read.
fn watch_config_changes<F, Fut>(
    label: &'static str,
    entities: &'static [&'static str],
    events: Option<Arc<dyn EventSubscriber>>,
    backstop: Duration,
    refresh: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        let filter = EventFilter::new().types(vec![EventType::System]);
        let mut stream = match &events {
            Some(bus) => match bus.subscribe(filter.clone()).await {
                Ok(stream) => Some(stream),
                Err(e) => {
                    warn!(error = %e, "{label} could not subscribe; the tick alone keeps it current");
                    None
                }
            },
            None => None,
        };

        let mut ticker = tokio::time::interval(backstop);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // the first tick is immediate, and the caller just read

        loop {
            // A stream that ended is taken up again; until it is, the tick
            // is what keeps this node current.
            if stream.is_none() {
                if let Some(bus) = &events {
                    match bus.subscribe(filter.clone()).await {
                        Ok(open) => {
                            info!("{label} resubscribed");
                            stream = Some(open);
                        }
                        Err(e) => warn!(error = %e, "{label} could not resubscribe"),
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
                                warn!("{label} event stream ended");
                                stream = None;
                                continue;
                            }
                        },
                    };
                    match event {
                        Some(event) if is_change_to(&event, entities) => {
                            debug!("{label}: a change was announced; re-reading")
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
            refresh().await;
        }
    })
}

/// Whether this event says an entity of one of `entities` changed.
fn is_change_to(event: &PlatformEvent, entities: &[&str]) -> bool {
    matches!(
        event,
        PlatformEvent::System(SystemEvent::ConfigChanged { entity_type, .. })
            if entities.contains(&entity_type.as_str())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::TenantId;
    use std::time::SystemTime;

    #[test]
    fn only_a_change_to_what_is_watched_reloads_it() {
        let changed = |entity_type: &str| {
            PlatformEvent::System(SystemEvent::ConfigChanged {
                entity_type: entity_type.to_string(),
                entity_id: "trunk-1".to_string(),
                tenant_id: TenantId::new(),
                timestamp: SystemTime::now(),
            })
        };
        assert!(is_change_to(&changed("trunk"), &[TRUNK_ENTITY]));
        assert!(!is_change_to(&changed("device"), &[TRUNK_ENTITY]));
        assert!(is_change_to(
            &changed("routing_rule"),
            &[ROUTING_RULE_ENTITY]
        ));
        assert!(!is_change_to(&changed("trunk"), &[ROUTING_RULE_ENTITY]));
    }
}
