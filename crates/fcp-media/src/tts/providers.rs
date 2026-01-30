//! TTS provider implementations.

use super::{TtsProvider, TtsProviderConfig};
use anyhow::Result;
use std::sync::Arc;

pub mod aws_polly;
pub mod azure;
pub mod elevenlabs;
pub mod google_cloud;
pub mod local;
pub mod openai;

/// Create a TTS provider from configuration.
pub async fn create_provider(config: TtsProviderConfig) -> Result<Arc<dyn TtsProvider>> {
    match config {
        TtsProviderConfig::GoogleCloud { api_key } => {
            let provider = google_cloud::GoogleCloudTts::new(api_key)?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::AwsPolly {
            region,
            access_key_id,
            secret_access_key,
        } => {
            let provider = aws_polly::AwsPollyTts::new(region, access_key_id, secret_access_key)?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::Azure {
            subscription_key,
            region,
        } => {
            let provider = azure::AzureTts::new(subscription_key, region)?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::OpenAi { api_key, model } => {
            let provider = openai::OpenAiTts::new(api_key, model)?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::ElevenLabs { api_key, model_id } => {
            let provider = elevenlabs::ElevenLabsTts::new(api_key, model_id)?;
            Ok(Arc::new(provider))
        }

        TtsProviderConfig::Local {
            engine,
            executable_path,
        } => {
            let provider = local::LocalTts::new(engine, executable_path)?;
            Ok(Arc::new(provider))
        }
    }
}
