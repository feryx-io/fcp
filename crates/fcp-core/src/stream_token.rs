//! Stream links: how a viewer who is not on FCP watches a webinar's HLS
//! stream (`docs/WEBINARS.md` §2.8, W6.3b). A host mints one for a meeting;
//! the admin serves the stream's playlist and segments to whoever holds it,
//! with no other sign-in. The token travels in the URL's path, because a
//! player resolves a playlist's segments relative to it and drops a query.
//!
//! Wire form as [`crate::join_token`]'s, signed with the same secret. Its
//! `purpose` keeps either kind from passing for the other: a join token has
//! none, and a stream token has no room, name or role.

use crate::join_token::{open_signed, sign_payload, unix_now, SignedError};
use crate::{MeetingId, TenantId};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The `purpose` a stream token carries.
const PURPOSE: &str = "stream";

/// The current format version.
pub const STREAM_TOKEN_VERSION: u8 = 1;

/// A link's lifetime when none is asked for: a day.
pub const DEFAULT_STREAM_TOKEN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The longest a link may live: a week, for a link sent out ahead.
pub const MAX_STREAM_TOKEN_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What a stream link says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamToken {
    #[serde(rename = "v")]
    pub version: u8,
    purpose: String,
    #[serde(rename = "tenant")]
    pub tenant_id: TenantId,
    /// The meeting whose stream it opens: any of its sittings.
    pub meeting: MeetingId,
    /// Expiry, seconds since the Unix epoch.
    #[serde(rename = "exp")]
    pub expires_at: u64,
    /// Random, so two links differ.
    pub nonce: String,
}

/// Why a link was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamTokenError {
    #[error("malformed stream link")]
    Malformed,
    #[error("stream link signature mismatch")]
    BadSignature,
    #[error("stream link expired")]
    Expired,
    #[error("unsupported stream link version {0}")]
    Version(u8),
}

impl StreamToken {
    /// A link to `meeting`'s stream, valid for `ttl` from now.
    pub fn new(tenant_id: TenantId, meeting: MeetingId, ttl: Duration) -> Self {
        Self {
            version: STREAM_TOKEN_VERSION,
            purpose: PURPOSE.to_string(),
            tenant_id,
            meeting,
            expires_at: unix_now().saturating_add(ttl.as_secs()),
            nonce: uuid::Uuid::new_v4().simple().to_string()[..16].to_string(),
        }
    }

    /// Sign with `secret` into the wire form.
    pub fn mint(&self, secret: &[u8]) -> String {
        let payload = serde_json::to_vec(self).expect("a stream token serialises");
        sign_payload(&payload, secret)
    }

    /// Verify a wire-form link against `secret` as of now.
    pub fn verify(token: &str, secret: &[u8]) -> Result<Self, StreamTokenError> {
        Self::verify_at(token, secret, unix_now())
    }

    /// Verify a wire-form link against `secret` as of `now`.
    pub fn verify_at(token: &str, secret: &[u8], now: u64) -> Result<Self, StreamTokenError> {
        let payload = open_signed(token, secret).map_err(|e| match e {
            SignedError::Malformed => StreamTokenError::Malformed,
            SignedError::BadSignature => StreamTokenError::BadSignature,
        })?;
        let parsed: StreamToken =
            serde_json::from_slice(&payload).map_err(|_| StreamTokenError::Malformed)?;
        if parsed.purpose != PURPOSE {
            return Err(StreamTokenError::Malformed);
        }
        if parsed.version != STREAM_TOKEN_VERSION {
            return Err(StreamTokenError::Version(parsed.version));
        }
        if parsed.expires_at <= now {
            return Err(StreamTokenError::Expired);
        }
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{JoinRole, JoinToken};

    const SECRET: &[u8] = b"a shared secret";

    #[test]
    fn a_link_verifies_until_it_expires_and_only_with_its_secret() {
        let meeting = MeetingId::new();
        let tenant = TenantId::new();
        let link = StreamToken::new(tenant, meeting, Duration::from_secs(60));
        let wire = link.mint(SECRET);
        let back = StreamToken::verify(&wire, SECRET).unwrap();
        assert_eq!((back.tenant_id, back.meeting), (tenant, meeting));
        assert_eq!(
            StreamToken::verify(&wire, b"another"),
            Err(StreamTokenError::BadSignature)
        );
        assert_eq!(
            StreamToken::verify_at(&wire, SECRET, link.expires_at),
            Err(StreamTokenError::Expired)
        );
        assert_eq!(
            StreamToken::verify("nope", SECRET),
            Err(StreamTokenError::Malformed)
        );
        // URL-safe: it is a path segment.
        assert!(wire
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')));
    }

    #[test]
    fn a_join_token_is_not_a_stream_link_nor_the_other_way() {
        let join = JoinToken::new(
            TenantId::new(),
            "8001",
            "Ann",
            JoinRole::Attendee,
            Duration::from_secs(60),
        )
        .mint(SECRET);
        assert_eq!(
            StreamToken::verify(&join, SECRET),
            Err(StreamTokenError::Malformed)
        );
        let link = StreamToken::new(TenantId::new(), MeetingId::new(), Duration::from_secs(60))
            .mint(SECRET);
        assert!(JoinToken::verify(&link, SECRET).is_err());
    }
}
