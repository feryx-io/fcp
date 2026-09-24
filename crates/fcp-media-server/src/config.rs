use anyhow::{Context, Result};
use forge_api::server::ApiServerConfig;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tracing::info;

/// Media server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MediaServerConfig {
    #[serde(default)]
    pub service: ServiceSection,

    #[serde(default)]
    pub http: HttpSection,

    #[serde(default)]
    pub media: MediaSection,

    #[serde(default)]
    pub recording: RecordingSection,

    #[serde(default)]
    pub tls: TlsSection,

    #[serde(default)]
    pub auth: AuthSection,

    #[serde(default)]
    pub ai: AiSection,

    #[serde(default)]
    pub logging: LoggingSection,

    #[serde(default)]
    pub storage: StorageConfigSection,

    #[serde(default)]
    pub cluster: ClusterSection,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,

    /// Maximum time (seconds) to wait for graceful shutdown.
    /// Default: 30.
    #[serde(default = "default_shutdown_timeout_secs")]
    pub shutdown_timeout_secs: u64,
}

fn default_service_type() -> String {
    "media".to_string()
}

fn default_node_id() -> String {
    format!("media-{}", uuid::Uuid::new_v4())
}

fn default_shutdown_timeout_secs() -> u64 {
    30
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
            shutdown_timeout_secs: default_shutdown_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,

    /// Enable CORS for cross-origin API access
    #[serde(default)]
    pub enable_cors: bool,

    /// Allowed CORS origins (requires enable_cors = true)
    #[serde(default)]
    pub allowed_origins: Vec<String>,

    /// Rate limit: requests per window
    #[serde(default = "default_rate_limit_requests")]
    pub rate_limit_requests: usize,

    /// Rate limit: window duration in seconds
    #[serde(default = "default_rate_limit_window_secs")]
    pub rate_limit_window_secs: u64,
}

fn default_http_bind() -> SocketAddr {
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 8080)
}

fn default_rate_limit_requests() -> usize {
    120
}

fn default_rate_limit_window_secs() -> u64 {
    60
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
            enable_cors: false,
            allowed_origins: Vec::new(),
            rate_limit_requests: default_rate_limit_requests(),
            rate_limit_window_secs: default_rate_limit_window_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSection {
    #[serde(default = "default_rtp_port_start")]
    pub rtp_port_start: u16,

    #[serde(default = "default_rtp_port_end")]
    pub rtp_port_end: u16,

    /// Public IP for RTP (required behind NAT)
    pub public_ip: Option<String>,

    /// Enable AF_XDP for high-performance RTP
    #[serde(default)]
    pub xdp_enabled: bool,

    /// Network interface for XDP
    #[serde(default = "default_xdp_interface")]
    pub xdp_interface: String,

    /// XDP mode: "generic" or "native"
    #[serde(default = "default_xdp_mode")]
    pub xdp_mode: String,

    /// Maximum frames in mixer buffer
    #[serde(default = "default_mixer_max_buffer_frames")]
    pub mixer_max_buffer_frames: usize,
}

fn default_rtp_port_start() -> u16 {
    10000
}

fn default_rtp_port_end() -> u16 {
    20000
}

fn default_xdp_interface() -> String {
    "lo".to_string()
}

fn default_xdp_mode() -> String {
    "generic".to_string()
}

fn default_mixer_max_buffer_frames() -> usize {
    forge_core::config::default_mixer_max_buffer_frames()
}

impl Default for MediaSection {
    fn default() -> Self {
        Self {
            rtp_port_start: default_rtp_port_start(),
            rtp_port_end: default_rtp_port_end(),
            public_ip: None,
            xdp_enabled: false,
            xdp_interface: default_xdp_interface(),
            xdp_mode: default_xdp_mode(),
            mixer_max_buffer_frames: default_mixer_max_buffer_frames(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingSection {
    /// Base directory for session recordings
    #[serde(default = "default_recording_base_dir")]
    pub base_dir: PathBuf,

    /// Root jail for recording path traversal prevention
    #[serde(default = "default_recording_root_jail")]
    pub root_jail: PathBuf,

    /// Directory for pre-recorded prompt/announcement files
    #[serde(default = "default_prompts_dir")]
    pub prompts_dir: PathBuf,
}

fn default_recording_base_dir() -> PathBuf {
    PathBuf::from("/var/lib/fcp/media/recordings")
}

fn default_recording_root_jail() -> PathBuf {
    PathBuf::from("/var/lib/fcp/media")
}

fn default_prompts_dir() -> PathBuf {
    PathBuf::from("/var/lib/fcp/media/prompts")
}

impl Default for RecordingSection {
    fn default() -> Self {
        Self {
            base_dir: default_recording_base_dir(),
            root_jail: default_recording_root_jail(),
            prompts_dir: default_prompts_dir(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TlsSection {
    /// Enable HTTPS listener
    #[serde(default)]
    pub enabled: bool,

    /// Where HTTPS listens, beside HTTP on `[http] bind_address`. Unset,
    /// it is the HTTP bind, which the two listeners cannot share.
    #[serde(default)]
    pub bind: Option<SocketAddr>,

    /// Path to TLS certificate (PEM)
    pub cert: Option<PathBuf>,

    /// Path to TLS private key (PEM)
    pub key: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthSection {
    /// Disable API authentication (NOT recommended for production)
    #[serde(default)]
    pub disable_auth: bool,

    /// Bearer tokens for API authentication
    #[serde(default = "default_auth_tokens")]
    pub tokens: Vec<String>,
}

fn default_auth_tokens() -> Vec<String> {
    vec![uuid::Uuid::new_v4().to_string()]
}

impl Default for AuthSection {
    fn default() -> Self {
        Self {
            disable_auth: false,
            tokens: default_auth_tokens(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AiSection {
    /// Allowed AI API endpoint URLs
    #[serde(default = "default_ai_allowed_endpoints")]
    pub allowed_endpoints: Vec<String>,
}

fn default_ai_allowed_endpoints() -> Vec<String> {
    forge_core::config::default_ai_allowed_endpoints()
}

impl Default for AiSection {
    fn default() -> Self {
        Self {
            allowed_endpoints: default_ai_allowed_endpoints(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingSection {
    #[serde(default = "default_log_level")]
    pub level: String,

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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfigSection {
    /// PostgreSQL connection URL for config store.
    #[serde(default)]
    pub database_url: Option<String>,

    /// Streaming read replica (hot standby) that configuration reads are
    /// served from while it is healthy; writes always go to `database_url`.
    /// See `docs/MULTI_REGION.md`.
    #[serde(default)]
    pub read_replica_url: Option<String>,

    /// Replication lag (seconds) beyond which reads return to the primary;
    /// also the read-after-write window. Default: 5.
    #[serde(default = "default_max_replica_lag_secs")]
    pub max_replica_lag_secs: u64,

    /// Maximum database connections (default: 10).
    #[serde(default = "default_storage_max_connections")]
    pub max_connections: u32,

    /// Redis connection URL for cluster membership.
    #[serde(default)]
    pub redis_url: Option<String>,
}

fn default_max_replica_lag_secs() -> u64 {
    5
}

fn default_storage_max_connections() -> u32 {
    10
}

impl Default for StorageConfigSection {
    fn default() -> Self {
        Self {
            database_url: None,
            read_replica_url: None,
            max_replica_lag_secs: default_max_replica_lag_secs(),
            max_connections: default_storage_max_connections(),
            redis_url: None,
        }
    }
}

/// Cluster configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterSection {
    #[serde(default)]
    pub enabled: bool,

    /// Deployment region this node advertises in cluster membership, e.g.
    /// `us-east`; falls back to the `FCP_REGION` environment variable.
    /// Placement prefers nodes in the caller's region. See
    /// `docs/MULTI_REGION.md`.
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,
    #[serde(default = "default_heartbeat_ttl")]
    pub heartbeat_ttl_secs: u64,
}

fn default_heartbeat_interval() -> u64 {
    5
}
fn default_heartbeat_ttl() -> u64 {
    15
}

impl Default for ClusterSection {
    fn default() -> Self {
        Self {
            enabled: false,
            region: None,
            heartbeat_interval_secs: default_heartbeat_interval(),
            heartbeat_ttl_secs: default_heartbeat_ttl(),
        }
    }
}

/// Convert FCP media server config to forge-api's ApiServerConfig.
pub fn to_api_server_config(config: &MediaServerConfig) -> ApiServerConfig {
    ApiServerConfig {
        bind_addr: config.http.bind_address,
        enable_cors: config.http.enable_cors,
        allowed_origins: config.http.allowed_origins.clone(),
        port_range_min: config.media.rtp_port_start,
        port_range_max: config.media.rtp_port_end,
        disable_auth: config.auth.disable_auth,
        auth_tokens: config.auth.tokens.clone(),
        rate_limit_requests_per_window: config.http.rate_limit_requests,
        rate_limit_window_secs: config.http.rate_limit_window_secs,
        trusted_proxies: Vec::new(),
        enable_https: config.tls.enabled,
        https_bind: config.tls.bind,
        tls_cert: config.tls.cert.clone(),
        tls_key: config.tls.key.clone(),
        recording_base_dir: config.recording.base_dir.clone(),
        recording_root_jail: config.recording.root_jail.clone(),
        prompts_base_dir: config.recording.prompts_dir.clone(),
        siprec_enabled: false,
        siprec_output_dir: PathBuf::from("/var/lib/fcp/media/siprec"),
        siprec_format: forge_core::AudioFormat::pcm_mono(),
        xdp_enabled: config.media.xdp_enabled,
        xdp_interface: config.media.xdp_interface.clone(),
        xdp_mode: config.media.xdp_mode.clone(),
        ai_allowed_endpoints: config.ai.allowed_endpoints.clone(),
        mixer_max_buffer_frames: config.media.mixer_max_buffer_frames,
        ..Default::default()
    }
}

/// Validate configuration for logical consistency.
pub fn validate_config(config: &MediaServerConfig) -> Result<()> {
    // RTP port range
    if config.media.rtp_port_start >= config.media.rtp_port_end {
        anyhow::bail!(
            "media.rtp_port_start ({}) must be less than rtp_port_end ({})",
            config.media.rtp_port_start,
            config.media.rtp_port_end
        );
    }

    // TLS consistency
    if config.tls.enabled && (config.tls.cert.is_none() || config.tls.key.is_none()) {
        anyhow::bail!("tls.enabled is true but tls.cert and/or tls.key are not configured");
    }
    if config.tls.enabled
        && config
            .tls
            .bind
            .is_none_or(|b| b == config.http.bind_address)
    {
        anyhow::bail!(
            "tls.enabled is true but tls.bind is not set (HTTPS listens beside HTTP, not on {})",
            config.http.bind_address
        );
    }

    // Auth consistency: not disabled and no tokens
    if !config.auth.disable_auth && config.auth.tokens.is_empty() {
        anyhow::bail!(
            "auth.disable_auth is false but no auth.tokens are configured — \
             either add tokens or set disable_auth = true"
        );
    }

    Ok(())
}

/// Load configuration from file or use defaults.
pub fn load_config(config_path: Option<&Path>) -> Result<MediaServerConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-media-server.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            MediaServerConfig::default()
        }
    };

    info!(
        node_id = %config.service.node_id,
        http_bind = %config.http.bind_address,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        "Configuration loaded"
    );

    Ok(config)
}

fn load_config_from_file(path: &Path) -> Result<MediaServerConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: MediaServerConfig = fcp_core::config::parse_toml(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = MediaServerConfig::default();
        assert_eq!(config.service.service_type, "media");
        assert_eq!(config.http.bind_address.port(), 8080);
        assert_eq!(config.media.rtp_port_start, 10000);
        assert_eq!(config.media.rtp_port_end, 20000);
        assert!(!config.auth.disable_auth);
        assert_eq!(config.auth.tokens.len(), 1);
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "media"
node_id = "media-test-01"

[http]
bind_address = "0.0.0.0:9090"

[media]
rtp_port_start = 15000
rtp_port_end = 25000

[auth]
disable_auth = true

[recording]
base_dir = "/tmp/recordings"
root_jail = "/tmp"
prompts_dir = "/tmp/prompts"

[logging]
level = "debug"
format = "compact"
"#;
        let config: MediaServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "media-test-01");
        assert_eq!(config.http.bind_address.port(), 9090);
        assert_eq!(config.media.rtp_port_start, 15000);
        assert!(config.auth.disable_auth);
    }

    #[test]
    fn test_validate_config_ok() {
        let mut config = MediaServerConfig::default();
        config.auth.disable_auth = true;
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_config_bad_port_range() {
        let mut config = MediaServerConfig::default();
        config.auth.disable_auth = true;
        config.media.rtp_port_start = 20000;
        config.media.rtp_port_end = 10000;
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_config_tls_incomplete() {
        let mut config = MediaServerConfig::default();
        config.auth.disable_auth = true;
        config.tls.enabled = true;
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_to_api_server_config() {
        let mut config = MediaServerConfig::default();
        config.auth.disable_auth = true;
        config.auth.tokens = vec!["test-token".to_string()];
        let api_config = to_api_server_config(&config);
        assert_eq!(api_config.bind_addr, config.http.bind_address);
        assert_eq!(api_config.port_range_min, config.media.rtp_port_start);
        assert_eq!(api_config.port_range_max, config.media.rtp_port_end);
        assert!(api_config.disable_auth);
        assert_eq!(api_config.auth_tokens, vec!["test-token"]);
    }
}
