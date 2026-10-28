//! Sign-in sessions (`docs/IDENTITY_AND_DEVICES.md` §2, phase 1).
//!
//! A session is a row: who signed in, to which tenant, how, until when,
//! and whether it has been revoked. What the browser holds is a short
//! access token naming the session (`sid`) and the user by id (`sub`),
//! and a refresh secret, rotated on each use, in an HttpOnly cookie.
//!
//! - The access token is an HS256 JWT signed with the platform's session
//!   key, with the key's id in its header so the key can rotate: tokens
//!   signed by a previous key are accepted while they last.
//! - Revoking a session (logout, a password change, a disabled user)
//!   refuses its access tokens at once, and its refresh secret.
//! - A refresh secret presented twice — after it was rotated — means two
//!   holders, and revokes the session, unless the second came within a few
//!   seconds of the first (two tabs refreshing together).
//!
//! The admin API and the events server both authenticate through
//! [`authenticate_session`].

use crate::storage::{AccountStatus, CredentialStore, User};
use crate::{LoginSessionId, TenantId, UserId};
use async_trait::async_trait;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long an access token lasts by default.
pub const DEFAULT_ACCESS_TTL: Duration = Duration::from_secs(15 * 60);
/// How long a session lasts by default, refreshed or not.
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(12 * 3600);
/// How soon after a rotation the previous refresh secret is still taken
/// (two tabs refreshing together) rather than read as a theft.
pub const REFRESH_REUSE_GRACE: Duration = Duration::from_secs(10);

/// How someone signed in.
pub mod method {
    pub const PASSWORD: &str = "password";
    pub const SIGNUP: &str = "signup";
    /// A password and a one-time code.
    pub const TOTP: &str = "totp";
    /// A password and a recovery code.
    pub const RECOVERY_CODE: &str = "recovery_code";
    /// Through a tenant's OpenID Connect provider.
    pub const OIDC: &str = "oidc";
    /// Through a tenant's SAML provider.
    pub const SAML: &str = "saml";
    /// An app session, granted by a sign-in in the browser or a fresh
    /// session (`docs/SOFTPHONE.md` §2.2).
    pub const APP: &str = "app";
}

/// How long an app session lasts after its last refresh, by default
/// (`docs/SOFTPHONE.md` §2.3); a tenant sets its own.
pub const DEFAULT_APP_SESSION_DAYS: u32 = 90;
/// How long an app's sign-in may sit in the browser, and how long its code
/// lives once granted.
pub const APP_AUTHORIZATION_TTL: Duration = Duration::from_secs(10 * 60);
pub const APP_CODE_TTL: Duration = Duration::from_secs(60);
/// How recent a browser sign-in must be to grant an app a session.
pub const APP_GRANT_FRESHNESS: Duration = Duration::from_secs(10 * 60);

/// The app an app session belongs to: what the user sees in their list of
/// devices, and the install it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppClient {
    /// What the app calls itself on this machine ("Alice's MacBook").
    pub name: String,
    /// Kept by the install across sign-ins, so a second sign-in on the same
    /// machine replaces the first rather than adding a device.
    pub install_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// An app's sign-in in the browser (RFC 8252, RFC 7636): begun by the app,
/// granted by the user signing in, its code redeemed once by the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAuthorization {
    pub id: String,
    pub client_id: String,
    pub redirect_uri: String,
    /// The S256 challenge the app's verifier must answer.
    pub code_challenge: String,
    pub state: Option<String>,
    pub created_at: SystemTime,
    pub expires_at: SystemTime,
    /// Set when granted: who, and the code's hash and expiry.
    pub user_id: Option<UserId>,
    pub tenant_id: Option<TenantId>,
    pub code_hash: Option<String>,
    pub code_expires_at: Option<SystemTime>,
}

/// The S256 challenge of a PKCE verifier (RFC 7636 §4.2).
pub fn pkce_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Whether `verifier` is a PKCE verifier at all (RFC 7636 §4.1): 43 to 128
/// unreserved characters.
pub fn valid_pkce_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~'))
}

/// Where an app may be sent back to: a loopback address on any port over
/// plain HTTP (RFC 8252 §7.3) — `127.0.0.1`, `[::1]` or `localhost`.
/// Mobile apps' claimed links and private-use schemes come with the mobile
/// project.
pub fn valid_app_redirect(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        match v6.split_once(']') {
            Some((host, port)) if port.is_empty() || port.starts_with(':') => host,
            _ => return false,
        }
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}

/// A sign-in session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSession {
    pub id: LoginSessionId,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    /// [`method`]: `password`, later `totp`, `oidc`, `saml`.
    pub method: String,
    pub created_at: SystemTime,
    /// When the refresh secret last turned over.
    pub refreshed_at: SystemTime,
    pub expires_at: SystemTime,
    pub revoked_at: Option<SystemTime>,
    pub revoked_reason: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    /// SHA-256 of the current refresh secret.
    pub refresh_hash: String,
    /// SHA-256 of the one before it.
    pub previous_refresh_hash: Option<String>,
    /// The identity provider it was begun through, and the provider's own
    /// session, so the provider's back-channel logout can end it
    /// (`docs/IDENTITY_AND_DEVICES.md` phase 3b).
    pub idp: Option<IdpSession>,
    /// The app, for an app session (`docs/SOFTPHONE.md` §2.3); a browser's
    /// session has none.
    pub app: Option<AppClient>,
    /// The install's device, for an app session (§2.1).
    pub device_id: Option<crate::DeviceId>,
}

/// A session's provider: its id, the `sid` its ID token carried (a SAML
/// assertion's `SessionIndex`), and what signing out there needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdpSession {
    pub provider_id: String,
    pub sid: Option<String>,
    /// The ID token for OIDC's `id_token_hint`, or the SAML NameID; kept
    /// only where the provider signs out at the provider.
    pub logout_hint: Option<String>,
}

impl LoginSession {
    /// Whether the session can still be used at `now`.
    pub fn is_live(&self, now: SystemTime) -> bool {
        self.revoked_at.is_none() && now < self.expires_at
    }
}

/// What presenting a refresh secret did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// The secret was current: it is now the previous one.
    Rotated,
    /// The previous secret, again, within the grace: the session stands,
    /// nothing turned over.
    RecentlyRotated,
    /// The previous secret, after the grace: the session is now revoked.
    Reused,
    /// No live session holds this secret.
    Unknown,
}

/// Where sessions are kept.
#[async_trait]
pub trait LoginSessionStore: Send + Sync {
    async fn create_login_session(&self, session: &LoginSession) -> crate::Result<()>;

    async fn get_login_session(&self, id: &LoginSessionId) -> crate::Result<Option<LoginSession>>;

    /// Turn the refresh secret over: `presented` for `next`, atomically.
    async fn rotate_refresh(
        &self,
        id: &LoginSessionId,
        presented_hash: &str,
        next_hash: &str,
        now: SystemTime,
    ) -> crate::Result<RefreshOutcome>;

    /// Revoke one session; false when it was not live.
    async fn revoke_login_session(
        &self,
        id: &LoginSessionId,
        reason: &str,
        now: SystemTime,
    ) -> crate::Result<bool>;

    /// Revoke every live session of a user, but `keep`; how many.
    async fn revoke_user_sessions(
        &self,
        user_id: &UserId,
        keep: Option<&LoginSessionId>,
        reason: &str,
        now: SystemTime,
    ) -> crate::Result<u64>;

    /// A user's live sessions, newest first.
    async fn list_user_sessions(&self, user_id: &UserId) -> crate::Result<Vec<LoginSession>>;

    /// Revoke the live sessions begun through `provider_id` that are the
    /// provider's session `sid`, or `user_id`'s, or both where both are
    /// given (a back-channel logout); how many.
    async fn revoke_idp_sessions(
        &self,
        provider_id: &str,
        user_id: Option<&UserId>,
        sid: Option<&str>,
        reason: &str,
        now: SystemTime,
    ) -> crate::Result<u64>;

    /// Push a session's expiry out (an app session, on each refresh).
    async fn extend_login_session(
        &self,
        id: &LoginSessionId,
        expires_at: SystemTime,
    ) -> crate::Result<()>;

    /// A user's live sessions of the app install `install_id`.
    async fn app_sessions_of_install(
        &self,
        user_id: &UserId,
        install_id: &str,
    ) -> crate::Result<Vec<LoginSession>>;

    /// The device an app install of `user_id` is.
    async fn app_install_device(
        &self,
        user_id: &UserId,
        install_id: &str,
    ) -> crate::Result<Option<crate::DeviceId>>;

    /// Record the device an app install is.
    async fn set_app_install_device(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        install_id: &str,
        device_id: &crate::DeviceId,
    ) -> crate::Result<()>;

    /// The install of each of a user's app devices: device to install id.
    async fn app_installs_of_user(
        &self,
        user_id: &UserId,
    ) -> crate::Result<Vec<(crate::DeviceId, String)>>;

    /// App devices no live session uses, made before `made_before`: the
    /// install signed out, its session expired, or it was replaced without
    /// a word (uninstalled, its files lost). A device being signed in has
    /// its session a moment after it is made, hence `made_before`. A
    /// session not refreshed since `idle_before` does not count when the
    /// same user has a newer install of the same app (`app_name`) signed
    /// in: that machine was reinstalled.
    async fn abandoned_app_devices(
        &self,
        made_before: SystemTime,
        idle_before: SystemTime,
    ) -> crate::Result<Vec<crate::DeviceId>>;

    /// Revoke the live sessions of a device's app; how many.
    async fn revoke_device_sessions(
        &self,
        device_id: &crate::DeviceId,
        reason: &str,
        now: SystemTime,
    ) -> crate::Result<u64>;

    /// Keep an app's sign-in begun in the browser.
    async fn create_app_authorization(&self, authorization: &AppAuthorization)
        -> crate::Result<()>;

    async fn get_app_authorization(&self, id: &str) -> crate::Result<Option<AppAuthorization>>;

    /// Grant an app's sign-in to a user: once, and only while it has not
    /// expired; false otherwise.
    async fn grant_app_authorization(
        &self,
        id: &str,
        user_id: &UserId,
        tenant_id: &TenantId,
        code_hash: &str,
        code_expires_at: SystemTime,
        now: SystemTime,
    ) -> crate::Result<bool>;

    /// Redeem a granted sign-in's code: once, before it expires.
    async fn redeem_app_code(
        &self,
        code_hash: &str,
        now: SystemTime,
    ) -> crate::Result<Option<AppAuthorization>>;

    /// Delete sessions that ended more than `keep` ago; how many.
    async fn sweep_ended_sessions(&self, keep: Duration) -> crate::Result<u64> {
        let _ = keep;
        Ok(0)
    }
}

/// An access token's claims.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionClaims {
    /// The user's id; a token made before sessions were rows has the
    /// username here and no `sid`.
    pub sub: String,
    /// The session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    /// The username when the token was made, for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub role: String,
    pub tenant_id: String,
    pub display_name: Option<String>,
    pub exp: usize,
    pub iat: usize,
    /// What the token is for when it is not an access token: a sign-in
    /// halfway through its second factor (`totp`, `totp_enroll`). Such a
    /// token is never a session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// One signing key, named by its id.
#[derive(Clone)]
pub struct SessionKey {
    pub kid: String,
    secret: Vec<u8>,
}

impl std::fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl SessionKey {
    /// A configured key: its id is a digest of it, so the same key has the
    /// same id on every node.
    pub fn configured(secret: &str) -> Self {
        let digest = Sha256::digest(secret.as_bytes());
        Self {
            kid: hex8(&digest),
            secret: secret.as_bytes().to_vec(),
        }
    }

    /// The key every release before this one used: derived from the
    /// platform's static token. Kept as a fallback when no key is
    /// configured, and to verify tokens it signed after one is.
    pub fn derived_from_auth_token(auth_token: &str) -> Self {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"fcp-jwt-key-derivation")
            .expect("HMAC accepts any key length");
        mac.update(auth_token.as_bytes());
        Self {
            kid: "derived".to_string(),
            secret: mac.finalize().into_bytes().to_vec(),
        }
    }
}

fn hex8(bytes: &[u8]) -> String {
    bytes.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// The key tokens are signed with, and every key they are verified with.
#[derive(Debug, Clone)]
pub struct SessionKeys {
    signing: SessionKey,
    verifying: Vec<SessionKey>,
}

/// Why a token was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    Malformed,
    BadSignature,
    Expired,
}

impl SessionKeys {
    /// Sign with `signing`; verify with it and with each of `previous`.
    pub fn new(signing: SessionKey, previous: Vec<SessionKey>) -> Self {
        let mut verifying = vec![signing.clone()];
        verifying.extend(previous);
        Self { signing, verifying }
    }

    /// The keys a node uses: the configured key if there is one, else the
    /// one derived from the static token; the configured previous keys and
    /// the derived key are accepted for verification.
    pub fn from_config(
        session_signing_key: Option<&str>,
        previous_session_signing_keys: &[String],
        auth_token: Option<&str>,
    ) -> Option<Self> {
        let derived = auth_token.map(SessionKey::derived_from_auth_token);
        let mut previous: Vec<SessionKey> = previous_session_signing_keys
            .iter()
            .map(|k| SessionKey::configured(k))
            .collect();
        match session_signing_key {
            Some(key) => {
                previous.extend(derived);
                Some(Self::new(SessionKey::configured(key), previous))
            }
            None => derived.map(|d| Self::new(d, previous)),
        }
    }

    /// The signing key's id.
    pub fn kid(&self) -> &str {
        &self.signing.kid
    }

    /// An access token for `claims`.
    pub fn encode(&self, claims: &SessionClaims) -> String {
        let header = serde_json::json!({"alg": "HS256", "typ": "JWT", "kid": self.signing.kid});
        let header = b64(header.to_string().as_bytes());
        let payload = b64(&serde_json::to_vec(claims).expect("claims serialize"));
        let message = format!("{header}.{payload}");
        let signature = b64(&sign(&self.signing.secret, message.as_bytes()));
        format!("{message}.{signature}")
    }

    /// The claims of a token signed by one of the keys, unexpired at `now`.
    pub fn decode(&self, token: &str, now: SystemTime) -> Result<SessionClaims, TokenError> {
        let mut parts = token.split('.');
        let (Some(header), Some(payload), Some(signature), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(TokenError::Malformed);
        };
        let header: serde_json::Value = unb64(header)
            .and_then(|h| serde_json::from_slice(&h).ok())
            .ok_or(TokenError::Malformed)?;
        if header["alg"] != "HS256" {
            return Err(TokenError::Malformed);
        }
        let signature = unb64(signature).ok_or(TokenError::Malformed)?;
        let message = format!("{}.{payload}", token.split('.').next().unwrap_or_default());
        // A token names its key; one made before keys had ids is tried
        // against each.
        let candidates: Vec<&SessionKey> = match header["kid"].as_str() {
            Some(kid) => self.verifying.iter().filter(|k| k.kid == kid).collect(),
            None => self.verifying.iter().collect(),
        };
        let verified = candidates.iter().any(|key| {
            let mut mac =
                Hmac::<Sha256>::new_from_slice(&key.secret).expect("HMAC accepts any key length");
            mac.update(message.as_bytes());
            mac.verify_slice(&signature).is_ok()
        });
        if !verified {
            return Err(TokenError::BadSignature);
        }
        let claims: SessionClaims = unb64(payload)
            .and_then(|p| serde_json::from_slice(&p).ok())
            .ok_or(TokenError::Malformed)?;
        if claims.exp <= epoch(now) {
            return Err(TokenError::Expired);
        }
        Ok(claims)
    }
}

fn sign(secret: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn unb64(data: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data)
        .ok()
}

/// Seconds since the epoch.
pub fn epoch(t: SystemTime) -> usize {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as usize
}

/// A new refresh secret, and the hash a session keeps of it.
pub fn new_refresh_secret() -> (String, String) {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    let secret = b64(&bytes);
    let hash = refresh_hash(&secret);
    (secret, hash)
}

/// What a session keeps of a refresh secret.
pub fn refresh_hash(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A refresh cookie's value: the session and its secret.
pub fn refresh_cookie_value(session: &LoginSessionId, secret: &str) -> String {
    format!("{session}.{secret}")
}

/// The session and secret a refresh cookie names.
pub fn parse_refresh_cookie(value: &str) -> Option<(LoginSessionId, String)> {
    let (session, secret) = value.split_once('.')?;
    Some((session.parse().ok()?, secret.to_string()))
}

/// Who an access token is.
#[derive(Debug, Clone)]
pub struct SessionPrincipal {
    pub user: User,
    /// The roles the user's groups give them now (`docs/USERS.md` §5).
    pub group_roles: Vec<crate::UserRole>,
    /// `None` for a token made before sessions were rows.
    pub session: Option<LoginSessionId>,
}

/// Whether a user may be signed in at all: not suspended, not locked.
pub fn user_is_active(user: &User) -> bool {
    !user.is_suspended() && user.account_status != AccountStatus::Locked
}

/// Who `token` is, if it is a live session's access token for an active
/// user. The user is read afresh, so a demotion or a disable takes effect
/// at once; a token naming a session needs that session live, so a logout
/// does too.
pub async fn authenticate_session(
    token: &str,
    keys: &SessionKeys,
    users: &dyn CredentialStore,
    sessions: Option<&dyn LoginSessionStore>,
    now: SystemTime,
) -> Option<SessionPrincipal> {
    let claims = keys.decode(token, now).ok()?;
    // A sign-in halfway through its second factor is not signed in.
    if claims.purpose.is_some() {
        return None;
    }
    let tenant_id: TenantId = claims.tenant_id.parse().ok()?;
    let user = match (claims.sid.as_deref(), claims.sub.parse::<UserId>()) {
        // A session's token: the session must be live, and the user's.
        (Some(sid), Ok(user_id)) => {
            let sid: LoginSessionId = sid.parse().ok()?;
            if let Some(sessions) = sessions {
                let session = sessions.get_login_session(&sid).await.ok()??;
                if !session.is_live(now)
                    || session.user_id != user_id
                    || session.tenant_id != tenant_id
                {
                    return None;
                }
            }
            let user = users.get_user_by_id(&user_id).await.ok()??;
            (user.tenant_id == tenant_id).then_some(user)?
        }
        // A session's token whose subject is not an id: forged or broken.
        (Some(_), Err(_)) => return None,
        // A token from before sessions were rows: its subject is a
        // username, honoured until it expires (a day at most).
        (None, _) => users
            .get_user_in_tenant(&claims.sub, &tenant_id)
            .await
            .ok()??,
    };
    if !user_is_active(&user) {
        return None;
    }
    let group_roles = users
        .group_roles_of_user(&user.id)
        .await
        .unwrap_or_else(|e| {
            // Fewer roles, never more, until the groups can be read.
            tracing::warn!(user_id = %user.id, error = %e, "Could not read a user's groups");
            Vec::new()
        });
    Some(SessionPrincipal {
        session: claims.sid.as_deref().and_then(|s| s.parse().ok()),
        user,
        group_roles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(exp_in: i64) -> SessionClaims {
        let now = epoch(SystemTime::now()) as i64;
        SessionClaims {
            sub: UserId::new().to_string(),
            sid: Some(LoginSessionId::new().to_string()),
            username: Some("alice".into()),
            role: "end_user".into(),
            tenant_id: TenantId::new().to_string(),
            display_name: None,
            exp: (now + exp_in) as usize,
            iat: now as usize,
            purpose: None,
        }
    }

    #[test]
    fn a_token_verifies_by_its_key_and_after_a_rotation() {
        let old = SessionKeys::new(
            SessionKey::configured("old-key-0123456789-0123456789-ab"),
            vec![],
        );
        let token = old.encode(&claims(60));
        let now = SystemTime::now();
        assert_eq!(
            old.decode(&token, now).unwrap().username.as_deref(),
            Some("alice")
        );

        // Rotated: the new key signs, the old one still verifies.
        let rotated = SessionKeys::new(
            SessionKey::configured("new-key-0123456789-0123456789-ab"),
            vec![SessionKey::configured("old-key-0123456789-0123456789-ab")],
        );
        assert!(rotated.decode(&token, now).is_ok());
        assert_ne!(rotated.kid(), old.kid());

        // Dropped: refused.
        let dropped = SessionKeys::new(
            SessionKey::configured("new-key-0123456789-0123456789-ab"),
            vec![],
        );
        assert_eq!(dropped.decode(&token, now), Err(TokenError::BadSignature));
    }

    #[test]
    fn a_token_made_before_keys_had_ids_still_verifies() {
        // As the releases before this one signed: no kid, the derived key.
        let derived = SessionKey::derived_from_auth_token("platform-token");
        let header = b64(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = b64(&serde_json::to_vec(&claims(60)).unwrap());
        let message = format!("{header}.{payload}");
        let token = format!(
            "{message}.{}",
            b64(&sign(&derived.secret, message.as_bytes()))
        );
        let keys = SessionKeys::from_config(
            Some("configured-key-0123456789-0123456"),
            &[],
            Some("platform-token"),
        )
        .unwrap();
        assert!(keys.decode(&token, SystemTime::now()).is_ok());
    }

    #[test]
    fn an_expired_or_altered_token_is_refused() {
        let keys = SessionKeys::new(
            SessionKey::configured("k-0123456789-0123456789-0123456"),
            vec![],
        );
        let now = SystemTime::now();
        assert_eq!(
            keys.decode(&keys.encode(&claims(-1)), now),
            Err(TokenError::Expired)
        );
        let token = keys.encode(&claims(60));
        let mut parts: Vec<&str> = token.split('.').collect();
        let mut forged = claims(60);
        forged.role = "super_admin".into();
        let payload = b64(&serde_json::to_vec(&forged).unwrap());
        parts[1] = &payload;
        assert_eq!(
            keys.decode(&parts.join("."), now),
            Err(TokenError::BadSignature)
        );
        assert_eq!(keys.decode("not.a-token", now), Err(TokenError::Malformed));
        // `alg: none` is not a way around the signature.
        let none = format!("{}.{}.", b64(br#"{"alg":"none"}"#), payload);
        assert_eq!(keys.decode(&none, now), Err(TokenError::Malformed));
    }

    #[test]
    fn a_refresh_cookie_names_its_session_and_secret() {
        let session = LoginSessionId::new();
        let (secret, hash) = new_refresh_secret();
        assert!(secret.len() >= 40);
        assert_eq!(refresh_hash(&secret), hash);
        let (parsed, got) = parse_refresh_cookie(&refresh_cookie_value(&session, &secret)).unwrap();
        assert_eq!(parsed, session);
        assert_eq!(got, secret);
        assert!(parse_refresh_cookie("no-dot").is_none());
    }
}

#[cfg(test)]
mod app_tests {
    use super::*;

    #[test]
    fn a_pkce_challenge_is_the_rfc_7636_example() {
        // RFC 7636 Appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(valid_pkce_verifier(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        ));
        assert!(!valid_pkce_verifier("short"));
        assert!(!valid_pkce_verifier(&"a".repeat(42)));
        assert!(!valid_pkce_verifier(&format!("{}+", "a".repeat(50))));
    }

    #[test]
    fn an_app_is_sent_back_only_to_a_loopback_address() {
        for ok in [
            "http://127.0.0.1:53124/callback",
            "http://127.0.0.1/",
            "http://localhost:8000/cb?x=1",
            "http://[::1]:7000/cb",
        ] {
            assert!(valid_app_redirect(ok), "{ok}");
        }
        for bad in [
            "https://127.0.0.1:53124/callback",
            "http://evil.example/cb",
            "http://127.0.0.1.evil.example/cb",
            "http://user@127.0.0.1/cb",
            "http://[::1]evil/cb",
            "fcp://callback",
            "",
        ] {
            assert!(!valid_app_redirect(bad), "{bad}");
        }
    }
}
