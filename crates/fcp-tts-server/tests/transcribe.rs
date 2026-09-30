//! Speech recognition through the running server: `/v1/transcribe` for a
//! file and `/v1/transcribe/stream` for a stream, relayed to a provider —
//! here the testkit's scripted recognizer behind a `remote` provider —
//! and the server's own token enforced (`docs/DEVELOPER_SURFACE.md`
//! phase 6).

use fcp_media::stt::{wav_bytes, SttConfig, SttOptions, SttProviderConfig, SttService};
use fcp_testkit::service::Service;
use fcp_testkit::smoke::{self, free_port};
use fcp_testkit::stt::ScriptedStt;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-tts");

fn tone(hz: f64, ms: usize) -> Vec<i16> {
    (0..16 * ms)
        .map(|i| (9000.0 * (2.0 * std::f64::consts::PI * hz * i as f64 / 16_000.0).sin()) as i16)
        .collect()
}

#[tokio::test]
async fn the_server_transcribes_files_and_streams_through_its_provider() {
    let scripted = ScriptedStt::start("upstream", &[(700, "sales"), (1300, "support")]).await;
    let http = free_port();
    let config = format!(
        r#"[http]
bind_address = "127.0.0.1:{http}"

[auth]
tokens = ["tts-token"]

[stt]
default_provider = "scripted"
language = "en-GB"

[stt.providers.scripted]
type = "remote"
url = "{url}"
auth_token = "upstream"

[logging]
level = "info"
format = "compact"
"#,
        url = scripted.url(),
    );
    let server = Service::start("fcp-tts", BIN, &config, &smoke::loopback(http, "/health")).await;
    let base = format!("http://127.0.0.1:{http}");

    // A file, over plain HTTP: the default language added on the way.
    let client = reqwest::Client::new();
    let answer: serde_json::Value = client
        .post(format!("{base}/v1/transcribe?hints=sales,support"))
        .bearer_auth("tts-token")
        .header("Content-Type", "audio/wav")
        .body(wav_bytes(&tone(700.0, 900)))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(answer["text"], "sales");
    assert_eq!(answer["is_final"], true);
    let upstream = scripted.requests();
    assert_eq!(upstream[0].query["language"], "en-GB");
    assert_eq!(upstream[0].query["hints"], "sales,support");

    // Without the server's token, nothing.
    let refused = client
        .post(format!("{base}/v1/transcribe"))
        .body(wav_bytes(&tone(700.0, 300)))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);
    // Not a WAV.
    let bad = client
        .post(format!("{base}/v1/transcribe"))
        .bearer_auth("tts-token")
        .body("not audio")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    // A provider it does not have.
    let unknown = client
        .post(format!("{base}/v1/transcribe?provider=deepgram"))
        .bearer_auth("tts-token")
        .body(wav_bytes(&tone(700.0, 300)))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 400);

    // A stream, as a node's `remote` provider opens one.
    let node = SttService::new(SttConfig {
        provider: SttProviderConfig::Remote {
            url: base.clone(),
            auth_token: "tts-token".into(),
            provider: None,
        },
        language: "en-US".into(),
    })
    .await
    .unwrap();
    let mut stream = node
        .stream(&SttOptions {
            speech_timeout_ms: Some(300),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut audio = tone(1300.0, 500);
    audio.extend(vec![0; 16 * 500]);
    audio.extend(tone(700.0, 500));
    for frame in audio.chunks(320) {
        stream.send(frame.to_vec()).unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    stream.finish();
    let mut heard = Vec::new();
    while let Some(t) = tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .expect("the stream ends")
    {
        heard.push(t.text);
    }
    assert_eq!(heard, vec!["support", "sales"]);
    let upstream = scripted.wait_for(2, Duration::from_secs(5)).await;
    assert_eq!(upstream[1].kind, "stream");
    assert_eq!(upstream[1].query["speech_timeout_ms"], "300");
    assert_eq!(upstream[1].query["language"], "en-US");

    server.stop().await;
}
