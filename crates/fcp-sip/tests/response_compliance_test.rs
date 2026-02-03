mod common;

use common::{build_invite_with_sdp, build_options_request, minimal_sdp, TestHarness};
use std::time::Duration;

#[tokio::test]
async fn test_error_response_has_required_headers() {
    let harness = TestHarness::builder().build().await;
    // Send a REGISTER to a handler with no registrar → 501
    let request =
        common::build_register_request("alice@example.com", "alice@127.0.0.1:5060");
    harness.send_request(&request).await;

    let resp = harness.last_response().await.expect("expected response");
    assert_eq!(resp.start_line().code(), 501);

    // RFC 3261 §8.2.6: response MUST contain Via, From, To, Call-ID, CSeq
    assert!(
        resp.headers().get("Via").is_some(),
        "Error response missing Via"
    );
    assert!(
        resp.headers().get("From").is_some(),
        "Error response missing From"
    );
    let to = resp.headers().get("To");
    assert!(to.is_some(), "Error response missing To");
    // To header must have a tag in responses > 100
    assert!(
        to.unwrap().contains(";tag="),
        "Error response To header missing tag"
    );
    assert!(
        resp.headers().get("Call-ID").is_some(),
        "Error response missing Call-ID"
    );
    assert!(
        resp.headers().get("CSeq").is_some(),
        "Error response missing CSeq"
    );
    assert!(
        resp.headers().get("Content-Length").is_some(),
        "Error response missing Content-Length"
    );
}

#[tokio::test]
async fn test_error_response_includes_allow_header() {
    let harness = TestHarness::builder().build().await;
    // Unknown method would give 501, but a REGISTER without registrar is simpler
    let request =
        common::build_register_request("alice@example.com", "alice@127.0.0.1:5060");
    harness.send_request(&request).await;

    let resp = harness.last_response().await.expect("expected response");
    let allow = resp.headers().get("Allow");
    assert!(allow.is_some(), "Error response missing Allow header");
    let allow_str = allow.unwrap().to_string();
    // Should list supported methods
    assert!(
        allow_str.contains("INVITE"),
        "Allow header should include INVITE"
    );
    assert!(
        allow_str.contains("BYE"),
        "Allow header should include BYE"
    );
    assert!(
        allow_str.contains("OPTIONS"),
        "Allow header should include OPTIONS"
    );
}

#[tokio::test]
async fn test_error_response_includes_supported_header() {
    let harness = TestHarness::builder().build().await;
    let request =
        common::build_register_request("alice@example.com", "alice@127.0.0.1:5060");
    harness.send_request(&request).await;

    let resp = harness.last_response().await.expect("expected response");
    let supported = resp.headers().get("Supported");
    assert!(
        supported.is_some(),
        "Error response missing Supported header"
    );
}

#[tokio::test]
async fn test_error_response_includes_contact() {
    let harness = TestHarness::builder().with_registrar().build().await;

    // INVITE error response should include Contact
    let request = build_invite_with_sdp(
        "alice@example.com",
        "nobody@example.com",
        Some(minimal_sdp()),
    );
    harness.send_request(&request).await;

    let responses = harness.wait_for_responses(2, Duration::from_secs(2)).await;
    let final_resp = responses.iter().find(|r| r.start_line().code() >= 200);
    if let Some(resp) = final_resp {
        assert!(
            resp.headers().get("Contact").is_some(),
            "INVITE error response should include Contact header"
        );
    }
}

#[tokio::test]
async fn test_content_length_matches_body() {
    let harness = TestHarness::builder().build().await;
    let request =
        common::build_register_request("alice@example.com", "alice@127.0.0.1:5060");
    harness.send_request(&request).await;

    let resp = harness.last_response().await.expect("expected response");
    let cl = resp
        .headers()
        .get("Content-Length")
        .expect("Missing Content-Length");
    let cl_value: usize = cl.parse().expect("Content-Length not a number");
    let body_len = resp.body().len();
    assert_eq!(
        cl_value, body_len,
        "Content-Length ({}) does not match body length ({})",
        cl_value, body_len
    );
}

#[tokio::test]
async fn test_options_response_200_with_capabilities() {
    let harness = TestHarness::builder().build().await;
    let request = build_options_request("test@127.0.0.1");
    harness.send_request(&request).await;

    let resp = harness.last_response().await.expect("expected response");
    assert_eq!(resp.start_line().code(), 200, "OPTIONS should return 200 OK");

    // Capabilities headers
    let allow = resp.headers().get("Allow");
    assert!(allow.is_some(), "OPTIONS 200 missing Allow header");
    let allow_str = allow.unwrap().to_string();
    assert!(
        allow_str.contains("INVITE"),
        "Allow should list INVITE"
    );
    assert!(
        allow_str.contains("OPTIONS"),
        "Allow should list OPTIONS"
    );

    let supported = resp.headers().get("Supported");
    assert!(
        supported.is_some(),
        "OPTIONS 200 missing Supported header"
    );
}
