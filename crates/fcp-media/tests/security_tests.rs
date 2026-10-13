//! Security-focused tests for fcp-media
//!
//! These tests verify that security validations are working correctly
//! across configuration, session management, and recording operations.

use fcp_media::{MediaConfig, MediaSessionConfig};

#[test]
fn test_crlf_injection_in_recording_dir() {
    let config = MediaConfig {
        recording_dir: "/tmp/recordings\r\ninjected".to_string(),
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject CRLF in recording_dir"
    );
}

#[test]
fn test_crlf_injection_in_recording_format() {
    let config = MediaConfig {
        recording_format: "wav\ninjection".to_string(),
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject CRLF in recording_format"
    );
}

#[test]
fn test_crlf_injection_in_public_ip() {
    let config = MediaConfig {
        public_ip: Some("192.168.1.1\r\nevil".to_string()),
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject CRLF in public_ip"
    );
}

#[test]
fn test_crlf_injection_in_stun_server() {
    let config = MediaConfig {
        stun_server: Some("stun.example.com\ninjection".to_string()),
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject CRLF in stun_server"
    );
}

#[test]
fn test_crlf_injection_in_turn_server() {
    let config = MediaConfig {
        turn_server: Some("turn.example.com\r\ninjection".to_string()),
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject CRLF in turn_server"
    );
}

#[test]
fn test_invalid_port_range_reversed() {
    let config = MediaConfig {
        rtp_port_start: 20000,
        rtp_port_end: 10000,
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject reversed port range"
    );
}

#[test]
fn test_invalid_port_range_zero() {
    let mut config = MediaConfig {
        rtp_port_start: 0,
        ..Default::default()
    };
    assert!(config.validate().is_err(), "Should reject zero port start");

    config.rtp_port_start = 10000;
    config.rtp_port_end = 0;
    assert!(config.validate().is_err(), "Should reject zero port end");
}

#[test]
fn test_excessive_port_range() {
    let mut config = MediaConfig {
        rtp_port_start: 10000,
        ..Default::default()
    };
    config.rtp_port_end = 30000; // 20,000 ports > MAX_RTP_PORT_SPAN (10,000)
    assert!(
        config.validate().is_err(),
        "Should reject excessive port range"
    );
}

// Session configuration tests

#[test]
fn test_session_crlf_injection_in_recording_path() {
    let config = MediaSessionConfig {
        enable_recording: true,
        recording_path: Some("path\r\ninjection".to_string()),
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject CRLF in recording path"
    );
}

#[test]
fn test_session_empty_recording_path() {
    let config = MediaSessionConfig {
        enable_recording: true,
        recording_path: Some("".to_string()),
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject empty recording path when recording enabled"
    );
}

#[test]
fn test_session_path_traversal_parent_dir() {
    let config = MediaSessionConfig {
        enable_recording: true,
        recording_path: Some("../etc/passwd".to_string()),
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject .. in recording path"
    );
}

#[test]
fn test_session_path_traversal_absolute_path() {
    let config = MediaSessionConfig {
        enable_recording: true,
        recording_path: Some("/etc/passwd".to_string()),
        ..Default::default()
    };
    assert!(config.validate().is_err(), "Should reject absolute paths");
}

#[test]
#[cfg(windows)]
fn test_session_path_traversal_windows_drive() {
    let config = MediaSessionConfig {
        enable_recording: true,
        recording_path: Some("C:\\Windows\\System32".to_string()),
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject Windows drive prefixes"
    );
}

#[test]
fn test_session_valid_relative_path() {
    let config = MediaSessionConfig {
        enable_recording: true,
        recording_path: Some("user123/call456.wav".to_string()),
        ..Default::default()
    };
    assert!(
        config.validate().is_ok(),
        "Should accept valid relative path"
    );
}

#[test]
fn test_excessive_codec_list() {
    let config = MediaSessionConfig {
        preferred_codecs: vec!["PCMU".to_string(); 33], // > 32 limit
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject excessive codec list"
    );
}

#[test]
fn test_valid_codec_list() {
    let config = MediaSessionConfig {
        preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string(), "opus".to_string()],
        ..Default::default()
    };
    assert!(
        config.validate().is_ok(),
        "Should accept reasonable codec list"
    );
}

#[test]
fn test_invalid_jitter_buffer_zero() {
    let config = MediaSessionConfig {
        jitter_buffer_ms: 0,
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject zero jitter buffer"
    );
}

#[test]
fn test_invalid_jitter_buffer_excessive() {
    let config = MediaSessionConfig {
        jitter_buffer_ms: 5001,
        ..Default::default()
    };
    assert!(
        config.validate().is_err(),
        "Should reject excessive jitter buffer"
    );
}

#[test]
fn test_valid_jitter_buffer_bounds() {
    let mut config = MediaSessionConfig {
        jitter_buffer_ms: 1,
        ..Default::default()
    };

    assert!(config.validate().is_ok(), "Should accept minimum value");

    config.jitter_buffer_ms = 5000;
    assert!(config.validate().is_ok(), "Should accept maximum value");

    config.jitter_buffer_ms = 50;
    assert!(config.validate().is_ok(), "Should accept typical value");
}

#[test]
fn test_recording_disabled_allows_invalid_path() {
    let config = MediaSessionConfig {
        enable_recording: false,
        recording_path: Some("../traversal".to_string()),
        ..Default::default()
    };
    // Path validation is skipped when recording is disabled
    assert!(
        config.validate().is_ok(),
        "Should not validate path when recording disabled"
    );
}

// Integration tests with MediaManager would go here if we had a test backend
// For now, these config-level tests provide comprehensive security coverage
