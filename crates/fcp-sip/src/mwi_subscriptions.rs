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
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// The event package.
pub const EVENT: &str = "message-summary";
/// The longest subscription granted, and the length of one that asks for
/// none.
const MAX_EXPIRES: u32 = 3600;
/// How often a forgotten subscription is told it is over.
const DEACTIVATE_EVERY: Duration = Duration::from_secs(60);

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
    /// Forgotten subscriptions told they are over, by Call-ID: when.
    deactivated: DashMap<String, Instant>,
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
            deactivated: DashMap::new(),
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
                // A shared line (`docs/DEVICE_LINES.md` §3): its owner's
                // mailbox, on that line of every device that has it.
                _ => crate::hoteling::line_of_number(&self.storage, &tenant, &from)
                    .await
                    .and_then(|line| line.owner_user_id),
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
    /// for the subscriber's own mailbox. `peer` and `transport` are where
    /// and how it came.
    pub async fn handle_subscribe(
        &self,
        request: &Request,
        handle: &sip_transaction::ServerTransactionHandle,
        peer: SocketAddr,
        transport: sip_transaction::TransportKind,
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
        // A refresh of a subscription this node does not hold — it went with
        // a restart. Challenged, a VVX resent the same refresh on its old
        // nonce for ever and never lit (`docs/DEEP_TESTING.md` D8, finding
        // 39); answered 481 (RFC 6665 §4.2.1.2), a VVX 250 on 6.4.7 resent
        // it every four seconds, and ended from our side with `deactivated`
        // (RFC 6665 §4.1.3) it answered 200 and resent it five minutes
        // later. So a refresh from where its subscriber registered is taken
        // up as the subscription it names; any other is 481 and ended.
        if let Some(to_tag) = header(request, "To").as_deref().and_then(tag_of) {
            if let Some(mailbox_id) = self.vouched_for(request, peer, transport).await {
                info!(call_id = %call_id, "A forgotten message-summary subscription taken up: refreshed from where its subscriber registered");
                return self
                    .answer(request, handle, mailbox_id, expires, Some(to_tag))
                    .await;
            }
            info!(call_id = %call_id, "A message-summary refresh for no subscription here: 481");
            respond(handle, request, 481, "Subscription Does Not Exist").await?;
            self.deactivate(request, peer, transport);
            return Ok(());
        }

        if let Some(registrar) = &self.registrar {
            match registrar.authenticate_invite(request).await {
                InviteAuth::Admitted { .. } | InviteAuth::NotRequired => {}
                InviteAuth::WrongUser { .. } => {
                    info!(call_id = %call_id, "A message-summary SUBSCRIBE for someone else's mailbox: 403");
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
            info!(call_id = %call_id, from = ?header(request, "From"), "A message-summary SUBSCRIBE from someone with no mailbox: 404");
            return respond(handle, request, 404, "No Mailbox").await;
        };
        // With `expires` 0, a fetch (RFC 6665 §4.4.3): the counts once,
        // then over.
        self.answer(request, handle, mailbox_id, expires, None)
            .await
    }

    /// Accept a subscription: a new one, or with `our_tag` a forgotten one
    /// taken up inside the dialog its refresh names.
    async fn answer(
        &self,
        request: &Request,
        handle: &sip_transaction::ServerTransactionHandle,
        mailbox_id: String,
        expires: u32,
        our_tag: Option<String>,
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
            // A dialog taken up continues above the NOTIFYs sent before.
            cseq: AtomicU32::new(if our_tag.is_some() { clock_cseq() } else { 0 }),
            our_tag: our_tag.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string()),
            subscriber: without_tag(&from),
            resource: without_tag(&to),
            contact,
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

    /// The mailbox of a refresh this node holds no subscription for, when
    /// the refresh carries its own warrant: over UDP, from exactly where a
    /// live binding of its `From` was registered (a digest-authenticated
    /// REGISTER), with a Contact there too, so every NOTIFY goes only to
    /// that registered address. A forged source gets nothing back.
    async fn vouched_for(
        &self,
        request: &Request,
        peer: SocketAddr,
        transport: sip_transaction::TransportKind,
    ) -> Option<String> {
        if !matches!(transport, sip_transaction::TransportKind::Udp) {
            return None;
        }
        let contact = contact_of(request).and_then(|c| SipUri::parse(&c).ok())?;
        if !reaches(&contact, peer)
            || !self
                .registrar
                .as_ref()?
                .registered_from(request, peer)
                .await
        {
            return None;
        }
        self.mailbox_of(request).await
    }

    /// End a subscription this node forgot, inside the dialog the refresh
    /// names, so the subscriber starts a new one. The refresh is not
    /// authenticated, so the NOTIFY goes only back where it came from — a
    /// UDP subscriber whose Contact is the address it was seen at (behind
    /// NAT, made so on arrival) — and once a minute per Call-ID.
    fn deactivate(
        &self,
        request: &Request,
        peer: SocketAddr,
        transport: sip_transaction::TransportKind,
    ) {
        if !matches!(transport, sip_transaction::TransportKind::Udp) {
            return;
        }
        let (Some(contact), Some(from), Some(to), Some(call_id)) = (
            contact_of(request).and_then(|c| SipUri::parse(&c).ok()),
            header(request, "From"),
            header(request, "To"),
            header(request, "Call-ID"),
        ) else {
            return;
        };
        if !reaches(&contact, peer) {
            debug!(call_id = %call_id, contact = %contact.as_str(), peer = %peer, "A forgotten subscription's Contact is not where it came from: not ended");
            return;
        }
        let now = Instant::now();
        self.deactivated
            .retain(|_, at| now.duration_since(*at) < DEACTIVATE_EVERY);
        if self.deactivated.contains_key(&call_id) {
            return;
        }
        self.deactivated.insert(call_id.clone(), now);
        let cseq = clock_cseq();
        let sub = Arc::new(Subscription {
            mailbox_id: String::new(),
            call_id: call_id.clone(),
            their_tag: tag_of(&from).unwrap_or_default(),
            our_tag: tag_of(&to).unwrap_or_default(),
            subscriber: without_tag(&from),
            resource: without_tag(&to),
            contact,
            cseq: AtomicU32::new(cseq),
            expires_at: std::sync::Mutex::new(now),
        });
        info!(call_id = %call_id, "A forgotten message-summary subscription ended: deactivated");
        self.send_notify(&sub, "terminated;reason=deactivated", None);
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

/// A NOTIFY CSeq above any the node sent in a dialog before it forgot it:
/// the clock's seconds.
fn clock_cseq() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_secs() & 0x7fff_ffff) as u32)
        .unwrap_or(1 << 30)
}

/// Whether a UDP Contact is the address a request came from (no port is
/// 5060).
fn reaches(contact: &SipUri, peer: SocketAddr) -> bool {
    let host = contact.host().trim_start_matches('[').trim_end_matches(']');
    host.parse::<std::net::IpAddr>().ok() == Some(peer.ip())
        && contact.port().unwrap_or(5060) == peer.port()
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
    fn a_forgotten_subscription_is_ended_only_where_it_came_from() {
        let peer: SocketAddr = "203.0.113.7:5062".parse().unwrap();
        let at = |uri: &str| reaches(&SipUri::parse(uri).unwrap(), peer);
        assert!(at("sip:alice@203.0.113.7:5062"));
        assert!(!at("sip:alice@203.0.113.7"), "no port is 5060");
        assert!(!at("sip:alice@198.51.100.9:5062"), "someone else");
        assert!(
            !at("sip:alice@example.com:5062"),
            "a name is not an address"
        );
        let default: SocketAddr = "203.0.113.7:5060".parse().unwrap();
        assert!(reaches(
            &SipUri::parse("sip:alice@203.0.113.7").unwrap(),
            default
        ));
    }

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
