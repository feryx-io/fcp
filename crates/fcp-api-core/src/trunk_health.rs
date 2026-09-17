//! Trunk health monitoring via periodic SIP OPTIONS probes.
//!
//! Sends OPTIONS to each trunk's hosts and tracks healthy/degraded/down status.
//! Designed to run as a background task alongside the admin API.

use dashmap::DashMap;
use fcp_core::storage::TrunkStore;
use fcp_core::{DynStorage, TenantId};
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Health status for a trunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Down,
    Unknown,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Down => write!(f, "down"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Per-trunk health information.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrunkHealthInfo {
    pub status: HealthStatus,
    pub consecutive_failures: u32,
    pub last_probe_ms: Option<u64>,
    pub last_success_ms: Option<u64>,
    pub last_failure_ms: Option<u64>,
}

/// Shared map of trunk health, keyed by trunk ID.
pub type TrunkHealthMap = Arc<DashMap<String, TrunkHealthEntry>>;

/// Internal mutable entry for health tracking.
pub struct TrunkHealthEntry {
    pub status: HealthStatus,
    pub consecutive_failures: u32,
    pub max_failures: u32,
    pub recovery_interval_secs: u64,
    pub last_probe: Option<Instant>,
    pub last_success: Option<Instant>,
    pub last_failure: Option<Instant>,
    /// When the trunk first became healthy (reset on any failure).
    pub healthy_since: Option<Instant>,
}

impl TrunkHealthEntry {
    /// Uptime in seconds (how long continuously healthy), or None.
    pub fn uptime_secs(&self) -> Option<u64> {
        if self.status == HealthStatus::Healthy {
            self.healthy_since
                .map(|t| Instant::now().duration_since(t).as_secs())
        } else {
            None
        }
    }
}

/// Create a new empty health map.
pub fn new_health_map() -> TrunkHealthMap {
    Arc::new(DashMap::new())
}

/// Run the trunk health probe loop. Spawns with `tokio::spawn`.
///
/// Periodically loads enabled trunks from the DB and sends SIP OPTIONS
/// to each trunk's hosts. Updates the shared health map with results.
pub async fn run_trunk_health_probes(
    storage: Arc<DynStorage>,
    health_map: TrunkHealthMap,
    tenant_id: TenantId,
) {
    info!("Starting trunk health probe loop");

    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    loop {
        ticker.tick().await;

        // Load current trunks from DB
        let trunks = match storage.list_trunks(&tenant_id).await {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "Failed to load trunks for health probing");
                continue;
            }
        };

        // Remove stale entries for deleted trunks
        let current_ids: std::collections::HashSet<String> =
            trunks.iter().map(|t| t.id.to_string()).collect();
        health_map.retain(|k, _| current_ids.contains(k));

        for trunk in &trunks {
            let trunk_id = trunk.id.to_string();

            if !trunk.enabled || !trunk.failover.health_check_enabled || trunk.hosts.is_empty() {
                // Ensure entry exists but don't probe
                health_map
                    .entry(trunk_id.clone())
                    .or_insert_with(|| TrunkHealthEntry {
                        status: if trunk.enabled {
                            HealthStatus::Unknown
                        } else {
                            HealthStatus::Down
                        },
                        consecutive_failures: 0,
                        max_failures: trunk.failover.max_failures,
                        recovery_interval_secs: trunk.failover.recovery_interval_secs,
                        last_probe: None,
                        last_success: None,
                        last_failure: None,
                        healthy_since: None,
                    });
                continue;
            }

            // Check if probe is due
            let should_probe = {
                match health_map.get(&trunk_id) {
                    Some(entry) => {
                        let interval = if entry.status == HealthStatus::Down {
                            entry.recovery_interval_secs
                        } else {
                            trunk.failover.health_check_interval_secs
                        };
                        match entry.last_probe {
                            Some(last) => Instant::now().duration_since(last).as_secs() >= interval,
                            None => true,
                        }
                    }
                    None => true,
                }
            };

            if !should_probe {
                continue;
            }

            // Send OPTIONS probe
            let success = probe_trunk_hosts(trunk).await;

            // Update health map
            let now = Instant::now();
            let mut entry =
                health_map
                    .entry(trunk_id.clone())
                    .or_insert_with(|| TrunkHealthEntry {
                        status: HealthStatus::Unknown,
                        consecutive_failures: 0,
                        max_failures: trunk.failover.max_failures,
                        recovery_interval_secs: trunk.failover.recovery_interval_secs,
                        last_probe: None,
                        last_success: None,
                        last_failure: None,
                        healthy_since: None,
                    });

            entry.last_probe = Some(now);
            entry.max_failures = trunk.failover.max_failures;
            entry.recovery_interval_secs = trunk.failover.recovery_interval_secs;

            if success {
                let was_healthy = entry.status == HealthStatus::Healthy;
                entry.consecutive_failures = 0;
                entry.last_success = Some(now);
                entry.status = HealthStatus::Healthy;
                if !was_healthy {
                    entry.healthy_since = Some(now);
                }
                debug!(trunk = %trunk.name, "OPTIONS probe succeeded");
            } else {
                entry.consecutive_failures += 1;
                entry.last_failure = Some(now);
                entry.healthy_since = None; // Reset uptime on failure
                if entry.consecutive_failures >= entry.max_failures {
                    entry.status = HealthStatus::Down;
                } else if entry.consecutive_failures >= entry.max_failures / 2 {
                    entry.status = HealthStatus::Degraded;
                }
                warn!(
                    trunk = %trunk.name,
                    failures = entry.consecutive_failures,
                    status = %entry.status,
                    "OPTIONS probe failed"
                );
            }

            // Check degraded recovery
            if entry.status == HealthStatus::Degraded {
                if let Some(last_fail) = entry.last_failure {
                    if now.duration_since(last_fail).as_secs() >= entry.recovery_interval_secs {
                        entry.status = HealthStatus::Healthy;
                        entry.consecutive_failures = 0;
                        entry.healthy_since = Some(now);
                        info!(trunk = %trunk.name, "Degraded trunk auto-recovered to Healthy");
                    }
                }
            }
        }
    }
}

/// Public wrapper for probe_trunk_hosts (used by test-call endpoint).
pub async fn probe_trunk_hosts_public(trunk: &fcp_core::Trunk) -> bool {
    probe_trunk_hosts(trunk).await
}

/// Send a SIP OPTIONS probe to a trunk's hosts via raw UDP.
/// Returns true if any host responds (2xx, 401, 407 all count as reachable).
async fn probe_trunk_hosts(trunk: &fcp_core::Trunk) -> bool {
    let default_port = trunk.transport.port.unwrap_or(5060);

    for host_str in &trunk.hosts {
        let (host, port) = parse_host_port(host_str, default_port);
        let target = format!("{}:{}", host, port);

        match tokio::time::timeout(
            Duration::from_secs(5),
            send_options_probe(&target, &host, port),
        )
        .await
        {
            Ok(Ok(true)) => return true,
            Ok(Ok(false)) => {
                debug!(host = %target, "OPTIONS probe got non-success response");
            }
            Ok(Err(e)) => {
                debug!(host = %target, error = %e, "OPTIONS probe error");
            }
            Err(_) => {
                debug!(host = %target, "OPTIONS probe timed out (5s)");
            }
        }
    }
    false
}

/// Send a raw SIP OPTIONS request via UDP and check for a response.
async fn send_options_probe(
    target: &str,
    host: &str,
    port: u16,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let target_addr = target.to_string();
    let host_owned = host.to_string();

    tokio::task::spawn_blocking(move || {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;

        let call_id = uuid::Uuid::new_v4();
        let branch = format!("z9hG4bK{}", &call_id.to_string()[..12]);
        let local_addr = socket.local_addr()?;

        let options = format!(
            "OPTIONS sip:ping@{host}:{port} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {local};branch={branch};rport\r\n\
             From: <sip:healthcheck@fcp.local>;tag={tag}\r\n\
             To: <sip:ping@{host}:{port}>\r\n\
             Call-ID: {call_id}@fcp.local\r\n\
             CSeq: 1 OPTIONS\r\n\
             Max-Forwards: 70\r\n\
             User-Agent: FCP-HealthProbe/1.0\r\n\
             Content-Length: 0\r\n\
             \r\n",
            host = host_owned,
            port = port,
            local = local_addr,
            branch = branch,
            tag = &call_id.to_string()[..8],
            call_id = call_id,
        );

        socket.send_to(options.as_bytes(), &target_addr)?;

        let mut buf = [0u8; 4096];
        match socket.recv_from(&mut buf) {
            Ok((len, _)) => {
                let response = String::from_utf8_lossy(&buf[..len]);
                // Check for SIP response status line: "SIP/2.0 XXX"
                if let Some(status_line) = response.lines().next() {
                    if let Some(code_str) = status_line.split_whitespace().nth(1) {
                        if let Ok(code) = code_str.parse::<u16>() {
                            // 2xx = success, 401/407 = trunk is alive but wants auth
                            return Ok((200..300).contains(&code) || code == 401 || code == 407);
                        }
                    }
                }
                Ok(false)
            }
            Err(_) => Ok(false), // timeout or error = probe failed
        }
    })
    .await?
}

fn parse_host_port(host_str: &str, default_port: u16) -> (String, u16) {
    if let Some((host, port_str)) = host_str.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            return (host.to_string(), port);
        }
    }
    (host_str.to_string(), default_port)
}
