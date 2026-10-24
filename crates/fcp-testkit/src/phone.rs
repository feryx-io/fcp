//! A scripted SIP phone (`docs/TEST_FLOOR.md`): a UAC and a UAS on one
//! dialog manager over an ephemeral loopback socket, with a media leg
//! per call. It registers with digest auth, places and answers calls,
//! answers what the far end sends in a dialog (re-INVITE, REFER, INFO,
//! NOTIFY, BYE) and reports all of it as [`PhoneEvent`]s the test reads.

use crate::media::MediaLeg;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use sip_core::{Request, Response, SipUri};
use sip_dialog::{Dialog, DialogManager};
use sip_transaction::{
    request_branch_id, ServerTransactionHandle, TransactionKey, TransactionManager,
    TransportContext, TransportDispatcher,
};
use sip_transport::InboundPacket;
use sip_uac::integrated::IntegratedUAC;
use sip_uas::integrated::{IntegratedUAS, UasRequestHandler};
use sip_uas::UserAgentServer;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, warn};

/// What a phone saw, by SIP Call-ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneEvent {
    /// A provisional on an outbound call.
    Ringing { call_id: String, code: u16 },
    /// An outbound call answered.
    Answered { call_id: String },
    /// An outbound call refused with a final response.
    Failed { call_id: String, code: u16 },
    /// An INVITE arrived; the call is in `Phone::incoming`.
    Incoming { call_id: String, from: String },
    /// The far end cancelled an unanswered incoming call.
    Cancelled { call_id: String },
    /// The far end hung up.
    Bye { call_id: String },
    /// A re-INVITE with this SDP (answered with our own).
    ReInvite { call_id: String, sdp: String },
    /// A REFER, accepted with 202.
    Refer { call_id: String, refer_to: String },
    /// An INFO with this body, answered 200.
    Info { call_id: String, body: String },
    /// A NOTIFY with this body, answered 200.
    Notify {
        call_id: String,
        /// The `Event` header: `refer`, `check-sync`, `message-summary`,
        /// `call-info`, `line-seize`.
        event: String,
        body: String,
        /// The `Call-Info` header: a shared line's lamps.
        call_info: Option<String>,
    },
}

impl PhoneEvent {
    pub fn call_id(&self) -> &str {
        match self {
            PhoneEvent::Ringing { call_id, .. }
            | PhoneEvent::Answered { call_id }
            | PhoneEvent::Failed { call_id, .. }
            | PhoneEvent::Incoming { call_id, .. }
            | PhoneEvent::Cancelled { call_id }
            | PhoneEvent::Bye { call_id }
            | PhoneEvent::ReInvite { call_id, .. }
            | PhoneEvent::Refer { call_id, .. }
            | PhoneEvent::Info { call_id, .. }
            | PhoneEvent::Notify { call_id, .. } => call_id,
        }
    }
}

/// Where a call stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallPhase {
    Dialing,
    Ringing,
    Answered,
    Failed(u16),
    Ended,
}

/// A REGISTER this phone was sent, as a carrier's registrar sees one.
#[derive(Debug, Clone)]
pub struct SeenRegistration {
    /// The account being bound, from the `To` header.
    pub account: String,
    /// Who sent it, from `From`.
    pub from: String,
    /// Where the binding points, from `Contact`.
    pub contact: String,
    /// How long it asks for, from `Expires` (0 is a de-registration).
    pub expires: u32,
    /// The username in `Authorization`, when the request carried one.
    pub auth_username: Option<String>,
}

/// One call's shared state: its dialog, media and phase.
struct CallState {
    call_id: String,
    dialog: RwLock<Option<Dialog>>,
    media: Arc<MediaLeg>,
    phase: watch::Sender<CallPhase>,
    timings: Mutex<CallTimings>,
    /// The SDP the far end answered with, once it has.
    answer: Mutex<Option<String>>,
}

impl CallState {
    /// The call is over, by either side: its phase says so and its media
    /// leg stops sending, as a phone's does.
    fn end(&self) {
        self.phase.send_replace(CallPhase::Ended);
        self.media.hang_up();
    }
}

/// When a call's milestones happened, for a caller measuring a node under
/// load (`docs/CAPACITY_AND_SURVIVABILITY.md` §8). Times are elapsed from
/// the INVITE leaving this phone.
#[derive(Debug, Clone, Copy, Default)]
pub struct CallTimings {
    /// The first provisional, and what it was: `100`, `180`, `183`.
    pub first_provisional: Option<(u16, Duration)>,
    /// The first `18x` — the far end ringing, rather than a proxy's `100`.
    pub ringing: Option<Duration>,
    /// The final response, and its code.
    pub final_response: Option<(u16, Duration)>,
}

/// What the phone's handler and its tasks share.
struct Inner {
    events: mpsc::UnboundedSender<PhoneEvent>,
    incoming: mpsc::UnboundedSender<IncomingCall>,
    calls: DashMap<String, Arc<CallState>>,
    uas: OnceLock<Arc<IntegratedUAS>>,
    uac: OnceLock<Arc<IntegratedUAC>>,
    /// Answer every INVITE at once, rather than ringing until told.
    auto_answer: std::sync::atomic::AtomicBool,
    /// On a REFER: dial the Refer-To, report with NOTIFY, drop the old
    /// leg on success — what a phone does on a blind transfer.
    follow_refers: std::sync::atomic::AtomicBool,
    /// Ignore everything that arrives, as a phone off the network does.
    silent: std::sync::atomic::AtomicBool,
    /// Where the server is, for bare-number targets.
    server: SocketAddr,
    /// `;transport=tls` for a phone that talks SIPS, else empty: on every
    /// request URI built from `server`.
    transport_param: &'static str,
    /// Where its RTP is bound: the phone's own address.
    media_ip: std::net::IpAddr,
    /// The address its Contact and SDP name: `media_ip`, or for a phone
    /// behind NAT ([`Phone::behind_nat`]) one it cannot be reached at.
    advertised: std::net::IpAddr,
    /// The codec it offers first and, offered it, answers with
    /// ([`Phone::prefer_codec`]).
    codec: std::sync::Mutex<crate::media::Codec>,
    /// Every REGISTER this phone has been sent, in order — a carrier's
    /// registrar, for a test that asks who registered and as whom.
    registrations: std::sync::Mutex<Vec<SeenRegistration>>,
    /// The realm to challenge a REGISTER with before accepting it; unset
    /// (the default) takes every REGISTER as it comes.
    challenge_realm: std::sync::Mutex<Option<String>>,
}

impl Inner {
    fn emit(&self, ev: PhoneEvent) {
        let _ = self.events.send(ev);
    }

    fn uac(&self) -> &Arc<IntegratedUAC> {
        self.uac.get().expect("uac built")
    }

    /// Place a call to `target` (a bare number is a user or extension at
    /// the server) and follow it to its end on a task.
    async fn place_call(self: &Arc<Self>, target: &str) -> Result<Call> {
        self.place_call_with_headers(target, sip_core::Headers::new())
            .await
    }

    /// As [`Inner::place_call`], with the SDP and `body`'s parts in one
    /// multipart body.
    async fn place_call_with_body(
        self: &Arc<Self>,
        target: &str,
        body: sip_core::MultipartBody,
        headers: sip_core::Headers,
    ) -> Result<Call> {
        self.place_call_inner(target, Some(body), headers).await
    }

    /// As [`Inner::place_call`], with extra request headers.
    async fn place_call_with_headers(
        self: &Arc<Self>,
        target: &str,
        headers: sip_core::Headers,
    ) -> Result<Call> {
        self.place_call_inner(target, None, headers).await
    }

    async fn place_call_inner(
        self: &Arc<Self>,
        target: &str,
        multipart: Option<sip_core::MultipartBody>,
        headers: sip_core::Headers,
    ) -> Result<Call> {
        self.place_call_edited(target, multipart, headers, |sdp| sdp)
            .await
    }

    /// As [`Inner::place_call_inner`], the offer passed through `edit`.
    async fn place_call_edited(
        self: &Arc<Self>,
        target: &str,
        multipart: Option<sip_core::MultipartBody>,
        headers: sip_core::Headers,
        edit: impl FnOnce(String) -> String,
    ) -> Result<Call> {
        let target = if target.starts_with("sip:") || target.starts_with("sips:") {
            target.to_string()
        } else {
            format!("sip:{target}@{}{}", self.server, self.transport_param)
        };
        let codec = *self.codec.lock().unwrap();
        let media =
            Arc::new(MediaLeg::open_preferring(self.media_ip, self.advertised, codec).await?);
        let offer = edit(media.sdp("sendrecv"));
        let options = match multipart {
            Some(mut body) => {
                // The SDP first, then whatever the caller added.
                let mut whole = sip_core::MultipartBody::mixed();
                whole
                    .push(
                        sip_core::BodyPart::new("application/sdp", offer.clone())
                            .map_err(|e| anyhow!("sdp part: {e}"))?,
                    )
                    .map_err(|e| anyhow!("body: {e}"))?;
                for part in body.parts() {
                    whole.push(part.clone()).map_err(|e| anyhow!("body: {e}"))?;
                }
                body = whole;
                sip_uac::InviteOptions {
                    body: Some(sip_uac::InviteBody::multipart(&body)),
                    extra_headers: headers,
                    from: None,
                    filter: None,
                    destination: None,
                }
            }
            None => sip_uac::InviteOptions {
                body: Some(sip_uac::InviteBody::sdp(&offer)),
                extra_headers: headers,
                from: None,
                filter: None,
                destination: None,
            },
        };
        let handle = Arc::new(
            tokio::time::timeout(
                Duration::from_secs(5),
                self.uac().invite_with_options(target.as_str(), options),
            )
            .await
            .context("INVITE could not be sent")??,
        );
        let invite = handle.invite_request().await;
        let call_id = sip_parse::header(invite.headers(), "Call-ID")
            .map(|v| v.to_string())
            .context("INVITE has a Call-ID")?;
        let (phase_tx, _) = watch::channel(CallPhase::Dialing);
        let state = Arc::new(CallState {
            call_id: call_id.clone(),
            dialog: RwLock::new(None),
            media: Arc::clone(&media),
            phase: phase_tx,
            timings: Mutex::new(CallTimings::default()),
            answer: Mutex::new(None),
        });
        self.calls.insert(call_id.clone(), Arc::clone(&state));

        let inner = Arc::clone(self);
        let st = Arc::clone(&state);
        let placed = std::time::Instant::now();
        tokio::spawn(async move {
            // Provisionals and the final together: a non-2xx leaves the
            // transaction (and the provisional stream) alive for timer D,
            // so waiting for the stream to end first would wait 32 s.
            let final_fut = handle.await_final();
            tokio::pin!(final_fut);
            let outcome = loop {
                tokio::select! {
                    r = &mut final_fut => break r,
                    p = handle.await_provisional() => match p {
                        Some(resp) => {
                            let code = resp.start_line().code();
                            {
                                let mut t = st.timings.lock().unwrap();
                                let at = placed.elapsed();
                                t.first_provisional.get_or_insert((code, at));
                                if code > 100 {
                                    t.ringing.get_or_insert(at);
                                }
                            }
                            if code > 100 {
                                st.phase.send_replace(CallPhase::Ringing);
                                inner.emit(PhoneEvent::Ringing {
                                    call_id: st.call_id.clone(),
                                    code,
                                });
                            }
                        }
                        None => break final_fut.await,
                    },
                }
            };
            match &outcome {
                Ok(resp) => st
                    .timings
                    .lock()
                    .unwrap()
                    .final_response
                    .get_or_insert((resp.start_line().code(), placed.elapsed())),
                Err(_) => st
                    .timings
                    .lock()
                    .unwrap()
                    .final_response
                    .get_or_insert((0, placed.elapsed())),
            };
            match outcome {
                Ok(resp) if (200..300).contains(&resp.start_line().code()) => {
                    // The ACK out before the call counts as answered, or an
                    // immediate re-INVITE beats it to the far end and draws a
                    // 481. A 2xx is the INVITE's outcome once it is ACKed.
                    // (Waiting for the provisional stream to close waited the
                    // whole second after an authenticated retry, whose
                    // challenged first attempt holds the stream open: a far
                    // end that answered and hung up at once ended the call
                    // before it was answered here.)
                    let _ =
                        tokio::time::timeout(Duration::from_secs(1), handle.invite_outcome()).await;
                    let dialog = handle.dialog.read().await.clone();
                    *st.dialog.write().await = Some(dialog);
                    let answer = String::from_utf8_lossy(resp.body()).to_string();
                    *st.answer.lock().unwrap() = Some(answer.clone());
                    if let Err(e) = st.media.set_remote(&answer).await {
                        warn!(error = %e, "answer SDP unusable");
                    }
                    // A far end that answers and hangs up at once can have
                    // its BYE handled while this waited for the ACK: the
                    // call stays ended.
                    st.phase.send_if_modified(|phase| {
                        if *phase == CallPhase::Ended {
                            return false;
                        }
                        *phase = CallPhase::Answered;
                        true
                    });
                    inner.emit(PhoneEvent::Answered {
                        call_id: st.call_id.clone(),
                    });
                }
                Ok(resp) => {
                    let code = resp.start_line().code();
                    st.phase.send_replace(CallPhase::Failed(code));
                    inner.emit(PhoneEvent::Failed {
                        call_id: st.call_id.clone(),
                        code,
                    });
                }
                Err(e) => {
                    debug!(error = %e, "INVITE ended without a final");
                    st.phase.send_replace(CallPhase::Failed(0));
                    inner.emit(PhoneEvent::Failed {
                        call_id: st.call_id.clone(),
                        code: 0,
                    });
                }
            }
        });
        Ok(Call {
            inner: Arc::clone(self),
            state,
        })
    }

    /// Report a REFER's outcome on the dialog it arrived in: the implicit
    /// `refer` subscription's NOTIFY, a sipfrag with `code`.
    async fn notify_refer(&self, dialog: &Dialog, code: u16, reason: &str) -> Result<u16> {
        let mut dialog = dialog.clone();
        let id = dialog.id().clone();
        let sub_id =
            sip_dialog::SubscriptionId::new(id.call_id(), id.remote_tag(), id.local_tag(), "refer")
                .map_err(|e| anyhow!("refer subscription id: {e}"))?;
        let mut sub = sip_dialog::Subscription::unchecked_new(
            sub_id,
            sip_dialog::SubscriptionState::Terminated,
            dialog.local_uri().clone(),
            dialog.remote_uri().clone(),
            dialog.remote_target().clone(),
            Duration::from_secs(0),
            dialog.local_cseq(),
            0,
        );
        let uas = self.uas.get().expect("uas built");
        let request = uas
            .helper()
            .lock()
            .await
            .create_notify_sipfrag(&mut sub, code, reason)
            .map_err(|e| anyhow!("refer NOTIFY: {e}"))?;
        let resp = tokio::time::timeout(
            Duration::from_secs(10),
            self.uac().send_in_dialog_non_invite(&mut dialog, request),
        )
        .await
        .context("NOTIFY timed out")??;
        Ok(resp.start_line().code())
    }

    /// A REFER arrived on `dialog`: dial `refer_to`, report, and on
    /// success drop the old leg. The Refer-To's URI headers become the
    /// INVITE's request headers (RFC 3261 §19.1.5) — that is how a
    /// migration ticket reaches the new node from a phone.
    async fn follow_refer(self: Arc<Self>, old_call_id: String, dialog: Dialog, refer_to: String) {
        let raw = refer_to
            .split('<')
            .nth(1)
            .and_then(|s| s.split('>').next())
            .unwrap_or(&refer_to)
            .trim()
            .to_string();
        let (target, headers) = match SipUri::parse(&raw) {
            Ok(uri) => {
                let mut headers = sip_core::Headers::new();
                for (name, value) in uri.headers() {
                    let _ = headers.push(
                        smol_str::SmolStr::new(name.as_str()),
                        smol_str::SmolStr::new(value.as_str()),
                    );
                }
                (uri.without_uri_headers().as_str().to_string(), headers)
            }
            Err(_) => (raw, sip_core::Headers::new()),
        };
        let outcome = match self.place_call_with_headers(&target, headers).await {
            Ok(call) => match call.answered(Duration::from_secs(15)).await {
                Ok(()) => Ok(()),
                Err(_) => Err(match call.phase() {
                    CallPhase::Failed(code) if code > 0 => code,
                    _ => 408,
                }),
            },
            Err(_) => Err(503),
        };
        match outcome {
            Ok(()) => {
                let _ = self.notify_refer(&dialog, 200, "OK").await;
                if let Some(old) = self.calls.get(&old_call_id).map(|e| e.value().clone()) {
                    let _ = tokio::time::timeout(Duration::from_secs(10), self.uac().bye(&dialog))
                        .await;
                    old.end();
                    self.calls.remove(&old_call_id);
                }
            }
            Err(code) => {
                let _ = self.notify_refer(&dialog, code, "Transfer Failed").await;
            }
        }
    }
}

/// A request's body as a multipart body, when its Content-Type says so.
fn multipart_of(request: &Request) -> Option<sip_core::MultipartBody> {
    let content_type = sip_parse::header(request.headers(), "Content-Type")?;
    sip_core::MultipartBody::parse(content_type.as_str(), request.body()).ok()
}

/// A subscription a [`Phone`] made: enough to refresh or end it.
#[derive(Debug, Clone)]
pub struct PhoneSubscription {
    pub target: String,
    pub event: String,
    pub call_id: String,
    pub from_tag: String,
    pub to_tag: Option<String>,
    pub cseq: u32,
    /// The last final response's status.
    pub code: u16,
}

/// A SIP phone on loopback.
pub struct Phone {
    pub username: String,
    pub password: String,
    pub realm: String,
    /// Where the server is.
    pub server: SocketAddr,
    /// Where this phone is.
    pub local_addr: SocketAddr,
    inner: Arc<Inner>,
    events: tokio::sync::Mutex<mpsc::UnboundedReceiver<PhoneEvent>>,
    incoming: tokio::sync::Mutex<mpsc::UnboundedReceiver<IncomingCall>>,
    _socket: Arc<UdpSocket>,
    /// A TLS phone's connections, which [`Phone::sleep`] closes.
    tls_pool: Option<Arc<sip_transport::pool::TlsPool>>,
}

/// The phone's outbound side: its own UDP socket, and for a phone made
/// with [`Phone::new_tls`] a pool of TLS connections to the server. A
/// request that arrived on a connection is answered on it.
struct PhoneDispatcher {
    socket: Arc<UdpSocket>,
    tls: Option<PhoneTls>,
}

struct PhoneTls {
    pool: Arc<sip_transport::pool::TlsPool>,
    client: Arc<tokio_rustls::rustls::ClientConfig>,
}

#[async_trait]
impl TransportDispatcher for PhoneDispatcher {
    async fn dispatch(&self, ctx: &TransportContext, payload: Bytes) -> anyhow::Result<()> {
        if let Some(writer) = ctx.stream() {
            return sip_transport::send_stream(transport_kind(ctx.transport()), writer, payload)
                .await;
        }
        match ctx.transport() {
            sip_transaction::TransportKind::Tls => {
                let tls = self
                    .tls
                    .as_ref()
                    .ok_or_else(|| anyhow!("this phone was not made for TLS"))?;
                let server_name = ctx
                    .server_name()
                    .map(str::to_string)
                    .unwrap_or_else(|| ctx.peer().ip().to_string());
                tls.pool
                    .send_tls(ctx.peer(), server_name, Arc::clone(&tls.client), payload)
                    .await
            }
            _ => {
                self.socket.send_to(&payload, ctx.peer()).await?;
                Ok(())
            }
        }
    }
}

fn transaction_kind(kind: sip_transport::TransportKind) -> sip_transaction::TransportKind {
    use sip_transaction::TransportKind as T;
    match kind {
        sip_transport::TransportKind::Udp => T::Udp,
        sip_transport::TransportKind::Tcp => T::Tcp,
        sip_transport::TransportKind::Tls => T::Tls,
        sip_transport::TransportKind::Sctp => T::Sctp,
        sip_transport::TransportKind::TlsSctp => T::TlsSctp,
        sip_transport::TransportKind::Ws => T::Ws,
        sip_transport::TransportKind::Wss => T::Wss,
    }
}

fn transport_kind(kind: sip_transaction::TransportKind) -> sip_transport::TransportKind {
    use sip_transport::TransportKind as T;
    match kind {
        sip_transaction::TransportKind::Udp => T::Udp,
        sip_transaction::TransportKind::Tcp => T::Tcp,
        sip_transaction::TransportKind::Tls => T::Tls,
        sip_transaction::TransportKind::Sctp => T::Sctp,
        sip_transaction::TransportKind::TlsSctp => T::TlsSctp,
        sip_transaction::TransportKind::Ws => T::Ws,
        sip_transaction::TransportKind::Wss => T::Wss,
    }
}

/// `sdp` offering SDES-SRTP: its audio `RTP/SAVP`, with `crypto`.
fn offering_srtp(sdp: String, crypto: &str) -> String {
    let sdp = sdp.replacen(" RTP/AVP ", " RTP/SAVP ", 1);
    format!("{sdp}a=crypto:{crypto}\r\n")
}

fn disable_rate_limits() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let _ = sip_transport::set_udp_rate_limit(sip_transport::RateLimitConfig::disabled());
    });
}

impl Phone {
    /// A phone for `username` at `realm`, talking to the server at
    /// `server`. Bound and listening on return.
    pub async fn new(
        username: &str,
        password: &str,
        realm: &str,
        server: SocketAddr,
    ) -> Result<Self> {
        Self::bound_to("127.0.0.1:0", username, password, realm, server).await
    }

    /// As [`Phone::new`], from a chosen local address: a test that turns on
    /// which network a device is on binds `127.0.0.2:0` and so on (every
    /// 127.0.0.0/8 address is local on Linux).
    pub async fn bound_to(
        bind: &str,
        username: &str,
        password: &str,
        realm: &str,
        server: SocketAddr,
    ) -> Result<Self> {
        Self::build(
            bind, username, username, password, realm, server, None, None, None, None,
        )
        .await
    }

    /// The codec this phone offers first, and answers with when offered
    /// it — PCMU unless set. Two phones preferring different codecs make
    /// the node transcode between them.
    pub fn prefer_codec(&self, codec: crate::media::Codec) {
        *self.inner.codec.lock().unwrap() = codec;
    }

    /// A phone behind NAT: bound at `bind`, it names `advertised` in its
    /// Contact and SDP, an address it cannot be reached at — what a phone
    /// naming its LAN address, or loopback, does from behind a router.
    /// Every 127.0.0.0/8 address is local on Linux, so `bind` 127.0.0.2:0
    /// and `advertised` 127.0.0.1 is a phone the server sees at one address
    /// that tells it another.
    pub async fn behind_nat(
        bind: &str,
        advertised: std::net::IpAddr,
        username: &str,
        password: &str,
        realm: &str,
        server: SocketAddr,
    ) -> Result<Self> {
        Self::build(
            bind,
            username,
            username,
            password,
            realm,
            server,
            None,
            None,
            None,
            Some(advertised),
        )
        .await
    }

    /// A phone whose address is `username` but whose digest credentials
    /// are `auth_username`'s: a device registering with its owner's
    /// credentials, or credentials trying an address that is not theirs.
    pub async fn with_credentials(
        username: &str,
        auth_username: &str,
        password: &str,
        realm: &str,
        server: SocketAddr,
    ) -> Result<Self> {
        Self::build(
            "127.0.0.1:0",
            username,
            auth_username,
            password,
            realm,
            server,
            None,
            None,
            None,
            None,
        )
        .await
    }

    /// As [`Phone::with_credentials`], saying it is `user_agent` on every
    /// request: a provisioned desk phone (`Yealink SIP-T54W 96.86.0.70
    /// 80:5e:c0:…`) whose REGISTER names its model and MAC.
    pub async fn with_user_agent(
        username: &str,
        auth_username: &str,
        password: &str,
        realm: &str,
        server: SocketAddr,
        user_agent: &str,
    ) -> Result<Self> {
        Self::build(
            "127.0.0.1:0",
            username,
            auth_username,
            password,
            realm,
            server,
            None,
            Some(user_agent),
            None,
            None,
        )
        .await
    }

    /// A phone that talks to the server over SIPS (`server` is its TLS
    /// port), trusting the CA in `ca_pem` and nothing else: what a phone
    /// pointed at the compose stack's `sips_bind` with its `ca.crt` does
    /// (`docs/PRODUCTION_STACK.md` phase 3). Every request it sends goes
    /// over a TLS connection it opens and keeps; answers come back on it.
    /// It still listens on its UDP socket, which its Contact names.
    pub async fn new_tls(
        username: &str,
        password: &str,
        realm: &str,
        server: SocketAddr,
        ca_pem: &std::path::Path,
    ) -> Result<Self> {
        Self::build(
            "127.0.0.1:0",
            username,
            username,
            password,
            realm,
            server,
            Some(ca_pem),
            None,
            None,
            None,
        )
        .await
    }

    /// A phone app (`docs/PUSH.md`): over SIPS as [`Phone::new_tls`],
    /// naming its `instance` in each REGISTER (RFC 5626 `+sip.instance`),
    /// with the push address [`Phone::set_push`] gives it, and able to
    /// [`Phone::sleep`] as an app does when it leaves the screen.
    pub async fn new_app(
        username: &str,
        password: &str,
        realm: &str,
        server: SocketAddr,
        ca_pem: &std::path::Path,
        instance: &str,
    ) -> Result<Self> {
        Self::build(
            "127.0.0.1:0",
            username,
            username,
            password,
            realm,
            server,
            Some(ca_pem),
            None,
            Some(instance),
            None,
        )
        .await
    }

    /// The push address the next REGISTER carries in its Contact URI
    /// (RFC 8599: `;pn-provider=…;pn-param=…;pn-prid=…`), or none.
    pub fn set_push(&self, params: Option<&str>) -> Result<()> {
        self.uac().set_register_contact_params(params)
    }

    /// Sleep, as an app does off the screen: every TLS connection to the
    /// server closes, so the server holds the binding with no way in. The
    /// next request opens a new one, as a woken app does.
    pub async fn sleep(&self) {
        if let Some(pool) = &self.tls_pool {
            pool.close_all();
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn build(
        bind: &str,
        username: &str,
        auth_username: &str,
        password: &str,
        realm: &str,
        server: SocketAddr,
        tls_ca: Option<&std::path::Path>,
        user_agent: Option<&str>,
        instance: Option<&str>,
        advertised: Option<std::net::IpAddr>,
    ) -> Result<Self> {
        disable_rate_limits();
        let socket = Arc::new(UdpSocket::bind(bind).await?);
        let local_addr = socket.local_addr()?;
        let local_uri = SipUri::parse(&format!("sip:{username}@{realm}"))
            .map_err(|e| anyhow!("local uri: {e}"))?;
        let contact = format!(
            "sip:{username}@{}",
            SocketAddr::new(advertised.unwrap_or(local_addr.ip()), local_addr.port())
        );

        // The inbound channel: the socket, and any TLS connection, feed it.
        let (tx, mut rx) = mpsc::channel::<InboundPacket>(256);
        let tls = match tls_ca {
            Some(ca) => {
                let pool = Arc::new(sip_transport::pool::TlsPool::new());
                pool.set_inbound_tx(tx.clone()).await;
                Some(PhoneTls {
                    pool,
                    client: crate::tls::client_config(ca)?,
                })
            }
            None => None,
        };
        let tls_pool = tls.as_ref().map(|t| Arc::clone(&t.pool));
        let transport_param = if tls.is_some() { ";transport=tls" } else { "" };
        let dispatcher: Arc<dyn TransportDispatcher> = Arc::new(PhoneDispatcher {
            socket: Arc::clone(&socket),
            tls,
        });
        let tm = Arc::new(TransactionManager::new(Arc::clone(&dispatcher)));

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (incoming_tx, incoming_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            events: events_tx,
            incoming: incoming_tx,
            calls: DashMap::new(),
            uas: OnceLock::new(),
            registrations: std::sync::Mutex::new(Vec::new()),
            challenge_realm: std::sync::Mutex::new(None),
            uac: OnceLock::new(),
            auto_answer: std::sync::atomic::AtomicBool::new(false),
            follow_refers: std::sync::atomic::AtomicBool::new(false),
            silent: std::sync::atomic::AtomicBool::new(false),
            server,
            transport_param,
            media_ip: local_addr.ip(),
            advertised: advertised.unwrap_or(local_addr.ip()),
            codec: std::sync::Mutex::new(crate::media::Codec::Pcmu),
        });

        let handler: Arc<dyn UasRequestHandler> = Arc::new(Handler(Arc::clone(&inner)));
        let uas = Arc::new(
            IntegratedUAS::builder()
                .local_uri(local_uri.as_str())?
                .contact_uri(&contact)?
                .local_addr(local_addr.to_string())?
                .transaction_manager(Arc::clone(&tm))
                .dispatcher(Arc::clone(&dispatcher))
                .request_handler(handler)
                .build()?,
        );
        let dm: Arc<DialogManager> = uas.dialog_manager();
        let resolver = Arc::new(sip_dns::SipResolver::from_system()?);
        let mut builder = IntegratedUAC::builder()
            .local_uri(local_uri.as_str())?
            .contact_uri(&contact)?
            .local_addr(local_addr.to_string())?
            .via_advertised_addr(local_addr.to_string())?
            .transaction_manager(Arc::clone(&tm))
            .resolver(resolver)
            .dispatcher(dispatcher)
            .dialog_manager(dm)
            .credentials(auth_username, password);
        if let Some(agent) = user_agent {
            builder = builder.user_agent(agent);
        }
        if let Some(instance) = instance {
            builder = builder.enable_outbound(instance);
        }
        if let Some(advertised) = advertised {
            builder = builder.contact_advertised_addr(
                SocketAddr::new(advertised, local_addr.port()).to_string(),
            )?;
        }
        let uac = Arc::new(builder.build()?);
        let _ = inner.uas.set(Arc::clone(&uas));
        let _ = inner.uac.set(Arc::clone(&uac));

        // The inbound loop: the socket to the transaction layer and the UAS.
        let recv_socket = Arc::clone(&socket);
        tokio::spawn(async move {
            if let Err(e) = sip_transport::run_udp(recv_socket, tx).await {
                debug!(error = %e, "phone UDP loop ended");
            }
        });
        let loop_tm = Arc::clone(&tm);
        let loop_uas = Arc::clone(&uas);
        let loop_socket = Arc::clone(&socket);
        let loop_inner = Arc::clone(&inner);
        tokio::spawn(async move {
            while let Some(packet) = rx.recv().await {
                if loop_inner.silent.load(std::sync::atomic::Ordering::Relaxed) {
                    continue;
                }
                handle_packet(&loop_tm, &loop_uas, &loop_socket, packet).await;
            }
        });

        Ok(Self {
            username: username.to_string(),
            password: password.to_string(),
            realm: realm.to_string(),
            server,
            local_addr,
            inner,
            events: tokio::sync::Mutex::new(events_rx),
            incoming: tokio::sync::Mutex::new(incoming_rx),
            _socket: socket,
            tls_pool,
        })
    }

    /// Answer every incoming call as it arrives.
    pub fn auto_answer(&self, yes: bool) {
        self.inner
            .auto_answer
            .store(yes, std::sync::atomic::Ordering::Relaxed);
    }

    /// Stop answering anything, as a phone that has lost its network: the
    /// server's requests to it go unanswered until their transactions time
    /// out.
    pub fn fall_silent(&self) {
        self.inner
            .silent
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Follow a REFER as a phone does on a blind transfer: dial the
    /// Refer-To, NOTIFY the outcome, drop the old leg on success.
    pub fn follow_refers(&self, yes: bool) {
        self.inner
            .follow_refers
            .store(yes, std::sync::atomic::Ordering::Relaxed);
    }

    fn uac(&self) -> &Arc<IntegratedUAC> {
        self.inner.uac.get().expect("uac built")
    }

    /// REGISTER at the server (digest handled), returning the final
    /// status code.
    pub async fn register(&self) -> Result<u16> {
        self.register_for(3600).await
    }

    /// As [`Phone::register`], asking for `expires` seconds — what a
    /// device with a short registration does, and what a load run that
    /// re-registers on expiry needs.
    pub async fn register_for(&self, expires: u32) -> Result<u16> {
        let target = format!("sip:{}{}", self.server, self.inner.transport_param);
        let resp = tokio::time::timeout(
            Duration::from_secs(10),
            self.uac().register(target.as_str(), Some(expires)),
        )
        .await
        .context("REGISTER timed out")??;
        Ok(resp.start_line().code())
    }

    /// Place a call to `target` (a user or extension at the server:
    /// `sip:101@…` is built from a bare number).
    pub async fn call(&self, target: &str) -> Result<Call> {
        self.inner.place_call(target).await
    }

    /// As [`Phone::call`], offering SDES-SRTP: the audio section
    /// `RTP/SAVP` with `crypto` (an `a=crypto` value). The phone's own media
    /// stays plain, so this is for what is signalled, not what is heard.
    pub async fn call_offering_srtp(&self, target: &str, crypto: &str) -> Result<Call> {
        self.call_offering_srtp_with_headers(target, crypto, &[])
            .await
    }

    /// As [`Phone::call_offering_srtp`], with extra request headers (a
    /// `Replaces`, …).
    pub async fn call_offering_srtp_with_headers(
        &self,
        target: &str,
        crypto: &str,
        headers: &[(&str, &str)],
    ) -> Result<Call> {
        let mut extra = sip_core::Headers::new();
        for (name, value) in headers {
            extra
                .push(*name, *value)
                .map_err(|e| anyhow!("header {name}: {e}"))?;
        }
        let crypto = crypto.to_string();
        self.inner
            .place_call_edited(target, None, extra, move |sdp| offering_srtp(sdp, &crypto))
            .await
    }

    /// As [`Phone::call`], with extra request headers on the INVITE.
    pub async fn call_with_headers(&self, target: &str, headers: &[(&str, &str)]) -> Result<Call> {
        let mut extra = sip_core::Headers::new();
        for (name, value) in headers {
            extra
                .push(*name, *value)
                .map_err(|e| anyhow!("header {name}: {e}"))?;
        }
        self.inner.place_call_with_headers(target, extra).await
    }

    /// As [`Phone::call`], with `parts` beside the SDP in a
    /// `multipart/mixed` body: a device that sends its own location sends
    /// a PIDF-LO this way (RFC 6442).
    pub async fn call_with_body_parts(
        &self,
        target: &str,
        parts: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<Call> {
        let mut extra = sip_core::Headers::new();
        for (name, value) in headers {
            extra
                .push(*name, *value)
                .map_err(|e| anyhow!("header {name}: {e}"))?;
        }
        let mut body = sip_core::MultipartBody::mixed();
        for (content_type, part) in parts {
            body.push(
                sip_core::BodyPart::new(*content_type, part.to_string())
                    .map_err(|e| anyhow!("body part: {e}"))?,
            )
            .map_err(|e| anyhow!("body: {e}"))?;
        }
        self.inner.place_call_with_body(target, body, extra).await
    }

    /// SUBSCRIBE to `event` at `target` (a bare name is at the server),
    /// with a `Call-Info` header when given: what a shared-line phone
    /// sends for `call-info` and `line-seize` (`docs/ENTERPRISE_CALLING.md`
    /// §3.9). The NOTIFYs arrive as [`PhoneEvent::Notify`].
    pub async fn subscribe(
        &self,
        target: &str,
        event: &str,
        expires: u32,
        call_info: Option<&str>,
    ) -> Result<PhoneSubscription> {
        let target = if target.starts_with("sip:") || target.starts_with("sips:") {
            target.to_string()
        } else {
            format!("sip:{target}@{}{}", self.server, self.inner.transport_param)
        };
        let mut sub = PhoneSubscription {
            target,
            event: event.to_string(),
            call_id: format!("{}@{}", uuid::Uuid::new_v4().simple(), self.local_addr.ip()),
            from_tag: uuid::Uuid::new_v4().simple().to_string()[..10].to_string(),
            to_tag: None,
            cseq: 0,
            code: 0,
        };
        sub.code = self.send_subscribe(&mut sub, expires, call_info).await?;
        Ok(sub)
    }

    /// Refresh `sub` for `expires` seconds (0 ends it), inside its dialog.
    pub async fn resubscribe(
        &self,
        sub: &mut PhoneSubscription,
        expires: u32,
        call_info: Option<&str>,
    ) -> Result<u16> {
        sub.code = self.send_subscribe(sub, expires, call_info).await?;
        Ok(sub.code)
    }

    async fn send_subscribe(
        &self,
        sub: &mut PhoneSubscription,
        expires: u32,
        call_info: Option<&str>,
    ) -> Result<u16> {
        sub.cseq += 1;
        let uri = SipUri::parse(&sub.target).map_err(|e| anyhow!("target: {e}"))?;
        let mut headers = sip_core::Headers::new();
        let mut push = |name: &str, value: String| {
            headers
                .push(name, value)
                .map_err(|e| anyhow!("header {name}: {e}"))
        };
        push(
            "Via",
            format!(
                "SIP/2.0/UDP placeholder;branch=z9hG4bK{}",
                uuid::Uuid::new_v4().simple()
            ),
        )?;
        push("Max-Forwards", "70".into())?;
        push(
            "From",
            format!(
                "<sip:{}@{}>;tag={}",
                self.username, self.realm, sub.from_tag
            ),
        )?;
        push(
            "To",
            match &sub.to_tag {
                Some(tag) => format!("<{}>;tag={tag}", sub.target),
                None => format!("<{}>", sub.target),
            },
        )?;
        push("Call-ID", sub.call_id.clone())?;
        push("CSeq", format!("{} SUBSCRIBE", sub.cseq))?;
        push(
            "Contact",
            format!(
                "<sip:{}@{}{}>",
                self.username, self.local_addr, self.inner.transport_param
            ),
        )?;
        push("Event", sub.event.clone())?;
        push("Expires", expires.to_string())?;
        if let Some(value) = call_info {
            push("Call-Info", value.to_string())?;
        }
        push("Content-Length", "0".into())?;
        let request = Request::new(
            sip_core::RequestLine::new(sip_core::Method::Subscribe, uri),
            headers,
            Bytes::new(),
        )
        .map_err(|e| anyhow!("SUBSCRIBE: {e}"))?;
        let server = SipUri::parse(&format!(
            "sip:{}{}",
            self.server, self.inner.transport_param
        ))
        .map_err(|e| anyhow!("server: {e}"))?;
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            self.uac()
                .send_request(request, sip_uac::integrated::RequestTarget::Uri(server)),
        )
        .await
        .context("SUBSCRIBE timed out")??;
        if response.start_line().is_success() && sub.to_tag.is_none() {
            sub.to_tag = sip_parse::header(response.headers(), "To").and_then(|to| {
                to.rsplit_once("tag=")
                    .map(|(_, t)| t.split(';').next().unwrap_or(t).trim().to_string())
            });
        }
        // A digest retry is sent with the next CSeq.
        if let Some(cseq) = sip_parse::header(response.headers(), "CSeq")
            .and_then(|c| c.split_whitespace().next().and_then(|n| n.parse().ok()))
        {
            sub.cseq = sub.cseq.max(cseq);
        }
        Ok(response.start_line().code())
    }

    /// Every REGISTER this phone has accepted, in order: a carrier's view
    /// of who is bound here and as whom.
    pub fn registrations(&self) -> Vec<SeenRegistration> {
        self.inner
            .registrations
            .lock()
            .expect("registrations")
            .clone()
    }

    /// Wait until `want` accepts the registrations seen so far; `false` at
    /// the deadline.
    pub async fn registrations_settle(
        &self,
        within: Duration,
        want: impl Fn(&[SeenRegistration]) -> bool,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if want(&self.registrations()) {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Challenge every REGISTER with this realm before accepting it, as a
    /// carrier does, so a test can see which credentials answered.
    pub fn challenge_registrations(&self, realm: &str) {
        *self.inner.challenge_realm.lock().expect("challenge realm") = Some(realm.to_string());
    }

    /// The next incoming call; `None` at the deadline.
    pub async fn incoming(&self, within: Duration) -> Option<IncomingCall> {
        let mut rx = self.incoming.lock().await;
        tokio::time::timeout(within, rx.recv()).await.ok().flatten()
    }

    /// The next event; `None` at the deadline.
    pub async fn next_event(&self, within: Duration) -> Option<PhoneEvent> {
        let mut rx = self.events.lock().await;
        tokio::time::timeout(within, rx.recv()).await.ok().flatten()
    }

    /// Wait for an event `want` accepts, discarding others; `None` at the
    /// deadline.
    pub async fn expect(
        &self,
        within: Duration,
        want: impl Fn(&PhoneEvent) -> bool,
    ) -> Option<PhoneEvent> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return None;
            }
            let ev = self.next_event(left).await?;
            if want(&ev) {
                return Some(ev);
            }
        }
    }
}

/// A call in progress, from either side.
pub struct Call {
    inner: Arc<Inner>,
    state: Arc<CallState>,
}

impl Call {
    pub fn call_id(&self) -> &str {
        &self.state.call_id
    }

    pub fn media(&self) -> &MediaLeg {
        &self.state.media
    }

    /// The SDP the far end answered this phone's offer with.
    pub fn answer_sdp(&self) -> Option<String> {
        self.state.answer.lock().unwrap().clone()
    }

    /// When this call's milestones happened, measured from the INVITE
    /// leaving this phone (`docs/CAPACITY_AND_SURVIVABILITY.md` §8).
    pub fn timings(&self) -> CallTimings {
        *self.state.timings.lock().unwrap()
    }

    pub fn phase(&self) -> CallPhase {
        self.state.phase.borrow().clone()
    }

    /// Wait for the call to be answered; the failure code otherwise.
    pub async fn answered(&self, within: Duration) -> Result<()> {
        let mut rx = self.state.phase.subscribe();
        let deadline = tokio::time::Instant::now() + within;
        loop {
            match rx.borrow().clone() {
                CallPhase::Answered => return Ok(()),
                CallPhase::Failed(code) => return Err(anyhow!("call failed with {code}")),
                CallPhase::Ended => return Err(anyhow!("call ended before answer")),
                _ => {}
            }
            tokio::time::timeout_at(deadline, rx.changed())
                .await
                .map_err(|_| anyhow!("not answered within {within:?}"))?
                .map_err(|_| anyhow!("call state gone"))?;
        }
    }

    /// Wait for the far end to hang up.
    pub async fn ended(&self, within: Duration) -> bool {
        let mut rx = self.state.phase.subscribe();
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if *rx.borrow() == CallPhase::Ended {
                return true;
            }
            if tokio::time::timeout_at(deadline, rx.changed())
                .await
                .is_err()
            {
                return false;
            }
        }
    }

    /// Wait for the call to be refused, and answer with the code the far
    /// end gave; `None` if it was answered, ended, or is still ringing at
    /// the deadline.
    pub async fn failed(&self, within: Duration) -> Option<u16> {
        let mut rx = self.state.phase.subscribe();
        let deadline = tokio::time::Instant::now() + within;
        loop {
            match rx.borrow().clone() {
                CallPhase::Failed(code) => return Some(code),
                CallPhase::Answered | CallPhase::Ended => return None,
                _ => {}
            }
            tokio::time::timeout_at(deadline, rx.changed())
                .await
                .ok()?
                .ok()?;
        }
    }

    async fn dialog(&self) -> Result<Dialog> {
        self.state
            .dialog
            .read()
            .await
            .clone()
            .context("no dialog yet")
    }

    fn uac(&self) -> &Arc<IntegratedUAC> {
        self.inner.uac.get().expect("uac built")
    }

    /// Hang up: BYE, and the call is over here whatever the answer.
    pub async fn hangup(&self) -> Result<u16> {
        let dialog = self.dialog().await?;
        let resp = tokio::time::timeout(Duration::from_secs(10), self.uac().bye(&dialog))
            .await
            .context("BYE timed out")??;
        self.state.end();
        self.inner.calls.remove(&self.state.call_id);
        Ok(resp.start_line().code())
    }

    /// A re-INVITE with this direction attribute (`sendonly` to hold,
    /// `sendrecv` to resume), returning the status.
    pub async fn reinvite(&self, direction: &str) -> Result<u16> {
        self.reinvite_answered(direction)
            .await
            .map(|(code, _)| code)
    }

    /// As [`Call::reinvite`], with the SDP the far end answered with.
    pub async fn reinvite_answered(&self, direction: &str) -> Result<(u16, String)> {
        self.reinvite_with(self.state.media.sdp(direction)).await
    }

    /// As [`Call::reinvite_answered`], offering SDES-SRTP with `crypto` as
    /// [`Phone::call_offering_srtp`] does.
    pub async fn reinvite_offering_srtp(
        &self,
        direction: &str,
        crypto: &str,
    ) -> Result<(u16, String)> {
        self.reinvite_with(offering_srtp(self.state.media.sdp(direction), crypto))
            .await
    }

    async fn reinvite_with(&self, sdp: String) -> Result<(u16, String)> {
        let mut dialog = self.dialog().await?;
        let resp = tokio::time::timeout(
            Duration::from_secs(10),
            self.uac().send_reinvite(&mut dialog, Some(&sdp)),
        )
        .await
        .context("re-INVITE timed out")??;
        *self.state.dialog.write().await = Some(dialog);
        let code = resp.start_line().code();
        let answer = String::from_utf8_lossy(resp.body()).to_string();
        if (200..300).contains(&code) && !answer.is_empty() {
            let _ = self.state.media.set_remote(&answer).await;
        }
        Ok((code, answer))
    }

    /// This call's dialog as a `Replaces` value (RFC 3891): what another
    /// INVITE, or a Refer-To, names it by.
    pub async fn replaces(&self) -> Result<String> {
        let dialog = self.dialog().await?;
        let id = dialog.id();
        Ok(format!(
            "{};to-tag={};from-tag={}",
            id.call_id(),
            id.remote_tag(),
            id.local_tag()
        ))
    }

    pub async fn hold(&self) -> Result<u16> {
        self.reinvite("sendonly").await
    }

    pub async fn resume(&self) -> Result<u16> {
        self.reinvite("sendrecv").await
    }

    /// REFER the far end to `target` (blind transfer); the status.
    pub async fn refer(&self, target: &str) -> Result<u16> {
        let mut dialog = self.dialog().await?;
        let uri = SipUri::parse(target).map_err(|e| anyhow!("refer target: {e}"))?;
        let (resp, _) = tokio::time::timeout(
            Duration::from_secs(10),
            self.uac().send_refer(&mut dialog, &uri, None),
        )
        .await
        .context("REFER timed out")??;
        *self.state.dialog.write().await = Some(dialog);
        Ok(resp.start_line().code())
    }

    /// A DTMF digit as SIP INFO (`application/dtmf-relay`).
    pub async fn send_info_dtmf(&self, digit: char) -> Result<u16> {
        let mut dialog = self.dialog().await?;
        let body = format!("Signal={digit}\r\nDuration=160\r\n");
        let resp = tokio::time::timeout(
            Duration::from_secs(10),
            self.uac()
                .send_info(&mut dialog, "application/dtmf-relay", &body),
        )
        .await
        .context("INFO timed out")??;
        *self.state.dialog.write().await = Some(dialog);
        Ok(resp.start_line().code())
    }
}

/// An INVITE that arrived and is ringing until the test decides.
pub struct IncomingCall {
    pub call_id: String,
    pub from: String,
    call: Call,
    pending: Mutex<Option<(Response, ServerTransactionHandle)>>,
    ctx: TransportContext,
    request: Request,
}

impl IncomingCall {
    /// A header of the INVITE as it arrived, by name (case-insensitive).
    pub fn header(&self, name: &str) -> Option<String> {
        sip_parse::header(self.request.headers(), name).map(|v| v.to_string())
    }

    /// Every value of a header of the INVITE.
    pub fn headers_named(&self, name: &str) -> Vec<String> {
        self.request
            .headers()
            .iter()
            .filter(|h| h.name().eq_ignore_ascii_case(name))
            .map(|h| h.value().to_string())
            .collect()
    }

    /// The INVITE's body, as it arrived.
    pub fn body(&self) -> &[u8] {
        self.request.body()
    }

    /// The INVITE's body as a multipart body, when it is one.
    pub fn multipart(&self) -> Option<sip_core::MultipartBody> {
        multipart_of(&self.request)
    }

    /// The INVITE's request line target.
    pub fn request_uri(&self) -> String {
        self.request.uri().to_string()
    }

    /// Send the 200 OK; the call is up.
    pub async fn answer(self) -> Result<Call> {
        let (response, handle) = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .context("already answered or cancelled")?;
        handle.send_final(response).await;
        self.call.state.phase.send_replace(CallPhase::Answered);
        Ok(self.call)
    }

    /// Refuse with `code` (486 Busy Here, 603 Decline, …).
    pub async fn reject(self, code: u16, reason: &str) -> Result<()> {
        self.reject_with_headers(code, reason, &[]).await
    }

    /// Refuse with `code` and extra response headers (a 424 with its
    /// `Geolocation-Error`, …).
    pub async fn reject_with_headers(
        self,
        code: u16,
        reason: &str,
        headers: &[(&str, &str)],
    ) -> Result<()> {
        let (ok, handle) = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .context("already answered or cancelled")?;
        let mut response = UserAgentServer::create_response(&self.request, code, reason);
        for (name, value) in headers {
            response
                .headers_mut()
                .push(*name, *value)
                .map_err(|e| anyhow!("header {name}: {e}"))?;
        }
        // The same To tag the 180 carried: one early dialog, refused.
        if let Some(to) = ok.headers().get("To") {
            let _ = response.headers_mut().set_or_push("To", to);
        }
        if let Some(uas) = self.call.inner.uas.get() {
            uas.prepare_response(&mut response, &self.ctx).await;
        }
        handle.send_final(response).await;
        if let Some(dialog) = self.call.state.dialog.read().await.as_ref() {
            if let Some(uas) = self.call.inner.uas.get() {
                uas.dialog_manager().remove(dialog.id());
            }
        }
        self.call.state.phase.send_replace(CallPhase::Failed(code));
        self.call.inner.calls.remove(&self.call_id);
        Ok(())
    }

    /// Leave it ringing; the caller's timeout decides.
    pub fn leave_ringing(self) -> Call {
        // Dropping the pending answer keeps the transaction unanswered.
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        self.call
    }

    pub fn media(&self) -> &MediaLeg {
        self.call.media()
    }
}

/// The UAS side: what arrives in or out of a dialog.
struct Handler(Arc<Inner>);

impl Handler {
    async fn call_of(&self, request: &Request) -> Option<Arc<CallState>> {
        let call_id = sip_parse::header(request.headers(), "Call-ID")?;
        self.0
            .calls
            .get(call_id.as_str())
            .map(|e| e.value().clone())
    }

    async fn plain_ok(
        &self,
        request: &Request,
        handle: ServerTransactionHandle,
        ctx: &TransportContext,
    ) {
        let mut response = UserAgentServer::create_response(request, 200, "OK");
        if let Some(uas) = self.0.uas.get() {
            uas.prepare_response(&mut response, ctx).await;
        }
        handle.send_final(response).await;
    }
}

#[async_trait]
impl UasRequestHandler for Handler {
    fn supported_methods(&self) -> &'static [&'static str] {
        &[
            "INVITE", "ACK", "BYE", "CANCEL", "OPTIONS", "REFER", "INFO", "NOTIFY",
        ]
    }

    async fn on_invite(
        &self,
        request: &Request,
        handle: ServerTransactionHandle,
        ctx: &TransportContext,
        dialog: Option<&Dialog>,
    ) -> anyhow::Result<()> {
        let uas = self.0.uas.get().expect("uas built");
        // The SDP alone, or the SDP part of a multipart body (a location by
        // value rides beside it, RFC 6442).
        let offer = match multipart_of(request) {
            Some(body) => body
                .part_by_type("application/sdp")
                .map(|p| String::from_utf8_lossy(p.body()).to_string())
                .unwrap_or_default(),
            None => String::from_utf8_lossy(request.body()).to_string(),
        };
        let call_id = sip_parse::header(request.headers(), "Call-ID")
            .map(|v| v.to_string())
            .unwrap_or_default();

        if let Some(dialog) = dialog {
            // A re-INVITE: take the new offer, answer with ours.
            if let Some(call) = self.0.calls.get(&call_id).map(|e| e.value().clone()) {
                if !offer.is_empty() {
                    let _ = call.media.set_remote(&offer).await;
                }
                let sdp = call.media.sdp("sendrecv");
                let mut response = uas
                    .helper()
                    .lock()
                    .await
                    .create_ok(request, Some(&sdp))
                    .map_err(|e| anyhow!("re-INVITE 200: {e}"))?;
                uas.prepare_response(&mut response, ctx).await;
                handle.send_final(response).await;
                self.0.emit(PhoneEvent::ReInvite {
                    call_id,
                    sdp: offer,
                });
            } else {
                let _ = dialog;
                let mut response = UserAgentServer::create_response(
                    request,
                    481,
                    "Call/Transaction Does Not Exist",
                );
                uas.prepare_response(&mut response, ctx).await;
                handle.send_final(response).await;
            }
            return Ok(());
        }

        // A new call: media, the answer built now (so the 180 carries
        // the same To tag), sent when the test answers.
        let codec = *self.0.codec.lock().unwrap();
        let media =
            Arc::new(MediaLeg::open_preferring(self.0.media_ip, self.0.advertised, codec).await?);
        if !offer.is_empty() {
            if let Err(e) = media.set_remote(&offer).await {
                warn!(error = %e, "offer SDP unusable");
            }
        }
        let answer_sdp = media.sdp("sendrecv");
        let (mut ok, dialog) = uas
            .helper()
            .lock()
            .await
            .accept_invite(request, Some(&answer_sdp))
            .map_err(|e| anyhow!("accept: {e}"))?;
        uas.prepare_response(&mut ok, ctx).await;
        let mut ringing = UserAgentServer::create_response(request, 180, "Ringing");
        if let Some(to) = ok.headers().get("To") {
            let _ = ringing.headers_mut().set_or_push("To", to);
        }
        uas.prepare_response(&mut ringing, ctx).await;
        let from = sip_parse::header(request.headers(), "From")
            .map(|v| v.to_string())
            .unwrap_or_default();
        let (phase_tx, _) = watch::channel(CallPhase::Ringing);
        let state = Arc::new(CallState {
            call_id: call_id.clone(),
            dialog: RwLock::new(Some(dialog)),
            media,
            phase: phase_tx,
            timings: Mutex::new(CallTimings::default()),
            answer: Mutex::new(None),
        });
        self.0.calls.insert(call_id.clone(), Arc::clone(&state));
        let call = Call {
            inner: Arc::clone(&self.0),
            state,
        };
        if self
            .0
            .auto_answer
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            handle.send_final(ok).await;
            call.state.phase.send_replace(CallPhase::Answered);
            self.0.emit(PhoneEvent::Incoming {
                call_id: call_id.clone(),
                from: from.clone(),
            });
            let incoming = IncomingCall {
                call_id,
                from,
                call,
                pending: Mutex::new(None),
                ctx: ctx.clone(),
                request: request.clone(),
            };
            let _ = self.0.incoming.send(incoming);
            return Ok(());
        }
        handle.send_provisional(ringing).await;
        self.0.emit(PhoneEvent::Incoming {
            call_id: call_id.clone(),
            from: from.clone(),
        });
        let incoming = IncomingCall {
            call_id,
            from,
            call,
            pending: Mutex::new(Some((ok, handle))),
            ctx: ctx.clone(),
            request: request.clone(),
        };
        let _ = self.0.incoming.send(incoming);
        Ok(())
    }

    async fn on_bye(
        &self,
        request: &Request,
        handle: ServerTransactionHandle,
        ctx: &TransportContext,
        _dialog: &Dialog,
    ) -> anyhow::Result<()> {
        self.plain_ok(request, handle, ctx).await;
        if let Some(call) = self.call_of(request).await {
            call.end();
            self.0.calls.remove(&call.call_id);
            self.0.emit(PhoneEvent::Bye {
                call_id: call.call_id.clone(),
            });
        }
        Ok(())
    }

    async fn on_cancel(
        &self,
        request: &Request,
        handle: ServerTransactionHandle,
        ctx: &TransportContext,
    ) -> anyhow::Result<()> {
        self.plain_ok(request, handle, ctx).await;
        if let Some(call) = self.call_of(request).await {
            call.end();
            self.0.calls.remove(&call.call_id);
            self.0.emit(PhoneEvent::Cancelled {
                call_id: call.call_id.clone(),
            });
        }
        Ok(())
    }

    async fn on_refer(
        &self,
        request: &Request,
        handle: ServerTransactionHandle,
        dialog: &Dialog,
    ) -> anyhow::Result<()> {
        let uas = self.0.uas.get().expect("uas built");
        let (response, refer_to) = uas
            .helper()
            .lock()
            .await
            .accept_refer(request, dialog)
            .map_err(|e| anyhow!("REFER: {e}"))?;
        handle.send_final(response).await;
        let call_id = sip_parse::header(request.headers(), "Call-ID")
            .map(|v| v.to_string())
            .unwrap_or_default();
        self.0.emit(PhoneEvent::Refer {
            call_id: call_id.clone(),
            refer_to: refer_to.clone(),
        });
        if self
            .0
            .follow_refers
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let inner = Arc::clone(&self.0);
            let dialog = dialog.clone();
            tokio::spawn(async move { inner.follow_refer(call_id, dialog, refer_to).await });
        }
        Ok(())
    }

    async fn on_info(
        &self,
        request: &Request,
        handle: ServerTransactionHandle,
        _dialog: &Dialog,
    ) -> anyhow::Result<()> {
        let response = UserAgentServer::create_response(request, 200, "OK");
        handle.send_final(response).await;
        let call_id = sip_parse::header(request.headers(), "Call-ID")
            .map(|v| v.to_string())
            .unwrap_or_default();
        self.0.emit(PhoneEvent::Info {
            call_id,
            body: String::from_utf8_lossy(request.body()).to_string(),
        });
        Ok(())
    }

    /// A carrier's registrar: record the binding and accept it, or
    /// challenge it once when the phone was told a realm, so a test can
    /// see which credentials answered.
    async fn on_register(
        &self,
        request: &Request,
        handle: ServerTransactionHandle,
        _ctx: &TransportContext,
    ) -> anyhow::Result<()> {
        let header = |name: &str| {
            sip_parse::header(request.headers(), name)
                .map(|v| v.to_string())
                .unwrap_or_default()
        };
        let authorization =
            sip_parse::header(request.headers(), "Authorization").map(|v| v.to_string());
        let auth_username = authorization.as_deref().and_then(|a| {
            a.split("username=").nth(1).map(|rest| {
                rest.trim_start_matches('"')
                    .split('"')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
        });
        let seen = SeenRegistration {
            account: header("To"),
            from: header("From"),
            contact: header("Contact"),
            expires: header("Expires").trim().parse().unwrap_or(0),
            auth_username,
        };
        let challenge = self
            .0
            .challenge_realm
            .lock()
            .expect("challenge realm")
            .clone();
        // Challenge the first send of a series and take the answer: what
        // a carrier does, and what proves whose credentials answered.
        let response = match (&challenge, &seen.auth_username) {
            (Some(realm), None) => {
                let mut unauthorized =
                    UserAgentServer::create_response(request, 401, "Unauthorized");
                let _ = unauthorized.headers_mut().push(
                    smol_str::SmolStr::new("WWW-Authenticate"),
                    smol_str::SmolStr::new(format!(
                        "Digest realm=\"{realm}\", nonce=\"{}\", algorithm=MD5",
                        uuid::Uuid::new_v4().simple()
                    )),
                );
                unauthorized
            }
            _ => UserAgentServer::create_response(request, 200, "OK"),
        };
        let accepted = response.start_line().code() == 200;
        if accepted {
            self.0
                .registrations
                .lock()
                .expect("registrations")
                .push(seen);
        }
        handle.send_final(response).await;
        Ok(())
    }

    async fn on_notify(
        &self,
        request: &Request,
        handle: ServerTransactionHandle,
    ) -> anyhow::Result<()> {
        let response = UserAgentServer::create_response(request, 200, "OK");
        handle.send_final(response).await;
        let call_id = sip_parse::header(request.headers(), "Call-ID")
            .map(|v| v.to_string())
            .unwrap_or_default();
        let event = sip_parse::header(request.headers(), "Event")
            .map(|v| v.to_string())
            .unwrap_or_default();
        let call_info = sip_parse::header(request.headers(), "Call-Info").map(|v| v.to_string());
        self.0.emit(PhoneEvent::Notify {
            call_id,
            event,
            body: String::from_utf8_lossy(request.body()).to_string(),
            call_info,
        });
        Ok(())
    }
}

/// One packet off the socket: a request to the UAS through the
/// transaction layer (ACKs straight to it), a response to the
/// transaction layer.
async fn handle_packet(
    tm: &Arc<TransactionManager>,
    uas: &Arc<IntegratedUAS>,
    socket: &Arc<UdpSocket>,
    packet: InboundPacket,
) {
    if let Some(req) = sip_parse::parse_request(packet.payload()) {
        let kind = transaction_kind(packet.transport());
        let mut ctx = TransportContext::new(kind, packet.peer(), packet.stream().cloned());
        if kind == sip_transaction::TransportKind::Udp {
            ctx = ctx.with_udp_socket(Some(Arc::clone(socket)));
        }
        if req.method() == &sip_core::Method::Ack {
            if let Some(branch) = request_branch_id(&req) {
                let key = TransactionKey::new(branch, sip_core::Method::Invite, true);
                let _ = tm.ack_received(&key).await;
            }
            return;
        }
        let handle = tm.receive_request(req.clone(), ctx.clone()).await;
        let uas = Arc::clone(uas);
        tokio::spawn(async move {
            if let Err(e) = uas.dispatch(&req, handle, &ctx).await {
                debug!(error = %e, "phone UAS dispatch");
            }
        });
        return;
    }
    if let Some(resp) = sip_parse::parse_response(packet.payload()) {
        tm.receive_response(resp).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two phones on loopback calling each other directly, no server:
    /// a refusal and an answer both reach the caller.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn phones_reach_each_other_directly() {
        let placeholder: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let a = Phone::new("a", "x", "test.local", placeholder)
            .await
            .unwrap();
        let b = Phone::new("b", "x", "test.local", placeholder)
            .await
            .unwrap();
        let target = format!("sip:b@{}", b.local_addr);

        let call = a.call(&target).await.unwrap();
        let incoming = b.incoming(Duration::from_secs(3)).await.expect("b rings");
        assert!(a
            .expect(Duration::from_secs(3), |e| matches!(
                e,
                PhoneEvent::Ringing { code: 180, .. }
            ))
            .await
            .is_some());
        incoming.reject(486, "Busy Here").await.unwrap();
        let err = call.answered(Duration::from_secs(3)).await.unwrap_err();
        assert!(err.to_string().contains("486"), "{err}");

        let call = a.call(&target).await.unwrap();
        let incoming = b
            .incoming(Duration::from_secs(3))
            .await
            .expect("b rings again");
        let b_call = incoming.answer().await.unwrap();
        call.answered(Duration::from_secs(3)).await.unwrap();
        assert_eq!(b_call.hangup().await.unwrap(), 200);
        assert!(call.ended(Duration::from_secs(3)).await);
    }
}
