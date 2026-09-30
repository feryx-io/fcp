use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level configuration for fcp-tts.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TtsServerConfig {
    #[serde(default)]
    pub service: ServiceSection,
    #[serde(default)]
    pub http: HttpSection,
    #[serde(default)]
    pub auth: AuthSection,
    #[serde(default)]
    pub tts: TtsSection,
    /// Speech recognition, served at `/v1/transcribe`.
    #[serde(default)]
    pub stt: SttSection,
    #[serde(default)]
    pub logging: LoggingSection,
    #[serde(default)]
    pub cluster: ClusterSection,
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSection {
    #[serde(default = "default_service_type")]
    pub service_type: String,
    #[serde(default = "default_node_id")]
    pub node_id: String,
}

fn default_service_type() -> String {
    "tts".to_string()
}
fn default_node_id() -> String {
    format!("tts-{}", uuid::Uuid::new_v4().simple())
}

impl Default for ServiceSection {
    fn default() -> Self {
        Self {
            service_type: default_service_type(),
            node_id: default_node_id(),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    #[serde(default = "default_http_bind")]
    pub bind_address: String,
}

fn default_http_bind() -> String {
    "127.0.0.1:8093".to_string()
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            bind_address: default_http_bind(),
        }
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthSection {
    #[serde(default)]
    pub tokens: Vec<String>,
    #[serde(default)]
    pub allow_unauthenticated: bool,
}

// ---------------------------------------------------------------------------
// TTS
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TtsSection {
    #[serde(default = "default_provider_name")]
    pub default_provider: String,

    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,

    #[serde(default = "default_cache_max_size_mb")]
    pub cache_max_size_mb: usize,

    #[serde(default = "default_language")]
    pub default_language: String,

    #[serde(default)]
    pub default_voice: Option<String>,

    #[serde(default)]
    pub providers: Vec<ProviderEntry>,
}

fn default_provider_name() -> String {
    "kokoro".to_string()
}
fn default_cache_dir() -> PathBuf {
    PathBuf::from("/tmp/fcp/tts_cache")
}
fn default_cache_max_size_mb() -> usize {
    1000
}
fn default_language() -> String {
    "en-US".to_string()
}

impl Default for TtsSection {
    fn default() -> Self {
        Self {
            default_provider: default_provider_name(),
            cache_dir: default_cache_dir(),
            cache_max_size_mb: default_cache_max_size_mb(),
            default_language: default_language(),
            default_voice: None,
            providers: Vec::new(),
        }
    }
}

/// A named TTS provider entry in the configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderEntry {
    /// Logical name used to reference this provider (e.g. "kokoro", "openai-prod").
    pub name: String,

    /// Provider type: "local", "kokoro", "openai", "google_cloud", "elevenlabs", etc.
    #[serde(rename = "type")]
    pub provider_type: String,

    /// Local engine name (for type = "local" / "kokoro").
    #[serde(default)]
    pub engine: Option<String>,

    /// Path to Python interpreter (for local providers).
    #[serde(default)]
    pub python_path: Option<String>,

    /// API key (for cloud providers).
    #[serde(default)]
    pub api_key: Option<String>,

    /// Model name (for OpenAI, ElevenLabs).
    #[serde(default)]
    pub model: Option<String>,

    /// GPU device for local models: "auto", "cuda", "cuda:0", "cuda:1", "cpu".
    #[serde(default = "default_device")]
    pub device: String,
}

fn default_device() -> String {
    "auto".to_string()
}

// ---------------------------------------------------------------------------
// STT
// ---------------------------------------------------------------------------

/// Speech recognition: named providers, each an
/// [`fcp_media::stt::SttProviderConfig`] —
/// `[stt.providers.deepgram] type = "deepgram"` and so on.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SttSection {
    /// The provider a request naming none is sent to; the only one when
    /// there is one.
    #[serde(default)]
    pub default_provider: Option<String>,

    /// The language a request naming none is in.
    #[serde(default = "default_language")]
    pub language: String,

    #[serde(default)]
    pub providers: std::collections::BTreeMap<String, fcp_media::stt::SttProviderConfig>,
}

impl SttSection {
    /// The default provider's name, when any provider is configured.
    pub fn default_name(&self) -> Option<&str> {
        self.default_provider
            .as_deref()
            .or_else(|| self.providers.keys().next().map(String::as_str))
    }
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

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
    "pretty".to_string()
}

impl Default for LoggingSection {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

// ---------------------------------------------------------------------------
// Cluster
// ---------------------------------------------------------------------------

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
    #[serde(default = "default_redis_url")]
    pub redis_url: String,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,
    #[serde(default = "default_heartbeat_ttl")]
    pub heartbeat_ttl_secs: u64,
}

fn default_redis_url() -> String {
    "redis://localhost:6379".to_string()
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
            redis_url: default_redis_url(),
            heartbeat_interval_secs: default_heartbeat_interval(),
            heartbeat_ttl_secs: default_heartbeat_ttl(),
        }
    }
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Load configuration from a TOML file, falling back to defaults.
pub fn load_config(path: Option<&str>) -> anyhow::Result<TtsServerConfig> {
    match path {
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("Failed to read config '{}': {}", p, e))?;
            let config: TtsServerConfig = fcp_core::config::parse_toml(&text)
                .map_err(|e| e.context(format!("Failed to parse config '{}'", p)))?;
            Ok(config)
        }
        None => Ok(TtsServerConfig {
            service: ServiceSection::default(),
            http: HttpSection::default(),
            auth: AuthSection::default(),
            tts: TtsSection::default(),
            stt: SttSection::default(),
            logging: LoggingSection::default(),
            cluster: ClusterSection::default(),
        }),
    }
}

pub fn validate_runtime_config(config: &TtsServerConfig) -> anyhow::Result<()> {
    if config.auth.tokens.is_empty() && !config.auth.allow_unauthenticated {
        anyhow::bail!(
            "Authentication is required by default. Configure [auth].tokens or set [auth].allow_unauthenticated = true explicitly"
        );
    }

    if config.tts.providers.is_empty() && config.stt.providers.is_empty() {
        anyhow::bail!(
            "At least one provider must be configured, under [tts].providers or [stt.providers]"
        );
    }

    if config.stt.providers.len() > 1 && config.stt.default_provider.is_none() {
        anyhow::bail!("[stt] default_provider must name one of the [stt.providers]");
    }
    if let Some(name) = &config.stt.default_provider {
        if !config.stt.providers.contains_key(name) {
            anyhow::bail!("[stt] default_provider = \"{name}\" is not one of the [stt.providers]");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> TtsServerConfig {
        TtsServerConfig {
            service: ServiceSection::default(),
            http: HttpSection::default(),
            auth: AuthSection::default(),
            tts: TtsSection {
                default_provider: "primary".to_string(),
                cache_dir: "/tmp/fcp/tts-test".into(),
                cache_max_size_mb: 100,
                default_language: "en-US".to_string(),
                default_voice: None,
                providers: vec![ProviderEntry {
                    name: "primary".to_string(),
                    provider_type: "openai".to_string(),
                    engine: None,
                    python_path: None,
                    api_key: Some("test-key".to_string()),
                    model: Some("tts-1".to_string()),
                    device: "auto".to_string(),
                }],
            },
            stt: SttSection::default(),
            logging: LoggingSection::default(),
            cluster: ClusterSection::default(),
        }
    }

    #[test]
    fn test_default_bind_is_loopback() {
        assert_eq!(default_http_bind(), "127.0.0.1:8093");
    }

    #[test]
    fn test_validate_runtime_config_requires_explicit_auth_opt_out() {
        let config = test_config();
        let error = validate_runtime_config(&config).unwrap_err();
        assert!(error
            .to_string()
            .contains("Authentication is required by default"));
    }

    #[test]
    fn test_validate_runtime_config_allows_explicit_unauthenticated_mode() {
        let mut config = test_config();
        config.auth.allow_unauthenticated = true;

        validate_runtime_config(&config).unwrap();
    }

    #[test]
    fn speech_providers_are_named_and_the_default_must_be_one_of_them() {
        let mut config: TtsServerConfig = fcp_core::config::parse_toml(
            r#"[auth]
tokens = ["t"]

[stt]
language = "en-GB"

[stt.providers.deepgram]
type = "deepgram"
api_key = "dg"

[stt.providers.whisper]
type = "local"
model = "small"
"#,
        )
        .unwrap();
        assert!(config.tts.providers.is_empty());
        let error = validate_runtime_config(&config).unwrap_err().to_string();
        assert!(error.contains("default_provider must name"), "{error}");
        config.stt.default_provider = Some("whisper".into());
        validate_runtime_config(&config).unwrap();
        assert_eq!(config.stt.default_name(), Some("whisper"));
        config.stt.default_provider = Some("nope".into());
        assert!(validate_runtime_config(&config).is_err());
        assert!(fcp_core::config::parse_toml::<TtsServerConfig>(
            "[stt.providers.x]\ntype = \"deepgram\"\napi_key = \"k\"\nvoice = \"v\"\n"
        )
        .is_err());
    }
}
