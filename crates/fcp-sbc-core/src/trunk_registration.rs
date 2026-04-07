//! Outbound trunk registration for the SBC.
//!
//! Trunks with `register: true` in their configuration need to maintain
//! active REGISTER bindings with their remote peers. Most SIP carriers
//! require registration before they accept outbound calls.
//!
//! This module provides:
//! - [`TrunkRegistrar`]: Tracks registration state per trunk
//! - [`run_trunk_registration`]: Background task that maintains registrations

use crate::trunk::TrunkConfig;
use crate::trunk_health::TrunkHealthMonitor;
use dashmap::DashMap;
use fcp_core::transport::TransportType;
use fcp_sip::uac::SipUserAgentClient;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Registration state for a trunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationState {
    /// Not yet registered or de-registered.
    Unregistered,
    /// REGISTER request in flight.
    Pending,
    /// Successfully registered with the remote peer.
    Registered,
    /// Last registration attempt failed.
    Failed,
}

impl std::fmt::Display for RegistrationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unregistered => write!(f, "unregistered"),
            Self::Pending => write!(f, "pending"),
            Self::Registered => write!(f, "registered"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Per-trunk registration entry.
struct TrunkRegEntry {
    reg_state: RegistrationState,
    expires_at: Option<Instant>,
    configured_expiry: u32,
    last_attempt: Option<Instant>,
    last_error: Option<String>,
    registrar_uri: String,
}

/// Tracks outbound registration state for trunks that require it.
pub struct TrunkRegistrar {
    state: DashMap<String, TrunkRegEntry>,
}

impl TrunkRegistrar {
    /// Create a new registrar from trunk configurations.
    ///
    /// Only trunks with `register: true` are included.
    pub fn new(trunks: &[TrunkConfig]) -> Self {
        let state = DashMap::new();
        for trunk in trunks {
            if !trunk.register {
                continue;
            }
            let uri = match build_registrar_uri(trunk) {
                Some(uri) => uri,
                None => {
                    warn!(trunk = %trunk.name, "Trunk has register=true but no hosts configured, skipping");
                    continue;
                }
            };
            state.insert(
                trunk.name.clone(),
                TrunkRegEntry {
                    reg_state: RegistrationState::Unregistered,
                    expires_at: None,
                    configured_expiry: trunk.register_expiry.unwrap_or(3600),
                    last_attempt: None,
                    last_error: None,
                    registrar_uri: uri,
                },
            );
        }
        Self { state }
    }

    /// Get the current registration state for a trunk.
    pub fn registration_state(&self, trunk: &str) -> Option<RegistrationState> {
        self.state.get(trunk).map(|e| e.reg_state)
    }

    /// Returns `true` if the trunk needs (re-)registration.
    ///
    /// A trunk needs registration when:
    /// - It is `Unregistered` or `Failed`
    /// - It is `Registered` but expiry is within 50% of the configured interval
    pub fn needs_registration(&self, trunk: &str) -> bool {
        self.state
            .get(trunk)
            .map(|e| match e.reg_state {
                RegistrationState::Unregistered | RegistrationState::Failed => true,
                RegistrationState::Pending => false,
                RegistrationState::Registered => {
                    match e.expires_at {
                        Some(expiry) => {
                            let now = Instant::now();
                            if now >= expiry {
                                return true;
                            }
                            let remaining = expiry.duration_since(now);
                            let threshold = Duration::from_secs(u64::from(e.configured_expiry) / 2);
                            remaining <= threshold
                        }
                        None => true, // No expiry recorded — re-register
                    }
                }
            })
            .unwrap_or(false)
    }

    /// Record a successful registration.
    pub fn record_registered(&self, trunk: &str, expires_secs: u32) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.reg_state = RegistrationState::Registered;
            entry.expires_at = Some(Instant::now() + Duration::from_secs(u64::from(expires_secs)));
            entry.last_error = None;
        }
    }

    /// Record a failed registration attempt.
    pub fn record_failed(&self, trunk: &str, reason: String) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.reg_state = RegistrationState::Failed;
            entry.last_error = Some(reason);
        }
    }

    /// Mark a trunk as pending (REGISTER in flight).
    pub fn mark_pending(&self, trunk: &str) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.reg_state = RegistrationState::Pending;
            entry.last_attempt = Some(Instant::now());
        }
    }

    /// Mark a trunk as unregistered (for admin force-register or de-registration).
    pub fn mark_unregistered(&self, trunk: &str) {
        if let Some(mut entry) = self.state.get_mut(trunk) {
            entry.reg_state = RegistrationState::Unregistered;
            entry.expires_at = None;
            entry.last_error = None;
        }
    }

    /// Get the registrar URI for a trunk.
    pub fn registrar_uri(&self, trunk: &str) -> Option<String> {
        self.state.get(trunk).map(|e| e.registrar_uri.clone())
    }

    /// Get the configured expiry for a trunk.
    pub fn configured_expiry(&self, trunk: &str) -> Option<u32> {
        self.state.get(trunk).map(|e| e.configured_expiry)
    }

    /// Get the last registration error for a trunk.
    pub fn last_error(&self, trunk: &str) -> Option<String> {
        self.state.get(trunk).and_then(|e| e.last_error.clone())
    }

    /// Get all trunk names managed by this registrar.
    pub fn trunk_names(&self) -> Vec<String> {
        self.state.iter().map(|e| e.key().clone()).collect()
    }

    /// Returns `true` if there is at least one trunk that requires registration.
    pub fn has_registerable_trunks(&self) -> bool {
        !self.state.is_empty()
    }
}

/// Build a registrar URI from a trunk's configuration.
///
/// Uses `sips:` for TLS transports and `sip:` otherwise. Only includes
/// the port if it differs from the transport's default.
fn build_registrar_uri(trunk: &TrunkConfig) -> Option<String> {
    let host = trunk.hosts.first()?;
    let scheme = if trunk.transport.protocol == TransportType::Tls {
        "sips"
    } else {
        "sip"
    };
    let default_port = trunk.transport.protocol.default_port();
    let port = trunk.transport.port.unwrap_or(default_port);
    if port == default_port {
        Some(format!("{scheme}:{host}"))
    } else {
        Some(format!("{scheme}:{host}:{port}"))
    }
}

/// Background task that maintains REGISTER bindings for all configured trunks.
///
/// Ticks every 5 seconds. On each tick:
/// 1. Checks if shutdown has been signalled — if so, sends REGISTER with
///    expires=0 for all registered trunks (best-effort) and returns.
/// 2. For each trunk that needs (re-)registration, sends REGISTER and
///    records the result.
///
/// Spawn with `tokio::spawn`. Runs until shutdown is signalled.
pub async fn run_trunk_registration(
    registrar: Arc<TrunkRegistrar>,
    health: Arc<TrunkHealthMonitor>,
    uac: Arc<SipUserAgentClient>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    info!(
        trunks = registrar.trunk_names().len(),
        "Starting trunk registration task"
    );

    let mut ticker = tokio::time::interval(Duration::from_secs(5));

    loop {
        ticker.tick().await;

        // Check for shutdown signal
        if *shutdown.borrow_and_update() {
            info!("Shutdown signalled, de-registering trunks");
            deregister_all(&registrar, &uac).await;
            return;
        }

        for name in registrar.trunk_names() {
            if !registrar.needs_registration(&name) {
                continue;
            }

            let uri = match registrar.registrar_uri(&name) {
                Some(uri) => uri,
                None => continue,
            };
            let expires = registrar.configured_expiry(&name).unwrap_or(3600);

            registrar.mark_pending(&name);

            debug!(trunk = %name, uri = %uri, expires, "Sending REGISTER");

            match tokio::time::timeout(Duration::from_secs(10), uac.register(&uri, expires)).await {
                Ok(Ok(response)) => {
                    let code = response.start_line().code();
                    if (200..300).contains(&code) {
                        info!(trunk = %name, code, "Trunk registered successfully");
                        registrar.record_registered(&name, expires);
                        health.record_success(&name);
                    } else {
                        let reason = format!("{} {}", code, response.start_line().reason());
                        warn!(trunk = %name, code, "Trunk registration rejected");
                        registrar.record_failed(&name, reason);
                        health.record_failure(&name);
                    }
                }
                Ok(Err(e)) => {
                    warn!(trunk = %name, error = %e, "Trunk registration failed");
                    registrar.record_failed(&name, e.to_string());
                    health.record_failure(&name);
                }
                Err(_) => {
                    warn!(trunk = %name, "Trunk registration timed out (10s)");
                    registrar.record_failed(&name, "timeout".to_string());
                    health.record_failure(&name);
                }
            }
        }
    }
}

/// Best-effort de-registration: sends REGISTER with expires=0 for all
/// trunks currently in `Registered` state.
async fn deregister_all(registrar: &TrunkRegistrar, uac: &SipUserAgentClient) {
    for name in registrar.trunk_names() {
        if registrar.registration_state(&name) != Some(RegistrationState::Registered) {
            continue;
        }
        let uri = match registrar.registrar_uri(&name) {
            Some(uri) => uri,
            None => continue,
        };

        debug!(trunk = %name, "Sending de-registration (expires=0)");

        match tokio::time::timeout(Duration::from_secs(3), uac.register(&uri, 0)).await {
            Ok(Ok(response)) => {
                let code = response.start_line().code();
                info!(trunk = %name, code, "Trunk de-registered");
            }
            Ok(Err(e)) => {
                error!(trunk = %name, error = %e, "Trunk de-registration failed");
            }
            Err(_) => {
                warn!(trunk = %name, "Trunk de-registration timed out (3s)");
            }
        }
        registrar.mark_unregistered(&name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trunk::TrunkTransport;

    fn make_trunk(name: &str, register: bool, register_expiry: Option<u32>) -> TrunkConfig {
        TrunkConfig {
            name: name.to_string(),
            register,
            register_expiry,
            hosts: vec!["sip.carrier.com".to_string()],
            ..Default::default()
        }
    }

    #[test]
    fn test_new_only_registering_trunks() {
        let trunks = vec![
            make_trunk("reg-trunk", true, Some(3600)),
            make_trunk("ip-auth-trunk", false, None),
            make_trunk("reg-trunk-2", true, None),
        ];
        let registrar = TrunkRegistrar::new(&trunks);
        let mut names = registrar.trunk_names();
        names.sort();
        assert_eq!(names, vec!["reg-trunk", "reg-trunk-2"]);
    }

    #[test]
    fn test_needs_registration_unregistered() {
        let trunks = vec![make_trunk("t", true, None)];
        let registrar = TrunkRegistrar::new(&trunks);
        assert!(registrar.needs_registration("t"));
    }

    #[test]
    fn test_needs_registration_recently_registered() {
        let trunks = vec![make_trunk("t", true, Some(3600))];
        let registrar = TrunkRegistrar::new(&trunks);
        // Register with full expiry — should not need re-registration yet
        registrar.record_registered("t", 3600);
        assert!(!registrar.needs_registration("t"));
    }

    #[test]
    fn test_needs_registration_expired() {
        let trunks = vec![make_trunk("t", true, Some(3600))];
        let registrar = TrunkRegistrar::new(&trunks);
        // Simulate an already-expired registration by recording 0s expiry
        registrar.record_registered("t", 0);
        assert!(registrar.needs_registration("t"));
    }

    #[test]
    fn test_needs_registration_failed() {
        let trunks = vec![make_trunk("t", true, None)];
        let registrar = TrunkRegistrar::new(&trunks);
        registrar.record_failed("t", "401 Unauthorized".to_string());
        assert!(registrar.needs_registration("t"));
    }

    #[test]
    fn test_needs_registration_pending() {
        let trunks = vec![make_trunk("t", true, None)];
        let registrar = TrunkRegistrar::new(&trunks);
        registrar.mark_pending("t");
        assert!(!registrar.needs_registration("t"));
    }

    #[test]
    fn test_record_registered_sets_state() {
        let trunks = vec![make_trunk("t", true, None)];
        let registrar = TrunkRegistrar::new(&trunks);
        registrar.record_registered("t", 1800);
        assert_eq!(
            registrar.registration_state("t"),
            Some(RegistrationState::Registered)
        );
        assert!(registrar.last_error("t").is_none());
    }

    #[test]
    fn test_record_failed_sets_state_and_error() {
        let trunks = vec![make_trunk("t", true, None)];
        let registrar = TrunkRegistrar::new(&trunks);
        registrar.record_failed("t", "403 Forbidden".to_string());
        assert_eq!(
            registrar.registration_state("t"),
            Some(RegistrationState::Failed)
        );
        assert_eq!(registrar.last_error("t").as_deref(), Some("403 Forbidden"));
    }

    #[test]
    fn test_mark_unregistered_clears() {
        let trunks = vec![make_trunk("t", true, None)];
        let registrar = TrunkRegistrar::new(&trunks);
        registrar.record_registered("t", 3600);
        registrar.mark_unregistered("t");
        assert_eq!(
            registrar.registration_state("t"),
            Some(RegistrationState::Unregistered)
        );
        assert!(registrar.last_error("t").is_none());
    }

    #[test]
    fn test_registrar_uri_construction_udp() {
        let trunk = TrunkConfig {
            name: "t".into(),
            register: true,
            hosts: vec!["sip.carrier.com".into()],
            transport: TrunkTransport {
                protocol: TransportType::Udp,
                port: None,
                verify_tls: false,
            },
            ..Default::default()
        };
        let uri = build_registrar_uri(&trunk).unwrap();
        assert_eq!(uri, "sip:sip.carrier.com");
    }

    #[test]
    fn test_registrar_uri_construction_tls() {
        let trunk = TrunkConfig {
            name: "t".into(),
            register: true,
            hosts: vec!["sip.carrier.com".into()],
            transport: TrunkTransport {
                protocol: TransportType::Tls,
                port: None,
                verify_tls: true,
            },
            ..Default::default()
        };
        let uri = build_registrar_uri(&trunk).unwrap();
        assert_eq!(uri, "sips:sip.carrier.com");
    }

    #[test]
    fn test_registrar_uri_construction_custom_port() {
        let trunk = TrunkConfig {
            name: "t".into(),
            register: true,
            hosts: vec!["sip.carrier.com".into()],
            transport: TrunkTransport {
                protocol: TransportType::Udp,
                port: Some(5080),
                verify_tls: false,
            },
            ..Default::default()
        };
        let uri = build_registrar_uri(&trunk).unwrap();
        assert_eq!(uri, "sip:sip.carrier.com:5080");
    }

    #[test]
    fn test_registrar_uri_construction_default_port_not_included() {
        let trunk = TrunkConfig {
            name: "t".into(),
            register: true,
            hosts: vec!["sip.carrier.com".into()],
            transport: TrunkTransport {
                protocol: TransportType::Tls,
                port: Some(5061), // default for TLS
                verify_tls: true,
            },
            ..Default::default()
        };
        let uri = build_registrar_uri(&trunk).unwrap();
        assert_eq!(uri, "sips:sip.carrier.com");
    }

    #[test]
    fn test_registrar_uri_no_hosts() {
        let trunk = TrunkConfig {
            name: "t".into(),
            register: true,
            hosts: vec![],
            ..Default::default()
        };
        assert!(build_registrar_uri(&trunk).is_none());
    }

    #[test]
    fn test_has_registerable_trunks() {
        let trunks_with_reg = vec![make_trunk("t", true, None)];
        assert!(TrunkRegistrar::new(&trunks_with_reg).has_registerable_trunks());

        let trunks_no_reg = vec![make_trunk("t", false, None)];
        assert!(!TrunkRegistrar::new(&trunks_no_reg).has_registerable_trunks());

        assert!(!TrunkRegistrar::new(&[]).has_registerable_trunks());
    }

    #[test]
    fn test_configured_expiry_defaults() {
        let trunks = vec![
            make_trunk("with-expiry", true, Some(1800)),
            make_trunk("default-expiry", true, None),
        ];
        let registrar = TrunkRegistrar::new(&trunks);
        assert_eq!(registrar.configured_expiry("with-expiry"), Some(1800));
        assert_eq!(registrar.configured_expiry("default-expiry"), Some(3600));
    }

    #[test]
    fn test_unknown_trunk_returns_none() {
        let registrar = TrunkRegistrar::new(&[]);
        assert!(registrar.registration_state("nonexistent").is_none());
        assert!(!registrar.needs_registration("nonexistent"));
        assert!(registrar.registrar_uri("nonexistent").is_none());
        assert!(registrar.configured_expiry("nonexistent").is_none());
        assert!(registrar.last_error("nonexistent").is_none());
    }
}
