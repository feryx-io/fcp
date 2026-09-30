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
        }
    }
}

fn error(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// The provider a request names, or why there is none.
fn service(
    state: &AppState,
    query: &TranscribeQuery,
) -> Result<Arc<SttService>, (StatusCode, String)> {
    if state.stt.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "speech recognition is not configured ([stt.providers])".into(),
        ));
    }
    state
        .stt_service(query.provider.as_deref())
        .cloned()
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!(
                    "unknown speech provider '{}'",
                    query.provider.as_deref().unwrap_or_default()
                ),
            )
        })
}

/// `POST /v1/transcribe`: a WAV file in the body, one final transcript
/// out.
pub async fn transcribe(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TranscribeQuery>,
    body: Bytes,
) -> Response {
    let service = match service(&state, &query) {
        Ok(s) => s,
        Err((status, message)) => return error(status, message),
    };
    let pcm = match tokio::task::spawn_blocking(move || stt::wav_bytes_16k_mono(&body)).await {
        Ok(Ok(pcm)) => pcm,
        Ok(Err(e)) => return error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let seconds = pcm.len() as f32 / stt::STT_SAMPLE_RATE as f32;
    match service.transcribe_pcm(&pcm, &query.options()).await {
        Ok(transcript) => {
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
    let service = match service(&state, &query) {
        Ok(s) => s,
        Err((status, message)) => return error(status, message),
    };
    let options = query.options();
    upgrade.on_upgrade(move |socket| stream_session(socket, service, options))
}

async fn stream_session(socket: WebSocket, service: Arc<SttService>, options: SttOptions) {
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
                    if stream.send(stt::pcm_from_le_bytes(&bytes)).is_err() {
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
