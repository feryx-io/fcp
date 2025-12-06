use std::path::PathBuf;
use std::time::Duration;
use crate::metrics::{MetricsHandle, noop_metrics};

/// Top-level server settings for fcp-sip.
#[derive(Clone)]
pub struct ServerSettings {
    pub metrics: MetricsHandle,
    pub transcript_dir: Option<PathBuf>,
    pub allow_info_content_types: Vec<String>,
    pub allow_refer: bool,
    pub rate_window: Duration,
    pub info_rate_limit: u32,
    pub reinvite_rate_limit: u32,
    pub update_rate_limit: u32,
    pub info_max_body_bytes: usize,
    pub notify_max_body_bytes: usize,
}

impl ServerSettings {
    /// Build settings using environment defaults.
    pub fn from_env() -> Self {
        let transcript_dir = std::env::var("FCP_SIP_TRANSCRIPT_DIR")
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.exists());

        Self {
            metrics: noop_metrics(),
            transcript_dir,
            allow_info_content_types: vec!["application/dtmf-relay".into(), "application/media_control+xml".into()],
            allow_refer: true,
            rate_window: Duration::from_secs(1),
            info_rate_limit: 10,
            reinvite_rate_limit: 3,
            update_rate_limit: 5,
            info_max_body_bytes: 4096,
            notify_max_body_bytes: 4096,
        }
    }

    pub fn with_metrics(mut self, metrics: MetricsHandle) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_transcript_dir<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.transcript_dir = Some(dir.into());
        self
    }

    pub fn with_info_content_types(mut self, types: Vec<String>) -> Self {
        self.allow_info_content_types = types;
        self
    }

    pub fn with_refer_allowed(mut self, allowed: bool) -> Self {
        self.allow_refer = allowed;
        self
    }

    pub fn with_rate_limits(
        mut self,
        window: Duration,
        info_limit: u32,
        reinvite_limit: u32,
        update_limit: u32,
    ) -> Self {
        self.rate_window = window;
        self.info_rate_limit = info_limit;
        self.reinvite_rate_limit = reinvite_limit;
        self.update_rate_limit = update_limit;
        self
    }

    pub fn with_body_limits(mut self, info_max: usize, notify_max: usize) -> Self {
        self.info_max_body_bytes = info_max;
        self.notify_max_body_bytes = notify_max;
        self
    }
}
