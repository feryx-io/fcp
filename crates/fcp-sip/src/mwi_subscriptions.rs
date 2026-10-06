//! Subscribed message waiting (RFC 3842): a phone or an app SUBSCRIBEs to
//! `message-summary` and is told its mailbox's counts at once, then
//! whenever they change (`docs/SOFTPHONE.md` phase A3).
//!
//! The unsolicited NOTIFYs of `crate::mwi` reach the devices on a
//! mailbox's line; a subscription reaches whoever asked, a softphone
//! included. A subscription lives on the node that answered it; every
//! node hears `voicemail.mwi_updated` and tells its own.

use crate::registrar::{FcpRegistrar, InviteAuth};
use crate::sca::{
    contact_of, event_of, expires_of, header, respond, respond_ok, tag_of, without_tag,
};
use crate::uac::SipUserAgentClient;
use bytes::Bytes;
use dashmap::DashMap;
use fcp_core::storage::{DeviceStore, VoicemailStore};
use fcp_core::{DynStorage, MwiStatus};
use sip_core::{Headers, Method, Request, RequestLine, SipUri};
use smol_str::SmolStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// The event package.
pub const EVENT: &str = "message-summary";
/// The longest subscription granted, and the length of one that asks for
/// none.
const MAX_EXPIRES: u32 = 3600;

/// One subscription: enough to send NOTIFYs inside it.
struct Subscription {
    mailbox_id: String,
    call_id: String,
    their_tag: String,
    our_tag: String,
    /// The subscriber's `From` without its tag: our NOTIFYs' `To`.
    subscriber: String,
    /// The address subscribed to: our NOTIFYs' `From`.
    resource: String,
    contact: SipUri,
    cseq: AtomicU32,
    expires_at: std::sync::Mutex<Instant>,
}

impl Subscription {
    fn remaining(&self) -> u64 {
        self.expires_at
            .lock()
            .expect("expiry")
            .saturating_duration_since(Instant::now())
            .as_secs()
    }
}

/// The message-summary subscriptions this node answers.
pub struct MwiSubscriptions {
    storage: Arc<DynStorage>,
    uac: Arc<SipUserAgentClient>,
    registrar: Option<Arc<FcpRegistrar>>,
    /// This node's SIP address, for our Contact.
    host: String,
    /// By the subscriber's Call-ID.
    subs: DashMap<String, Arc<Subscription>>,
}

/// An RFC 3842 body.
pub fn summary_body(status: &MwiStatus) -> String {
    format!(
        "Messages-Waiting: {}\r\nVoice-Message: {}/{} ({}/{})\r\n",
        if status.has_messages() { "yes" } else { "no" },
        status.new_messages,
        status.old_messages,
        status.urgent_new_messages,
        status.urgent_old_messages,
    )
}

impl MwiSubscriptions {
    pub fn new(
        storage: Arc<DynStorage>,
        uac: Arc<SipUserAgentClient>,
        registrar: Option<Arc<FcpRegistrar>>,
        host: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            storage,
            uac,
            registrar,
            host,
            subs: DashMap::new(),
        })
    }

    /// Whether a SUBSCRIBE is for this package.
    pub fn handles_event(request: &Request) -> bool {
        event_of(request).as_deref() == Some(EVENT)
    }

    /// The subscriber's mailbox: the user their (authenticated) `From`
    /// names, through an extension, a number or a device they act for.
    async fn mailbox_of(&self, request: &Request) -> Option<String> {
        let registrar = self.registrar.as_ref()?;
        let tenant = registrar.caller_tenant(request).await?;
        let from = crate::registrar::from_user(request)?;
        let user = match crate::ring_plan::user_of(&self.storage, &tenant, &from).await {
            Some(user) => Some(user),
            // A device of its own (an app install, a hot desk): its owner.
            None => match self.storage.get_device_by_sip_name(&tenant, &from).await {
                Ok(Some(device)) => device.owner_user_id,
                _ => None,
            },
        }?;
        self.storage
            .get_mailbox_by_user(&user)
            .await
            .ok()
            .flatten()
            .map(|m| m.id.to_string())
    }

    /// Answer a `message-summary` SUBSCRIBE: authenticated like an INVITE,
    /// for the subscriber's own mailbox.
    pub async fn handle_subscribe(
        &self,
        request: &Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> anyhow::Result<()> {
        let call_id = header(request, "Call-ID").unwrap_or_default();
        let expires = expires_of(request).unwrap_or(MAX_EXPIRES).min(MAX_EXPIRES);

        // A refresh or an end, inside a subscription we hold.
        if let Some(sub) = self.subs.get(&call_id).map(|s| Arc::clone(&s)) {
            *sub.expires_at.lock().expect("expiry") =
                Instant::now() + Duration::from_secs(u64::from(expires));
            respond_ok(handle, request, Some(&sub.our_tag), expires).await?;
            let status = self.status(&sub.mailbox_id).await;
            if expires == 0 {
                self.subs.remove(&call_id);
                self.send_notify(&sub, "terminated;reason=timeout", status.as_ref());
            } else {
                self.send_notify(&sub, &format!("active;expires={expires}"), status.as_ref());
            }
            return Ok(());
        }

        if let Some(registrar) = &self.registrar {
            match registrar.authenticate_invite(request).await {
                InviteAuth::Admitted { .. } | InviteAuth::NotRequired => {}
                InviteAuth::WrongUser { .. } => {
                    return respond(handle, request, 403, "Forbidden").await;
                }
                InviteAuth::Challenge { stale } => {
                    let response =
                        registrar
                            .invite_challenge(request, stale)
                            .unwrap_or_else(|_| {
                                sip_uas::UserAgentServer::create_response(
                                    request,
                                    500,
                                    "Server Internal Error",
                                )
                            });
                    handle.send_final(response).await;
                    return Ok(());
                }
            }
        }
        let Some(mailbox_id) = self.mailbox_of(request).await else {
            return respond(handle, request, 404, "No Mailbox").await;
        };
        if expires == 0 {
            // A fetch (RFC 6665 §4.4.3): the counts once, then over.
            return self.answer_new(request, handle, mailbox_id, 0).await;
        }
        self.answer_new(request, handle, mailbox_id, expires).await
    }

    async fn answer_new(
        &self,
        request: &Request,
        handle: &sip_transaction::ServerTransactionHandle,
        mailbox_id: String,
        expires: u32,
    ) -> anyhow::Result<()> {
        let (Some(contact), Some(from), Some(call_id)) = (
            contact_of(request).and_then(|c| SipUri::parse(&c).ok()),
            header(request, "From"),
            header(request, "Call-ID"),
        ) else {
            return respond(handle, request, 400, "Bad Request").await;
        };
        let to = header(request, "To").unwrap_or_default();
        let sub = Arc::new(Subscription {
            mailbox_id,
            call_id: call_id.clone(),
            their_tag: tag_of(&from).unwrap_or_default(),
            our_tag: uuid::Uuid::new_v4().simple().to_string(),
            subscriber: without_tag(&from),
            resource: without_tag(&to),
            contact,
            cseq: AtomicU32::new(0),
            expires_at: std::sync::Mutex::new(
                Instant::now() + Duration::from_secs(u64::from(expires)),
            ),
        });
        respond_ok(handle, request, Some(&sub.our_tag), expires).await?;
        let status = self.status(&sub.mailbox_id).await;
        if expires == 0 {
            self.send_notify(&sub, "terminated;reason=timeout", status.as_ref());
            return Ok(());
        }
        info!(mailbox_id = %sub.mailbox_id, subscriber = %sub.subscriber, expires, "A subscription to message waiting");
        self.send_notify(&sub, &format!("active;expires={expires}"), status.as_ref());
        self.subs.insert(call_id, sub);
        Ok(())
    }

    async fn status(&self, mailbox_id: &str) -> Option<MwiStatus> {
        match self.storage.get_mwi_status(mailbox_id).await {
            Ok(status) => Some(status),
            Err(e) => {
                warn!(mailbox_id, error = %e, "A mailbox's counts could not be read");
                None
            }
        }
    }

    /// A mailbox's counts changed (`voicemail.mwi_updated`): its
    /// subscribers on this node are told. Lapsed subscriptions go.
    pub async fn mailbox_changed(&self, mailbox_id: &str) {
        self.subs.retain(|_, s| s.remaining() > 0);
        let watching: Vec<Arc<Subscription>> = self
            .subs
            .iter()
            .filter(|s| s.mailbox_id == mailbox_id)
            .map(|s| Arc::clone(&s))
            .collect();
        if watching.is_empty() {
            return;
        }
        let status = self.status(mailbox_id).await;
        for sub in watching {
            let state = format!("active;expires={}", sub.remaining());
            self.send_notify(&sub, &state, status.as_ref());
        }
    }

    /// Send a NOTIFY inside `sub`, from a task of its own.
    fn send_notify(&self, sub: &Arc<Subscription>, state: &str, status: Option<&MwiStatus>) {
        let cseq = sub.cseq.fetch_add(1, Ordering::SeqCst) + 1;
        let body = status.map(summary_body);
        let request = match notify_request(sub, &self.host, cseq, state, body.as_deref()) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "A message-summary NOTIFY could not be built");
                return;
            }
        };
        let uac = Arc::clone(&self.uac);
        let target = sub.contact.clone();
        let mailbox = sub.mailbox_id.clone();
        tokio::spawn(async move {
            match uac
                .inner()
                .send_request(request, sip_uac::integrated::RequestTarget::Uri(target))
                .await
            {
                Ok(r) if r.start_line().is_success() => {}
                Ok(r) => {
                    debug!(mailbox_id = %mailbox, status = r.start_line().code(), "A message-summary NOTIFY was refused")
                }
                Err(e) => {
                    debug!(mailbox_id = %mailbox, error = %e, "A message-summary NOTIFY was not delivered")
                }
            }
        });
    }
}

/// A NOTIFY inside `sub`, with the summary when there is one.
fn notify_request(
    sub: &Subscription,
    host: &str,
    cseq: u32,
    state: &str,
    body: Option<&str>,
) -> anyhow::Result<Request> {
    let mut headers = Headers::new();
    let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
    let mut push = |name: &str, value: String| {
        headers
            .push(SmolStr::new(name), SmolStr::new(value))
            .map_err(|e| anyhow::anyhow!("{name}: {e}"))
    };
    push("Via", format!("SIP/2.0/UDP placeholder;branch={branch}"))?;
    push("Max-Forwards", "70".into())?;
    let resource = if sub.resource.is_empty() {
        format!("<sip:{host}>")
    } else {
        sub.resource.clone()
    };
    push("From", format!("{resource};tag={}", sub.our_tag))?;
    let to = if sub.their_tag.is_empty() {
        sub.subscriber.clone()
    } else {
        format!("{};tag={}", sub.subscriber, sub.their_tag)
    };
    push("To", to)?;
    push("Call-ID", sub.call_id.clone())?;
    push("CSeq", format!("{cseq} NOTIFY"))?;
    push("Contact", format!("<sip:{host}>"))?;
    push("Event", EVENT.to_string())?;
    push("Subscription-State", state.to_string())?;
    let body = body.unwrap_or_default();
    if !body.is_empty() {
        push("Content-Type", "application/simple-message-summary".into())?;
    }
    push("Content-Length", body.len().to_string())?;
    Request::new(
        RequestLine::new(Method::Notify, sub.contact.clone()),
        headers,
        Bytes::from(body.to_string()),
    )
    .map_err(|e| anyhow::anyhow!("NOTIFY: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_rfc_3842s() {
        let status = MwiStatus {
            mailbox_id: "m".into(),
            line_binding_id: None,
            new_messages: 2,
            old_messages: 5,
            urgent_new_messages: 1,
            urgent_old_messages: 0,
        };
        assert_eq!(
            summary_body(&status),
            "Messages-Waiting: yes\r\nVoice-Message: 2/5 (1/0)\r\n"
        );
    }
}
