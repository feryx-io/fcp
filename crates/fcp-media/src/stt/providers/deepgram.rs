//! Deepgram: files over HTTPS (`POST /v1/listen`) and a live stream over
//! a WebSocket (`/v1/listen` upgraded).
//!
//! Deepgram finalises an utterance in pieces (`is_final`), and marks its
//! end (`speech_final`) after `endpointing` milliseconds of silence. The
//! pieces are joined, so one final [`Transcript`] is one utterance, as the
//! other providers give.

use super::super::{
    pcm_to_le_bytes, wav_bytes, SttOptions, SttProvider, SttStream, Transcript, STT_SAMPLE_RATE,
};
use super::ws;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

const DEFAULT_BASE: &str = "https://api.deepgram.com";

/// Recognition by Deepgram.
pub struct DeepgramStt {
    http: reqwest::Client,
    api_key: String,
    model: String,
    base: String,
}

#[derive(Deserialize)]
struct FileAnswer {
    results: FileResults,
}

#[derive(Deserialize)]
struct FileResults {
    channels: Vec<Channel>,
}

#[derive(Deserialize, Default)]
struct Channel {
    #[serde(default)]
    alternatives: Vec<Alternative>,
}

#[derive(Deserialize, Clone)]
struct Alternative {
    #[serde(default)]
    transcript: String,
    #[serde(default)]
    confidence: Option<f64>,
}

/// A message on the live stream; only `Results` carry speech.
#[derive(Deserialize)]
struct StreamMessage {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    is_final: bool,
    #[serde(default)]
    speech_final: bool,
    #[serde(default)]
    channel: Channel,
}

impl DeepgramStt {
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

    /// The query both forms share. Nova-3 takes its hints as `keyterm`,
    /// the older models as `keywords`.
    fn query(&self, options: &SttOptions) -> Vec<(&'static str, String)> {
        let mut query = vec![
            ("model", self.model.clone()),
            ("smart_format", "true".to_string()),
        ];
        if let Some(language) = &options.language {
            query.push(("language", language.clone()));
        }
        let hint_key = if self.model.starts_with("nova-3") {
            "keyterm"
        } else {
            "keywords"
        };
        for hint in &options.hints {
            query.push((hint_key, hint.clone()));
        }
        query
    }
}

#[async_trait]
impl SttProvider for DeepgramStt {
    fn name(&self) -> String {
        "deepgram".to_string()
    }

    async fn transcribe(&self, pcm: &[i16], options: &SttOptions) -> Result<Transcript> {
        let response = self
            .http
            .post(format!("{}/v1/listen", self.base))
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Content-Type", "audio/wav")
            .query(&self.query(options))
            .body(wav_bytes(pcm))
            .send()
            .await
            .context("the Deepgram request")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Deepgram returned {status}: {body}");
        }
        let answer: FileAnswer = response.json().await.context("Deepgram's answer")?;
        let best = answer
            .results
            .channels
            .first()
            .and_then(|c| c.alternatives.first())
            .cloned()
            .unwrap_or(Alternative {
                transcript: String::new(),
                confidence: None,
            });
        Ok(Transcript::final_text(best.transcript, best.confidence))
    }

    async fn stream(&self, options: &SttOptions) -> Result<Option<SttStream>> {
        let mut query = self.query(options);
        query.extend([
            ("encoding", "linear16".to_string()),
            ("sample_rate", STT_SAMPLE_RATE.to_string()),
            ("channels", "1".to_string()),
            ("interim_results", "true".to_string()),
            ("endpointing", options.speech_timeout_ms().to_string()),
        ]);
        let url = format!(
            "{}/v1/listen?{}",
            ws::ws_url(&self.base),
            serde_urlencoded::to_string(query)?
        );
        let socket = ws::connect(&url, &format!("Token {}", self.api_key)).await?;
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
            let _ = sink
                .send(Message::Text(r#"{"type":"CloseStream"}"#.to_string()))
                .await;
        });
        let interim = options.interim_results;
        tokio::spawn(async move {
            let mut utterance = Utterance::default();
            while let Some(message) = source.next().await {
                let text = match message {
                    Ok(Message::Text(text)) => text,
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(e) => {
                        debug!(error = %e, "the Deepgram stream ended");
                        break;
                    }
                };
                let message: StreamMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(error = %e, "an unreadable Deepgram message");
                        continue;
                    }
                };
                if !utterance.take(message, interim, &results).await {
                    return;
                }
            }
            utterance.flush(&results).await;
        });
        Ok(Some(stream))
    }
}

/// The pieces of the utterance in progress.
#[derive(Default)]
struct Utterance {
    pieces: Vec<String>,
    confidences: Vec<f64>,
}

impl Utterance {
    /// Take one stream message; false once the listener has gone.
    async fn take(
        &mut self,
        message: StreamMessage,
        interim: bool,
        results: &mpsc::Sender<Transcript>,
    ) -> bool {
        if message.kind == "UtteranceEnd" {
            return self.flush(results).await;
        }
        if message.kind != "Results" {
            return true;
        }
        let best = message.channel.alternatives.into_iter().next();
        let (text, confidence) = best
            .map(|a| (a.transcript, a.confidence))
            .unwrap_or_default();
        if message.is_final {
            if !text.trim().is_empty() {
                self.pieces.push(text.trim().to_string());
                self.confidences.extend(confidence);
            }
            if message.speech_final {
                return self.flush(results).await;
            }
            return true;
        }
        if interim && !text.trim().is_empty() {
            let mut so_far = self.pieces.clone();
            so_far.push(text.trim().to_string());
            let partial = Transcript {
                text: so_far.join(" "),
                confidence,
                is_final: false,
            };
            return results.send(partial).await.is_ok();
        }
        true
    }

    /// Send the utterance, if it has anything; false once the listener
    /// has gone.
    async fn flush(&mut self, results: &mpsc::Sender<Transcript>) -> bool {
        if self.pieces.is_empty() {
            return true;
        }
        let confidence = (!self.confidences.is_empty())
            .then(|| self.confidences.iter().sum::<f64>() / self.confidences.len() as f64);
        let text = std::mem::take(&mut self.pieces).join(" ");
        self.confidences.clear();
        results
            .send(Transcript::final_text(text, confidence))
            .await
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn results(text: &str, is_final: bool, speech_final: bool) -> StreamMessage {
        serde_json::from_value(serde_json::json!({
            "type": "Results",
            "is_final": is_final,
            "speech_final": speech_final,
            "channel": {"alternatives": [{"transcript": text, "confidence": 0.9}]}
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn pieces_of_an_utterance_are_joined_and_interims_carry_what_came_before() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut utterance = Utterance::default();
        assert!(
            utterance
                .take(results("i would", false, false), true, &tx)
                .await
        );
        assert!(
            utterance
                .take(results("i would like", true, false), true, &tx)
                .await
        );
        assert!(
            utterance
                .take(results("sales", false, false), true, &tx)
                .await
        );
        assert!(
            utterance
                .take(results("sales please", true, true), true, &tx)
                .await
        );
        let texts: Vec<(String, bool)> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|t| (t.text, t.is_final))
            .collect();
        assert_eq!(
            texts,
            vec![
                ("i would".to_string(), false),
                ("i would like sales".to_string(), false),
                ("i would like sales please".to_string(), true),
            ]
        );
    }

    #[test]
    fn nova_3_takes_its_hints_as_keyterms() {
        let options = SttOptions {
            hints: vec!["sales".into()],
            ..Default::default()
        };
        let nova3 = DeepgramStt::new("k".into(), "nova-3".into(), None).unwrap();
        assert!(nova3.query(&options).contains(&("keyterm", "sales".into())));
        let nova2 = DeepgramStt::new("k".into(), "nova-2".into(), None).unwrap();
        assert!(nova2
            .query(&options)
            .contains(&("keywords", "sales".into())));
    }
}
