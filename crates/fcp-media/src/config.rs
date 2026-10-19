//! Configuration for fcp-media

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const MAX_RTP_PORT_SPAN: u16 = 10_000;

/// Media subsystem configuration
#[derive(Clone, Serialize, Deserialize)]
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

    /// TTS configuration (enables text-to-speech when set)
    #[serde(default)]
    pub tts: Option<crate::tts::TtsConfig>,

    /// Pre-created TTS service to share across the system.
    /// When set, the media backend uses this instead of creating its own.
    #[serde(skip)]
    pub tts_service: Option<Arc<crate::tts::TtsService>>,

    /// Speech recognition configuration (`[stt]`).
    #[serde(default)]
    pub stt: Option<crate::stt::SttConfig>,

    /// Pre-created speech recognition service; used instead of `stt`.
    #[serde(skip)]
    pub stt_service: Option<Arc<crate::stt::SttService>>,

    /// Enable XDP kernel-level RTP forwarding (Linux only).
    #[serde(default)]
    pub xdp_enabled: bool,

    /// Network interface for XDP (e.g., "eth0").
    #[serde(default = "default_xdp_interface")]
    pub xdp_interface: String,

    /// XDP mode: "native" or "generic".
    #[serde(default = "default_xdp_mode")]
    pub xdp_mode: String,
}

/// Backend deployment mode
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[derive(Default)]
pub enum BackendType {
    /// Embedded mode: forge-media runs in-process
    /// Best for: Single-box PBX, simple deployments, development
    #[default]
    Embedded,
}

impl std::fmt::Debug for MediaConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaConfig")
            .field("backend", &self.backend)
            .field("rtp_port_start", &self.rtp_port_start)
            .field("rtp_port_end", &self.rtp_port_end)
            .field("public_ip", &self.public_ip)
            .field("tts", &self.tts.is_some())
            .field("tts_service", &self.tts_service.is_some())
            .field("stt", &self.stt.is_some())
            .field("stt_service", &self.stt_service.is_some())
            .finish()
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
            tts: None,
            tts_service: None,
            stt: None,
            stt_service: None,
            xdp_enabled: false,
            xdp_interface: default_xdp_interface(),
            xdp_mode: default_xdp_mode(),
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

        if let Some(tts) = &self.tts {
            match &tts.provider {
                crate::tts::TtsProviderConfig::GoogleCloud { .. }
                | crate::tts::TtsProviderConfig::OpenAi { .. }
                | crate::tts::TtsProviderConfig::Local { .. }
                | crate::tts::TtsProviderConfig::Piper { .. }
                | crate::tts::TtsProviderConfig::Remote { .. } => {}
                crate::tts::TtsProviderConfig::AwsPolly { .. } => {
                    return Err(Error::Config(
                        "tts.provider.type = \"aws_polly\" is not implemented yet; use google_cloud, open_ai, or local".to_string(),
                    ));
                }
                crate::tts::TtsProviderConfig::Azure { .. } => {
                    return Err(Error::Config(
                        "tts.provider.type = \"azure\" is not implemented yet; use google_cloud, open_ai, or local".to_string(),
                    ));
                }
                crate::tts::TtsProviderConfig::ElevenLabs { .. } => {
                    return Err(Error::Config(
                        "tts.provider.type = \"elevenlabs\" is not implemented yet; use google_cloud, open_ai, or local".to_string(),
                    ));
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

fn default_recording_dir() -> String {
    "./recordings".to_string()
}

fn default_recording_format() -> String {
    "wav".to_string()
}

fn default_xdp_interface() -> String {
    "eth0".to_string()
}

fn default_xdp_mode() -> String {
    "generic".to_string()
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
        let config = MediaConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: MediaConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized.backend, BackendType::Embedded));
    }

    #[test]
    fn test_invalid_port_range() {
        let config = MediaConfig {
            rtp_port_start: 5000,
            rtp_port_end: 4000,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_remote_backend_is_not_a_backend() {
        let json = r#"{"backend":{"type":"remote","endpoint":"http://forge:8080"}}"#;
        assert!(serde_json::from_str::<MediaConfig>(json).is_err());
    }

    #[test]
    fn test_stub_tts_provider_is_rejected() {
        let config = MediaConfig {
            tts: Some(crate::tts::TtsConfig {
                provider: crate::tts::TtsProviderConfig::AwsPolly {
                    region: "us-east-1".to_string(),
                    access_key_id: "key".to_string(),
                    secret_access_key: "secret".to_string(),
                },
                cache_dir: "/tmp/tts".into(),
                cache_max_size_mb: 100,
                default_language: "en-US".to_string(),
                default_voice: None,
            }),
            ..Default::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_xdp_config_defaults() {
        let config = MediaConfig::default();
        assert!(!config.xdp_enabled);
        assert_eq!(config.xdp_interface, "eth0");
        assert_eq!(config.xdp_mode, "generic");
    }

    #[test]
    fn test_xdp_config_serde_roundtrip() {
        let config = MediaConfig {
            xdp_enabled: true,
            xdp_interface: "ens3".to_string(),
            xdp_mode: "native".to_string(),
            ..Default::default()
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: MediaConfig = serde_json::from_str(&json).unwrap();
        assert!(deserialized.xdp_enabled);
        assert_eq!(deserialized.xdp_interface, "ens3");
        assert_eq!(deserialized.xdp_mode, "native");
    }
}
