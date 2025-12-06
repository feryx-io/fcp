use std::path::PathBuf;
use std::time::Duration;
use crate::metrics::{MetricsHandle, noop_metrics};
use anyhow::{Result, bail};

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

    /// Validate settings and return detailed errors if invalid.
    ///
    /// Checks:
    /// - rate_window is not zero
    /// - Rate limits are non-zero (0 would disable rate limiting)
    /// - Body size limits are within reasonable bounds (1 byte to 100MB)
    /// - transcript_dir exists and is a directory if provided
    pub fn validate(&self) -> Result<()> {
        // Validate rate window
        if self.rate_window.is_zero() {
            bail!("rate_window must not be zero");
        }

        // Rate window should be between 100ms and 1 hour
        if self.rate_window.as_millis() < 100 {
            bail!("rate_window too short (minimum 100ms), got {:?}", self.rate_window);
        }
        if self.rate_window.as_secs() > 3600 {
            bail!("rate_window too long (maximum 1 hour), got {:?}", self.rate_window);
        }

        // Validate rate limits (0 means unlimited, which is allowed but we warn via comments)
        // For safety, we enforce minimum of 1 req/window
        if self.info_rate_limit == 0 {
            bail!("info_rate_limit must be > 0 (set to reasonable value like 10)");
        }
        if self.reinvite_rate_limit == 0 {
            bail!("reinvite_rate_limit must be > 0 (set to reasonable value like 3)");
        }
        if self.update_rate_limit == 0 {
            bail!("update_rate_limit must be > 0 (set to reasonable value like 5)");
        }

        // Check rate limits are reasonable (not excessively high)
        const MAX_RATE_LIMIT: u32 = 1000;
        if self.info_rate_limit > MAX_RATE_LIMIT {
            bail!("info_rate_limit too high (maximum {}), got {}", MAX_RATE_LIMIT, self.info_rate_limit);
        }
        if self.reinvite_rate_limit > MAX_RATE_LIMIT {
            bail!("reinvite_rate_limit too high (maximum {}), got {}", MAX_RATE_LIMIT, self.reinvite_rate_limit);
        }
        if self.update_rate_limit > MAX_RATE_LIMIT {
            bail!("update_rate_limit too high (maximum {}), got {}", MAX_RATE_LIMIT, self.update_rate_limit);
        }

        // Validate body size limits
        const MIN_BODY_SIZE: usize = 1;
        const MAX_BODY_SIZE: usize = 100 * 1024 * 1024; // 100MB

        if self.info_max_body_bytes < MIN_BODY_SIZE {
            bail!("info_max_body_bytes too small (minimum {} bytes), got {}", MIN_BODY_SIZE, self.info_max_body_bytes);
        }
        if self.info_max_body_bytes > MAX_BODY_SIZE {
            bail!("info_max_body_bytes too large (maximum {} bytes / 100MB), got {}", MAX_BODY_SIZE, self.info_max_body_bytes);
        }

        if self.notify_max_body_bytes < MIN_BODY_SIZE {
            bail!("notify_max_body_bytes too small (minimum {} bytes), got {}", MIN_BODY_SIZE, self.notify_max_body_bytes);
        }
        if self.notify_max_body_bytes > MAX_BODY_SIZE {
            bail!("notify_max_body_bytes too large (maximum {} bytes / 100MB), got {}", MAX_BODY_SIZE, self.notify_max_body_bytes);
        }

        // Validate transcript directory if provided
        if let Some(ref dir) = self.transcript_dir {
            if !dir.exists() {
                bail!("transcript_dir does not exist: {}", dir.display());
            }
            if !dir.is_dir() {
                bail!("transcript_dir is not a directory: {}", dir.display());
            }
            // Check if writable by attempting to create a test file
            let test_file = dir.join(".fcp_sip_write_test");
            if let Err(e) = std::fs::write(&test_file, b"test") {
                bail!("transcript_dir is not writable: {} (error: {})", dir.display(), e);
            }
            // Clean up test file
            let _ = std::fs::remove_file(test_file);
        }

        Ok(())
    }

    /// Build and validate settings, returning an error if invalid.
    ///
    /// This is a convenience method that calls validate() and returns
    /// the settings if validation passes.
    ///
    /// # Example
    /// ```no_run
    /// use fcp_sip::settings::ServerSettings;
    /// use std::time::Duration;
    ///
    /// let settings = ServerSettings::from_env()
    ///     .with_rate_limits(Duration::from_secs(1), 10, 3, 5)
    ///     .build()?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn build(self) -> Result<Self> {
        self.validate()?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_default_settings() {
        let settings = ServerSettings::from_env();
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_zero_rate_window() {
        let settings = ServerSettings::from_env()
            .with_rate_limits(Duration::from_secs(0), 10, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rate_window must not be zero"));
    }

    #[test]
    fn test_rate_window_too_short() {
        let settings = ServerSettings::from_env()
            .with_rate_limits(Duration::from_millis(50), 10, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rate_window too short"));
    }

    #[test]
    fn test_rate_window_too_long() {
        let settings = ServerSettings::from_env()
            .with_rate_limits(Duration::from_secs(7200), 10, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rate_window too long"));
    }

    #[test]
    fn test_zero_info_rate_limit() {
        let settings = ServerSettings::from_env()
            .with_rate_limits(Duration::from_secs(1), 0, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("info_rate_limit must be > 0"));
    }

    #[test]
    fn test_zero_reinvite_rate_limit() {
        let settings = ServerSettings::from_env()
            .with_rate_limits(Duration::from_secs(1), 10, 0, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("reinvite_rate_limit must be > 0"));
    }

    #[test]
    fn test_zero_update_rate_limit() {
        let settings = ServerSettings::from_env()
            .with_rate_limits(Duration::from_secs(1), 10, 3, 0);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("update_rate_limit must be > 0"));
    }

    #[test]
    fn test_rate_limit_too_high() {
        let settings = ServerSettings::from_env()
            .with_rate_limits(Duration::from_secs(1), 2000, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("info_rate_limit too high"));
    }

    #[test]
    fn test_body_size_too_small() {
        let settings = ServerSettings::from_env()
            .with_body_limits(0, 4096);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("info_max_body_bytes too small"));
    }

    #[test]
    fn test_body_size_too_large() {
        let settings = ServerSettings::from_env()
            .with_body_limits(200 * 1024 * 1024, 4096); // 200MB

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("info_max_body_bytes too large"));
    }

    #[test]
    fn test_notify_body_size_too_small() {
        let settings = ServerSettings::from_env()
            .with_body_limits(4096, 0);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("notify_max_body_bytes too small"));
    }

    #[test]
    fn test_build_method_with_valid_settings() {
        let result = ServerSettings::from_env()
            .with_rate_limits(Duration::from_secs(1), 10, 3, 5)
            .with_body_limits(4096, 4096)
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn test_build_method_with_invalid_settings() {
        let result = ServerSettings::from_env()
            .with_rate_limits(Duration::from_secs(0), 10, 3, 5)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_valid_rate_limits_boundary() {
        // Test boundary values
        let settings = ServerSettings::from_env()
            .with_rate_limits(Duration::from_millis(100), 1, 1, 1);
        assert!(settings.validate().is_ok());

        let settings = ServerSettings::from_env()
            .with_rate_limits(Duration::from_secs(3600), 1000, 1000, 1000);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_valid_body_limits_boundary() {
        // Test boundary values
        let settings = ServerSettings::from_env()
            .with_body_limits(1, 1);
        assert!(settings.validate().is_ok());

        let settings = ServerSettings::from_env()
            .with_body_limits(100 * 1024 * 1024, 100 * 1024 * 1024);
        assert!(settings.validate().is_ok());
    }
}
