//! Text-to-Speech (TTS) service for IVR nodes.
//!
//! Provides a flexible abstraction over multiple TTS providers:
//! - Google Cloud TTS
//! - Piper, the default local engine (`docs/SPEECH.md` §8)
//! - Kokoro, local, in the `fcp-tts-full` image
//! - OpenAI TTS
//! - AWS Polly, Azure and ElevenLabs over their vendors' APIs (#339)
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │           TtsService                        │
//! │  - Caching layer (LRU)                      │
//! │  - Provider selection                       │
//! │  - Audio format conversion                  │
//! └──────────────┬──────────────────────────────┘
//!                │
//!                ▼
//!      ┌─────────────────────┐
//!      │   TtsProvider trait │
//!      └─────────────────────┘
//!                │
//!     ┌──────────┼──────────┬─────────┬──────────┐
//!     ▼          ▼          ▼         ▼          ▼
//! ┌───────┐ ┌───────┐ ┌────────┐ ┌────────┐ ┌──────────┐
//! │Google │ │  AWS  │ │ Azure  │ │ OpenAI │ │ElevenLabs│
//! │ Cloud │ │ Polly │ │   TTS  │ │  TTS   │ │          │
//! └───────┘ └───────┘ └────────┘ └────────┘ └──────────┘
//!     │                                            │
//!     └──────────────────┬─────────────────────────┘
//!                        ▼
//!                  ┌──────────┐
//!                  │  Local   │
//!                  │   TTS    │
//!                  └──────────┘
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use fcp_media::tts::{TtsService, TtsConfig, TtsProviderConfig};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = TtsConfig {
//!     provider: TtsProviderConfig::GoogleCloud {
//!         api_key: "your-api-key".to_string(),
//!     },
//!     cache_dir: "./tts_cache".into(),
//!     cache_max_size_mb: 500,
//!     default_language: "en-US".to_string(),
//!     default_voice: None,
//! };
//!
//! let tts_service = TtsService::new(config).await?;
//!
//! // Synthesize text to audio file
//! let audio_path = tts_service
//!     .synthesize("Hello, world!", Some("en-US"), None)
//!     .await?;
//!
//! println!("Audio saved to: {}", audio_path.display());
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

pub mod providers;

/// TTS service for synthesizing text to speech.
pub struct TtsService {
    /// TTS provider implementation
    provider: Arc<dyn TtsProvider>,

    /// Stable cache namespace derived from the provider configuration.
    provider_cache_namespace: String,

    /// Cache directory for generated audio files
    cache_dir: PathBuf,

    /// Maximum cache size in MB
    cache_max_size_mb: usize,

    /// Cache index (key -> (path, size_bytes))
    cache_index: Arc<RwLock<HashMap<String, (PathBuf, usize)>>>,

    /// Current cache size in bytes
    cache_size_bytes: Arc<RwLock<usize>>,

    /// Default language code
    default_language: String,

    /// Default voice name
    default_voice: Option<String>,

    /// One lock per text being synthesized: a second request for the same
    /// text waits for the first and reads its file, rather than
    /// synthesizing it again and rewriting the file under a playback
    /// (`TtsService::warm`).
    in_flight: dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

impl TtsService {
    /// Create a new TTS service.
    pub async fn new(config: TtsConfig) -> Result<Self> {
        let provider_cache_namespace = provider_cache_namespace(&config.provider)?;

        // Create cache directory if it doesn't exist
        tokio::fs::create_dir_all(&config.cache_dir)
            .await
            .context("Failed to create TTS cache directory")?;

        // Initialize provider
        let provider = providers::create_provider(config.provider).await?;

        // Load cache index
        let (cache_index, cache_size_bytes) = Self::load_cache_index(&config.cache_dir).await?;

        info!(
            cache_dir = %config.cache_dir.display(),
            cache_size_mb = cache_size_bytes / (1024 * 1024),
            provider = %provider.name(),
            "TTS service initialized"
        );

        Ok(Self {
            provider,
            provider_cache_namespace,
            cache_dir: config.cache_dir,
            cache_max_size_mb: config.cache_max_size_mb,
            cache_index: Arc::new(RwLock::new(cache_index)),
            cache_size_bytes: Arc::new(RwLock::new(cache_size_bytes)),
            default_language: config.default_language,
            default_voice: config.default_voice,
            in_flight: dashmap::DashMap::new(),
        })
    }

    /// Synthesize text to speech and return path to audio file.
    ///
    /// # Arguments
    /// * `text` - Text to synthesize
    /// * `language` - Language code (e.g., "en-US", "es-ES"), uses default if None
    /// * `voice` - Voice name (provider-specific), uses default if None
    ///
    /// # Returns
    /// Path to generated audio file (WAV format, 8kHz, 16-bit PCM)
    pub async fn synthesize(
        &self,
        text: &str,
        language: Option<&str>,
        voice: Option<&str>,
    ) -> Result<PathBuf> {
        self.synthesize_for(text, language, voice, None).await
    }

    /// [`Self::synthesize`] for `tenant`'s call, prompt or mailbox
    /// (`docs/TENANT_SPEECH.md`): the tenant is passed to the TTS server,
    /// which speaks with the tenant's provider and voice where the request
    /// names none, and kept apart in the cache, since the same words may
    /// sound different for another tenant.
    pub async fn synthesize_for(
        &self,
        text: &str,
        language: Option<&str>,
        voice: Option<&str>,
        tenant: Option<&fcp_core::TenantId>,
    ) -> Result<PathBuf> {
        let tenant = tenant.map(|t| t.as_uuid().to_string());
        // A tenant's words sent to the shared TTS server carry only what
        // the caller chose: the server fills in the tenant's language and
        // voice, then its own (`docs/TENANT_SPEECH.md` S2). An empty
        // language is "unset" to the remote provider.
        let (language, voice) = if tenant.is_some() && self.provider.name() == "remote" {
            (language.unwrap_or(""), voice)
        } else {
            (
                language.unwrap_or(&self.default_language),
                voice.or(self.default_voice.as_deref()),
            )
        };

        // Generate cache key
        let cache_key = self.generate_cache_key(text, language, voice, tenant.as_deref());

        if let Some(path) = self.cached(&cache_key).await {
            return Ok(path);
        }
        let lock = self.in_flight.entry(cache_key.clone()).or_default().clone();
        let _synthesizing = lock.lock().await;
        let result = self
            .synthesize_uncached(text, language, voice, tenant.as_deref(), &cache_key)
            .await;
        self.in_flight.remove(&cache_key);
        result
    }

    /// Synthesize `texts` into the cache in the background, so a call that
    /// speaks them one after another hears each without waiting for it.
    pub fn warm(
        self: &Arc<Self>,
        texts: Vec<String>,
        language: Option<&str>,
        tenant: Option<&fcp_core::TenantId>,
    ) {
        let language = language.map(str::to_string);
        let tenant = tenant.copied();
        for text in texts {
            let tts = Arc::clone(self);
            let language = language.clone();
            tokio::spawn(async move {
                if let Err(e) = tts
                    .synthesize_for(&text, language.as_deref(), None, tenant.as_ref())
                    .await
                {
                    warn!(error = %e, "TTS warm-up of a prompt failed");
                }
            });
        }
    }

    async fn cached(&self, cache_key: &str) -> Option<PathBuf> {
        let cache_index = self.cache_index.read().await;
        let (path, _) = cache_index.get(cache_key)?;
        if !path.exists() {
            return None;
        }
        debug!(
            cache_key = %cache_key,
            path = %path.display(),
            "TTS cache hit"
        );
        Some(path.clone())
    }

    async fn synthesize_uncached(
        &self,
        text: &str,
        language: &str,
        voice: Option<&str>,
        tenant: Option<&str>,
        cache_key: &str,
    ) -> Result<PathBuf> {
        // Whoever held the lock before us may have made it.
        if let Some(path) = self.cached(cache_key).await {
            return Ok(path);
        }

        debug!(
            text_len = text.len(),
            language = %language,
            voice = ?voice,
            "TTS cache miss, synthesizing"
        );

        // Synthesize audio
        let audio_data = self
            .provider
            .synthesize_for(text, language, voice, tenant)
            .await
            .context("TTS synthesis failed")?;

        // Save to cache
        let cache_path = self.cache_dir.join(format!("{}.wav", cache_key));
        tokio::fs::write(&cache_path, &audio_data)
            .await
            .context("Failed to write TTS audio to cache")?;

        let file_size = audio_data.len();

        // Update cache index
        {
            let mut cache_index = self.cache_index.write().await;
            cache_index.insert(cache_key.to_string(), (cache_path.clone(), file_size));
        }

        // Update cache size and evict if necessary
        {
            let mut cache_size = self.cache_size_bytes.write().await;
            *cache_size += file_size;

            if *cache_size > self.cache_max_size_mb * 1024 * 1024 {
                drop(cache_size);
                self.evict_cache_lru().await?;
            }
        }

        info!(
            cache_key = %cache_key,
            path = %cache_path.display(),
            size_kb = file_size / 1024,
            "TTS audio cached"
        );

        Ok(cache_path)
    }

    /// Generate cache key from text, language, and voice.
    fn generate_cache_key(
        &self,
        text: &str,
        language: &str,
        voice: Option<&str>,
        tenant: Option<&str>,
    ) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(self.provider_cache_namespace.as_bytes());
        hasher.update(text.as_bytes());
        hasher.update(language.as_bytes());
        if let Some(v) = voice {
            hasher.update(v.as_bytes());
        }
        // A tenant's words are its own (`docs/TENANT_SPEECH.md`).
        if let Some(t) = tenant {
            hasher.update(b"\0tenant\0");
            hasher.update(t.as_bytes());
        }

        format!("{:x}", hasher.finalize())
    }

    /// Load cache index from directory.
    async fn load_cache_index(
        cache_dir: &Path,
    ) -> Result<(HashMap<String, (PathBuf, usize)>, usize)> {
        let mut index = HashMap::new();
        let mut total_size = 0;

        let mut entries = tokio::fs::read_dir(cache_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("wav") {
                if let Ok(metadata) = entry.metadata().await {
                    let size = metadata.len() as usize;
                    let key = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();

                    index.insert(key, (path, size));
                    total_size += size;
                }
            }
        }

        Ok((index, total_size))
    }

    /// Evict least recently used cache entries.
    async fn evict_cache_lru(&self) -> Result<()> {
        let mut cache_index = self.cache_index.write().await;
        let mut cache_size = self.cache_size_bytes.write().await;

        // Get all files with their last access times
        let mut files: Vec<(String, PathBuf, usize, std::time::SystemTime)> = Vec::new();

        for (key, (path, size)) in cache_index.iter() {
            if let Ok(metadata) = tokio::fs::metadata(path).await {
                if let Ok(accessed) = metadata.accessed() {
                    files.push((key.clone(), path.clone(), *size, accessed));
                }
            }
        }

        // Sort by access time (oldest first)
        files.sort_by_key(|(_, _, _, accessed)| *accessed);

        // Evict until cache is under limit
        let target_size = (self.cache_max_size_mb * 1024 * 1024 * 3) / 4; // 75% of max
        let mut evicted_count = 0;

        for (key, path, size, _) in files {
            if *cache_size <= target_size {
                break;
            }

            // Delete file
            if let Err(e) = tokio::fs::remove_file(&path).await {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to delete cache file"
                );
                continue;
            }

            // Update index and size
            cache_index.remove(&key);
            *cache_size = cache_size.saturating_sub(size);
            evicted_count += 1;
        }

        if evicted_count > 0 {
            info!(
                evicted_count = evicted_count,
                cache_size_mb = *cache_size / (1024 * 1024),
                "TTS cache evicted entries"
            );
        }

        Ok(())
    }

    /// Get current cache size in bytes.
    pub async fn cache_size_bytes(&self) -> usize {
        *self.cache_size_bytes.read().await
    }

    /// Get number of cached entries.
    pub async fn cache_entry_count(&self) -> usize {
        self.cache_index.read().await.len()
    }

    /// Clear all cache entries.
    pub async fn clear_cache(&self) -> Result<()> {
        let mut cache_index = self.cache_index.write().await;
        let mut cache_size = self.cache_size_bytes.write().await;

        for (path, _) in cache_index.values() {
            if let Err(e) = tokio::fs::remove_file(path).await {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to delete cache file"
                );
            }
        }

        cache_index.clear();
        *cache_size = 0;

        info!("TTS cache cleared");

        Ok(())
    }

    /// Get provider name.
    pub fn provider_name(&self) -> String {
        self.provider.name()
    }

    /// List available voices for a language.
    /// `provider`'s voices for `language` (the shared TTS server's, by
    /// name; its default when `None`).
    pub async fn voices(&self, language: &str, provider: Option<&str>) -> Result<Vec<VoiceInfo>> {
        self.provider.voices_of(language, provider).await
    }

    /// The providers this service can speak with.
    pub async fn providers(&self) -> Result<Vec<ProviderInfo>> {
        self.provider.providers().await
    }

    /// Speak `text` once, uncached, with what is given and the tenant's
    /// settings for what is not (`docs/TENANT_SPEECH.md`): a preview.
    pub async fn preview(
        &self,
        text: &str,
        language: Option<&str>,
        voice: Option<&str>,
        tenant: Option<&fcp_core::TenantId>,
        provider: Option<&str>,
    ) -> Result<Vec<u8>> {
        let tenant = tenant.map(|t| t.as_uuid().to_string());
        let remote = self.provider.name() == "remote";
        let language = match (language, remote && tenant.is_some()) {
            (Some(l), _) => l,
            (None, true) => "",
            (None, false) => &self.default_language,
        };
        let voice = if remote && tenant.is_some() {
            voice
        } else {
            voice.or(self.default_voice.as_deref())
        };
        self.provider
            .synthesize_with(text, language, voice, tenant.as_deref(), provider)
            .await
    }

    pub async fn list_voices(&self, language: &str) -> Result<Vec<VoiceInfo>> {
        self.provider.list_voices(language).await
    }
}

fn provider_cache_namespace(config: &TtsProviderConfig) -> Result<String> {
    use sha2::{Digest, Sha256};

    let serialized =
        serde_json::to_vec(config).context("Failed to serialize TTS provider config")?;
    let mut hasher = Sha256::new();
    hasher.update(&serialized);
    Ok(format!("{:x}", hasher.finalize()))
}

/// TTS provider trait.
///
/// Implement this trait to add support for a new TTS provider.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    /// Provider name (e.g., "google_cloud", "aws_polly")
    fn name(&self) -> String;

    /// Synthesize text to speech.
    ///
    /// # Arguments
    /// * `text` - Text to synthesize
    /// * `language` - Language code (e.g., "en-US")
    /// * `voice` - Optional voice name (provider-specific)
    ///
    /// # Returns
    /// Audio data in WAV format (8kHz, 16-bit PCM mono)
    async fn synthesize(&self, text: &str, language: &str, voice: Option<&str>) -> Result<Vec<u8>>;

    /// Synthesize for a tenant (`docs/TENANT_SPEECH.md`): a provider that
    /// can say whose words these are (the shared TTS server, which picks the
    /// tenant's provider and voice) is told; any other speaks as above.
    async fn synthesize_for(
        &self,
        text: &str,
        language: &str,
        voice: Option<&str>,
        _tenant: Option<&str>,
    ) -> Result<Vec<u8>> {
        self.synthesize(text, language, voice).await
    }

    /// List available voices for a language.
    async fn list_voices(&self, language: &str) -> Result<Vec<VoiceInfo>>;

    /// `provider`'s voices for `language`, where this provider fronts
    /// several (the shared TTS server); any other answers with its own.
    async fn voices_of(&self, language: &str, _provider: Option<&str>) -> Result<Vec<VoiceInfo>> {
        self.list_voices(language).await
    }

    /// The providers behind this one: itself, unless it fronts several.
    async fn providers(&self) -> Result<Vec<ProviderInfo>> {
        Ok(vec![ProviderInfo {
            name: self.name(),
            is_default: true,
        }])
    }

    /// Speak once with `provider` (the shared TTS server's, by name), for
    /// `tenant`: a preview of a choice not yet saved. Any other provider
    /// speaks as [`Self::synthesize_for`].
    async fn synthesize_with(
        &self,
        text: &str,
        language: &str,
        voice: Option<&str>,
        tenant: Option<&str>,
        _provider: Option<&str>,
    ) -> Result<Vec<u8>> {
        self.synthesize_for(text, language, voice, tenant).await
    }
}

/// A provider the TTS server speaks with (`GET /v1/providers`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
}

/// Information about a TTS voice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceInfo {
    /// Voice name/ID
    pub name: String,

    /// Display name
    pub display_name: String,

    /// Language code
    pub language: String,

    /// Gender (male, female, neutral)
    pub gender: Option<String>,

    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// TTS service configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    /// TTS provider configuration
    pub provider: TtsProviderConfig,

    /// Cache directory for generated audio
    pub cache_dir: PathBuf,

    /// Maximum cache size in MB
    #[serde(default = "default_cache_max_size")]
    pub cache_max_size_mb: usize,

    /// Default language code
    #[serde(default = "default_language")]
    pub default_language: String,

    /// Default voice name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_voice: Option<String>,
}

fn default_cache_max_size() -> usize {
    500
}

fn default_language() -> String {
    "en-US".to_string()
}

/// TTS provider configuration. Its `Debug` leaves out every key.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TtsProviderConfig {
    /// Google Cloud Text-to-Speech
    GoogleCloud {
        /// API key
        api_key: String,
    },

    /// Amazon Polly, signed with AWS Signature Version 4.
    AwsPolly {
        /// AWS region (`us-east-1`).
        region: String,
        /// Static credentials. Unset, they are read from
        /// `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN`,
        /// else from the instance's role (EC2 instance metadata, IMDSv2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_key_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret_access_key: Option<String>,
        /// `neural` (the default), `standard`, `long-form` or `generative`.
        #[serde(default = "default_polly_engine")]
        engine: String,
        /// Another endpoint than `https://polly.<region>.amazonaws.com`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
    },

    /// Azure AI Speech (Cognitive Services) text to speech.
    Azure {
        /// The Speech resource's key.
        subscription_key: String,
        /// The resource's region (`westeurope`).
        region: String,
        /// Another endpoint than
        /// `https://<region>.tts.speech.microsoft.com`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
    },

    /// OpenAI TTS (ChatGPT)
    OpenAi {
        /// OpenAI API key
        api_key: String,
        /// Model (tts-1, tts-1-hd)
        #[serde(default = "default_openai_model")]
        model: String,
    },

    /// ElevenLabs.
    #[serde(rename = "elevenlabs", alias = "eleven_labs")]
    ElevenLabs {
        /// API key
        api_key: String,
        /// Model ID; `eleven_flash_v2_5` when unset.
        #[serde(skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
        /// Another endpoint than `https://api.elevenlabs.io`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
    },

    /// Kokoro, run by a Python worker (the `fcp-tts-full` image).
    Local {
        /// The engine: `kokoro`, the only one.
        engine: String,
        /// Path to TTS executable
        #[serde(skip_serializing_if = "Option::is_none")]
        executable_path: Option<PathBuf>,
        /// Optional compute device override for local engines.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
    },

    /// Piper (`docs/SPEECH.md`): the default local engine, a native
    /// binary and ONNX voices, no Python.
    Piper {
        /// The `piper` binary; `piper` on the PATH when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binary: Option<PathBuf>,
        /// Where the voices are: `<voice>.onnx` and `<voice>.onnx.json`.
        voices_dir: PathBuf,
        /// The voice spoken when a request names none and no voice speaks
        /// its language.
        voice: String,
    },

    /// Remote TTS via shared fcp-tts HTTP service.
    Remote {
        /// URL of the fcp-tts service (e.g. "http://localhost:8093")
        url: String,
        /// Bearer token for authentication
        auth_token: String,
        /// Optional provider name to request on the remote server
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        /// Optional tenant ID for usage tracking
        #[serde(skip_serializing_if = "Option::is_none")]
        tenant_id: Option<String>,
    },
}

fn default_openai_model() -> String {
    "tts-1".to_string()
}

fn default_polly_engine() -> String {
    "neural".to_string()
}

impl std::fmt::Debug for TtsProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const KEY: &str = "<redacted>";
        match self {
            Self::GoogleCloud { .. } => f
                .debug_struct("GoogleCloud")
                .field("api_key", &KEY)
                .finish(),
            Self::AwsPolly {
                region,
                access_key_id,
                secret_access_key,
                engine,
                endpoint,
            } => f
                .debug_struct("AwsPolly")
                .field("region", region)
                .field("access_key_id", access_key_id)
                .field(
                    "secret_access_key",
                    &secret_access_key.as_ref().map(|_| KEY),
                )
                .field("engine", engine)
                .field("endpoint", endpoint)
                .finish(),
            Self::Azure {
                region, endpoint, ..
            } => f
                .debug_struct("Azure")
                .field("subscription_key", &KEY)
                .field("region", region)
                .field("endpoint", endpoint)
                .finish(),
            Self::OpenAi { model, .. } => f
                .debug_struct("OpenAi")
                .field("api_key", &KEY)
                .field("model", model)
                .finish(),
            Self::ElevenLabs {
                model_id, endpoint, ..
            } => f
                .debug_struct("ElevenLabs")
                .field("api_key", &KEY)
                .field("model_id", model_id)
                .field("endpoint", endpoint)
                .finish(),
            Self::Local {
                engine,
                executable_path,
                device,
            } => f
                .debug_struct("Local")
                .field("engine", engine)
                .field("executable_path", executable_path)
                .field("device", device)
                .finish(),
            Self::Piper {
                binary,
                voices_dir,
                voice,
            } => f
                .debug_struct("Piper")
                .field("binary", binary)
                .field("voices_dir", voices_dir)
                .field("voice", voice)
                .finish(),
            Self::Remote {
                url,
                provider,
                tenant_id,
                ..
            } => f
                .debug_struct("Remote")
                .field("url", url)
                .field("auth_token", &KEY)
                .field("provider", provider)
                .field("tenant_id", tenant_id)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    struct StubProvider {
        name: String,
    }

    #[async_trait]
    impl TtsProvider for StubProvider {
        fn name(&self) -> String {
            self.name.clone()
        }

        async fn synthesize(
            &self,
            _text: &str,
            _language: &str,
            _voice: Option<&str>,
        ) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }

        async fn list_voices(&self, _language: &str) -> Result<Vec<VoiceInfo>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn test_cache_key_generation() {
        let config = TtsConfig {
            provider: TtsProviderConfig::GoogleCloud {
                api_key: "test".to_string(),
            },
            cache_dir: "/tmp/tts".into(),
            cache_max_size_mb: 500,
            default_language: "en-US".to_string(),
            default_voice: None,
        };

        // Can't easily test without async, but structure is correct
        assert_eq!(config.cache_max_size_mb, 500);
    }

    #[test]
    fn test_provider_cache_namespace_changes_with_provider_config() {
        let openai_a = provider_cache_namespace(&TtsProviderConfig::OpenAi {
            api_key: "test-key".to_string(),
            model: "tts-1".to_string(),
        })
        .unwrap();
        let openai_b = provider_cache_namespace(&TtsProviderConfig::OpenAi {
            api_key: "test-key".to_string(),
            model: "tts-1-hd".to_string(),
        })
        .unwrap();

        assert_ne!(openai_a, openai_b);
    }

    #[test]
    fn test_cache_keys_are_isolated_by_provider_namespace() {
        let service_a = TtsService {
            provider: Arc::new(StubProvider {
                name: "openai".to_string(),
            }),
            provider_cache_namespace: "provider-a".to_string(),
            cache_dir: "/tmp/tts-a".into(),
            cache_max_size_mb: 500,
            cache_index: Arc::new(RwLock::new(HashMap::new())),
            cache_size_bytes: Arc::new(RwLock::new(0)),
            default_language: "en-US".to_string(),
            default_voice: None,
            in_flight: Default::default(),
        };
        let service_b = TtsService {
            provider: Arc::new(StubProvider {
                name: "openai".to_string(),
            }),
            provider_cache_namespace: "provider-b".to_string(),
            cache_dir: "/tmp/tts-b".into(),
            cache_max_size_mb: 500,
            cache_index: Arc::new(RwLock::new(HashMap::new())),
            cache_size_bytes: Arc::new(RwLock::new(0)),
            default_language: "en-US".to_string(),
            default_voice: None,
            in_flight: Default::default(),
        };

        assert_ne!(
            service_a.generate_cache_key("hello", "en-US", Some("nova"), None),
            service_b.generate_cache_key("hello", "en-US", Some("nova"), None)
        );
        // One tenant's words are kept apart from another's, and from none.
        let a = service_a.generate_cache_key("hello", "en-US", None, Some("t-1"));
        assert_ne!(
            a,
            service_a.generate_cache_key("hello", "en-US", None, Some("t-2"))
        );
        assert_ne!(
            a,
            service_a.generate_cache_key("hello", "en-US", None, None)
        );
    }

    /// A provider that takes a while and counts what it is asked for.
    struct SlowProvider {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl TtsProvider for SlowProvider {
        fn name(&self) -> String {
            "slow".to_string()
        }

        async fn synthesize(
            &self,
            text: &str,
            _language: &str,
            _voice: Option<&str>,
        ) -> Result<Vec<u8>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            Ok(text.as_bytes().to_vec())
        }

        async fn list_voices(&self, _language: &str) -> Result<Vec<VoiceInfo>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn a_warmed_text_is_synthesized_once() {
        let dir = tempfile::tempdir().unwrap();
        let provider = Arc::new(SlowProvider {
            calls: Default::default(),
        });
        let service = Arc::new(TtsService {
            provider: provider.clone(),
            provider_cache_namespace: "slow".to_string(),
            cache_dir: dir.path().to_path_buf(),
            cache_max_size_mb: 500,
            cache_index: Arc::new(RwLock::new(HashMap::new())),
            cache_size_bytes: Arc::new(RwLock::new(0)),
            default_language: "en-US".to_string(),
            default_voice: None,
            in_flight: Default::default(),
        });

        // A call reaching a sentence still being warmed waits for it.
        service.warm(vec!["one".into(), "two".into()], Some("en-US"), None);
        tokio::task::yield_now().await;
        let (a, b) = tokio::join!(
            service.synthesize("one", Some("en-US"), None),
            service.synthesize("two", Some("en-US"), None),
        );
        assert_eq!(std::fs::read(a.unwrap()).unwrap(), b"one");
        assert_eq!(std::fs::read(b.unwrap()).unwrap(), b"two");
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "each text synthesized once"
        );
        assert_eq!(service.cache_size_bytes().await, 6);
    }
}
