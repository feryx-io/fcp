//! Azure Cognitive Services TTS provider (stub implementation).
//!
//! This is a placeholder implementation. Full Azure TTS integration will be added later.

use super::super::{TtsProvider, VoiceInfo};
use anyhow::Result;
use async_trait::async_trait;

/// Azure Cognitive Services TTS provider (stub).
pub struct AzureTts {
    #[allow(dead_code)]
    subscription_key: String,
    #[allow(dead_code)]
    region: String,
}

impl AzureTts {
    /// Create a new Azure TTS provider.
    pub fn new(subscription_key: String, region: String) -> Result<Self> {
        Ok(Self {
            subscription_key,
            region,
        })
    }
}

#[async_trait]
impl TtsProvider for AzureTts {
    fn name(&self) -> String {
        "azure".to_string()
    }

    async fn synthesize(
        &self,
        _text: &str,
        _language: &str,
        _voice: Option<&str>,
    ) -> Result<Vec<u8>> {
        anyhow::bail!("Azure TTS provider not yet implemented")
    }

    async fn list_voices(&self, _language: &str) -> Result<Vec<VoiceInfo>> {
        anyhow::bail!("Azure TTS provider not yet implemented")
    }
}
