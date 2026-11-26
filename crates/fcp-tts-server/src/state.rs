use fcp_media::stt::SttService;
use fcp_media::tts::TtsService;
use std::collections::HashMap;
use std::sync::Arc;

use crate::config::TtsServerConfig;

/// Shared application state passed to Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: TtsServerConfig,
    /// Named TTS services — each wraps a provider + its own cache.
    /// The default provider is always present under its configured name.
    pub services: HashMap<String, Arc<TtsService>>,
    /// Name of the default provider.
    pub default_provider: String,
    /// Named speech recognition services.
    pub stt: HashMap<String, Arc<SttService>>,
    /// The speech provider a request naming none gets.
    pub stt_default: Option<String>,
    /// Tenants' speech settings, where a database is configured.
    pub speech: Option<Arc<dyn fcp_core::speech::SpeechSettingsStore>>,
    /// What was read, for [`SPEECH_TTL`].
    pub speech_cache: SpeechCache,
}

/// Tenants' settings as last read, and when.
pub type SpeechCache = Arc<
    std::sync::Mutex<
        HashMap<fcp_core::TenantId, (std::time::Instant, Option<fcp_core::speech::TenantSpeech>)>,
    >,
>;

/// How long a tenant's settings are kept here before they are read again:
/// a change reaches calls within this.
pub const SPEECH_TTL: std::time::Duration = std::time::Duration::from_secs(30);

impl AppState {
    /// Get the default TtsService.
    pub fn default_service(&self) -> Option<&Arc<TtsService>> {
        self.services.get(&self.default_provider)
    }

    /// Get a TtsService by provider name, or the default when no provider was specified.
    pub fn service(&self, name: Option<&str>) -> Option<&Arc<TtsService>> {
        match name {
            Some(n) => self.services.get(n),
            None => self.default_service(),
        }
    }

    /// `tenant`'s speech settings (cached), when there is a database and the
    /// tenant has saved some. A read that fails is none: the defaults.
    pub async fn tenant_speech(
        &self,
        tenant: Option<&str>,
    ) -> Option<fcp_core::speech::TenantSpeech> {
        let store = self.speech.as_ref()?;
        let raw = tenant?.trim();
        let tenant: fcp_core::TenantId = raw
            .parse()
            .or_else(|_| format!("tenant-{raw}").parse())
            .ok()?;
        if let Some((at, cached)) = self
            .speech_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&tenant)
        {
            if at.elapsed() < SPEECH_TTL {
                return cached.clone();
            }
        }
        let read = match store.get_tenant_speech(&tenant).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(tenant_id = %tenant, error = %e, "A tenant's speech settings could not be read");
                None
            }
        };
        self.speech_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(tenant, (std::time::Instant::now(), read.clone()));
        read
    }

    /// A speech recognition service by name, or the default.
    pub fn stt_service(&self, name: Option<&str>) -> Option<&Arc<SttService>> {
        self.stt.get(name.or(self.stt_default.as_deref())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_config() -> TtsServerConfig {
        TtsServerConfig {
            service: crate::config::ServiceSection::default(),
            http: crate::config::HttpSection::default(),
            auth: crate::config::AuthSection {
                tokens: vec!["test-token".to_string()],
                allow_unauthenticated: false,
            },
            tts: crate::config::TtsSection {
                default_provider: "primary".to_string(),
                cache_dir: std::env::temp_dir()
                    .join(format!("fcp-tts-state-test-{}", uuid::Uuid::new_v4())),
                cache_max_size_mb: 100,
                default_language: "en-US".to_string(),
                default_voice: None,
                providers: Vec::new(),
            },
            stt: crate::config::SttSection::default(),
            logging: crate::config::LoggingSection::default(),
            cluster: crate::config::ClusterSection::default(),
            storage: crate::config::StorageSection::default(),
        }
    }

    #[tokio::test]
    async fn test_service_lookup_requires_explicit_provider_match() {
        let cache_dir =
            std::env::temp_dir().join(format!("fcp-tts-state-provider-{}", uuid::Uuid::new_v4()));
        let service = Arc::new(
            TtsService::new(fcp_media::tts::TtsConfig {
                provider: fcp_media::tts::TtsProviderConfig::OpenAi {
                    api_key: "test-key".to_string(),
                    model: "tts-1".to_string(),
                },
                cache_dir,
                cache_max_size_mb: 100,
                default_language: "en-US".to_string(),
                default_voice: None,
            })
            .await
            .unwrap(),
        );

        let mut services = HashMap::new();
        services.insert("primary".to_string(), service);

        let state = AppState {
            config: build_config(),
            services,
            default_provider: "primary".to_string(),
            stt: HashMap::new(),
            stt_default: None,
            speech: None,
            speech_cache: Default::default(),
        };

        assert!(state.service(None).is_some());
        assert!(state.service(Some("primary")).is_some());
        assert!(state.service(Some("missing")).is_none());
    }
}
