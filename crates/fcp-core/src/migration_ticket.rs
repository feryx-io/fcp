//! Migration tickets: how a caller moves from a draining conference node
//! to another one as the participant they already were
//! (`docs/DISTRIBUTED_CONFERENCING.md`, phase 5). The draining node mints
//! one per caller and puts it in the `Refer-To` of the REFER it sends the
//! caller's leg, as the `X-FCP-Migrate` URI header; whoever holds the leg
//! dials the new node with the same value as a request header. The new
//! node verifies the signature and expiry, takes the tenant, room and
//! participant from the ticket, and admits the caller past PIN, lock and
//! waiting room, restoring role, mute, gain and video.
//!
//! Signed under the cluster secret (`cluster.secret`), the trust the
//! conference nodes already share for cross-node control, in the join
//! token's wire form: `base64url(payload) "." base64url(hmac-sha256)`.
//! Unlike a join token a ticket names one participant, lives a minute,
//! and carries a nonce the admitting node remembers for that long so it
//! admits once.

use crate::join_token::{open_signed, sign_payload, unix_now, SignedError};
use crate::TenantId;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The header (SIP request header, and `Refer-To` URI header) a ticket
/// travels in.
pub const MIGRATION_TICKET_HEADER: &str = "X-FCP-Migrate";

/// The current format version.
pub const MIGRATION_TICKET_VERSION: u8 = 1;

/// How long a ticket is good for. Long enough for a REFER, an INVITE and
/// a NOTIFY; short enough that a leaked one is worthless.
pub const MIGRATION_TICKET_TTL: Duration = Duration::from_secs(60);

/// What a ticket says: the participant as the draining node had them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MigrationTicket {
    /// Format version.
    #[serde(rename = "v")]
    pub version: u8,
    /// The tenant the room belongs to.
    #[serde(rename = "tenant")]
    pub tenant_id: TenantId,
    /// The room number.
    #[serde(rename = "room")]
    pub room_number: String,
    /// The participant id, kept across the move.
    #[serde(rename = "participant")]
    pub participant_id: String,
    /// Display name as the old node had it.
    #[serde(rename = "name", default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Caller id as the old node had it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_id: Option<String>,
    /// Moderator.
    #[serde(rename = "host")]
    pub is_host: bool,
    /// A webinar's attendee, who listens (`docs/WEBINARS.md` §2.2): the
    /// new node admits them as a listener.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub listener: bool,
    /// Muted in the mixer.
    #[serde(default)]
    pub muted: bool,
    /// Input gain in the mixer (1.0 is unity).
    #[serde(default = "unity")]
    pub gain: f32,
    /// The participant's video was on (not disabled by a host).
    #[serde(default = "yes")]
    pub video: bool,
    /// Told the content floor by `INFO`: came through the WebRTC gateway
    /// (admitted by a join token) or asked with `Recv-Info: fcp-content`,
    /// so the new node tells it too.
    #[serde(default)]
    pub token_join: bool,
    /// The webinar registration they joined by (`docs/WEBINARS.md` §2.6),
    /// carried so the new node records who they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registrant: Option<String>,
    /// The draining node's id.
    #[serde(rename = "from")]
    pub from_node: String,
    /// Expiry, seconds since the Unix epoch.
    #[serde(rename = "exp")]
    pub expires_at: u64,
    /// Random, remembered by the admitting node until expiry.
    pub nonce: String,
}

fn unity() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

/// Why a ticket was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MigrationTicketError {
    /// Not the expected two-part form, or not decodable.
    #[error("malformed migration ticket")]
    Malformed,
    /// The signature does not match the secret.
    #[error("migration ticket signature mismatch")]
    BadSignature,
    /// Past its expiry.
    #[error("migration ticket expired")]
    Expired,
    /// A version this code does not know.
    #[error("unsupported migration ticket version {0}")]
    Version(u8),
}

impl From<SignedError> for MigrationTicketError {
    fn from(e: SignedError) -> Self {
        match e {
            SignedError::Malformed => Self::Malformed,
            SignedError::BadSignature => Self::BadSignature,
        }
    }
}

impl MigrationTicket {
    /// Sign with `secret` into the wire form.
    pub fn mint(&self, secret: &[u8]) -> String {
        let payload = serde_json::to_vec(self).expect("a migration ticket serialises");
        sign_payload(&payload, secret)
    }

    /// Verify a wire-form ticket against `secret` as of now.
    pub fn verify(ticket: &str, secret: &[u8]) -> Result<Self, MigrationTicketError> {
        Self::verify_at(ticket, secret, unix_now())
    }

    /// Verify as of `now` (seconds since the Unix epoch).
    pub fn verify_at(ticket: &str, secret: &[u8], now: u64) -> Result<Self, MigrationTicketError> {
        let payload = open_signed(ticket, secret)?;
        let parsed: MigrationTicket =
            serde_json::from_slice(&payload).map_err(|_| MigrationTicketError::Malformed)?;
        if parsed.version != MIGRATION_TICKET_VERSION {
            return Err(MigrationTicketError::Version(parsed.version));
        }
        if parsed.expires_at <= now {
            return Err(MigrationTicketError::Expired);
        }
        Ok(parsed)
    }

    /// The expiry a ticket minted now gets.
    pub fn expiry_from_now() -> u64 {
        unix_now().saturating_add(MIGRATION_TICKET_TTL.as_secs())
    }

    /// A fresh nonce.
    pub fn new_nonce() -> String {
        uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> MigrationTicket {
        MigrationTicket {
            version: MIGRATION_TICKET_VERSION,
            tenant_id: TenantId::new(),
            room_number: "8000".into(),
            participant_id: "abc12345-0011aabb".into(),
            display_name: Some("Ann".into()),
            caller_id: Some("1001".into()),
            is_host: true,
            listener: false,
            muted: true,
            gain: 1.5,
            video: false,
            token_join: false,
            registrant: None,
            from_node: "conf-a".into(),
            expires_at: MigrationTicket::expiry_from_now(),
            nonce: MigrationTicket::new_nonce(),
        }
    }

    #[test]
    fn mint_and_verify_round_trip() {
        let t = ticket();
        let wire = t.mint(b"cluster");
        // URI-header safe: base64url and a dot, nothing to escape.
        assert!(wire
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'));
        let back = MigrationTicket::verify(&wire, b"cluster").unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn wrong_secret_expiry_and_version_are_refused() {
        let mut t = ticket();
        let wire = t.mint(b"cluster");
        assert_eq!(
            MigrationTicket::verify(&wire, b"other").unwrap_err(),
            MigrationTicketError::BadSignature
        );
        assert_eq!(
            MigrationTicket::verify_at(&wire, b"cluster", t.expires_at).unwrap_err(),
            MigrationTicketError::Expired
        );
        t.version = 2;
        assert_eq!(
            MigrationTicket::verify(&t.mint(b"cluster"), b"cluster").unwrap_err(),
            MigrationTicketError::Version(2)
        );
        assert_eq!(
            MigrationTicket::verify("nope", b"cluster").unwrap_err(),
            MigrationTicketError::Malformed
        );
    }

    #[test]
    fn older_tickets_default_the_optional_claims() {
        // A ticket without mute, gain, video or token_join reads as an
        // unmuted, unity-gain, video-on caller.
        let t = ticket();
        let mut json = serde_json::to_value(&t).unwrap();
        for k in ["muted", "gain", "video", "token_join", "name", "caller_id"] {
            json.as_object_mut().unwrap().remove(k);
        }
        let wire = sign_payload(&serde_json::to_vec(&json).unwrap(), b"cluster");
        let back = MigrationTicket::verify(&wire, b"cluster").unwrap();
        assert!(!back.muted && back.video && !back.token_join);
        assert_eq!(back.gain, 1.0);
        assert_eq!(back.display_name, None);
    }
}
