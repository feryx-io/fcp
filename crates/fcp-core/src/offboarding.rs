//! A deprovisioned user's purge (`docs/USERS.md` §4,
//! `docs/IDENTITY_AND_DEVICES.md` phase 5b).
//!
//! A delete suspends the user and marks them deprovisioned; once the
//! tenant's grace period has run, the purge takes them out of every store
//! in one transaction:
//!
//! - their devices are unassigned, and kept by the tenant;
//! - their lines are released (the numbers stay the tenant's), and the
//!   extensions that reach them removed;
//! - their mailbox is detached from the line and kept, closed, until the
//!   tenant's voicemail retention empties it; an empty detached mailbox is
//!   then removed;
//! - their calling settings, API tokens, hoteling session, queue
//!   memberships, declared location, registrations and presence go, and
//!   every other user's forwards, find-me steps and assistants forget them;
//! - their meetings move to the owner the tenant names, or to nobody;
//! - their profile picture goes on the file queue;
//! - the user row goes last, taking their sign-in sessions, second factor
//!   and identities with it.
//!
//! What the purge leaves is what retention and erasure decide: call
//! records, recordings, voicemail messages, audit entries. An erasure
//! request open for the user runs first: the purge waits for it.

use std::time::SystemTime;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{TenantId, User, UserId};

/// One purge, as asked.
#[derive(Debug, Clone)]
pub struct PurgeRequest {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    /// The grace period's end: a user deprovisioned after it is not yet
    /// due. `None` purges now (the platform admin's purge, a grace of 0).
    pub deprovisioned_before: Option<SystemTime>,
    /// Purge a user who is not deprovisioned, and without waiting for an
    /// open erasure: a site mirroring the cloud's delete, which has
    /// already decided.
    pub mirror: bool,
    /// The username the user's meetings move to.
    pub meetings_owner: Option<String>,
    /// The profile picture's file, for the file queue.
    pub profile_picture_path: Option<String>,
}

/// What a purge did, counted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PurgeReport {
    pub devices_unassigned: u64,
    pub lines_released: u64,
    pub extensions_released: u64,
    pub mailboxes_kept: u64,
    pub calling_settings_removed: u64,
    pub others_settings_changed: u64,
    pub api_tokens_removed: u64,
    pub hoteling_sessions_ended: u64,
    pub queue_memberships_removed: u64,
    pub declared_locations_removed: u64,
    pub meetings_moved: u64,
    pub meetings_unowned: u64,
    pub registrations_removed: u64,
    pub presence_removed: u64,
    pub files_queued: u64,
    /// Every registration address removed, for the call managers to
    /// forget (`registration.unregistered`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_aors: Vec<String>,
}

/// How a purge came out.
#[derive(Debug, Clone)]
pub enum PurgeOutcome {
    Purged {
        user: Box<User>,
        report: PurgeReport,
    },
    /// No such user: purged already, or never there.
    Gone,
    /// The user is not deprovisioned (restored since, or never deleted).
    NotDeprovisioned,
    /// The grace period has not run out.
    NotDue,
    /// An erasure request for the user is open; it runs first.
    ErasureOpen,
}

/// Where purges run.
#[async_trait]
pub trait OffboardingStore: Send + Sync {
    /// Purge one user, in one transaction, under the deleter in scope
    /// ([`crate::compliance::deleting_as`]).
    async fn purge_user(&self, request: &PurgeRequest) -> anyhow::Result<PurgeOutcome>;

    /// A tenant's users deprovisioned before `before`.
    async fn deprovisioned_users(
        &self,
        tenant_id: &TenantId,
        before: SystemTime,
    ) -> anyhow::Result<Vec<User>>;

    /// Remove a tenant's mailboxes whose user is purged once they hold no
    /// messages; how many.
    async fn remove_emptied_mailboxes(&self, tenant_id: &TenantId) -> anyhow::Result<u64>;
}

/// The day a deprovisioned user is purged: their deprovisioning plus the
/// grace period.
pub fn purge_after(user: &User, grace: std::time::Duration) -> Option<SystemTime> {
    user.deprovisioned_at
        .filter(|_| user.is_deprovisioned())
        .map(|at| at + grace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AccountStatus;

    #[test]
    fn only_a_deprovisioned_user_has_a_purge_day() {
        let mut user: User = serde_json::from_value(serde_json::json!({
            "username": "alice",
            "password": "",
            "display_name": null,
            "department_id": null,
            "enabled": true,
            "created_at": 0,
            "updated_at": 0,
            "email": null,
            "phone_number": null,
            "mobile_number": null,
            "pin": null,
            "extension": null,
            "department": null,
            "job_title": null,
            "location": null,
            "location_id": null,
            "primary_phone_number_id": null,
            "voicemail_box_id": null,
            "profile_picture": null,
            "digest_password": null,
            "sip_ha1_hash": null
        }))
        .unwrap();
        let day = std::time::Duration::from_secs(86_400);
        assert_eq!(purge_after(&user, day), None);

        user.account_status = AccountStatus::Deprovisioned;
        user.normalize_status();
        assert!(!user.enabled);
        let at = user.deprovisioned_at.expect("stamped");
        assert_eq!(purge_after(&user, day), Some(at + day));

        // A restore sets both: `enabled: false` alone still suspends.
        user.account_status = AccountStatus::Active;
        user.enabled = true;
        user.normalize_status();
        assert!(user.enabled);
        assert_eq!(user.deprovisioned_at, None);
        assert_eq!(purge_after(&user, day), None);
    }
}
