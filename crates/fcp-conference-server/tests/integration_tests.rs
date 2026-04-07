//! Integration tests for fcp-conference-server.
//!
//! These tests exercise configuration loading, parsing, and validation
//! without requiring a running SIP stack or database.

use std::io::Write;
use tempfile::NamedTempFile;

/// Helper to parse a TOML config string into a ConferenceServerConfig.
/// We test the config module by using toml deserialization directly.
fn parse_config(toml_str: &str) -> fcp_conference_server::ConferenceServerConfig {
    toml::from_str(toml_str).expect("Failed to parse config")
}

// =========================================================================
// Config Parsing Tests
// =========================================================================

#[test]
fn test_minimal_config_defaults() {
    let config = parse_config("");
    assert_eq!(config.service.service_type, "conference");
    assert_eq!(config.sip.udp_bind, "0.0.0.0:5060");
    assert_eq!(config.sip.tcp_bind, "0.0.0.0:5060");
    assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
    assert_eq!(config.http.bind_address.port(), 8090);
    assert_eq!(config.conference.sample_rate, 8000);
    assert_eq!(config.conference.frame_size, 160);
    assert!(config.conference.enable_vad);
    assert!(config.conference.auto_destroy_empty_rooms);
    assert_eq!(config.conference.max_pin_attempts, 3);
    assert_eq!(config.conference.pin_timeout_secs, 10);
    assert_eq!(config.conference.jitter_buffer_ms, 40);
    assert!(config.conference.dtmf_enabled);
    assert_eq!(config.conference.cache_ttl_secs, 5);
    assert!(config.conference.recording_retention_days.is_none());
    assert!(config.conference.allowed_codecs.is_none());
    assert!(!config.conference.enable_srtp);
    assert_eq!(
        config.conference.announcements_dir,
        "/var/fcp/conference/announcements"
    );
    assert_eq!(config.conference.max_wait_for_moderator_secs, 300);
    assert_eq!(config.conference.session_expires_secs, 1800);
    assert_eq!(config.conference.max_recording_size_mb, 500);
    assert!(!config.ai.enabled);
    assert_eq!(config.ai.model, "gpt-4o-realtime-preview");
    assert_eq!(config.ai.voice, "alloy");
}

#[test]
fn test_full_config_override() {
    let config = parse_config(
        r#"
[service]
service_type = "conference"
node_id = "prod-conf-01"

[sip]
udp_bind = "10.0.0.1:5080"
tcp_bind = "10.0.0.1:5080"
local_ip = "10.0.0.1"
local_uri = "sip:conference@10.0.0.1:5080"

[storage]
database_url = "postgres://user:pass@db:5432/fcp"
max_connections = 20

[http]
bind_address = "0.0.0.0:9090"
auth_enabled = true
auth_token = "secret-token-123"

[media]
rtp_port_start = 30000
rtp_port_end = 40000
codecs = ["PCMU", "G722"]
public_ip = "203.0.113.10"

[conference]
sample_rate = 16000
frame_size = 320
enable_vad = false
default_max_participants = 50
auto_destroy_empty_rooms = false
recordings_dir = "/mnt/recordings"
pin_timeout_secs = 30
max_pin_attempts = 5
jitter_buffer_ms = 80
dtmf_enabled = false
cache_ttl_secs = 120
recording_retention_days = 30
allowed_codecs = ["PCMU", "PCMA"]

[ai]
enabled = true
api_key = "sk-test-key"
model = "gpt-4o-mini"
voice = "nova"
instructions = "You are a conference assistant."
endpoint = "https://api.custom.ai/v1"

[events]
server_url = "https://events.example.com/webhook"

[logging]
level = "debug"
format = "compact"
"#,
    );

    assert_eq!(config.service.node_id, "prod-conf-01");
    assert_eq!(config.sip.udp_bind, "10.0.0.1:5080");
    assert_eq!(config.sip.local_ip.as_deref(), Some("10.0.0.1"));
    assert_eq!(
        config.sip.local_uri.as_deref(),
        Some("sip:conference@10.0.0.1:5080")
    );
    assert_eq!(
        config.storage.database_url,
        "postgres://user:pass@db:5432/fcp"
    );
    assert_eq!(config.storage.max_connections, 20);
    assert_eq!(config.http.bind_address.port(), 9090);
    assert!(config.http.auth_enabled);
    assert_eq!(config.http.auth_token.as_deref(), Some("secret-token-123"));
    assert_eq!(config.media.rtp_port_start, 30000);
    assert_eq!(config.media.rtp_port_end, 40000);
    assert_eq!(config.media.public_ip.as_deref(), Some("203.0.113.10"));
    assert_eq!(config.conference.sample_rate, 16000);
    assert_eq!(config.conference.frame_size, 320);
    assert!(!config.conference.enable_vad);
    assert_eq!(config.conference.default_max_participants, 50);
    assert!(!config.conference.auto_destroy_empty_rooms);
    assert_eq!(config.conference.recordings_dir, "/mnt/recordings");
    assert_eq!(config.conference.pin_timeout_secs, 30);
    assert_eq!(config.conference.max_pin_attempts, 5);
    assert_eq!(config.conference.jitter_buffer_ms, 80);
    assert!(!config.conference.dtmf_enabled);
    assert_eq!(config.conference.cache_ttl_secs, 120);
    assert_eq!(config.conference.recording_retention_days, Some(30));
    assert_eq!(
        config.conference.allowed_codecs,
        Some(vec!["PCMU".to_string(), "PCMA".to_string()])
    );
    assert!(config.ai.enabled);
    assert_eq!(config.ai.api_key.as_deref(), Some("sk-test-key"));
    assert_eq!(config.ai.model, "gpt-4o-mini");
    assert_eq!(config.ai.voice, "nova");
    assert_eq!(
        config.ai.instructions.as_deref(),
        Some("You are a conference assistant.")
    );
    assert_eq!(
        config.ai.endpoint.as_deref(),
        Some("https://api.custom.ai/v1")
    );
    assert_eq!(
        config.events.server_url.as_deref(),
        Some("https://events.example.com/webhook")
    );
    assert_eq!(config.logging.level, "debug");
    assert_eq!(config.logging.format, "compact");
}

#[test]
fn test_partial_config_keeps_defaults() {
    let config = parse_config(
        r#"
[conference]
max_pin_attempts = 10
"#,
    );

    // Overridden field
    assert_eq!(config.conference.max_pin_attempts, 10);
    // Remaining defaults preserved
    assert_eq!(config.conference.sample_rate, 8000);
    assert_eq!(config.conference.frame_size, 160);
    assert!(config.conference.enable_vad);
    assert!(config.conference.dtmf_enabled);
    assert_eq!(config.conference.jitter_buffer_ms, 40);
}

#[test]
fn test_config_roundtrip_serialization() {
    let config = parse_config("");
    let serialized = toml::to_string(&config).expect("Failed to serialize config");
    let deserialized: fcp_conference_server::ConferenceServerConfig =
        toml::from_str(&serialized).expect("Failed to deserialize config");
    assert_eq!(
        config.conference.sample_rate,
        deserialized.conference.sample_rate
    );
    assert_eq!(
        config.conference.frame_size,
        deserialized.conference.frame_size
    );
    assert_eq!(
        config.conference.max_pin_attempts,
        deserialized.conference.max_pin_attempts
    );
    assert_eq!(config.ai.model, deserialized.ai.model);
}

#[test]
fn test_config_with_trunks() {
    let config = parse_config(
        r#"
[[trunks]]
name = "carrier1"
enabled = true
direction = "Upstream"
hosts = ["sip.carrier1.com"]
weight = 100
priority = 1
register = false
max_concurrent_calls = 0

[trunks.transport]
protocol = "Udp"
verify_tls = false

[trunks.failover]
max_failures = 3
recovery_interval_secs = 30
health_check_enabled = false
health_check_interval_secs = 60
"#,
    );

    assert_eq!(config.trunks.len(), 1);
    assert_eq!(config.trunks[0].name, "carrier1");
    assert_eq!(config.trunks[0].hosts, vec!["sip.carrier1.com".to_string()]);
}

#[test]
fn test_config_load_from_file() {
    let mut tmpfile = NamedTempFile::new().expect("Failed to create temp file");
    write!(
        tmpfile,
        r#"
[service]
node_id = "file-test-01"

[conference]
max_pin_attempts = 7
jitter_buffer_ms = 60
"#
    )
    .expect("Failed to write temp config");

    let config =
        fcp_conference_server::load_config(Some(tmpfile.path())).expect("Failed to load config");
    assert_eq!(config.service.node_id, "file-test-01");
    assert_eq!(config.conference.max_pin_attempts, 7);
    assert_eq!(config.conference.jitter_buffer_ms, 60);
    // Defaults preserved
    assert_eq!(config.conference.sample_rate, 8000);
}

#[test]
fn test_config_load_invalid_file() {
    let mut tmpfile = NamedTempFile::new().expect("Failed to create temp file");
    write!(tmpfile, "this is not valid {{{{ toml").expect("Failed to write");

    let result = fcp_conference_server::load_config(Some(tmpfile.path()));
    assert!(result.is_err());
}

// =========================================================================
// Storage Backend Feature Tests
// =========================================================================

#[test]
fn test_storage_config_custom_url() {
    let config = parse_config(
        r#"
[storage]
database_url = "postgresql://user:pass@db:5432/mydb"
max_connections = 100
"#,
    );
    assert_eq!(
        config.storage.database_url,
        "postgresql://user:pass@db:5432/mydb"
    );
    assert_eq!(config.storage.max_connections, 100);
}

// =========================================================================
// Conference Section Validation
// =========================================================================

#[test]
fn test_allowed_codecs_empty_list() {
    let config = parse_config(
        r#"
[conference]
allowed_codecs = []
"#,
    );
    assert_eq!(config.conference.allowed_codecs, Some(vec![]));
}

#[test]
fn test_recording_retention_zero() {
    let config = parse_config(
        r#"
[conference]
recording_retention_days = 0
"#,
    );
    assert_eq!(config.conference.recording_retention_days, Some(0));
}

#[test]
fn test_cache_ttl_zero_disables_cache() {
    let config = parse_config(
        r#"
[conference]
cache_ttl_secs = 0
"#,
    );
    assert_eq!(config.conference.cache_ttl_secs, 0);
}

// =========================================================================
// AI Section Tests
// =========================================================================

#[test]
fn test_ai_disabled_by_default() {
    let config = parse_config("");
    assert!(!config.ai.enabled);
    assert!(config.ai.api_key.is_none());
    assert!(config.ai.instructions.is_none());
    assert!(config.ai.endpoint.is_none());
}

#[test]
fn test_ai_enabled_without_key() {
    let config = parse_config(
        r#"
[ai]
enabled = true
"#,
    );
    assert!(config.ai.enabled);
    assert!(config.ai.api_key.is_none());
}

// =========================================================================
// HTTP Auth Config Tests
// =========================================================================

#[test]
fn test_auth_disabled_by_default() {
    let config = parse_config("");
    assert!(!config.http.auth_enabled);
    assert!(config.http.auth_token.is_none());
}

#[test]
fn test_auth_enabled_with_token() {
    let config = parse_config(
        r#"
[http]
auth_enabled = true
auth_token = "my-secret-token"
"#,
    );
    assert!(config.http.auth_enabled);
    assert_eq!(config.http.auth_token.as_deref(), Some("my-secret-token"));
}

#[test]
fn test_rate_limit_config() {
    let config = parse_config("");
    assert!(config.http.rate_limit_rps.is_none());

    let config = parse_config(
        r#"
[http]
rate_limit_rps = 100
"#,
    );
    assert_eq!(config.http.rate_limit_rps, Some(100));
}

// =========================================================================
// SRTP / Security Config Tests
// =========================================================================

#[test]
fn test_srtp_disabled_by_default() {
    let config = parse_config("");
    assert!(!config.conference.enable_srtp);
}

#[test]
fn test_srtp_enabled() {
    let config = parse_config(
        r#"
[conference]
enable_srtp = true
"#,
    );
    assert!(config.conference.enable_srtp);
}

// =========================================================================
// Announcements Config Tests
// =========================================================================

#[test]
fn test_announcements_dir_default() {
    let config = parse_config("");
    assert_eq!(
        config.conference.announcements_dir,
        "/var/fcp/conference/announcements"
    );
}

#[test]
fn test_announcements_dir_custom() {
    let config = parse_config(
        r#"
[conference]
announcements_dir = "/opt/sounds"
"#,
    );
    assert_eq!(config.conference.announcements_dir, "/opt/sounds");
}

// =========================================================================
// Config Validation Tests
// =========================================================================

#[test]
fn test_validate_default_config_passes() {
    let config = parse_config("");
    assert!(fcp_conference_server::validate_config(&config).is_ok());
}

#[test]
fn test_validate_invalid_rtp_port_range() {
    let config = parse_config(
        r#"
[media]
rtp_port_start = 30000
rtp_port_end = 20000
"#,
    );
    let result = fcp_conference_server::validate_config(&config);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("rtp_port_start"));
}

#[test]
fn test_validate_equal_rtp_ports() {
    let config = parse_config(
        r#"
[media]
rtp_port_start = 20000
rtp_port_end = 20000
"#,
    );
    let result = fcp_conference_server::validate_config(&config);
    assert!(result.is_err());
}

#[test]
fn test_validate_invalid_sample_rate() {
    let config = parse_config(
        r#"
[conference]
sample_rate = 44100
"#,
    );
    let result = fcp_conference_server::validate_config(&config);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("sample_rate"));
}

#[test]
fn test_validate_unknown_codec() {
    let config = parse_config(
        r#"
[conference]
allowed_codecs = ["PCMU", "INVALID_CODEC"]
"#,
    );
    let result = fcp_conference_server::validate_config(&config);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("INVALID_CODEC"));
}

#[test]
fn test_storage_default_url() {
    let config = parse_config("");
    assert_eq!(config.storage.database_url, "postgresql://localhost/fcp");
}

#[test]
fn test_validate_valid_16khz_config() {
    let config = parse_config(
        r#"
[conference]
sample_rate = 16000
frame_size = 320
"#,
    );
    assert!(fcp_conference_server::validate_config(&config).is_ok());
}

// =========================================================================
// Wait-for-Moderator Timeout Config Tests
// =========================================================================

#[test]
fn test_wait_for_moderator_default() {
    let config = parse_config("");
    assert_eq!(config.conference.max_wait_for_moderator_secs, 300);
}

#[test]
fn test_wait_for_moderator_custom() {
    let config = parse_config(
        r#"
[conference]
max_wait_for_moderator_secs = 600
"#,
    );
    assert_eq!(config.conference.max_wait_for_moderator_secs, 600);
}

#[test]
fn test_wait_for_moderator_unlimited() {
    let config = parse_config(
        r#"
[conference]
max_wait_for_moderator_secs = 0
"#,
    );
    assert_eq!(config.conference.max_wait_for_moderator_secs, 0);
}
