use anyhow::{Context, Result};
use forge_api::server::ApiServerConfig;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tracing::info;

/// WebRTC gateway configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub service: ServiceSection,

    #[serde(default)]
    pub http: HttpSection,

    #[serde(default)]
    pub media: MediaSection,

    #[serde(default)]
    pub sip: SipSection,

    #[serde(default)]
    pub webrtc: WebRtcSection,

    #[serde(default)]
    pub tls: TlsSection,

    #[serde(default)]
    pub auth: AuthSection,

    #[serde(default)]
    pub logging: LoggingSection,

    #[serde(default)]
    pub storage: StorageSection,

    #[serde(default)]
    pub cluster: ClusterSection,

    #[serde(default)]
    pub conference: ConferenceSection,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,

    #[serde(default = "default_node_id")]
    pub node_id: String,

    #[serde(default = "default_shutdown_timeout_secs")]
    pub shutdown_timeout_secs: u64,
}

fn default_service_type() -> String {
    "webrtc-gateway".to_string()
}

fn default_node_id() -> String {
    format!("webrtc-gw-{}", uuid::Uuid::new_v4())
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
pub struct HttpSection {
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,

    #[serde(default)]
    pub enable_cors: bool,

    #[serde(default)]
    pub allowed_origins: Vec<String>,

    #[serde(default = "default_rate_limit_requests")]
    pub rate_limit_requests: usize,

    #[serde(default = "default_rate_limit_window_secs")]
    pub rate_limit_window_secs: u64,

    /// Serve the built-in join page at `/join` (no credential needed to
    /// load it; joining still needs `public_join` or an operator token).
    #[serde(default)]
    pub serve_join_page: bool,

    /// Let `POST /v1/rooms/{room}/join` be called without an operator
    /// token. The join token inside the request is what admits the caller
    /// (the conference server verifies it); the rate limit bounds abuse.
    #[serde(default)]
    pub public_join: bool,
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
            serve_join_page: false,
            public_join: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaSection {
    #[serde(default = "default_rtp_port_start")]
    pub rtp_port_start: u16,

    #[serde(default = "default_rtp_port_end")]
    pub rtp_port_end: u16,

    /// Public IP for RTP (required behind NAT)
    pub public_ip: Option<String>,
}

fn default_rtp_port_start() -> u16 {
    10000
}

fn default_rtp_port_end() -> u16 {
    20000
}

impl Default for MediaSection {
    fn default() -> Self {
        Self {
            rtp_port_start: default_rtp_port_start(),
            rtp_port_end: default_rtp_port_end(),
            public_ip: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SipSection {
    #[serde(default = "default_sip_udp_bind")]
    pub udp_bind: String,

    /// Local SIP URI (e.g., "sip:webrtc-gw@example.com")
    pub local_uri: Option<String>,

    /// Local IP override for NAT
    pub local_ip: Option<String>,

    /// Outbound SIP proxy
    pub outbound_proxy: Option<String>,
}

fn default_sip_udp_bind() -> String {
    "0.0.0.0:5060".to_string()
}

impl Default for SipSection {
    fn default() -> Self {
        Self {
            udp_bind: default_sip_udp_bind(),
            local_uri: None,
            local_ip: None,
            outbound_proxy: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebRtcSection {
    /// STUN servers (`stun:host:port`) the gateway's own ICE agent asks
    /// for a server-reflexive candidate — what a browser on the far side
    /// of a NAT reaches when the gateway has no public interface.
    #[serde(default = "default_stun_servers")]
    pub stun_servers: Vec<String>,

    /// TURN servers for relay candidates, the fallback when neither side
    /// can be reached directly.
    #[serde(default)]
    pub turn_servers: Vec<TurnServerSection>,

    /// ICE servers handed to browsers with the offer (`RTCPeerConnection`
    /// `iceServers`): STUN for their reflexive address, TURN with a
    /// credential for the ones no path reaches. Empty by default.
    #[serde(default)]
    pub browser_ice_servers: Vec<BrowserIceServerSection>,

    /// How long to wait for candidate gathering (the STUN round trip)
    /// before the offer is built, so server-reflexive candidates ride in
    /// the offer rather than being trickled — the browser never fetches
    /// the gateway's candidates after the offer.
    #[serde(default = "default_gather_timeout_ms")]
    pub gather_timeout_ms: u64,

    /// How long a browser has to complete ICE and DTLS after the offer
    /// before the session is failed.
    #[serde(default = "default_ice_timeout_secs")]
    pub ice_timeout_secs: u64,

    /// How long a connected browser may send nothing (no RTP, no RTCP)
    /// before its session is ended as gone.
    #[serde(default = "default_inactivity_timeout_secs")]
    pub inactivity_timeout_secs: u64,
}

/// One ICE server for browsers.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BrowserIceServerSection {
    /// `stun:host:port`, `turn:host:port?transport=udp`, `turns:…`.
    pub urls: Vec<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub credential: Option<String>,
}

/// One TURN server with its long-term credential.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TurnServerSection {
    /// `turn:host:port` (UDP).
    pub uri: String,
    pub username: String,
    pub password: String,
}

fn default_stun_servers() -> Vec<String> {
    vec!["stun:stun.l.google.com:19302".to_string()]
}

fn default_gather_timeout_ms() -> u64 {
    1500
}

fn default_ice_timeout_secs() -> u64 {
    30
}

fn default_inactivity_timeout_secs() -> u64 {
    30
}

impl Default for WebRtcSection {
    fn default() -> Self {
        Self {
            stun_servers: default_stun_servers(),
            turn_servers: Vec::new(),
            browser_ice_servers: Vec::new(),
            gather_timeout_ms: default_gather_timeout_ms(),
            ice_timeout_secs: default_ice_timeout_secs(),
            inactivity_timeout_secs: default_inactivity_timeout_secs(),
        }
    }
}

/// How room joins reach the conference server.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConferenceSection {
    /// The conference server a room join INVITEs when cluster placement
    /// is off or finds no node: a host, `host:port`, or a `sip:` URI whose
    /// host part is used. Without it, and without a cluster, room joins
    /// fail with a clear error.
    #[serde(default)]
    pub server_uri: Option<String>,

    /// Participants a conference node is assumed to take before placement
    /// prefers another; the same figure the call manager uses.
    #[serde(default = "default_node_capacity")]
    pub node_capacity: usize,

    /// The tenant whose rooms this gateway joins, as the conference
    /// registry keys them. Lets placement send a join to the node already
    /// hosting the room; without it placement goes by load and region
    /// alone.
    #[serde(default)]
    pub tenant_id: Option<String>,

    /// Offer SDES SRTP (`a=crypto`) on the SIP leg. A conference server
    /// with `enable_srtp` answers with its key; one without ignores the
    /// offer and the leg runs plain RTP.
    #[serde(default = "default_true")]
    pub srtp: bool,

    /// How long to wait for the conference server's final answer to the
    /// INVITE before giving up.
    #[serde(default = "default_invite_timeout_secs")]
    pub invite_timeout_secs: u64,
}

fn default_node_capacity() -> usize {
    100
}

fn default_true() -> bool {
    true
}

fn default_invite_timeout_secs() -> u64 {
    30
}

impl Default for ConferenceSection {
    fn default() -> Self {
        Self {
            server_uri: None,
            node_capacity: default_node_capacity(),
            tenant_id: None,
            srtp: default_true(),
            invite_timeout_secs: default_invite_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TlsSection {
    #[serde(default)]
    pub enabled: bool,

    pub cert: Option<PathBuf>,

    pub key: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthSection {
    #[serde(default)]
    pub disable_auth: bool,

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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct StorageSection {
    #[serde(default)]
    pub redis_url: Option<String>,
}

/// Cluster configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
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

/// Convert gateway config to forge-api's ApiServerConfig.
#[allow(dead_code)]
pub fn to_api_server_config(config: &GatewayConfig) -> ApiServerConfig {
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
        https_bind: None,
        tls_cert: config.tls.cert.clone(),
        tls_key: config.tls.key.clone(),
        ..Default::default()
    }
}

/// Validate configuration for logical consistency.
pub fn validate_config(config: &GatewayConfig) -> Result<()> {
    if config.media.rtp_port_start >= config.media.rtp_port_end {
        anyhow::bail!(
            "media.rtp_port_start ({}) must be less than rtp_port_end ({})",
            config.media.rtp_port_start,
            config.media.rtp_port_end
        );
    }

    if config.tls.enabled && (config.tls.cert.is_none() || config.tls.key.is_none()) {
        anyhow::bail!("tls.enabled is true but tls.cert and/or tls.key are not configured");
    }

    if config.media.rtp_port_end - config.media.rtp_port_start < 400 {
        anyhow::bail!(
            "media.rtp_port_start..rtp_port_end must span at least 400 ports (the relay takes \
             the lower half, forge-api media sessions the upper half)"
        );
    }

    if config.conference.invite_timeout_secs == 0 {
        anyhow::bail!("conference.invite_timeout_secs must be greater than 0");
    }

    if config.webrtc.ice_timeout_secs == 0 {
        anyhow::bail!("webrtc.ice_timeout_secs must be greater than 0");
    }

    if config.webrtc.inactivity_timeout_secs == 0 {
        anyhow::bail!("webrtc.inactivity_timeout_secs must be greater than 0");
    }

    for turn in &config.webrtc.turn_servers {
        if turn.uri.is_empty() {
            anyhow::bail!("webrtc.turn_servers entries need a uri");
        }
    }

    if !config.auth.disable_auth && config.auth.tokens.is_empty() {
        anyhow::bail!(
            "auth.disable_auth is false but no auth.tokens are configured — \
             either add tokens or set disable_auth = true"
        );
    }

    // Validate SIP bind address
    config
        .sip
        .udp_bind
        .parse::<SocketAddr>()
        .context("Invalid sip.udp_bind address")?;

    Ok(())
}

/// Load configuration from file or use defaults.
pub fn load_config(config_path: Option<&Path>) -> Result<GatewayConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-webrtc-gateway.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            GatewayConfig::default()
        }
    };

    info!(
        node_id = %config.service.node_id,
        http_bind = %config.http.bind_address,
        sip_bind = %config.sip.udp_bind,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        "Configuration loaded"
    );

    Ok(config)
}

fn load_config_from_file(path: &Path) -> Result<GatewayConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: GatewayConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GatewayConfig::default();
        assert_eq!(config.service.service_type, "webrtc-gateway");
        assert_eq!(config.http.bind_address.port(), 8080);
        assert_eq!(config.media.rtp_port_start, 10000);
        assert_eq!(config.media.rtp_port_end, 20000);
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert!(!config.auth.disable_auth);
        assert_eq!(config.auth.tokens.len(), 1);
        assert_eq!(config.webrtc.stun_servers.len(), 1);
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "webrtc-gateway"
node_id = "gw-test-01"

[http]
bind_address = "0.0.0.0:9090"

[media]
rtp_port_start = 15000
rtp_port_end = 25000

[sip]
udp_bind = "0.0.0.0:5080"
local_uri = "sip:gw@example.com"

[webrtc]
stun_servers = ["stun:stun1.example.com:3478", "stun:stun2.example.com:3478"]

[auth]
disable_auth = true

[logging]
level = "debug"
format = "compact"
"#;
        let config: GatewayConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "gw-test-01");
        assert_eq!(config.http.bind_address.port(), 9090);
        assert_eq!(config.media.rtp_port_start, 15000);
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5080");
        assert_eq!(config.sip.local_uri.as_deref(), Some("sip:gw@example.com"));
        assert_eq!(config.webrtc.stun_servers.len(), 2);
        assert!(config.auth.disable_auth);
    }

    #[test]
    fn test_validate_config_ok() {
        let mut config = GatewayConfig::default();
        config.auth.disable_auth = true;
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_config_bad_port_range() {
        let mut config = GatewayConfig::default();
        config.auth.disable_auth = true;
        config.media.rtp_port_start = 20000;
        config.media.rtp_port_end = 10000;
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_config_tls_incomplete() {
        let mut config = GatewayConfig::default();
        config.auth.disable_auth = true;
        config.tls.enabled = true;
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_config_bad_sip_bind() {
        let mut config = GatewayConfig::default();
        config.auth.disable_auth = true;
        config.sip.udp_bind = "not-a-valid-address".to_string();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_to_api_server_config() {
        let mut config = GatewayConfig::default();
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
