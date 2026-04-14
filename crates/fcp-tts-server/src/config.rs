use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level configuration for fcp-tts.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsServerConfig {
    #[serde(default)]
    pub service: ServiceSection,
    #[serde(default)]
    pub http: HttpSection,
    #[serde(default)]
    pub auth: AuthSection,
    #[serde(default)]
    pub tts: TtsSection,
    #[serde(default)]
    pub logging: LoggingSection,
    #[serde(default)]
    pub cluster: ClusterSection,
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
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
pub struct HttpSection {
    #[serde(default = "default_http_bind")]
    pub bind_address: String,
}

fn default_http_bind() -> String {
    "0.0.0.0:8093".to_string()
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
pub struct AuthSection {
    #[serde(default)]
    pub tokens: Vec<String>,
}

// ---------------------------------------------------------------------------
// TTS
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
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
// Logging
// ---------------------------------------------------------------------------

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
pub struct ClusterSection {
    #[serde(default)]
    pub enabled: bool,
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
            let config: TtsServerConfig = toml::from_str(&text)
                .map_err(|e| anyhow::anyhow!("Failed to parse config '{}': {}", p, e))?;
            Ok(config)
        }
        None => Ok(TtsServerConfig {
            service: ServiceSection::default(),
            http: HttpSection::default(),
            auth: AuthSection::default(),
            tts: TtsSection::default(),
            logging: LoggingSection::default(),
            cluster: ClusterSection::default(),
        }),
    }
}
