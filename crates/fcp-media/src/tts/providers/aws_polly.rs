//! AWS Polly Text-to-Speech provider (stub implementation).
//!
//! This is a placeholder implementation. Full AWS Polly integration will be added later.

use super::super::{TtsProvider, VoiceInfo};
use anyhow::Result;
use async_trait::async_trait;

/// AWS Polly TTS provider (stub).
pub struct AwsPollyTts {
    #[allow(dead_code)]
    region: String,
    #[allow(dead_code)]
    access_key_id: String,
    #[allow(dead_code)]
    secret_access_key: String,
}

impl AwsPollyTts {
    /// Create a new AWS Polly TTS provider.
    pub fn new(region: String, access_key_id: String, secret_access_key: String) -> Result<Self> {
        Ok(Self {
            region,
            access_key_id,
            secret_access_key,
        })
    }
}

#[async_trait]
impl TtsProvider for AwsPollyTts {
    fn name(&self) -> String {
        "aws_polly".to_string()
    }

    async fn synthesize(
        &self,
        _text: &str,
        _language: &str,
        _voice: Option<&str>,
    ) -> Result<Vec<u8>> {
        anyhow::bail!("AWS Polly TTS provider not yet implemented")
    }

    async fn list_voices(&self, _language: &str) -> Result<Vec<VoiceInfo>> {
        anyhow::bail!("AWS Polly TTS provider not yet implemented")
    }
}
