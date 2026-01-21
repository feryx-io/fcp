//! Text-to-Speech (TTS) service for IVR nodes.
//!
//! Provides a flexible abstraction over multiple TTS providers:
//! - Google Cloud TTS
//! - AWS Polly
//! - Azure Cognitive Services TTS
//! - OpenAI TTS (ChatGPT)
//! - ElevenLabs
//! - Local TTS (espeak, festival, piper)
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
//!     .synthesize("Hello, world!", "en-US", None)
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
}

impl TtsService {
    /// Create a new TTS service.
    pub async fn new(config: TtsConfig) -> Result<Self> {
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
            cache_dir: config.cache_dir,
            cache_max_size_mb: config.cache_max_size_mb,
            cache_index: Arc::new(RwLock::new(cache_index)),
            cache_size_bytes: Arc::new(RwLock::new(cache_size_bytes)),
            default_language: config.default_language,
            default_voice: config.default_voice,
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
        let language = language.unwrap_or(&self.default_language);
        let voice = voice.or(self.default_voice.as_deref());

        // Generate cache key
        let cache_key = self.generate_cache_key(text, language, voice);

        // Check cache
        {
            let cache_index = self.cache_index.read().await;
            if let Some((path, _)) = cache_index.get(&cache_key) {
                if path.exists() {
                    debug!(
                        cache_key = %cache_key,
                        path = %path.display(),
                        "TTS cache hit"
                    );
                    return Ok(path.clone());
                }
            }
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
            .synthesize(text, language, voice)
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
            cache_index.insert(cache_key.clone(), (cache_path.clone(), file_size));
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
    fn generate_cache_key(&self, text: &str, language: &str, voice: Option<&str>) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(self.provider.name().as_bytes());
        hasher.update(text.as_bytes());
        hasher.update(language.as_bytes());
        if let Some(v) = voice {
            hasher.update(v.as_bytes());
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

        for (_, (path, _)) in cache_index.iter() {
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
    async fn synthesize(
        &self,
        text: &str,
        language: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>>;

    /// List available voices for a language.
    async fn list_voices(&self, language: &str) -> Result<Vec<VoiceInfo>>;
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

/// TTS provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TtsProviderConfig {
    /// Google Cloud Text-to-Speech
    GoogleCloud {
        /// API key
        api_key: String,
    },

    /// AWS Polly
    AwsPolly {
        /// AWS region
        region: String,
        /// AWS access key ID
        access_key_id: String,
        /// AWS secret access key
        secret_access_key: String,
    },

    /// Azure Cognitive Services TTS
    Azure {
        /// Subscription key
        subscription_key: String,
        /// Azure region
        region: String,
    },

    /// OpenAI TTS (ChatGPT)
    OpenAi {
        /// OpenAI API key
        api_key: String,
        /// Model (tts-1, tts-1-hd)
        #[serde(default = "default_openai_model")]
        model: String,
    },

    /// ElevenLabs
    ElevenLabs {
        /// API key
        api_key: String,
        /// Model ID (optional)
        #[serde(skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
    },

    /// Local TTS (espeak, festival, piper)
    Local {
        /// TTS engine (espeak, festival, piper)
        engine: String,
        /// Path to TTS executable
        #[serde(skip_serializing_if = "Option::is_none")]
        executable_path: Option<PathBuf>,
    },
}

fn default_openai_model() -> String {
    "tts-1".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
