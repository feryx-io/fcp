//! The shared `fcp-tts-server`'s speech recognition.
//!
//! - `POST /v1/transcribe?language=…&hints=a,b&provider=…` with a WAV
//!   body answers one [`Transcript`] as JSON.
//! - `GET /v1/transcribe/stream?…` upgrades to a WebSocket: binary
//!   messages are 16 kHz mono little-endian PCM, a text
//!   `{"type":"finish"}` ends the audio, and each transcript comes back
//!   as a text message of JSON; the server closes when the last is sent.

use super::super::{pcm_to_le_bytes, wav_bytes, SttOptions, SttProvider, SttStream, Transcript};
use super::ws;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

/// The message that ends a stream's audio.
pub const FINISH: &str = r#"{"type":"finish"}"#;

/// Recognition by a remote `fcp-tts-server`.
pub struct RemoteStt {
    http: reqwest::Client,
    url: String,
    auth_token: String,
    provider: Option<String>,
}

impl RemoteStt {
    pub fn new(url: String, auth_token: String, provider: Option<String>) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()?,
            url: url.trim_end_matches('/').to_string(),
            auth_token,
            provider,
        })
    }

    /// The query a request carries.
    fn query(&self, options: &SttOptions, streaming: bool) -> Vec<(&'static str, String)> {
        let mut query = Vec::new();
        if let Some(language) = &options.language {
            query.push(("language", language.clone()));
        }
        if !options.hints.is_empty() {
            query.push(("hints", options.hints.join(",")));
        }
        if let Some(provider) = &self.provider {
            query.push(("provider", provider.clone()));
        }
        if streaming {
            query.push(("interim_results", options.interim_results.to_string()));
            query.push(("speech_timeout_ms", options.speech_timeout_ms().to_string()));
        }
        query
    }
}

#[async_trait]
impl SttProvider for RemoteStt {
    fn name(&self) -> String {
        "remote".to_string()
    }

    async fn transcribe(&self, pcm: &[i16], options: &SttOptions) -> Result<Transcript> {
        let response = self
            .http
            .post(format!("{}/v1/transcribe", self.url))
            .bearer_auth(&self.auth_token)
            .query(&self.query(options, false))
            .header("Content-Type", "audio/wav")
            .body(wav_bytes(pcm))
            .send()
            .await
            .context("the remote transcription request")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("the remote transcription returned {status}: {body}");
        }
        response
            .json::<Transcript>()
            .await
            .context("the remote transcription's answer")
    }

    async fn stream(&self, options: &SttOptions) -> Result<Option<SttStream>> {
        let query = serde_urlencoded::to_string(self.query(options, true))?;
        let url = format!("{}/v1/transcribe/stream?{query}", ws::ws_url(&self.url));
        let socket = ws::connect(&url, &format!("Bearer {}", self.auth_token)).await?;
        let (stream, mut audio, results) = SttStream::channel();
        let (mut sink, mut source) = socket.split();
        tokio::spawn(async move {
            while let Some(frame) = audio.recv().await {
                if sink
                    .send(Message::Binary(pcm_to_le_bytes(&frame)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = sink.send(Message::Text(FINISH.to_string())).await;
        });
        tokio::spawn(async move {
            while let Some(message) = source.next().await {
                match message {
                    Ok(Message::Text(text)) => match serde_json::from_str::<Transcript>(&text) {
                        Ok(transcript) => {
                            if results.send(transcript).await.is_err() {
                                return;
                            }
                        }
                        Err(e) => warn!(error = %e, "an unreadable remote transcript"),
                    },
                    Ok(Message::Close(_)) => return,
                    Ok(_) => {}
                    Err(e) => {
                        debug!(error = %e, "the remote transcription stream ended");
                        return;
                    }
                }
            }
        });
        Ok(Some(stream))
    }
}
