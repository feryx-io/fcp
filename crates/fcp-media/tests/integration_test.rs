//! Integration tests for fcp-media

use fcp_core::{CallId, DialogId};
use fcp_media::{BackendType, MediaConfig, MediaManager, MediaSessionConfig, MediaSessionState};

#[tokio::test]
async fn test_create_embedded_backend() {
    // Create config for embedded mode
    let config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: 20000,
        rtp_port_end: 21000,
        ..Default::default()
    };

    // Create media manager
    let manager = MediaManager::new(config).await.expect("Failed to create MediaManager");

    // Verify backend type
    assert_eq!(manager.backend_name(), "embedded");
}

#[tokio::test]
async fn test_session_lifecycle() {
    // Create manager
    let config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: 22000,
        rtp_port_end: 23000,
        ..Default::default()
    };

    let manager = MediaManager::new(config).await.expect("Failed to create manager");

    // Create session
    let call_id = CallId::new();
    let dialog_id = DialogId::new();

    let session = manager
        .create_session_default(call_id.clone(), dialog_id.clone())
        .await
        .expect("Failed to create session");

    // Verify session state
    assert_eq!(session.state().await, MediaSessionState::Active);
    assert_eq!(session.call_id, call_id);
    assert_eq!(session.dialog_id, dialog_id);

    // Get session
    let retrieved = manager
        .get_session(&call_id)
        .await
        .expect("Failed to get session");

    assert_eq!(retrieved.call_id, call_id);

    // Terminate session
    manager
        .terminate_session(&call_id)
        .await
        .expect("Failed to terminate session");

    // Verify session is gone
    let result = manager.get_session(&call_id).await;
    assert!(result.is_err(), "Session should be terminated");
}

#[tokio::test]
async fn test_multiple_sessions() {
    let config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: 24000,
        rtp_port_end: 25000,
        ..Default::default()
    };

    let manager = MediaManager::new(config).await.expect("Failed to create manager");

    // Create multiple sessions
    let call_id_1 = CallId::new();
    let call_id_2 = CallId::new();
    let call_id_3 = CallId::new();

    let session_config = MediaSessionConfig {
        preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
        ..Default::default()
    };

    manager
        .create_session(call_id_1.clone(), DialogId::new(), session_config.clone())
        .await
        .expect("Failed to create session 1");

    manager
        .create_session(call_id_2.clone(), DialogId::new(), session_config.clone())
        .await
        .expect("Failed to create session 2");

    manager
        .create_session(call_id_3.clone(), DialogId::new(), session_config)
        .await
        .expect("Failed to create session 3");

    // Verify all sessions exist
    assert!(manager.get_session(&call_id_1).await.is_ok());
    assert!(manager.get_session(&call_id_2).await.is_ok());
    assert!(manager.get_session(&call_id_3).await.is_ok());

    // Terminate all
    manager.terminate_session(&call_id_1).await.unwrap();
    manager.terminate_session(&call_id_2).await.unwrap();
    manager.terminate_session(&call_id_3).await.unwrap();

    // Verify all gone
    assert!(manager.get_session(&call_id_1).await.is_err());
    assert!(manager.get_session(&call_id_2).await.is_err());
    assert!(manager.get_session(&call_id_3).await.is_err());
}

#[tokio::test]
async fn test_health_check() {
    let config = MediaConfig::default();
    let manager = MediaManager::new(config).await.expect("Failed to create manager");

    let health = manager.health_check().await.expect("Health check failed");
    assert!(health, "Backend should be healthy");
}

#[tokio::test]
async fn test_session_with_opus_codec() {
    let config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: 26000,
        rtp_port_end: 27000,
        enable_transcoding: true,
        ..Default::default()
    };

    let manager = MediaManager::new(config).await.expect("Failed to create manager");

    let session_config = MediaSessionConfig {
        preferred_codecs: vec!["opus".to_string()],
        enable_transcoding: true,
        ..Default::default()
    };

    let call_id = CallId::new();
    let session = manager
        .create_session(call_id.clone(), DialogId::new(), session_config)
        .await
        .expect("Failed to create opus session");

    assert_eq!(session.state().await, MediaSessionState::Active);

    manager.terminate_session(&call_id).await.unwrap();
}

#[tokio::test]
async fn test_recording_lifecycle() {
    use std::path::PathBuf;
    use tempfile::TempDir;

    // Create temp directory for recordings
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let recording_path = temp_dir.path().join("test_recording.wav");

    // Create manager
    let config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: 28000,
        rtp_port_end: 29000,
        enable_recording: true,
        recording_dir: temp_dir.path().to_str().unwrap().to_string(),
        recording_format: "wav".to_string(),
        ..Default::default()
    };

    let manager = MediaManager::new(config).await.expect("Failed to create manager");

    // Create session
    let call_id = CallId::new();
    let session = manager
        .create_session_default(call_id.clone(), DialogId::new())
        .await
        .expect("Failed to create session");

    assert_eq!(session.state().await, MediaSessionState::Active);

    // Start recording
    manager
        .start_recording(&call_id, recording_path.to_str().unwrap().to_string())
        .await
        .expect("Failed to start recording");

    // Wait a bit to simulate call in progress
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Stop recording
    manager
        .stop_recording(&call_id)
        .await
        .expect("Failed to stop recording");

    // Verify recording file was created
    assert!(
        recording_path.exists(),
        "Recording file should exist at {:?}",
        recording_path
    );

    // Check file size (should have WAV header at minimum)
    let metadata = std::fs::metadata(&recording_path).expect("Failed to get file metadata");
    assert!(metadata.len() >= 44, "WAV file should have at least 44 bytes for header");

    // Terminate session
    manager
        .terminate_session(&call_id)
        .await
        .expect("Failed to terminate session");

    // Cleanup happens automatically when temp_dir is dropped
}

#[tokio::test]
#[ignore] // Opus support is optional - enable with --features opus
async fn test_recording_with_opus_format() {
    use tempfile::TempDir;

    // Create temp directory for recordings
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let recording_path = temp_dir.path().join("test_recording.opus");

    // Create manager
    let config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: 30000,
        rtp_port_end: 31000,
        enable_recording: true,
        ..Default::default()
    };

    let manager = MediaManager::new(config).await.expect("Failed to create manager");

    // Create session
    let call_id = CallId::new();
    manager
        .create_session_default(call_id.clone(), DialogId::new())
        .await
        .expect("Failed to create session");

    // Start recording with .opus extension
    // Note: This will fail if forge-recorder isn't built with opus feature
    let result = manager
        .start_recording(&call_id, recording_path.to_str().unwrap().to_string())
        .await;

    if result.is_err() {
        eprintln!("Opus recording not supported - skipping test");
        manager.terminate_session(&call_id).await.unwrap();
        return;
    }

    // Wait briefly
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Stop recording
    manager
        .stop_recording(&call_id)
        .await
        .expect("Failed to stop recording");

    // Verify recording file was created
    assert!(
        recording_path.exists(),
        "Recording file should exist at {:?}",
        recording_path
    );

    // Terminate session
    manager.terminate_session(&call_id).await.unwrap();
}
