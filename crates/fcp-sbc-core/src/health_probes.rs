//! Periodic OPTIONS health probes for SBC trunks.

use crate::metrics::TrunkHealthStatus;
use crate::trunk::TrunkConfig;
use crate::trunk_health::TrunkHealthMonitor;
use crate::trunk_set::TrunkSet;
use fcp_core::transport::TransportType;
use fcp_sip::uac::SipUserAgentClient;
use sip_dns::{DnsTarget, Transport as DnsTransport};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Run periodic OPTIONS health probes for every trunk this node routes by —
/// its own and every tenant's (`docs/TENANT_TRUNKS.md` §4). A tenant trunk
/// created while the node runs is probed as soon as the set holds it.
///
/// This function loops forever. Spawn it with `tokio::spawn`.
/// Every 5 seconds it checks which trunks are due for probing (based on their
/// configured interval and current health state), sends OPTIONS to each host,
/// and records the result.
pub async fn run_health_probes(
    monitor: Arc<TrunkHealthMonitor>,
    trunks: Arc<TrunkSet>,
    uac: Arc<SipUserAgentClient>,
    announce: HealthAnnouncer,
) {
    info!("Starting OPTIONS health probes");

    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    loop {
        ticker.tick().await;

        // Auto-recover Degraded trunks whose last failure is old enough
        for (key, name) in monitor.check_degraded_recovery() {
            info!(trunk = %name, "Degraded trunk auto-recovered to Healthy");
            announce
                .changed(&trunks, &key, TrunkHealthStatus::Degraded, &monitor)
                .await;
        }

        let enabled: Vec<_> = trunks
            .all()
            .into_iter()
            .filter(|t| t.failover.health_check_enabled)
            .collect();

        for trunk in &enabled {
            // Health is kept per trunk, not per name (`docs/TENANT_TRUNKS.md`
            // §4); the name is what the log says.
            let key = trunk.key();
            if !monitor.should_probe(&key) {
                continue;
            }

            let success = probe_trunk_hosts(&uac, trunk).await;
            monitor.mark_probed(&key);

            let was = if success {
                debug!(trunk = %trunk.name, "OPTIONS probe succeeded");
                monitor.record_success(&key)
            } else {
                warn!(trunk = %trunk.name, "OPTIONS probe failed");
                monitor.record_failure(&key)
            };
            // Only a change is news: a trunk that was healthy and still is
            // says nothing.
            if let Some(was) = was {
                announce.changed(&trunks, &key, was, &monitor).await;
            }
        }
    }
}

/// Probe a trunk by sending OPTIONS to each host in order.
/// Returns true on the first response indicating reachability: 2xx (success)
/// or 401/407 (auth required — trunk is alive but demanding credentials).
async fn probe_trunk_hosts(uac: &SipUserAgentClient, trunk: &TrunkConfig) -> bool {
    let transport = map_transport(trunk.transport.protocol);
    let default_port = trunk.transport.protocol.default_port();

    for host in &trunk.hosts {
        let target = DnsTarget::unchecked_new(
            host.as_str(),
            trunk.transport.port.unwrap_or(default_port),
            transport,
        );

        match tokio::time::timeout(Duration::from_secs(5), uac.inner().ping_options(target)).await {
            Ok(Ok(response)) => {
                let code = response.code();
                // 2xx = success, 401/407 = trunk is reachable but requires auth
                if (200..300).contains(&code) || code == 401 || code == 407 {
                    return true;
                }
                debug!(host = %host, code, "OPTIONS probe got non-success response");
            }
            Ok(Err(e)) => {
                debug!(host = %host, error = %e, "OPTIONS probe error");
            }
            Err(_) => {
                debug!(host = %host, "OPTIONS probe timed out (5s)");
            }
        }
    }
    false
}

fn map_transport(t: TransportType) -> DnsTransport {
    match t {
        TransportType::Udp => DnsTransport::Udp,
        TransportType::Tcp => DnsTransport::Tcp,
        TransportType::Tls => DnsTransport::Tls,
        TransportType::Ws => DnsTransport::Ws,
        TransportType::Wss => DnsTransport::Wss,
    }
}

/// Where a change in what this node's probes find gets announced
/// (`docs/TENANT_TRUNKS.md` §9). A node with nowhere to publish keeps its
/// findings to itself, which is what every node did before.
#[derive(Clone, Default)]
pub struct HealthAnnouncer {
    events: Option<Arc<dyn fcp_events::EventPublisher>>,
    node_id: String,
}

impl HealthAnnouncer {
    pub fn new(events: Arc<dyn fcp_events::EventPublisher>, node_id: String) -> Self {
        Self {
            events: Some(events),
            node_id,
        }
    }

    /// Announce that `key`'s health here is no longer `was`.
    async fn changed(
        &self,
        trunks: &TrunkSet,
        key: &str,
        was: TrunkHealthStatus,
        monitor: &TrunkHealthMonitor,
    ) {
        let Some(events) = &self.events else {
            return;
        };
        let Some(trunk) = trunks.find(key) else {
            return;
        };
        let status = monitor.status(key).unwrap_or_default();
        let event = fcp_events::PlatformEvent::Trunk(fcp_events::TrunkEvent::HealthChanged {
            trunk_id: key.to_string(),
            trunk_name: trunk.name.clone(),
            tenant_id: trunk.tenant_id,
            status: status.to_string(),
            previous: was.to_string(),
            consecutive_failures: monitor.consecutive_failures(key),
            node_id: self.node_id.clone(),
            timestamp: std::time::SystemTime::now(),
        });
        if let Err(e) = events.publish(event).await {
            debug!(error = %e, "Could not publish the trunk health change");
        }
    }
}
