//! Time-based one-time passwords (RFC 6238) for password sign-in
//! (`docs/IDENTITY_AND_DEVICES.md` phase 2b): HMAC-SHA1, six digits, a
//! thirty-second step, a step's drift either way accepted. What every
//! authenticator app speaks.
//!
//! A user's secret is kept sealed with their other secrets; a code's step is
//! recorded when it is used, so a code read over a shoulder is good once.
//! Ten recovery codes, kept as SHA-256 hashes, each good once, stand in for
//! a lost phone.

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds a code lasts.
pub const STEP_SECS: u64 = 30;
/// Digits in a code.
pub const DIGITS: u32 = 6;
/// Steps of drift either way a code is accepted at.
pub const DRIFT_STEPS: u64 = 1;
/// Recovery codes a user is given.
pub const RECOVERY_CODES: usize = 10;

/// A user's second factor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTotp {
    pub user_id: crate::UserId,
    pub tenant_id: crate::TenantId,
    /// The shared secret, base32 (RFC 4648, no padding).
    pub secret: String,
    /// When the user proved their app holds it; until then it signs
    /// nothing in.
    pub confirmed_at: Option<SystemTime>,
    /// The last step a code was accepted at.
    pub last_step: Option<u64>,
    /// SHA-256 (hex) of each recovery code not yet used.
    pub recovery_hashes: Vec<String>,
    pub created_at: SystemTime,
}

impl UserTotp {
    pub fn is_confirmed(&self) -> bool {
        self.confirmed_at.is_some()
    }
}

/// Where second factors are kept.
#[async_trait]
pub trait TotpStore: Send + Sync {
    async fn get_user_totp(&self, user_id: &crate::UserId) -> crate::Result<Option<UserTotp>>;

    /// Create or replace a user's (unconfirmed) secret.
    async fn put_user_totp(&self, totp: &UserTotp) -> crate::Result<()>;

    /// Confirm it, with its recovery codes.
    async fn confirm_user_totp(
        &self,
        user_id: &crate::UserId,
        recovery_hashes: &[String],
        step: u64,
        at: SystemTime,
    ) -> crate::Result<()>;

    /// Record `step` as used, if it is later than the last one: false for a
    /// replay.
    async fn use_totp_step(&self, user_id: &crate::UserId, step: u64) -> crate::Result<bool>;

    /// Spend a recovery code by its hash: false when it is not one.
    async fn use_recovery_code(&self, user_id: &crate::UserId, hash: &str) -> crate::Result<bool>;

    /// Replace the recovery codes.
    async fn set_recovery_codes(
        &self,
        user_id: &crate::UserId,
        recovery_hashes: &[String],
    ) -> crate::Result<()>;

    async fn delete_user_totp(&self, user_id: &crate::UserId) -> crate::Result<()>;
}

/// A new secret: 20 random bytes, base32.
pub fn new_secret() -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.truncate(20);
    data_encoding::BASE32_NOPAD.encode(&bytes)
}

/// The step `t` falls in.
pub fn step_at(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / STEP_SECS
}

/// The code for `step` (RFC 4226's dynamic truncation).
pub fn code_at(secret: &str, step: u64) -> Option<String> {
    let key = data_encoding::BASE32_NOPAD
        .decode(secret.trim_end_matches('=').to_ascii_uppercase().as_bytes())
        .ok()?;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&key).ok()?;
    mac.update(&step.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = (hash[hash.len() - 1] & 0x0f) as usize;
    let binary = (u32::from(hash[offset] & 0x7f) << 24)
        | (u32::from(hash[offset + 1]) << 16)
        | (u32::from(hash[offset + 2]) << 8)
        | u32::from(hash[offset + 3]);
    Some(format!(
        "{:0width$}",
        binary % 10u32.pow(DIGITS),
        width = DIGITS as usize
    ))
}

/// The step `code` is right for at `now`, within the drift; `None` when it
/// is right for none.
pub fn matching_step(secret: &str, code: &str, now: SystemTime) -> Option<u64> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() != DIGITS as usize || !code.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let current = step_at(now);
    (current.saturating_sub(DRIFT_STEPS)..=current + DRIFT_STEPS).find(|step| {
        code_at(secret, *step).is_some_and(|c| crate::secret::constant_time_eq(&c, &code))
    })
}

/// The `otpauth://` URI an authenticator app scans.
pub fn otpauth_uri(secret: &str, issuer: &str, account: &str) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    format!(
        "otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}",
        enc(issuer),
        enc(account),
        enc(issuer)
    )
}

/// New recovery codes, shown once, and the hashes kept of them.
pub fn new_recovery_codes() -> (Vec<String>, Vec<String>) {
    let codes: Vec<String> = (0..RECOVERY_CODES)
        .map(|_| {
            let hex = uuid::Uuid::new_v4().simple().to_string();
            format!("{}-{}", &hex[..5], &hex[5..10])
        })
        .collect();
    let hashes = codes.iter().map(|c| recovery_hash(c)).collect();
    (codes, hashes)
}

/// What is kept of a recovery code: its SHA-256, case and dashes aside.
pub fn recovery_hash(code: &str) -> String {
    let normal: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    Sha256::digest(normal.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// RFC 6238 appendix B's SHA-1 vectors (the eight-digit codes, read at
    /// six): the secret is the ASCII `12345678901234567890`.
    #[test]
    fn the_rfc_6238_vectors() {
        let secret = data_encoding::BASE32_NOPAD.encode(b"12345678901234567890");
        for (t, eight) in [
            (59u64, "94287082"),
            (1_111_111_109, "07081804"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
        ] {
            assert_eq!(
                code_at(&secret, t / STEP_SECS).unwrap(),
                eight[2..],
                "t={t}"
            );
        }
    }

    #[test]
    fn a_code_is_taken_a_step_either_way_and_no_further() {
        let secret = new_secret();
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let step = step_at(now);
        for drift in [step - 1, step, step + 1] {
            let code = code_at(&secret, drift).unwrap();
            assert_eq!(matching_step(&secret, &code, now), Some(drift));
        }
        let far = code_at(&secret, step + 2).unwrap();
        assert_eq!(matching_step(&secret, &far, now), None);
        assert_eq!(matching_step(&secret, "12345", now), None);
        assert_eq!(matching_step(&secret, "abcdef", now), None);
    }

    #[test]
    fn recovery_codes_are_kept_hashed_and_read_loosely() {
        let (codes, hashes) = new_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODES);
        assert_eq!(
            recovery_hash(&codes[0].to_uppercase().replace('-', " ")),
            hashes[0]
        );
        assert_ne!(hashes[0], hashes[1]);
    }

    #[test]
    fn an_otpauth_uri_names_the_issuer_and_account() {
        let uri = otpauth_uri("JBSWY3DPEHPK3PXP", "FCP Acme", "jane@acme.com");
        assert!(
            uri.starts_with("otpauth://totp/FCP%20Acme:jane%40acme.com?secret=JBSWY3DPEHPK3PXP")
        );
        assert!(uri.contains("&issuer=FCP%20Acme"));
    }
}
