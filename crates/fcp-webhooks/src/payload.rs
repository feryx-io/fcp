//! Webhook payload construction and HMAC-SHA256 signing.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Webhook payload sent to registered endpoints via HTTP POST.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    /// Unique event delivery ID
    pub event_id: String,
    /// Dotted event type name (e.g., "call.initiated")
    pub event_type: String,
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// Tenant ID (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// Event data
    pub data: Value,
}

/// The header a delivery's signature travels in.
pub const SIGNATURE_HEADER: &str = "X-FCP-Signature";

/// One secret's signature of a delivery: hex HMAC-SHA256 of
/// `"<timestamp>.<body>"`.
pub fn sign(secret: &str, timestamp: u64, body: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// `X-FCP-Signature` (docs/DEVELOPER_SURFACE.md §3, phase 5):
/// `t=<unix seconds>,v1=<signature>`, with a `v1` for each secret — the
/// current one, and the one a rotation replaced while it overlaps. A
/// receiver checks any `v1` against its secret and refuses a `t` too far
/// from now, so a captured delivery cannot be replayed.
pub fn signature_header(secrets: &[&str], timestamp: u64, body: &str) -> String {
    let mut header = format!("t={timestamp}");
    for secret in secrets {
        header.push_str(",v1=");
        header.push_str(&sign(secret, timestamp, body));
    }
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vector every FCP webhook implementation checks against: the
    /// SDKs' `verifyWebhook` / `verify_webhook` tests hold the same one
    /// (sdk/typescript/test/webhook.test.js, sdk/python/tests/test_webhook.py).
    #[test]
    fn the_signature_matches_the_shared_vector() {
        let secret = "whsec_test_vector";
        let body = r#"{"event":"call.started"}"#;
        let expected = "008976dcc4d8a2d92dbfe2c643bdb8f498825fa20a43baa8d0e7a0358ffa159e";
        assert_eq!(sign(secret, 1790000000, body), expected);
        assert_eq!(
            signature_header(&[secret], 1790000000, body),
            format!("t=1790000000,v1={expected}")
        );
    }

    #[test]
    fn a_rotating_secret_signs_twice() {
        let header = signature_header(&["new", "old"], 5, "b");
        let parts: Vec<&str> = header.split(',').collect();
        assert_eq!(parts[0], "t=5");
        assert_eq!(parts[1], format!("v1={}", sign("new", 5, "b")));
        assert_eq!(parts[2], format!("v1={}", sign("old", 5, "b")));
        assert_ne!(
            sign("new", 5, "b"),
            sign("new", 6, "b"),
            "the time is signed"
        );
    }

    #[test]
    fn test_webhook_payload_serialization() {
        let payload = WebhookPayload {
            event_id: "evt-123".to_string(),
            event_type: "call.initiated".to_string(),
            timestamp: "2026-02-20T00:00:00Z".to_string(),
            tenant_id: Some("tenant-abc".to_string()),
            data: serde_json::json!({"call_id": "call-xyz"}),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("call.initiated"));
        assert!(json.contains("tenant-abc"));
    }

    #[test]
    fn test_webhook_payload_without_tenant() {
        let payload = WebhookPayload {
            event_id: "evt-123".to_string(),
            event_type: "system.started".to_string(),
            timestamp: "2026-02-20T00:00:00Z".to_string(),
            tenant_id: None,
            data: serde_json::json!({}),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(!json.contains("tenant_id"));
    }
}
