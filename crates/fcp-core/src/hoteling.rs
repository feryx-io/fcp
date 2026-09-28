//! Hoteling (`docs/ENTERPRISE_CALLING.md` §3.8): a user logs in on a
//! shared desk phone — a host — and for as long as the session lives the
//! phone is theirs. It rings for their calls, not its owner's; calls from
//! it are theirs for caller ID, class of service, the record and billing.
//! A session ends when the guest logs out, an admin ends it, another login
//! replaces it, or it runs out.

use crate::{DeviceId, HotelingSessionId, TenantId, UserId};
use anyhow::Result as AnyhowResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// How long a session lasts when the tenant does not say.
pub const DEFAULT_SESSION_HOURS: u32 = 12;

/// The longest session a tenant may ask for: a week.
pub const MAX_SESSION_HOURS: u32 = 168;

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEnd {
    /// The guest logged out (`*89`, or `/v1/me/hoteling`).
    Logout,
    /// An admin ended it.
    Admin,
    /// Its time ran out.
    Expired,
    /// The guest logged in somewhere else, or someone else logged in on
    /// the same host.
    Replaced,
}

impl SessionEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Logout => "logout",
            Self::Admin => "admin",
            Self::Expired => "expired",
            Self::Replaced => "replaced",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "logout" => Some(Self::Logout),
            "admin" => Some(Self::Admin),
            "expired" => Some(Self::Expired),
            "replaced" => Some(Self::Replaced),
            _ => None,
        }
    }
}

/// A guest on a host phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotelingSession {
    pub id: HotelingSessionId,
    pub tenant_id: TenantId,
    /// The host.
    pub device_id: DeviceId,
    /// The guest.
    pub user_id: UserId,
    pub started_at: SystemTime,
    pub expires_at: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_by: Option<SessionEnd>,
}

impl HotelingSession {
    /// A session starting now for `length`.
    pub fn start(
        tenant_id: TenantId,
        device_id: DeviceId,
        user_id: UserId,
        length: Duration,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            id: HotelingSessionId::new(),
            tenant_id,
            device_id,
            user_id,
            started_at: now,
            expires_at: now + length,
            ended_at: None,
            ended_by: None,
        }
    }

    /// Not ended, and not yet run out at `now`.
    pub fn is_active_at(&self, now: SystemTime) -> bool {
        self.ended_at.is_none() && self.expires_at > now
    }
}

/// The user a device's calls are: its guest while a session lives, else
/// its owner.
pub fn effective_user(device: &crate::Device, session: Option<&HotelingSession>) -> Option<UserId> {
    session
        .filter(|s| s.device_id == device.id && s.is_active_at(SystemTime::now()))
        .map(|s| s.user_id)
        .or(device.owner_user_id)
}

/// The store of hoteling sessions. "Active" is not ended and not run
/// out; a host has at most one active session, and so has a guest.
#[async_trait]
pub trait HotelingStore: Send + Sync {
    /// Start `session`, ending (as [`SessionEnd::Replaced`]) the guest's
    /// other active session and the host's other guest, in one
    /// transaction. Returns the sessions it ended.
    async fn start_hoteling_session(
        &self,
        session: HotelingSession,
    ) -> AnyhowResult<Vec<HotelingSession>>;

    async fn get_hoteling_session(
        &self,
        id: &HotelingSessionId,
    ) -> AnyhowResult<Option<HotelingSession>>;

    /// The host's active session.
    async fn hoteling_session_for_device(
        &self,
        device_id: &DeviceId,
    ) -> AnyhowResult<Option<HotelingSession>>;

    /// The guest's active session.
    async fn hoteling_session_for_user(
        &self,
        user_id: &UserId,
    ) -> AnyhowResult<Option<HotelingSession>>;

    /// The tenant's sessions, newest first: the active ones, or with
    /// `include_ended` the last ones whatever their state.
    async fn list_hoteling_sessions(
        &self,
        tenant_id: &TenantId,
        include_ended: bool,
    ) -> AnyhowResult<Vec<HotelingSession>>;

    /// End the session if it is active; the session as ended, or `None`
    /// when it was not active.
    async fn end_hoteling_session(
        &self,
        id: &HotelingSessionId,
        by: SessionEnd,
    ) -> AnyhowResult<Option<HotelingSession>>;

    /// Mark every session run out by `now` as [`SessionEnd::Expired`] and
    /// return them. Each is returned to one caller only, so every node
    /// may sweep.
    async fn expire_hoteling_sessions(&self, now: SystemTime)
        -> AnyhowResult<Vec<HotelingSession>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Device, DeviceType};

    #[test]
    fn a_live_session_makes_the_guest_the_devices_user() {
        let tenant = TenantId::new();
        let owner = UserId::new();
        let guest = UserId::new();
        let mut host = Device::new("desk".into(), DeviceType::DeskPhone, tenant);
        host.owner_user_id = Some(owner);
        assert_eq!(effective_user(&host, None), Some(owner));

        let mut session = HotelingSession::start(tenant, host.id, guest, Duration::from_secs(3600));
        assert_eq!(effective_user(&host, Some(&session)), Some(guest));

        session.ended_at = Some(SystemTime::now());
        assert_eq!(effective_user(&host, Some(&session)), Some(owner));

        let run_out = HotelingSession {
            expires_at: SystemTime::now() - Duration::from_secs(1),
            ..HotelingSession::start(tenant, host.id, guest, Duration::ZERO)
        };
        assert_eq!(effective_user(&host, Some(&run_out)), Some(owner));

        let elsewhere =
            HotelingSession::start(tenant, DeviceId::new(), guest, Duration::from_secs(3600));
        assert_eq!(effective_user(&host, Some(&elsewhere)), Some(owner));
    }

    #[test]
    fn session_ends_round_trip() {
        for end in [
            SessionEnd::Logout,
            SessionEnd::Admin,
            SessionEnd::Expired,
            SessionEnd::Replaced,
        ] {
            assert_eq!(SessionEnd::parse(end.as_str()), Some(end));
        }
    }
}
