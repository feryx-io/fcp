//! Speech recognition: `POST /v1/transcribe` for a recording and
//! `GET /v1/transcribe/stream` for a live stream, the protocol
//! `fcp_media::stt::providers::remote` speaks
//! (`docs/DEVELOPER_SURFACE.md` phase 6).

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use fcp_media::stt::{self, SttOptions, SttService};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::state::AppState;

/// What a recognition asks for, in the query.
#[derive(Debug, Default, Deserialize)]
pub struct TranscribeQuery {
    #[serde(default)]
    pub language: Option<String>,
    /// Comma-separated.
    #[serde(default)]
    pub hints: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub interim_results: bool,
    #[serde(default)]
    pub speech_timeout_ms: Option<u32>,
    /// Whose speech it is (`docs/TENANT_RECOGNITION.md` R1).
    #[serde(default)]
    pub tenant_id: Option<String>,
}

impl TranscribeQuery {
    fn options(&self) -> SttOptions {
        SttOptions {
            language: self.language.clone(),
            hints: self
                .hints
                .as_deref()
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|h| !h.is_empty())
                .map(String::from)
                .collect(),
            interim_results: self.interim_results,
            speech_timeout_ms: self.speech_timeout_ms,
            tenant_id: self.tenant_id.clone(),
            // Chosen here; a provider behind this one is not told a name.
            provider: None,
        }
    }

    /// The options, with what the request left unset taken from its
    /// tenant: the language it is heard in (`docs/TENANT_RECOGNITION.md`);
    /// else the recognizer's default, always told to a provider behind it.
    fn resolved(
        &self,
        tenant: Option<&fcp_core::speech::TenantSpeech>,
        service: &SttService,
    ) -> SttOptions {
        let mut options = self.options();
        if options.language.is_none() {
            options.language = Some(
                tenant
                    .and_then(|t| t.heard_language())
                    .unwrap_or(service.language())
                    .to_string(),
            );
        }
        options
    }
}

fn error(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// What hears a request, the tenant's settings, and the name its usage is
/// counted under.
struct Heard {
    service: Arc<SttService>,
    tenant: Option<fcp_core::speech::TenantSpeech>,
    by: String,
}

/// What hears a request (`docs/TENANT_RECOGNITION.md` R2–R3): the
/// recognizer it names; else the tenant's choice — one of the server's that
/// is offered, or the tenant's own account that hears — falling back to the
/// server's default when that cannot hear or has reached its minute cap, so
/// a Gather is never deaf. A recognizer a request names that cannot hear
/// says why; one at its cap is 429 `speech_cap_reached`.
async fn service(state: &AppState, query: &TranscribeQuery) -> Result<Heard, Box<Response>> {
    let tenant = state.tenant_speech(query.tenant_id.as_deref()).await;
    let named = query.provider.as_deref().filter(|p| !p.is_empty());
    let tenants_choice = tenant.as_ref().and_then(|t| t.recognizer.clone());
    let chosen = named.or(tenants_choice.as_deref());
    let refuse = |why: String| Err(Box::new(error(StatusCode::BAD_REQUEST, why)));
    let default_name = state.stt_default.clone().or_else(|| {
        (state.stt.len() == 1)
            .then(|| state.stt.keys().next().cloned())
            .flatten()
    });
    if let Some(p) = chosen.filter(|p| Some(*p) != default_name.as_deref()) {
        // At its cap this month: the default hears instead.
        let reached = match (&state.usage, query.tenant_id.as_deref()) {
            (Some(usage), Some(t)) => usage.caps.reached(t, p, fcp_core::speech::CapUnit::Minutes),
            _ => None,
        };
        if let Some(r) = reached {
            if named.is_some() {
                return Err(Box::new(
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(serde_json::json!({
                            "error": format!(
                                "This month's recognition cap on {p} is reached ({} of {} minutes)",
                                r.used / 60_000,
                                r.cap
                            ),
                            "code": "speech_cap_reached",
                            "cap": r.cap,
                            "used": r.used / 60_000,
                        })),
                    )
                        .into_response(),
                ));
            }
            info!(provider = %p, cap = r.cap, "A tenant's recognition cap is reached; the default hears");
        } else {
            match fcp_core::speech::account_of(p) {
                Some(id) => match query.tenant_id.as_deref() {
                    Some(t) => match state.account_recognizer(t, id).await {
                        Ok(service) => {
                            return Ok(Heard {
                                service,
                                tenant,
                                by: p.to_string(),
                            })
                        }
                        Err(why) if named.is_some() => return refuse(why),
                        Err(why) => {
                            warn!(account = %id, why = %why, "A tenant's account cannot hear; the default hears");
                        }
                    },
                    None if named.is_some() => {
                        return refuse("an account hears for its tenant only".into())
                    }
                    None => {}
                },
                None => {
                    // The tenant's own choice of the platform's is held to
                    // what the platform offers; a named one (a preview the
                    // admin checked) is not.
                    let offered =
                        named.is_some() || !state.held_back().await.iter().any(|h| h == p);
                    match state.stt.get(p) {
                        Some(service) if offered => {
                            return Ok(Heard {
                                service: Arc::clone(service),
                                tenant,
                                by: p.to_string(),
                            })
                        }
                        _ if named.is_some() => {
                            return refuse(format!("unknown speech provider '{p}'"))
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    match (state.stt_service(None), default_name) {
        (Some(service), Some(by)) => Ok(Heard {
            service: Arc::clone(service),
            tenant,
            by,
        }),
        _ => Err(Box::new(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "speech recognition is not configured ([stt.providers])".into(),
        ))),
    }
}

/// What a recognition heard, counted when it ends however it ends
/// (`docs/TENANT_RECOGNITION.md` R3).
struct Tally {
    usage: Option<Arc<crate::usage::UsageCounter>>,
    tenant: Option<String>,
    by: String,
    samples: u64,
}

impl Drop for Tally {
    fn drop(&mut self) {
        if let (Some(usage), Some(tenant)) = (&self.usage, &self.tenant) {
            if self.samples > 0 {
                let ms = self.samples * 1000 / u64::from(stt::STT_SAMPLE_RATE);
                usage.count_heard(tenant, &self.by, ms);
            }
        }
    }
}

/// `POST /v1/transcribe`: a WAV file in the body, one final transcript
/// out.
pub async fn transcribe(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TranscribeQuery>,
    body: Bytes,
) -> Response {
    let Heard {
        service,
        tenant,
        by,
    } = match service(&state, &query).await {
        Ok(h) => h,
        Err(refused) => return *refused,
    };
    let pcm = match tokio::task::spawn_blocking(move || stt::wav_bytes_16k_mono(&body)).await {
        Ok(Ok(pcm)) => pcm,
        Ok(Err(e)) => return error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let seconds = pcm.len() as f32 / stt::STT_SAMPLE_RATE as f32;
    let options = query.resolved(tenant.as_ref(), &service);
    match service.transcribe_pcm(&pcm, &options).await {
        Ok(transcript) => {
            drop(Tally {
                usage: state.usage.clone(),
                tenant: query.tenant_id.clone(),
                by,
                samples: pcm.len() as u64,
            });
            info!(
                provider = %service.provider_name(),
                seconds,
                chars = transcript.text.len(),
                "transcribed"
            );
            Json(transcript).into_response()
        }
        Err(e) => {
            warn!(provider = %service.provider_name(), error = %e, "transcription failed");
            error(StatusCode::BAD_GATEWAY, format!("{e:#}"))
        }
    }
}

/// `GET /v1/transcribe/stream`: binary messages of 16 kHz mono
/// little-endian PCM in, a text `{"type":"finish"}` to end the audio,
/// transcripts out as text messages of JSON; closed after the last.
pub async fn transcribe_stream(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TranscribeQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Heard {
        service,
        tenant,
        by,
    } = match service(&state, &query).await {
        Ok(h) => h,
        Err(refused) => return *refused,
    };
    let options = query.resolved(tenant.as_ref(), &service);
    let tally = Tally {
        usage: state.usage.clone(),
        tenant: query.tenant_id.clone(),
        by,
        samples: 0,
    };
    upgrade.on_upgrade(move |socket| stream_session(socket, service, options, tally))
}

async fn stream_session(
    socket: WebSocket,
    service: Arc<SttService>,
    options: SttOptions,
    mut tally: Tally,
) {
    let (mut sink, mut source) = socket.split();
    let mut stream = match service.stream(&options).await {
        Ok(stream) => stream,
        Err(e) => {
            warn!(provider = %service.provider_name(), error = %e, "a stream could not start");
            let _ = sink
                .send(Message::Text(
                    serde_json::json!({ "error": format!("{e:#}") }).to_string(),
                ))
                .await;
            let _ = sink.send(Message::Close(None)).await;
            return;
        }
    };
    debug!(provider = %service.provider_name(), "a transcription stream opened");
    let mut finished = false;
    loop {
        tokio::select! {
            message = source.next(), if !finished => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    let pcm = stt::pcm_from_le_bytes(&bytes);
                    tally.samples += pcm.len() as u64;
                    if stream.send(pcm).is_err() {
                        stream.finish();
                        finished = true;
                    }
                }
                Some(Ok(Message::Text(text))) if text.contains("finish") => {
                    stream.finish();
                    finished = true;
                }
                Some(Ok(_)) => {}
                // The client went away: nobody to send results to.
                Some(Err(_)) | None => return,
            },
            transcript = stream.next() => match transcript {
                Some(transcript) => {
                    let text = serde_json::to_string(&transcript).unwrap_or_default();
                    if sink.send(Message::Text(text)).await.is_err() {
                        return;
                    }
                }
                None => {
                    let _ = sink.send(Message::Close(None)).await;
                    return;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::tests::{test_state, Settings};
    use fcp_media::stt::{wav_bytes, SttConfig, SttProviderConfig};
    use fcp_testkit::stt::ScriptedStt;
    use std::time::Duration;

    /// `docs/TENANT_RECOGNITION.md` R1: a request naming its tenant and no
    /// language is heard in the tenant's language; one naming a language,
    /// or no tenant, as before.
    #[tokio::test]
    async fn a_tenant_is_heard_in_its_own_language() {
        let scripted = ScriptedStt::start("up", &[(700, "oui")]).await;
        let service = Arc::new(
            SttService::new(SttConfig {
                provider: SttProviderConfig::Remote {
                    url: scripted.url().into(),
                    auth_token: "up".into(),
                    provider: None,
                },
                language: "en-US".into(),
            })
            .await
            .unwrap(),
        );
        let a = fcp_core::TenantId::new();
        let mut speech = fcp_core::speech::TenantSpeech::defaults(a);
        speech.language = Some("fr-CA".into());
        let base = test_state().await;
        let state = Arc::new(AppState {
            services: base.services.clone(),
            speech: Some(Arc::new(Settings([(a, speech)].into_iter().collect()))),
            speech_cache: Default::default(),
            accounts: None,
            account_services: Default::default(),
            account_recognizers: Default::default(),
            held_back: Default::default(),
            usage: None,
            config: base.config.clone(),
            default_provider: base.default_provider.clone(),
            stt: [("scripted".to_string(), service)].into_iter().collect(),
            stt_default: Some("scripted".into()),
        });
        let tone: Vec<i16> = (0..16 * 400)
            .map(|i| {
                (9000.0 * (2.0 * std::f64::consts::PI * 700.0 * i as f64 / 16_000.0).sin()) as i16
            })
            .collect();
        let hear = |tenant: Option<String>, language: Option<&str>| {
            let state = Arc::clone(&state);
            let query = TranscribeQuery {
                tenant_id: tenant,
                language: language.map(String::from),
                ..Default::default()
            };
            let body = Bytes::from(wav_bytes(&tone));
            async move { transcribe(State(state), Query(query), body).await.status() }
        };
        assert_eq!(
            hear(Some(a.as_uuid().to_string()), None).await,
            StatusCode::OK
        );
        assert_eq!(
            hear(Some(a.to_string()), Some("es-ES")).await,
            StatusCode::OK
        );
        assert_eq!(hear(None, None).await, StatusCode::OK);
        assert_eq!(
            hear(Some(fcp_core::TenantId::new().to_string()), None).await,
            StatusCode::OK
        );
        let languages: Vec<String> = scripted
            .wait_for(4, Duration::from_secs(5))
            .await
            .iter()
            .map(|r| r.query.get("language").cloned().unwrap_or_default())
            .collect();
        assert_eq!(languages, ["fr-CA", "es-ES", "en-US", "en-US"]);
    }

    /// `docs/TENANT_RECOGNITION.md` R2: the tenant's recognizer hears it —
    /// one of the server's, or its own account that hears — in its
    /// recognition language; one held back, or an account that cannot
    /// hear, leaves the default to hear; a request naming one that cannot
    /// is refused.
    #[tokio::test]
    async fn a_tenant_is_heard_by_the_recognizer_it_chose() {
        use axum::routing::post;
        let primary = ScriptedStt::start("up", &[(700, "primary")]).await;
        let alt = ScriptedStt::start("up", &[(700, "alt")]).await;
        // OpenAI's transcription, as the tenant's own account reaches it.
        let router = axum::Router::new().route(
            "/v1/audio/transcriptions",
            post(|| async { Json(serde_json::json!({ "text": "the account" })) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let openai = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let remote = |url: &str| {
            let url = url.to_string();
            async move {
                Arc::new(
                    SttService::new(SttConfig {
                        provider: SttProviderConfig::Remote {
                            url,
                            auth_token: "up".into(),
                            provider: None,
                        },
                        language: "en-US".into(),
                    })
                    .await
                    .unwrap(),
                )
            }
        };
        let now = std::time::SystemTime::now();
        // From JSON, so a field the account gains later takes its default.
        let account =
            |t: fcp_core::TenantId, kind: fcp_core::speech::SpeechAccountKind, id: uuid::Uuid| {
                serde_json::from_value::<fcp_core::speech::SpeechAccount>(serde_json::json!({
                    "id": id,
                    "tenant_id": t,
                    "name": "ours",
                    "kind": kind,
                    "endpoint": openai,
                    "secret": "sk",
                    "created_at": now,
                    "updated_at": now,
                }))
                .unwrap()
            };
        let (a, b, c, d) = (
            fcp_core::TenantId::new(),
            fcp_core::TenantId::new(),
            fcp_core::TenantId::new(),
            fcp_core::TenantId::new(),
        );
        let (hears, speaks) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let settings = |t, recognizer: &str, language: Option<&str>| {
            let mut s = fcp_core::speech::TenantSpeech::defaults(t);
            s.recognizer = Some(recognizer.into());
            s.language = Some("de-DE".into());
            s.recognition_language = language.map(String::from);
            (t, s)
        };
        let base = test_state().await;
        let state = Arc::new(AppState {
            services: base.services.clone(),
            speech: Some(Arc::new(Settings(
                [
                    settings(a, "alt", Some("fr-CA")),
                    settings(b, &format!("account:{hears}"), None),
                    settings(c, &format!("account:{speaks}"), None),
                    settings(d, "held", None),
                ]
                .into_iter()
                .collect(),
            ))),
            speech_cache: Default::default(),
            accounts: Some(Arc::new(crate::routes::tests::Accounts(vec![
                account(b, fcp_core::speech::SpeechAccountKind::Openai, hears),
                account(c, fcp_core::speech::SpeechAccountKind::Elevenlabs, speaks),
            ]))),
            account_services: Default::default(),
            account_recognizers: Default::default(),
            held_back: Arc::new(std::sync::Mutex::new(Some((
                std::time::Instant::now(),
                vec!["held".to_string()],
            )))),
            usage: None,
            config: base.config.clone(),
            default_provider: base.default_provider.clone(),
            stt: [
                ("primary".to_string(), remote(primary.url()).await),
                ("alt".to_string(), remote(alt.url()).await),
                ("held".to_string(), remote(alt.url()).await),
            ]
            .into_iter()
            .collect(),
            stt_default: Some("primary".into()),
        });
        let tone: Vec<i16> = (0..16 * 400)
            .map(|i| {
                (9000.0 * (2.0 * std::f64::consts::PI * 700.0 * i as f64 / 16_000.0).sin()) as i16
            })
            .collect();
        let hear = |tenant: fcp_core::TenantId, provider: Option<&str>| {
            let state = Arc::clone(&state);
            let query = TranscribeQuery {
                tenant_id: Some(tenant.to_string()),
                provider: provider.map(String::from),
                ..Default::default()
            };
            let body = Bytes::from(wav_bytes(&tone));
            async move {
                let r = transcribe(State(state), Query(query), body).await;
                let status = r.status();
                let body = axum::body::to_bytes(r.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                (status, v["text"].as_str().unwrap_or_default().to_string())
            }
        };
        // Its own pick of the server's, in its recognition language.
        assert_eq!(hear(a, None).await, (StatusCode::OK, "alt".into()));
        // Its own account that hears.
        assert_eq!(hear(b, None).await, (StatusCode::OK, "the account".into()));
        // An account that only speaks, and one held back: the default.
        assert_eq!(hear(c, None).await, (StatusCode::OK, "primary".into()));
        assert_eq!(hear(d, None).await, (StatusCode::OK, "primary".into()));
        // Named and unable to hear: refused.
        let named = format!("account:{speaks}");
        assert_eq!(hear(c, Some(&named)).await.0, StatusCode::BAD_REQUEST);
        let languages: Vec<String> = alt
            .requests()
            .iter()
            .map(|r| r.query.get("language").cloned().unwrap_or_default())
            .collect();
        assert_eq!(languages, ["fr-CA"]);
        let languages: Vec<String> = primary
            .requests()
            .iter()
            .map(|r| r.query.get("language").cloned().unwrap_or_default())
            .collect();
        // No recognition language of their own: their speech language.
        assert_eq!(languages, ["de-DE", "de-DE"]);
    }

    /// `docs/TENANT_RECOGNITION.md` R3: what is heard is counted in
    /// milliseconds under the recognizer that heard it; at its minute cap a
    /// tenant's recognizer gives way to the default, and naming it is 429.
    #[tokio::test]
    async fn a_recognizer_at_its_minute_cap_gives_way_to_the_default() {
        use fcp_core::speech::{CapUnit, SpeechCap, SpeechCapKind, SpeechUsage, SpeechUsageStore};
        #[derive(Default)]
        struct Kept(std::sync::Mutex<Vec<SpeechUsage>>);
        #[async_trait::async_trait]
        impl SpeechUsageStore for Kept {
            async fn add_speech_usage(&self, u: &[SpeechUsage]) -> anyhow::Result<()> {
                self.0.lock().unwrap().extend_from_slice(u);
                Ok(())
            }
            async fn speech_usage(
                &self,
                _: &fcp_core::TenantId,
                _: std::time::SystemTime,
                _: std::time::SystemTime,
            ) -> anyhow::Result<Vec<SpeechUsage>> {
                Ok(Vec::new())
            }
            async fn all_speech_caps(&self) -> anyhow::Result<Vec<SpeechCap>> {
                Ok(vec![SpeechCap {
                    tenant_id: None,
                    provider: "alt".into(),
                    kind: SpeechCapKind::Platform,
                    unit: CapUnit::Minutes,
                    monthly_characters: Some(1),
                }])
            }
        }
        let primary = ScriptedStt::start("up", &[(700, "primary")]).await;
        let alt = ScriptedStt::start("up", &[(700, "alt")]).await;
        let remote = |url: &str| {
            let url = url.to_string();
            async move {
                Arc::new(
                    SttService::new(SttConfig {
                        provider: SttProviderConfig::Remote {
                            url,
                            auth_token: "up".into(),
                            provider: None,
                        },
                        language: "en-US".into(),
                    })
                    .await
                    .unwrap(),
                )
            }
        };
        let t = fcp_core::TenantId::new();
        let mut speech = fcp_core::speech::TenantSpeech::defaults(t);
        speech.recognizer = Some("alt".into());
        let kept = Arc::new(Kept::default());
        let usage = crate::usage::UsageCounter::new(kept.clone());
        usage.caps.refresh().await;
        let base = test_state().await;
        let state = Arc::new(AppState {
            services: base.services.clone(),
            speech: Some(Arc::new(Settings([(t, speech)].into_iter().collect()))),
            speech_cache: Default::default(),
            accounts: None,
            account_services: Default::default(),
            account_recognizers: Default::default(),
            held_back: Default::default(),
            usage: Some(Arc::clone(&usage)),
            config: base.config.clone(),
            default_provider: base.default_provider.clone(),
            stt: [
                ("primary".to_string(), remote(primary.url()).await),
                ("alt".to_string(), remote(alt.url()).await),
            ]
            .into_iter()
            .collect(),
            stt_default: Some("primary".into()),
        });
        // Two seconds of tone.
        let tone: Vec<i16> = (0..16 * 2000)
            .map(|i| {
                (9000.0 * (2.0 * std::f64::consts::PI * 700.0 * i as f64 / 16_000.0).sin()) as i16
            })
            .collect();
        let hear = |provider: Option<&str>| {
            let state = Arc::clone(&state);
            let query = TranscribeQuery {
                tenant_id: Some(t.to_string()),
                provider: provider.map(String::from),
                ..Default::default()
            };
            let body = Bytes::from(wav_bytes(&tone));
            async move {
                let r = transcribe(State(state), Query(query), body).await;
                let status = r.status();
                let body = axum::body::to_bytes(r.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                (status, v)
            }
        };
        let (status, v) = hear(None).await;
        assert_eq!((status, v["text"].as_str()), (StatusCode::OK, Some("alt")));
        // 58 more seconds heard elsewhere on this server: the minute is up.
        usage.count_heard(&t.to_string(), "alt", 58_000);
        let (status, v) = hear(None).await;
        assert_eq!(
            (status, v["text"].as_str()),
            (StatusCode::OK, Some("primary"))
        );
        let (status, v) = hear(Some("alt")).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(v["code"], "speech_cap_reached");

        usage.flush().await;
        let rows = kept.0.lock().unwrap().clone();
        let ms = |p: &str| {
            rows.iter()
                .filter(|r| r.provider == p)
                .map(|r| r.recognized_ms)
                .sum::<u64>()
        };
        assert_eq!(ms("alt"), 60_000, "{rows:?}");
        assert_eq!(ms("primary"), 2_000, "{rows:?}");
    }
}
