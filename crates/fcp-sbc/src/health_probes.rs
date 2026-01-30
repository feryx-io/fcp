//! Periodic OPTIONS health probes for SBC trunks.

use crate::trunk_health::TrunkHealthMonitor;
use fcp_sbc_core::trunk::TrunkConfig;
use fcp_sip::uac::SipUserAgentClient;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Run periodic OPTIONS health probes for all enabled trunks.
///
/// This function loops forever. Spawn it with `tokio::spawn`.
/// Every 5 seconds it checks which trunks are due for probing (based on their
/// configured interval and current health state), sends OPTIONS to each host,
/// and records the result.
pub async fn run_health_probes(
    monitor: Arc<TrunkHealthMonitor>,
    trunks: Vec<TrunkConfig>,
    uac: Arc<SipUserAgentClient>,
) {
    let enabled: Vec<_> = trunks
        .into_iter()
        .filter(|t| t.failover.health_check_enabled)
        .collect();

    if enabled.is_empty() {
        info!("No trunks with health checks enabled, probe task exiting");
        return;
    }

    info!(count = enabled.len(), "Starting OPTIONS health probes");

    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    loop {
        ticker.tick().await;

        for trunk in &enabled {
            if !monitor.should_probe(&trunk.name) {
                continue;
            }

            let success = probe_trunk_hosts(&uac, trunk).await;
            monitor.mark_probed(&trunk.name);

            if success {
                debug!(trunk = %trunk.name, "OPTIONS probe succeeded");
                monitor.record_success(&trunk.name);
            } else {
                warn!(trunk = %trunk.name, "OPTIONS probe failed");
                monitor.record_failure(&trunk.name);
            }
        }
    }
}

/// Probe a trunk by sending OPTIONS to each host in order.
/// Returns true on the first response indicating reachability: 2xx (success)
/// or 401/407 (auth required — trunk is alive but demanding credentials).
async fn probe_trunk_hosts(uac: &SipUserAgentClient, trunk: &TrunkConfig) -> bool {
    use fcp_core::transport::TransportType;

    let transport_param = match trunk.transport.protocol {
        TransportType::Udp => None,
        TransportType::Tcp => Some("tcp"),
        TransportType::Tls => Some("tls"),
        TransportType::Ws => Some("ws"),
        TransportType::Wss => Some("wss"),
    };

    for host in &trunk.hosts {
        let uri = match (trunk.transport.port, transport_param) {
            (Some(p), Some(t)) => format!("sip:{}:{};transport={}", host, p, t),
            (Some(p), None) => format!("sip:{}:{}", host, p),
            (None, Some(t)) => format!("sip:{};transport={}", host, t),
            (None, None) => format!("sip:{}", host),
        };

        match tokio::time::timeout(
            Duration::from_secs(5),
            uac.inner().ping_options(uri.as_str()),
        )
        .await
        {
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
