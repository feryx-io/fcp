//! Configuration for fcp-media

use crate::{Error, Result};
use serde::{Deserialize, Serialize};

const MAX_RTP_PORT_SPAN: u16 = 10_000;
const MAX_TIMEOUT_MS: u64 = 120_000;
const MAX_MAX_RETRIES: u32 = 10;

/// Media subsystem configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Backend deployment mode
    #[serde(default)]
    pub backend: BackendType,

    /// RTP port range start
    #[serde(default = "default_rtp_port_start")]
    pub rtp_port_start: u16,

    /// RTP port range end
    #[serde(default = "default_rtp_port_end")]
    pub rtp_port_end: u16,

    /// Enable SRTP encryption
    #[serde(default)]
    pub enable_srtp: bool,

    /// Enable automatic transcoding between codecs
    #[serde(default = "default_true")]
    pub enable_transcoding: bool,

    /// Enable recording capability
    #[serde(default)]
    pub enable_recording: bool,

    /// Enable DTMF detection
    #[serde(default = "default_true")]
    pub enable_dtmf: bool,

    /// Recording directory path
    #[serde(default = "default_recording_dir")]
    pub recording_dir: String,

    /// Recording format (wav or opus)
    #[serde(default = "default_recording_format")]
    pub recording_format: String,

    /// Public IP address for media (for NAT scenarios)
    pub public_ip: Option<String>,

    /// STUN server for NAT traversal (future)
    pub stun_server: Option<String>,

    /// TURN server for NAT traversal (future)
    pub turn_server: Option<String>,
}

/// Backend deployment mode
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BackendType {
    /// Embedded mode: forge-media runs in-process
    /// Best for: Single-box PBX, simple deployments, development
    Embedded,

    /// Remote mode: forge-media runs as separate service
    /// Best for: Distributed scaling, dedicated media servers, conferencing
    Remote {
        /// Base URL of forge-media server
        endpoint: String,

        /// Optional authentication token
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_token: Option<String>,

        /// Connection timeout in milliseconds
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u64,

        /// Enable retry on failure
        #[serde(default = "default_true")]
        retry_enabled: bool,

        /// Maximum retry attempts
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    },
}

impl Default for BackendType {
    fn default() -> Self {
        // Default to embedded for Phase 1
        Self::Embedded
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            backend: BackendType::Embedded,
            rtp_port_start: default_rtp_port_start(),
            rtp_port_end: default_rtp_port_end(),
            enable_srtp: false,
            enable_transcoding: true,
            enable_recording: false,
            enable_dtmf: true,
            recording_dir: default_recording_dir(),
            recording_format: default_recording_format(),
            public_ip: None,
            stun_server: None,
            turn_server: None,
        }
    }
}

impl MediaConfig {
    /// Validates configuration for security and correctness.
    ///
    /// # Security Checks
    /// - **CRLF Injection Prevention**: All string fields are checked for CR/LF characters
    ///   to prevent header injection attacks
    /// - **RTP Port Range**: Validates port range is non-zero, properly ordered, and within
    ///   reasonable bounds (max 10,000 ports) to prevent resource exhaustion
    /// - **Backend Endpoint**: Ensures remote backend URLs use http:// or https:// schemes
    ///   to prevent protocol confusion attacks
    /// - **Timeout Bounds**: Limits timeout to 2 minutes to prevent indefinite hangs
    /// - **Retry Limits**: Caps retry attempts at 10 to prevent infinite retry loops
    ///
    /// # Errors
    /// Returns `Error::Config` if any validation check fails.
    pub fn validate(&self) -> Result<()> {
        if self.rtp_port_start == 0 || self.rtp_port_end == 0 {
            return Err(Error::Config(
                "RTP port range must be greater than zero".to_string(),
            ));
        }
        if self.rtp_port_start > self.rtp_port_end {
            return Err(Error::Config(
                "rtp_port_start cannot exceed rtp_port_end".to_string(),
            ));
        }
        if self.rtp_port_end - self.rtp_port_start > MAX_RTP_PORT_SPAN {
            return Err(Error::Config(format!(
                "RTP port range span is too large (max {} ports)",
                MAX_RTP_PORT_SPAN
            )));
        }

        reject_crlf("recording_dir", &self.recording_dir)?;
        reject_crlf("recording_format", &self.recording_format)?;
        if let Some(ip) = &self.public_ip {
            reject_crlf("public_ip", ip)?;
        }
        if let Some(stun) = &self.stun_server {
            reject_crlf("stun_server", stun)?;
        }
        if let Some(turn) = &self.turn_server {
            reject_crlf("turn_server", turn)?;
        }

        match &self.backend {
            BackendType::Embedded => {}
            BackendType::Remote {
                endpoint,
                auth_token,
                timeout_ms,
                retry_enabled,
                max_retries,
            } => {
                reject_crlf("backend.endpoint", endpoint)?;
                if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
                    return Err(Error::Config(
                        "backend.endpoint must start with http:// or https://".to_string(),
                    ));
                }
                if let Some(token) = auth_token {
                    reject_crlf("backend.auth_token", token)?;
                }
                if *timeout_ms == 0 || *timeout_ms > MAX_TIMEOUT_MS {
                    return Err(Error::Config(format!(
                        "backend.timeout_ms must be between 1 and {}",
                        MAX_TIMEOUT_MS
                    )));
                }
                if *retry_enabled && (*max_retries == 0 || *max_retries > MAX_MAX_RETRIES) {
                    return Err(Error::Config(format!(
                        "backend.max_retries must be between 1 and {} when retry_enabled",
                        MAX_MAX_RETRIES
                    )));
                }
            }
        }

        Ok(())
    }
}

// Default value functions
fn default_rtp_port_start() -> u16 {
    10000
}

fn default_rtp_port_end() -> u16 {
    20000
}

fn default_true() -> bool {
    true
}

fn default_timeout_ms() -> u64 {
    5000
}

fn default_max_retries() -> u32 {
    3
}

fn default_recording_dir() -> String {
    "./recordings".to_string()
}

fn default_recording_format() -> String {
    "wav".to_string()
}

fn reject_crlf(field: &str, value: &str) -> Result<()> {
    if value.bytes().any(|b| matches!(b, b'\r' | b'\n')) {
        Err(Error::Config(format!(
            "{field} must not contain CR or LF characters"
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = MediaConfig::default();
        assert!(matches!(config.backend, BackendType::Embedded));
        assert_eq!(config.rtp_port_start, 10000);
        assert_eq!(config.rtp_port_end, 20000);
        assert!(config.enable_transcoding);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_serialization() {
        let config = MediaConfig {
            backend: BackendType::Remote {
                endpoint: "http://forge:8080".into(),
                auth_token: Some("secret".into()),
                timeout_ms: 5000,
                retry_enabled: true,
                max_retries: 3,
            },
            ..Default::default()
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: MediaConfig = serde_json::from_str(&json).unwrap();

        match deserialized.backend {
            BackendType::Remote { endpoint, .. } => {
                assert_eq!(endpoint, "http://forge:8080");
            }
            _ => panic!("Expected Remote backend"),
        }
    }

    #[test]
    fn test_invalid_port_range() {
        let mut config = MediaConfig::default();
        config.rtp_port_start = 5000;
        config.rtp_port_end = 4000;
        assert!(config.validate().is_err());
    }
}
