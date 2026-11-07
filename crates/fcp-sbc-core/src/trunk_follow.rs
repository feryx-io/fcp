//! Trunk health taken from the call managers rather than probed again
//! (#336).
//!
//! The conference, IVR and voicemail servers route their dial-outs by the
//! same trunks as the call manager, and each used to probe every one of
//! them: four OPTIONS per carrier address every interval where one would
//! do. A node with the cluster's Redis now reads what the call managers
//! publish of their trunks (`trunk_report`, `docs/TENANT_TRUNKS.md` §9)
//! and takes their finding as its own. A node without it has no one to
//! follow, and probes as before.

use crate::health_probes::{run_health_probes, HealthAnnouncer};
use crate::metrics::TrunkHealthStatus;
use crate::trunk_health::TrunkHealthMonitor;
use crate::trunk_set::TrunkSet;
use async_trait::async_trait;
use fcp_core::trunk_report::{TrunkNodeReport, TrunkNodeView};
use fcp_sip::uac::SipUserAgentClient;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

/// How often a follower reads the call managers' views. They publish
/// every 10 s.
pub const FOLLOW_INTERVAL: Duration = Duration::from_secs(5);

/// Where the call managers' views of their trunks are read.
#[async_trait]
pub trait TrunkViews: Send + Sync {
    async fn reports(&self) -> anyhow::Result<Vec<TrunkNodeReport>>;
}

#[async_trait]
impl TrunkViews for fcp_cluster::TrunkRegistry {
    async fn reports(&self) -> anyhow::Result<Vec<TrunkNodeReport>> {
        fcp_cluster::TrunkRegistry::reports(self).await
    }
}

/// Keep `health` current for a node that routes by trunks but is not a
/// call manager: follow `views` when there are any, else probe.
pub fn watch_trunk_health(
    health: Arc<TrunkHealthMonitor>,
    trunks: Arc<TrunkSet>,
    uac: Arc<SipUserAgentClient>,
    announce: HealthAnnouncer,
    views: Option<Arc<dyn TrunkViews>>,
) {
    match views {
        Some(views) => {
            info!("Trunk health follows the call managers' probes");
            tokio::spawn(follow_trunk_health(health, views, FOLLOW_INTERVAL));
        }
        None => {
            tokio::spawn(run_health_probes(health, trunks, uac, announce));
        }
    }
}

/// Read `views` every `every` and take each trunk's health from them.
///
/// This function loops forever. Spawn it with `tokio::spawn`.
pub async fn follow_trunk_health(
    health: Arc<TrunkHealthMonitor>,
    views: Arc<dyn TrunkViews>,
    every: Duration,
) {
    let mut ticker = tokio::time::interval(every);
    loop {
        ticker.tick().await;
        match views.reports().await {
            Ok(reports) => apply_reports(&health, &reports),
            Err(e) => debug!(error = %e, "Could not read the call managers' trunk views"),
        }
    }
}

/// Take each trunk's health from the best of the call managers' views of
/// it: a trunk one of them can reach is reachable. A trunk no call manager
/// has probed yet, or reports at all, keeps what it has.
pub fn apply_reports(health: &TrunkHealthMonitor, reports: &[TrunkNodeReport]) {
    let mut best: HashMap<&str, (TrunkHealthStatus, &TrunkNodeView)> = HashMap::new();
    for view in reports.iter().flat_map(|r| &r.trunks) {
        let Some(status) = parse_status(&view.health) else {
            continue;
        };
        let better = best
            .get(view.trunk_id.as_str())
            .is_none_or(|(held, _)| rank(status) > rank(*held));
        if better {
            best.insert(view.trunk_id.as_str(), (status, view));
        }
    }
    for (trunk_id, (status, view)) in best {
        if let Some(was) = health.follow(
            trunk_id,
            status,
            view.consecutive_failures,
            &view.hosts_down,
        ) {
            info!(
                trunk = %view.name,
                from = was.as_str(),
                to = status.as_str(),
                "Trunk health changed, as the call managers found it"
            );
        }
    }
}

/// A finding a call manager has made; `unknown` is none.
fn parse_status(health: &str) -> Option<TrunkHealthStatus> {
    match health {
        "healthy" => Some(TrunkHealthStatus::Healthy),
        "degraded" => Some(TrunkHealthStatus::Degraded),
        "down" => Some(TrunkHealthStatus::Down),
        _ => None,
    }
}

fn rank(status: TrunkHealthStatus) -> u8 {
    match status {
        TrunkHealthStatus::Healthy => 3,
        TrunkHealthStatus::Degraded => 2,
        TrunkHealthStatus::Down => 1,
        TrunkHealthStatus::Unknown => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trunk::TrunkConfig;
    use std::time::SystemTime;

    fn trunk(name: &str) -> TrunkConfig {
        TrunkConfig {
            name: name.to_string(),
            hosts: vec!["a.example".to_string(), "b.example".to_string()],
            ..Default::default()
        }
    }

    fn view(trunk: &TrunkConfig, health: &str, hosts_down: &[&str]) -> TrunkNodeView {
        TrunkNodeView {
            node_id: None,
            trunk_id: trunk.key(),
            name: trunk.name.clone(),
            tenant_id: None,
            enabled: true,
            health: health.to_string(),
            consecutive_failures: if health == "down" { 3 } else { 0 },
            hosts_down: hosts_down.iter().map(|h| h.to_string()).collect(),
            calls_in_flight: 0,
            routable: health != "down",
            registration: None,
            account: None,
        }
    }

    fn report(node: &str, trunks: Vec<TrunkNodeView>) -> TrunkNodeReport {
        TrunkNodeReport {
            node_id: node.to_string(),
            reported_at: SystemTime::now(),
            trunks,
        }
    }

    #[test]
    fn a_trunk_takes_the_call_managers_finding() {
        let carrier = trunk("carrier");
        let health = TrunkHealthMonitor::new(std::slice::from_ref(&carrier));
        apply_reports(
            &health,
            &[report(
                "cm-1",
                vec![view(&carrier, "down", &["a.example", "b.example"])],
            )],
        );
        assert_eq!(health.status("carrier"), Some(TrunkHealthStatus::Down));
        assert!(!health.is_healthy("carrier"));
        assert_eq!(health.hosts_down("carrier"), ["a.example", "b.example"]);

        apply_reports(
            &health,
            &[report("cm-1", vec![view(&carrier, "healthy", &[])])],
        );
        assert_eq!(health.status("carrier"), Some(TrunkHealthStatus::Healthy));
        assert!(health.hosts_down("carrier").is_empty());
    }

    #[test]
    fn the_best_finding_wins_and_none_is_no_finding() {
        let carrier = trunk("carrier");
        let health = TrunkHealthMonitor::new(std::slice::from_ref(&carrier));
        // One call manager cannot reach it, another can.
        apply_reports(
            &health,
            &[
                report("cm-1", vec![view(&carrier, "down", &["a.example"])]),
                report("cm-2", vec![view(&carrier, "healthy", &[])]),
            ],
        );
        assert_eq!(health.status("carrier"), Some(TrunkHealthStatus::Healthy));
        // A call manager that has not probed yet does not undo one that has.
        apply_reports(
            &health,
            &[
                report("cm-1", vec![view(&carrier, "down", &["a.example"])]),
                report("cm-3", vec![view(&carrier, "unknown", &[])]),
            ],
        );
        assert_eq!(health.status("carrier"), Some(TrunkHealthStatus::Down));
        // Nobody reporting the trunk leaves it as it was.
        apply_reports(&health, &[]);
        assert_eq!(health.status("carrier"), Some(TrunkHealthStatus::Down));
    }
}
