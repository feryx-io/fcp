//! A provisioned phone kept in step with its device (`docs/PROVISIONING.md`
//! §8, `docs/IDENTITY_AND_DEVICES.md` phase 7c): what it registers with is
//! written on the device, and it is told to fetch its configuration again
//! with its vendor's NOTIFY.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use dashmap::DashMap;
use fcp_core::config_template::PhoneVendor;
use fcp_core::device::DeviceRegistrationUpdate;
use fcp_core::storage::{CredentialStore, DeviceStore};
use fcp_core::{Device, DeviceId, DynStorage, RegistrationState, TenantId};
use tracing::{debug, warn};

use crate::registrar::FcpRegistrar;
use crate::uac::SipUserAgentClient;

/// The `Event` a vendor's phones take as "fetch your configuration again",
/// without a reboot where the phone allows it.
pub fn resync_event(vendor: Option<PhoneVendor>) -> &'static str {
    match vendor {
        Some(PhoneVendor::Cisco) => "resync",
        Some(PhoneVendor::Yealink | PhoneVendor::Polycom | PhoneVendor::Snom) => {
            "check-sync;reboot=false"
        }
        _ => "check-sync",
    }
}

/// The MACs a User-Agent carries (`80:5e:c0:a1:b2:c3`, `805ec0a1b2c3`,
/// `80-5E-C0-…`), lower case and bare.
pub fn macs_in(user_agent: &str) -> Vec<String> {
    let mut found = Vec::new();
    for word in user_agent.split(|c: char| !(c.is_ascii_hexdigit() || c == ':' || c == '-')) {
        let hex: String = word.chars().filter(char::is_ascii_hexdigit).collect();
        let separators = word.len() - hex.len();
        // Twelve digits, bare or in six separated pairs.
        // A MAC glued on with `_` (`…_0004f2a1b2c3`) is split off; a longer
        // run of hex is not one.
        if hex.len() == 12 && (separators == 0 || separators == 5) {
            found.push(hex.to_ascii_lowercase());
        }
    }
    found
}

/// The device a registration is for: the one whose own credentials it
/// used; else the one whose MAC its User-Agent carries; else, among the
/// devices of the user it registered as, the one whose last registration
/// or last served provisioning fetch came from the same address (a desk
/// phone registering with its owner's credentials, its REGISTER naming no
/// MAC); else the one whose SIP URI is the name it registered as; else
/// that user's only device.
pub async fn device_for_registration(
    storage: &DynStorage,
    tenant_id: &TenantId,
    aor_user: &str,
    auth_username: Option<&str>,
    user_agent: Option<&str>,
    source: Option<std::net::IpAddr>,
) -> Option<Device> {
    if let Some(auth) = auth_username {
        if let Ok(Some(d)) = storage.get_device_by_sip_name(tenant_id, auth).await {
            let own = d
                .sip_username
                .as_deref()
                .is_some_and(|u| u.eq_ignore_ascii_case(auth));
            if own && d.tenant_id == *tenant_id {
                return Some(d);
            }
        }
    }
    for mac in user_agent.map(macs_in).unwrap_or_default() {
        if let Ok(Some(d)) = storage.get_device_by_mac_address(&mac).await {
            if d.tenant_id == *tenant_id {
                return Some(d);
            }
        }
    }
    let users_devices: Vec<Device> = match storage.get_user_by_sip_name(aor_user, tenant_id).await {
        Ok(Some(user)) => storage
            .list_devices_by_user(&user.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.tenant_id == *tenant_id && !d.has_own_credentials())
            .collect(),
        _ => Vec::new(),
    };
    if let Some(ip) = source {
        if let Some(d) = by_address(storage, tenant_id, &users_devices, ip).await {
            return Some(d);
        }
    }
    if let Ok(Some(d)) = storage.get_device_by_sip_name(tenant_id, aor_user).await {
        if d.tenant_id == *tenant_id {
            return Some(d);
        }
    }
    match users_devices.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// The one device of `devices` last seen at `ip`: registered from it, or
/// served its configuration to it. None when it is none or several.
async fn by_address(
    storage: &DynStorage,
    tenant_id: &TenantId,
    devices: &[Device],
    ip: std::net::IpAddr,
) -> Option<Device> {
    let host_of = |a: &str| -> Option<std::net::IpAddr> {
        a.parse::<std::net::SocketAddr>()
            .map(|s| s.ip())
            .ok()
            .or_else(|| a.parse().ok())
    };
    let mut found: Vec<&Device> = Vec::new();
    for d in devices {
        let registered_here = d.last_ip_address.as_deref().and_then(host_of) == Some(ip);
        let fetched_here = match storage.provisioning_store() {
            Some(store) if !registered_here => {
                store
                    .device_fetches(tenant_id, &d.id, 20)
                    .await
                    .unwrap_or_default()
                    .iter()
                    .find(|f| f.outcome == "served")
                    .and_then(|f| f.address.as_deref().and_then(host_of))
                    == Some(ip)
            }
            _ => false,
        };
        if registered_here || fetched_here {
            found.push(d);
        }
    }
    match found.as_slice() {
        [only] => Some((*only).clone()),
        _ => None,
    }
}

/// How often an unchanged registration is written again.
const REWRITE_AFTER: Duration = Duration::from_secs(10 * 60);

/// Writes registrations on devices, at most once per [`REWRITE_AFTER`] for
/// one that has not changed: a phone re-registers every few minutes, and
/// each write is a row change.
pub struct DeviceRegistrations {
    storage: Arc<DynStorage>,
    written: DashMap<DeviceId, (Instant, RegistrationState, Option<String>, Option<String>)>,
}

impl DeviceRegistrations {
    pub fn new(storage: Arc<DynStorage>) -> Self {
        Self {
            storage,
            written: DashMap::new(),
        }
    }

    /// Note a REGISTER (`registered`) or a de-REGISTER, on a task of its
    /// own: the phone's answer does not wait for it.
    #[allow(clippy::too_many_arguments)]
    pub fn note(
        self: &Arc<Self>,
        tenant_id: TenantId,
        aor_user: String,
        auth_username: Option<String>,
        address: Option<String>,
        source: Option<std::net::IpAddr>,
        user_agent: Option<String>,
        registered: bool,
    ) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            this.write(
                tenant_id,
                &aor_user,
                auth_username.as_deref(),
                address,
                source,
                user_agent,
                registered,
            )
            .await;
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn write(
        &self,
        tenant_id: TenantId,
        aor_user: &str,
        auth_username: Option<&str>,
        address: Option<String>,
        source: Option<std::net::IpAddr>,
        user_agent: Option<String>,
        registered: bool,
    ) {
        let Some(device) = device_for_registration(
            &self.storage,
            &tenant_id,
            aor_user,
            auth_username,
            user_agent.as_deref(),
            source,
        )
        .await
        else {
            return;
        };
        let state = if registered {
            RegistrationState::Registered
        } else {
            RegistrationState::Unregistered
        };
        if let Some(last) = self.written.get(&device.id) {
            let (at, s, a, ua) = &*last;
            if *s == state && *a == address && *ua == user_agent && at.elapsed() < REWRITE_AFTER {
                return;
            }
        }
        let identity = user_agent
            .as_deref()
            .and_then(fcp_core::provisioning::identify)
            .unwrap_or_default();
        let update = DeviceRegistrationUpdate {
            state,
            at: SystemTime::now(),
            address: address.clone(),
            user_agent: user_agent.clone(),
            vendor: identity.vendor,
            model: identity.model,
            firmware: identity.firmware,
        };
        match self
            .storage
            .record_device_registration(&device.id, &update)
            .await
        {
            Ok(()) => {
                debug!(device_id = %device.id, ?state, "Wrote a registration on its device");
                self.written
                    .insert(device.id, (Instant::now(), state, address, user_agent));
            }
            Err(e) => {
                warn!(device_id = %device.id, error = %e, "Could not write a registration on its device")
            }
        }
    }
}

/// A device's registered contacts: under its own SIP URI or credentials,
/// else among its owner's, the one carrying its MAC or registered from its
/// last address — or the owner's only one when the device is the owner's
/// only phone.
pub async fn device_contacts(
    registrar: &FcpRegistrar,
    storage: &DynStorage,
    device: &Device,
) -> Vec<sip_core::SipUri> {
    let realm = registrar.realm().to_string();
    let lookup = |aor: String| {
        let realm = realm.clone();
        async move {
            match registrar
                .get_registrations_for_device(&device.tenant_id, &aor, &realm)
                .await
            {
                Ok(bindings) => bindings,
                Err(e) => {
                    debug!(device_id = %device.id, error = %e, "Could not read a device's registrations");
                    Vec::new()
                }
            }
        }
    };
    let to_uris = |bindings: Vec<sip_registrar::Binding>| -> Vec<sip_core::SipUri> {
        bindings
            .iter()
            .filter_map(|b| sip_core::SipUri::parse(&crate::ring_plan::contact_uri(b)).ok())
            .collect()
    };
    if let Some(uri) = device.sip_uri.as_deref() {
        let bindings = lookup(uri.to_string()).await;
        if !bindings.is_empty() {
            return to_uris(bindings);
        }
    }
    if let Some(name) = device.sip_username.as_deref() {
        let bindings = lookup(format!("sip:{name}@{realm}")).await;
        if !bindings.is_empty() {
            return to_uris(bindings);
        }
    }
    let Some(owner) = device.owner_user_id.as_ref() else {
        return Vec::new();
    };
    let Ok(Some(owner)) = storage.get_user_by_id(owner).await else {
        return Vec::new();
    };
    let bindings = lookup(format!("sip:{}@{realm}", owner.sip_name())).await;
    let mac = device
        .mac_address
        .as_deref()
        .and_then(fcp_core::provisioning::mac_of);
    let mine: Vec<sip_registrar::Binding> = bindings
        .iter()
        .filter(|b| {
            let by_mac = mac
                .as_ref()
                .is_some_and(|m| b.user_agent().is_some_and(|ua| macs_in(ua).contains(m)));
            let by_address = device
                .last_ip_address
                .as_deref()
                .is_some_and(|a| crate::ring_plan::contact_uri(b).contains(&format!("@{a}")));
            by_mac || by_address
        })
        .cloned()
        .collect();
    if !mine.is_empty() {
        return to_uris(mine);
    }
    let owners_phones = storage
        .list_devices_by_user(&owner.id)
        .await
        .map(|d| d.len())
        .unwrap_or(0);
    if owners_phones == 1 && bindings.len() == 1 {
        return to_uris(bindings);
    }
    Vec::new()
}

/// Tell a device's phone to fetch its configuration again: its vendor's
/// NOTIFY to each contact, each on a task of its own. How many were sent.
pub async fn resync_device(
    uac: &Arc<SipUserAgentClient>,
    registrar: &FcpRegistrar,
    storage: &DynStorage,
    device: &Device,
) -> usize {
    let event = resync_event(device.vendor);
    let contacts = device_contacts(registrar, storage, device).await;
    for contact in &contacts {
        let uac = Arc::clone(uac);
        let contact = contact.clone();
        tokio::spawn(async move {
            match uac
                .send_unsolicited_notify_with_state(
                    &contact,
                    event,
                    "application/simple-message-summary",
                    "",
                    "terminated",
                )
                .await
            {
                Ok(r) => {
                    debug!(contact = %contact.as_str(), code = r.code(), event, "Resync NOTIFY answered")
                }
                Err(e) => {
                    debug!(contact = %contact.as_str(), error = %e, event, "Resync NOTIFY failed")
                }
            }
        });
    }
    contacts.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_vendor_is_told_its_own_way() {
        assert_eq!(resync_event(Some(PhoneVendor::Cisco)), "resync");
        assert_eq!(
            resync_event(Some(PhoneVendor::Yealink)),
            "check-sync;reboot=false"
        );
        assert_eq!(
            resync_event(Some(PhoneVendor::Polycom)),
            "check-sync;reboot=false"
        );
        assert_eq!(
            resync_event(Some(PhoneVendor::Snom)),
            "check-sync;reboot=false"
        );
        assert_eq!(resync_event(Some(PhoneVendor::Grandstream)), "check-sync");
        assert_eq!(resync_event(None), "check-sync");
    }

    #[test]
    fn a_user_agent_carries_its_mac_in_any_form() {
        assert_eq!(
            macs_in("Yealink SIP-T54W 96.86.0.70 80:5e:c0:a1:b2:c3"),
            vec!["805ec0a1b2c3"]
        );
        assert_eq!(
            macs_in("PolycomVVX-VVX_450-UA/6.4.0.4006_0004F2A1B2C3"),
            vec!["0004f2a1b2c3"]
        );
        assert_eq!(
            macs_in("Grandstream GXP2170 1.0.11.6 DevId 000b82-a1-b2-c3"),
            Vec::<String>::new()
        );
        assert_eq!(
            macs_in("Fanvil X4U 2.12.1 0c-38-3e-11-22-33"),
            vec!["0c383e112233"]
        );
        assert!(macs_in("Zoiper rv2.10.8.3").is_empty());
        assert!(macs_in("deadbeefdeadbeef").is_empty());
    }
}
