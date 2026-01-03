use crate::metrics::{noop_metrics, MetricsHandle};
use anyhow::{bail, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

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
    pub transcript_capture_bodies: bool,
    pub transcript_max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ServerSettingsFile {
    transcript_dir: Option<PathBuf>,
    allow_info_content_types: Option<Vec<String>>,
    allow_refer: Option<bool>,
    rate_window_secs: Option<u64>,
    info_rate_limit: Option<u32>,
    reinvite_rate_limit: Option<u32>,
    update_rate_limit: Option<u32>,
    info_max_body_bytes: Option<usize>,
    notify_max_body_bytes: Option<usize>,
    transcript_capture_bodies: Option<bool>,
    transcript_max_bytes: Option<u64>,
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
            allow_info_content_types: vec![
                "application/dtmf-relay".into(),
                "application/media_control+xml".into(),
            ],
            allow_refer: true,
            rate_window: Duration::from_secs(1),
            info_rate_limit: 10,
            reinvite_rate_limit: 3,
            update_rate_limit: 5,
            info_max_body_bytes: 4096,
            notify_max_body_bytes: 4096,
            transcript_capture_bodies: false,
            transcript_max_bytes: Some(5 * 1024 * 1024), // 5MB per file by default
        }
    }

    /// Build settings from a config file (TOML), overlaying env defaults.
    pub fn from_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        let cfg: ServerSettingsFile = toml::from_str(&raw)?;
        let mut base = Self::from_env();
        if let Some(dir) = cfg.transcript_dir {
            base.transcript_dir = Some(dir);
        }
        if let Some(types) = cfg.allow_info_content_types {
            base.allow_info_content_types = types;
        }
        if let Some(allow_refer) = cfg.allow_refer {
            base.allow_refer = allow_refer;
        }
        if let Some(secs) = cfg.rate_window_secs {
            base.rate_window = Duration::from_secs(secs);
        }
        if let Some(v) = cfg.info_rate_limit {
            base.info_rate_limit = v;
        }
        if let Some(v) = cfg.reinvite_rate_limit {
            base.reinvite_rate_limit = v;
        }
        if let Some(v) = cfg.update_rate_limit {
            base.update_rate_limit = v;
        }
        if let Some(v) = cfg.info_max_body_bytes {
            base.info_max_body_bytes = v;
        }
        if let Some(v) = cfg.notify_max_body_bytes {
            base.notify_max_body_bytes = v;
        }
        if let Some(v) = cfg.transcript_capture_bodies {
            base.transcript_capture_bodies = v;
        }
        if let Some(v) = cfg.transcript_max_bytes {
            base.transcript_max_bytes = Some(v);
        }
        Ok(base)
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

    pub fn with_transcript_limits(mut self, capture_bodies: bool, max_bytes: Option<u64>) -> Self {
        self.transcript_capture_bodies = capture_bodies;
        self.transcript_max_bytes = max_bytes;
        self
    }

    /// Load settings from an optional file; defaults to env when None.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        if let Some(p) = path {
            Self::from_file(p)
        } else {
            Ok(Self::from_env())
        }
    }

    /// Validate settings and return detailed errors if invalid.
    ///
    /// Checks:
    /// - rate_window is not zero
    /// - Rate limits are non-zero (0 would disable rate limiting)
    /// - Body size limits are within reasonable bounds (1 byte to 100MB)
    /// - transcript_dir exists and is a directory if provided
    pub fn validate(&self) -> Result<()> {
        for ct in &self.allow_info_content_types {
            if contains_crlf(ct) {
                bail!("allow_info_content_types entries must not contain CR/LF");
            }
        }

        // Validate rate window
        if self.rate_window.is_zero() {
            bail!("rate_window must not be zero");
        }

        // Rate window should be between 100ms and 1 hour
        if self.rate_window.as_millis() < 100 {
            bail!(
                "rate_window too short (minimum 100ms), got {:?}",
                self.rate_window
            );
        }
        if self.rate_window.as_secs() > 3600 {
            bail!(
                "rate_window too long (maximum 1 hour), got {:?}",
                self.rate_window
            );
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
            bail!(
                "info_rate_limit too high (maximum {}), got {}",
                MAX_RATE_LIMIT,
                self.info_rate_limit
            );
        }
        if self.reinvite_rate_limit > MAX_RATE_LIMIT {
            bail!(
                "reinvite_rate_limit too high (maximum {}), got {}",
                MAX_RATE_LIMIT,
                self.reinvite_rate_limit
            );
        }
        if self.update_rate_limit > MAX_RATE_LIMIT {
            bail!(
                "update_rate_limit too high (maximum {}), got {}",
                MAX_RATE_LIMIT,
                self.update_rate_limit
            );
        }

        // Validate body size limits
        const MIN_BODY_SIZE: usize = 1;
        const MAX_BODY_SIZE: usize = 100 * 1024 * 1024; // 100MB

        if self.info_max_body_bytes < MIN_BODY_SIZE {
            bail!(
                "info_max_body_bytes too small (minimum {} bytes), got {}",
                MIN_BODY_SIZE,
                self.info_max_body_bytes
            );
        }
        if self.info_max_body_bytes > MAX_BODY_SIZE {
            bail!(
                "info_max_body_bytes too large (maximum {} bytes / 100MB), got {}",
                MAX_BODY_SIZE,
                self.info_max_body_bytes
            );
        }

        if self.notify_max_body_bytes < MIN_BODY_SIZE {
            bail!(
                "notify_max_body_bytes too small (minimum {} bytes), got {}",
                MIN_BODY_SIZE,
                self.notify_max_body_bytes
            );
        }
        if self.notify_max_body_bytes > MAX_BODY_SIZE {
            bail!(
                "notify_max_body_bytes too large (maximum {} bytes / 100MB), got {}",
                MAX_BODY_SIZE,
                self.notify_max_body_bytes
            );
        }

        // Validate transcript directory if provided
        if let Some(ref dir) = self.transcript_dir {
            if contains_crlf(&dir.to_string_lossy()) {
                bail!("transcript_dir must not contain CR/LF");
            }

            // Path traversal protection: canonicalize path to resolve ".." and symlinks
            let canonical = match dir.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    bail!(
                        "transcript_dir cannot be canonicalized (may not exist or contain invalid path segments): {} (error: {})",
                        dir.display(),
                        e
                    );
                }
            };

            // Verify the canonical path doesn't contain ".." in its string representation
            // This catches attempts to use path traversal sequences
            let path_str = canonical.to_string_lossy();
            if path_str.contains("..") {
                bail!(
                    "transcript_dir contains path traversal sequences (..): {}",
                    dir.display()
                );
            }

            // Use canonical path for remaining checks
            if !canonical.exists() {
                bail!("transcript_dir does not exist: {}", canonical.display());
            }
            if !canonical.is_dir() {
                bail!("transcript_dir is not a directory: {}", canonical.display());
            }

            // Check if writable by attempting to create a test file
            let test_file = canonical.join(".fcp_sip_write_test");
            if let Err(e) = std::fs::write(&test_file, b"test") {
                bail!(
                    "transcript_dir is not writable: {} (error: {})",
                    canonical.display(),
                    e
                );
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

fn contains_crlf(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, b'\r' | b'\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_defaults() {
        let settings = ServerSettings::from_env();
        assert!(settings
            .allow_info_content_types
            .iter()
            .any(|v| v == "application/dtmf-relay"));
    }

    #[test]
    fn test_rate_limit_overrides() {
        let settings = ServerSettings::from_env().with_rate_limits(Duration::from_secs(2), 5, 2, 3);
        assert_eq!(settings.rate_window, Duration::from_secs(2));
        assert_eq!(settings.info_rate_limit, 5);
        assert_eq!(settings.reinvite_rate_limit, 2);
        assert_eq!(settings.update_rate_limit, 3);
    }

    #[test]
    fn test_body_limit_overrides() {
        let settings = ServerSettings::from_env().with_body_limits(1024, 2048);
        assert_eq!(settings.info_max_body_bytes, 1024);
        assert_eq!(settings.notify_max_body_bytes, 2048);
    }

    #[test]
    fn test_valid_default_settings() {
        let settings = ServerSettings::from_env();
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_from_file_overrides() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let toml_str = r#"
allow_refer = false
rate_window_secs = 2
info_rate_limit = 5
transcript_capture_bodies = true
"#;
        std::fs::write(tmp.path(), toml_str).unwrap();
        let settings = ServerSettings::load(Some(tmp.path())).unwrap();
        assert_eq!(settings.allow_refer, false);
        assert_eq!(settings.rate_window, Duration::from_secs(2));
        assert_eq!(settings.info_rate_limit, 5);
        assert_eq!(settings.transcript_capture_bodies, true);
    }

    #[test]
    fn test_zero_rate_window() {
        let settings =
            ServerSettings::from_env().with_rate_limits(Duration::from_secs(0), 10, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("rate_window must not be zero"));
    }

    #[test]
    fn test_rate_window_too_short() {
        let settings =
            ServerSettings::from_env().with_rate_limits(Duration::from_millis(50), 10, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("rate_window too short"));
    }

    #[test]
    fn test_rate_window_too_long() {
        let settings =
            ServerSettings::from_env().with_rate_limits(Duration::from_secs(7200), 10, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("rate_window too long"));
    }

    #[test]
    fn test_zero_info_rate_limit() {
        let settings = ServerSettings::from_env().with_rate_limits(Duration::from_secs(1), 0, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("info_rate_limit must be > 0"));
    }

    #[test]
    fn test_zero_reinvite_rate_limit() {
        let settings =
            ServerSettings::from_env().with_rate_limits(Duration::from_secs(1), 10, 0, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("reinvite_rate_limit must be > 0"));
    }

    #[test]
    fn test_zero_update_rate_limit() {
        let settings =
            ServerSettings::from_env().with_rate_limits(Duration::from_secs(1), 10, 3, 0);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("update_rate_limit must be > 0"));
    }

    #[test]
    fn test_rate_limit_too_high() {
        let settings =
            ServerSettings::from_env().with_rate_limits(Duration::from_secs(1), 2000, 3, 5);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("info_rate_limit too high"));
    }

    #[test]
    fn test_body_size_too_small() {
        let settings = ServerSettings::from_env().with_body_limits(0, 4096);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("info_max_body_bytes too small"));
    }

    #[test]
    fn test_body_size_too_large() {
        let settings = ServerSettings::from_env().with_body_limits(200 * 1024 * 1024, 4096); // 200MB

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("info_max_body_bytes too large"));
    }

    #[test]
    fn test_notify_body_size_too_small() {
        let settings = ServerSettings::from_env().with_body_limits(4096, 0);

        let result = settings.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("notify_max_body_bytes too small"));
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
        let settings =
            ServerSettings::from_env().with_rate_limits(Duration::from_millis(100), 1, 1, 1);
        assert!(settings.validate().is_ok());

        let settings = ServerSettings::from_env().with_rate_limits(
            Duration::from_secs(3600),
            1000,
            1000,
            1000,
        );
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_valid_body_limits_boundary() {
        // Test boundary values
        let settings = ServerSettings::from_env().with_body_limits(1, 1);
        assert!(settings.validate().is_ok());

        let settings =
            ServerSettings::from_env().with_body_limits(100 * 1024 * 1024, 100 * 1024 * 1024);
        assert!(settings.validate().is_ok());
    }
}
