//! Supervising a call (`docs/ENTERPRISE_CALLING.md` §3.11): a supervisor
//! monitors a call (hears both parties), whispers (heard by one party
//! alone) or barges (heard by both).
//!
//! The call must be anchored: the supervisor is a third party on the call's
//! own media session (`fcp_media::MediaManager::add_party`, forge-engine's
//! party), mixed in by forge-mixer's `SupervisionMix`. This node dials the
//! supervisor's phone with the party's SDP; the supervisor hanging up, a
//! request to leave, or the call ending takes them off. A phone on a shared
//! line that allows barge-in comes in the other way — its own INVITE naming
//! an active appearance — and is answered with the party's SDP
//! ([`Supervisions::barge_in`]).

use crate::registrar::FcpRegistrar;
use crate::uac::SipUserAgentClient;
use dashmap::DashMap;
use fcp_b2bua_core::{CallId, CallManager, CallState};
use fcp_core::{DynStorage, UserId};
use fcp_media::{MediaLeg, MediaManager, Supervision};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

/// How long the supervisor's phone may ring.
const RING_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a call cannot be supervised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuperviseError {
    /// No such call on this node.
    NoSuchCall,
    /// The call is not up, or its media does not pass through this node.
    NotAnchored,
    /// Somebody else supervises it.
    AlreadySupervised,
    /// The supervisor has no phone up.
    Unreachable(String),
    /// The supervisor's phone refused (its final status).
    Refused(u16),
    /// The media could not take a third party.
    Media(String),
}

impl SuperviseError {
    /// The HTTP status an API answers with.
    pub fn status(&self) -> u16 {
        match self {
            Self::NoSuchCall => 404,
            Self::NotAnchored | Self::AlreadySupervised => 409,
            Self::Unreachable(_) | Self::Refused(_) => 480,
            Self::Media(_) => 500,
        }
    }
}

impl std::fmt::Display for SuperviseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchCall => f.write_str("no such call"),
            Self::NotAnchored => f.write_str("the call is not up with its media through this node"),
            Self::AlreadySupervised => f.write_str("the call already has a supervisor"),
            Self::Unreachable(who) => write!(f, "{who} has no phone up"),
            Self::Refused(code) => write!(f, "the supervisor's phone answered {code}"),
            Self::Media(e) => write!(f, "media: {e}"),
        }
    }
}

impl std::error::Error for SuperviseError {}

/// A call's supervisor.
#[derive(Debug, Clone)]
struct Active {
    mode: Supervision,
    /// Who: the target asked for, or the barging phone.
    party: String,
    user_id: Option<UserId>,
    /// The supervisor's dialog, once up.
    dialog: Option<sip_dialog::Dialog>,
    /// The call's media session.
    session: fcp_core::CallId,
}

/// A supervision as the API shows it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SupervisionView {
    pub mode: &'static str,
    /// For a whisper, the party that hears it (`a` or `b`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard_by: Option<&'static str>,
    pub party: String,
    /// The supervisor, when a user of the tenant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

/// The calls supervised on this node.
pub struct Supervisions {
    call_manager: Arc<CallManager>,
    media: Arc<MediaManager>,
    uac: Arc<SipUserAgentClient>,
    storage: Option<Arc<DynStorage>>,
    registrar: Option<Arc<FcpRegistrar>>,
    active: DashMap<CallId, Active>,
    /// The supervisor's SIP Call-ID → the call they supervise.
    by_dialog: DashMap<String, CallId>,
}

impl Supervisions {
    pub fn new(
        call_manager: Arc<CallManager>,
        media: Arc<MediaManager>,
        uac: Arc<SipUserAgentClient>,
        storage: Option<Arc<DynStorage>>,
        registrar: Option<Arc<FcpRegistrar>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            call_manager,
            media,
            uac,
            storage,
            registrar,
            active: DashMap::new(),
            by_dialog: DashMap::new(),
        })
    }

    /// The call's supervision, if it has one.
    pub fn of(&self, call_id: &CallId) -> Option<SupervisionView> {
        self.active.get(call_id).map(|a| SupervisionView {
            mode: a.mode.as_str(),
            heard_by: match a.mode {
                Supervision::Whisper(MediaLeg::A) => Some("a"),
                Supervision::Whisper(MediaLeg::B) => Some("b"),
                _ => None,
            },
            party: a.party.clone(),
            user_id: a.user_id.map(|u| u.to_string()),
        })
    }

    /// The anchored, established call and its media session.
    fn anchored(&self, call_id: &CallId) -> Result<fcp_b2bua_core::Call, SuperviseError> {
        let call = self
            .call_manager
            .get_call(call_id)
            .ok_or(SuperviseError::NoSuchCall)?;
        if call.state != CallState::Established
            || !call.media_anchored
            || call.media_session_id.is_none()
        {
            return Err(SuperviseError::NotAnchored);
        }
        Ok(call)
    }

    /// Put `target` (a user's extension or name, or a SIP URI) on the call
    /// in `mode`: their phone is dialed with the party's SDP and joined
    /// when it answers. A call supervised by the same target changes mode.
    pub async fn supervise(
        &self,
        call_id: &CallId,
        target: &str,
        mode: Supervision,
    ) -> Result<(), SuperviseError> {
        let call = self.anchored(call_id)?;
        let session = call.media_session_id.ok_or(SuperviseError::NotAnchored)?;
        if let Some(mut active) = self.active.get_mut(call_id) {
            if active.party != target {
                return Err(SuperviseError::AlreadySupervised);
            }
            self.media
                .set_party_mode(&session, mode)
                .await
                .map_err(|e| SuperviseError::Media(e.to_string()))?;
            active.mode = mode;
            info!(call_id = %call_id, mode = mode.as_str(), "The supervisor's mode changed");
            return Ok(());
        }
        let (contact, user_id) = self.reach(&call.tenant_id, target).await?;
        // Taken now, so a second request waits for none of this.
        match self.active.entry(*call_id) {
            dashmap::Entry::Occupied(_) => return Err(SuperviseError::AlreadySupervised),
            dashmap::Entry::Vacant(slot) => {
                slot.insert(Active {
                    mode,
                    party: target.to_string(),
                    user_id,
                    dialog: None,
                    session,
                });
            }
        }
        match self.dial(call_id, &session, &contact, mode).await {
            Ok(dialog) => {
                self.by_dialog
                    .insert(dialog.id().call_id().to_string(), *call_id);
                if let Some(mut active) = self.active.get_mut(call_id) {
                    active.dialog = Some(dialog);
                }
                info!(call_id = %call_id, supervisor = %target, mode = mode.as_str(), "A supervisor joined");
                self.joined(call_id, mode, target, user_id);
                Ok(())
            }
            Err(e) => {
                self.active.remove(call_id);
                let _ = self.media.remove_party(&session).await;
                Err(e)
            }
        }
    }

    /// The supervisor's phone, and who they are.
    async fn reach(
        &self,
        tenant_id: &fcp_core::TenantId,
        target: &str,
    ) -> Result<(String, Option<UserId>), SuperviseError> {
        if target.starts_with("sip:") || target.starts_with("sips:") {
            return Ok((target.to_string(), None));
        }
        let unreachable = || SuperviseError::Unreachable(target.to_string());
        let (Some(storage), Some(registrar)) = (&self.storage, &self.registrar) else {
            return Err(unreachable());
        };
        let user = crate::ring_plan::user_of(storage, tenant_id, target)
            .await
            .ok_or_else(unreachable)?;
        let contacts =
            crate::ring_plan::contacts_for_user(storage, registrar, tenant_id, &user, "").await;
        let contact = contacts
            .first()
            .map(crate::ring_plan::contact_uri)
            .ok_or_else(unreachable)?;
        Ok((contact, Some(user)))
    }

    /// Dial the supervisor with the party's SDP; their dialog once joined.
    async fn dial(
        &self,
        call_id: &CallId,
        session: &fcp_core::CallId,
        contact: &str,
        mode: Supervision,
    ) -> Result<sip_dialog::Dialog, SuperviseError> {
        let offer = self
            .media
            .add_party(session, mode)
            .await
            .map_err(|e| SuperviseError::Media(e.to_string()))?;
        // The supervisor sees the call they are joining.
        self.uac
            .inner()
            .set_display_name(Some(format!("Supervise {}", mode.as_str())))
            .await;
        self.uac.inner().set_from_uri(None).await;
        let sent = tokio::time::timeout(
            Duration::from_secs(5),
            self.uac.invite(contact, Some(&offer)),
        )
        .await;
        self.uac.inner().set_display_name(None).await;
        let handle = match sent {
            Ok(Ok(handle)) => handle,
            Ok(Err(e)) => {
                warn!(call_id = %call_id, error = %e, "The supervisor's INVITE failed");
                return Err(SuperviseError::Refused(503));
            }
            Err(_) => return Err(SuperviseError::Refused(408)),
        };
        let response = match tokio::time::timeout(RING_TIMEOUT, handle.await_final()).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => return Err(SuperviseError::Refused(503)),
            Err(_) => return Err(SuperviseError::Refused(408)),
        };
        let code = response.start_line().code();
        if !(200..300).contains(&code) {
            return Err(SuperviseError::Refused(code));
        }
        let dialog = handle.dialog.read().await.clone();
        let answer = String::from_utf8_lossy(response.body()).to_string();
        if let Err(e) = self.media.connect_party(session, answer).await {
            let _ = self.uac.bye(&dialog).await;
            return Err(SuperviseError::Media(e.to_string()));
        }
        Ok(dialog)
    }

    /// A phone barging into `call_id` with its own INVITE (a shared line's
    /// active appearance): the party added and connected to its offer; the
    /// answer to send it. [`Supervisions::attach_dialog`] follows once the
    /// INVITE's dialog exists.
    pub async fn barge_in(
        &self,
        call_id: &CallId,
        party: &str,
        offer: String,
    ) -> Result<String, SuperviseError> {
        let call = self.anchored(call_id)?;
        let session = call.media_session_id.ok_or(SuperviseError::NotAnchored)?;
        match self.active.entry(*call_id) {
            dashmap::Entry::Occupied(_) => return Err(SuperviseError::AlreadySupervised),
            dashmap::Entry::Vacant(slot) => {
                slot.insert(Active {
                    mode: Supervision::Barge,
                    party: party.to_string(),
                    user_id: None,
                    dialog: None,
                    session,
                });
            }
        }
        let joined = async {
            self.media
                .add_party(&session, Supervision::Barge)
                .await
                .map_err(|e| SuperviseError::Media(e.to_string()))?;
            self.media
                .connect_party(&session, offer)
                .await
                .map_err(|e| SuperviseError::Media(e.to_string()))
        }
        .await;
        match joined {
            Ok(answer) => {
                info!(call_id = %call_id, party, "A phone barged in");
                self.joined(call_id, Supervision::Barge, party, None);
                Ok(answer)
            }
            Err(e) => {
                self.active.remove(call_id);
                let _ = self.media.remove_party(&session).await;
                Err(e)
            }
        }
    }

    /// The barging phone's dialog, now that the INVITE is answered.
    pub fn attach_dialog(&self, call_id: &CallId, dialog: sip_dialog::Dialog) {
        self.by_dialog
            .insert(dialog.id().call_id().to_string(), *call_id);
        if let Some(mut active) = self.active.get_mut(call_id) {
            active.dialog = Some(dialog);
        }
    }

    /// Whether a SIP Call-ID is a supervisor's dialog.
    pub fn is_supervisor(&self, sip_call_id: &str) -> bool {
        self.by_dialog.contains_key(sip_call_id)
    }

    /// The supervisor sent BYE: they are taken off (the BYE is the
    /// caller's to answer).
    pub async fn hung_up(&self, sip_call_id: &str) {
        let Some((_, call_id)) = self.by_dialog.remove(sip_call_id) else {
            return;
        };
        if let Some((_, active)) = self.active.remove(&call_id) {
            let _ = self.media.remove_party(&active.session).await;
            info!(call_id = %call_id, supervisor = %active.party, "The supervisor hung up");
            self.left(&call_id, &active);
        }
    }

    /// Take the supervisor off: their phone is sent BYE. `false` when the
    /// call has none.
    pub async fn leave(&self, call_id: &CallId) -> bool {
        let Some((_, active)) = self.active.remove(call_id) else {
            return false;
        };
        let _ = self.media.remove_party(&active.session).await;
        self.bye(&active);
        info!(call_id = %call_id, supervisor = %active.party, "The supervisor was taken off");
        self.left(call_id, &active);
        true
    }

    /// The call ended: its supervisor is sent BYE (the media went with the
    /// call's session).
    pub fn ended(&self, call_id: &CallId) {
        if let Some((_, active)) = self.active.remove(call_id) {
            self.bye(&active);
            self.left(call_id, &active);
        }
    }

    fn bye(&self, active: &Active) {
        if let Some(dialog) = active.dialog.clone() {
            self.by_dialog.remove(dialog.id().call_id());
            let uac = Arc::clone(&self.uac);
            tokio::spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(10), uac.bye(&dialog)).await;
            });
        }
    }

    fn joined(&self, call_id: &CallId, mode: Supervision, party: &str, user_id: Option<UserId>) {
        self.call_manager.publish(fcp_events::PlatformEvent::Call(
            fcp_events::CallEvent::PartyJoined {
                call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                tenant_id: self.call_manager.get_call(call_id).map(|c| c.tenant_id),
                mode: mode.as_str().to_string(),
                party: party.to_string(),
                user_id: user_id.map(|u| u.to_string()),
                timestamp: SystemTime::now(),
                node_id: None,
            },
        ));
    }

    fn left(&self, call_id: &CallId, active: &Active) {
        self.call_manager.publish(fcp_events::PlatformEvent::Call(
            fcp_events::CallEvent::PartyLeft {
                call_id: fcp_core::CallId::from_uuid(*call_id.as_uuid()),
                tenant_id: self.call_manager.get_call(call_id).map(|c| c.tenant_id),
                party: active.party.clone(),
                timestamp: SystemTime::now(),
                node_id: None,
            },
        ));
    }
}
