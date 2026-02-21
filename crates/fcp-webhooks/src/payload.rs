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

/// Compute HMAC-SHA256 signature for a payload body.
///
/// Returns a string in the format `sha256=<hex-encoded-digest>`.
pub fn compute_signature(secret: &str, body: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
    mac.update(body.as_bytes());
    let result = mac.finalize();
    let bytes = result.into_bytes();
    format!("sha256={}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_signature() {
        let sig = compute_signature("my-secret", r#"{"test": true}"#);
        assert!(sig.starts_with("sha256="));
        // sha256= prefix (7 chars) + 64 hex chars = 71 total
        assert_eq!(sig.len(), 71);
    }

    #[test]
    fn test_signature_deterministic() {
        let sig1 = compute_signature("secret", "body");
        let sig2 = compute_signature("secret", "body");
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_different_secrets_produce_different_signatures() {
        let sig1 = compute_signature("secret1", "body");
        let sig2 = compute_signature("secret2", "body");
        assert_ne!(sig1, sig2);
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
