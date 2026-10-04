//! Hoteling (`docs/ENTERPRISE_CALLING.md` §3.8): devices with credentials
//! of their own, who a credential may be over SIP, the user a phone's calls
//! are, and the sessions that put a guest on a host phone.

use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use fcp_core::hoteling::{HotelingSession, HotelingStore, SessionEnd};
use fcp_core::storage::{
    CredentialStore, DeviceStore, ExtensionStore, LineBindingStore, PhoneNumberStore, TenantStore,
    VoicemailStore,
};
use fcp_core::{Device, DeviceId, DynStorage, TenantId, User, UserId};
use fcp_events::{EventPublisher, PlatformEvent, UserEvent};
use tracing::{debug, info, warn};

use crate::registrar::FcpRegistrar;
use crate::uac::SipUserAgentClient;

/// Whose digest credentials a username is.
#[derive(Debug, Clone)]
pub enum Credential {
    User(Box<User>),
    Device(Box<Device>),
}

/// The enabled user, else the enabled device with credentials of its own,
/// that `name` is in the tenant. A user wins: the API refuses a device
/// username a user already has.
pub async fn credential(
    storage: &DynStorage,
    tenant_id: &TenantId,
    name: &str,
) -> Option<Credential> {
    if let Ok(Some(user)) = storage.get_user_by_sip_name(name, tenant_id).await {
        return user
            .may_use_services()
            .then_some(Credential::User(Box::new(user)));
    }
    match storage.get_device_by_sip_name(tenant_id, name).await {
        Ok(Some(device))
            if device.enabled
                && device
                    .sip_username
                    .as_deref()
                    .is_some_and(|u| u.eq_ignore_ascii_case(name)) =>
        {
            // A device whose owner is suspended goes quiet with them.
            if let Some(owner) = &device.owner_user_id {
                let suspended = storage
                    .get_user_by_id(owner)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|u| u.is_suspended());
                if suspended {
                    return None;
                }
            }
            Some(Credential::Device(Box::new(device)))
        }
        _ => None,
    }
}

/// Whether the credentials `credential_name` may register, or place calls
/// as, the name `aor_user` (the user part of an address of record or of a
/// `From`): their own name; for a user, a device they own, an extension
/// that is theirs, a number whose line is theirs; for a device, its own
/// address, its owner's name or extension, a number whose line it is on.
pub async fn may_act_as(
    storage: &DynStorage,
    tenant_id: &TenantId,
    credential_name: &str,
    aor_user: &str,
) -> bool {
    if credential_name.eq_ignore_ascii_case(aor_user) {
        return true;
    }
    match credential(storage, tenant_id, credential_name).await {
        Some(Credential::User(user)) => {
            if let Ok(Some(device)) = storage.get_device_by_sip_name(tenant_id, aor_user).await {
                if device.owner_user_id == Some(user.id)
                    && device
                        .aor_user()
                        .is_some_and(|u| u.eq_ignore_ascii_case(aor_user))
                {
                    return true;
                }
            }
            if let Ok(Some(ext)) = storage.get_extension_by_number(tenant_id, aor_user).await {
                if ext.target == fcp_core::ExtensionTarget::User(user.id) {
                    return true;
                }
            }
            line_of_number(storage, tenant_id, aor_user)
                .await
                .is_some_and(|line| line.owner_user_id == Some(user.id))
        }
        Some(Credential::Device(device)) => {
            if device
                .aor_user()
                .is_some_and(|u| u.eq_ignore_ascii_case(aor_user))
            {
                return true;
            }
            if let Some(owner) = device.owner_user_id {
                if crate::ring_plan::user_of(storage, tenant_id, aor_user).await == Some(owner) {
                    return true;
                }
            }
            line_of_number(storage, tenant_id, aor_user)
                .await
                .is_some_and(|line| line.devices.iter().any(|d| d.device_id == device.id))
        }
        None => false,
    }
}

/// The tenant's line for a number written as a user part: `+15551234567`
/// or its digits.
async fn line_of_number(
    storage: &DynStorage,
    tenant_id: &TenantId,
    name: &str,
) -> Option<fcp_core::LineBinding> {
    let digits = name.strip_prefix('+').unwrap_or(name);
    if digits.len() < 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number = match storage
        .get_phone_number_by_e164(&format!("+{digits}"))
        .await
    {
        Ok(Some(number)) if number.tenant_id == *tenant_id => number,
        _ => return None,
    };
    storage
        .get_line_binding_by_phone_number(&number.id)
        .await
        .ok()
        .flatten()
        .filter(|line| line.tenant_id == *tenant_id)
}

/// The host's live session, read only for a host.
pub async fn session_of(storage: &DynStorage, device: &Device) -> Option<HotelingSession> {
    if !device.hoteling_host {
        return None;
    }
    match storage.hoteling_session_for_device(&device.id).await {
        Ok(session) => session,
        Err(e) => {
            warn!(device_id = %device.id, error = %e, "Failed to read the host's hoteling session");
            None
        }
    }
}

/// The user a SIP name places calls as: a device's effective user — its
/// hoteling guest, else its owner — else the user the name is, by
/// username or extension.
pub async fn caller_user(storage: &DynStorage, tenant_id: &TenantId, name: &str) -> Option<UserId> {
    if let Ok(Some(device)) = storage.get_device_by_sip_name(tenant_id, name).await {
        if device.enabled {
            let session = session_of(storage, &device).await;
            if let Some(user) = fcp_core::hoteling::effective_user(&device, session.as_ref()) {
                // A suspended guest or owner places no calls.
                let suspended = storage
                    .get_user_by_id(&user)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|u| u.is_suspended());
                return (!suspended).then_some(user);
            }
        }
    }
    crate::ring_plan::user_of(storage, tenant_id, name).await
}

/// Whether the device is a host some other user than `user` is logged in
/// on: it rings for its guest, not its owner.
pub async fn lent_to_another(storage: &DynStorage, device: &Device, user: &UserId) -> bool {
    session_of(storage, device)
        .await
        .is_some_and(|s| s.user_id != *user)
}

/// The host a user is logged in on, if any.
pub async fn hosting(storage: &DynStorage, user: &UserId) -> Option<Device> {
    let session = match storage.hoteling_session_for_user(user).await {
        Ok(Some(session)) => session,
        Ok(None) => return None,
        Err(e) => {
            warn!(user_id = %user, error = %e, "Failed to read the user's hoteling session");
            return None;
        }
    };
    storage
        .get_device(&session.device_id)
        .await
        .ok()
        .flatten()
        .filter(|d| d.enabled && d.hoteling_host)
}

/// Starts and ends sessions, and tells the phones.
pub struct Hoteling {
    storage: Arc<DynStorage>,
    registrar: Option<Arc<FcpRegistrar>>,
    uac: Option<Arc<SipUserAgentClient>>,
    events: Option<Arc<dyn EventPublisher>>,
}

impl Hoteling {
    pub fn new(storage: Arc<DynStorage>) -> Self {
        Self {
            storage,
            registrar: None,
            uac: None,
            events: None,
        }
    }

    /// Reach the phones: their registrations, and a client to NOTIFY them.
    pub fn with_phones(
        mut self,
        registrar: Arc<FcpRegistrar>,
        uac: Arc<SipUserAgentClient>,
    ) -> Self {
        self.registrar = Some(registrar);
        self.uac = Some(uac);
        self
    }

    pub fn with_events(mut self, events: Arc<dyn EventPublisher>) -> Self {
        self.events = Some(events);
        self
    }

    pub fn storage(&self) -> &Arc<DynStorage> {
        &self.storage
    }

    /// Log `user` in on the host `device` for the tenant's session length,
    /// ending the user's other session and the host's other guest.
    pub async fn login(&self, device: &Device, user: &User) -> Result<HotelingSession> {
        anyhow::ensure!(
            device.hoteling_host,
            "device {} is not a hoteling host",
            device.id
        );
        anyhow::ensure!(
            device.tenant_id == user.tenant_id,
            "user {} is not in the host's tenant",
            user.id
        );
        let length = match self.storage.get_tenant(&device.tenant_id).await {
            Ok(Some(tenant)) => tenant.calling.hoteling_session(),
            _ => fcp_core::TenantCalling::default().hoteling_session(),
        };
        let session = HotelingSession::start(device.tenant_id, device.id, user.id, length);
        let replaced = self.storage.start_hoteling_session(session.clone()).await?;
        for old in &replaced {
            if old.device_id != device.id {
                self.check_sync(&old.device_id).await;
            }
            self.announce_end(old, SessionEnd::Replaced).await;
        }
        info!(
            session_id = %session.id,
            device_id = %device.id,
            user_id = %user.id,
            username = %user.username,
            "Hoteling login"
        );
        self.publish(UserEvent::HotelingLogin {
            session_id: session.id.to_string(),
            user_id: user.id.to_string(),
            username: user.username.clone(),
            device_id: device.id.to_string(),
            tenant_id: device.tenant_id,
            expires_at: session.expires_at,
            timestamp: SystemTime::now(),
        })
        .await;
        self.check_sync(&device.id).await;
        self.message_waiting(device, &user.id).await;
        Ok(session)
    }

    /// End the host's session, if it has one.
    pub async fn logout_device(&self, device: &Device, by: SessionEnd) -> Option<HotelingSession> {
        let session = session_of(&self.storage, device).await?;
        self.end(&session, by).await
    }

    /// End `session` if it is still active: the event, and the phone told.
    pub async fn end(&self, session: &HotelingSession, by: SessionEnd) -> Option<HotelingSession> {
        let ended = match self.storage.end_hoteling_session(&session.id, by).await {
            Ok(Some(ended)) => ended,
            Ok(None) => return None,
            Err(e) => {
                warn!(session_id = %session.id, error = %e, "Failed to end a hoteling session");
                return None;
            }
        };
        info!(session_id = %ended.id, device_id = %ended.device_id, reason = by.as_str(), "Hoteling session ended");
        self.check_sync(&ended.device_id).await;
        self.announce_end(&ended, by).await;
        Some(ended)
    }

    /// End every session that has run out; how many this node ended.
    pub async fn sweep(&self) -> usize {
        let expired = match self
            .storage
            .expire_hoteling_sessions(SystemTime::now())
            .await
        {
            Ok(expired) => expired,
            Err(e) => {
                warn!(error = %e, "Hoteling sweep failed");
                return 0;
            }
        };
        for session in &expired {
            info!(session_id = %session.id, device_id = %session.device_id, "Hoteling session expired");
            self.check_sync(&session.device_id).await;
            self.announce_end(session, SessionEnd::Expired).await;
        }
        expired.len()
    }

    async fn announce_end(&self, session: &HotelingSession, by: SessionEnd) {
        self.publish(UserEvent::HotelingLogout {
            session_id: session.id.to_string(),
            user_id: session.user_id.to_string(),
            device_id: session.device_id.to_string(),
            tenant_id: session.tenant_id,
            reason: by.as_str().to_string(),
            resynced: self.uac.is_some(),
            timestamp: SystemTime::now(),
        })
        .await;
    }

    async fn publish(&self, event: UserEvent) {
        if let Some(events) = &self.events {
            if let Err(e) = events.publish(PlatformEvent::User(event)).await {
                debug!(error = %e, "Failed to publish a hoteling event");
            }
        }
    }

    /// The registered contacts of a device.
    async fn contacts(&self, device: &Device) -> Vec<sip_core::SipUri> {
        let (Some(registrar), Some(sip_uri)) = (&self.registrar, device.sip_uri.as_deref()) else {
            return Vec::new();
        };
        let domain = sip_core::SipUri::parse(sip_uri)
            .map(|u| u.host().to_string())
            .unwrap_or_default();
        match registrar
            .get_registrations_for_device(&device.tenant_id, sip_uri, &domain)
            .await
        {
            Ok(bindings) => bindings
                .iter()
                .filter_map(|b| sip_core::SipUri::parse(&crate::ring_plan::contact_uri(b)).ok())
                .collect(),
            Err(e) => {
                warn!(device_id = %device.id, error = %e, "Failed to read the host's registrations");
                Vec::new()
            }
        }
    }

    /// The phone's resync NOTIFY (`check-sync`, in its vendor's variant),
    /// so a phone that re-provisions shows who it now belongs to. Each is
    /// sent from a task of its own: a phone that never answers must not
    /// hold anyone up.
    pub async fn check_sync(&self, device_id: &DeviceId) {
        let (Some(uac), Some(registrar)) = (&self.uac, &self.registrar) else {
            return;
        };
        let device = match self.storage.get_device(device_id).await {
            Ok(Some(device)) => device,
            _ => return,
        };
        crate::device_sync::resync_device(uac, registrar, &self.storage, &device).await;
    }

    /// The guest's message-waiting state, to the host.
    async fn message_waiting(&self, device: &Device, user: &UserId) {
        let Some(uac) = self.uac.clone() else { return };
        let mailbox = match self.storage.get_mailbox_by_user(user).await {
            Ok(Some(mailbox)) => mailbox,
            _ => return,
        };
        let status = match self.storage.get_mwi_status(&mailbox.id).await {
            Ok(status) => status,
            Err(e) => {
                debug!(user_id = %user, error = %e, "No message-waiting state for the guest");
                return;
            }
        };
        let body = format!(
            "Messages-Waiting: {}\r\nVoice-Message: {}/{} ({}/{})\r\n",
            if status.has_messages() { "yes" } else { "no" },
            status.new_messages,
            status.old_messages,
            status.urgent_new_messages,
            status.urgent_old_messages,
        );
        for contact in self.contacts(device).await {
            let (uac, body) = (Arc::clone(&uac), body.clone());
            tokio::spawn(async move {
                let _ = uac
                    .send_unsolicited_notify(
                        &contact,
                        "message-summary",
                        "application/simple-message-summary",
                        &body,
                    )
                    .await;
            });
        }
    }
}
