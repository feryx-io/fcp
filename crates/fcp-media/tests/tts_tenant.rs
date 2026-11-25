//! Speech for a tenant reaches the TTS server naming the tenant, and is
//! cached apart from another tenant's (`docs/TENANT_SPEECH.md` S1).

use axum::{routing::post, Json, Router};
use fcp_core::TenantId;
use fcp_media::tts::{TtsConfig, TtsProviderConfig, TtsService};
use std::sync::{Arc, Mutex};

/// A TTS server that answers a tiny WAV and keeps what each request named.
async fn stand_in() -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let seen: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let heard = Arc::clone(&seen);
    let router = Router::new().route(
        "/v1/synthesize",
        post(move |Json(body): Json<serde_json::Value>| {
            let heard = Arc::clone(&heard);
            async move {
                heard.lock().unwrap().push(body);
                wav()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (url, seen)
}

/// 8 kHz mono 16-bit silence, 10 ms.
fn wav() -> Vec<u8> {
    let data = vec![0u8; 160];
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&8000u32.to_le_bytes());
    w.extend_from_slice(&16000u32.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    w
}

#[tokio::test]
async fn a_tenants_words_name_the_tenant_and_are_cached_apart() {
    let (url, seen) = stand_in().await;
    let dir = tempfile::tempdir().unwrap();
    let tts = TtsService::new(TtsConfig {
        provider: TtsProviderConfig::Remote {
            url,
            auth_token: "t".into(),
            provider: None,
            tenant_id: None,
        },
        cache_dir: dir.path().to_path_buf(),
        cache_max_size_mb: 10,
        default_language: "en-US".into(),
        default_voice: None,
    })
    .await
    .unwrap();
    let a = TenantId::new();
    let b = TenantId::new();

    let for_a = tts
        .synthesize_for("Welcome", None, None, Some(&a))
        .await
        .unwrap();
    // Cached for A: not asked again.
    tts.synthesize_for("Welcome", None, None, Some(&a))
        .await
        .unwrap();
    let for_b = tts
        .synthesize_for("Welcome", None, None, Some(&b))
        .await
        .unwrap();
    let for_none = tts.synthesize("Welcome", None, None).await.unwrap();

    let asked: Vec<Option<String>> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|r| r["tenant_id"].as_str().map(String::from))
        .collect();
    assert_eq!(
        asked,
        vec![
            Some(a.as_uuid().to_string()),
            Some(b.as_uuid().to_string()),
            None
        ],
        "each tenant asked for once, and the platform without one"
    );
    assert_ne!(for_a, for_b);
    assert_ne!(for_a, for_none);
}
