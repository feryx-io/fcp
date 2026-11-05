//! A device's lines (`docs/DEVICE_LINES.md`): what each registers as, what
//! it is called, and whether the device may be it — one answer for the
//! admin API that checks a line, provisioning that writes it, `/me/softphone`
//! that tells an app of it, and the registrar that lets the device be it.

use fcp_core::calling::CallingStore;
use fcp_core::storage::{
    CredentialStore, ExtensionStore, LineBindingStore, PhoneNumberStore, TenantStore, User,
};
use fcp_core::{Device, DeviceLine, DynStorage, ExtensionTarget, LineKind, TenantId, UserId};
use std::collections::HashMap;

/// A line as a device carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLine {
    /// 1 for the first.
    pub index: u32,
    pub kind: LineKind,
    /// The line itself, canonical whatever the administrator typed:
    /// `own`, `extension:1005`, `shared:+15551234567`,
    /// `on_behalf:<username>` — what a user's name for it is kept under,
    /// and what `X-FCP-Line` names on a call for it.
    pub key: String,
    /// The user part the device registers the line as.
    pub aor_user: String,
    /// What it is called: the user's name for it (where the tenant lets
    /// them), else the administrator's, else `default_label`.
    pub label: String,
    /// `<extension> - <name>`, or the number, or the device's name.
    pub default_label: String,
    /// A shared line, with its appearances.
    pub shared: bool,
    pub appearances: u32,
    /// A shared line whose active calls another of its phones may join.
    pub barge_in: bool,
    pub ring: bool,
}

fn display(user: &User) -> String {
    user.shown_name().unwrap_or_else(|| user.username.clone())
}

/// A user of the tenant named by username or by id.
async fn user_named(storage: &DynStorage, tenant: &TenantId, name: &str) -> Option<User> {
    if let Ok(id) = name.parse::<UserId>() {
        if let Ok(Some(user)) = storage.get_user_by_id(&id).await {
            return (user.tenant_id == *tenant).then_some(user);
        }
    }
    storage
        .get_user_in_tenant(name, tenant)
        .await
        .ok()
        .flatten()
}

/// Whether `owner` is one of `executive`'s assistants.
async fn assists(
    storage: &DynStorage,
    tenant: &TenantId,
    owner: &UserId,
    executive: &UserId,
) -> bool {
    match storage.get_calling_settings(tenant, executive).await {
        Ok(Some(settings)) => settings
            .executive
            .as_ref()
            .is_some_and(|e| e.assistants.iter().any(|a| a.user_id == *owner)),
        _ => false,
    }
}

/// The lines a device carries: its own lines, or, with none written, its
/// owner's own line (provisioning still writes a phone's shared line as
/// its second account then, as before lines).
pub fn lines_of(device: &Device) -> Vec<DeviceLine> {
    if !device.lines.is_empty() {
        return device.lines.clone();
    }
    vec![DeviceLine {
        kind: LineKind::Own,
        value: String::new(),
        label: None,
        ring: true,
    }]
}

/// One line resolved, or why the device may not carry it.
async fn resolve_one(
    storage: &DynStorage,
    device: &Device,
    owner: Option<&User>,
    line: &DeviceLine,
) -> Result<(String, String, String, bool, u32, bool), String> {
    let tenant = &device.tenant_id;
    let value = line.value.trim();
    match line.kind {
        LineKind::Own => {
            let aor = device
                .aor_user()
                .map(str::to_string)
                .or_else(|| device.sip_username.clone())
                .or_else(|| owner.map(|o| o.sip_name().to_string()))
                .ok_or("the device has no SIP name of its own")?;
            let label = match owner {
                Some(owner) => fcp_core::device::default_line_label(
                    owner.extension.as_deref().unwrap_or(&owner.username),
                    Some(&display(owner)),
                ),
                None => device.name.clone(),
            };
            Ok(("own".to_string(), aor, label, false, 0, false))
        }
        LineKind::Extension => {
            let owner = owner.ok_or("a device with no owner holds no extension")?;
            let ext = storage
                .get_extension_by_number(tenant, value)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("{value} is no extension"))?;
            if ext.target != ExtensionTarget::User(owner.id) {
                return Err(format!("{value} is not {}'s extension", owner.username));
            }
            Ok((
                format!("extension:{value}"),
                value.to_string(),
                fcp_core::device::default_line_label(value, Some(&display(owner))),
                false,
                0,
                false,
            ))
        }
        LineKind::Shared => {
            let digits = value.trim_start_matches('+');
            let e164 = format!("+{digits}");
            let number = storage
                .get_phone_number_by_e164(&e164)
                .await
                .map_err(|e| e.to_string())?
                .filter(|n| n.tenant_id == *tenant)
                .ok_or_else(|| format!("{value} is no number of the tenant's"))?;
            let binding = storage
                .get_line_binding_by_phone_number(&number.id)
                .await
                .map_err(|e| e.to_string())?
                .filter(|b| b.shared)
                .ok_or_else(|| format!("{e164} is not a shared line"))?;
            if !binding.devices.iter().any(|d| d.device_id == device.id) {
                return Err(format!("the device is not on the shared line {e164}"));
            }
            let name = match binding.owner_user_id {
                Some(id) => storage
                    .get_user_by_id(&id)
                    .await
                    .ok()
                    .flatten()
                    .map(|u| display(&u)),
                None => None,
            };
            Ok((
                format!("shared:{e164}"),
                e164.clone(),
                fcp_core::device::default_line_label(&e164, name.as_deref()),
                true,
                u32::from(binding.max_appearances),
                binding.barge_in,
            ))
        }
        LineKind::OnBehalf => {
            let owner = owner.ok_or("a device with no owner acts for nobody")?;
            let executive = user_named(storage, tenant, value)
                .await
                .ok_or_else(|| format!("{value} is no user of the tenant's"))?;
            if !assists(storage, tenant, &owner.id, &executive.id).await {
                return Err(format!(
                    "{} is not one of {}'s assistants",
                    owner.username, executive.username
                ));
            }
            Ok((
                format!("on_behalf:{}", executive.username),
                executive.sip_name().to_string(),
                fcp_core::device::default_line_label(
                    executive
                        .extension
                        .as_deref()
                        .unwrap_or(&executive.username),
                    Some(&display(&executive)),
                ),
                false,
                0,
                false,
            ))
        }
    }
}

/// Every line of the device, or the first it may not carry.
pub async fn resolve(
    storage: &DynStorage,
    device: &Device,
    owner: Option<&User>,
    lines: &[DeviceLine],
) -> Result<Vec<ResolvedLine>, String> {
    let mut out = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let (key, aor_user, default_label, shared, appearances, barge_in) =
            resolve_one(storage, device, owner, line)
                .await
                .map_err(|why| format!("line {}: {why}", i + 1))?;
        out.push(ResolvedLine {
            index: i as u32 + 1,
            kind: line.kind,
            key,
            label: line
                .label
                .clone()
                .filter(|l| !l.trim().is_empty())
                .unwrap_or_else(|| default_label.clone()),
            default_label,
            aor_user,
            shared,
            appearances,
            barge_in,
            ring: line.ring,
        });
    }
    Ok(out)
}

/// The device's lines with the names its owner gave them, where the tenant
/// lets users rename: what a phone and an app show. A line the device can
/// no longer carry (an extension moved, an assistant removed) is left out.
pub async fn shown(storage: &DynStorage, device: &Device) -> Vec<ResolvedLine> {
    let owner = match device.owner_user_id {
        Some(id) => storage.get_user_by_id(&id).await.ok().flatten(),
        None => None,
    };
    let mut lines = Vec::new();
    for line in lines_of(device) {
        match resolve(storage, device, owner.as_ref(), std::slice::from_ref(&line)).await {
            Ok(mut one) => {
                let mut l = one.remove(0);
                l.index = lines.len() as u32 + 1;
                lines.push(l);
            }
            Err(why) => {
                tracing::warn!(device_id = %device.id, %why, "A line the device can no longer carry")
            }
        }
    }
    let may_rename = storage
        .get_tenant(&device.tenant_id)
        .await
        .ok()
        .flatten()
        .is_some_and(|t| t.lines.users_may_rename);
    if let (true, Some(owner), Some(store)) = (may_rename, owner, storage.line_label_store()) {
        let names: HashMap<String, String> = store.line_labels(&owner.id).await.unwrap_or_default();
        for line in &mut lines {
            if let Some(name) = names.get(&line.key) {
                line.label = name.clone();
            }
        }
    }
    lines
}

/// Whether `device` may register or call as `target`'s line because its
/// owner acts for them (`on_behalf`): the line is on the device and the
/// owner is still one of `target`'s assistants.
pub async fn acts_for(
    storage: &DynStorage,
    tenant: &TenantId,
    device: &Device,
    target: &UserId,
) -> bool {
    let Some(owner) = device.owner_user_id else {
        return false;
    };
    for line in device.lines.iter().filter(|l| l.kind == LineKind::OnBehalf) {
        if let Some(user) = user_named(storage, tenant, line.value.trim()).await {
            if user.id == *target && assists(storage, tenant, &owner, target).await {
                return true;
            }
        }
    }
    false
}

/// The header naming which of a device's lines a call rings it for
/// (`docs/DEVICE_LINES.md` §3): a line's key, as `/me/softphone` gives it.
pub const LINE_HEADER: &str = "X-FCP-Line";

/// Which line a call to a user is for, as a line's key: an executive's
/// screened call ringing the assistants is `on_behalf:<executive>`, a
/// shared line's call `shared:<number>`, a call to another of the user's
/// extensions `extension:<number>`, anything else `own`.
pub async fn line_of_call(
    storage: &DynStorage,
    tenant: &TenantId,
    callee: Option<&UserId>,
    dialed: Option<&str>,
    shared_number: Option<&str>,
    screened_for_executive: bool,
) -> Option<String> {
    if let Some(number) = shared_number {
        return Some(format!("shared:{number}"));
    }
    let user = match callee {
        Some(id) => storage.get_user_by_id(id).await.ok().flatten()?,
        None => return None,
    };
    if screened_for_executive {
        return Some(format!("on_behalf:{}", user.username));
    }
    let dialed = dialed
        .map(|d| {
            let d = d.trim().trim_start_matches('<');
            let d = d
                .strip_prefix("sips:")
                .or_else(|| d.strip_prefix("sip:"))
                .unwrap_or(d);
            d.split(['@', ';', '>']).next().unwrap_or(d).to_string()
        })
        .filter(|d| !d.is_empty());
    match dialed {
        Some(d)
            if user.extension.as_deref() != Some(d.as_str())
                && !d.eq_ignore_ascii_case(&user.username)
                && !d.eq_ignore_ascii_case(user.sip_name())
                && matches!(
                    storage.get_extension_by_number(tenant, &d).await,
                    Ok(Some(ext)) if ext.target == ExtensionTarget::User(user.id)
                ) =>
        {
            Some(format!("extension:{d}"))
        }
        _ => Some("own".to_string()),
    }
}
