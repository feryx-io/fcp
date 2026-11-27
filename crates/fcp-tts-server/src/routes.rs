use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// POST /v1/synthesize
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SynthesizeRequest {
    pub text: String,
    /// Unset: the tenant's language, else `[tts] default_language`.
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    /// Optional provider name override (uses default if omitted).
    #[serde(default)]
    pub provider: Option<String>,
    /// Whose words these are: what the request leaves unset is the
    /// tenant's speech settings' (`docs/TENANT_SPEECH.md`).
    #[serde(default)]
    pub tenant_id: Option<String>,
}

fn default_language() -> String {
    "en-US".to_string()
}

pub async fn synthesize(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SynthesizeRequest>,
) -> Response {
    // What the request leaves unset: the tenant's, else the server's.
    let tenant = state.tenant_speech(req.tenant_id.as_deref()).await;
    let held_back = match tenant.as_ref().and_then(|t| t.provider.as_ref()) {
        Some(_) => state.held_back().await,
        None => Vec::new(),
    };
    let tenant_provider = tenant
        .as_ref()
        .and_then(|t| t.provider.as_deref())
        // A provider this server does not have is the default's business,
        // as is one the platform holds back from tenants; one of the
        // tenant's own accounts is its own.
        .filter(|p| {
            fcp_core::speech::account_of(p).is_some()
                || (state.services.contains_key(*p) && !held_back.iter().any(|h| h == *p))
        });
    let mut provider_name = req.provider.as_deref().or(tenant_provider);
    let language = req
        .language
        .clone()
        .filter(|l| !l.trim().is_empty())
        .or_else(|| tenant.as_ref().and_then(|t| t.language.clone()))
        .unwrap_or_else(|| state.config.tts.default_language.clone());
    let refuse = |why: String| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": why })),
        )
            .into_response()
    };
    // The tenant's own account (`docs/TENANT_SPEECH.md` S3), only for the
    // tenant that has it. One the tenant chose that cannot speak falls back
    // to the server's default, so a call is never silent; one a request
    // names (a preview) says why.
    // Who speaks, as usage counts it; and whether the default stepped in
    // for an account that could not, whose voice is then not its own.
    let mut spoken_by = provider_name.unwrap_or(&state.default_provider).to_string();
    let mut fell_back = false;
    // A cap reached (`docs/SPEECH_CAPS.md`): the default speaks for the
    // rest of the month, as it does for an account that cannot; a request
    // naming the provider is told. The default itself is never capped.
    if let (Some(usage), Some(tenant_id), Some(p)) =
        (&state.usage, req.tenant_id.as_deref(), provider_name)
    {
        if p != state.default_provider {
            if let Some(reached) =
                usage
                    .caps
                    .reached(tenant_id, p, fcp_core::speech::CapUnit::Characters)
            {
                if req.provider.is_some() {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(serde_json::json!({
                            "error": format!(
                                "This month's speech cap on {p} is reached ({} of {} characters)",
                                reached.used, reached.cap
                            ),
                            "code": "speech_cap_reached",
                            "cap": reached.cap,
                            "used": reached.used,
                        })),
                    )
                        .into_response();
                }
                info!(provider = %p, cap = reached.cap, used = reached.used,
                    "A tenant's speech cap is reached; the default speaks");
                provider_name = None;
                spoken_by = state.default_provider.clone();
                fell_back = true;
            }
        }
    }
    let service: Arc<fcp_media::tts::TtsService> = match provider_name
        .and_then(fcp_core::speech::account_of)
    {
        Some(id) => {
            let named = req.provider.is_some();
            let found = match req.tenant_id.as_deref() {
                Some(t) => state.account_service(t, id).await,
                None => Err("an account is spoken with for its tenant only".to_string()),
            };
            match (found, named, state.default_service()) {
                (Ok(s), _, _) => s,
                (Err(why), true, _) => return refuse(why),
                (Err(why), false, Some(default)) => {
                    warn!(account = %id, why = %why, "A tenant's account cannot speak; the default speaks");
                    spoken_by = state.default_provider.clone();
                    fell_back = true;
                    Arc::clone(default)
                }
                (Err(why), false, None) => return refuse(why),
            }
        }
        None => match state.service(provider_name) {
            Some(s) => Arc::clone(s),
            None => {
                let name = provider_name.unwrap_or(&state.default_provider);
                warn!(provider = %name, "Provider not found");
                return refuse(format!("unknown provider '{name}'"));
            }
        },
    };

    // A tenant's voice belongs to its provider: kept only when the request
    // did not name another.
    let tenant_voice = tenant
        .as_ref()
        .filter(|_| !fell_back)
        .filter(|_| req.provider.is_none() || req.provider.as_deref() == tenant_provider)
        .and_then(|t| t.voice_for(&language));
    let voice = req
        .voice
        .as_deref()
        .or(tenant_voice)
        .or(state.config.tts.default_voice.as_deref());

    match service
        .synthesize_counted(&req.text, Some(&language), voice, None)
        .await
    {
        Ok((path, synthesized)) => {
            let chars = req.text.len();
            if let (Some(usage), Some(tenant)) = (&state.usage, req.tenant_id.as_deref()) {
                usage.count(tenant, &spoken_by, req.text.chars().count(), synthesized);
            }
            info!(
                text_len = chars,
                language = %language,
                voice = ?voice,
                provider = provider_name.unwrap_or(&state.default_provider),
                tenant_id = ?req.tenant_id,
                cached_path = %path.display(),
                "Synthesis complete"
            );

            // Read the WAV file and return as bytes.
            match tokio::fs::read(&path).await {
                Ok(bytes) => {
                    (StatusCode::OK, [(header::CONTENT_TYPE, "audio/wav")], bytes).into_response()
                }
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "Failed to read synthesized audio");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": "failed to read audio file"})),
                    )
                        .into_response()
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "Synthesis failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("{}", e)})),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// GET /v1/voices
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct VoicesQuery {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default = "default_language")]
    pub language: String,
    /// The tenant whose account `provider` may name (`account:<id>`).
    #[serde(default)]
    pub tenant_id: Option<String>,
}

pub async fn list_voices(
    State(state): State<Arc<AppState>>,
    Query(q): Query<VoicesQuery>,
) -> Response {
    let provider_name = q.provider.as_deref();
    let service = match provider_name.and_then(fcp_core::speech::account_of) {
        Some(id) => match q.tenant_id.as_deref() {
            Some(t) => match state.account_service(t, id).await {
                Ok(s) => Some(s),
                Err(why) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": why })),
                    )
                        .into_response()
                }
            },
            None => None,
        },
        None => state.service(provider_name).cloned(),
    };
    let service = match service {
        Some(service) => service,
        None => {
            let name = provider_name.unwrap_or(&state.default_provider);
            warn!(provider = %name, "Provider not found for voice listing");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("unknown provider '{}'", name)})),
            )
                .into_response();
        }
    };

    match service.list_voices(&q.language).await {
        Ok(mut voices) => {
            let logical_provider = provider_name.unwrap_or(&state.default_provider).to_string();
            for voice in &mut voices {
                voice
                    .metadata
                    .entry("logical_provider".to_string())
                    .or_insert_with(|| logical_provider.clone());
            }

            (StatusCode::OK, Json(voices)).into_response()
        }
        Err(error) => {
            let name = provider_name.unwrap_or(&state.default_provider);
            warn!(provider = %name, error = %error, "Failed to list provider voices");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("failed to list voices for provider '{}'", name)})),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// GET /v1/providers
// ---------------------------------------------------------------------------

use fcp_media::tts::ProviderInfo;

/// The server's voice providers and recognizers (`docs/TENANT_RECOGNITION.md`
/// R2), one entry for a name that does both.
pub async fn list_providers(State(state): State<Arc<AppState>>) -> Json<Vec<ProviderInfo>> {
    let mut list: Vec<ProviderInfo> = state
        .services
        .keys()
        .map(|name| ProviderInfo {
            name: name.clone(),
            is_default: name == &state.default_provider,
            speaks: true,
            hears: false,
            is_default_recognizer: false,
        })
        .collect();
    for name in state.stt.keys() {
        let default = state.stt_default.as_deref() == Some(name.as_str())
            || (state.stt_default.is_none() && state.stt.len() == 1);
        match list.iter_mut().find(|p| &p.name == name) {
            Some(p) => {
                p.hears = true;
                p.is_default_recognizer = default;
            }
            None => list.push(ProviderInfo {
                name: name.clone(),
                is_default: false,
                speaks: false,
                hears: true,
                is_default_recognizer: default,
            }),
        }
    }
    Json(list)
}

// ---------------------------------------------------------------------------
// GET /v1/cache/stats
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CacheStats {
    entry_count: usize,
    size_bytes: usize,
    max_size_mb: usize,
}

pub async fn cache_stats(State(state): State<Arc<AppState>>) -> Json<CacheStats> {
    let (count, size) = if let Some(svc) = state.default_service() {
        (svc.cache_entry_count().await, svc.cache_size_bytes().await)
    } else {
        (0, 0)
    };
    Json(CacheStats {
        entry_count: count,
        size_bytes: size,
        max_size_mb: state.config.tts.cache_max_size_mb,
    })
}

// ---------------------------------------------------------------------------
// DELETE /v1/cache
// ---------------------------------------------------------------------------

pub async fn clear_cache(State(state): State<Arc<AppState>>) -> StatusCode {
    for svc in state.services.values() {
        let _ = svc.clear_cache().await;
    }
    info!("TTS cache cleared");
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct HealthResponse {
    service: &'static str,
    status: &'static str,
    providers: Vec<String>,
    cache_entries: usize,
}

pub async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let providers: Vec<String> = state.services.keys().cloned().collect();
    let cache_entries = if let Some(svc) = state.default_service() {
        svc.cache_entry_count().await
    } else {
        0
    };
    Json(HealthResponse {
        service: "fcp-tts",
        status: if state.services.is_empty() {
            "degraded"
        } else {
            "healthy"
        },
        providers,
        cache_entries,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::body::to_bytes;

    pub(crate) async fn test_state() -> Arc<AppState> {
        let cache_dir =
            std::env::temp_dir().join(format!("fcp-tts-routes-test-{}", uuid::Uuid::new_v4()));
        let service = Arc::new(
            fcp_media::tts::TtsService::new(fcp_media::tts::TtsConfig {
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

        let mut services = std::collections::HashMap::new();
        services.insert("primary".to_string(), service);

        Arc::new(AppState {
            config: crate::config::TtsServerConfig {
                service: crate::config::ServiceSection::default(),
                http: crate::config::HttpSection::default(),
                auth: crate::config::AuthSection {
                    tokens: vec!["test-token".to_string()],
                    allow_unauthenticated: false,
                },
                tts: crate::config::TtsSection {
                    default_provider: "primary".to_string(),
                    cache_dir: std::env::temp_dir()
                        .join(format!("fcp-tts-routes-config-{}", uuid::Uuid::new_v4())),
                    cache_max_size_mb: 100,
                    default_language: "en-US".to_string(),
                    default_voice: None,
                    providers: vec![crate::config::ProviderEntry {
                        name: "primary".to_string(),
                        provider_type: "openai".to_string(),
                        engine: None,
                        python_path: None,
                        api_key: Some("test-key".to_string()),
                        model: Some("tts-1".to_string()),
                        device: "auto".to_string(),
                        binary: None,
                        voices_dir: None,
                        voice: None,
                        region: None,
                        access_key_id: None,
                        secret_access_key: None,
                        endpoint: None,
                    }],
                },
                stt: crate::config::SttSection::default(),
                logging: crate::config::LoggingSection::default(),
                cluster: crate::config::ClusterSection::default(),
                storage: crate::config::StorageSection::default(),
            },
            services,
            default_provider: "primary".to_string(),
            stt: std::collections::HashMap::new(),
            stt_default: None,
            speech: None,
            speech_cache: Default::default(),
            accounts: None,
            account_services: Default::default(),
            account_recognizers: Default::default(),
            held_back: Default::default(),
            usage: None,
        })
    }

    #[tokio::test]
    async fn test_synthesize_rejects_unknown_provider() {
        let response = synthesize(
            State(test_state().await),
            Json(SynthesizeRequest {
                text: "hello".to_string(),
                language: Some("en-US".to_string()),
                voice: None,
                provider: Some("missing".to_string()),
                tenant_id: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "unknown provider 'missing'");
    }

    #[tokio::test]
    async fn test_list_voices_returns_real_provider_voices() {
        let response = list_voices(
            State(test_state().await),
            Query(VoicesQuery {
                provider: None,
                language: "en-US".to_string(),
                tenant_id: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let voices: Vec<fcp_media::tts::VoiceInfo> = serde_json::from_slice(&body).unwrap();
        assert!(!voices.is_empty());
        assert!(voices.iter().any(|voice| voice.name == "alloy"));
        assert!(voices
            .iter()
            .all(|voice| voice.metadata.get("logical_provider") == Some(&"primary".to_string())));
    }

    /// A tenant's speech settings in memory.
    pub(crate) struct Settings(
        pub(crate) std::collections::HashMap<fcp_core::TenantId, fcp_core::speech::TenantSpeech>,
    );

    #[async_trait::async_trait]
    impl fcp_core::speech::SpeechSettingsStore for Settings {
        async fn get_tenant_speech(
            &self,
            tenant_id: &fcp_core::TenantId,
        ) -> anyhow::Result<Option<fcp_core::speech::TenantSpeech>> {
            Ok(self.0.get(tenant_id).cloned())
        }
        async fn put_tenant_speech(
            &self,
            _: &fcp_core::speech::TenantSpeech,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_tenant_speech(&self, _: &fcp_core::TenantId) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    /// A provider's service that answers a WAV and keeps what it was asked.
    async fn heard_by(
        name: &'static str,
        heard: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    ) -> Arc<fcp_media::tts::TtsService> {
        let router = axum::Router::new().route(
            "/v1/synthesize",
            axum::routing::post(move |Json(body): Json<serde_json::Value>| {
                let heard = Arc::clone(&heard);
                async move {
                    heard.lock().unwrap().push((name.to_string(), body));
                    let mut wav = b"RIFF\x24\x00\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00\x40\x1f\x00\x00\x80\x3e\x00\x00\x02\x00\x10\x00data\x00\x00\x00\x00".to_vec();
                    wav.truncate(44);
                    wav
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Arc::new(
            fcp_media::tts::TtsService::new(fcp_media::tts::TtsConfig {
                provider: fcp_media::tts::TtsProviderConfig::Remote {
                    url,
                    auth_token: "t".into(),
                    provider: None,
                    tenant_id: None,
                },
                cache_dir: std::env::temp_dir()
                    .join(format!("fcp-tts-tenant-{name}-{}", uuid::Uuid::new_v4())),
                cache_max_size_mb: 10,
                default_language: "en-US".into(),
                default_voice: None,
            })
            .await
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn a_tenants_settings_fill_what_a_request_leaves_unset() {
        let heard: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>> = Arc::default();
        let base = test_state().await;
        let mut services = std::collections::HashMap::new();
        services.insert(
            "primary".to_string(),
            heard_by("primary", Arc::clone(&heard)).await,
        );
        services.insert("alt".to_string(), heard_by("alt", Arc::clone(&heard)).await);
        let a = fcp_core::TenantId::new();
        let ghost = fcp_core::TenantId::new();
        let mut speech = fcp_core::speech::TenantSpeech::defaults(a);
        speech.provider = Some("alt".into());
        speech.voice = Some("lucie".into());
        speech.language = Some("fr-CA".into());
        speech.voices.insert("es".into(), "davefx".into());
        let mut other = fcp_core::speech::TenantSpeech::defaults(ghost);
        other.provider = Some("gone".into());
        let state = Arc::new(AppState {
            services,
            speech: Some(Arc::new(Settings(
                [(a, speech), (ghost, other)].into_iter().collect(),
            ))),
            speech_cache: Default::default(),
            accounts: None,
            account_services: Default::default(),
            account_recognizers: Default::default(),
            held_back: Default::default(),
            usage: None,
            config: base.config.clone(),
            default_provider: base.default_provider.clone(),
            stt: Default::default(),
            stt_default: None,
        });
        let say = |text: &str,
                   tenant: Option<String>,
                   language: Option<&str>,
                   voice: Option<&str>,
                   provider: Option<&str>| {
            let state = Arc::clone(&state);
            let req = SynthesizeRequest {
                text: text.to_string(),
                language: language.map(String::from),
                voice: voice.map(String::from),
                provider: provider.map(String::from),
                tenant_id: tenant,
            };
            async move { synthesize(State(state), Json(req)).await.status() }
        };
        let tenant = || Some(a.as_uuid().to_string());

        assert_eq!(say("one", tenant(), None, None, None).await, StatusCode::OK);
        assert_eq!(
            say("two", tenant(), Some("es-ES"), None, None).await,
            StatusCode::OK
        );
        assert_eq!(
            say("three", tenant(), None, Some("own"), None).await,
            StatusCode::OK
        );
        assert_eq!(
            say(
                "four",
                Some(fcp_core::TenantId::new().to_string()),
                None,
                None,
                None
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            say("five", tenant(), None, None, Some("primary")).await,
            StatusCode::OK
        );
        assert_eq!(
            say("six", Some(ghost.to_string()), None, None, None).await,
            StatusCode::OK
        );

        let heard = heard.lock().unwrap().clone();
        let at = |i: usize| {
            (
                heard[i].0.as_str(),
                heard[i].1["voice"].as_str(),
                heard[i].1["language"].as_str(),
            )
        };
        assert_eq!(heard.len(), 6, "{heard:?}");
        assert_eq!(
            at(0),
            ("alt", Some("lucie"), Some("fr-CA")),
            "the tenant's provider, voice and language"
        );
        assert_eq!(
            at(1),
            ("alt", Some("davefx"), Some("es-ES")),
            "its voice for Spanish"
        );
        assert_eq!(
            at(2),
            ("alt", Some("own"), Some("fr-CA")),
            "the request's voice wins"
        );
        assert_eq!(
            at(3),
            ("primary", None, Some("en-US")),
            "a tenant with no settings: the server's"
        );
        assert_eq!(
            at(4),
            ("primary", None, Some("fr-CA")),
            "another provider named: not the tenant's voice"
        );
        assert_eq!(
            at(5),
            ("primary", None, Some("en-US")),
            "a provider this server lacks: its default"
        );
    }

    /// A tenant's accounts in memory.
    pub(crate) struct Accounts(pub(crate) Vec<fcp_core::speech::SpeechAccount>);

    #[async_trait::async_trait]
    impl fcp_core::speech::SpeechAccountStore for Accounts {
        async fn list_speech_accounts(
            &self,
            t: &fcp_core::TenantId,
        ) -> anyhow::Result<Vec<fcp_core::speech::SpeechAccount>> {
            Ok(self
                .0
                .iter()
                .filter(|a| &a.tenant_id == t)
                .cloned()
                .collect())
        }
        async fn get_speech_account(
            &self,
            t: &fcp_core::TenantId,
            id: &uuid::Uuid,
        ) -> anyhow::Result<Option<fcp_core::speech::SpeechAccount>> {
            Ok(self
                .0
                .iter()
                .find(|a| &a.tenant_id == t && &a.id == id)
                .cloned())
        }
        async fn put_speech_account(
            &self,
            _: &fcp_core::speech::SpeechAccount,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_speech_account(
            &self,
            _: &fcp_core::TenantId,
            _: &uuid::Uuid,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    #[tokio::test]
    async fn a_tenants_own_account_speaks_for_it_alone() {
        let vendor = fcp_testkit::tts::StandInTts::elevenlabs("xi-their-key").await;
        let heard: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>> = Arc::default();
        let base = test_state().await;
        let mut services = std::collections::HashMap::new();
        services.insert(
            "primary".to_string(),
            heard_by("primary", Arc::clone(&heard)).await,
        );
        let a = fcp_core::TenantId::new();
        let b = fcp_core::TenantId::new();
        let now = std::time::SystemTime::now();
        let account = fcp_core::speech::SpeechAccount {
            id: uuid::Uuid::new_v4(),
            tenant_id: a,
            name: "ours".into(),
            kind: fcp_core::speech::SpeechAccountKind::Elevenlabs,
            region: None,
            model: None,
            endpoint: Some(vendor.url().to_string()),
            access_key_id: None,
            secret: Some("xi-their-key".into()),
            price_per_million: None,
            price_per_minute: None,
            created_at: now,
            updated_at: now,
        };
        let named = format!("account:{}", account.id);
        let mut settings = fcp_core::speech::TenantSpeech::defaults(a);
        settings.provider = Some(named.clone());
        settings.voice = Some("21m00Tcm4TlvDq8ikWAM".into());
        let mut b_settings = fcp_core::speech::TenantSpeech::defaults(b);
        b_settings.provider = Some(named.clone());
        let state = Arc::new(AppState {
            services,
            speech: Some(Arc::new(Settings(
                [(a, settings), (b, b_settings)].into_iter().collect(),
            ))),
            speech_cache: Default::default(),
            accounts: Some(Arc::new(Accounts(vec![account.clone()]))),
            account_services: Default::default(),
            account_recognizers: Default::default(),
            held_back: Default::default(),
            usage: None,
            config: base.config.clone(),
            default_provider: base.default_provider.clone(),
            stt: Default::default(),
            stt_default: None,
        });
        let say = |text: &str, tenant: fcp_core::TenantId, provider: Option<String>| {
            let state = Arc::clone(&state);
            let req = SynthesizeRequest {
                text: text.to_string(),
                language: None,
                voice: None,
                provider,
                tenant_id: Some(tenant.as_uuid().to_string()),
            };
            async move { synthesize(State(state), Json(req)).await.status() }
        };

        // A's chosen account speaks A's words, with A's key and voice.
        assert_eq!(say("hello from a", a, None).await, StatusCode::OK);
        let sent = vendor.requests_to("/v1/text-to-speech");
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(
            sent[0].path.ends_with("21m00Tcm4TlvDq8ikWAM"),
            "{}",
            sent[0].path
        );
        assert!(
            heard.lock().unwrap().is_empty(),
            "not the platform's provider"
        );

        // B names A's account: not B's, so the platform's default speaks.
        assert_eq!(say("hello from b", b, None).await, StatusCode::OK);
        assert_eq!(vendor.requests_to("/v1/text-to-speech").len(), 1);
        assert_eq!(heard.lock().unwrap().len(), 1);

        // B naming it in a request (a preview) is refused.
        assert_eq!(
            say("preview from b", b, Some(named)).await,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn a_provider_held_back_from_tenants_is_not_theirs_to_choose() {
        let heard: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>> = Arc::default();
        let base = test_state().await;
        let mut services = std::collections::HashMap::new();
        services.insert(
            "primary".to_string(),
            heard_by("primary", Arc::clone(&heard)).await,
        );
        services.insert(
            "costly".to_string(),
            heard_by("costly", Arc::clone(&heard)).await,
        );
        let a = fcp_core::TenantId::new();
        let mut speech = fcp_core::speech::TenantSpeech::defaults(a);
        speech.provider = Some("costly".into());
        let state = Arc::new(AppState {
            services,
            speech: Some(Arc::new(Settings([(a, speech)].into_iter().collect()))),
            speech_cache: Default::default(),
            accounts: None,
            account_services: Default::default(),
            account_recognizers: Default::default(),
            held_back: Arc::new(std::sync::Mutex::new(Some((
                std::time::Instant::now(),
                vec!["costly".to_string()],
            )))),
            usage: None,
            config: base.config.clone(),
            default_provider: base.default_provider.clone(),
            stt: Default::default(),
            stt_default: None,
        });
        let req = SynthesizeRequest {
            text: "held back".into(),
            language: None,
            voice: None,
            provider: None,
            tenant_id: Some(a.as_uuid().to_string()),
        };
        assert_eq!(
            synthesize(State(state), Json(req)).await.status(),
            StatusCode::OK
        );
        let heard = heard.lock().unwrap();
        assert_eq!(heard.len(), 1);
        assert_eq!(
            heard[0].0, "primary",
            "the default speaks, not the held-back choice"
        );
    }
    /// Usage and caps in memory (`docs/SPEECH_CAPS.md`).
    #[derive(Default)]
    struct Capped(std::sync::Mutex<Vec<fcp_core::speech::SpeechCap>>);

    #[async_trait::async_trait]
    impl fcp_core::speech::SpeechUsageStore for Capped {
        async fn add_speech_usage(
            &self,
            _: &[fcp_core::speech::SpeechUsage],
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn speech_usage(
            &self,
            _: &fcp_core::TenantId,
            _: std::time::SystemTime,
            _: std::time::SystemTime,
        ) -> anyhow::Result<Vec<fcp_core::speech::SpeechUsage>> {
            Ok(Vec::new())
        }
        async fn all_speech_caps(&self) -> anyhow::Result<Vec<fcp_core::speech::SpeechCap>> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    #[tokio::test]
    async fn at_its_cap_a_tenant_is_spoken_for_by_the_default() {
        let heard: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>> = Arc::default();
        let base = test_state().await;
        let mut services = std::collections::HashMap::new();
        services.insert(
            "primary".to_string(),
            heard_by("primary", Arc::clone(&heard)).await,
        );
        services.insert("alt".to_string(), heard_by("alt", Arc::clone(&heard)).await);
        let a = fcp_core::TenantId::new();
        let mut speech = fcp_core::speech::TenantSpeech::defaults(a);
        speech.provider = Some("alt".into());
        speech.voice = Some("lucie".into());
        let store = Arc::new(Capped::default());
        store.0.lock().unwrap().extend([
            // The platform's default cap on alt; one on the default is
            // never enforced.
            fcp_core::speech::SpeechCap {
                tenant_id: None,
                provider: "alt".into(),
                kind: fcp_core::speech::SpeechCapKind::Platform,
                unit: fcp_core::speech::CapUnit::Characters,
                monthly_characters: Some(10),
            },
            fcp_core::speech::SpeechCap {
                tenant_id: None,
                provider: "primary".into(),
                kind: fcp_core::speech::SpeechCapKind::Platform,
                unit: fcp_core::speech::CapUnit::Characters,
                monthly_characters: Some(0),
            },
        ]);
        let usage = crate::usage::UsageCounter::new(store.clone());
        usage.caps.refresh().await;
        let state = Arc::new(AppState {
            services,
            speech: Some(Arc::new(Settings([(a, speech)].into_iter().collect()))),
            speech_cache: Default::default(),
            accounts: None,
            account_services: Default::default(),
            account_recognizers: Default::default(),
            held_back: Default::default(),
            usage: Some(usage),
            config: base.config.clone(),
            default_provider: base.default_provider.clone(),
            stt: Default::default(),
            stt_default: None,
        });
        let say = |text: &str, tenant: Option<String>, provider: Option<&str>| {
            let state = Arc::clone(&state);
            let req = SynthesizeRequest {
                text: text.to_string(),
                language: None,
                voice: None,
                provider: provider.map(String::from),
                tenant_id: tenant,
            };
            async move {
                let r = synthesize(State(state), Json(req)).await;
                let status = r.status();
                let body = to_bytes(r.into_body(), usize::MAX).await.unwrap();
                (status, body)
            }
        };
        let tenant = || Some(a.to_string());

        // Under the cap: the tenant's provider and voice.
        assert_eq!(say("hello world!", tenant(), None).await.0, StatusCode::OK);
        // Twelve characters spoken: at the cap, the default speaks, in its
        // own voice.
        assert_eq!(say("again", tenant(), None).await.0, StatusCode::OK);
        // Naming the capped provider is refused and says why.
        let (status, body) = say("named", tenant(), Some("alt")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "speech_cap_reached");
        assert_eq!(
            (body["cap"].as_u64(), body["used"].as_u64()),
            (Some(10), Some(12))
        );
        // Another tenant, and a request without one, are not held to it.
        assert_eq!(
            say(
                "theirs",
                Some(fcp_core::TenantId::new().to_string()),
                Some("alt")
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(say("nobody's", None, Some("alt")).await.0, StatusCode::OK);
        // The default, capped at nothing, still speaks.
        assert_eq!(
            say("primary", tenant(), Some("primary")).await.0,
            StatusCode::OK
        );

        let heard = heard.lock().unwrap().clone();
        let who: Vec<(&str, Option<&str>)> = heard
            .iter()
            .map(|(n, b)| (n.as_str(), b["voice"].as_str()))
            .collect();
        assert_eq!(
            who,
            [
                ("alt", Some("lucie")),
                ("primary", None),
                ("alt", None),
                ("alt", None),
                ("primary", None),
            ]
        );
    }
}
