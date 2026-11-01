//! Deprovisioning, restoring and purging a user (`docs/USERS.md` §4,
//! `docs/IDENTITY_AND_DEVICES.md` phase 5b): what a delete, a restore, the
//! platform admin's purge and the hourly sweep share.

use std::time::{Duration, SystemTime};

use fcp_core::compliance::{deleting_as, Deleter, DeletionCause};
use fcp_core::offboarding::{PurgeOutcome, PurgeReport, PurgeRequest};
use fcp_core::{AccountStatus, CredentialStore, TenantStore, User};
use fcp_events::{PlatformEvent, SystemEvent, UserEvent};
use serde::Serialize;
use tracing::{info, warn};

use crate::error::ApiError;
use crate::state::ApiState;

/// How often the sweep looks for users whose grace period has run out.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

fn storage(state: &ApiState) -> Result<&fcp_core::DynStorage, ApiError> {
    state
        .storage
        .as_deref()
        .ok_or_else(|| ApiError::Internal("Storage not configured".to_string()))
}

async fn offboarding_of(
    state: &ApiState,
    tenant_id: &fcp_core::TenantId,
) -> Result<fcp_core::TenantOffboarding, ApiError> {
    Ok(storage(state)?
        .get_tenant(tenant_id)
        .await?
        .map(|t| t.offboarding)
        .unwrap_or_default())
}

async fn config_changed(state: &ApiState, user: &User) {
    // The call managers' credential caches hear of it.
    let _ = state
        .event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "user".to_string(),
            entity_id: user.id.to_string(),
            tenant_id: user.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
}

/// Delete a user: suspended at once, purged when the tenant's grace period
/// runs out (at once for a grace of 0). A user already deprovisioned is
/// left as they are.
pub async fn deprovision(state: &ApiState, user: User, actor: &str) -> Result<User, ApiError> {
    if user.is_deprovisioned() {
        return Ok(user);
    }
    let storage = storage(state)?;
    let offboarding = offboarding_of(state, &user.tenant_id).await?;
    let before = user.clone();
    let mut user = user;
    user.account_status = AccountStatus::Deprovisioned;
    user.normalize_status();
    user.updated_at = SystemTime::now();
    storage.put_user(user.clone()).await?;
    crate::v1::users::status_changed(state, &before, &user).await;

    let purge_after = fcp_core::offboarding::purge_after(&user, offboarding.grace())
        .unwrap_or_else(SystemTime::now);
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Deprovisioned {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            purge_after,
            timestamp: SystemTime::now(),
        }))
        .await;
    config_changed(state, &user).await;
    warn!(
        event = "user_deprovisioned",
        username = %user.username,
        tenant_id = %user.tenant_id,
        grace_days = offboarding.grace_days,
        actor,
        "User deleted; purged when the grace period ends"
    );

    if offboarding.grace_days == 0 {
        match purge(state, &user, None, actor).await? {
            PurgeOutcome::Purged { .. } | PurgeOutcome::Gone => {}
            // An erasure first, or a restore raced the delete: the sweep
            // comes back for them.
            other => info!(username = %user.username, outcome = ?other, "Purge deferred"),
        }
    }
    Ok(user)
}

/// Undo a delete within the grace period: the user is active again.
pub async fn restore(state: &ApiState, user: User) -> Result<User, ApiError> {
    if !user.is_deprovisioned() {
        return Err(ApiError::Conflict(format!(
            "User '{}' is not deleted",
            user.username
        )));
    }
    let storage = storage(state)?;
    let before = user.clone();
    let mut user = user;
    user.account_status = AccountStatus::Active;
    user.enabled = true;
    user.normalize_status();
    user.updated_at = SystemTime::now();
    storage.put_user(user.clone()).await?;
    crate::v1::users::status_changed(state, &before, &user).await;
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Restored {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
    config_changed(state, &user).await;
    info!(event = "user_restored", username = %user.username, tenant_id = %user.tenant_id, "User restored");
    Ok(user)
}

/// Purge a deprovisioned user: `deprovisioned_before` is the grace
/// period's end, or `None` for now. The removed registrations are
/// announced and `user.deleted` published.
pub async fn purge(
    state: &ApiState,
    user: &User,
    deprovisioned_before: Option<SystemTime>,
    actor: &str,
) -> Result<PurgeOutcome, ApiError> {
    let storage = storage(state)?;
    let Some(store) = storage.offboarding_store() else {
        return Err(ApiError::Internal(
            "This backend cannot purge users".to_string(),
        ));
    };
    let offboarding = offboarding_of(state, &user.tenant_id).await?;
    let request = PurgeRequest {
        tenant_id: user.tenant_id,
        user_id: user.id,
        deprovisioned_before,
        mirror: false,
        meetings_owner: offboarding.meetings_owner,
        profile_picture_path: user.profile_picture.as_deref().and_then(|file| {
            state
                .profile_pictures_dir
                .as_ref()
                .map(|dir| dir.join(file).to_string_lossy().into_owned())
        }),
    };
    let outcome = deleting_as(
        Deleter::new(DeletionCause::Offboarding, actor),
        store.purge_user(&request),
    )
    .await
    .map_err(|e| ApiError::Internal(format!("purging '{}': {e}", user.username)))?;
    if let PurgeOutcome::Purged { user, report } = &outcome {
        purged(state, user, report).await;
    }
    Ok(outcome)
}

async fn purged(state: &ApiState, user: &User, report: &PurgeReport) {
    for aor in &report.removed_aors {
        crate::v1::registrations::announce_removed(state, user.tenant_id, aor, "*").await;
    }
    let _ = state
        .event_bus
        .publish(PlatformEvent::User(UserEvent::Deleted {
            user_id: user.id.to_string(),
            username: user.username.clone(),
            tenant_id: user.tenant_id,
            timestamp: SystemTime::now(),
        }))
        .await;
    config_changed(state, user).await;
    warn!(
        event = "user_purged",
        username = %user.username,
        tenant_id = %user.tenant_id,
        report = ?report,
        "User purged"
    );
}

/// What one sweep did.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct SweepReport {
    pub purged: u64,
    /// Due, and waiting for an erasure asked for them.
    pub waiting_for_erasure: u64,
    pub mailboxes_removed: u64,
    /// Deleted contacts whose tombstones had done their work
    /// (`fcp_core::contact::TOMBSTONE_DAYS`).
    pub contact_tombstones_removed: u64,
    /// Deleted presence statuses whose tombstones had done their work
    /// (`fcp_core::presence_status::TOMBSTONE_DAYS`).
    pub status_tombstones_removed: u64,
    pub errors: Vec<String>,
}

/// Purge every user whose grace period ended by `now`, in every tenant,
/// and remove purged users' mailboxes that retention has emptied.
pub async fn run_sweep(state: &ApiState, now: SystemTime, actor: &str) -> SweepReport {
    let mut report = SweepReport::default();
    let Ok(storage) = storage(state) else {
        return report;
    };
    let Some(store) = storage.offboarding_store().cloned() else {
        return report;
    };
    let tenants = match storage.list_tenants().await {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(format!("tenants: {e}"));
            return report;
        }
    };
    for tenant in tenants {
        let before = now
            .checked_sub(tenant.offboarding.grace())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let due = match store.deprovisioned_users(&tenant.id, before).await {
            Ok(due) => due,
            Err(e) => {
                report.errors.push(format!("{}: {e}", tenant.id));
                continue;
            }
        };
        for user in due {
            match purge(state, &user, Some(before), actor).await {
                Ok(PurgeOutcome::Purged { .. }) => report.purged += 1,
                Ok(PurgeOutcome::ErasureOpen) => report.waiting_for_erasure += 1,
                Ok(_) => {}
                Err(e) => report.errors.push(format!("{}: {e:?}", user.username)),
            }
        }
        match deleting_as(
            Deleter::new(DeletionCause::Offboarding, actor),
            store.remove_emptied_mailboxes(&tenant.id),
        )
        .await
        {
            Ok(n) => report.mailboxes_removed += n,
            Err(e) => report.errors.push(format!("{} mailboxes: {e}", tenant.id)),
        }
    }
    if let Some(contacts) = storage.contact_store() {
        let before = now
            .checked_sub(Duration::from_secs(
                fcp_core::contact::TOMBSTONE_DAYS * 24 * 3600,
            ))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        match contacts.sweep_contact_tombstones(before).await {
            Ok(n) => report.contact_tombstones_removed += n,
            Err(e) => report.errors.push(format!("contact tombstones: {e}")),
        }
    }
    if let Some(statuses) = storage.presence_status_store() {
        let before = now
            .checked_sub(Duration::from_secs(
                fcp_core::presence_status::TOMBSTONE_DAYS * 24 * 3600,
            ))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        match statuses.sweep_status_tombstones(before).await {
            Ok(n) => report.status_tombstones_removed += n,
            Err(e) => report.errors.push(format!("status tombstones: {e}")),
        }
    }
    report
}

/// The sweep, once an hour, by one admin replica at a time.
pub fn spawn_sweep(state: ApiState, actor: String) {
    tokio::spawn(async move {
        use fcp_core::storage::CdrStore;
        let mut interval = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            interval.tick().await;
            let Some(storage) = state.storage.clone() else {
                continue;
            };
            let lock = match storage.try_maintenance_lock("fcp:offboarding").await {
                Ok(Some(lock)) => lock,
                Ok(None) => continue,
                Err(e) => {
                    warn!(error = %e, "Offboarding sweep: could not take the lock");
                    continue;
                }
            };
            let report = run_sweep(&state, SystemTime::now(), &actor).await;
            if report != SweepReport::default() {
                info!(report = ?report, "Offboarding sweep completed");
            }
            lock.release().await;
        }
    });
}
