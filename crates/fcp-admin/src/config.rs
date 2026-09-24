//! Configuration loading and management for fcp-admin.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tracing::info;

/// Admin API configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdminConfig {
    /// Service configuration
    #[serde(default)]
    pub service: ServiceConfig,

    /// API configuration
    #[serde(default)]
    pub api: ApiConfig,

    /// Storage configuration
    #[serde(default)]
    pub storage: StorageConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// Events configuration
    #[serde(default)]
    pub events: EventsConfig,

    /// Cluster monitoring configuration
    #[serde(default)]
    pub cluster: ClusterConfig,

    /// Conference server used for live room controls
    #[serde(default)]
    pub conference: ConferenceConfig,

    /// The call manager a route preview is asked of
    #[serde(default)]
    pub call_manager: CallManagerConfig,

    /// Directory for voicemail greetings (used by greeting upload/download endpoints)
    #[serde(default)]
    pub voicemail_greetings_dir: Option<String>,

    /// Directory for voicemail recordings (used by message audio playback)
    #[serde(default)]
    pub voicemail_recordings_dir: Option<String>,

    /// Directory for user profile pictures
    #[serde(default)]
    pub profile_pictures_dir: Option<String>,

    /// Directory for audio prompt files (IVR, voicemail, system prompts)
    #[serde(default)]
    pub prompts_dir: Option<String>,

    /// Directory holding call recordings, mounted at the path the
    /// recording hosts write under. Recordings are deleted by their rows;
    /// the reaper removes the files from here.
    #[serde(default)]
    pub call_recordings_dir: Option<String>,

    /// Platform upkeep beside the tenants' retention
    /// (`docs/COMPLIANCE_FOLLOWUPS.md` §4).
    #[serde(default)]
    pub retention: RetentionConfig,

    /// Subject requests: where export bundles go and how long they stay
    /// (`docs/COMPLIANCE_FOLLOWUPS.md` §6).
    #[serde(default)]
    pub compliance: ComplianceConfig,

    /// Security policy configuration (account lockout, password expiration)
    #[serde(default)]
    pub security: SecurityConfig,

    /// TTS configuration for prompt audio synthesis
    #[serde(default)]
    pub tts: Option<TtsSection>,

    /// Emergency calling: how E911 addresses are validated
    /// (`docs/E911_BLOCK_2.md` §7).
    #[serde(default)]
    pub e911: E911Section,
}

/// The `[e911]` section.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct E911Section {
    #[serde(default)]
    pub validator: ValidatorConfig,
}

/// `[e911.validator]`: which validator answers
/// `POST …/e911/validate`.
///
/// ```toml
/// [e911.validator]
/// provider = "http"
/// name = "acme-msag"              # what validated addresses are marked by
/// url = "https://validate.example.net/e911"
/// token = "enc:v1:…"              # sealed with POST /v1/system/encryption/seal
/// # token_file = "/run/secrets/e911-validator"
/// timeout_secs = 5
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidatorConfig {
    /// `basic` (completeness and structure, no external call) or `http`.
    #[serde(default = "default_validator_provider")]
    pub provider: String,

    /// What a validated address is marked by; the provider's name by
    /// default, so several deployments' answers stay distinguishable.
    #[serde(default)]
    pub name: Option<String>,

    /// Where `http` posts the address.
    #[serde(default)]
    pub url: Option<String>,

    /// The bearer token for that URL. A value sealed with the deployment's
    /// encryption at rest (`enc:v1:…`, from
    /// `POST /api/v1/system/encryption/seal`) is opened at startup.
    #[serde(default)]
    pub token: Option<String>,

    /// A file holding that token instead (a mounted secret). Takes
    /// precedence over `token`; its contents may also be sealed.
    #[serde(default)]
    pub token_file: Option<String>,

    /// How long to wait for the provider; an administrator is waiting.
    #[serde(default = "default_validator_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_validator_provider() -> String {
    "basic".to_string()
}

fn default_validator_timeout_secs() -> u64 {
    5
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self {
            provider: default_validator_provider(),
            name: None,
            url: None,
            token: None,
            token_file: None,
            timeout_secs: default_validator_timeout_secs(),
        }
    }
}

/// The `[compliance]` section.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComplianceConfig {
    /// Where subject export bundles are written; the API serves them from
    /// here, so every admin replica mounts the same directory.
    #[serde(default = "default_exports_dir")]
    pub exports_dir: String,
    /// Days a finished export's bundle can be downloaded.
    #[serde(default = "default_export_ttl_days")]
    pub export_ttl_days: u32,
    /// How often the worker looks for requests.
    #[serde(default = "default_subject_poll_secs")]
    pub poll_secs: u64,
}

fn default_exports_dir() -> String {
    "/var/lib/fcp/exports".to_string()
}

fn default_export_ttl_days() -> u32 {
    fcp_core::subject_request::DEFAULT_EXPORT_TTL_DAYS
}

fn default_subject_poll_secs() -> u64 {
    30
}

impl Default for ComplianceConfig {
    fn default() -> Self {
        Self {
            exports_dir: default_exports_dir(),
            export_ttl_days: default_export_ttl_days(),
            poll_secs: default_subject_poll_secs(),
        }
    }
}

/// The `[retention]` section: what the platform tidies whatever the
/// tenants' policies say.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionConfig {
    /// Remove files in the recordings, voicemail and profile picture
    /// directories that no row names once they are this many days old.
    /// Unset leaves them.
    #[serde(default)]
    pub orphan_files_days: Option<u32>,
    /// Remove SIP traces of no tenant's call (registrations, options)
    /// after this many days. Unset keeps them.
    #[serde(default)]
    pub system_trace_days: Option<u32>,
    /// Remove voicemail messages this many days after a user deleted them.
    #[serde(default = "default_deleted_voicemail_grace_days")]
    pub deleted_voicemail_grace_days: u32,
    /// More directories files may be removed from (a queue's recording
    /// directory mounted here, a voicemail server's).
    #[serde(default)]
    pub extra_file_roots: Vec<String>,
}

fn default_deleted_voicemail_grace_days() -> u32 {
    7
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            orphan_files_days: None,
            system_trace_days: None,
            deleted_voicemail_grace_days: default_deleted_voicemail_grace_days(),
            extra_file_roots: Vec::new(),
        }
    }
}

/// TTS configuration for synthesizing prompt audio.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TtsSection {
    /// TTS engine name (e.g., "kokoro")
    pub engine: String,

    /// Path to Python interpreter (e.g., ".venv/bin/python3")
    #[serde(default)]
    pub python_path: Option<String>,

    /// TTS cache directory
    #[serde(default = "default_tts_cache_dir")]
    pub cache_dir: String,

    /// Default voice
    #[serde(default)]
    pub voice: Option<String>,
}

fn default_tts_cache_dir() -> String {
    "/tmp/fcp/tts_cache".to_string()
}

/// Security policy configuration for login protection.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityConfig {
    /// Maximum failed login attempts before lockout (default: 5)
    #[serde(default = "default_max_failed_attempts")]
    pub max_failed_attempts: u32,

    /// Lockout duration in seconds (default: 1800 = 30 minutes)
    #[serde(default = "default_lockout_duration_secs")]
    pub lockout_duration_secs: u64,

    /// Password expiration in days (None/0 = disabled)
    #[serde(default)]
    pub password_expiration_days: Option<u32>,
}

fn default_max_failed_attempts() -> u32 {
    5
}

fn default_lockout_duration_secs() -> u64 {
    1800
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            max_failed_attempts: default_max_failed_attempts(),
            lockout_duration_secs: default_lockout_duration_secs(),
            password_expiration_days: None,
        }
    }
}

/// Service configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// Service type (always "admin")
    #[serde(default = "default_service_type")]
    pub service_type: String,

    /// Node identifier
    #[serde(default = "default_node_id")]
    pub node_id: String,

    /// Default tenant scope for tenant-scoped admin operations.
    #[serde(default = "default_tenant_id")]
    pub default_tenant_id: fcp_core::TenantId,
}

fn default_service_type() -> String {
    "admin".to_string()
}

fn default_node_id() -> String {
    format!("admin-{}", uuid::Uuid::new_v4())
}

fn default_tenant_id() -> fcp_core::TenantId {
    fcp_core::TenantId::nil()
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
            default_tenant_id: default_tenant_id(),
        }
    }
}

/// API configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    /// Bind address (internal only, behind gateway)
    #[serde(default = "default_bind_address")]
    pub bind_address: SocketAddr,

    /// Enable authentication
    #[serde(default = "default_true")]
    pub auth_enabled: bool,

    /// Bearer authentication token
    #[serde(default)]
    pub auth_token: Option<String>,

    /// Shared token accepted only by the device auto-provisioning endpoint.
    ///
    /// Keep this distinct from `auth_token`; provisioning tokens may be placed
    /// in device bootstrap URLs and must not grant admin API access.
    #[serde(default)]
    pub provisioning_auth_token: Option<String>,

    /// Require HTTPS for the provisioning endpoint (checked via
    /// `X-Forwarded-Proto`).  Defaults to `true` because provisioning
    /// responses may contain SIP digest credentials.
    #[serde(default = "default_true")]
    pub provisioning_require_https: bool,

    /// Allow unauthenticated tenant self-service signup.
    #[serde(default)]
    pub allow_public_signup: bool,

    /// Deployment mode for permission evaluation
    #[serde(default)]
    pub deployment_mode: fcp_core::permissions::DeploymentMode,
}

fn default_bind_address() -> SocketAddr {
    "127.0.0.1:8080".parse().unwrap()
}

fn default_true() -> bool {
    true
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            auth_enabled: true,
            auth_token: Some(uuid::Uuid::new_v4().to_string()),
            provisioning_auth_token: None,
            provisioning_require_https: true,
            allow_public_signup: false,
            deployment_mode: fcp_core::permissions::DeploymentMode::default(),
        }
    }
}

/// Storage configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
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

    /// Minimum database connections
    #[serde(default = "default_min_connections")]
    pub min_connections: u32,
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

fn default_min_connections() -> u32 {
    5
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            database_url: default_database_url(),
            read_replica_url: None,
            max_replica_lag_secs: default_max_replica_lag_secs(),
            max_connections: default_max_connections(),
            min_connections: default_min_connections(),
        }
    }
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
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

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

/// Events configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventsConfig {
    /// URL of fcp-events-server ingest endpoint (optional)
    /// Example: "http://fcp-events-server:8081/ingest"
    #[serde(default)]
    pub server_url: Option<String>,

    /// Optional bearer token for authenticating to fcp-events-server.
    #[serde(default)]
    pub auth_token: Option<String>,
}

/// Cluster monitoring configuration.
///
/// When `redis_url` is set, the admin API connects to Redis to read
/// cluster heartbeat keys written by other FCP services. When absent,
/// cluster monitoring is disabled and the UI shows setup instructions.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterConfig {
    /// Redis connection URL (e.g. "redis://localhost:6379").
    /// When None, cluster monitoring is disabled.
    #[serde(default)]
    pub redis_url: Option<String>,

    /// Key prefix for Redis keys (default: "fcp:").
    #[serde(default = "default_key_prefix")]
    pub key_prefix: String,
}

fn default_key_prefix() -> String {
    "fcp:".to_string()
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            redis_url: None,
            key_prefix: default_key_prefix(),
        }
    }
}

/// Conference server configuration.
///
/// The call manager the route preview asks what it would do with a call
/// (`docs/TENANT_TRUNKS.md` §12, phase 6d). Without `url`, a healthy,
/// non-draining call manager is found through `[cluster]`; with neither,
/// the preview answers 503.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CallManagerConfig {
    /// Base URL of a call manager's HTTP API
    /// Example: "http://fcp-call-manager:8080"
    #[serde(default)]
    pub url: Option<String>,

    /// Its `[http] api_token`, when it has one — the preview is behind it.
    #[serde(default)]
    pub auth_token: Option<String>,
}

/// Live room controls (video layout, pin, spotlight, participant video,
/// dial-out) are proxied to one conference server's HTTP API. Any
/// conference node will do: it applies an operation when it hosts the
/// room and otherwise forwards to the hosting nodes itself. When
/// `server_url` is absent the live endpoints answer 503.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConferenceConfig {
    /// Base URL of a conference server's HTTP API
    /// Example: "http://fcp-conference-server:8090"
    #[serde(default)]
    pub server_url: Option<String>,

    /// Bearer token for that API, when it has `http.auth_enabled`.
    #[serde(default)]
    pub auth_token: Option<String>,

    /// The secret browser join tokens are signed with; every conference
    /// server must carry the same value as `conference.join_secret`.
    /// Without it the join-token endpoints answer 503.
    #[serde(default)]
    pub join_secret: Option<String>,

    /// The WebRTC gateway's public base URL (e.g. "https://meet.example.com").
    /// With it, a minted token comes with the link to the gateway's join
    /// page, and the admin UI's "Join in browser" opens it.
    #[serde(default)]
    pub gateway_url: Option<String>,
}

/// Load configuration from file or use defaults.
///
/// Configuration priority:
/// 1. File specified via --config or FCP_ADMIN_CONFIG
/// 2. ./fcp-admin.toml in current directory
/// 3. Default configuration
pub fn load_config(config_path: Option<&Path>) -> Result<AdminConfig> {
    let config = if let Some(path) = config_path {
        load_config_from_file(path)?
    } else {
        let default_path = Path::new("fcp-admin.toml");
        if default_path.exists() {
            info!("Loading configuration from {}", default_path.display());
            load_config_from_file(default_path)?
        } else {
            info!("No configuration file found, using defaults");
            AdminConfig::default()
        }
    };

    info!(
        bind_address = %config.api.bind_address,
        auth_enabled = config.api.auth_enabled,
        "Configuration loaded successfully; validation deferred until after overrides"
    );

    Ok(config)
}

pub fn validate_config(config: &AdminConfig) -> Result<()> {
    match config.e911.validator.provider.as_str() {
        "basic" => {}
        "http" => {
            if config
                .e911
                .validator
                .url
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
            {
                anyhow::bail!(
                    "e911.validator.url must be set when e911.validator.provider = \"http\""
                );
            }
            if config.e911.validator.timeout_secs == 0 {
                anyhow::bail!("e911.validator.timeout_secs must be greater than zero");
            }
        }
        other => {
            anyhow::bail!("e911.validator.provider must be \"basic\" or \"http\" (got {other:?})")
        }
    }

    let public_bind = !config.api.bind_address.ip().is_loopback();
    let has_token = config
        .api
        .auth_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty());

    if config.api.auth_enabled && !has_token {
        anyhow::bail!("api.auth_token must be set when api.auth_enabled = true");
    }

    let has_provisioning_token = config
        .api
        .provisioning_auth_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty());

    if let Some(raw_token) = config.api.provisioning_auth_token.as_deref() {
        if raw_token.contains('\r') || raw_token.contains('\n') {
            anyhow::bail!("api.provisioning_auth_token must not contain CR/LF characters");
        }
        let trimmed = raw_token.trim();
        if !raw_token.is_empty() && trimmed.is_empty() {
            anyhow::bail!(
                "api.provisioning_auth_token must not be whitespace-only (use unset to disable auto-provisioning)"
            );
        }
        // Auto-provisioning tokens gate access to device SIP credentials, so
        // require the same ≥32-char entropy floor enforced for per-device
        // tokens. An unset value is fine — the endpoint fails closed.
        if !trimmed.is_empty() && trimmed.len() < 32 {
            anyhow::bail!(
                "api.provisioning_auth_token must be at least 32 characters (got {})",
                trimmed.len()
            );
        }
    }

    if has_token && has_provisioning_token {
        let auth_token = config.api.auth_token.as_deref().unwrap_or_default().trim();
        let provisioning_token = config
            .api
            .provisioning_auth_token
            .as_deref()
            .unwrap_or_default()
            .trim();
        if auth_token == provisioning_token {
            anyhow::bail!("api.provisioning_auth_token must be distinct from api.auth_token");
        }
    }

    if config.api.allow_public_signup && !has_token {
        anyhow::bail!("api.auth_token must be set when api.allow_public_signup = true");
    }

    if public_bind && !config.api.auth_enabled {
        anyhow::bail!(
            "api.auth_enabled must be true when binding the admin API to a non-loopback address ({})",
            config.api.bind_address
        );
    }

    if public_bind && !has_token {
        anyhow::bail!(
            "api.auth_token must be set when binding the admin API to a non-loopback address ({})",
            config.api.bind_address
        );
    }

    Ok(())
}

/// Load configuration from a specific file.
fn load_config_from_file(path: &Path) -> Result<AdminConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: AdminConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    info!("Configuration loaded from {}", path.display());
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AdminConfig::default();
        assert_eq!(config.service.service_type, "admin");
        assert_eq!(config.service.default_tenant_id, fcp_core::TenantId::nil());
        assert_eq!(config.api.bind_address.port(), 8080);
        assert!(config.api.bind_address.ip().is_loopback());
        assert!(config.api.auth_enabled);
        assert!(config
            .api
            .auth_token
            .as_deref()
            .is_some_and(|token| !token.is_empty()));
        assert!(config.api.provisioning_auth_token.is_none());
        assert!(!config.api.allow_public_signup);
        assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn conference_section_is_optional_and_parses() {
        let config: AdminConfig = toml::from_str("").unwrap();
        assert!(config.conference.server_url.is_none());
        let config: AdminConfig = toml::from_str(
            "[conference]\nserver_url = \"http://conf-a:8090\"\nauth_token = \"s3cret\"\n",
        )
        .unwrap();
        assert_eq!(
            config.conference.server_url.as_deref(),
            Some("http://conf-a:8090")
        );
        assert_eq!(config.conference.auth_token.as_deref(), Some("s3cret"));
    }

    #[test]
    fn test_load_default_config() {
        // Should not fail when no config file exists
        let result = load_config(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_public_bind_requires_auth() {
        let mut config = AdminConfig::default();
        config.api.bind_address = "0.0.0.0:8080".parse().unwrap();
        config.api.auth_enabled = false;
        assert!(validate_config(&config).is_err());

        config.api.auth_enabled = true;
        config.api.auth_token = None;
        assert!(validate_config(&config).is_err());

        config.api.auth_token = Some("secret".to_string());
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_public_signup_requires_token() {
        let mut config = AdminConfig::default();
        config.api.allow_public_signup = true;
        config.api.auth_token = None;
        assert!(validate_config(&config).is_err());

        config.api.auth_token = Some("secret".to_string());
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_provisioning_token_must_be_distinct() {
        let mut config = AdminConfig::default();
        let shared = "a".repeat(32);
        config.api.auth_token = Some(shared.clone());
        config.api.provisioning_auth_token = Some(shared);
        assert!(validate_config(&config).is_err());

        config.api.provisioning_auth_token = Some("b".repeat(32));
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_provisioning_token_minimum_length() {
        let mut config = AdminConfig::default();
        config.api.auth_token = Some("a".repeat(32));

        // Too short.
        config.api.provisioning_auth_token = Some("short-token".to_string());
        let err = validate_config(&config).expect_err("11-char token must be rejected");
        assert!(err.to_string().contains("at least 32 characters"));

        // 31 chars is still short.
        config.api.provisioning_auth_token = Some("b".repeat(31));
        assert!(validate_config(&config).is_err());

        // Exactly 32 chars is the floor.
        config.api.provisioning_auth_token = Some("b".repeat(32));
        assert!(validate_config(&config).is_ok());

        // Unset is fine — the endpoint fails closed at runtime.
        config.api.provisioning_auth_token = None;
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_provisioning_token_rejects_whitespace_only() {
        let mut config = AdminConfig::default();
        config.api.auth_token = Some("a".repeat(32));

        config.api.provisioning_auth_token = Some("   ".to_string());
        let err = validate_config(&config).expect_err("whitespace-only must be rejected");
        assert!(err.to_string().contains("whitespace-only"));

        config.api.provisioning_auth_token = Some("\t\t".to_string());
        assert!(validate_config(&config).is_err());
    }
}
