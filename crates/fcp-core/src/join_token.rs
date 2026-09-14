//! Join tokens: how a browser is admitted to a conference room without a
//! PIN. The admin API mints one for a meeting — who may join, as whom, in
//! which role, until when — signed with a secret it shares with the
//! conference servers, and the WebRTC gateway carries it to the
//! conference server in the `X-FCP-Join` header of its INVITE. The
//! conference server verifies the signature and expiry, takes the tenant
//! and room from the token rather than from the request, and admits the
//! caller with the token's role, skipping PIN collection.
//!
//! Wire form: `base64url(payload) "." base64url(hmac-sha256(secret, payload))`,
//! the payload being the JSON of [`JoinToken`]. The token names a room,
//! not a participant, so one token may be used for more than one join
//! until it expires; a short lifetime (minutes) is the intended control.

use crate::TenantId;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// The SIP header a join token travels in.
pub const JOIN_TOKEN_HEADER: &str = "X-FCP-Join";

/// The role a token admits its holder in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinRole {
    /// An ordinary participant.
    #[default]
    Guest,
    /// A host (moderator), as the host PIN would grant.
    Host,
}

/// What a token says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinToken {
    /// Format version.
    #[serde(rename = "v")]
    pub version: u8,
    /// The tenant the room belongs to.
    #[serde(rename = "tenant")]
    pub tenant_id: TenantId,
    /// The room number.
    #[serde(rename = "room")]
    pub room_number: String,
    /// The name the participant joins under.
    #[serde(rename = "name")]
    pub display_name: String,
    /// The role admitted.
    pub role: JoinRole,
    /// Expiry, seconds since the Unix epoch.
    #[serde(rename = "exp")]
    pub expires_at: u64,
    /// Random, so two tokens for the same room differ.
    pub nonce: String,
}

/// Why a token was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JoinTokenError {
    /// Not the expected two-part form, or not decodable.
    #[error("malformed join token")]
    Malformed,
    /// The signature does not match the secret.
    #[error("join token signature mismatch")]
    BadSignature,
    /// Past its expiry.
    #[error("join token expired")]
    Expired,
    /// A version this code does not know.
    #[error("unsupported join token version {0}")]
    Version(u8),
}

/// The current format version.
pub const JOIN_TOKEN_VERSION: u8 = 1;

/// The longest lifetime the admin API will mint.
pub const MAX_JOIN_TOKEN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Why a signed payload could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignedError {
    Malformed,
    BadSignature,
}

/// `base64url(payload) "." base64url(hmac-sha256(secret, payload))`: the
/// wire form join tokens and migration tickets share.
pub(crate) fn sign_payload(payload: &[u8], secret: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(payload);
    let sig = mac.finalize().into_bytes();
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(sig)
    )
}

/// The payload of a wire-form value whose signature checks out.
pub(crate) fn open_signed(token: &str, secret: &[u8]) -> Result<Vec<u8>, SignedError> {
    let token = token.trim();
    let (payload_b64, sig_b64) = token.split_once('.').ok_or(SignedError::Malformed)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| SignedError::Malformed)?;
    let sig = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| SignedError::Malformed)?;
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(&payload);
    mac.verify_slice(&sig)
        .map_err(|_| SignedError::BadSignature)?;
    Ok(payload)
}

impl JoinToken {
    /// A token for `room_number` in `tenant_id`, valid for `ttl` from now.
    pub fn new(
        tenant_id: TenantId,
        room_number: impl Into<String>,
        display_name: impl Into<String>,
        role: JoinRole,
        ttl: Duration,
    ) -> Self {
        Self {
            version: JOIN_TOKEN_VERSION,
            tenant_id,
            room_number: room_number.into(),
            display_name: display_name.into(),
            role,
            expires_at: unix_now().saturating_add(ttl.as_secs()),
            nonce: uuid::Uuid::new_v4().simple().to_string()[..16].to_string(),
        }
    }

    /// Sign with `secret` into the wire form.
    pub fn mint(&self, secret: &[u8]) -> String {
        let payload = serde_json::to_vec(self).expect("a join token serialises");
        sign_payload(&payload, secret)
    }

    /// Verify a wire-form token against `secret` as of now.
    pub fn verify(token: &str, secret: &[u8]) -> Result<Self, JoinTokenError> {
        Self::verify_at(token, secret, unix_now())
    }

    /// Verify as of `now` (seconds since the Unix epoch).
    pub fn verify_at(token: &str, secret: &[u8], now: u64) -> Result<Self, JoinTokenError> {
        let payload = open_signed(token, secret).map_err(|e| match e {
            SignedError::Malformed => JoinTokenError::Malformed,
            SignedError::BadSignature => JoinTokenError::BadSignature,
        })?;
        let parsed: JoinToken =
            serde_json::from_slice(&payload).map_err(|_| JoinTokenError::Malformed)?;
        if parsed.version != JOIN_TOKEN_VERSION {
            return Err(JoinTokenError::Version(parsed.version));
        }
        if parsed.expires_at <= now {
            return Err(JoinTokenError::Expired);
        }
        Ok(parsed)
    }

    /// What a token says, **without** checking its signature or expiry: for
    /// attributing a record to the tenant a conference node has already
    /// admitted the token for, never for admitting anyone. `None` when the
    /// token is not one.
    pub fn peek_unverified(token: &str) -> Option<Self> {
        let (payload_b64, _) = token.trim().split_once('.')?;
        let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
        serde_json::from_slice(&payload).ok()
    }

    /// Seconds until expiry as of now (zero when past).
    pub fn seconds_left(&self) -> u64 {
        self.expires_at.saturating_sub(unix_now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> TenantId {
        TenantId::new()
    }

    #[test]
    fn a_token_can_be_read_without_its_secret_but_not_verified() {
        let t = JoinToken::new(
            tenant(),
            "8001",
            "Ann",
            JoinRole::Guest,
            Duration::from_secs(300),
        );
        let token = t.mint(b"secret");
        let peeked = JoinToken::peek_unverified(&token).expect("reads");
        assert_eq!(peeked.tenant_id, t.tenant_id);
        assert_eq!(peeked.room_number, "8001");
        assert!(JoinToken::verify(&token, b"other").is_err());
        assert!(JoinToken::peek_unverified("not a token").is_none());
    }

    #[test]
    fn mint_and_verify_round_trip() {
        let t = JoinToken::new(
            tenant(),
            "8001",
            "Ann",
            JoinRole::Host,
            Duration::from_secs(300),
        );
        let wire = t.mint(b"secret");
        assert!(wire.contains('.'));
        let back = JoinToken::verify(&wire, b"secret").unwrap();
        assert_eq!(back, t);
        assert_eq!(back.role, JoinRole::Host);
        assert!(back.seconds_left() > 290);
    }

    #[test]
    fn wrong_secret_and_tampering_are_refused() {
        let t = JoinToken::new(
            tenant(),
            "8001",
            "Ann",
            JoinRole::Guest,
            Duration::from_secs(300),
        );
        let wire = t.mint(b"secret");
        assert_eq!(
            JoinToken::verify(&wire, b"other").unwrap_err(),
            JoinTokenError::BadSignature
        );
        // Change the payload (a different room) but keep the signature.
        let (payload, sig) = wire.split_once('.').unwrap();
        let mut json: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        json["room"] = serde_json::Value::String("9999".into());
        let forged = format!(
            "{}.{sig}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json).unwrap())
        );
        assert_eq!(
            JoinToken::verify(&forged, b"secret").unwrap_err(),
            JoinTokenError::BadSignature
        );
        assert_eq!(
            JoinToken::verify("garbage", b"secret").unwrap_err(),
            JoinTokenError::Malformed
        );
        assert_eq!(
            JoinToken::verify("a.b", b"secret").unwrap_err(),
            JoinTokenError::Malformed
        );
    }

    #[test]
    fn expiry_is_enforced() {
        let t = JoinToken::new(
            tenant(),
            "8001",
            "Ann",
            JoinRole::Guest,
            Duration::from_secs(60),
        );
        let wire = t.mint(b"secret");
        assert!(JoinToken::verify_at(&wire, b"secret", t.expires_at - 1).is_ok());
        assert_eq!(
            JoinToken::verify_at(&wire, b"secret", t.expires_at).unwrap_err(),
            JoinTokenError::Expired
        );
    }

    #[test]
    fn two_tokens_for_the_same_room_differ() {
        let a = JoinToken::new(
            tenant(),
            "8001",
            "Ann",
            JoinRole::Guest,
            Duration::from_secs(60),
        );
        let b = JoinToken::new(
            a.tenant_id,
            "8001",
            "Ann",
            JoinRole::Guest,
            Duration::from_secs(60),
        );
        assert_ne!(a.mint(b"s"), b.mint(b"s"));
    }
}
