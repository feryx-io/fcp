//! The speech providers against stand-ins for their services: what each
//! sends (auth, query, body) and how it reads the answer
//! (`docs/DEVELOPER_SURFACE.md` phase 6). The real services are checked by
//! hand; see the phase's PR.

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Multipart, Query, State,
    },
    http::HeaderMap,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use fcp_media::stt::{SttConfig, SttOptions, SttProviderConfig, SttService};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Seen {
    authorization: Vec<String>,
    queries: Vec<Vec<(String, String)>>,
    fields: HashMap<String, String>,
    file_bytes: usize,
    streamed_bytes: usize,
}

type Shared = Arc<Mutex<Seen>>;

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

fn remember(seen: &Shared, headers: &HeaderMap, query: Vec<(String, String)>) {
    let mut seen = seen.lock().unwrap();
    seen.authorization.push(
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string(),
    );
    seen.queries.push(query);
}

fn speech() -> Vec<i16> {
    (0..16_000)
        .map(|i| (8000.0 * (i as f64 * 0.17).sin()) as i16)
        .collect()
}

async fn service(provider: SttProviderConfig) -> SttService {
    SttService::new(SttConfig {
        provider,
        language: "en-US".into(),
    })
    .await
    .unwrap()
}

// ── Deepgram ────────────────────────────────────────────────────────────

async fn deepgram_file(
    State(seen): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<Vec<(String, String)>>,
    body: Bytes,
) -> impl IntoResponse {
    remember(&seen, &headers, query);
    seen.lock().unwrap().file_bytes = body.len();
    Json(serde_json::json!({
        "results": {"channels": [{"alternatives": [
            {"transcript": "sales please", "confidence": 0.93}
        ]}]}
    }))
}

async fn deepgram_live(
    State(seen): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<Vec<(String, String)>>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    remember(&seen, &headers, query);
    upgrade.on_upgrade(move |socket| deepgram_session(socket, seen))
}

/// Answers the audio it was sent, in two pieces, once the client closes
/// the stream; the first piece a partial, as Deepgram sends them.
async fn deepgram_session(mut socket: WebSocket, seen: Shared) {
    while let Some(Ok(message)) = socket.recv().await {
        match message {
            Message::Binary(b) => seen.lock().unwrap().streamed_bytes += b.len(),
            Message::Text(t) if t.contains("CloseStream") => break,
            _ => {}
        }
    }
    let result = |text: &str, is_final: bool, speech_final: bool| {
        Message::Text(
            serde_json::json!({
                "type": "Results", "is_final": is_final, "speech_final": speech_final,
                "channel": {"alternatives": [{"transcript": text, "confidence": 0.8}]}
            })
            .to_string(),
        )
    };
    let _ = socket
        .send(Message::Text(r#"{"type":"Metadata"}"#.into()))
        .await;
    let _ = socket.send(result("billing", false, false)).await;
    let _ = socket.send(result("billing", true, false)).await;
    let _ = socket.send(result("department", true, true)).await;
    let _ = socket.send(result("again", true, false)).await;
    let _ = socket.close().await;
}

#[tokio::test]
async fn deepgram_transcribes_a_file_and_a_stream() {
    let seen = Shared::default();
    let base = serve(
        Router::new()
            .route("/v1/listen", post(deepgram_file).get(deepgram_live))
            .with_state(seen.clone()),
    )
    .await;
    let stt = service(SttProviderConfig::Deepgram {
        api_key: "dg-key".into(),
        model: "nova-3".into(),
        base_url: Some(base),
    })
    .await;

    let options = SttOptions {
        hints: vec!["sales".into()],
        ..Default::default()
    };
    let transcript = stt.transcribe_pcm(&speech(), &options).await.unwrap();
    assert_eq!(transcript.text, "sales please");
    assert_eq!(transcript.confidence, Some(0.93));
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.authorization[0], "Token dg-key");
        let query = &seen.queries[0];
        assert!(query.contains(&("language".into(), "en-US".into())));
        assert!(query.contains(&("keyterm".into(), "sales".into())));
        assert_eq!(seen.file_bytes, 44 + 32_000, "a WAV of the audio");
    }

    let mut stream = stt
        .stream(&SttOptions {
            interim_results: true,
            speech_timeout_ms: Some(600),
            ..Default::default()
        })
        .await
        .unwrap();
    for frame in speech().chunks(320) {
        stream.send(frame.to_vec()).unwrap();
    }
    stream.finish();
    let mut got = Vec::new();
    while let Some(t) = stream.next().await {
        got.push((t.text, t.is_final));
    }
    assert_eq!(
        got,
        vec![
            ("billing".to_string(), false),
            ("billing department".to_string(), true),
            // The stream ended mid-utterance: what was final is flushed.
            ("again".to_string(), true),
        ]
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.streamed_bytes, 32_000);
    assert_eq!(seen.authorization[1], "Token dg-key");
    let query = &seen.queries[1];
    for expected in [
        ("encoding", "linear16"),
        ("sample_rate", "16000"),
        ("endpointing", "600"),
        ("language", "en-US"),
    ] {
        assert!(
            query.contains(&(expected.0.into(), expected.1.into())),
            "{expected:?} in {query:?}"
        );
    }
}

// ── OpenAI ──────────────────────────────────────────────────────────────

async fn openai_file(
    State(seen): State<Shared>,
    headers: HeaderMap,
    mut form: Multipart,
) -> impl IntoResponse {
    remember(&seen, &headers, Vec::new());
    while let Some(field) = form.next_field().await.unwrap() {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let bytes = field.bytes().await.unwrap();
            seen.lock().unwrap().file_bytes += bytes.len();
        } else {
            let text = field.text().await.unwrap();
            seen.lock().unwrap().fields.insert(name, text);
        }
    }
    Json(serde_json::json!({"text": " Two tickets, please. "}))
}

#[tokio::test]
async fn openai_transcribes_a_file_and_cuts_a_stream_into_utterances() {
    let seen = Shared::default();
    let base = serve(
        Router::new()
            .route("/v1/audio/transcriptions", post(openai_file))
            .with_state(seen.clone()),
    )
    .await;
    let stt = service(SttProviderConfig::OpenAi {
        api_key: "sk-test".into(),
        model: "whisper-1".into(),
        base_url: Some(base),
    })
    .await;

    let transcript = stt
        .transcribe_pcm(
            &speech(),
            &SttOptions {
                language: Some("fr-CA".into()),
                hints: vec!["tickets".into(), "matinee".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(transcript.text, "Two tickets, please.");
    assert_eq!(transcript.confidence, None);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.authorization[0], "Bearer sk-test");
        assert_eq!(seen.fields["model"], "whisper-1");
        assert_eq!(seen.fields["language"], "fr", "whisper takes ISO 639-1");
        assert_eq!(seen.fields["prompt"], "tickets, matinee");
        assert_eq!(seen.file_bytes, 44 + 32_000);
    }

    // No stream of its own: the service cuts one utterance and sends it.
    let mut stream = stt
        .stream(&SttOptions {
            speech_timeout_ms: Some(300),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut audio = speech();
    audio.extend(std::iter::repeat_n(0, 16_000));
    for frame in audio.chunks(320) {
        stream.send(frame.to_vec()).unwrap();
    }
    let first = stream.next().await.unwrap();
    assert!(first.is_final);
    assert_eq!(first.text, "Two tickets, please.");
    stream.finish();
    assert!(stream.next().await.is_none(), "silence is no utterance");
    assert_eq!(seen.lock().unwrap().authorization.len(), 2);
}
