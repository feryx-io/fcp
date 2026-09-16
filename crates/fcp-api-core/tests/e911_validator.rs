//! The HTTP address validator against a provider that answers like one
//! (`docs/E911_BLOCK_2.md` §7).

use axum::{routing::post, Json, Router};
use fcp_core::e911::E911AddressValidator;
use fcp_core::E911Address;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A provider that records what it was asked and answers `reply`.
async fn provider(reply: Value, seen: Arc<Mutex<Vec<Value>>>) -> (SocketAddr, String) {
    let handler = {
        let seen = Arc::clone(&seen);
        move |body: Json<Value>| {
            let seen = Arc::clone(&seen);
            let reply = reply.clone();
            async move {
                seen.lock().expect("seen").push(body.0);
                Json(reply)
            }
        }
    };
    let app = Router::new().route("/e911", post(handler)).route(
        "/refuses",
        post(|| async { (axum::http::StatusCode::BAD_GATEWAY, "upstream is down") }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, format!("http://{addr}/e911"))
}

fn address(line1: &str) -> E911Address {
    E911Address::new(
        line1.to_string(),
        "San Francisco".to_string(),
        "ca".to_string(),
        "94103".to_string(),
        "US".to_string(),
    )
}

#[tokio::test]
async fn the_http_validator_sends_a_structured_address_and_keeps_the_providers_answer() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (_, url) = provider(
        json!({
            "valid": true,
            "normalized": {
                "address1": "1455 Market St", "city": "San Francisco", "state": "CA",
                "postal_code": "94103-1692", "country": "US",
                "house_number": "1455", "street_name": "Market", "street_suffix": "St",
                "validated": true
            },
            "issues": [],
            "provider_reference": "msag-42"
        }),
        Arc::clone(&seen),
    )
    .await;

    let validator = fcp_api_core::e911_validator::HttpAddressValidator::new(
        "acme-msag",
        url,
        Some("s3cret".to_string()),
        Duration::from_secs(5),
    )
    .unwrap();
    let result = validator
        .validate(&address("  1455  market street "))
        .await
        .unwrap();

    // What went out: the address tidied and split, not what was typed.
    let sent = seen.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let sent = &sent[0]["address"];
    assert_eq!(sent["address1"], "1455 market street");
    assert_eq!(sent["state"], "CA", "codes are upper-cased before sending");
    assert_eq!(sent["house_number"], "1455");
    assert_eq!(sent["street_suffix"], "St");

    // What came back: the provider's normalised address and reference, and
    // its name as the validator — but never its claim to be validated,
    // which only this platform writes.
    assert!(result.valid);
    assert_eq!(result.validator, "acme-msag");
    assert_eq!(result.provider_reference.as_deref(), Some("msag-42"));
    assert_eq!(result.normalized.postal_code, "94103-1692");
    assert!(!result.normalized.validated);

    let stored = result.validated_address(std::time::SystemTime::now());
    assert!(stored.validated);
    assert_eq!(stored.validated_by.as_deref(), Some("acme-msag"));
    assert_eq!(stored.provider_reference.as_deref(), Some("msag-42"));
}

/// A provider that accepts an address this platform could not dispatch to
/// does not get to validate it.
#[tokio::test]
async fn an_incomplete_answer_is_not_validated_however_sure_the_provider_is() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (_, url) = provider(
        json!({
            "valid": true,
            "normalized": {
                "address1": "1455 Market St", "city": "", "state": "CA",
                "postal_code": "94103", "country": "US"
            }
        }),
        Arc::clone(&seen),
    )
    .await;
    let validator = fcp_api_core::e911_validator::HttpAddressValidator::new(
        "http",
        url,
        None,
        Duration::from_secs(5),
    )
    .unwrap();
    let result = validator
        .validate(&address("1455 Market St"))
        .await
        .unwrap();
    assert!(!result.valid, "{:?}", result);
    assert!(
        result.issues.iter().any(|i| i.contains("city")),
        "{:?}",
        result.issues
    );
}

#[tokio::test]
async fn a_refusal_is_an_invalid_address_and_a_failure_is_an_error() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (addr, url) = provider(
        json!({ "valid": false, "issues": ["no such house number on Market St"] }),
        Arc::clone(&seen),
    )
    .await;

    let validator = fcp_api_core::e911_validator::HttpAddressValidator::new(
        "http",
        url,
        None,
        Duration::from_secs(5),
    )
    .unwrap();
    let result = validator
        .validate(&address("99999 Market St"))
        .await
        .unwrap();
    assert!(!result.valid);
    assert_eq!(result.issues.len(), 1);
    assert!(
        !result
            .validated_address(std::time::SystemTime::now())
            .validated
    );

    // A provider that cannot answer is an error: nothing is validated by a
    // service that was not reached.
    let broken = fcp_api_core::e911_validator::HttpAddressValidator::new(
        "http",
        format!("http://{addr}/refuses"),
        None,
        Duration::from_secs(5),
    )
    .unwrap();
    let error = broken
        .validate(&address("1455 Market St"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("502"), "{error}");
}
