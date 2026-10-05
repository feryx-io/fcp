//! The deployment's SIP realm, as the admin computes digest hashes in it
//! (`docs/SINGLE-NODE-SETUP.md`, `docs/PROVISIONING.md` §5).
//!
//! A user's or a device's `sip_ha1_hash` is `MD5(name:realm:password)`, so
//! it opens only in the realm it was made in: the admin must make it in
//! the realm the deployment's registrars challenge with. The admin takes
//! `[sip] realm` when it is set, else the realm the call managers keep in
//! the operational configuration (`call-manager` / `registrar`), else
//! `fcp.local`. When the realm changes, every hash whose password is kept
//! (for provisioning) is made again in the new one; the rest cannot be,
//! and are named so their passwords can be set again.

use fcp_core::config_store::ConfigStore;
use fcp_core::storage::{password::compute_ha1, CredentialStore, DeviceStore, TenantStore};
use fcp_core::DynStorage;
use tracing::{info, warn};

/// The realm before the admin had one of its own: every hash it made was
/// made in it.
pub const DEFAULT_REALM: &str = "fcp.local";

/// Where the realm came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmSource {
    /// `[sip] realm`.
    Configured,
    /// The call managers' `[registrar] realm`, from the operational
    /// configuration.
    CallManagers,
    Default,
}

/// The call managers' realm, as they keep it in the operational
/// configuration (`call-manager` / `registrar` / `_system`).
pub async fn call_managers_realm(config_store: &dyn ConfigStore) -> Option<String> {
    let entry = config_store
        .get_config("call-manager", "registrar", "_system")
        .await
        .ok()??;
    entry.value["realm"]
        .as_str()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(String::from)
}

/// The realm the admin computes hashes in, and where it came from.
pub async fn resolve(
    configured: Option<&str>,
    config_store: Option<&dyn ConfigStore>,
) -> (String, RealmSource) {
    if let Some(realm) = configured.map(str::trim).filter(|r| !r.is_empty()) {
        if let Some(store) = config_store {
            if let Some(theirs) = call_managers_realm(store).await {
                if !theirs.eq_ignore_ascii_case(realm) {
                    warn!(
                        admin = realm,
                        call_managers = %theirs,
                        "[sip] realm differs from the call managers' [registrar] realm: \
                         passwords set here will not open there"
                    );
                }
            }
        }
        return (realm.to_string(), RealmSource::Configured);
    }
    if let Some(store) = config_store {
        if let Some(theirs) = call_managers_realm(store).await {
            return (theirs, RealmSource::CallManagers);
        }
    }
    (DEFAULT_REALM.to_string(), RealmSource::Default)
}

/// What adopting a new realm did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rehashed {
    pub from: String,
    pub users: u64,
    pub devices: u64,
    /// `user <name> (tenant <id>)` or `device <id>`: a hash with no kept
    /// password, which cannot be made again.
    pub unrecoverable: Vec<String>,
}

const MARK_SERVICE: &str = "admin";
const MARK_SECTION: &str = "sip";
const MARK_TENANT: &str = "_system";

/// The realm the stored hashes were made in: the one recorded, else the
/// one the admin always used before it had a setting.
pub async fn recorded_realm(config_store: &dyn ConfigStore) -> String {
    config_store
        .get_config(MARK_SERVICE, MARK_SECTION, MARK_TENANT)
        .await
        .ok()
        .flatten()
        .and_then(|e| e.value["realm"].as_str().map(String::from))
        .unwrap_or_else(|| DEFAULT_REALM.to_string())
}

/// Make the stored hashes `realm`'s, once: when it differs from the realm
/// they were made in, every user's and device's hash whose password is
/// kept is made again; the rest are named. Recorded either way, so it
/// runs once per change. `None` when nothing changed.
pub async fn adopt(
    storage: &DynStorage,
    config_store: &dyn ConfigStore,
    realm: &str,
) -> anyhow::Result<Option<Rehashed>> {
    let from = recorded_realm(config_store).await;
    let record = || async {
        config_store
            .set_config(
                MARK_SERVICE,
                MARK_SECTION,
                MARK_TENANT,
                serde_json::json!({ "realm": realm }),
                Some("admin"),
            )
            .await
    };
    if from == realm {
        // Write it down the first time, so a later change knows.
        if config_store
            .get_config(MARK_SERVICE, MARK_SECTION, MARK_TENANT)
            .await?
            .is_none()
        {
            record().await?;
        }
        return Ok(None);
    }
    let mut done = Rehashed {
        from: from.clone(),
        ..Default::default()
    };
    for mut user in storage.list_users().await? {
        if user.sip_ha1_hash.is_none() {
            continue;
        }
        match user.sip_provisioning_password.clone() {
            Some(password) => {
                user.sip_ha1_hash = Some(compute_ha1(user.sip_name(), realm, &password));
                storage.put_user(user).await?;
                done.users += 1;
            }
            None => done.unrecoverable.push(format!(
                "user {} (tenant {})",
                user.username, user.tenant_id
            )),
        }
    }
    for tenant in storage.list_tenants().await? {
        for mut device in storage.list_devices(&tenant.id).await? {
            let (Some(username), Some(_)) = (device.sip_username.clone(), &device.sip_ha1_hash)
            else {
                continue;
            };
            match device.sip_provisioning_password.clone() {
                Some(password) => {
                    device.sip_ha1_hash = Some(compute_ha1(&username, realm, &password));
                    storage.put_device(device).await?;
                    done.devices += 1;
                }
                None => done.unrecoverable.push(format!("device {}", device.id)),
            }
        }
    }
    record().await?;
    info!(
        from = %from,
        to = realm,
        users = done.users,
        devices = done.devices,
        "The SIP realm changed: digest hashes made again"
    );
    if !done.unrecoverable.is_empty() {
        warn!(
            count = done.unrecoverable.len(),
            accounts = %done.unrecoverable.join(", "),
            "These digest hashes keep no password and stay in the old realm: set their SIP passwords again"
        );
    }
    Ok(Some(done))
}
