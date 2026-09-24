//! Configuration loading and management for fcp-registrar.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Registrar server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrarConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceSection,

    /// SIP transport configuration
    #[serde(default)]
    pub sip: SipSection,

    /// Registrar behavior configuration
    #[serde(default)]
    pub registrar: RegistrarSection,

    /// What this node accepts, and what it refuses
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §5). Every limit is optional;
    /// unset is unlimited, as it was before the section existed.
    #[serde(default)]
    pub admission: fcp_sbc_core::AdmissionSection,

    /// Storage configuration
    #[serde(default)]
    pub storage: StorageSection,

    /// HTTP health endpoint configuration
    #[serde(default)]
    pub http: HttpSection,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingSection,
}

/// Service identification.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,

    #[serde(default = "default_tenant_id")]
    pub default_tenant_id: fcp_core::TenantId,

    /// Seconds a drain waits for the devices registered here to register
    /// somewhere else before the node stops
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6). A draining registrar
    /// grants short registrations, so they come back within the minute.
    /// Zero — the default — is what this binary always did: it stops where
    /// it stands.
    #[serde(default)]
    pub drain_wait_secs: u64,
}

fn default_service_type() -> String {
    "registrar".to_string()
}

fn default_node_id() -> String {
    format!("registrar-{}", uuid::Uuid::new_v4())
}

fn default_tenant_id() -> fcp_core::TenantId {
    fcp_core::TenantId::nil()
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
            default_tenant_id: default_tenant_id(),
            drain_wait_secs: 0,
        }
    }
}

/// SIP transport configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SipSection {
    /// UDP bind address
    #[serde(default = "default_sip_bind")]
    pub udp_bind: String,

    /// TCP bind address
    #[serde(default = "default_sip_bind")]
    pub tcp_bind: String,

    /// TLS/SIPS bind address.
    #[serde(default = "default_sips_bind")]
    pub sips_bind: String,

    /// Path to TLS certificate file (PEM format)
    pub tls_cert_path: Option<String>,

    /// Path to TLS private key file (PEM format)
    pub tls_key_path: Option<String>,

    /// Optional SIP-over-WebSocket bind address.
    #[serde(default)]
    pub ws_bind: Option<String>,

    /// Optional secure SIP-over-WebSocket bind address.
    #[serde(default)]
    pub wss_bind: Option<String>,
}

fn default_sip_bind() -> String {
    "0.0.0.0:5060".to_string()
}

fn default_sips_bind() -> String {
    "0.0.0.0:5061".to_string()
}

impl Default for SipSection {
    fn default() -> Self {
        Self {
            udp_bind: default_sip_bind(),
            tcp_bind: default_sip_bind(),
            sips_bind: default_sips_bind(),
            tls_cert_path: None,
            tls_key_path: None,
            ws_bind: None,
            wss_bind: None,
        }
    }
}

/// Registrar behavior configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrarSection {
    /// Enable SIP Digest authentication
    #[serde(default = "default_true")]
    pub auth_enabled: bool,

    /// Authentication realm
    #[serde(default = "default_realm")]
    pub realm: String,

    /// Default registration expiry in seconds
    #[serde(default = "default_expires")]
    pub default_expires: u32,

    /// Minimum registration expiry in seconds
    #[serde(default = "default_min_expires")]
    pub min_expires: u32,

    /// Maximum registration expiry in seconds
    #[serde(default = "default_max_expires")]
    pub max_expires: u32,
}

fn default_realm() -> String {
    "fcp.local".to_string()
}

fn default_true() -> bool {
    true
}

fn default_expires() -> u32 {
    3600
}

fn default_min_expires() -> u32 {
    60
}

fn default_max_expires() -> u32 {
    86400
}

impl Default for RegistrarSection {
    fn default() -> Self {
        Self {
            auth_enabled: true,
            realm: default_realm(),
            default_expires: default_expires(),
            min_expires: default_min_expires(),
            max_expires: default_max_expires(),
        }
    }
}

/// Storage backend configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageSection {
    /// PostgreSQL connection URL
    #[serde(default = "default_database_url")]
    pub database_url: String,

    /// Streaming read replica (hot standby) that configuration reads are
    /// served from while it is healthy; writes always go to `database_url`.
    /// See `docs/MULTI_REGION.md`.
    #[serde(default)]
    pub read_replica_url: Option<String>,

    /// Replication lag (seconds) beyond which reads return to the primary;
    /// also the read-after-write window. Default: 5.
    #[serde(default = "default_max_replica_lag_secs")]
    pub max_replica_lag_secs: u64,

    /// Maximum database connections
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

fn default_max_replica_lag_secs() -> u64 {
    5
}

fn default_database_url() -> String {
    "postgresql://localhost/fcp".to_string()
}

fn default_max_connections() -> u32 {
    50
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            database_url: default_database_url(),
            read_replica_url: None,
            max_replica_lag_secs: default_max_replica_lag_secs(),
            max_connections: default_max_connections(),
        }
    }
}

/// HTTP health endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    /// Bind address for the health, status and drain endpoints. Loopback
    /// by default, like every other binary's; a non-loopback bind needs
    /// `api_token`.
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,

    /// Bearer token for the operator routes — `/status` and `/drain`.
    /// `/health` and `/ready` never need it, so probes stay simple. A
    /// public bind requires this to be set
    /// (`docs/DEPLOYMENT_GAPS.md` §1.3).
    #[serde(default)]
    pub api_token: Option<String>,
}

fn default_http_bind() -> SocketAddr {
    "127.0.0.1:8083".parse().unwrap()
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
            api_token: None,
        }
    }
}

/// Logging configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingSection {
    /// Log level
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log format (json, pretty, compact)
    #[serde(default = "default_log_format")]
    pub format: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> String {
    "json".to_string()
}

impl Default for LoggingSection {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

/// Load configuration from file or use defaults.
///
/// Priority:
/// 1. File specified via --config or FCP_REGISTRAR_CONFIG
/// 2. ./fcp-registrar.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<RegistrarConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-registrar.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            RegistrarConfig::default()
        }
    };

    // Validated by the caller after its overrides (`validate_config`),
    // so a secret from the environment counts.

    info!(
        node_id = %config.service.node_id,
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        sips_bind = %config.sip.sips_bind,
        ws_bind = ?config.sip.ws_bind,
        wss_bind = ?config.sip.wss_bind,
        auth_enabled = config.registrar.auth_enabled,
        realm = %config.registrar.realm,
        http_bind = %config.http.bind_address,
        "Configuration loaded"
    );

    Ok(config)
}

pub fn validate_config(config: &RegistrarConfig) -> Result<()> {
    fcp_sip::validate_sip_binds(
        &config.sip.udp_bind,
        &config.sip.tcp_bind,
        config.sip.ws_bind.as_deref(),
        config.sip.wss_bind.as_deref(),
        config.sip.tls_cert_path.as_deref(),
        config.sip.tls_key_path.as_deref(),
    )?;

    let public_udp = bind_is_public(&config.sip.udp_bind)
        .with_context(|| format!("Invalid SIP UDP bind address '{}'", config.sip.udp_bind))?;
    let public_tcp = bind_is_public(&config.sip.tcp_bind)
        .with_context(|| format!("Invalid SIP TCP bind address '{}'", config.sip.tcp_bind))?;
    let public_tls = bind_is_public(&config.sip.sips_bind)
        .with_context(|| format!("Invalid SIP TLS bind address '{}'", config.sip.sips_bind))?;
    let public_ws = config
        .sip
        .ws_bind
        .as_deref()
        .map(bind_is_public)
        .transpose()
        .with_context(|| format!("Invalid SIP WS bind address '{:?}'", config.sip.ws_bind))?
        .unwrap_or(false);
    let public_wss = config
        .sip
        .wss_bind
        .as_deref()
        .map(bind_is_public)
        .transpose()
        .with_context(|| format!("Invalid SIP WSS bind address '{:?}'", config.sip.wss_bind))?
        .unwrap_or(false);

    if (public_udp || public_tcp || public_tls || public_ws || public_wss)
        && !config.registrar.auth_enabled
    {
        anyhow::bail!(
            "registrar.auth_enabled must be true when binding the standalone registrar to a non-loopback SIP address (udp={}, tcp={}, tls={}, ws={:?}, wss={:?})",
            config.sip.udp_bind,
            config.sip.tcp_bind,
            config.sip.sips_bind,
            config.sip.ws_bind,
            config.sip.wss_bind
        );
    }

    validate_http_config(&config.http)?;

    Ok(())
}

/// The operator routes are behind a token wherever anyone but this host
/// can reach them: a drain is a change of service, not a probe.
fn validate_http_config(http: &HttpSection) -> Result<()> {
    let public_bind = !http.bind_address.ip().is_loopback();
    let has_token = http
        .api_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty());

    if public_bind && !has_token {
        anyhow::bail!(
            "http.api_token must be set when binding the registrar HTTP API to a non-loopback address ({})",
            http.bind_address
        );
    }

    Ok(())
}

fn bind_is_public(bind: &str) -> Result<bool> {
    let addr: std::net::SocketAddr = bind.parse()?;
    Ok(!addr.ip().is_loopback())
}

fn load_config_from_file(path: &Path) -> Result<RegistrarConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: RegistrarConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RegistrarConfig::default();
        assert_eq!(config.service.service_type, "registrar");
        assert_eq!(config.service.default_tenant_id, fcp_core::TenantId::nil());
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert_eq!(config.sip.tcp_bind, "0.0.0.0:5060");
        assert_eq!(config.sip.sips_bind, "0.0.0.0:5061");
        assert!(config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "fcp.local");
        assert_eq!(config.registrar.default_expires, 3600);
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.http.bind_address.port(), 8083);
        assert!(config.http.bind_address.ip().is_loopback());
        assert!(config.http.api_token.is_none());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "registrar"
node_id = "reg-test-01"
default_tenant_id = "00000000-0000-0000-0000-000000000123"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
sips_bind = "0.0.0.0:5061"

[registrar]
auth_enabled = true
realm = "test.local"
default_expires = 1800

[storage]
database_url = "postgresql://localhost/fcp"

[http]
bind_address = "0.0.0.0:9090"

[logging]
level = "debug"
format = "pretty"
"#;
        let config: RegistrarConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "reg-test-01");
        assert_eq!(
            config.service.default_tenant_id,
            "00000000-0000-0000-0000-000000000123".parse().unwrap()
        );
        assert!(config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "test.local");
        assert_eq!(config.registrar.default_expires, 1800);
        assert_eq!(config.http.bind_address.port(), 9090);
    }

    #[test]
    fn test_public_sip_bind_requires_auth() {
        let mut config = RegistrarConfig::default();
        config.registrar.auth_enabled = false;
        assert!(validate_config(&config).is_err());

        config.sip.udp_bind = "127.0.0.1:5060".to_string();
        config.sip.tcp_bind = "127.0.0.1:5060".to_string();
        config.sip.sips_bind = "127.0.0.1:5061".to_string();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn a_public_http_bind_requires_the_token() {
        let mut config = RegistrarConfig::default();
        assert!(validate_config(&config).is_ok(), "loopback needs no token");

        config.http.bind_address = "0.0.0.0:8083".parse().unwrap();
        let err = validate_config(&config).unwrap_err().to_string();
        assert!(err.contains("http.api_token"), "{err}");

        config.http.api_token = Some("   ".to_string());
        assert!(validate_config(&config).is_err(), "blank is unset");

        config.http.api_token = Some("operator".to_string());
        assert!(validate_config(&config).is_ok());
    }
}
