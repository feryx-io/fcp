//! Join tokens for browser joins: `POST /conferences/meetings/:id/join-tokens`
//! (admins) and `POST /me/conferences/:id/join-tokens` (owners) mint a
//! token for a meeting, signed with `[conference] join_secret` — the same
//! secret the conference servers verify with — and, when the WebRTC
//! gateway's URL is configured, the link that opens its join page with the
//! token filled in. See `docs/WEBRTC.md`.

use crate::error::ApiError;
use crate::state::ApiState;
use fcp_core::join_token::MAX_JOIN_TOKEN_TTL;
use fcp_core::{JoinRole, JoinToken, Meeting};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The default lifetime of a minted token.
pub const DEFAULT_TTL_SECS: u64 = 15 * 60;

/// `POST …/join-tokens` body.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct MintJoinTokenRequest {
    /// The name the participant joins under (the meeting's name plus
    /// "guest"/"host" when absent).
    #[serde(default)]
    pub display_name: Option<String>,
    /// Guest (default) or host.
    #[serde(default)]
    pub role: JoinRole,
    /// Lifetime in seconds; 15 minutes by default, one day at most.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// A minted token.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct JoinTokenResponse {
    pub token: String,
    pub room_number: String,
    pub display_name: String,
    pub role: JoinRole,
    /// Seconds since the Unix epoch.
    #[serde(with = "crate::time::epoch_secs")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: u64,
    /// The gateway's join page with the token filled in, when
    /// `[conference] gateway_url` is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join_url: Option<String>,
}

/// Percent-encode a query value (RFC 3986 unreserved characters kept).
fn query_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The join page URL for a token.
pub fn join_url(gateway_url: &str, token: &str, room_number: &str, display_name: &str) -> String {
    format!(
        "{}/join?room={}&name={}&token={}&autojoin=true",
        gateway_url.trim_end_matches('/'),
        query_encode(room_number),
        query_encode(display_name),
        query_encode(token)
    )
}

/// Mint a token for `meeting`. Shared by the admin and owner routes.
pub fn mint_for_meeting(
    state: &ApiState,
    meeting: &Meeting,
    req: MintJoinTokenRequest,
) -> Result<JoinTokenResponse, ApiError> {
    let secret = state
        .conference_join_secret
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "join tokens are not configured: set [conference] join_secret".to_string(),
            )
        })?;
    if !meeting.enabled {
        return Err(ApiError::Conflict("the meeting is disabled".to_string()));
    }
    let ttl_secs = req.ttl_secs.unwrap_or(DEFAULT_TTL_SECS);
    if ttl_secs == 0 || ttl_secs > MAX_JOIN_TOKEN_TTL.as_secs() {
        return Err(ApiError::BadRequest(format!(
            "ttl_secs must be between 1 and {}",
            MAX_JOIN_TOKEN_TTL.as_secs()
        )));
    }
    let display_name = req
        .display_name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            format!(
                "{} {}",
                meeting.name,
                match req.role {
                    JoinRole::Host => "host",
                    JoinRole::Guest => "guest",
                }
            )
        });
    if display_name.chars().count() > 64 {
        return Err(ApiError::BadRequest(
            "display_name must be at most 64 characters".to_string(),
        ));
    }
    let token = JoinToken::new(
        meeting.tenant_id,
        meeting.room_number.clone(),
        display_name.clone(),
        req.role,
        Duration::from_secs(ttl_secs),
    );
    let wire = token.mint(secret.as_bytes());
    let join_url = state
        .conference_gateway_url
        .as_deref()
        .filter(|u| !u.is_empty())
        .map(|u| join_url(u, &wire, &meeting.room_number, &display_name));
    Ok(JoinTokenResponse {
        token: wire,
        room_number: meeting.room_number.clone(),
        display_name,
        role: req.role,
        expires_at: token.expires_at,
        join_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_url_encodes_its_parts() {
        let url = join_url("https://gw.example.com/", "abc.def", "8001", "Ann Lee");
        assert_eq!(
            url,
            "https://gw.example.com/join?room=8001&name=Ann%20Lee&token=abc.def&autojoin=true"
        );
    }
}
