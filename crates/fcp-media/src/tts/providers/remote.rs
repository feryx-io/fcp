//! Remote TTS provider — delegates synthesis to a shared fcp-tts service via HTTP.
//!
//! When the fcp-tts service is running, other services use this provider instead
//! of spawning their own local TTS subprocess.  This eliminates duplicate model
//! loads and lets the TTS service run on a dedicated GPU node.

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::Serialize;
use tracing::{debug, warn};

use super::super::{TtsProvider, VoiceInfo};

/// Request body sent to `POST /v1/synthesize` on the fcp-tts service.
#[derive(Serialize)]
struct SynthesizeRequest<'a> {
    text: &'a str,
    language: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_id: Option<&'a str>,
}

/// TTS provider that calls a remote fcp-tts HTTP service.
pub struct RemoteTtsProvider {
    http: reqwest::Client,
    url: String,
    auth_token: String,
    /// Optional provider name to request on the remote server.
    provider_override: Option<String>,
    /// Tenant ID passed with each request for usage tracking.
    tenant_id: Option<String>,
}

impl RemoteTtsProvider {
    /// Create a new remote TTS provider.
    ///
    /// # Arguments
    /// * `url` — Base URL of the fcp-tts service (e.g. `http://localhost:8093`)
    /// * `auth_token` — Bearer token for authentication
    /// * `provider` — Optional provider name to request (uses server default if None)
    /// * `tenant_id` — Optional tenant ID for usage tracking
    pub fn new(
        url: String,
        auth_token: String,
        provider: Option<String>,
        tenant_id: Option<String>,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()?;

        Ok(Self {
            http,
            url: url.trim_end_matches('/').to_string(),
            auth_token,
            provider_override: provider,
            tenant_id,
        })
    }
}

#[async_trait]
impl TtsProvider for RemoteTtsProvider {
    fn name(&self) -> String {
        "remote".to_string()
    }

    async fn synthesize(&self, text: &str, language: &str, voice: Option<&str>) -> Result<Vec<u8>> {
        self.synthesize_for(text, language, voice, None).await
    }

    async fn synthesize_for(
        &self,
        text: &str,
        language: &str,
        voice: Option<&str>,
        tenant: Option<&str>,
    ) -> Result<Vec<u8>> {
        let url = format!("{}/v1/synthesize", self.url);

        let body = SynthesizeRequest {
            text,
            language,
            voice,
            provider: self.provider_override.as_deref(),
            // The request's tenant, else the one this client was made for.
            tenant_id: tenant.or(self.tenant_id.as_deref()),
        };

        debug!(
            url = %url,
            text_len = text.len(),
            language,
            voice,
            "Sending synthesis request to remote TTS service"
        );

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.auth_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Remote TTS request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            bail!(
                "Remote TTS service returned {} — {}",
                status.as_u16(),
                error_body
            );
        }

        let bytes = response.bytes().await?;
        debug!(
            bytes = bytes.len(),
            "Received synthesized audio from remote TTS service"
        );
        Ok(bytes.to_vec())
    }

    async fn list_voices(&self, language: &str) -> Result<Vec<VoiceInfo>> {
        let url = format!("{}/v1/voices", self.url);
        let mut query_params = vec![("language", language.to_string())];
        if let Some(provider) = self.provider_override.as_ref() {
            query_params.push(("provider", provider.clone()));
        }

        let response = self
            .http
            .get(&url)
            .bearer_auth(&self.auth_token)
            .query(&query_params)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => match resp.json::<Vec<VoiceInfo>>().await {
                Ok(voices) => Ok(voices),
                Err(error) => {
                    warn!(error = %error, "Failed to decode remote TTS voices response");
                    Ok(Vec::new())
                }
            },
            Ok(resp) => {
                warn!(status = %resp.status(), "Remote TTS voices endpoint returned error");
                Ok(Vec::new())
            }
            Err(e) => {
                warn!(error = %e, "Failed to reach remote TTS service for voice list");
                Ok(Vec::new())
            }
        }
    }
}
