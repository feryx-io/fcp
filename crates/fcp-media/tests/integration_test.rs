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
