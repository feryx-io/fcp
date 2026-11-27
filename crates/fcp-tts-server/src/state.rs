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
    /// Tenants' own vendor accounts (`docs/TENANT_SPEECH.md` S3).
    pub accounts: Option<Arc<dyn fcp_core::speech::SpeechAccountStore>>,
    /// A service per account, built when first spoken with and again when
    /// the account changes; keyed by the account's id.
    pub account_services: AccountServices,
    /// A recognizer per account that hears (`docs/TENANT_RECOGNITION.md`
    /// R2), built and rebuilt as the services above.
    pub account_recognizers: AccountRecognizers,
    /// The platform's providers held back from tenants, as last read.
    pub held_back: HeldBack,
    /// Each tenant's speech counted (`docs/TENANT_SPEECH.md` S4), where
    /// there is a database.
    pub usage: Option<Arc<crate::usage::UsageCounter>>,
}

/// When the held-back list was read, and what it said.
pub type HeldBack = Arc<std::sync::Mutex<Option<(std::time::Instant, Vec<String>)>>>;

/// Each account's service, with when it was read and the account's
/// `updated_at` it was built from. Keyed by tenant and account, so a
/// service is only ever handed back to the tenant that owns the account.
pub type AccountServices = Arc<
    tokio::sync::Mutex<
        HashMap<
            (fcp_core::TenantId, uuid::Uuid),
            (std::time::Instant, std::time::SystemTime, Arc<TtsService>),
        >,
    >,
>;

/// Recognizers per tenant account: when each was read, the account's
/// `updated_at`, the service.
pub type AccountRecognizers = Arc<
    tokio::sync::Mutex<
        HashMap<
            (fcp_core::TenantId, uuid::Uuid),
            (std::time::Instant, std::time::SystemTime, Arc<SttService>),
        >,
    >,
>;

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

    /// The platform's providers tenants may not choose (cached). A read
    /// that fails holds nothing back.
    pub async fn held_back(&self) -> Vec<String> {
        let Some(store) = self.speech.as_ref() else {
            return Vec::new();
        };
        if let Some((at, list)) = self
            .held_back
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if at.elapsed() < SPEECH_TTL {
                return list.clone();
            }
        }
        let list = store.held_back_providers().await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "The platform's held-back providers could not be read");
            Vec::new()
        });
        *self.held_back.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((std::time::Instant::now(), list.clone()));
        list
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

    /// `tenant`'s own account `id` as a service, when the tenant has it:
    /// read at most every [`SPEECH_TTL`], rebuilt when it changed. `Err`
    /// says why it cannot speak (no such account, no secret, a bad key).
    pub async fn account_service(
        &self,
        tenant: &str,
        id: uuid::Uuid,
    ) -> Result<Arc<TtsService>, String> {
        let store = self
            .accounts
            .as_ref()
            .ok_or("this TTS server has no database for tenants' accounts")?;
        let tenant: fcp_core::TenantId = tenant
            .parse()
            .or_else(|_| format!("tenant-{tenant}").parse())
            .map_err(|_| "a tenant names the account".to_string())?;
        let mut services = self.account_services.lock().await;
        let key = (tenant, id);
        if let Some((at, _, service)) = services.get(&key) {
            if at.elapsed() < SPEECH_TTL {
                return Ok(Arc::clone(service));
            }
        }
        let account = store
            .get_speech_account(&tenant, &id)
            .await
            .map_err(|e| format!("the account could not be read: {e:#}"))?
            .ok_or("no such account for this tenant")?;
        if let Some((_, updated, service)) = services.get(&key) {
            if *updated == account.updated_at {
                let service = Arc::clone(service);
                services.insert(
                    key,
                    (
                        std::time::Instant::now(),
                        account.updated_at,
                        Arc::clone(&service),
                    ),
                );
                return Ok(service);
            }
        }
        let provider = provider_of(&account)?;
        let service = TtsService::new(fcp_media::tts::TtsConfig {
            provider,
            cache_dir: self
                .config
                .tts
                .cache_dir
                .join("accounts")
                .join(id.to_string()),
            cache_max_size_mb: (self.config.tts.cache_max_size_mb / 10).max(50),
            default_language: self.config.tts.default_language.clone(),
            default_voice: None,
        })
        .await
        .map_err(|e| format!("the account cannot speak: {e:#}"))?;
        let service = Arc::new(service);
        services.insert(
            key,
            (
                std::time::Instant::now(),
                account.updated_at,
                Arc::clone(&service),
            ),
        );
        Ok(service)
    }

    /// A speech recognition service by name, or the default.
    pub fn stt_service(&self, name: Option<&str>) -> Option<&Arc<SttService>> {
        self.stt.get(name.or(self.stt_default.as_deref())?)
    }

    /// `tenant`'s own account `id` as a recognizer, when the tenant has it
    /// and it hears; `Err` says why not. Read at most every [`SPEECH_TTL`]
    /// and rebuilt when the account changed, as [`Self::account_service`].
    pub async fn account_recognizer(
        &self,
        tenant: &str,
        id: uuid::Uuid,
    ) -> Result<Arc<SttService>, String> {
        let store = self
            .accounts
            .as_ref()
            .ok_or("this server has no database for tenants' accounts")?;
        let tenant: fcp_core::TenantId = tenant
            .parse()
            .or_else(|_| format!("tenant-{tenant}").parse())
            .map_err(|_| "a tenant names the account".to_string())?;
        let mut recognizers = self.account_recognizers.lock().await;
        let key = (tenant, id);
        if let Some((at, _, service)) = recognizers.get(&key) {
            if at.elapsed() < SPEECH_TTL {
                return Ok(Arc::clone(service));
            }
        }
        let account = store
            .get_speech_account(&tenant, &id)
            .await
            .map_err(|e| format!("the account could not be read: {e:#}"))?
            .ok_or("no such account for this tenant")?;
        if let Some((_, updated, service)) = recognizers.get(&key) {
            if *updated == account.updated_at {
                let service = Arc::clone(service);
                recognizers.insert(
                    key,
                    (
                        std::time::Instant::now(),
                        account.updated_at,
                        Arc::clone(&service),
                    ),
                );
                return Ok(service);
            }
        }
        let provider = recognizer_of(&account)?;
        let service = SttService::new(fcp_media::stt::SttConfig {
            provider,
            language: self.config.stt.language.clone(),
        })
        .await
        .map_err(|e| format!("the account cannot hear: {e:#}"))?;
        let service = Arc::new(service);
        recognizers.insert(
            key,
            (
                std::time::Instant::now(),
                account.updated_at,
                Arc::clone(&service),
            ),
        );
        Ok(service)
    }
}

/// The recognizer an account hears through (`docs/TENANT_RECOGNITION.md`
/// Q3–Q4): OpenAI's transcription with its key, Deepgram's; the other kinds
/// only speak. The account's `model` is its voice model for OpenAI, so its
/// recognition is OpenAI's default; Deepgram's `model` is its own.
pub fn recognizer_of(
    a: &fcp_core::speech::SpeechAccount,
) -> Result<fcp_media::stt::SttProviderConfig, String> {
    use fcp_core::speech::SpeechAccountKind as K;
    use fcp_media::stt::SttProviderConfig as P;
    let secret = a
        .secret
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or("the account has no key")?;
    Ok(match a.kind {
        K::Openai => P::OpenAi {
            api_key: secret,
            model: "whisper-1".into(),
            base_url: a.endpoint.clone(),
        },
        K::Deepgram => P::Deepgram {
            api_key: secret,
            model: a.model.clone().unwrap_or_else(|| "nova-3".into()),
            base_url: a.endpoint.clone(),
        },
        k => return Err(format!("a {} account does not hear", k.as_str())),
    })
}

/// The provider an account speaks through.
pub fn provider_of(
    a: &fcp_core::speech::SpeechAccount,
) -> Result<fcp_media::tts::TtsProviderConfig, String> {
    use fcp_core::speech::SpeechAccountKind as K;
    use fcp_media::tts::TtsProviderConfig as P;
    let secret = a
        .secret
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or("the account has no key")?;
    let region = || {
        a.region
            .clone()
            .filter(|r| !r.is_empty())
            .ok_or("the account names no region")
    };
    Ok(match a.kind {
        K::Openai => P::OpenAi {
            api_key: secret,
            model: a.model.clone().unwrap_or_else(|| "tts-1".into()),
        },
        K::Elevenlabs => P::ElevenLabs {
            api_key: secret,
            model_id: a.model.clone(),
            endpoint: a.endpoint.clone(),
        },
        K::Azure => P::Azure {
            subscription_key: secret,
            region: region()?,
            endpoint: a.endpoint.clone(),
        },
        K::AwsPolly => P::AwsPolly {
            region: region()?,
            access_key_id: a.access_key_id.clone(),
            secret_access_key: Some(secret),
            engine: a.model.clone().unwrap_or_else(|| "neural".into()),
            endpoint: a.endpoint.clone(),
        },
        K::GoogleCloud => P::GoogleCloud { api_key: secret },
        K::Deepgram => return Err("a Deepgram account hears; it does not speak".into()),
    })
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
            accounts: None,
            account_services: Default::default(),
            account_recognizers: Default::default(),
            held_back: Default::default(),
            usage: None,
        };

        assert!(state.service(None).is_some());
        assert!(state.service(Some("primary")).is_some());
        assert!(state.service(Some("missing")).is_none());
    }
}
