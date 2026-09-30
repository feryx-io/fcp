//! A scripted speech recognizer: a stand-in for `fcp-tts-server`'s
//! `/v1/transcribe` that answers a known phrase for a known tone
//! (`docs/DEVELOPER_SURFACE.md` phase 6).
//!
//! A phone in a test cannot speak, but it can play tones. The script maps
//! a tone to what "was said" — `(700, "leave a message for Alice")` — and
//! the recognizer answers that phrase for audio whose voiced frames are
//! mostly that tone. A binary is pointed at it with the `remote` provider:
//!
//! ```toml
//! [stt.provider]
//! type = "remote"
//! url = "<ScriptedStt::url>"
//! auth_token = "<the token given>"
//! ```
//!
//! Both forms are served: a file (`POST /v1/transcribe`) and a stream
//! (`GET /v1/transcribe/stream`), each cut into utterances by
//! [`fcp_media::stt::Endpointer`] as the real providers' streams are.

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use fcp_media::stt::{self, Endpointer, Transcript, STT_SAMPLE_RATE};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One request the recognizer answered.
#[derive(Debug, Clone)]
pub struct SttRequest {
    /// `file` or `stream`.
    pub kind: &'static str,
    /// The query it carried (`language`, `hints`, …).
    pub query: HashMap<String, String>,
    /// Seconds of audio it was sent.
    pub seconds: f32,
    /// What it answered, final transcripts in order.
    pub answered: Vec<String>,
}

struct Shared {
    token: String,
    script: Vec<(u32, String)>,
    requests: Mutex<Vec<SttRequest>>,
}

/// The scripted recognizer, serving on a loopback port until the process
/// ends.
pub struct ScriptedStt {
    url: String,
    shared: Arc<Shared>,
}

impl ScriptedStt {
    /// Serve, answering each `(hz, phrase)` of the script for that tone.
    pub async fn start(token: &str, script: &[(u32, &str)]) -> Self {
        let shared = Arc::new(Shared {
            token: token.to_string(),
            script: script.iter().map(|(hz, p)| (*hz, p.to_string())).collect(),
            requests: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .route("/v1/transcribe", post(file))
            .route("/v1/transcribe/stream", get(stream))
            .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
            .with_state(Arc::clone(&shared));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the scripted recognizer");
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Self { url, shared }
    }

    /// The base URL, for a `remote` provider's `url`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The requests answered so far.
    pub fn requests(&self) -> Vec<SttRequest> {
        self.shared.requests.lock().unwrap().clone()
    }

    /// Wait for `n` requests to have been answered.
    pub async fn wait_for(&self, n: usize, within: Duration) -> Vec<SttRequest> {
        let deadline = Instant::now() + within;
        loop {
            let requests = self.requests();
            if requests.len() >= n || Instant::now() >= deadline {
                return requests;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// The phrase for the tone most of the voiced 20 ms frames carry.
fn recognize(script: &[(u32, String)], pcm: &[i16]) -> Transcript {
    let frame = STT_SAMPLE_RATE as usize / 50;
    let mut votes = vec![0usize; script.len()];
    let mut voiced = 0usize;
    for chunk in pcm.chunks(frame) {
        if stt::rms(chunk) < 200.0 {
            continue;
        }
        voiced += 1;
        for (i, (hz, _)) in script.iter().enumerate() {
            if crate::media::goertzel_ratio_at(chunk, *hz, STT_SAMPLE_RATE) > 0.5 {
                votes[i] += 1;
            }
        }
    }
    match votes.iter().enumerate().max_by_key(|(_, v)| **v) {
        Some((i, &v)) if voiced > 0 && v * 2 > voiced => {
            Transcript::final_text(script[i].1.clone(), Some(0.9))
        }
        _ => Transcript::final_text("", Some(0.0)),
    }
}

fn authorized(shared: &Shared, headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {}", shared.token))
}

async fn file(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if !authorized(&shared, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let pcm = match stt::wav_bytes_16k_mono(&body) {
        Ok(pcm) => pcm,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    };
    let transcript = recognize(&shared.script, &pcm);
    shared.requests.lock().unwrap().push(SttRequest {
        kind: "file",
        query,
        seconds: pcm.len() as f32 / STT_SAMPLE_RATE as f32,
        answered: vec![transcript.text.clone()],
    });
    Json(transcript).into_response()
}

async fn stream(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !authorized(&shared, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    upgrade.on_upgrade(move |socket| stream_session(socket, shared, query))
}

async fn stream_session(socket: WebSocket, shared: Arc<Shared>, query: HashMap<String, String>) {
    let silence_ms = query
        .get("speech_timeout_ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(stt::DEFAULT_SPEECH_TIMEOUT_MS);
    let mut endpointer = Endpointer::new(silence_ms);
    let (mut sink, mut source) = socket.split();
    let mut samples = 0usize;
    let mut answered = Vec::new();
    let mut send = |utterance: Vec<i16>| {
        let transcript = recognize(&shared.script, &utterance);
        answered.push(transcript.text.clone());
        serde_json::to_string(&transcript).unwrap()
    };
    let mut outgoing = Vec::new();
    while let Some(Ok(message)) = source.next().await {
        match message {
            Message::Binary(bytes) => {
                let pcm = stt::pcm_from_le_bytes(&bytes);
                samples += pcm.len();
                if let Some(utterance) = endpointer.push(&pcm) {
                    outgoing.push(send(utterance));
                }
            }
            Message::Text(text) if text.contains("finish") => break,
            Message::Close(_) => break,
            _ => {}
        }
        for text in outgoing.drain(..) {
            if sink.send(Message::Text(text)).await.is_err() {
                return;
            }
        }
    }
    if let Some(utterance) = endpointer.flush() {
        let text = send(utterance);
        let _ = sink.send(Message::Text(text)).await;
    }
    let _ = sink.send(Message::Close(None)).await;
    shared.requests.lock().unwrap().push(SttRequest {
        kind: "stream",
        query,
        seconds: samples as f32 / STT_SAMPLE_RATE as f32,
        answered,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_media::stt::{SttConfig, SttOptions, SttProviderConfig, SttService};

    fn tone(hz: f64, ms: usize) -> Vec<i16> {
        (0..16 * ms)
            .map(|i| {
                (9000.0 * (2.0 * std::f64::consts::PI * hz * i as f64 / 16_000.0).sin()) as i16
            })
            .collect()
    }

    async fn service(url: &str, token: &str) -> SttService {
        SttService::new(SttConfig {
            provider: SttProviderConfig::Remote {
                url: url.into(),
                auth_token: token.into(),
                provider: None,
            },
            language: "en-US".into(),
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn the_remote_provider_hears_the_scripted_phrases() {
        let scripted = ScriptedStt::start("s3cret", &[(700, "sales"), (1300, "support")]).await;
        let stt = service(scripted.url(), "s3cret").await;

        let file = stt
            .transcribe_pcm(
                &tone(1300.0, 800),
                &SttOptions {
                    hints: vec!["sales".into(), "support".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(file.text, "support");

        let mut stream = stt
            .stream(&SttOptions {
                speech_timeout_ms: Some(300),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut audio = tone(700.0, 600);
        audio.extend(vec![0; 16 * 500]);
        audio.extend(tone(1300.0, 400));
        for frame in audio.chunks(320) {
            stream.send(frame.to_vec()).unwrap();
        }
        stream.finish();
        let mut heard = Vec::new();
        while let Some(t) = stream.next().await {
            heard.push(t.text);
        }
        assert_eq!(heard, vec!["sales", "support"]);

        let requests = scripted.wait_for(2, Duration::from_secs(5)).await;
        assert_eq!(requests[0].kind, "file");
        assert_eq!(requests[0].query["hints"], "sales,support");
        assert_eq!(requests[0].query["language"], "en-US");
        assert_eq!(requests[1].kind, "stream");
        assert_eq!(requests[1].query["speech_timeout_ms"], "300");

        let wrong = service(scripted.url(), "nope").await;
        assert!(wrong
            .transcribe_pcm(&tone(700.0, 300), &SttOptions::default())
            .await
            .is_err());
    }
}
