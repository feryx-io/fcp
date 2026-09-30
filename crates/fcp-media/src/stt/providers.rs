//! Speech-to-text provider implementations.

use super::{SttProvider, SttProviderConfig};
use anyhow::Result;
use std::sync::Arc;

pub mod deepgram;
pub mod local;
pub mod openai;
pub mod remote;
pub(crate) mod ws;

/// The provider a configuration names.
pub async fn create_provider(config: SttProviderConfig) -> Result<Arc<dyn SttProvider>> {
    Ok(match config {
        SttProviderConfig::Deepgram {
            api_key,
            model,
            base_url,
        } => Arc::new(deepgram::DeepgramStt::new(api_key, model, base_url)?),
        SttProviderConfig::OpenAi {
            api_key,
            model,
            base_url,
        } => Arc::new(openai::OpenAiStt::new(api_key, model, base_url)?),
        SttProviderConfig::Local {
            python_path,
            model,
            device,
        } => Arc::new(local::LocalStt::new(python_path, model, device)),
        SttProviderConfig::Remote {
            url,
            auth_token,
            provider,
        } => Arc::new(remote::RemoteStt::new(url, auth_token, provider)?),
    })
}
