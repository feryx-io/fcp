//! Configuration loading and management for fcp-call-manager.

use anyhow::{Context, Result};
use fcp_sbc_core::AdmissionSection;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::{info, warn};

/// Call manager server configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CallManagerConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceSection,

    /// SIP transport configuration
    #[serde(default)]
    pub sip: SipSection,

    /// Registrar behavior configuration
    #[serde(default)]
    pub registrar: RegistrarSection,

    /// Storage configuration
    #[serde(default)]
    pub storage: StorageSection,

    /// The CDR pipeline's node side: where finished calls' records spool
    /// before they ship (`docs/CDR_PIPELINE.md`).
    #[serde(default)]
    pub cdr: fcp_cdr::NodeConfig,

    /// HTTP health/API endpoint configuration
    #[serde(default)]
    pub http: HttpSection,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingSection,

    /// Call handling configuration
    #[serde(default)]
    pub calls: CallsSection,

    /// What this node will carry, and what it refuses
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §4). Every limit is unset —
    /// unlimited — until a deployment writes one down.
    #[serde(default)]
    pub admission: AdmissionSection,

    /// Media/RTP configuration
    #[serde(default)]
    pub media: MediaSection,

    /// Speech recognition (`docs/DEVELOPER_SURFACE.md` phase 6): voicemail
    /// transcribed for mailboxes that ask. Unset, nothing is recognised.
    #[serde(default)]
    pub stt: Option<fcp_media::SttConfig>,

    /// Routing configuration
    #[serde(default)]
    pub routing: RoutingSection,

    /// Events configuration
    #[serde(default)]
    pub events: EventsSection,

    /// B2BUA behavior configuration (extension threshold, topology hiding, etc.)
    #[serde(default)]
    pub b2bua: fcp_b2bua_core::B2buaConfig,

    /// SIP trunk configurations (session-manager mode).
    /// When non-empty and registrar.enabled=false, operates as session manager.
    #[serde(default)]
    pub trunks: Vec<fcp_sbc_core::TrunkConfig>,

    /// PBX features (hunt groups, parking, pickup, intercom, CMC/FAC).
    #[serde(default)]
    pub pbx: fcp_pbx::PbxConfig,

    /// ACD/Queue configuration.
    #[serde(default)]
    pub queue: fcp_queue::QueueSystemConfig,

    /// Presence service configuration.
    #[serde(default)]
    pub presence: fcp_presence::PresenceConfig,

    /// Cluster configuration.
    #[serde(default)]
    pub cluster: ClusterSection,

    /// What reaches this node's SIP port, and what a flood of it costs
    /// (`docs/EDGE_SECURITY.md` §2.4). The call manager is the edge of
    /// every single-binary deployment, so this is where it is defended.
    #[serde(default)]
    pub security: fcp_sip::edge::EdgeConfig,

    /// Emergency calling (`docs/E911.md`, `docs/E911_BLOCK_2.md`).
    #[serde(default)]
    pub e911: E911Section,

    /// How the node keeps the tenants' trunks current
    /// (`docs/TENANT_TRUNKS.md` §4).
    #[serde(default)]
    pub trunk_routing: TrunkRoutingSection,

    /// Header manipulation rules, by name. A trunk's `header_rules` names
    /// the ones applied to the calls it carries (`docs/TENANT_TRUNKS.md`
    /// §6); the rules themselves are the node's, as they are on the SBC.
    #[serde(default)]
    pub header_manipulation: fcp_sbc_core::hmr::HeaderManipulationConfig,

    /// STIR/SHAKEN (RFC 8224). The signing key and the certificate it
    /// names are the node's; which calls are signed, and at what
    /// attestation, is each trunk's `stir_shaken` to say
    /// (`docs/TENANT_TRUNKS.md` §6).
    #[serde(default)]
    pub stir_shaken: fcp_sbc_core::stir_shaken::StirShakenConfig,

    /// Push (`docs/PUSH.md`): the publisher's keys for waking a phone app
    /// that sleeps, and how long its binding lives. Unset, a REGISTER with
    /// a push address is answered 555 and the app registers without one.
    /// Read from the file only, never written out (the stored sections
    /// leave it alone): it holds the publisher's keys.
    #[serde(default, skip_serializing)]
    pub push: Option<fcp_core::push::PushConfig>,
}

/// What a node does about trunks it did not read from its own config.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrunkRoutingSection {
    /// How often the node re-reads the tenants' trunks when nothing has
    /// told it they changed. `system.config_changed` from the API is the
    /// fast path; this is the backstop, for a node that missed it.
    #[serde(default = "default_trunk_refresh_secs")]
    pub refresh_secs: u64,
}

fn default_trunk_refresh_secs() -> u64 {
    fcp_sbc_core::trunk_refresh::REFRESH_INTERVAL.as_secs()
}

impl Default for TrunkRoutingSection {
    fn default() -> Self {
        Self {
            refresh_secs: default_trunk_refresh_secs(),
        }
    }
}

/// Emergency calling.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct E911Section {
    /// The edges — a WebRTC gateway, an SBC — whose `X-FCP-Caller-Tenant`
    /// and `X-FCP-Caller-User` headers name the tenant user a call is
    /// placed for, as addresses or CIDR ranges. From any other peer those
    /// headers are ignored. Empty trusts no one.
    #[serde(default)]
    pub trusted_edges: Vec<String>,

    /// Tell the next hop where each caller is (`X-FCP-Client-Address`).
    /// A node that stands in front of phones — an SBC — sets this; a call
    /// manager behind one does not.
    #[serde(default)]
    pub vouch_client_address: bool,
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

    /// How long a drain waits for the calls this node is carrying before
    /// it ends what is left (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    /// Zero — the default — is the behaviour every node had before there
    /// was a drain: SIGTERM stops the node where it stands.
    #[serde(default)]
    pub drain_wait_secs: u64,
}

fn default_service_type() -> String {
    "call-manager".to_string()
}

fn default_node_id() -> String {
    format!("cm-{}", uuid::Uuid::new_v4())
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

    /// TLS/SIPS bind address (default: 0.0.0.0:5061)
    /// Only active if tls_cert_path and tls_key_path are set.
    #[serde(default = "default_sips_bind")]
    pub sips_bind: String,

    /// Path to TLS certificate file (PEM format)
    pub tls_cert_path: Option<String>,

    /// Path to TLS private key file (PEM format)
    pub tls_key_path: Option<String>,

    /// A CA certificate (PEM) trusted for the TLS connections this node
    /// opens — a trunk to a cloud SBC whose certificate is not from a
    /// public CA (`docs/HYBRID_CONTROL_PLANE.md` §3.6) — beside the
    /// system's roots.
    #[serde(default)]
    pub tls_ca_path: Option<String>,

    /// Optional SIP-over-WebSocket bind address.
    #[serde(default)]
    pub ws_bind: Option<String>,

    /// Optional secure SIP-over-WebSocket bind address.
    #[serde(default)]
    pub wss_bind: Option<String>,

    /// Local IP for SDP/Contact (auto-detect if not set)
    pub local_ip: Option<String>,

    /// Local SIP URI (default: sip:call-manager@<local_ip>)
    pub local_uri: Option<String>,
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
            tls_ca_path: None,
            ws_bind: None,
            wss_bind: None,
            local_ip: None,
            local_uri: None,
        }
    }
}

/// Registrar behavior configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrarSection {
    /// Enable the SIP registrar. Set to false for session-manager mode.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Enable SIP Digest authentication
    #[serde(default = "default_true")]
    pub auth_enabled: bool,

    /// Require a digest on INVITE as well as on REGISTER
    /// (`docs/EDGE_SECURITY.md` §2.1).
    ///
    /// On by default since v0.25.397 (off for v0.25.224–396, with a warning
    /// at startup; `docs/DEEP_TESTING.md` D2 found the promised flip never
    /// made). Off, a call is authorised by the address it came from, so
    /// anything that can source packets from a registered device's address
    /// — the same NAT, the same office LAN, a compromised handset beside it
    /// — places calls as that user, at that tenant's cost; a warning says so
    /// at startup. Without `auth_enabled` there are no credentials to
    /// check, and nothing is challenged either way.
    ///
    /// When it is on, a carrier arriving on an enabled trunk and a peer in
    /// `[e911] trusted_edges` are still admitted by source; everything
    /// else, loopback included, is challenged. An embedded component that
    /// INVITEs this node over loopback therefore needs a place in
    /// `trusted_edges` or credentials of its own.
    #[serde(default = "default_auth_invites")]
    pub auth_invites: bool,

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

    /// Domain aliases - list of domains that should be treated as equivalent.
    /// All domains in the same list are considered aliases of each other.
    /// Example: ["fcp.local", "192.168.1.225", "pbx.example.com"]
    /// When looking up registrations for alice@192.168.1.225, we also check
    /// alice@fcp.local and alice@pbx.example.com.
    #[serde(default)]
    pub domain_aliases: Vec<String>,

    /// Where the admin is, e.g. `https://pbx.example.com`: told to every
    /// phone a REGISTER succeeds for in `X-FCP-Provisioning-Url`, so the
    /// softphone finds its settings and branding (`docs/SOFTPHONE.md` §2.5).
    /// Unset, the header is not sent.
    #[serde(default)]
    pub provisioning_url: Option<String>,

    /// Seconds a registration lookup is kept before the store is read
    /// again (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1, phase 2). Zero reads
    /// the store on every lookup, as this node did before the cache.
    ///
    /// A REGISTER or deregistration this node handles is seen by its next
    /// lookup whatever the TTL. One another node handles is seen when its
    /// registration event arrives — which needs `[events] use_redis`, and
    /// in another region the events server's bridge — and within the TTL
    /// at the latest when no event does: a standalone `fcp-registrar`
    /// publishes none, so behind one this is how long a phone that has
    /// just registered can go unfound here.
    #[serde(default = "default_registration_cache_ttl")]
    pub cache_ttl_secs: u64,
}

fn default_registration_cache_ttl() -> u64 {
    30
}

fn default_realm() -> String {
    "fcp.local".to_string()
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
            enabled: true,
            auth_enabled: true,
            realm: default_realm(),
            default_expires: default_expires(),
            min_expires: default_min_expires(),
            max_expires: default_max_expires(),
            domain_aliases: Vec::new(),
            provisioning_url: None,
            auth_invites: default_auth_invites(),
            cache_ttl_secs: default_registration_cache_ttl(),
        }
    }
}

fn default_auth_invites() -> bool {
    true
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

    /// Redis connection URL for registration store and cluster state.
    /// When set alongside `registration_backend = "redis"`, registrations
    /// are stored in Redis while other data remains in PostgreSQL.
    /// Example: "redis://redis:6379"
    #[serde(default)]
    pub redis_url: Option<String>,

    /// Override the registration storage backend.
    /// Set to "redis" to use Redis for registrations (requires `redis_url`).
    /// When unset or null, registrations use the primary backend.
    #[serde(default)]
    pub registration_backend: Option<String>,
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
            redis_url: None,
            registration_backend: None,
        }
    }
}

/// HTTP health/API endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    /// Bind address for health/status/calls endpoint
    #[serde(default = "default_http_bind")]
    pub bind_address: SocketAddr,

    /// Optional bearer token for authenticating the HTTP control API.
    /// Also supports `?token=<token>` for the wallboard WebSocket endpoint.
    /// Public binds require this to be set.
    #[serde(default)]
    pub api_token: Option<String>,
}

fn default_http_bind() -> SocketAddr {
    "127.0.0.1:8084".parse().unwrap()
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

impl CallManagerConfig {
    /// The limits this node keeps (`docs/CAPACITY_AND_SURVIVABILITY.md`
    /// §10): `[admission]`, with the older `[calls] max_concurrent_calls`
    /// honoured when it is set and `[admission]` names no call limit. A
    /// node that sets neither is capped by neither.
    pub fn admission(&self) -> fcp_sbc_core::AdmissionConfig {
        let mut cfg = self.admission.resolve();
        if cfg.max_concurrent_calls == 0 {
            if let Some(legacy) = self.calls.max_concurrent_calls.filter(|l| *l != 0) {
                warn!(
                    max_concurrent_calls = legacy,
                    "[calls] max_concurrent_calls is now enforced; write it as                      [admission] max_concurrent_calls"
                );
                cfg.max_concurrent_calls = legacy;
            }
        }
        cfg
    }
}

/// Call handling configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CallsSection {
    /// Maximum concurrent calls. Until v0.25.207 this was parsed and never
    /// enforced; a node that sets it now means it, and the limit is read as
    /// `[admission] max_concurrent_calls`. Unset is no limit.
    #[serde(default)]
    pub max_concurrent_calls: Option<u32>,

    /// Max ring time before 408 timeout (seconds)
    #[serde(default = "default_call_timeout")]
    pub call_timeout_seconds: u32,

    /// Generate SDP offers in INVITE
    #[serde(default = "default_true")]
    pub enable_sdp: bool,

    /// Support REFER call transfer
    #[serde(default = "default_true")]
    pub enable_refer: bool,
}

fn default_call_timeout() -> u32 {
    60
}

fn default_true() -> bool {
    true
}

impl Default for CallsSection {
    fn default() -> Self {
        Self {
            max_concurrent_calls: None,
            call_timeout_seconds: default_call_timeout(),
            enable_sdp: true,
            enable_refer: true,
        }
    }
}

/// Media/RTP configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSection {
    /// RTP port range start
    #[serde(default = "default_rtp_port_start")]
    pub rtp_port_start: u16,

    /// RTP port range end
    #[serde(default = "default_rtp_port_end")]
    pub rtp_port_end: u16,

    /// Supported codecs
    #[serde(default = "default_codecs")]
    pub codecs: Vec<String>,

    /// Public IP for SDP/RTP (required when behind NAT or in Docker)
    pub public_ip: Option<String>,

    /// TTS engine name (e.g., "kokoro"). Enables text-to-speech when set.
    #[serde(default)]
    pub tts_engine: Option<String>,

    /// Default TTS voice (e.g., "af_heart" for Kokoro)
    #[serde(default)]
    pub tts_voice: Option<String>,

    /// TTS cache directory for generated audio files
    #[serde(default = "default_tts_cache_dir")]
    pub tts_cache_dir: String,

    /// Path to Python interpreter for local TTS (e.g., ".venv/bin/python3")
    #[serde(default)]
    pub tts_python: Option<String>,

    /// URL of shared fcp-tts service.  When set, uses Remote provider
    /// instead of spawning a local TTS subprocess.
    #[serde(default)]
    pub tts_remote_url: Option<String>,

    /// Bearer token for the shared fcp-tts service.
    #[serde(default)]
    pub tts_remote_token: Option<String>,

    /// Directory for pre-generated prompt audio files
    #[serde(default = "default_prompts_dir")]
    pub prompts_dir: String,

    /// Record every call this node anchors, through the tenant's consent
    /// step: the `platform` source (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1).
    #[serde(default)]
    pub enable_recording: bool,

    /// Holds answered by the node, with music played into the held leg —
    /// the holder's choice, the tenant's hold-music prompt, or the built-in
    /// default written to `prompts_dir` at startup. On, every call is
    /// anchored; off, holds are passed to the far phone
    /// (`docs/ENTERPRISE_CALLING.md` §3.3).
    #[serde(default = "default_true")]
    pub hold_music: bool,

    /// Where call recordings are written: under it, each tenant's calls by
    /// day.
    #[serde(default = "default_recording_dir")]
    pub recording_dir: String,
}

fn default_rtp_port_start() -> u16 {
    10000
}

fn default_rtp_port_end() -> u16 {
    20000
}

fn default_codecs() -> Vec<String> {
    vec!["PCMU".to_string(), "PCMA".to_string()]
}

fn default_tts_cache_dir() -> String {
    "/tmp/fcp/tts_cache".to_string()
}

fn default_recording_dir() -> String {
    "./recordings".to_string()
}

fn default_prompts_dir() -> String {
    "/tmp/fcp/audio/prompts".to_string()
}

impl Default for MediaSection {
    fn default() -> Self {
        Self {
            rtp_port_start: default_rtp_port_start(),
            rtp_port_end: default_rtp_port_end(),
            codecs: default_codecs(),
            public_ip: None,
            tts_engine: None,
            tts_voice: None,
            tts_cache_dir: default_tts_cache_dir(),
            tts_python: None,
            tts_remote_url: None,
            tts_remote_token: None,
            prompts_dir: default_prompts_dir(),
            enable_recording: false,
            hold_music: true,
            recording_dir: default_recording_dir(),
        }
    }
}

/// Routing configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RoutingSection {
    /// Path to routing rules YAML file (optional)
    pub rules_file: Option<String>,

    /// Enable hot-reload of rules file
    #[serde(default)]
    pub hot_reload: bool,
}

/// Events configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EventsSection {
    /// URL of fcp-events-server ingest endpoint (optional).
    /// Used for HTTP-based event forwarding (fallback when Redis is not configured).
    /// Example: "http://fcp-events-server:8081/ingest"
    pub server_url: Option<String>,

    /// Optional bearer token for authenticating to fcp-events-server.
    #[serde(default)]
    pub auth_token: Option<String>,

    /// Use Redis Pub/Sub + Streams for distributed event bus.
    /// Requires `storage.redis_url` to be set.
    #[serde(default)]
    pub use_redis: bool,

    /// Not a setting. The call manager publishes to `fcp:events` and
    /// `fcp:event_stream`, and the events server and `fcp-jobs` read those
    /// names from their own `fcp:` prefix; this key was parsed and never
    /// read, so a value here is refused at startup
    /// (`docs/DEPLOYMENT_GAPS.md` §1.5).
    #[doc(hidden)]
    #[serde(default, rename = "redis_channel", skip_serializing)]
    pub unread_redis_channel: Option<String>,

    /// Not a setting; see `unread_redis_channel`.
    #[doc(hidden)]
    #[serde(default, rename = "redis_stream_key", skip_serializing)]
    pub unread_redis_stream_key: Option<String>,

    /// External webhook endpoints to receive event notifications.
    #[serde(default)]
    pub webhooks: Vec<WebhookConfig>,
}

/// Cluster configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterSection {
    /// Enable cluster mode. Requires `storage.redis_url`.
    #[serde(default)]
    pub enabled: bool,

    /// Deployment region this node advertises in cluster membership, e.g.
    /// `us-east`; falls back to the `FCP_REGION` environment variable.
    /// Placement prefers nodes in the caller's region. See
    /// `docs/MULTI_REGION.md`.
    #[serde(default)]
    pub region: Option<String>,

    /// Count each tenant's concurrent calls across the cluster, by a
    /// lease per call in Redis, rather than on this node alone
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §7). A tenant's
    /// `max_concurrent_calls` is a contract with the platform, so this is
    /// on wherever there is a cluster to count across; turning it off
    /// leaves the per-node count phase 1 shipped. Redis being unreachable
    /// falls back to that count by itself.
    #[serde(default = "default_lease_tenant_calls")]
    pub lease_tenant_calls: bool,

    /// Count each trunk's channels across the cluster, rather than on
    /// this node alone (`docs/CAPACITY_AND_SURVIVABILITY.md` §7). A
    /// carrier sells channels to the platform; counted per node, a
    /// two-node cluster offers a trunk twice them. Redis being
    /// unreachable falls back to the per-node count by itself.
    #[serde(default = "default_lease_tenant_calls")]
    pub lease_trunk_channels: bool,

    /// Seconds between the ticks that give back the leases of calls that
    /// have ended and push out the expiry of the ones still up.
    #[serde(default = "default_lease_settle_secs")]
    pub lease_settle_secs: u64,

    /// Heartbeat interval in seconds (default: 5).
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,

    /// Heartbeat TTL in seconds — if a node misses this many seconds of
    /// heartbeats, it is considered dead (default: 15).
    #[serde(default = "default_heartbeat_ttl")]
    pub heartbeat_ttl_secs: u64,

    /// Call registry TTL in seconds — maximum time a call ownership key
    /// persists in Redis before expiring (default: 7200 = 2 hours).
    #[serde(default = "default_call_ttl")]
    pub call_ttl_secs: u64,

    /// Circuit breaker: number of consecutive Redis failures before opening.
    #[serde(default = "default_cb_threshold")]
    pub redis_circuit_breaker_threshold: u32,

    /// Circuit breaker: cooldown in seconds before allowing a probe request.
    #[serde(default = "default_cb_cooldown")]
    pub redis_circuit_breaker_cooldown_secs: u64,
}

fn default_lease_tenant_calls() -> bool {
    true
}

fn default_lease_settle_secs() -> u64 {
    5
}

fn default_heartbeat_interval() -> u64 {
    5
}

fn default_heartbeat_ttl() -> u64 {
    15
}

fn default_call_ttl() -> u64 {
    7200
}

fn default_cb_threshold() -> u32 {
    5
}

fn default_cb_cooldown() -> u64 {
    30
}

impl Default for ClusterSection {
    fn default() -> Self {
        Self {
            enabled: false,
            region: None,
            lease_tenant_calls: default_lease_tenant_calls(),
            lease_trunk_channels: default_lease_tenant_calls(),
            lease_settle_secs: default_lease_settle_secs(),
            heartbeat_interval_secs: default_heartbeat_interval(),
            heartbeat_ttl_secs: default_heartbeat_ttl(),
            call_ttl_secs: default_call_ttl(),
            redis_circuit_breaker_threshold: default_cb_threshold(),
            redis_circuit_breaker_cooldown_secs: default_cb_cooldown(),
        }
    }
}

/// Webhook endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookConfig {
    /// HTTP(S) URL to POST events to.
    pub url: String,

    /// Event type filter — only forward events of these types.
    /// Valid values: "call", "media", "queue", "recording", "system", etc.
    /// If empty/unset, all events are forwarded.
    #[serde(default)]
    pub event_types: Vec<String>,

    /// Optional secret: deliveries then carry `X-FCP-Signature:
    /// t=<unix>,v1=<hex HMAC-SHA256 of "<t>.<body>">`, as a tenant's
    /// webhooks do (docs/WEBHOOKS.md).
    pub secret: Option<String>,
}

/// Load configuration from file or use defaults.
///
/// Priority:
/// 1. File specified via --config or FCP_CALL_MANAGER_CONFIG
/// 2. ./fcp-call-manager.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<CallManagerConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-call-manager.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            CallManagerConfig::default()
        }
    };

    // Validated by the caller after its overrides (`validate_config`),
    // so a secret from the environment counts.
    info!(
        node_id = %config.service.node_id,
        default_tenant_id = %config.service.default_tenant_id,
        udp_bind = %config.sip.udp_bind,
        tcp_bind = %config.sip.tcp_bind,
        registrar_enabled = config.registrar.enabled,
        auth_enabled = config.registrar.auth_enabled,
        realm = %config.registrar.realm,
        http_bind = %config.http.bind_address,
        max_calls = config.admission().max_concurrent_calls,
        rtp_ports = format!("{}-{}", config.media.rtp_port_start, config.media.rtp_port_end),
        trunks = config.trunks.len(),
        "Configuration loaded"
    );

    Ok(config)
}

pub fn validate_config(config: &CallManagerConfig) -> Result<()> {
    validate_http_config(&config.http)?;
    validate_events_config(&config.events)?;
    validate_registrar_config(&config.sip, &config.registrar)?;
    // A typo in the address list is a startup error, not a node that
    // quietly refuses everyone (`docs/EDGE_SECURITY.md` §2.4).
    config
        .security
        .validate()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    fcp_pbx::config::validate_config(&config.pbx).map_err(anyhow::Error::msg)?;
    fcp_sip::emergency::TrustedEdges::parse(&config.e911.trusted_edges)
        .map_err(|e| anyhow::anyhow!("[e911] {e}"))?;
    if let Some(push) = &config.push {
        push.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(())
}

/// A key that was parsed and never read is refused rather than kept: an
/// operator who set it expected it to do something.
fn validate_events_config(events: &EventsSection) -> Result<()> {
    for (key, value) in [
        ("redis_channel", &events.unread_redis_channel),
        ("redis_stream_key", &events.unread_redis_stream_key),
    ] {
        if value.is_some() {
            anyhow::bail!(
                "events.{key} is not a setting and was never read: the call manager publishes to \
                 fcp:events and fcp:event_stream, which the events server's [event_bus] \
                 redis_key_prefix and fcp-jobs' key_prefix (both \"fcp:\") name; remove it"
            );
        }
    }
    Ok(())
}

fn validate_http_config(http: &HttpSection) -> Result<()> {
    let public_bind = !http.bind_address.ip().is_loopback();
    let has_token = http
        .api_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty());

    if public_bind && !has_token {
        anyhow::bail!(
            "http.api_token must be set when binding the call-manager HTTP API to a non-loopback address ({})",
            http.bind_address
        );
    }

    Ok(())
}

fn validate_registrar_config(sip: &SipSection, registrar: &RegistrarSection) -> Result<()> {
    if !registrar.enabled {
        return Ok(());
    }

    let public_udp = bind_is_public(&sip.udp_bind)
        .with_context(|| format!("Invalid SIP UDP bind address '{}'", sip.udp_bind))?;
    let public_tcp = bind_is_public(&sip.tcp_bind)
        .with_context(|| format!("Invalid SIP TCP bind address '{}'", sip.tcp_bind))?;
    let public_ws = sip
        .ws_bind
        .as_deref()
        .map(bind_is_public)
        .transpose()
        .with_context(|| format!("Invalid SIP WS bind address '{:?}'", sip.ws_bind))?
        .unwrap_or(false);
    let public_wss = sip
        .wss_bind
        .as_deref()
        .map(bind_is_public)
        .transpose()
        .with_context(|| format!("Invalid SIP WSS bind address '{:?}'", sip.wss_bind))?
        .unwrap_or(false);

    if (public_udp || public_tcp || public_ws || public_wss) && !registrar.auth_enabled {
        anyhow::bail!(
            "registrar.auth_enabled must be true when the call-manager registrar binds to a non-loopback SIP address (udp={}, tcp={}, ws={:?}, wss={:?})",
            sip.udp_bind,
            sip.tcp_bind,
            sip.ws_bind,
            sip.wss_bind
        );
    }

    if sip.wss_bind.is_some() && (sip.tls_cert_path.is_none() || sip.tls_key_path.is_none()) {
        anyhow::bail!("sip.wss_bind requires sip.tls_cert_path and sip.tls_key_path");
    }

    // On by default; turned off, it is said at every start
    // (`docs/EDGE_SECURITY.md` §2.1).
    if registrar.auth_enabled && !registrar.auth_invites {
        warn!(
            "registrar.auth_invites is off: an INVITE is authorised by the address it came from \
             and never asked to prove who placed it, so anything that can source packets from a \
             registered device's address — the same NAT, the same LAN, a compromised handset \
             beside it — places calls as that user. Remove registrar.auth_invites = false \
             (docs/EDGE_SECURITY.md §2.1)"
        );
    }

    Ok(())
}

fn bind_is_public(bind: &str) -> Result<bool> {
    let addr: std::net::SocketAddr = bind.parse()?;
    Ok(!addr.ip().is_loopback())
}

fn load_config_from_file(path: &Path) -> Result<CallManagerConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: CallManagerConfig = fcp_core::config::parse_toml(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unread_events_keys_are_refused() {
        let config: CallManagerConfig = toml::from_str(
            r#"
[events]
redis_stream_key = "custom:stream"
"#,
        )
        .unwrap();
        let err = validate_events_config(&config.events)
            .unwrap_err()
            .to_string();
        assert!(err.contains("events.redis_stream_key"), "{err}");
        assert!(err.contains("fcp:event_stream"), "{err}");
        assert!(validate_events_config(&EventsSection::default()).is_ok());
        // Not carried into the database config store either.
        let json = serde_json::to_value(&config.events).unwrap();
        assert!(json.get("redis_stream_key").is_none());
    }

    #[test]
    fn test_default_config() {
        let config = CallManagerConfig::default();
        assert_eq!(config.service.service_type, "call-manager");
        assert_eq!(config.service.default_tenant_id, fcp_core::TenantId::nil());
        assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
        assert_eq!(config.sip.tcp_bind, "0.0.0.0:5060");
        assert!(config.registrar.enabled);
        assert!(config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "fcp.local");
        assert_eq!(config.registrar.default_expires, 3600);
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.http.bind_address, "127.0.0.1:8084".parse().unwrap());
        assert!(config.http.api_token.is_none());
        assert_eq!(config.calls.max_concurrent_calls, None);
        assert_eq!(config.admission().max_concurrent_calls, 0);
        assert_eq!(config.calls.call_timeout_seconds, 60);
        assert!(config.calls.enable_sdp);
        assert!(config.calls.enable_refer);
        assert_eq!(config.media.rtp_port_start, 10000);
        assert_eq!(config.media.rtp_port_end, 20000);
        // New sections
        assert!(config.routing.rules_file.is_none());
        assert!(!config.routing.hot_reload);
        assert!(config.events.server_url.is_none());
        assert!(config.events.webhooks.is_empty());
        assert!(config.trunks.is_empty());
        // PBX defaults (enabled by default)
        assert!(config.pbx.enabled);
        assert!(config.pbx.hunt_groups.is_empty());
        // Queue defaults
        assert!(!config.queue.enabled);
        assert!(config.queue.queues.is_empty());
    }

    #[test]
    fn test_public_http_bind_requires_token() {
        let http = HttpSection {
            bind_address: "0.0.0.0:8084".parse().unwrap(),
            api_token: None,
        };
        assert!(validate_http_config(&http).is_err());

        let http = HttpSection {
            bind_address: "0.0.0.0:8084".parse().unwrap(),
            api_token: Some("secret".to_string()),
        };
        assert!(validate_http_config(&http).is_ok());
    }

    #[test]
    fn test_public_registrar_bind_requires_auth() {
        let sip = SipSection {
            udp_bind: "0.0.0.0:5060".to_string(),
            tcp_bind: "0.0.0.0:5060".to_string(),
            local_ip: None,
            sips_bind: default_sips_bind(),
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
            ws_bind: None,
            wss_bind: None,
            local_uri: None,
        };
        let registrar = RegistrarSection {
            enabled: true,
            auth_enabled: false,
            realm: default_realm(),
            default_expires: default_expires(),
            min_expires: default_min_expires(),
            max_expires: default_max_expires(),
            domain_aliases: Vec::new(),
            provisioning_url: None,
            auth_invites: false,
            cache_ttl_secs: default_registration_cache_ttl(),
        };
        assert!(validate_registrar_config(&sip, &registrar).is_err());

        let registrar = RegistrarSection {
            auth_enabled: true,
            ..registrar
        };
        assert!(validate_registrar_config(&sip, &registrar).is_ok());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[service]
service_type = "call-manager"
node_id = "cm-test-01"
default_tenant_id = "00000000-0000-0000-0000-000000000123"

[sip]
udp_bind = "0.0.0.0:5060"
tcp_bind = "0.0.0.0:5060"
local_ip = "192.168.1.100"

[registrar]
auth_enabled = true
realm = "test.local"
default_expires = 1800

[storage]
database_url = "postgresql://localhost/fcp"

[http]
bind_address = "0.0.0.0:9090"

[calls]
max_concurrent_calls = 500
call_timeout_seconds = 30
enable_sdp = true
enable_refer = false

[media]
rtp_port_start = 20000
rtp_port_end = 30000
codecs = ["PCMU", "PCMA", "G722"]

[logging]
level = "debug"
format = "pretty"

[routing]
rules_file = "/app/routing.yaml"
hot_reload = true

[events]
server_url = "http://fcp-events-server:8081/ingest"

[[events.webhooks]]
url = "https://hooks.example.com/queue-events"
event_types = ["queue"]
secret = "whsec_test123"

[[events.webhooks]]
url = "https://hooks.example.com/all-events"
"#;
        let config: CallManagerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.service.node_id, "cm-test-01");
        assert_eq!(
            config.service.default_tenant_id,
            "00000000-0000-0000-0000-000000000123".parse().unwrap()
        );
        assert!(config.registrar.auth_enabled);
        assert_eq!(config.registrar.realm, "test.local");
        assert_eq!(config.registrar.default_expires, 1800);
        assert_eq!(config.http.bind_address.port(), 9090);
        assert_eq!(config.calls.max_concurrent_calls, Some(500));
        // An explicit [calls] limit is what the node now keeps.
        assert_eq!(config.admission().max_concurrent_calls, 500);
        assert!(!config.calls.enable_refer);
        assert_eq!(config.media.rtp_port_start, 20000);
        assert_eq!(config.media.codecs.len(), 3);
        assert_eq!(config.sip.local_ip.as_deref(), Some("192.168.1.100"));
        // New sections
        assert_eq!(
            config.routing.rules_file.as_deref(),
            Some("/app/routing.yaml")
        );
        assert!(config.routing.hot_reload);
        assert_eq!(
            config.events.server_url.as_deref(),
            Some("http://fcp-events-server:8081/ingest")
        );
        // Webhook config
        assert_eq!(config.events.webhooks.len(), 2);
        assert_eq!(
            config.events.webhooks[0].url,
            "https://hooks.example.com/queue-events"
        );
        assert_eq!(config.events.webhooks[0].event_types, vec!["queue"]);
        assert_eq!(
            config.events.webhooks[0].secret.as_deref(),
            Some("whsec_test123")
        );
        assert_eq!(
            config.events.webhooks[1].url,
            "https://hooks.example.com/all-events"
        );
        assert!(config.events.webhooks[1].event_types.is_empty());
        assert!(config.events.webhooks[1].secret.is_none());
    }
}
