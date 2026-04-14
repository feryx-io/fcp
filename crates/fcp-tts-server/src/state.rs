use fcp_media::tts::TtsService;
use std::collections::HashMap;
use std::sync::Arc;

use crate::config::TtsServerConfig;

/// Shared application state passed to Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: TtsServerConfig,
    /// Named TTS services — each wraps a provider + its own cache.
    /// The default provider is always present under its configured name.
    pub services: HashMap<String, Arc<TtsService>>,
    /// Name of the default provider.
    pub default_provider: String,
}

impl AppState {
    /// Get the default TtsService.
    pub fn default_service(&self) -> Option<&Arc<TtsService>> {
        self.services.get(&self.default_provider)
    }

    /// Get a TtsService by provider name, falling back to default.
    pub fn service(&self, name: Option<&str>) -> Option<&Arc<TtsService>> {
        match name {
            Some(n) => self.services.get(n).or_else(|| self.default_service()),
            None => self.default_service(),
        }
    }
}
