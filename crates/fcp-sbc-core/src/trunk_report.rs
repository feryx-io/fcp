//! What one node sees of the trunks it routes by (`docs/TENANT_TRUNKS.md` §9).
//!
//! Health is per node and always was: a carrier unreachable from one node
//! may be reachable from another, and a call should be placed by a node
//! that can reach it. What was missing is anywhere to *see* that. A node
//! answers for itself on `GET /trunks`, and publishes the same view to the
//! cluster so `/v1/trunks` can show every node's at once — including the
//! disagreements, which are the interesting part.

use crate::trunk_health::TrunkHealthMonitor;
use crate::trunk_registration::TrunkRegistrar;
use crate::trunk_set::TrunkSet;
use fcp_core::trunk_report::{TrunkNodeReport, TrunkNodeView};
use std::sync::Arc;
use std::time::SystemTime;

/// What this node sees of every trunk it routes by, right now.
pub fn node_view(
    node_id: &str,
    trunks: &TrunkSet,
    health: &TrunkHealthMonitor,
    registrar: Option<&TrunkRegistrar>,
) -> TrunkNodeReport {
    let mut views: Vec<TrunkNodeView> = trunks
        .all()
        .iter()
        .map(|trunk| view_of(trunk, health, registrar))
        .collect();
    // A stable order, so two nodes' answers line up beside each other.
    views.sort_by(|a, b| a.name.cmp(&b.name).then(a.trunk_id.cmp(&b.trunk_id)));
    TrunkNodeReport {
        node_id: node_id.to_string(),
        reported_at: SystemTime::now(),
        trunks: views,
    }
}

fn view_of(
    trunk: &Arc<fcp_core::Trunk>,
    health: &TrunkHealthMonitor,
    registrar: Option<&TrunkRegistrar>,
) -> TrunkNodeView {
    let key = trunk.key();
    let status = health
        .status(&key)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    TrunkNodeView {
        node_id: None,
        trunk_id: key.clone(),
        name: trunk.name.clone(),
        tenant_id: trunk.tenant_id.map(|t| t.to_string()),
        enabled: trunk.enabled,
        health: status,
        consecutive_failures: health.consecutive_failures(&key),
        calls_in_flight: health.concurrent_calls(&key),
        routable: trunk.enabled && health.is_healthy(&key),
        registration: trunk.register.then(|| {
            registrar
                .and_then(|r| r.registration_state(&key))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unregistered".to_string())
        }),
        account: registrar.and_then(|r| r.account(&key)),
    }
}

/// Publish what this node sees, every `every`, for as long as it runs
/// (`docs/TENANT_TRUNKS.md` §9). A deployment with no Redis has nowhere to
/// publish and answers for itself on `GET /trunks` alone.
pub async fn run_trunk_reporting(
    registry: Arc<fcp_cluster::TrunkRegistry>,
    trunks: Arc<TrunkSet>,
    health: Arc<TrunkHealthMonitor>,
    registrar: Option<Arc<TrunkRegistrar>>,
    every: std::time::Duration,
) {
    let node_id = registry.node_id().to_string();
    tracing::info!(node_id = %node_id, "Publishing this node's trunk view");
    let mut ticker = tokio::time::interval(every);
    loop {
        ticker.tick().await;
        let report = node_view(&node_id, &trunks, &health, registrar.as_deref());
        if let Err(e) = registry.publish(&report).await {
            tracing::debug!(error = %e, "Could not publish this node's trunk view");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::{TenantId, Trunk, TrunkId};

    fn trunk(name: &str) -> Arc<Trunk> {
        let mut t = Trunk::new(name.to_string(), TenantId::new());
        t.tenant_id = None;
        t.id = TrunkId::platform(name);
        t.hosts = vec!["carrier.example".to_string()];
        Arc::new(t)
    }

    /// The view is this node's own: its probes, its calls, its binding.
    #[test]
    fn a_node_answers_for_what_it_routes_by() {
        let set = TrunkSet::platform_only(vec![(*trunk("carrier-a")).clone()]);
        let health = TrunkHealthMonitor::new(&[]);
        health.track(&set.all());
        let key = set.all()[0].key();
        health.record_success(&key);
        health.call_started(&key);

        let report = node_view("node-a", &set, &health, None);
        assert_eq!(report.node_id, "node-a");
        assert_eq!(report.trunks.len(), 1);
        let view = &report.trunks[0];
        assert_eq!(view.name, "carrier-a");
        assert_eq!(view.health, "healthy");
        assert_eq!(view.calls_in_flight, 1);
        assert!(view.routable);
        assert_eq!(view.registration, None, "this trunk does not register");
    }

    /// A trunk this node's probes have knocked down is not routable here,
    /// which is the whole point of keeping health per node.
    #[test]
    fn a_trunk_this_node_cannot_reach_is_not_routable_here() {
        let mut t = (*trunk("carrier-a")).clone();
        t.failover.max_failures = 1;
        let set = TrunkSet::platform_only(vec![t]);
        let health = TrunkHealthMonitor::new(&[]);
        health.track(&set.all());
        let key = set.all()[0].key();
        health.record_failure(&key);

        let report = node_view("node-a", &set, &health, None);
        assert_eq!(report.trunks[0].health, "down");
        assert!(!report.trunks[0].routable);
        assert_eq!(report.trunks[0].consecutive_failures, 1);
    }

    /// A registering trunk says what its binding is doing here; one that
    /// does not register says nothing at all.
    #[test]
    fn a_registering_trunk_reports_its_binding() {
        let mut registering = (*trunk("registers")).clone();
        registering.register = true;
        let set = TrunkSet::platform_only(vec![registering, (*trunk("plain")).clone()]);
        let health = TrunkHealthMonitor::new(&[]);
        health.track(&set.all());
        let registrar = TrunkRegistrar::new();
        registrar.track(&set.all());
        let key = set
            .all()
            .iter()
            .find(|t| t.name == "registers")
            .unwrap()
            .key();
        registrar.record_registered(&key, 3600);

        let report = node_view("node-a", &set, &health, Some(&registrar));
        let registering = report
            .trunks
            .iter()
            .find(|v| v.name == "registers")
            .unwrap();
        let plain = report.trunks.iter().find(|v| v.name == "plain").unwrap();
        assert_eq!(registering.registration.as_deref(), Some("registered"));
        assert_eq!(plain.registration, None);
    }

    /// A disabled trunk is listed — a tenant that turned one off should
    /// see it — and is routable nowhere.
    #[test]
    fn a_disabled_trunk_is_listed_and_routable_nowhere() {
        let mut t = (*trunk("carrier-a")).clone();
        t.enabled = false;
        let set = TrunkSet::platform_only(vec![t]);
        let health = TrunkHealthMonitor::new(&[]);
        health.track(&set.all());

        let report = node_view("node-a", &set, &health, None);
        assert_eq!(report.trunks.len(), 1);
        assert!(!report.trunks[0].enabled);
        assert!(!report.trunks[0].routable);
    }

    /// The report round-trips: it is what a node publishes to its peers.
    #[test]
    fn a_report_survives_the_wire() {
        let set = TrunkSet::platform_only(vec![(*trunk("carrier-a")).clone()]);
        let health = TrunkHealthMonitor::new(&[]);
        health.track(&set.all());
        let report = node_view("node-a", &set, &health, None);

        let json = serde_json::to_string(&report).expect("serialises");
        let back: TrunkNodeReport = serde_json::from_str(&json).expect("parses");
        assert_eq!(back.node_id, "node-a");
        assert_eq!(back.trunks, report.trunks);
    }
}
