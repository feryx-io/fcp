mod common;

use common::{build_register_request, TestHarness};
use fcp_core::RegistrationStore;

#[tokio::test]
async fn test_register_success() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let response = harness
        .register_device("alice@example.com", "alice@127.0.0.1:5060")
        .await;

    assert_eq!(response.start_line().code(), 200);
    assert_eq!(response.start_line().reason(), "OK");
}

#[tokio::test]
async fn test_register_without_registrar_returns_501() {
    let harness = TestHarness::builder().build().await;
    let request = build_register_request("alice@example.com", "alice@127.0.0.1:5060");
    harness.send_request(&request).await;

    let resp = harness.last_response().await.expect("expected response");
    assert_eq!(resp.start_line().code(), 501);
    assert_eq!(resp.start_line().reason(), "Not Implemented");
}

#[tokio::test]
async fn test_register_multiple_devices() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let aor = "multi@example.com";

    // Register first device
    let r1 = harness.register_device(aor, "multi@10.0.0.1:5060").await;
    assert_eq!(r1.start_line().code(), 200);

    // Register second device (different contact)
    let r2 = harness.register_device(aor, "multi@10.0.0.2:5060").await;
    assert_eq!(r2.start_line().code(), 200);

    // Both bindings should be stored
    let storage = harness.storage.as_ref().expect("storage required");
    let regs = storage
        .get_registrations(&format!("sip:{}", aor))
        .await
        .unwrap();
    assert!(
        regs.len() >= 2,
        "Expected at least 2 registrations, got {}",
        regs.len()
    );
}

#[tokio::test]
async fn test_deregister_with_expires_zero() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let aor = "dereg@example.com";

    // Register first
    let r1 = harness.register_device(aor, "dereg@10.0.0.1:5060").await;
    assert_eq!(r1.start_line().code(), 200);

    // Deregister with Expires: 0
    let request = {
        let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
        let call_id = format!("dereg-{}@test", uuid::Uuid::new_v4().simple());
        let tag = uuid::Uuid::new_v4().simple().to_string();

        let mut headers = sip_core::Headers::new();
        let _ = headers.push(
            smol_str::SmolStr::new("Via"),
            smol_str::SmolStr::new(format!("SIP/2.0/UDP 127.0.0.1:15060;branch={}", branch)),
        );
        let _ = headers.push(
            smol_str::SmolStr::new("From"),
            smol_str::SmolStr::new(format!("<sip:{}>;tag={}", aor, tag)),
        );
        let _ = headers.push(
            smol_str::SmolStr::new("To"),
            smol_str::SmolStr::new(format!("<sip:{}>", aor)),
        );
        let _ = headers.push(
            smol_str::SmolStr::new("Call-ID"),
            smol_str::SmolStr::new(&call_id),
        );
        let _ = headers.push(
            smol_str::SmolStr::new("CSeq"),
            smol_str::SmolStr::new("2 REGISTER"),
        );
        let _ = headers.push(
            smol_str::SmolStr::new("Max-Forwards"),
            smol_str::SmolStr::new("70"),
        );
        let _ = headers.push(
            smol_str::SmolStr::new("Contact"),
            smol_str::SmolStr::new("<sip:dereg@10.0.0.1:5060>;expires=0"),
        );
        let _ = headers.push(
            smol_str::SmolStr::new("Expires"),
            smol_str::SmolStr::new("0"),
        );
        let _ = headers.push(
            smol_str::SmolStr::new("Content-Length"),
            smol_str::SmolStr::new("0"),
        );
        sip_core::Request::new(
            sip_core::RequestLine::new(
                sip_core::Method::Register,
                sip_core::SipUri::parse("sip:example.com").unwrap(),
            ),
            headers,
            bytes::Bytes::new(),
        )
        .unwrap()
    };

    harness.send_request(&request).await;
    let resp = harness.last_response().await.expect("expected response");
    assert_eq!(resp.start_line().code(), 200);

    // Should have no remaining bindings
    let storage = harness.storage.as_ref().unwrap();
    let regs = storage
        .get_registrations(&format!("sip:{}", aor))
        .await
        .unwrap();
    assert!(
        regs.is_empty(),
        "Expected no registrations after deregister, got {}",
        regs.len()
    );
}

#[tokio::test]
async fn test_register_updates_existing_binding() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let aor = "update@example.com";
    let contact = "update@10.0.0.1:5060";

    // Register twice with same contact
    let r1 = harness.register_device(aor, contact).await;
    assert_eq!(r1.start_line().code(), 200);

    let r2 = harness.register_device(aor, contact).await;
    assert_eq!(r2.start_line().code(), 200);

    // Should have exactly 1 binding, not 2
    let storage = harness.storage.as_ref().unwrap();
    let regs = storage
        .get_registrations(&format!("sip:{}", aor))
        .await
        .unwrap();
    assert_eq!(
        regs.len(),
        1,
        "Expected 1 registration (updated, not duplicated), got {}",
        regs.len()
    );
}

#[tokio::test]
async fn test_register_via_pipeline() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let request = build_register_request("pipeline@example.com", "pipeline@10.0.0.1:5060");

    // Serialize → handle_inbound → capture → parse
    harness.send_request(&request).await;

    let resp = harness.last_response().await.expect("expected response");
    assert_eq!(resp.start_line().code(), 200);

    // Verify mandatory response headers are present
    assert!(resp.headers().get("Via").is_some(), "Missing Via header");
    assert!(resp.headers().get("From").is_some(), "Missing From header");
    assert!(resp.headers().get("To").is_some(), "Missing To header");
    assert!(
        resp.headers().get("Call-ID").is_some(),
        "Missing Call-ID header"
    );
    assert!(resp.headers().get("CSeq").is_some(), "Missing CSeq header");
}

#[tokio::test]
async fn test_register_missing_contact_header() {
    let harness = TestHarness::builder().with_registrar().build().await;

    // Build REGISTER without Contact header
    let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
    let call_id = format!("nocontact-{}@test", uuid::Uuid::new_v4().simple());
    let tag = uuid::Uuid::new_v4().simple().to_string();

    let mut headers = sip_core::Headers::new();
    let _ = headers.push(
        smol_str::SmolStr::new("Via"),
        smol_str::SmolStr::new(format!("SIP/2.0/UDP 127.0.0.1:15060;branch={}", branch)),
    );
    let _ = headers.push(
        smol_str::SmolStr::new("From"),
        smol_str::SmolStr::new(format!("<sip:nocontact@example.com>;tag={}", tag)),
    );
    let _ = headers.push(
        smol_str::SmolStr::new("To"),
        smol_str::SmolStr::new("<sip:nocontact@example.com>"),
    );
    let _ = headers.push(
        smol_str::SmolStr::new("Call-ID"),
        smol_str::SmolStr::new(&call_id),
    );
    let _ = headers.push(
        smol_str::SmolStr::new("CSeq"),
        smol_str::SmolStr::new("1 REGISTER"),
    );
    let _ = headers.push(
        smol_str::SmolStr::new("Max-Forwards"),
        smol_str::SmolStr::new("70"),
    );
    // No Contact header
    let _ = headers.push(
        smol_str::SmolStr::new("Content-Length"),
        smol_str::SmolStr::new("0"),
    );
    let request = sip_core::Request::new(
        sip_core::RequestLine::new(
            sip_core::Method::Register,
            sip_core::SipUri::parse("sip:example.com").unwrap(),
        ),
        headers,
        bytes::Bytes::new(),
    )
    .unwrap();

    harness.send_request(&request).await;
    let resp = harness.last_response().await.expect("expected response");

    // REGISTER without Contact is a query (RFC 3261 §10.2.3) — should return 200
    // with current bindings (none). This is valid SIP.
    assert_eq!(resp.start_line().code(), 200);
}

#[tokio::test]
async fn test_register_stores_correct_aor() {
    let harness = TestHarness::builder().with_registrar().build().await;
    let aor = "storecheck@example.com";
    let contact = "storecheck@10.0.0.5:5060";

    let resp = harness.register_device(aor, contact).await;
    assert_eq!(resp.start_line().code(), 200);

    let storage = harness.storage.as_ref().unwrap();
    let regs = storage
        .get_registrations(&format!("sip:{}", aor))
        .await
        .unwrap();
    assert!(!regs.is_empty(), "Expected at least one registration");
    // The stored contact should reference our contact address
    let stored_contact = &regs[0].contact;
    assert!(
        stored_contact.contains("storecheck@10.0.0.5:5060"),
        "Stored contact '{}' doesn't match expected contact",
        stored_contact
    );
}
