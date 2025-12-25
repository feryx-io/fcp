//! Configuration for fcp-media

use serde::{Deserialize, Serialize};

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
}
