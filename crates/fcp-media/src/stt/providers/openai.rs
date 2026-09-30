//! OpenAI's transcription API (`/v1/audio/transcriptions`): a WAV file in,
//! its text out. It has no stream of its own, so a call's audio is cut
//! into utterances ([`super::super::Endpointer`]) and each sent as a file.
//! It gives no confidence.

use super::super::{primary_language, wav_bytes, SttOptions, SttProvider, Transcript};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

const DEFAULT_BASE: &str = "https://api.openai.com";

/// Recognition by OpenAI.
pub struct OpenAiStt {
    http: reqwest::Client,
    api_key: String,
    model: String,
    base: String,
}

#[derive(Deserialize)]
struct Answer {
    text: String,
}

impl OpenAiStt {
    pub fn new(api_key: String, model: String, base_url: Option<String>) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()?,
            api_key,
            model,
            base: base_url
                .unwrap_or_else(|| DEFAULT_BASE.to_string())
                .trim_end_matches('/')
                .to_string(),
        })
    }
}

#[async_trait]
impl SttProvider for OpenAiStt {
    fn name(&self) -> String {
        "openai".to_string()
    }

    async fn transcribe(&self, pcm: &[i16], options: &SttOptions) -> Result<Transcript> {
        let file = reqwest::multipart::Part::bytes(wav_bytes(pcm))
            .file_name("audio.wav")
            .mime_str("audio/wav")?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("response_format", "json");
        if let Some(language) = &options.language {
            form = form.text("language", primary_language(language).to_string());
        }
        if !options.hints.is_empty() {
            form = form.text("prompt", options.hints.join(", "));
        }
        let response = self
            .http
            .post(format!("{}/v1/audio/transcriptions", self.base))
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .context("the OpenAI transcription request")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("OpenAI transcription returned {status}: {body}");
        }
        let answer: Answer = response
            .json()
            .await
            .context("OpenAI's transcription answer")?;
        Ok(Transcript::final_text(answer.text.trim(), None))
    }
}
