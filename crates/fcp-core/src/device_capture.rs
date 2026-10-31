//! Captures from people's apps (`docs/CALL_DIAGNOSTICS.md` §3): a call as
//! the phone saw it — its SIP before TLS, its RTP and RTCP as on the wire,
//! the SRTP keys — written by the app as pcapng and uploaded when the call
//! ends.
//!
//! Off by default. A tenant turns captures on (`diagnostics.capture_allowed`),
//! then an administrator allows a user or one device until a date at most
//! [`MAX_ALLOW_DAYS`] away. An uploaded capture is kept for the tenant's
//! `diagnostics.capture_keep_days` unless a legal hold keeps it longer.

use crate::{CallId, DeviceId, TenantId, UserId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// The longest an allow may run.
pub const MAX_ALLOW_DAYS: u64 = 30;

/// The first four bytes of every pcapng file: its section header block.
pub const PCAPNG_MAGIC: [u8; 4] = [0x0a, 0x0d, 0x0d, 0x0a];

/// Whom an allow lets capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AllowSubject {
    /// Every app the user signs in to.
    User { user_id: UserId },
    /// One app.
    Device { device_id: DeviceId },
}

/// An administrator's allow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureAllow {
    pub id: uuid::Uuid,
    pub tenant_id: TenantId,
    pub subject: AllowSubject,
    /// Captures may be taken until this instant.
    pub until: SystemTime,
    /// Why, for the record.
    pub reason: Option<String>,
    pub created_by: String,
    pub created_at: SystemTime,
}

impl CaptureAllow {
    pub fn is_active(&self, now: SystemTime) -> bool {
        self.until > now
    }

    /// Whether this allow covers `device` used by `user`.
    pub fn covers(&self, user: &UserId, device: &DeviceId) -> bool {
        match &self.subject {
            AllowSubject::User { user_id } => user_id == user,
            AllowSubject::Device { device_id } => device_id == device,
        }
    }
}

/// The latest end among `allows` still active for `device` used by `user`.
pub fn allowed_until(
    allows: &[CaptureAllow],
    user: &UserId,
    device: &DeviceId,
    now: SystemTime,
) -> Option<SystemTime> {
    allows
        .iter()
        .filter(|a| a.is_active(now) && a.covers(user, device))
        .map(|a| a.until)
        .max()
}

/// The furthest an allow made at `now` may run.
pub fn latest_until(now: SystemTime) -> SystemTime {
    now + Duration::from_secs(MAX_ALLOW_DAYS * 86_400)
}

/// An uploaded capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCapture {
    pub id: uuid::Uuid,
    pub tenant_id: TenantId,
    /// The app that took it, which may since have been removed.
    pub device_id: DeviceId,
    pub user_id: UserId,
    /// The call it holds, when the app named one.
    pub call_id: Option<CallId>,
    pub size: u64,
    pub sha256: String,
    /// Where the file is, in the captures' store.
    pub file_key: String,
    /// The user's subject keys when it was uploaded, so a legal hold on
    /// them finds it (`crate::legal_hold`).
    pub subject_keys: Vec<String>,
    pub uploaded_at: SystemTime,
    /// When retention removes it, unless a hold keeps it.
    pub expires_at: SystemTime,
}

impl DeviceCapture {
    /// The store's key for a capture: `<tenant>/<id>.pcapng`.
    pub fn key_for(tenant: &TenantId, id: &uuid::Uuid) -> String {
        format!("{tenant}/{}.pcapng", id.simple())
    }
}

#[async_trait]
pub trait DeviceCaptureStore: Send + Sync {
    async fn put_capture_allow(&self, allow: &CaptureAllow) -> anyhow::Result<()>;
    /// The tenant's allows, ended ones included, newest first.
    async fn list_capture_allows(&self, tenant: &TenantId) -> anyhow::Result<Vec<CaptureAllow>>;
    async fn get_capture_allow(
        &self,
        tenant: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<CaptureAllow>>;
    /// Remove an allow; whether there was one.
    async fn delete_capture_allow(
        &self,
        tenant: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<bool>;
    /// The allows that could cover `device` used by `user`, active or not.
    async fn capture_allows_for(
        &self,
        tenant: &TenantId,
        user: &UserId,
        device: &DeviceId,
    ) -> anyhow::Result<Vec<CaptureAllow>>;

    async fn put_device_capture(&self, capture: &DeviceCapture) -> anyhow::Result<()>;
    /// The tenant's captures, newest first, of one device or user when named.
    async fn list_device_captures(
        &self,
        tenant: &TenantId,
        device: Option<&DeviceId>,
        user: Option<&UserId>,
    ) -> anyhow::Result<Vec<DeviceCapture>>;
    async fn get_device_capture(
        &self,
        tenant: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<DeviceCapture>>;
    /// Remove a capture's row, returning it; refused with a
    /// [`crate::legal_hold::LegalHoldViolation`] while a hold covers it.
    /// The file is the caller's to remove.
    async fn delete_device_capture(
        &self,
        tenant: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<DeviceCapture>>;
    /// Up to `limit` captures past their expiry that no hold covers.
    async fn expired_device_captures(
        &self,
        now: SystemTime,
        limit: usize,
    ) -> anyhow::Result<Vec<DeviceCapture>>;

    /// Up to `limit` removed captures' files due to be removed from the
    /// store (however their rows went), by key.
    async fn due_capture_files(&self, limit: usize) -> anyhow::Result<Vec<String>>;
    /// A removed capture's file is gone.
    async fn capture_file_removed(&self, key: &str) -> anyhow::Result<()>;
    /// Removing it failed: try again after `retry`.
    async fn capture_file_failed(
        &self,
        key: &str,
        error: &str,
        retry: Duration,
    ) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow(subject: AllowSubject, until: SystemTime) -> CaptureAllow {
        CaptureAllow {
            id: uuid::Uuid::new_v4(),
            tenant_id: TenantId::new(),
            subject,
            until,
            reason: None,
            created_by: "admin".into(),
            created_at: SystemTime::now(),
        }
    }

    #[test]
    fn a_user_or_device_is_allowed_until_the_latest_active_allow() {
        let now = SystemTime::now();
        let (user, other) = (UserId::new(), UserId::new());
        let (device, phone) = (DeviceId::new(), DeviceId::new());
        let day = Duration::from_secs(86_400);
        let allows = vec![
            allow(AllowSubject::User { user_id: user }, now + day),
            allow(AllowSubject::Device { device_id: device }, now + 3 * day),
            allow(AllowSubject::User { user_id: other }, now + 9 * day),
            allow(AllowSubject::Device { device_id: phone }, now - day),
        ];
        assert_eq!(
            allowed_until(&allows, &user, &device, now),
            Some(now + 3 * day)
        );
        assert_eq!(allowed_until(&allows, &user, &phone, now), Some(now + day));
        // An ended allow lets nobody.
        assert_eq!(allowed_until(&allows, &UserId::new(), &phone, now), None);
        assert!(latest_until(now) == now + 30 * day);
    }
}
