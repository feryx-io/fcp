mod common;

use common::{build_invite_with_sdp, minimal_sdp, TestHarness};
use fcp_pbx::config::{HuntGroupConfig, RingStrategy};
use std::time::Duration;

fn enabled_pbx_config() -> fcp_pbx::PbxConfig {
    fcp_pbx::PbxConfig {
        enabled: true,
        feature_codes: fcp_pbx::config::FeatureCodeConfig::default(),
        ..Default::default()
    }
}

fn disabled_pbx_config() -> fcp_pbx::PbxConfig {
    fcp_pbx::PbxConfig {
        enabled: false,
        ..Default::default()
    }
}

/// Helper: build INVITE to a feature code URI.
fn invite_to_feature_code(code: &str) -> sip_core::Request {
    // Feature codes are dialed as sip:*8@domain, etc.
    build_invite_with_sdp(
        "alice@example.com",
        &format!("{}@example.com", code),
        Some(minimal_sdp()),
    )
}

#[tokio::test]
async fn test_feature_code_group_pickup_recognized() {
    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(enabled_pbx_config())
        .build()
        .await;

    let request = invite_to_feature_code("*8");
    harness.send_request(&request).await;

    // Should get 100 Trying + a final response (480 no ringing call, since
    // there are no ringing calls to pick up)
    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    assert!(
        final_resp.is_some(),
        "Expected a final response for group pickup"
    );
    // 480 "No Ringing Call" because there are no ringing calls
    let code = final_resp.unwrap().start_line().code();
    assert_eq!(
        code, 480,
        "Expected 480 for group pickup with no ringing calls, got {}",
        code
    );
}

#[tokio::test]
async fn test_feature_code_directed_pickup() {
    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(enabled_pbx_config())
        .build()
        .await;

    // *9 is directed pickup prefix, 1001 is target extension
    let request = invite_to_feature_code("*91001");
    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    assert!(
        final_resp.is_some(),
        "Expected a final response for directed pickup"
    );
    let code = final_resp.unwrap().start_line().code();
    // 480 "No Ringing Call" because no call is ringing at ext 1001
    assert_eq!(
        code, 480,
        "Expected 480 for directed pickup with no ringing call at 1001, got {}",
        code
    );
}

#[tokio::test]
async fn test_feature_code_park_call() {
    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(enabled_pbx_config())
        .build()
        .await;

    // *70 is park call prefix
    let request = invite_to_feature_code("*70");
    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    assert!(
        final_resp.is_some(),
        "Expected a final response for park call"
    );
    // Park with no active call should return 480 "No Active Call"
    let code = final_resp.unwrap().start_line().code();
    assert_eq!(
        code, 480,
        "Expected 480 for park with no active call, got {}",
        code
    );
}

#[tokio::test]
async fn test_feature_code_retrieve_parked() {
    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(enabled_pbx_config())
        .build()
        .await;

    // *71 is retrieve prefix, 701 is orbit
    let request = invite_to_feature_code("*71701");
    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    assert!(
        final_resp.is_some(),
        "Expected a final response for retrieve parked"
    );
    // No parked call at orbit 701 → 480 "Orbit Not Occupied"
    let code = final_resp.unwrap().start_line().code();
    assert_eq!(
        code, 480,
        "Expected 480 for retrieve from empty orbit, got {}",
        code
    );
}

#[tokio::test]
async fn test_feature_code_intercom_oneway() {
    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(enabled_pbx_config())
        .build()
        .await;

    // *80 is one-way intercom prefix, 1001 is target
    let request = invite_to_feature_code("*801001");
    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    assert!(
        final_resp.is_some(),
        "Expected a final response for intercom one-way"
    );
    // Should get an error since 1001 is not registered
    let code = final_resp.unwrap().start_line().code();
    assert!(
        code >= 400,
        "Expected error for intercom to unregistered ext, got {}",
        code
    );
}

#[tokio::test]
async fn test_feature_code_intercom_twoway() {
    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(enabled_pbx_config())
        .build()
        .await;

    // *81 is two-way intercom prefix, 1001 is target
    let request = invite_to_feature_code("*811001");
    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    assert!(
        final_resp.is_some(),
        "Expected a final response for intercom two-way"
    );
    let code = final_resp.unwrap().start_line().code();
    assert!(
        code >= 400,
        "Expected error for intercom to unregistered ext, got {}",
        code
    );
}

#[tokio::test]
async fn test_hunt_group_extension_resolved() {
    let mut config = enabled_pbx_config();
    // Configure a hunt group at extension 5000
    config.hunt_groups.insert(
        "5000".to_string(),
        HuntGroupConfig {
            name: "Sales".to_string(),
            strategy: RingStrategy::Sequential,
            members: vec!["1001".to_string(), "1002".to_string(), "1003".to_string()],
            ring_timeout: 20,
            total_timeout: 30,
            no_answer_destination: None,
            skip_busy: false,
        },
    );

    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(config)
        .build()
        .await;

    // INVITE to hunt group extension (not a feature code)
    let request = build_invite_with_sdp(
        "alice@example.com",
        "5000@example.com",
        Some(minimal_sdp()),
    );
    harness.send_request(&request).await;

    // Should get responses — at minimum 100 Trying + an error since members
    // aren't registered
    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    assert!(
        !responses.is_empty(),
        "Expected at least one response for hunt group INVITE"
    );
}

#[tokio::test]
async fn test_regular_extension_not_treated_as_feature_code() {
    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(enabled_pbx_config())
        .build()
        .await;

    // Normal extension 1001, should NOT be treated as a feature code
    let request = build_invite_with_sdp(
        "alice@example.com",
        "1001@example.com",
        Some(minimal_sdp()),
    );
    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);

    if let Some(resp) = final_resp {
        // A normal extension INVITE shouldn't return 480 "No Ringing Call"
        // (which is the PBX pickup response). It should get a routing error
        // like 404 or 480 "Temporarily Unavailable".
        let reason = resp.start_line().reason().to_string();
        assert!(
            !reason.contains("No Ringing Call"),
            "Regular extension should not trigger PBX pickup logic"
        );
    }
}

#[tokio::test]
async fn test_pbx_disabled_ignores_feature_codes() {
    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(disabled_pbx_config())
        .build()
        .await;

    // *8 should NOT be treated as group pickup when PBX is disabled
    let request = invite_to_feature_code("*8");
    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);

    if let Some(resp) = final_resp {
        // Should NOT get 480 "No Ringing Call" (PBX pickup response)
        let reason = resp.start_line().reason().to_string();
        assert!(
            !reason.contains("No Ringing Call"),
            "PBX disabled should not dispatch feature codes"
        );
    }
}

#[tokio::test]
async fn test_custom_feature_code_prefix() {
    let mut config = enabled_pbx_config();
    // Override directed pickup from *9 to **9 (a star-code-based custom prefix)
    config.feature_codes.directed_pickup = "**9".to_string();

    let harness = TestHarness::builder()
        .with_registrar()
        .with_pbx(config)
        .build()
        .await;

    // *91001 should no longer be directed pickup (because prefix changed to **9)
    let request_old = invite_to_feature_code("*91001");
    harness.send_request(&request_old).await;
    let responses_old = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_old = responses_old.iter().find(|r| r.start_line().code() >= 200);
    if let Some(resp) = final_old {
        let reason = resp.start_line().reason().to_string();
        // With *9 no longer the prefix, this should NOT be handled as directed pickup
        // (480 "No Ringing Call" is the directed pickup response)
        assert!(
            !reason.contains("No Ringing Call"),
            "*91001 should not trigger directed pickup with custom prefix **9"
        );
    }

    // Clear captured messages for next test
    harness.drain_responses().await;

    // **91001 should now be directed pickup with the custom prefix
    let request_new = invite_to_feature_code("**91001");
    harness.send_request(&request_new).await;
    let responses_new = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_new = responses_new.iter().find(|r| r.start_line().code() >= 200);
    assert!(
        final_new.is_some(),
        "Expected a final response for custom directed pickup"
    );
    let code = final_new.unwrap().start_line().code();
    // 480 "No Ringing Call" means the PBX did recognise it as directed pickup
    assert_eq!(
        code, 480,
        "Expected 480 for custom directed pickup **91001, got {}",
        code
    );
}
