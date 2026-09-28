//! Executive-assistant (`docs/ENTERPRISE_CALLING.md` §3.10): what the call
//! path reads of the relation. The relation is the executive's: their
//! calling settings list their assistants, each with the assistant's own
//! opt-in, so it is read and synced with those settings. Which executives
//! an assistant serves is a scan of the tenant's settings — a feature code
//! or a page asks it, never a call being routed.

use fcp_core::storage::CredentialStore;
use fcp_core::{CallingSettings, CallingStore, DynStorage, TenantId, UserId};
use std::time::SystemTime;

/// The executives `assistant` is listed for, in the tenant.
pub async fn executives_of(
    storage: &DynStorage,
    tenant_id: &TenantId,
    assistant: &UserId,
) -> anyhow::Result<Vec<CallingSettings>> {
    Ok(storage
        .list_calling_settings(tenant_id)
        .await?
        .into_iter()
        .filter(|s| s.assistant(assistant).is_some())
        .collect())
}

/// The executive an assistant acts for by default: the first, by name,
/// they are opted in for (a call on behalf names no executive).
pub async fn first_executive(
    storage: &DynStorage,
    tenant_id: &TenantId,
    assistant: &UserId,
) -> Option<UserId> {
    let mut executives: Vec<(String, UserId)> = Vec::new();
    for settings in executives_of(storage, tenant_id, assistant).await.ok()? {
        if !settings.assistant(assistant).is_some_and(|a| a.opted_in) {
            continue;
        }
        let name = storage
            .get_user_by_id(&settings.user_id)
            .await
            .ok()
            .flatten()
            .map(|u| u.username)
            .unwrap_or_default();
        executives.push((name, settings.user_id));
    }
    executives.sort_by(|a, b| a.0.cmp(&b.0));
    executives.first().map(|(_, id)| *id)
}

/// Opt `assistant` in or out for every executive listing them: the
/// executives whose settings changed.
pub async fn set_opted_in(
    storage: &DynStorage,
    tenant_id: &TenantId,
    assistant: &UserId,
    opted_in: bool,
) -> anyhow::Result<Vec<UserId>> {
    let mut changed = Vec::new();
    for mut settings in executives_of(storage, tenant_id, assistant).await? {
        let Some(executive) = settings.executive.as_mut() else {
            continue;
        };
        for a in executive
            .assistants
            .iter_mut()
            .filter(|a| a.user_id == *assistant && a.opted_in != opted_in)
        {
            a.opted_in = opted_in;
        }
        settings.updated_at = SystemTime::now();
        changed.push(settings.user_id);
        storage.put_calling_settings(settings).await?;
    }
    Ok(changed)
}

/// What a leg ringing an assistant shows as the caller: "<caller> for
/// <executive>".
pub fn for_executive(caller: &str, executive: &str) -> String {
    format!("{caller} for {executive}")
}

/// The `Alert-Info` on a call an assistant takes for an executive: a phone
/// set to ring differently for it matches `executive`.
pub const EXECUTIVE_ALERT_INFO: &str = "<http://127.0.0.1/executive>;info=executive";
