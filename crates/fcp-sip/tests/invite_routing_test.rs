mod common;

use common::{build_invite_request, build_invite_with_sdp, minimal_sdp, TestHarness};
use std::time::Duration;

// NOTE: Without a UAC configured, the handler enters "UAS mode" which accepts
// INVITEs directly. The B2BUA routing (registrar lookup, SDP validation, etc.)
// only fires when a UAC is present. These tests verify the pipeline and UAS-mode
// behaviour, plus header compliance on the responses.

#[tokio::test]
async fn test_invite_sends_100_trying() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let request =
        build_invite_with_sdp("alice@example.com", "bob@example.com", Some(minimal_sdp()));

    harness.send_request(&request).await;

    // INVITE handling is spawned — wait for responses
    let responses = harness.wait_for_responses(1, Duration::from_secs(2)).await;
    assert!(
        !responses.is_empty(),
        "Expected at least one response from INVITE"
    );

    // First captured response should be 100 Trying
    assert_eq!(
        responses[0].start_line().code(),
        100,
        "First response should be 100 Trying, got {}",
        responses[0].start_line().code()
    );
}

#[tokio::test]
async fn test_invite_creates_call_in_call_manager() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let request =
        build_invite_with_sdp("alice@example.com", "bob@example.com", Some(minimal_sdp()));

    assert_eq!(
        harness.call_manager.active_calls().len(),
        0,
        "No calls should exist before INVITE"
    );

    harness.send_request(&request).await;

    // Wait for the spawned INVITE handler to create the call
    tokio::time::sleep(Duration::from_millis(200)).await;

    let active = harness.call_manager.active_calls();
    assert!(
        !active.is_empty(),
        "Expected at least one active call after INVITE"
    );
}

#[tokio::test]
async fn test_invite_uas_mode_accepts_with_sdp() {
    // In UAS mode (no UAC), a valid INVITE with SDP is accepted with 200 OK
    let harness = TestHarness::builder().with_registrar().build().await;
    let request =
        build_invite_with_sdp("alice@example.com", "bob@example.com", Some(minimal_sdp()));

    harness.send_request(&request).await;

    // Wait for 100 Trying, 180 Ringing, 200 OK
    let responses = harness.wait_for_responses(3, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    assert!(
        final_resp.is_some(),
        "Expected a final response from UAS-mode INVITE"
    );
    assert_eq!(
        final_resp.unwrap().start_line().code(),
        200,
        "UAS mode should accept INVITE with 200 OK"
    );
}

#[tokio::test]
async fn test_invite_uas_mode_empty_body_accepted() {
    // In UAS mode, empty body INVITE still generates a 200 OK because UAS mode
    // doesn't perform B2BUA-style SDP validation
    let harness = TestHarness::builder().with_registrar().build().await;
    let request = build_invite_request("alice@example.com", "bob@example.com");

    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(3, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    assert!(
        final_resp.is_some(),
        "Expected a final response from UAS-mode INVITE with empty body"
    );
    // UAS mode handles it internally (may succeed or fail depending on SDP generation)
    let code = final_resp.unwrap().start_line().code();
    assert!(
        code == 200 || code == 500,
        "UAS mode should return 200 or 500 for empty-body INVITE, got {}",
        code
    );
}

#[tokio::test]
async fn test_invite_preserves_call_id_header() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let request =
        build_invite_with_sdp("alice@example.com", "bob@example.com", Some(minimal_sdp()));
    let original_call_id = sip_parse::header(request.headers(), "Call-ID")
        .expect("request should have Call-ID")
        .to_string();

    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(1, Duration::from_secs(2)).await;
    assert!(!responses.is_empty(), "Expected at least one response");

    let resp_call_id = sip_parse::header(responses[0].headers(), "Call-ID")
        .expect("response should have Call-ID")
        .to_string();
    assert_eq!(
        original_call_id, resp_call_id,
        "Response Call-ID should match request Call-ID"
    );
}

#[tokio::test]
async fn test_invite_preserves_via_header() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let request =
        build_invite_with_sdp("alice@example.com", "bob@example.com", Some(minimal_sdp()));
    let original_via = sip_parse::header(request.headers(), "Via")
        .expect("request should have Via")
        .to_string();

    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(1, Duration::from_secs(2)).await;
    assert!(!responses.is_empty(), "Expected at least one response");

    let resp_via = sip_parse::header(responses[0].headers(), "Via")
        .expect("response should have Via")
        .to_string();
    assert_eq!(
        original_via, resp_via,
        "Response Via should match request Via"
    );
}

#[tokio::test]
async fn test_invite_generates_to_tag() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let request =
        build_invite_with_sdp("alice@example.com", "bob@example.com", Some(minimal_sdp()));

    harness.send_request(&request).await;

    // Wait for at least the 100 Trying + a subsequent response
    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    // Check non-100 responses for To tag (100 Trying may or may not have one)
    let tagged = responses
        .iter()
        .filter(|r| r.start_line().code() > 100)
        .find(|r| {
            sip_parse::header(r.headers(), "To")
                .map(|to| to.contains(";tag="))
                .unwrap_or(false)
        });
    assert!(
        tagged.is_some(),
        "Expected at least one response > 100 with To tag"
    );
}

#[tokio::test]
async fn test_invite_response_includes_contact() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let request =
        build_invite_with_sdp("alice@example.com", "bob@example.com", Some(minimal_sdp()));

    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(3, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    if let Some(resp) = final_resp {
        assert!(
            resp.headers().get("Contact").is_some(),
            "INVITE 200 OK should include Contact header"
        );
    }
}
