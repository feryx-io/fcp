//! The ring plan (`docs/ENTERPRISE_CALLING.md` §2, §3.1): how a call to a
//! user is placed, decided in one place from the user's calling settings
//! and executed by every path that reaches a user — a number bound to
//! them, their extension, their own address, a flow's `Dial`, a hunt
//! group. The plan is either a treatment applied at once (forward all, do
//! not disturb, nobody registered) or the contacts to ring, for how long,
//! and what to do when they refuse or do not answer.

use crate::registrar::FcpRegistrar;
use fcp_core::storage::{
    CredentialStore, DeviceStore, ExtensionStore, TenantStore, VoicemailStore,
};
use fcp_core::{
    CallingSettings, CallingStore, DynStorage, LineBinding, TenantId, Treatment, Treatments, UserId,
};
use sip_registrar::Binding;
use std::time::Duration;
use tracing::{debug, warn};

/// What the plan decided.
#[derive(Debug, Clone, PartialEq)]
pub enum RingPlan<C = Binding> {
    /// Nothing rings: the treatment applies now, for this reason (what the
    /// voicemail hears: `unconditional`, `busy`, `unreachable`).
    Immediate {
        treatment: Treatment,
        reason: &'static str,
    },
    /// Ring these contacts for this long (the node's default when the
    /// user set none); then the treatments.
    Ring {
        contacts: Vec<C>,
        ring_time: Option<Duration>,
        treatments: Treatments,
    },
}

/// The plan for a user with these settings, these registered contacts and
/// a mailbox or not. The rules, in order: forward all; do not disturb;
/// nobody registered; otherwise ring, then the busy or no-answer treatment.
pub fn plan<C>(settings: &CallingSettings, contacts: Vec<C>, has_mailbox: bool) -> RingPlan<C> {
    if let Some(target) = &settings.forward_all {
        return RingPlan::Immediate {
            treatment: Treatment::Forward {
                target: target.clone(),
            },
            reason: "unconditional",
        };
    }
    let busy = fallback(&settings.forward_busy, has_mailbox, 486);
    if settings.dnd {
        return RingPlan::Immediate {
            treatment: busy,
            reason: "busy",
        };
    }
    if contacts.is_empty() {
        return RingPlan::Immediate {
            treatment: fallback(&settings.forward_unreachable, has_mailbox, 480),
            reason: "unreachable",
        };
    }
    RingPlan::Ring {
        contacts,
        ring_time: settings.no_answer(),
        treatments: Treatments {
            busy,
            no_answer: fallback(&settings.forward_no_answer, has_mailbox, 480),
        },
    }
}

/// A forward when set, else voicemail when there is one, else the code.
fn fallback(forward: &Option<fcp_core::CallTarget>, has_mailbox: bool, code: u16) -> Treatment {
    match forward {
        Some(target) => Treatment::Forward {
            target: target.clone(),
        },
        None if has_mailbox => Treatment::Voicemail,
        None => Treatment::Refuse { code },
    }
}

/// The URI of a registered contact, without the angle brackets and the
/// contact parameters a phone registered it with.
pub fn contact_uri(binding: &Binding) -> String {
    let contact = binding.contact();
    match (contact.find('<'), contact.find('>')) {
        (Some(start), Some(end)) if start < end => contact[start + 1..end].to_string(),
        _ => contact.split(';').next().unwrap_or(contact).to_string(),
    }
}

/// A final code that means the callee refused rather than did not answer.
pub fn is_busy_code(code: Option<u16>) -> bool {
    matches!(code, Some(486) | Some(600) | Some(603))
}

/// A user's settings, the defaults when none are stored (or the store
/// failed, which is logged: a call must still be placed).
pub async fn settings_for(
    storage: &DynStorage,
    tenant_id: &TenantId,
    user_id: &UserId,
) -> CallingSettings {
    match storage.get_calling_settings(tenant_id, user_id).await {
        Ok(Some(settings)) => settings,
        Ok(None) => CallingSettings::defaults(*tenant_id, *user_id),
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Failed to read calling settings; using the defaults");
            CallingSettings::defaults(*tenant_id, *user_id)
        }
    }
}

/// Whether the user has an enabled mailbox to fall back to.
pub async fn has_mailbox(storage: &DynStorage, user_id: &UserId) -> bool {
    match storage.get_mailbox_by_user(user_id).await {
        Ok(Some(mailbox)) => mailbox.enabled,
        Ok(None) => false,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Failed to look up the user's mailbox");
            false
        }
    }
}

/// The registered contacts of a line binding's enabled devices. With
/// `invite_domain`, a device's registrations are read as the tenant's
/// (domain aliases included); without, at the device's own SIP URI.
pub async fn contacts_for_binding(
    storage: &DynStorage,
    registrar: &FcpRegistrar,
    binding: &LineBinding,
    invite_domain: Option<&str>,
) -> Vec<Binding> {
    let mut contacts = Vec::new();
    for device_binding in binding.enabled_devices() {
        let device = match storage.get_device(&device_binding.device_id).await {
            Ok(Some(device)) => device,
            Ok(None) => {
                warn!(device_id = %device_binding.device_id, "Device of a line binding not found");
                continue;
            }
            Err(e) => {
                warn!(device_id = %device_binding.device_id, error = %e, "Failed to get a line binding's device");
                continue;
            }
        };
        let Some(sip_uri) = device.sip_uri.as_deref() else {
            debug!(device = %device.name, "Device has no SIP URI configured");
            continue;
        };
        let found = match invite_domain {
            Some(domain) => {
                registrar
                    .get_registrations_for_device(&binding.tenant_id, sip_uri, domain)
                    .await
            }
            None => registrar.get_registrations(sip_uri).await,
        };
        match found {
            Ok(found) if !found.is_empty() => {
                debug!(device = %device.name, sip_uri = %sip_uri, contacts = found.len(), "Found registrations for device");
                contacts.extend(found);
            }
            Ok(_) => {
                debug!(device = %device.name, sip_uri = %sip_uri, "Device not currently registered")
            }
            Err(e) => {
                warn!(device = %device.name, error = %e, "Failed to get registrations for device")
            }
        }
    }
    contacts
}

/// The registered contacts of a user: every enabled device the user
/// owns, else the registrations at the user's own address in each of
/// the tenant's domains.
pub async fn contacts_for_user(
    storage: &DynStorage,
    registrar: &FcpRegistrar,
    tenant_id: &TenantId,
    user_id: &UserId,
    invite_domain: &str,
) -> Vec<Binding> {
    let mut contacts = Vec::new();
    match storage.list_devices_by_user(user_id).await {
        Ok(devices) => {
            for device in devices.iter().filter(|d| d.enabled) {
                let Some(sip_uri) = device.sip_uri.as_deref() else {
                    continue;
                };
                match registrar
                    .get_registrations_for_device(tenant_id, sip_uri, invite_domain)
                    .await
                {
                    Ok(found) if !found.is_empty() => {
                        debug!(device_id = %device.id, sip_uri = %sip_uri, count = found.len(), "Found registrations for user device");
                        contacts.extend(found);
                    }
                    Ok(_) => {
                        debug!(device_id = %device.id, sip_uri = %sip_uri, "Device not registered")
                    }
                    Err(e) => {
                        warn!(device_id = %device.id, error = %e, "Failed to get registrations for device")
                    }
                }
            }
        }
        Err(e) => warn!(user_id = %user_id, error = %e, "Failed to list user devices"),
    }
    if contacts.is_empty() {
        contacts =
            registrations_at_users_aor(storage, registrar, tenant_id, user_id, invite_domain).await;
    }
    contacts
}

/// The registrations at `sip:<username>@<domain>` for each of the
/// tenant's domains and the one the INVITE named: a user with no device
/// rows whose phone registered as them.
pub async fn registrations_at_users_aor(
    storage: &DynStorage,
    registrar: &FcpRegistrar,
    tenant_id: &TenantId,
    user_id: &UserId,
    invite_domain: &str,
) -> Vec<Binding> {
    let user = match storage.get_user_by_id(user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => return Vec::new(),
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Failed to read the user to ring their own address");
            return Vec::new();
        }
    };
    let mut domains: Vec<String> = match storage.get_tenant(tenant_id).await {
        Ok(Some(tenant)) => tenant.sip_domains,
        _ => Vec::new(),
    };
    if !invite_domain.is_empty()
        && !domains
            .iter()
            .any(|d| d.eq_ignore_ascii_case(invite_domain))
    {
        domains.push(invite_domain.to_string());
    }
    for domain in domains {
        let aor = format!("sip:{}@{domain}", user.username);
        match registrar
            .get_registrations_for_device(tenant_id, &aor, invite_domain)
            .await
        {
            Ok(bindings) if !bindings.is_empty() => {
                debug!(user_id = %user_id, aor = %aor, count = bindings.len(), "Ringing the user's own address");
                return bindings;
            }
            Ok(_) => {}
            Err(e) => {
                warn!(user_id = %user_id, aor = %aor, error = %e, "Failed to read the registrations at the user's own address");
            }
        }
    }
    Vec::new()
}

/// The user a dialed name is: an extension pointing at a user, else a
/// username of the tenant.
pub async fn user_of(storage: &DynStorage, tenant_id: &TenantId, dialed: &str) -> Option<UserId> {
    if let Ok(Some(ext)) = storage.get_extension_by_number(tenant_id, dialed).await {
        if let fcp_core::ExtensionTarget::User(user_id) = ext.target {
            return Some(user_id);
        }
    }
    match storage.get_user_in_tenant(dialed, tenant_id).await {
        Ok(Some(user)) => Some(user.id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::CallTarget;

    fn settings() -> CallingSettings {
        CallingSettings::defaults(TenantId::new(), UserId::new())
    }

    #[test]
    fn a_plain_user_rings_then_gets_the_default_treatments() {
        let s = settings();
        match plan(&s, vec!["a", "b"], false) {
            RingPlan::Ring {
                contacts,
                ring_time,
                treatments,
            } => {
                assert_eq!(contacts, vec!["a", "b"]);
                assert_eq!(ring_time, None);
                assert_eq!(treatments.busy, Treatment::Refuse { code: 486 });
                assert_eq!(treatments.no_answer, Treatment::Refuse { code: 480 });
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_mailbox_catches_busy_no_answer_and_unreachable() {
        let s = settings();
        match plan(&s, vec!["a"], true) {
            RingPlan::Ring { treatments, .. } => {
                assert_eq!(treatments.busy, Treatment::Voicemail);
                assert_eq!(treatments.no_answer, Treatment::Voicemail);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            plan(&s, Vec::<&str>::new(), true),
            RingPlan::Immediate {
                treatment: Treatment::Voicemail,
                reason: "unreachable"
            }
        );
    }

    #[test]
    fn forwards_win_over_the_mailbox_in_their_own_case() {
        let mut s = settings();
        let ext = CallTarget::Extension("2002".into());
        s.forward_busy = Some(ext.clone());
        s.forward_no_answer = Some(CallTarget::Voicemail);
        s.forward_unreachable = Some(CallTarget::Number("+15551234567".into()));
        s.no_answer_secs = Some(12);
        match plan(&s, vec!["a"], true) {
            RingPlan::Ring {
                ring_time,
                treatments,
                ..
            } => {
                assert_eq!(ring_time, Some(Duration::from_secs(12)));
                assert_eq!(
                    treatments.busy,
                    Treatment::Forward {
                        target: ext.clone()
                    }
                );
                assert_eq!(
                    treatments.no_answer,
                    Treatment::Forward {
                        target: CallTarget::Voicemail
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            plan(&s, Vec::<&str>::new(), true),
            RingPlan::Immediate {
                treatment: Treatment::Forward {
                    target: CallTarget::Number("+15551234567".into())
                },
                reason: "unreachable"
            }
        );
    }

    #[test]
    fn dnd_is_the_busy_treatment_without_ringing() {
        let mut s = settings();
        s.dnd = true;
        assert_eq!(
            plan(&s, vec!["a"], false),
            RingPlan::Immediate {
                treatment: Treatment::Refuse { code: 486 },
                reason: "busy"
            }
        );
        s.forward_busy = Some(CallTarget::Extension("2002".into()));
        assert_eq!(
            plan(&s, vec!["a"], true),
            RingPlan::Immediate {
                treatment: Treatment::Forward {
                    target: CallTarget::Extension("2002".into())
                },
                reason: "busy"
            }
        );
    }

    #[test]
    fn forward_all_comes_before_everything() {
        let mut s = settings();
        s.dnd = true;
        s.forward_all = Some(CallTarget::Voicemail);
        assert_eq!(
            plan(&s, Vec::<&str>::new(), false),
            RingPlan::Immediate {
                treatment: Treatment::Forward {
                    target: CallTarget::Voicemail
                },
                reason: "unconditional"
            }
        );
    }

    #[test]
    fn the_ring_time_stays_in_range() {
        let mut s = settings();
        s.no_answer_secs = Some(1);
        match plan(&s, vec!["a"], false) {
            RingPlan::Ring { ring_time, .. } => assert_eq!(ring_time, Some(Duration::from_secs(5))),
            other => panic!("{other:?}"),
        }
    }
}
